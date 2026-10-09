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
        if spec.session != self.session || !self.packages.iter().any(|p| p.id == spec.owner) {
            bail!("Outdated extension prompt owner/session");
        }
        if let Some(command) = spec.command {
            let pending = self
                .pending
                .get(&command)
                .context("Outdated extension prompt command")?;
            if pending.method != "initialize"
                && (pending.method != "execute" || pending.owner != spec.command_owner)
            {
                bail!("Invalid extension prompt command owner");
            }
        }
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
    fn prompt_queue_checks_ownership_bounds_deadlines_and_stale_answers() {
        let directory = tempfile::tempdir().unwrap();
        let extension =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/command-extension");
        let settings = Settings::load(&[]).unwrap();
        let mut client =
            Client::start("node", &extension, directory.path(), &[], 0, &settings).unwrap();
        let session = client.session;
        let owner = client.packages[0].id.clone();
        let request = || json!({"session":session,"owner":owner,"kind":"quickPick","items":[{"label":"label","detail":"detail"}],"command":1});
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
}
