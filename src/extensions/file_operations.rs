use super::*;
use crate::files::Action;
use anyhow::ensure;
use std::collections::VecDeque;
const MAX_EVENTS: usize = 32;
const MAX_BYTES: usize = 1024 * 1024;
const MAX_EVENT_BYTES: usize = 64 * 1024;
#[derive(Default)]
pub(super) struct Queue {
    next: u64,
    bytes: usize,
    events: VecDeque<(Value, usize)>,
}
fn uri(path: &Path) -> Result<String> {
    ensure!(
        path.is_absolute() && path.as_os_str().len() <= 4096,
        "Invalid native file operation path"
    );
    Ok(url::Url::from_file_path(path)
        .map_err(|_| anyhow::anyhow!("Invalid file operation URI"))?
        .to_string())
}
impl Client {
    pub(crate) fn file_operation_capacity(&self) -> bool {
        self.file_operations.events.len() < MAX_EVENTS
            && self.file_operations.bytes <= MAX_BYTES - MAX_EVENT_BYTES
            && self.file_operations.next < (1u64 << 53) - 1
    }
    pub(crate) fn queue_file_operation(&mut self, action: &Action) -> Result<()> {
        ensure!(
            self.file_operation_capacity(),
            "Native file operation notification capacity exhausted"
        );
        let (kind, files) = match action {
            Action::CreateFile(path) | Action::CreateFolder(path) => {
                ("create", json!([{"target":uri(path)?}]))
            }
            Action::Rename { from, to } => {
                ("rename", json!([{"source":uri(from)?,"target":uri(to)?}]))
            }
            Action::Trash(path) => ("delete", json!([{"target":uri(path)?}])),
        };
        let next = self.file_operations.next + 1;
        let frame = json!({"session":self.session,"sequence":next,"kind":kind,"files":files});
        let bytes = serde_json::to_vec(&frame)?.len();
        ensure!(
            bytes <= MAX_EVENT_BYTES,
            "Native file operation event exceeds budget"
        );
        self.file_operations
            .events
            .try_reserve(1)
            .context("Cannot retain native file operation event")?;
        self.file_operations.events.push_back((frame, bytes));
        self.file_operations.next = next;
        self.file_operations.bytes += bytes;
        Ok(())
    }
    pub(super) fn flush_file_operations(&mut self) -> Result<()> {
        while let Some((event, bytes)) = self.file_operations.events.front() {
            if let Err(error) = self
                .process
                .send(json!({"method":"fileOperation","params":event}))
            {
                if error
                    .downcast_ref::<crate::transport::OutputBackpressure>()
                    .is_some()
                {
                    return Ok(());
                }
                return Err(error);
            }
            self.file_operations.bytes -= bytes;
            self.file_operations.events.pop_front();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn client(root: &Path) -> Client {
        Client::start(
            "node",
            &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/command-extension"),
            root,
            &[Document::from_text("original")],
            0,
            &Settings::default(),
        )
        .unwrap()
    }
    #[test]
    fn event_queue_is_bounded_and_invalid_resources_do_not_consume_sequence_or_capacity() {
        let root = tempfile::tempdir().unwrap();
        let mut client = client(root.path());
        assert!(
            client
                .queue_file_operation(&Action::CreateFile("relative".into()))
                .is_err()
        );
        assert_eq!(client.file_operations.next, 0);
        for _ in 0..MAX_EVENTS {
            client
                .queue_file_operation(&Action::CreateFolder(root.path().join("folder")))
                .unwrap();
        }
        assert!(!client.file_operation_capacity());
        let bytes = client.file_operations.bytes;
        assert!(
            client
                .queue_file_operation(&Action::Trash(root.path().join("folder")))
                .is_err()
        );
        assert_eq!(client.file_operations.events.len(), MAX_EVENTS);
        assert_eq!(client.file_operations.bytes, bytes);
        assert_eq!(client.file_operations.next, MAX_EVENTS as u64);
    }
    #[test]
    fn actual_writer_backpressure_retains_original_operation_frame_without_reset_or_duplication() {
        let root = tempfile::tempdir().unwrap();
        let mut client = client(root.path());
        let (entered, release) = client.fixture_gate_native_writer().unwrap();
        client.fixture_native_output_barrier().unwrap();
        entered.recv_timeout(Duration::from_secs(5)).unwrap();
        let mut full = false;
        for _ in 0..256 {
            if client.fixture_native_output_probe().is_err() {
                full = true;
                break;
            }
        }
        assert!(full, "actual native writer queue must be full");
        client
            .queue_file_operation(&Action::Rename {
                from: root.path().join("a 猫.cpp"),
                to: root.path().join("b.cpp"),
            })
            .unwrap();
        let original = client.file_operations.events.front().unwrap().0.clone();
        let bytes = client.file_operations.bytes;
        client.flush_file_operations().unwrap();
        assert_eq!(client.file_operations.events.front().unwrap().0, original);
        assert_eq!(client.file_operations.bytes, bytes);
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !client.file_operations.events.is_empty() {
            client.flush_file_operations().unwrap();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(client.file_operations.next, 1);
        assert_eq!(client.file_operations.bytes, 0);
    }
}
