use super::*;

pub const MAX_PROMPTS: usize = 8;
pub const MAX_PROMPT_BYTES: usize = 64 * 1024;
pub const MAX_PROMPT_TEXT: usize = 4096;
pub const PROMPT_LIFETIME: Duration = Duration::from_secs(300);
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuickPickItem {
    pub label: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub detail: String,
}
#[derive(Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum PromptType {
    QuickPick,
    InputBox,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PromptSpec {
    pub session: u64,
    pub owner: String,
    pub kind: PromptType,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub place_holder: String,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub items: Vec<QuickPickItem>,
    #[serde(default)]
    pub match_on_description: bool,
    #[serde(default)]
    pub match_on_detail: bool,
    #[serde(default)]
    pub command: Option<u64>,
    #[serde(default)]
    pub command_owner: Option<String>,
}
#[derive(Clone)]
pub struct NativePrompt {
    pub id: String,
    pub spec: PromptSpec,
    started: Instant,
    bytes: usize,
}
impl NativePrompt {
    pub fn matches(&self, query: &str) -> Vec<(usize, &QuickPickItem)> {
        let query = query.to_lowercase();
        self.spec
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                item.label.to_lowercase().contains(&query)
                    || self.spec.match_on_description
                        && item.description.to_lowercase().contains(&query)
                    || self.spec.match_on_detail && item.detail.to_lowercase().contains(&query)
            })
            .collect()
    }
}
impl Client {
    pub(super) fn queue_prompt(&mut self, id: Value, params: Value) -> Result<()> {
        let bytes = serde_json::to_vec(&params)?.len();
        if bytes > MAX_PROMPT_BYTES
            || self.prompts.iter().map(|p| p.bytes).sum::<usize>() + bytes > MAX_PROMPT_BYTES
        {
            bail!("Extension prompt presentation exceeds 64 KiB session budget");
        }
        if self.prompts.len() >= MAX_PROMPTS {
            bail!("Extension prompt queue limit reached");
        }
        let id = id
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= 128)
            .context("Invalid extension prompt ID")?
            .to_owned();
        if self.prompts.iter().any(|prompt| prompt.id == id) {
            bail!("Duplicate extension prompt ID");
        }
        let spec: PromptSpec = serde_json::from_value(params)?;
        self.validate_native_origin(
            spec.session,
            &spec.owner,
            spec.command,
            spec.command_owner.as_deref(),
        )?;
        if spec.items.len() > 128 || spec.kind == PromptType::InputBox && !spec.items.is_empty() {
            bail!("Invalid extension prompt item count");
        }
        let texts = [&spec.title, &spec.prompt, &spec.place_holder, &spec.value]
            .into_iter()
            .chain(
                spec.items
                    .iter()
                    .flat_map(|item| [&item.label, &item.description, &item.detail]),
            );
        if texts.into_iter().any(|text| text.len() > MAX_PROMPT_TEXT) {
            bail!("Extension prompt text exceeds 4 KiB");
        }
        self.prompts.push_back(NativePrompt {
            id,
            spec,
            bytes,
            started: Instant::now(),
        });
        Ok(())
    }
    pub fn prompt(&self) -> Option<&NativePrompt> {
        self.prompts.front()
    }
    pub fn answer_prompt(
        &mut self,
        session: u64,
        owner: &str,
        id: &str,
        result: Value,
    ) -> Result<bool> {
        let Some(index) = self.prompts.iter().position(|prompt| {
            prompt.id == id && prompt.spec.session == session && prompt.spec.owner == owner
        }) else {
            return Ok(false);
        };
        let prompt = &self.prompts[index];
        if !result.is_null() {
            match prompt.spec.kind {
                PromptType::QuickPick
                    if result
                        .as_u64()
                        .is_some_and(|index| index < prompt.spec.items.len() as u64) => {}
                PromptType::InputBox
                    if result
                        .as_str()
                        .is_some_and(|text| text.len() <= MAX_PROMPT_TEXT) => {}
                _ => bail!("Invalid extension prompt response"),
            }
        }
        self.process
            .send(json!({"id":id, "result":{"value":result}}))?;
        let prompt = self.prompts.remove(index).unwrap();
        if let Some(command) = prompt.spec.command
            && let Some(pending) = self.pending.get_mut(&command)
        {
            pending.started = Instant::now();
        }
        Ok(true)
    }
    pub fn cancel_prompts(&mut self) {
        while let Some(prompt) = self.prompts.front().cloned() {
            if self
                .answer_prompt(
                    prompt.spec.session,
                    &prompt.spec.owner,
                    &prompt.id,
                    Value::Null,
                )
                .is_err()
            {
                self.prompts.clear();
                break;
            }
        }
    }
    pub(super) fn expire_prompts(&mut self) -> Result<()> {
        let expired: Vec<_> = self
            .prompts
            .iter()
            .filter(|prompt| prompt.started.elapsed() >= PROMPT_LIFETIME)
            .cloned()
            .collect();
        for prompt in expired {
            self.answer_prompt(
                prompt.spec.session,
                &prompt.spec.owner,
                &prompt.id,
                Value::Null,
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn activation_service_origins_keep_api_owner_separate_and_reject_stale_or_orphan_commands() {
        let directory = tempfile::tempdir().unwrap();
        let extension =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/command-extension");
        let mut client = Client::start(
            "node",
            &extension,
            directory.path(),
            &[],
            0,
            &Settings::default(),
        )
        .unwrap();
        let owner = client.packages[0].id.clone();
        let mut second = client.packages[0].clone();
        second.id = "test.second".into();
        client.packages.push(second);
        client.pending.insert(
            2,
            Pending {
                owner: Some(owner.clone()),
                method: "activate".into(),
                started: Instant::now(),
            },
        );
        assert!(
            client
                .validate_native_origin(client.session, "test.second", Some(2), Some(&owner))
                .is_ok()
        );
        assert!(
            client
                .validate_native_origin(client.session, "test.second", Some(2), Some("test.second"))
                .is_err()
        );
        client.pending.remove(&2);
        assert!(
            client
                .validate_native_origin(client.session, "test.second", Some(2), Some(&owner))
                .is_err()
        );
        assert!(
            client
                .validate_native_origin(client.session, &owner, None, Some(&owner))
                .is_err()
        );
        client.pending.insert(
            3,
            Pending {
                owner: None,
                method: "activate".into(),
                started: Instant::now(),
            },
        );
        assert!(
            client
                .validate_native_origin(client.session, "test.second", Some(3), Some("test.second"))
                .is_ok()
        );
        assert!(
            client
                .validate_native_origin(client.session + 1, &owner, Some(3), Some(&owner))
                .is_err()
        );
    }
    #[test]
    fn prompt_queue_checks_ownership_bounds_deadlines_and_stale_answers() {
        let directory = tempfile::tempdir().unwrap();
        let extension =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/command-extension");
        let settings = Settings::load(&[]).unwrap();
        let mut client =
            Client::start("node", &extension, directory.path(), &[], 0, &settings).unwrap();
        let session = client.session;
        let owner = client.packages[0].id.clone();
        let request = || json!({"session":session,"owner":owner,"kind":"quickPick","items":[{"label":"label","detail":"detail"}],"command":1,"commandOwner":owner});
        client.queue_prompt(json!("one"), request()).unwrap();
        assert_eq!(client.prompt().unwrap().matches("label").len(), 1);
        assert_eq!(client.prompt().unwrap().matches("detail").len(), 0);
        assert!(client.queue_prompt(json!("one"), request()).is_err());
        let mut invalid = request();
        invalid["session"] = json!(session + 1);
        assert!(client.queue_prompt(json!("two"), invalid).is_err());
        let mut invalid = request();
        invalid["owner"] = json!("unknown.owner");
        assert!(client.queue_prompt(json!("two"), invalid).is_err());
        let mut invalid = request();
        invalid["title"] = json!("x".repeat(4097));
        assert!(client.queue_prompt(json!("two"), invalid).is_err());
        let mut invalid = request();
        invalid["items"] = json!(vec![json!({"label":"x"}); 129]);
        assert!(client.queue_prompt(json!("two"), invalid).is_err());
        for i in 1..8 {
            client
                .queue_prompt(json!(format!("prompt{i}")), request())
                .unwrap();
        }
        assert!(client.queue_prompt(json!("overflow"), request()).is_err());
        assert!(
            !client
                .answer_prompt(session + 1, &owner, "one", Value::Null)
                .unwrap()
        );
        assert!(
            !client
                .answer_prompt(session, "unknown.owner", "one", Value::Null)
                .unwrap()
        );
        assert!(
            client
                .answer_prompt(session, &owner, "one", json!(1))
                .is_err()
        );
        client.pending.get_mut(&1).unwrap().started = Instant::now() - Duration::from_secs(31);
        client
            .answer_prompt(session, &owner, "one", json!(0))
            .unwrap();
        assert!(client.pending[&1].started.elapsed() < Duration::from_secs(1));
        client.prompts.front_mut().unwrap().started = Instant::now() - PROMPT_LIFETIME;
        client.expire_prompts().unwrap();
        assert_eq!(client.prompts.len(), 6);
        client.cancel_prompts();
        assert!(client.prompt().is_none());
    }
    #[test]
    fn module_load_human_wait_pauses_only_its_origin_deadline() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("package.json"),
            r#"{"publisher":"deadline","name":"prompt","version":"1.0.0","main":"extension.cjs"}"#,
        )
        .unwrap();
        std::fs::write(
            directory.path().join("extension.cjs"),
            r#"
const vscode = require('vscode');
const input = vscode.window.showInputBox({ title: 'module load' });
exports.activate = async () => {
 await input;
 vscode.commands.registerCommand('deadline.prompt', async () => await vscode.window.showInputBox());
};"#,
        )
        .unwrap();
        let settings = Settings::default();
        let mut client = Client::start(
            "node",
            directory.path(),
            directory.path(),
            &[],
            0,
            &settings,
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while client.prompt().is_none() {
            client.poll(&mut [], 0, &settings).unwrap();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        let prompt = client.prompt().unwrap().clone();
        assert_eq!(prompt.spec.command, Some(1));
        assert_eq!(
            prompt.spec.command_owner.as_deref(),
            Some("deadline.prompt")
        );
        client.pending.get_mut(&1).unwrap().started = Instant::now() - Duration::from_secs(31);
        client.poll(&mut [], 0, &settings).unwrap();
        client
            .answer_prompt(
                prompt.spec.session,
                &prompt.spec.owner,
                &prompt.id,
                Value::Null,
            )
            .unwrap();
        while !client.ready {
            client.poll(&mut [], 0, &settings).unwrap();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        client
            .execute("deadline.prompt", None, &[], 0, &settings)
            .unwrap();
        while client.prompt().is_none() {
            client.poll(&mut [], 0, &settings).unwrap();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        let command = client.prompt().unwrap().spec.command.unwrap();
        client.pending.get_mut(&command).unwrap().started =
            Instant::now() - Duration::from_secs(31);
        client.poll(&mut [], 0, &settings).unwrap();
        // Another command from the same package must keep its own deadline.
        client.pending.insert(
            u64::MAX,
            Pending {
                owner: Some("deadline.prompt".into()),
                method: "execute".into(),
                started: Instant::now() - Duration::from_secs(31),
            },
        );
        assert!(
            client
                .poll(&mut [], 0, &settings)
                .unwrap_err()
                .to_string()
                .contains("timed out")
        );
    }
    #[test]
    fn completed_activation_origin_does_not_revive_during_the_next_package() {
        let directory = tempfile::tempdir().unwrap();
        let mut packages = Vec::new();
        for (name, source) in [
            (
                "a",
                "const vscode=require('vscode');exports.activate=()=>{setTimeout(()=>vscode.window.showInputBox({title:'completed A'}),20);};",
            ),
            (
                "b",
                "const vscode=require('vscode');exports.activate=async()=>{await vscode.window.showInputBox({title:'active B'});};",
            ),
        ] {
            let folder = directory.path().join(name);
            std::fs::create_dir(&folder).unwrap();
            std::fs::write(
                folder.join("package.json"),
                json!({"publisher":"origin","name":name,"version":"1.0.0","main":"extension.cjs"})
                    .to_string(),
            )
            .unwrap();
            std::fs::write(folder.join("extension.cjs"), source).unwrap();
            packages.push(Package::read(&folder).unwrap());
        }
        let settings = Settings::default();
        let mut client =
            Client::start_many("node", &packages, directory.path(), &[], 0, &settings).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while client.prompts.len() < 2 {
            client.poll(&mut [], 0, &settings).unwrap();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        let active = client
            .prompts
            .iter()
            .find(|p| p.spec.title == "active B")
            .unwrap();
        assert_eq!(active.spec.command, Some(1));
        assert_eq!(active.spec.command_owner.as_deref(), Some("origin.b"));
        let completed = client
            .prompts
            .iter()
            .find(|p| p.spec.title == "completed A")
            .unwrap();
        assert_eq!(completed.spec.command, None);
        assert_eq!(completed.spec.command_owner, None);
        client.cancel_prompts();
    }
    #[test]
    fn cross_package_prompt_keeps_api_owner_and_originating_command_distinct() {
        let directory = tempfile::tempdir().unwrap();
        let mut packages = Vec::new();
        for (name, source) in [
            (
                "a",
                "const v=require('vscode');exports.activate=()=>v.commands.registerCommand('origin.outer',async()=>await v.commands.executeCommand('origin.inner'));",
            ),
            (
                "b",
                "const v=require('vscode');exports.activate=()=>v.commands.registerCommand('origin.inner',async()=>await v.window.showInputBox());",
            ),
        ] {
            let folder = directory.path().join(name);
            std::fs::create_dir(&folder).unwrap();
            std::fs::write(
                folder.join("package.json"),
                json!({"publisher":"cross","name":name,"version":"1.0.0","main":"extension.cjs"})
                    .to_string(),
            )
            .unwrap();
            std::fs::write(folder.join("extension.cjs"), source).unwrap();
            packages.push(Package::read(&folder).unwrap());
        }
        let settings = Settings::default();
        let mut client =
            Client::start_many("node", &packages, directory.path(), &[], 0, &settings).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !client.ready {
            client.poll(&mut [], 0, &settings).unwrap();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        client
            .execute("origin.outer", None, &[], 0, &settings)
            .unwrap();
        while client.prompt().is_none() {
            client.poll(&mut [], 0, &settings).unwrap();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        let prompt = client.prompt().unwrap().clone();
        assert_eq!(prompt.spec.owner, "cross.b");
        assert_eq!(prompt.spec.command_owner.as_deref(), Some("cross.a"));
        client
            .pending
            .get_mut(&prompt.spec.command.unwrap())
            .unwrap()
            .started = Instant::now() - Duration::from_secs(31);
        client.poll(&mut [], 0, &settings).unwrap();
        client
            .answer_prompt(
                prompt.spec.session,
                &prompt.spec.owner,
                &prompt.id,
                Value::Null,
            )
            .unwrap();
        while client.busy() {
            client.poll(&mut [], 0, &settings).unwrap();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}
