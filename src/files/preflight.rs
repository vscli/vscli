//! Original Explorer operation authority, captured and checked on workers.
use super::*;
use crate::resource_journal::{Capture, Control, FileOptions, Limits, Operation, Presence};
use anyhow::ensure;

pub(crate) struct Preflight {
    pub action: Action,
    capture: Capture,
}

impl Preflight {
    pub fn prepare(action: Action, control: &Control<'_>) -> Result<Self> {
        let operation = match &action {
            Action::CreateFile(path) | Action::CreateFolder(path) => Operation::Create {
                path: path.clone(),
                options: FileOptions {
                    folder: Presence::Value(matches!(action, Action::CreateFolder(_))),
                    ..Default::default()
                },
                contents: None,
            },
            Action::Rename { from, to } => Operation::Move {
                source: from.clone(),
                destination: to.clone(),
                options: FileOptions::default(),
            },
            Action::Trash(path) => Operation::Delete {
                path: path.clone(),
                options: FileOptions {
                    recursive: Presence::Value(true),
                    ..Default::default()
                },
            },
        };
        let capture = Capture::prepare(vec![operation], vec![], Limits::default(), control)?;
        let resource = |path: &PathBuf, present: bool| -> Result<PathBuf> {
            let resource = capture
                .resources()
                .iter()
                .find(|resource| &resource.requested == path)
                .context("Original file operand was not captured")?;
            ensure!(
                resource.present == present,
                "Original file operand existence changed"
            );
            ensure!(
                resource.missing_ancestors.is_empty(),
                "Original file parent is missing"
            );
            Ok(resource.canonical.clone())
        };
        let action = match action {
            Action::CreateFile(path) => Action::CreateFile(resource(&path, false)?),
            Action::CreateFolder(path) => Action::CreateFolder(resource(&path, false)?),
            Action::Rename { from, to } if from == to => Action::Rename {
                from: resource(&from, true)?,
                to: resource(&to, true)?,
            },
            Action::Rename { from, to } => Action::Rename {
                from: resource(&from, true)?,
                to: resource(&to, false)?,
            },
            Action::Trash(path) => Action::Trash(resource(&path, true)?),
        };
        capture.verify_originals(control)?;
        Ok(Self { action, capture })
    }
    pub fn verify(&self, control: &Control<'_>) -> Result<()> {
        self.capture.verify_originals(control)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{Arc, atomic::AtomicBool},
        time::{Duration, Instant},
    };
    #[test]
    fn changed_original_or_new_destination_cannot_be_renamed_after_review() {
        for destination_change in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let root = std::fs::canonicalize(directory.path()).unwrap();
            let from = root.join("original");
            let to = root.join("new");
            fs::write(&from, b"original").unwrap();
            let canceled = AtomicBool::new(false);
            let control = Control::new(&canceled, Instant::now() + Duration::from_secs(5));
            let prepared = Preflight::prepare(
                Action::Rename {
                    from: from.clone(),
                    to: to.clone(),
                },
                &control,
            )
            .unwrap();
            fs::write(
                if destination_change { &to } else { &from },
                b"foreign bytes",
            )
            .unwrap();
            let mut job = Job::start_preflighted(Arc::new(prepared));
            let result = loop {
                if let Some(result) = job.poll() {
                    break result;
                }
                assert!(Instant::now() < control.deadline());
                std::thread::sleep(Duration::from_millis(1));
            };
            assert!(result.is_err());
            assert_eq!(
                fs::read(if destination_change { &to } else { &from }).unwrap(),
                b"foreign bytes"
            );
            assert!(from.exists());
        }
    }
    #[test]
    fn missing_parent_and_existing_destination_refuse_without_filesystem_effects() {
        let directory = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        let original = root.join("original");
        fs::write(&original, b"original").unwrap();
        let canceled = AtomicBool::new(false);
        let control = Control::new(&canceled, Instant::now() + Duration::from_secs(5));
        assert!(Preflight::prepare(Action::CreateFile(original.clone()), &control).is_err());
        assert!(
            Preflight::prepare(Action::CreateFolder(root.join("absent/new")), &control).is_err()
        );
        assert!(!root.join("absent").exists());
        assert_eq!(fs::read(original).unwrap(), b"original");
    }
}
