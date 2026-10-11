use super::*;
use crate::extension_state::{Scope, Store};
use std::collections::VecDeque;

pub(crate) const SAVE_SERVICE_LIFETIME: Duration = Duration::from_secs(360);
pub(crate) const SERVICE_LIFETIME: Duration = Duration::from_secs(3);
pub(crate) const NATIVE_COMMANDS: &[&str] = &[
    "undo",
    "redo",
    "editor.action.selectAll",
    "cursorLeft",
    "cursorRight",
    "cursorUp",
    "cursorDown",
    "cursorHome",
    "cursorEnd",
    "cursorTop",
    "cursorBottom",
];
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Request {
    pub session: u64,
    pub owner: String,
    pub command: Option<u64>,
    pub command_owner: Option<String>,
    pub generation: Option<u64>,
    pub args: Value,
}
#[derive(Clone)]
pub(crate) enum Operation {
    Open(Open),
    Save(Save),
    Show(Show),
    Command(Command),
    State(StatePatch),
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Save {
    pub document: u64,
    pub version: u64,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Open {
    pub path: String,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Show {
    pub document: u64,
    pub version: u64,
    pub selection: Option<lsp::Range>,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Command {
    pub id: String,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StatePatch {
    pub scope: Scope,
    pub key: String,
    pub remove: bool,
    pub value: Value,
}
#[derive(Clone)]
pub(crate) struct NativeService {
    pub id: String,
    pub request: Request,
    pub operation: Operation,
    pub started: Instant,
    save_origin: Option<Mirror>,
}
impl NativeService {
    pub fn fresh(&self) -> Result<()> {
        let lifetime = if matches!(self.operation, Operation::Save(_)) {
            SAVE_SERVICE_LIFETIME
        } else {
            SERVICE_LIFETIME
        };
        if self.started.elapsed() >= lifetime {
            bail!("Native extension service expired; retry");
        }
        Ok(())
    }
}
impl Client {
    pub(crate) fn queue_service(&mut self, method: &str, id: Value, params: Value) -> Result<()> {
        if self.services.len() >= 8 {
            bail!("Native extension service queue limit reached");
        }
        if serde_json::to_vec(&params)?.len() > 72 * 1024 {
            bail!("Native extension service exceeds 72 KiB");
        }
        let id = id
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= 128)
            .context("Invalid native service ID")?
            .to_owned();
        if self.services.iter().any(|request| request.id == id) {
            bail!("Duplicate native service ID");
        }
        let request: Request = serde_json::from_value(params)?;
        self.validate_native_origin(
            request.session,
            &request.owner,
            request.command,
            request.command_owner.as_deref(),
        )?;
        let operation = match method {
            "nativeDocumentOpen" => {
                let open: Open = serde_json::from_value(request.args.clone())?;
                if open.path.len() > 16 * 1024 || open.path.is_empty() {
                    bail!("Invalid document path");
                }
                Operation::Open(open)
            }
            "nativeDocumentSave" => {
                let save: Save = serde_json::from_value(request.args.clone())?;
                if save.document == 0 || save.version == 0 {
                    bail!("Invalid save model/version");
                }
                Operation::Save(save)
            }
            "nativeDocumentShow" => Operation::Show(serde_json::from_value(request.args.clone())?),
            "nativeCommand" => {
                let command: Command = serde_json::from_value(request.args.clone())?;
                if !NATIVE_COMMANDS.contains(&command.id.as_str()) {
                    bail!(
                        "Native command delegation is not implemented: {}",
                        command.id
                    );
                }
                Operation::Command(command)
            }
            "nativeStateWrite" => Operation::State(serde_json::from_value(request.args.clone())?),
            _ => bail!("Unknown native service"),
        };
        if !matches!(operation, Operation::State(_) | Operation::Save(_))
            && request.generation != Some(self.mirror.generation)
        {
            bail!("Native document state changed before service request; retry");
        }
        let save_origin = if let Operation::Save(save) = &operation {
            let mirror = self
                .mirror
                .mirrors
                .get(&save.document)
                .context("Save model is not mirrored")?;
            if mirror.version != save.version || mirror.uri.len() > 4096 || mirror.eol.len() > 2 {
                bail!("Save source mirror changed or exceeds metadata budget");
            }
            Some(mirror.clone())
        } else {
            None
        };
        self.services.push_back(NativeService {
            id,
            request,
            operation,
            started: Instant::now(),
            save_origin,
        });
        Ok(())
    }
    pub(crate) fn queued_services(&self) -> impl Iterator<Item = &NativeService> {
        self.services.iter()
    }
    pub(crate) fn service(&self) -> Option<&NativeService> {
        self.services.front()
    }
    pub(crate) fn service_valid(&self, service: &NativeService) -> Result<()> {
        service.fresh()?;
        if !matches!(service.operation, Operation::State(_) | Operation::Save(_))
            && service.request.generation != Some(self.mirror.generation)
        {
            bail!("Native document state changed before service completion; retry");
        }
        if !self
            .services
            .front()
            .is_some_and(|current| current.id == service.id)
        {
            bail!("Native service is no longer pending");
        }
        if matches!(service.operation, Operation::Save(_)) {
            self.validate_native_origin(service.request.session, &service.request.owner, None, None)
        } else if matches!(service.operation, Operation::State(_)) {
            // Receipt already authorized the exact origin. An accepted durable
            // patch may outlive that command; never bind it to a later command.
            self.validate_selected_owner(service.request.session, &service.request.owner)
        } else {
            self.validate_native_origin(
                service.request.session,
                &service.request.owner,
                service.request.command,
                service.request.command_owner.as_deref(),
            )
        }
    }
    pub(crate) fn answer_service(
        &mut self,
        service: &NativeService,
        result: Result<Value>,
    ) -> Result<()> {
        let id = &service.id;
        if !self.services.front().is_some_and(|request| {
            request.id == *id
                && request.request.session == service.request.session
                && request.request.owner == service.request.owner
        }) {
            bail!("Native service response no longer pending");
        }
        let message = match result {
            Ok(result) => json!({"id":id,"result":result}),
            Err(error) => json!({"id":id,"error":{"message":format!("{error:#}")}}),
        };
        self.process.send(message)?;
        self.services.pop_front();
        Ok(())
    }
    pub(crate) fn service_document_current(&self, document: &Document, version: u64) -> bool {
        self.mirror.mirrors.get(&document.id).is_some_and(|mirror| {
            mirror.version == version
                && mirror.revision == document.revision
                && mirror.text_epoch == document.text_epoch()
                && document_uri(document).ok().as_ref() == Some(&mirror.uri)
                && mirror.eol == document.eol
        })
    }
    pub(crate) fn service_save_document_current(
        &self,
        service: &NativeService,
        document: &Document,
    ) -> bool {
        let Operation::Save(save) = &service.operation else {
            return false;
        };
        service.save_origin.as_ref().is_some_and(|origin| {
            document.id == save.document
                && document.revision == origin.revision
                && document.text_epoch() == origin.text_epoch
                && document.save_generation() == origin.save_generation
                && document.eol == origin.eol
                && document_uri(document).ok().as_ref() == Some(&origin.uri)
                && self.service_document_current(document, save.version)
        })
    }
    pub(crate) fn state_store(&self) -> Option<Store> {
        self.state_store.clone()
    }
}
impl Prepared {
    pub fn with_storage_root(mut self, root: Option<PathBuf>) -> Self {
        self.storage_root = root;
        self
    }
}
pub(crate) fn initial_state(store: Option<&Store>, packages: &[Package]) -> Result<Value> {
    let mut values = serde_json::Map::new();
    for package in packages {
        let read = |scope| -> Result<_> {
            Ok(match store {
                Some(store) => store.read(&package.id, scope)?,
                None => serde_json::Map::new(),
            })
        };
        values.insert(
            package.id.clone(),
            json!({"global":read(Scope::Global)?,"workspace":read(Scope::Workspace)?}),
        );
    }
    Ok(Value::Object(values))
}
pub(super) fn empty_queue() -> VecDeque<NativeService> {
    VecDeque::new()
}

#[cfg(test)]
mod tests {
    #[test]
    fn save_dto_rejects_unknown_and_invalid_version_types_without_open_show_coercion() {
        use super::Save;
        let parsed: Save =
            serde_json::from_value(serde_json::json!({"document":1,"version":2})).unwrap();
        assert_eq!(parsed.document, 1);
        assert_eq!(parsed.version, 2);
        for invalid in [
            serde_json::json!({"document":1,"version":2,"path":"other"}),
            serde_json::json!({"document":1,"version":2.5}),
            serde_json::json!({"document":1,"version":-1}),
            serde_json::json!({"document":1}),
        ] {
            assert!(serde_json::from_value::<Save>(invalid).is_err());
        }
    }
    use super::*;
    #[test]
    fn late_reply_cannot_consume_same_request_id_in_a_replacement_session() {
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
        let mut request = json!({"session":client.session,"owner":owner,"args":{"scope":"global","key":"accepted","remove":false,"value":true}});
        client
            .queue_service("nativeStateWrite", json!("host-1"), request.clone())
            .unwrap();
        let old = client.service().unwrap().clone();
        client.services.clear();
        client.session += 1;
        request["session"] = client.session.into();
        client
            .queue_service("nativeStateWrite", json!("host-1"), request)
            .unwrap();
        assert!(
            client
                .answer_service(&old, Err(anyhow::anyhow!("old worker stopped")))
                .is_err()
        );
        assert_eq!(client.services.len(), 1);
        assert_eq!(client.service().unwrap().request.session, client.session);
        assert!(client.service_valid(client.service().unwrap()).is_ok());
    }
    #[test]
    fn accepted_state_patch_outlives_its_origin_without_rebinding_or_crossing_sessions() {
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
        client.queue_service("nativeStateWrite",json!("unawaited"),json!({"session":client.session,"owner":owner,"command":1,"commandOwner":owner,"args":{"scope":"global","key":"accepted","remove":false,"value":true}})).unwrap();
        let accepted = client.service().unwrap().clone();
        client.pending.remove(&1); // originating initialize/command has returned
        assert!(client.service_valid(&accepted).is_ok());
        client.pending.insert(
            2,
            Pending {
                owner: Some("other.owner".into()),
                method: "execute".into(),
                started: Instant::now(),
            },
        );
        assert!(client.service_valid(&accepted).is_ok());
        client.session += 1;
        assert!(client.service_valid(&accepted).is_err());
        client.session -= 1;
        client.packages.clear();
        assert!(client.service_valid(&accepted).is_err());
    }
    #[test]
    fn service_queue_rejects_unknown_fields_wrong_origins_stale_generations_and_overflow() {
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
        let request = json!({"session":client.session,"owner":client.packages[0].id,"generation":1,"args":{"path":"one"}});
        let mut bad = request.clone();
        bad["args"]["unsupported"] = true.into();
        assert!(
            client
                .queue_service("nativeDocumentOpen", json!("bad"), bad)
                .is_err()
        );
        let mut bad = request.clone();
        bad["owner"] = "other.owner".into();
        assert!(
            client
                .queue_service("nativeDocumentOpen", json!("bad"), bad)
                .is_err()
        );
        let mut bad = request.clone();
        bad["generation"] = 0.into();
        assert!(
            client
                .queue_service("nativeDocumentOpen", json!("bad"), bad)
                .is_err()
        );
        let mut bad = request.clone();
        bad["args"] = json!({"id":"workbench.action.tasks.runTask"});
        assert!(
            client
                .queue_service("nativeCommand", json!("bad"), bad)
                .is_err()
        );
        for index in 0..8 {
            client
                .queue_service(
                    "nativeDocumentOpen",
                    json!(format!("request-{index}")),
                    request.clone(),
                )
                .unwrap();
        }
        assert!(
            client
                .queue_service("nativeDocumentOpen", json!("ninth"), request)
                .is_err()
        );
        let service = client.service().unwrap().clone();
        assert!(client.service_valid(&service).is_ok());
        client.mirror.generation += 1;
        assert!(client.service_valid(&service).is_err());
        client.mirror.generation -= 1;
        let mut expired = service;
        expired.started = Instant::now() - SERVICE_LIFETIME;
        assert!(client.service_valid(&expired).is_err());
    }
}
