//! Atomic, bounded diagnostic snapshots from the optional extension process.
use super::*;
use std::{
    collections::BTreeMap,
    io::{self, Write},
};
const MAX_BYTES: usize = 2 * 1024 * 1024;
const MAX_ITEMS: usize = 5000;
const MAX_RESOURCES: usize = 128;
const MAX_COLLECTIONS: usize = 64;
const MAX_SAFE: u64 = (1 << 53) - 1;
#[derive(Default)]
pub(super) struct State {
    owners: BTreeMap<String, Owner>,
    epoch: u64,
}
struct Owner {
    generation: u64,
    bytes: usize,
    collections: BTreeMap<u64, Vec<Entry>>,
}
struct Entry {
    document: u64,
    revision: u64,
    text_epoch: u64,
    version: u64,
    path: Option<PathBuf>,
    items: Vec<lsp::Diagnostic>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Update {
    session: u64,
    owner: String,
    generation: u64,
    collections: Vec<Collection>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Collection {
    id: u64,
    name: String,
    entries: Vec<WireEntry>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireEntry {
    document: u64,
    version: u64,
    diagnostics: Vec<lsp::Diagnostic>,
}
struct Budget(usize);
impl Write for Budget {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.0 {
            return Err(io::Error::other("Extension diagnostics exceed 2 MiB"));
        }
        self.0 -= bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn bounded_text(value: &Value, limit: usize) -> Result<()> {
    if value.as_str().is_none_or(|text| text.len() > limit) {
        bail!("Invalid diagnostic metadata text");
    }
    Ok(())
}
fn validate(item: &lsp::Diagnostic, document: &Document) -> Result<()> {
    if item.message.len() > 4096
        || item
            .severity
            .is_none_or(|severity| !(1..=4).contains(&severity))
    {
        bail!("Invalid diagnostic message/severity");
    }
    let start = lsp::offset(document, item.range.start)?;
    let end = lsp::offset(document, item.range.end)?;
    if start > end {
        bail!("Reversed diagnostic range");
    }
    for (key, value) in &item.extra {
        match key.as_str() {
            "source" => bounded_text(value, 256)?,
            "code" => {
                if value.is_string() {
                    bounded_text(value, 4096)?;
                } else if value.as_i64().is_none_or(|n| n.unsigned_abs() > MAX_SAFE) {
                    bail!("Invalid diagnostic code");
                }
            }
            "codeDescription" => {
                let object = value
                    .as_object()
                    .context("Invalid diagnostic code description")?;
                if object.len() != 1 {
                    bail!("Invalid diagnostic code description");
                }
                bounded_text(&value["href"], 4096)?;
                if !matches!(
                    url::Url::parse(value["href"].as_str().unwrap())?.scheme(),
                    "file" | "https" | "http"
                ) {
                    bail!("Unsupported diagnostic code URI");
                }
            }
            "tags" => {
                let tags = value.as_array().context("Invalid diagnostic tags")?;
                if tags.len() > 2 || tags.iter().any(|tag| !matches!(tag.as_u64(), Some(1 | 2))) {
                    bail!("Invalid diagnostic tags");
                }
            }
            "relatedInformation" => {
                let related = value
                    .as_array()
                    .context("Invalid diagnostic related information")?;
                if related.len() > 32 {
                    bail!("Diagnostic related information exceeds 32 entries");
                }
                for entry in related {
                    let object = entry
                        .as_object()
                        .context("Invalid diagnostic related information")?;
                    if object.len() != 2 || object.keys().any(|k| k != "location" && k != "message")
                    {
                        bail!("Invalid diagnostic related information fields");
                    }
                    bounded_text(&entry["message"], 4096)?;
                    bounded_text(&entry["location"]["uri"], 4096)?;
                    url::Url::parse(entry["location"]["uri"].as_str().unwrap())?;
                    let location = entry["location"]
                        .as_object()
                        .context("Invalid diagnostic location")?;
                    if location.len() != 2 || location.keys().any(|k| k != "uri" && k != "range") {
                        bail!("Invalid diagnostic location fields");
                    }
                    let range: lsp::Range =
                        serde_json::from_value(entry["location"]["range"].clone())?;
                    if [
                        range.start.line,
                        range.start.character,
                        range.end.line,
                        range.end.character,
                    ]
                    .iter()
                    .any(|n| *n > i32::MAX as usize)
                        || (range.start.line, range.start.character)
                            > (range.end.line, range.end.character)
                    {
                        bail!("Invalid diagnostic related range");
                    }
                }
            }
            _ => bail!("Unsupported diagnostic metadata: {key}"),
        }
    }
    Ok(())
}
impl State {
    pub(super) fn remove_owner(&mut self, owner: &str) {
        if self.owners.remove(owner).is_some() {
            self.epoch = self.epoch.wrapping_add(1);
        }
    }
}
impl Client {
    pub fn diagnostics_epoch(&self) -> u64 {
        self.diagnostics.epoch
    }
    /// Rendering borrows stored items without allocating or resolving filesystem paths.
    pub fn diagnostics_for<'a>(
        &'a self,
        document: &'a Document,
    ) -> impl Iterator<Item = &'a lsp::Diagnostic> + 'a {
        self.diagnostics
            .owners
            .iter()
            .filter(move |(owner, _)| self.owner_active(owner))
            .flat_map(|(_, owner)| owner.collections.values())
            .flat_map(|entries| entries.iter())
            .filter(move |entry| {
                entry.document == document.id
                    && entry.revision == document.revision
                    && entry.text_epoch == document.text_epoch()
                    && entry.path == document.path
                    && self.mirror.mirrors.get(&document.id).is_some_and(|mirror| {
                        mirror.version == entry.version
                            && mirror.revision == document.revision
                            && mirror.text_epoch == document.text_epoch()
                    })
            })
            .flat_map(|entry| entry.items.iter())
    }
    pub(super) fn register_diagnostics(
        &mut self,
        value: Value,
        documents: &[Document],
        hidden: &[Document],
    ) -> Result<()> {
        let mut budget = Budget(MAX_BYTES);
        serde_json::to_writer(&mut budget, &value)?;
        let bytes = MAX_BYTES - budget.0;
        let update: Update = serde_json::from_value(value)?;
        self.validate_selected_owner(update.session, &update.owner)?;
        if self.owner_retired(&update.owner) {
            bail!("Diagnostic owner has retired");
        }
        if update.generation == 0
            || update.generation > MAX_SAFE
            || self
                .diagnostics
                .owners
                .get(&update.owner)
                .is_some_and(|owner| update.generation <= owner.generation)
        {
            bail!("Outdated diagnostic collection generation");
        }
        if update.collections.len() > MAX_COLLECTIONS {
            bail!("Diagnostic collections exceed 64 sources");
        }
        let mut next = Owner {
            generation: update.generation,
            bytes,
            collections: BTreeMap::new(),
        };
        for collection in update.collections {
            if collection.id == 0
                || collection.id > MAX_SAFE
                || collection.name.len() > 128
                || collection.entries.len() > MAX_RESOURCES
                || next.collections.contains_key(&collection.id)
            {
                bail!("Invalid diagnostic collection identity/bounds");
            }
            let mut entries = Vec::new();
            let mut ids = HashSet::new();
            for entry in collection.entries {
                if !ids.insert(entry.document) || entry.diagnostics.len() > MAX_ITEMS {
                    bail!("Duplicate diagnostic resource or too many items");
                }
                let document = documents
                    .iter()
                    .chain(hidden)
                    .find(|doc| doc.id == entry.document)
                    .context("Diagnostic target is not a mirrored document")?;
                if !self.service_document_current(document, entry.version) {
                    bail!("Outdated diagnostic document version");
                }
                for item in &entry.diagnostics {
                    validate(item, document)?;
                }
                entries.push(Entry {
                    document: document.id,
                    revision: document.revision,
                    text_epoch: document.text_epoch(),
                    version: entry.version,
                    path: document.path.clone(),
                    items: entry.diagnostics,
                });
            }
            entries.sort_by_key(|entry| entry.document);
            next.collections.insert(collection.id, entries);
        }
        let owners = self
            .diagnostics
            .owners
            .iter()
            .filter(|(owner, _)| *owner != &update.owner)
            .map(|(_, state)| state)
            .chain(std::iter::once(&next));
        let (mut total_bytes, mut count, mut collections, mut resources) =
            (0, 0, 0, HashSet::new());
        for owner in owners {
            total_bytes += owner.bytes;
            collections += owner.collections.len();
            for entry in owner.collections.values().flatten() {
                count += entry.items.len();
                resources.insert(entry.document);
            }
        }
        if total_bytes > MAX_BYTES
            || count > MAX_ITEMS
            || collections > MAX_COLLECTIONS
            || resources.len() > MAX_RESOURCES
        {
            bail!("Extension diagnostic aggregate budget exceeded");
        }
        self.diagnostics.owners.insert(update.owner, next);
        self.diagnostics.epoch = self.diagnostics.epoch.wrapping_add(1);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn client(root: &Path, docs: &mut [Document]) -> Client {
        let mut packages = Vec::new();
        for owner in ["one", "two"] {
            let path = root.join(owner);
            std::fs::create_dir(&path).unwrap();
            std::fs::write(
                path.join("package.json"),
                format!(
                    r#"{{"publisher":"test","name":"{owner}","version":"1","main":"index.cjs"}}"#
                ),
            )
            .unwrap();
            std::fs::write(path.join("index.cjs"), "exports.activate=()=>{};").unwrap();
            packages.push(Package::read(&path).unwrap());
        }
        let mut client =
            Client::start_many("node", &packages, root, docs, 0, &Settings::default()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !client.ready {
            client.poll(docs, 0, &Settings::default()).unwrap();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        client
    }
    fn item(message: &str) -> Value {
        json!({"range":{"start":{"line":0,"character":1},"end":{"line":0,"character":3}},"severity":2,"message":message,"source":"fixture"})
    }
    fn update(
        client: &Client,
        owner: &str,
        generation: u64,
        doc: &Document,
        items: Vec<Value>,
    ) -> Value {
        json!({"session":client.session,"owner":owner,"generation":generation,"collections":[{"id":1,"name":"same","entries":[{"document":doc.id,"version":client.mirror.mirrors[&doc.id].version,"diagnostics":items}]}]})
    }
    #[test]
    fn collection_wire_is_atomic_owner_scoped_and_hides_edit_undo_revision_reuse() {
        let root = tempfile::tempdir().unwrap();
        let mut docs = vec![Document::from_text("猫🙂\r\nlast")];
        let mut client = client(root.path(), &mut docs);
        for (owner, message) in [("test.two", "second"), ("test.one", "first")] {
            client
                .register_diagnostics(
                    update(&client, owner, 1, &docs[0], vec![item(message)]),
                    &docs,
                    &[],
                )
                .unwrap();
        }
        assert_eq!(
            client
                .diagnostics_for(&docs[0])
                .map(|d| d.message.as_str())
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
        let epoch = client.diagnostics_epoch();
        for bad in [
            json!({"severity":0}),
            json!({"message":"x".repeat(4097)}),
            json!({"range":{"start":{"line":0,"character":2},"end":{"line":0,"character":3}}}),
            json!({"unsafe":{"x":true}}),
        ] {
            let mut diagnostic = item("bad");
            for (key, value) in bad.as_object().unwrap() {
                diagnostic[key] = value.clone();
            }
            assert!(
                client
                    .register_diagnostics(
                        update(&client, "test.one", 2, &docs[0], vec![diagnostic]),
                        &docs,
                        &[]
                    )
                    .is_err()
            );
            assert_eq!(client.diagnostics_epoch(), epoch);
            assert_eq!(client.diagnostics_for(&docs[0]).count(), 2);
        }
        let mut stale = update(&client, "test.one", 2, &docs[0], vec![item("stale")]);
        stale["collections"][0]["entries"][0]["version"] = json!(99);
        assert!(client.register_diagnostics(stale, &docs, &[]).is_err());
        let revision = docs[0].revision;
        docs[0].insert("x", false);
        docs[0].undo();
        assert_eq!(docs[0].revision, revision);
        assert_eq!(client.diagnostics_for(&docs[0]).count(), 0);
        client.sync_with_hidden(&docs, &[], 0).unwrap();
        client
            .register_diagnostics(
                update(&client, "test.one", 2, &docs[0], vec![item("fresh")]),
                &docs,
                &[],
            )
            .unwrap();
        assert_eq!(client.diagnostics_for(&docs[0]).count(), 1);
        client.diagnostics.remove_owner("test.one");
        assert_eq!(client.diagnostics_for(&docs[0]).count(), 0);
        assert!(client.diagnostics.owners.contains_key("test.two"));
    }
    #[test]
    fn collection_wire_bounds_aggregate_items_sources_bytes_and_provenance() {
        let root = tempfile::tempdir().unwrap();
        let mut docs = vec![Document::from_text("猫🙂")];
        let mut client = client(root.path(), &mut docs);
        client
            .register_diagnostics(
                update(&client, "test.one", 1, &docs[0], vec![item("first"); 3000]),
                &docs,
                &[],
            )
            .unwrap();
        assert!(
            client
                .register_diagnostics(
                    update(&client, "test.two", 1, &docs[0], vec![item("second"); 2001]),
                    &docs,
                    &[]
                )
                .is_err()
        );
        assert_eq!(client.diagnostics_for(&docs[0]).count(), 3000);
        assert!(
            client
                .register_diagnostics(
                    update(
                        &client,
                        "test.two",
                        1,
                        &docs[0],
                        vec![item(&"x".repeat(4096)); 500]
                    ),
                    &docs,
                    &[]
                )
                .is_err()
        );
        let mut many = update(&client, "test.two", 1, &docs[0], vec![]);
        many["collections"] = json!(
            (1..=64)
                .map(|id| json!({"id":id,"name":"same","entries":[]}))
                .collect::<Vec<_>>()
        );
        assert!(client.register_diagnostics(many, &docs, &[]).is_err());
        for field in ["owner", "session", "generation"] {
            let mut invalid = update(&client, "test.one", 2, &docs[0], vec![item("invalid")]);
            invalid[field] = match field {
                "owner" => json!("evil.unknown"),
                "session" => json!(client.session + 1),
                _ => json!(1),
            };
            assert!(client.register_diagnostics(invalid, &docs, &[]).is_err());
        }
        client
            .activation_states
            .insert("test.one".into(), "failed".into());
        assert_eq!(client.diagnostics_for(&docs[0]).count(), 0);
        assert!(
            client
                .register_diagnostics(update(&client, "test.one", 2, &docs[0], vec![]), &docs, &[])
                .is_err()
        );
    }
    #[test]
    fn real_host_diagnostics_and_save_events_use_updated_hidden_and_untitled_mirrors() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("extension");
        std::fs::create_dir(&path).unwrap();
        std::fs::write(
            path.join("package.json"),
            r#"{"publisher":"test","name":"diagnostics","version":"1","main":"index.cjs"}"#,
        )
        .unwrap();
        std::fs::write(path.join("index.cjs"), r#"const v=require('vscode');exports.activate=()=>{const c=v.languages.createDiagnosticCollection('real');const lint=d=>c.set(d.uri,[new v.Diagnostic(new v.Range(0,1,0,3),'host warning',v.DiagnosticSeverity.Warning)]);for(const d of v.workspace.textDocuments)lint(d);v.workspace.onDidChangeTextDocument(e=>lint(e.document));v.workspace.onDidSaveTextDocument(d=>v.window.showInformationMessage('saved:'+d.version+':'+v.workspace.textDocuments.map(x=>x.isDirty).join(',')));};"#).unwrap();
        let file = root.path().join("猫.txt");
        std::fs::write(&file, "猫🙂\r\nlast").unwrap();
        let mut docs = vec![Document::open(&file).unwrap()];
        let mut hidden = vec![Document::from_text("猫🙂 hidden")];
        hidden[0].cursor = hidden[0].len();
        hidden[0].insert("!", false);
        let prepared =
            Client::prepare_with_hidden(&docs, &hidden, 0, &Settings::default()).unwrap();
        let package = Package::read(&path).unwrap();
        let mut client =
            Client::start_many_prepared("node", &[package], root.path(), prepared).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while client.diagnostics_for(&docs[0]).count() == 0
            || client.diagnostics_for(&hidden[0]).count() == 0
        {
            client
                .poll_with_hidden(&mut docs, &mut hidden, 0, &Settings::default())
                .unwrap();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(
            client.diagnostics_for(&docs[0]).next().unwrap().severity,
            Some(2)
        );
        let version = client.mirror.mirrors[&docs[0].id].version;
        docs[0].save().unwrap();
        let mut saved = false;
        while !saved {
            saved = client
                .poll_with_hidden(&mut docs, &mut hidden, 0, &Settings::default())
                .unwrap()
                .iter()
                .any(|message| message == &format!("saved:{version}:false,true"));
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(client.mirror.mirrors[&docs[0].id].version, version);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "猫🙂\r\nlast");
        let old = docs.remove(0);
        client
            .poll_with_hidden(&mut docs, &mut hidden, 0, &Settings::default())
            .unwrap();
        assert_eq!(client.diagnostics_for(&old).count(), 0);
        assert_eq!(client.diagnostics_for(&hidden[0]).count(), 1);
    }
}
