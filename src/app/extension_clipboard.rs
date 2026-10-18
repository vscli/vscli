//! Owner-scoped clipboard services share the existing bounded native worker.
use super::*;
use crate::{
    extensions::services::{NativeService, Operation},
    snippet_clipboard::Worker,
};
use anyhow::{Context as _, ensure};
use std::sync::atomic::Ordering;
#[cfg(test)]
use std::sync::{Arc, atomic::AtomicBool};

struct Active {
    service: NativeService,
    worker: Option<Worker>,
    ready: Option<std::result::Result<Value, String>>,
}
#[cfg(test)]
#[derive(Clone)]
struct Backend {
    busy: Arc<AtomicBool>,
    read: Vec<(String, Vec<String>)>,
    write: Vec<(String, Vec<String>)>,
}
#[derive(Default)]
pub(super) struct State {
    active: Option<Active>,
    #[cfg(test)]
    backend: Option<Backend>,
}
impl State {
    pub(super) fn pending(&self, session: u64, id: &str) -> bool {
        self.active.as_ref().is_some_and(|active| {
            active.service.request.session == session && active.service.id == id
        })
    }
}
impl App {
    pub(super) fn start_extension_clipboard(&mut self, service: NativeService) -> Result<()> {
        if self.extension_clipboard.active.is_some() {
            return Ok(());
        }
        let input = match &service.operation {
            Operation::ClipboardRead => None,
            Operation::ClipboardWrite(text) => Some(text.clone()),
            _ => anyhow::bail!("Invalid clipboard operation"),
        };
        #[cfg(test)]
        let occupied = self
            .extension_clipboard
            .backend
            .as_ref()
            .map_or_else(Worker::occupied, |backend| {
                backend.busy.load(Ordering::Acquire)
            });
        #[cfg(not(test))]
        let occupied = Worker::occupied();
        if occupied {
            return Ok(());
        }
        #[cfg(test)]
        let worker = if let Some(backend) = &self.extension_clipboard.backend {
            Worker::start_commands(
                backend.busy.clone(),
                if input.is_some() {
                    backend.write.clone()
                } else {
                    backend.read.clone()
                },
                input,
            )?
        } else {
            match input {
                Some(text) => Worker::start_write(text)?,
                None => Worker::start()?,
            }
        };
        #[cfg(not(test))]
        let worker = match input {
            Some(text) => Worker::start_write(text)?,
            None => Worker::start()?,
        };
        self.extension_clipboard.active = Some(Active {
            service,
            worker: Some(worker),
            ready: None,
        });
        Ok(())
    }
    pub(super) fn poll_extension_clipboard(&mut self) -> bool {
        let Some(mut active) = self.extension_clipboard.active.take() else {
            return false;
        };
        let current = !active.service.caller_retired.load(Ordering::Acquire)
            && active.service.fresh().is_ok()
            && self
                .extension_host
                .as_ref()
                .is_some_and(|host| host.session == active.service.request.session);
        if !current && let Some(worker) = &active.worker {
            worker.cancel();
        }
        let mut changed = false;
        if let Some(result) = active.worker.as_mut().and_then(Worker::try_finish) {
            active.worker = None;
            changed = true;
            let result = (|| -> Result<Value> {
                ensure!(
                    current,
                    "Original clipboard caller retired; an authorized system write may have completed"
                );
                let text = result?.context(
                    "System clipboard unavailable; install or configure a clipboard backend",
                )?;
                ensure!(
                    text.len() <= crate::snippet_clipboard::MAX_BYTES,
                    "Clipboard result exceeds 1 MiB"
                );
                match &active.service.operation {
                    Operation::ClipboardRead => Ok(Value::String(text)),
                    Operation::ClipboardWrite(input) => {
                        ensure!(text.is_empty(), "Invalid native clipboard write result");
                        self.clipboard = input.to_string();
                        self.clipboard_line = false;
                        Ok(Value::Null)
                    }
                    _ => anyhow::bail!("Invalid native clipboard operation"),
                }
            })();
            active.ready = Some(result.map_err(|error| format!("{error:#}")));
        }
        if active.worker.is_none()
            && self
                .extension_host
                .as_ref()
                .is_none_or(|host| host.session != active.service.request.session)
        {
            return true;
        }
        self.extension_clipboard.active = Some(active);
        changed
    }
    pub(super) fn complete_extension_clipboard(
        &mut self,
        host: &mut crate::extensions::Client,
    ) -> bool {
        let Some(mut active) = self.extension_clipboard.active.take() else {
            return false;
        };
        let Some(ready) = active.ready.take() else {
            self.extension_clipboard.active = Some(active);
            return false;
        };
        if host.session != active.service.request.session
            || host
                .service()
                .is_none_or(|service| service.id != active.service.id)
        {
            active.ready = Some(ready);
            self.extension_clipboard.active = Some(active);
            return false;
        }
        let result = ready.clone().map_err(anyhow::Error::msg).and_then(|value| {
            host.service_valid(&active.service)?;
            Ok(value)
        });
        match host.answer_service(&active.service, result) {
            Ok(()) => true,
            Err(error) => {
                self.message = format!("Clipboard receipt retained: {error:#}");
                active.ready = Some(ready);
                self.extension_clipboard.active = Some(active);
                false
            }
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    const SEED: &str = "seed 猫🙂\r\n";
    const VALUE: &str = "λ🙂 clipboard\r\nsecond\r\n";
    fn until(app: &mut App, check: impl Fn(&App) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            app.poll();
            if check(app) {
                return;
            }
            assert!(Instant::now() < deadline, "{}", app.message);
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn ready(root: &Path, backend: Backend) -> App {
        let file = root.join("original.txt");
        std::fs::write(&file, "original 猫🙂\r\n").unwrap();
        let mut app = App::new(root.into(), crate::keys::Profile::Linux);
        app.open(&file).unwrap();
        app.doc_mut().insert("dirty ", true);
        app.extension_clipboard.backend = Some(backend);
        let package =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/clipboard-extension");
        app.start_extension_packages(vec![crate::extensions::Package::read(&package).unwrap()])
            .unwrap();
        until(&mut app, |app| {
            app.extension_host.as_ref().is_some_and(|host| host.ready)
        });
        app
    }
    fn commands(script: &str, path: &Path) -> Vec<(String, Vec<String>)> {
        vec![(
            "/bin/sh".into(),
            vec![
                "-c".into(),
                script.into(),
                "fixture".into(),
                path.to_str().unwrap().into(),
            ],
        )]
    }
    #[test]
    fn public_clipboard_real_child_receipts_and_eight_fifo_reads_preserve_native_dirty_identity_history_and_disk()
     {
        for parallel in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("clipboard");
            std::fs::write(&path, SEED).unwrap();
            let backend = Backend {
                busy: Arc::new(AtomicBool::new(false)),
                read: commands("cat \"$1\"", &path),
                write: commands("cat > \"$1\"", &path),
            };
            let mut app = ready(root.path(), backend);
            let id = app.doc().id;
            let dirty = app.doc().text.to_string();
            app.execute(
                if parallel {
                    "fixture.clipboard.parallel"
                } else {
                    "fixture.clipboard.roundtrip"
                },
                Value::Null,
            );
            until(&mut app, |app| {
                app.message
                    == if parallel {
                        "Parallel clipboard slots retained"
                    } else {
                        "Clipboard roundtrip complete"
                    }
            });
            assert_eq!(
                std::fs::read_to_string(&path).unwrap(),
                if parallel { SEED } else { VALUE }
            );
            if !parallel {
                assert_eq!(app.clipboard, VALUE);
                assert!(!app.clipboard_line);
            }
            assert_eq!(app.doc().id, id);
            assert_eq!(app.doc().text.to_string(), dirty);
            assert!(app.doc().dirty());
            app.execute("undo", Value::Null);
            assert_eq!(app.doc().text.to_string(), "original 猫🙂\r\n");
            app.execute("redo", Value::Null);
            assert_eq!(app.doc().text.to_string(), dirty);
            assert_eq!(
                std::fs::read_to_string(root.path().join("original.txt")).unwrap(),
                "original 猫🙂\r\n"
            );
            app.execute("vscli.extensions.stop", Value::Null);
        }
    }
    #[test]
    fn unavailable_clipboard_is_an_error_and_original_retirement_cancels_actual_work_without_borrowing_new_authority()
     {
        for retired in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let entered = root.path().join("entered");
            let busy = Arc::new(AtomicBool::new(false));
            let backend = if retired {
                Backend {
                    busy: busy.clone(),
                    read: Vec::new(),
                    write: commands("touch \"$1\"; exec sleep 2", &entered),
                }
            } else {
                Backend {
                    busy: busy.clone(),
                    read: Vec::new(),
                    write: Vec::new(),
                }
            };
            let mut app = ready(root.path(), backend);
            let original = app.doc().text.to_string();
            let id = app.doc().id;
            app.clipboard = "internal protected".into();
            app.execute(
                if retired {
                    "fixture.clipboard.retired"
                } else {
                    "fixture.clipboard.unavailable"
                },
                Value::Null,
            );
            if retired {
                until(&mut app, |app| {
                    app.extension_clipboard.active.is_some() && entered.exists()
                });
                app.extension_clipboard
                    .active
                    .as_ref()
                    .unwrap()
                    .service
                    .caller_retired
                    .store(true, Ordering::Release);
                assert!(busy.load(Ordering::Acquire));
                assert!(
                    app.extension_clipboard
                        .active
                        .as_ref()
                        .unwrap()
                        .worker
                        .is_some()
                );
            }
            until(&mut app, |app| {
                app.message
                    == if retired {
                        "Original clipboard caller retired"
                    } else {
                        "Clipboard backend unavailable"
                    }
            });
            assert!(app.extension_clipboard.active.is_none());
            assert!(!busy.load(Ordering::Acquire));
            assert_eq!(app.clipboard, "internal protected");
            assert_eq!(app.doc().text.to_string(), original);
            assert_eq!(app.doc().id, id);
            app.execute("undo", Value::Null);
            assert_eq!(app.doc().text.to_string(), "original 猫🙂\r\n");
            app.execute("vscli.extensions.stop", Value::Null);
        }
    }
}
