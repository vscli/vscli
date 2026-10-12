//! A bounded original-provider cohort for native symbol navigation.
use super::{extension_services::Context, *};
use crate::lsp;
use crate::{
    extension_providers::{Kind, Provider},
    extensions::providers::{NavigationRankIntent, Ticket},
};
use anyhow::{Context as _, Result, bail, ensure};
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};
const LOCATIONS: usize = 512;
const BYTES: usize = 1024 * 1024;
const DEADLINE: Duration = Duration::from_secs(15);
struct Candidate {
    ticket: Ticket,
    item: Value,
    uri: String,
    full: lsp::Range,
    rank: usize,
}
struct Run {
    context: Context,
    epoch: u64,
    session: u64,
    kind: Kind,
    rank_intent: NavigationRankIntent,
    rank_request: Option<u64>,
    ranked: bool,
    waiting: VecDeque<Provider>,
    tickets: Vec<Ticket>,
    awaiting: std::collections::HashSet<u64>,
    candidates: Vec<Candidate>,
    bytes: usize,
    failed: usize,
    started: Instant,
    published: bool,
    extra: Value,
    loading_notice: String,
}
#[derive(Default)]
pub(super) struct State {
    run: Option<Run>,
}
impl App {
    fn navigation_current(&self) -> bool {
        self.provider_navigation.run.as_ref().is_some_and(|run| {
            run.context.same_editor(self)
                && self.active_document().is_some_and(|doc| {
                    self.extension_host.as_ref().is_some_and(|host| {
                        host.session == run.session
                            && host.navigation_registry_current(run.epoch)
                            && host.navigation_rank_current(&run.rank_intent, doc)
                            && run
                                .tickets
                                .iter()
                                .all(|ticket| host.provider_ticket_current(ticket, doc))
                            && run.waiting.iter().all(|provider| {
                                host.provider_registration_matches(provider, run.epoch, doc)
                            })
                    })
                })
        })
    }
    pub(super) fn cancel_provider_navigation(&mut self) {
        if let Some(run) = self.provider_navigation.run.take()
            && let Some(host) = &mut self.extension_host
        {
            if let Some(id) = run.rank_request {
                host.cancel_navigation_rank(id);
            }
            // Completed location lists own no lazy item handles. Cancel only
            // still occupied callbacks, keeping the bounded transport queue
            // available for the current wave even after a 128-provider cohort.
            for ticket in run.tickets {
                if host.language_callback_occupied(&ticket) || run.awaiting.contains(&ticket.id) {
                    let _ = host.cancel_language_provider(&ticket);
                }
            }
        }
    }
    pub(super) fn retire_stale_provider_navigation(&mut self) -> bool {
        if self.provider_navigation.run.is_some() && !self.navigation_current() {
            // A newer input/registry operation already owns presentation. Late
            // obsolete callbacks cannot replace its notice or publish a picker.
            let announce = self
                .provider_navigation
                .run
                .as_ref()
                .is_some_and(|run| self.message == run.loading_notice);
            self.cancel_provider_navigation();
            if announce {
                self.message = "Extension navigation retired: original source changed".into();
            }
            true
        } else {
            false
        }
    }
    pub(super) fn advance_provider_navigation_interaction(&mut self) {
        if self.navigation_current()
            && let Some(run) = &mut self.provider_navigation.run
        {
            run.context.accept_next_interaction();
        }
    }
    pub(super) fn request_provider_navigation(&mut self, kind: Kind, extra: Value) -> bool {
        if !kind.is_navigation()
            || !self
                .extension_host
                .as_ref()
                .is_some_and(|host| host.navigation_family_registered(kind))
        {
            return false;
        }
        self.cancel_extension_provider();
        let result = (|| -> Result<()> {
            Context::check_budget(self)?;
            ensure!(
                self.focus == Focus::Editor && self.prompt.is_none() && self.modal.is_none(),
                "Focus the editor before requesting extension navigation"
            );
            let context = Context::capture(self);
            let host = self
                .extension_host
                .as_mut()
                .context("Extension host stopped")?;
            let rank_intent = host.capture_navigation_rank(
                kind,
                &self.documents,
                &self.hidden_documents,
                self.active,
            )?;
            let loading_notice = format!("Loading extension navigation: {}", kind.method());
            self.provider_navigation.run = Some(Run {
                context,
                epoch: host.navigation_registry_epoch(),
                session: host.session,
                kind,
                rank_intent,
                rank_request: None,
                ranked: false,
                waiting: VecDeque::new(),
                tickets: Vec::new(),
                awaiting: Default::default(),
                candidates: Vec::new(),
                bytes: 0,
                failed: 0,
                started: Instant::now(),
                published: false,
                extra,
                loading_notice: loading_notice.clone(),
            });
            self.dispatch_provider_navigation()?;
            self.message = loading_notice;
            Ok(())
        })();
        if let Err(error) = result {
            self.cancel_provider_navigation();
            self.message = format!("Extension navigation rejected: {error:#}");
        }
        true
    }
    fn dispatch_provider_navigation(&mut self) -> Result<bool> {
        if !self.navigation_current() {
            bail!("Original navigation source changed");
        }
        let mut changed = false;
        loop {
            let host = self
                .extension_host
                .as_mut()
                .context("Extension host stopped")?;
            let run = self
                .provider_navigation
                .run
                .as_mut()
                .context("Navigation retired")?;
            if !run.ranked {
                if run.rank_request.is_none() && host.navigation_rank_available() {
                    let doc = self
                        .documents
                        .get(self.active)
                        .context("Navigation source closed")?;
                    run.rank_request = Some(host.request_navigation_rank(&run.rank_intent, doc)?);
                    changed = true;
                }
                break;
            }
            if run.published || run.waiting.is_empty() || !host.language_provider_capacity() {
                break;
            }
            let provider = run.waiting.front().unwrap();
            host.sync_configuration(&self.settings)?;
            let ticket = host.request_language_provider_from(
                provider,
                &self.documents,
                &self.hidden_documents,
                self.active,
                run.extra.clone(),
            )?;
            run.waiting.pop_front();
            run.awaiting.insert(ticket.id);
            run.tickets.push(ticket);
            changed = true;
        }
        Ok(changed)
    }
    pub(super) fn navigation_reply_owned(&self, ticket: &Ticket) -> bool {
        self.provider_navigation.run.as_ref().is_some_and(|run| {
            run.session == ticket.session
                && run.tickets.iter().any(|original| {
                    original.id == ticket.id
                        && original.provider.id == ticket.provider.id
                        && original.provider.owner == ticket.provider.owner
                })
        })
    }
    pub(super) fn provider_navigation_reply(
        &mut self,
        ticket: Ticket,
        result: Result<Value, String>,
    ) -> Result<()> {
        ensure!(
            self.navigation_current(),
            "Original navigation source changed"
        );
        let run = self
            .provider_navigation
            .run
            .as_mut()
            .context("Navigation retired")?;
        if !run.awaiting.remove(&ticket.id) {
            return Ok(());
        }
        let value = match result {
            Ok(value) => value,
            Err(_) => {
                run.failed += 1;
                return Ok(());
            }
        };
        if value.is_null() {
            return Ok(());
        }
        let rank = run
            .tickets
            .iter()
            .position(|original| original.id == ticket.id)
            .context("Original navigation ticket absent")?;
        let values = value
            .as_array()
            .context("Invalid extension navigation locations")?;
        ensure!(
            run.candidates
                .len()
                .checked_add(values.len())
                .is_some_and(|count| count <= LOCATIONS),
            "Extension navigation aggregate exceeds 512 locations"
        );
        let bytes = serde_json::to_vec(&value)?.len();
        ensure!(
            run.bytes
                .checked_add(bytes)
                .is_some_and(|bytes| bytes <= BYTES),
            "Extension navigation aggregate exceeds 1 MiB"
        );
        // Validate the entire response before storing any candidate.
        let staged = values
            .iter()
            .map(|item| -> Result<_> {
                let uri = item["uri"]
                    .as_str()
                    .or(item["targetUri"].as_str())
                    .context("Missing location URI")?;
                ensure!(
                    uri.len() <= 4096 && !uri.contains('\0'),
                    "Navigation URI exceeds bounds"
                );
                let full: lsp::Range = serde_json::from_value(
                    item.get("range")
                        .or(item.get("targetRange"))
                        .context("Missing location range")?
                        .clone(),
                )?;
                let selection: lsp::Range = serde_json::from_value(
                    item.get("targetSelectionRange")
                        .or(item.get("range"))
                        .context("Missing location selection")?
                        .clone(),
                )?;
                ensure!(
                    [full.start, full.end, selection.start, selection.end]
                        .iter()
                        .all(|position| position.line <= u32::MAX as usize
                            && position.character <= u32::MAX as usize)
                        && (full.start.line, full.start.character)
                            <= (selection.start.line, selection.start.character)
                        && (selection.start.line, selection.start.character)
                            <= (selection.end.line, selection.end.character)
                        && (selection.end.line, selection.end.character)
                            <= (full.end.line, full.end.character),
                    "Invalid navigation target range"
                );
                Ok(Candidate {
                    ticket: ticket.clone(),
                    item: item.clone(),
                    uri: uri.into(),
                    full,
                    rank,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        run.bytes += bytes;
        run.candidates.extend(staged);
        Ok(())
    }
    pub(super) fn poll_provider_navigation(&mut self) -> bool {
        let ranks = self
            .extension_host
            .as_mut()
            .map(|host| host.take_navigation_ranks())
            .unwrap_or_default();
        let mut rename_changed = false;
        for rank in ranks {
            if self.rename_rank_owned(rank.id) {
                rename_changed = true;
                if let Err(error) = self.rename_rank_reply(rank) {
                    self.cancel_rename();
                    self.message = format!("Rename: {error:#}");
                }
                continue;
            }
            if !self
                .provider_navigation
                .run
                .as_ref()
                .is_some_and(|run| run.rank_request == Some(rank.id))
            {
                continue;
            }
            if !self.navigation_current() {
                return self.retire_stale_provider_navigation();
            }
            match rank.result {
                Ok(providers) if providers.is_empty() => {
                    // Empty authoritative matching does not suppress the real
                    // native LSP provider. Keep the same source/currentness
                    // fence, and bypass optional-host selection recursion.
                    let run = self.provider_navigation.run.take().unwrap();
                    self.language_request_native(run.kind.method(), run.extra);
                    return true;
                }
                Ok(providers) => {
                    let run = self.provider_navigation.run.as_mut().unwrap();
                    run.waiting = providers.into();
                    run.ranked = true;
                }
                Err(error) => {
                    self.cancel_provider_navigation();
                    self.message = format!("Extension navigation matching: {error}");
                    return true;
                }
            }
        }
        let Some(run) = &self.provider_navigation.run else {
            return rename_changed;
        };
        if !self.navigation_current() {
            return self.retire_stale_provider_navigation();
        }
        if !run.published && run.started.elapsed() >= DEADLINE {
            self.cancel_provider_navigation();
            self.message = "Extension navigation retired: cohort deadline elapsed".into();
            return true;
        }
        let changed = match self.dispatch_provider_navigation() {
            Ok(changed) => changed,
            Err(error) => {
                self.cancel_provider_navigation();
                self.message = format!("Extension navigation: {error:#}");
                return true;
            }
        };
        let run = self.provider_navigation.run.as_mut().unwrap();
        if !run.ranked || run.published || !run.waiting.is_empty() || !run.awaiting.is_empty() {
            return changed;
        }
        run.published = true;
        run.candidates.sort_by(|a, b| {
            a.uri
                .cmp(&b.uri)
                .then_with(|| range_key(a.full).cmp(&range_key(b.full)))
                .then_with(|| a.rank.cmp(&b.rank))
        });
        run.candidates
            .dedup_by(|a, b| a.uri == b.uri && range_key(a.full) == range_key(b.full));
        let mut items = Vec::with_capacity(run.candidates.len());
        for candidate in &run.candidates {
            let range: lsp::Range = serde_json::from_value(
                candidate
                    .item
                    .get("targetSelectionRange")
                    .or(candidate.item.get("range"))
                    .unwrap()
                    .clone(),
            )
            .expect("validated navigation range");
            items.push(language::LanguageItem {
                label: format!(
                    "{}:{}:{}",
                    candidate.uri,
                    range.start.line + 1,
                    range.start.character + 1
                ),
                action: language::LanguageAction::Provider {
                    ticket: candidate.ticket.clone(),
                    item: candidate.item.clone(),
                },
            });
        }
        if items.is_empty() {
            self.message = if run.failed > 0 {
                "Extension navigation returned no locations; some providers failed"
            } else {
                "Extension navigation returned no locations"
            }
            .into();
        } else if items.len() == 1 {
            if let Err(error) = self.language_action(&items.remove(0).action) {
                self.cancel_provider_navigation();
                self.message = format!("Extension location: {error:#}");
            }
        } else {
            self.modal = Some(Modal::Language {
                title: " Extension Locations · Enter opens · Esc closes ".into(),
                items,
                selected: 0,
            });
        }
        true
    }
    pub(super) fn provider_navigation_context(&self, ticket: &Ticket) -> Option<Context> {
        (self.navigation_reply_owned(ticket) && self.navigation_current()).then(|| {
            self.provider_navigation
                .run
                .as_ref()
                .unwrap()
                .context
                .clone()
        })
    }
}
fn range_key(range: lsp::Range) -> (usize, usize, usize, usize) {
    (
        range.start.line,
        range.start.character,
        range.end.line,
        range.end.character,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    fn until(app: &mut App, predicate: impl Fn(&App) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            app.poll();
            if predicate(app) {
                return;
            }
            assert!(Instant::now() < deadline, "{}", app.message);
            std::thread::yield_now();
        }
    }
    fn fixture(root: &Path, body: &str) -> App {
        let input = root.join("input.cpp");
        std::fs::write(&input, "0123456789 猫🙂\r\nlast\r\n").unwrap();
        let package = root.join("providers");
        std::fs::create_dir(&package).unwrap();
        std::fs::write(
            package.join("package.json"),
            r#"{"publisher":"test","name":"navigation","version":"1","main":"index.cjs"}"#,
        )
        .unwrap();
        std::fs::write(package.join("index.cjs"), body).unwrap();
        let mut app = App::new(root.into(), crate::keys::Profile::Linux);
        app.open(&input).unwrap();
        app.start_extension_packages(vec![crate::extensions::Package::read(&package).unwrap()])
            .unwrap();
        until(&mut app, |app| {
            app.has_extension_provider(Kind::Implementation)
        });
        app
    }
    #[test]
    fn empty_authoritative_rank_enters_native_tail_once_without_recursion_or_edit() {
        let root = tempfile::tempdir().unwrap();
        let mut app = fixture(
            root.path(),
            r#"const v=require('vscode'),fs=require('node:fs'),path=require('node:path');
exports.activate=c=>{c.subscriptions.push(v.languages.registerImplementationProvider('cpp',{provideImplementation(){return []}}));
c.subscriptions.push(v.languages.registerDefinitionProvider('rust',{provideDefinition(){fs.writeFileSync(path.join(v.workspace.rootPath,'wrong-callback'),'called');return []}}));};"#,
        );
        let settings = root.path().join("settings.json");
        std::fs::write(&settings, r#"{"vscli.languageServer.enabled":false}"#).unwrap();
        app.configure_settings(Some(settings)).unwrap();
        app.lsp = None;
        app.doc_mut().insert("dirty ", false);
        let id = app.doc().id;
        let selections = app.doc().selections();
        let text = app.doc().text.to_string();
        let proof = app
            .doc()
            .capture_save(root.path().join("input.cpp"))
            .unwrap();
        assert!(
            app.extension_host
                .as_ref()
                .unwrap()
                .navigation_family_registered(Kind::Definition)
        );
        assert!(!app.has_extension_provider(Kind::Definition));
        app.execute("editor.action.revealDefinition", Value::Null);
        until(&mut app, |app| {
            app.message
                == "Language server not ready; use Language: Server Status or Language: Restart Server"
        });
        assert!(app.provider_navigation.run.is_none());
        assert!(app.modal.is_none());
        assert!(
            app.extension_host
                .as_ref()
                .unwrap()
                .navigation_rank_available()
        );
        assert_eq!(
            app.extension_host
                .as_ref()
                .unwrap()
                .language_provider_slots(),
            8
        );
        assert!(!root.path().join("wrong-callback").exists());
        let message = app.message.clone();
        for _ in 0..3 {
            app.poll();
            assert_eq!(app.message, message);
            assert!(app.provider_navigation.run.is_none());
        }
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().text.to_string(), text);
        assert_eq!(app.doc().selections(), selections);
        let current = app
            .doc()
            .capture_save(root.path().join("input.cpp"))
            .unwrap();
        assert_eq!(
            (current.text_epoch(), current.save_generation()),
            (proof.text_epoch(), proof.save_generation())
        );
        app.doc_mut().undo();
        assert_eq!(app.doc().text.to_string(), "0123456789 猫🙂\r\nlast\r\n");
        app.doc_mut().redo();
        assert_eq!(app.doc().text.to_string(), text);
        assert!(app.doc().dirty());
        assert_eq!(
            std::fs::read(root.path().join("input.cpp")).unwrap(),
            "0123456789 猫🙂\r\nlast\r\n".as_bytes()
        );
    }
    #[test]
    fn all_nine_providers_dispatch_in_bounded_waves_and_picker_reveals_collapsed_original_source() {
        let root = tempfile::tempdir().unwrap();
        let mut app = fixture(
            root.path(),
            r#"const v=require('vscode'),fs=require('node:fs'),path=require('node:path');
exports.activate=c=>{for(let i=0;i<9;i++)c.subscriptions.push(v.languages.registerImplementationProvider('cpp',{
 provideImplementation(doc){fs.appendFileSync(path.join(v.workspace.rootPath,'entered'),i+'\n');return new v.Location(doc.uri,new v.Range(0,i,0,i+1));}
}));};"#,
        );
        app.doc_mut().insert("!", false);
        let id = app.doc().id;
        let text = app.doc().text.to_string();
        let proof = app
            .doc()
            .capture_save(root.path().join("input.cpp"))
            .unwrap();
        app.execute("editor.action.goToImplementation", Value::Null);
        until(
            &mut app,
            |app| matches!(&app.modal,Some(Modal::Language{items,..}) if items.len()==9),
        );
        let trace = std::fs::read_to_string(root.path().join("entered")).unwrap();
        assert_eq!(
            trace
                .lines()
                .map(|line| line.parse::<usize>().unwrap())
                .collect::<Vec<_>>(),
            (0..9).rev().collect::<Vec<_>>()
        );
        assert_eq!(app.doc().text.to_string(), text);
        let current = app
            .doc()
            .capture_save(root.path().join("input.cpp"))
            .unwrap();
        assert_eq!(
            (current.text_epoch(), current.save_generation()),
            (proof.text_epoch(), proof.save_generation())
        );
        for _ in 0..2 {
            app.event(Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)));
        }
        app.event(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        until(&mut app, |app| app.modal.is_none());
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().cursor, 2);
        assert!(app.doc().anchor.is_none());
        assert!(app.doc().dirty());
        app.execute("undo", Value::Null);
        assert_eq!(app.doc().text.to_string(), "0123456789 猫🙂\r\nlast\r\n");
        app.execute("redo", Value::Null);
        assert_eq!(app.doc().text.to_string(), text);
        assert_eq!(
            std::fs::read(root.path().join("input.cpp")).unwrap(),
            "0123456789 猫🙂\r\nlast\r\n".as_bytes()
        );
    }
    #[test]
    fn held_navigation_edit_undo_and_registry_retirement_never_publishes_or_navigates() {
        let root = tempfile::tempdir().unwrap();
        let mut app = fixture(
            root.path(),
            r#"const v=require('vscode'),fs=require('node:fs'),path=require('node:path');let held=[];
exports.activate=c=>{let provider=v.languages.registerImplementationProvider('cpp',{
 provideImplementation(doc){fs.writeFileSync(path.join(v.workspace.rootPath,'entered'),'yes');return new Promise(resolve=>held.push(()=>resolve(new v.Location(doc.uri,new v.Range(0,3,0,4)))));}
});c.subscriptions.push(provider,v.commands.registerCommand('test.release',()=>{for(const finish of held.splice(0))finish();}),v.commands.registerCommand('test.retire',()=>provider.dispose()));};"#,
        );
        let original = app.doc().text.to_string();
        let id = app.doc().id;
        app.execute("editor.action.goToImplementation", Value::Null);
        until(&mut app, |_| root.path().join("entered").exists());
        let ticket = app.provider_navigation.run.as_ref().unwrap().tickets[0].clone();
        let epoch = app.doc().text_epoch();
        app.doc_mut().insert("newer ", false);
        app.doc_mut().undo();
        assert_eq!(app.doc().text.to_string(), original);
        assert!(app.doc().text_epoch() > epoch);
        app.message = "Newer user operation remains authoritative".into();
        app.poll();
        assert_eq!(app.message, "Newer user operation remains authoritative");
        assert!(app.provider_navigation.run.is_none());
        assert_eq!(
            app.extension_host
                .as_ref()
                .unwrap()
                .language_provider_slots(),
            7
        );
        app.execute("test.retire", Value::Null);
        until(&mut app, |app| {
            !app.has_extension_provider(Kind::Implementation)
        });
        app.execute("test.release", Value::Null);
        until(&mut app, |app| {
            app.extension_host
                .as_ref()
                .unwrap()
                .language_provider_slots()
                == 8
        });
        assert!(!app.navigation_reply_owned(&ticket));
        assert!(app.modal.is_none());
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().cursor, 0);
        assert_eq!(app.doc().text.to_string(), original);
        app.execute("redo", Value::Null);
        assert_eq!(app.doc().text.to_string(), format!("newer {original}"));
        assert_eq!(
            std::fs::read(root.path().join("input.cpp")).unwrap(),
            original.as_bytes()
        );
    }
}
