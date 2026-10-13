//! Worker-only profile storage. The caller must validate its original profile
//! generation before authorizing the existing joined resource worker.
use crate::{
    persistence,
    resource_worker::{Preparation, Transaction},
};
use anyhow::{Context, Result, ensure};
use serde_json::{Map, Value};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
pub const KEY: &str = "file.particpants.additionalEdits";
const MAX_BYTES: usize = 64 * 1024;
#[derive(Clone)]
pub struct Store {
    path: PathBuf,
}
impl Store {
    pub fn new(profile: &Path) -> Result<Self> {
        ensure!(
            profile.is_absolute(),
            "File-participant preferences require an absolute native profile"
        );
        let path = profile.join("file-participant-preferences.json");
        persistence::validate_path(&path)?;
        Ok(Self { path })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    fn target(&self) -> Result<persistence::Target> {
        persistence::Target::open(
            &self.path,
            persistence::Options {
                max_bytes: MAX_BYTES,
                parent: persistence::ParentPolicy::Create,
                expected: persistence::Expected::Captured,
                models: Vec::new(),
            },
        )
    }
    pub fn load(&self, control: &Preparation) -> Result<Option<bool>> {
        control.check()?;
        let mut target = self.target()?;
        control.check()?;
        let values = decode(target.read_bytes(MAX_BYTES)?)?;
        control.check()?;
        choice(&values)
    }
    pub fn write(&self, choice: Option<bool>, profile: u64) -> Write {
        Write {
            store: self.clone(),
            choice,
            profile,
        }
    }
}
fn decode(bytes: Option<Vec<u8>>) -> Result<Map<String, Value>> {
    let Some(bytes) = bytes else {
        return Ok(Map::new());
    };
    let value: Value = serde_json::from_slice(&bytes)?;
    let fields = value
        .as_object()
        .context("Invalid file-participant preferences")?;
    ensure!(
        fields.len() == 2 && value["schema"] == 1,
        "Unsupported file-participant preferences schema"
    );
    let values = value["values"]
        .as_object()
        .context("Invalid file-participant preference values")?;
    ensure!(
        values.len() <= 256,
        "File-participant preferences exceed 256 keys"
    );
    Ok(values.clone())
}
fn choice(values: &Map<String, Value>) -> Result<Option<bool>> {
    values
        .get(KEY)
        .map(|value| {
            value
                .as_bool()
                .context("Stored file-participant preview choice must be boolean")
        })
        .transpose()
}
pub struct Write {
    store: Store,
    choice: Option<bool>,
    profile: u64,
}
pub struct Prepared {
    write: Option<persistence::Prepared>,
    profile: u64,
    choice: Option<bool>,
}
#[derive(Clone, Debug)]
pub struct Info {
    pub path: PathBuf,
    pub canonical_path: PathBuf,
    pub profile: u64,
    pub choice: Option<bool>,
}
pub struct Receipt {
    pub profile: u64,
    pub choice: Option<bool>,
    pub commit: Option<persistence::Commit>,
}
impl Transaction for Write {
    type Prepared = Prepared;
    type Info = Arc<Info>;
    type Receipt = Receipt;
    fn prepare(self, control: &Preparation) -> Result<(Prepared, Arc<Info>)> {
        control.check()?;
        let mut target = self.store.target()?;
        control.check()?;
        let mut values = decode(target.read_bytes(MAX_BYTES)?)?;
        let previous = choice(&values)?;
        let info = Arc::new(Info {
            path: self.store.path.clone(),
            canonical_path: target.info().canonical_path,
            profile: self.profile,
            choice: self.choice,
        });
        if previous == self.choice {
            control.check()?;
            return Ok((
                Prepared {
                    write: None,
                    profile: self.profile,
                    choice: self.choice,
                },
                info,
            ));
        }
        if let Some(choice) = self.choice {
            values.insert(KEY.into(), Value::Bool(choice));
        } else {
            values.remove(KEY);
        }
        let bytes = serde_json::to_vec(&serde_json::json!({"schema":1,"values":values}))?;
        ensure!(
            bytes.len() <= MAX_BYTES,
            "File-participant preferences exceed 64 KiB"
        );
        control.check()?;
        let write = target.stage_bytes(&bytes)?;
        control.check()?;
        Ok((
            Prepared {
                write: Some(write),
                profile: self.profile,
                choice: self.choice,
            },
            info,
        ))
    }
    fn commit(prepared: Prepared) -> Result<Receipt> {
        let commit = prepared
            .write
            .map(persistence::Prepared::commit)
            .transpose()?;
        Ok(Receipt {
            profile: prepared.profile,
            choice: prepared.choice,
            commit,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource_worker::{Event, Outcome, Worker};
    use serde_json::json;
    use std::time::{Duration, Instant};
    struct Read(Store);
    impl Transaction for Read {
        type Prepared = Option<bool>;
        type Info = Option<bool>;
        type Receipt = Option<bool>;
        fn prepare(self, control: &Preparation) -> Result<(Option<bool>, Option<bool>)> {
            let value = self.0.load(control)?;
            Ok((value, value))
        }
        fn commit(value: Option<bool>) -> Result<Option<bool>> {
            Ok(value)
        }
    }
    fn prepared<T: Transaction>(worker: &mut Worker<T>) -> T::Info {
        let deadline = Instant::now() + Duration::from_secs(6);
        loop {
            if let Some(event) = worker.poll() {
                return match event {
                    Event::Prepared { info, .. } => info,
                    Event::Finished { result, .. } => {
                        panic!("Preparation failed: {}", result.err().unwrap_or_default())
                    }
                };
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn finished<T: Transaction>(worker: &mut Worker<T>) -> Result<Outcome<T::Receipt>, String> {
        let deadline = Instant::now() + Duration::from_secs(6);
        loop {
            if let Some(event) = worker.poll() {
                return match event {
                    Event::Finished { result, .. } => result,
                    Event::Prepared { .. } => panic!("Unexpected preparation"),
                };
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn load(store: &Store) -> Option<bool> {
        let mut worker = Worker::default();
        worker
            .start(
                1,
                Read(store.clone()),
                Instant::now() + Duration::from_secs(5),
            )
            .unwrap();
        let value = prepared(&mut worker);
        worker.authorize(1).unwrap();
        assert!(matches!(finished(&mut worker), Ok(Outcome::Committed(_))));
        value
    }
    fn write(store: &Store, value: Option<bool>) -> Receipt {
        let mut worker = Worker::default();
        worker
            .start(
                1,
                store.write(value, 7),
                Instant::now() + Duration::from_secs(5),
            )
            .unwrap();
        let info = prepared(&mut worker);
        assert_eq!(info.profile, 7);
        assert_eq!(info.choice, value);
        worker.authorize(1).unwrap();
        match finished(&mut worker).unwrap() {
            Outcome::Committed(receipt) => receipt,
            _ => panic!("Expected committed preference"),
        }
    }
    #[test]
    fn remember_preview_apply_and_reset_roundtrip_preserve_unrelated_profile_state() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::new(directory.path()).unwrap();
        std::fs::write(
            store.path(),
            serde_json::to_vec(&json!({"schema":1,"values":{"unrelated":{"nested":[1,"猫🙂"]}}}))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(load(&store), None);
        for choice in [Some(true), Some(false), None] {
            let receipt = write(&store, choice);
            assert_eq!(receipt.profile, 7);
            assert_eq!(receipt.choice, choice);
            assert!(receipt.commit.is_some());
            assert_eq!(load(&store), choice);
            let value: Value =
                serde_json::from_slice(&std::fs::read(store.path()).unwrap()).unwrap();
            assert_eq!(value["values"]["unrelated"], json!({"nested":[1,"猫🙂"]}));
        }
        assert!(write(&store, None).commit.is_none());
    }
    #[test]
    fn canceled_preparation_keeps_original_bytes_and_actual_slot_until_join() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::new(directory.path()).unwrap();
        let original = b"{\"schema\":1,\"values\":{\"file.particpants.additionalEdits\":true}}";
        std::fs::write(store.path(), original).unwrap();
        let mut worker = Worker::default();
        worker
            .start(
                1,
                store.write(None, 4),
                Instant::now() + Duration::from_secs(5),
            )
            .unwrap();
        prepared(&mut worker);
        worker.retire(1);
        assert!(worker.busy());
        assert!(
            worker
                .start(
                    2,
                    store.write(Some(false), 4),
                    Instant::now() + Duration::from_secs(5)
                )
                .is_err()
        );
        assert!(matches!(finished(&mut worker), Ok(Outcome::Rejected)));
        assert!(!worker.busy());
        assert_eq!(std::fs::read(store.path()).unwrap(), original);
    }
    #[test]
    fn corrupt_oversized_and_wrong_typed_choices_are_never_replaced() {
        for bytes in [
            b"broken".to_vec(),
            serde_json::to_vec(&json!({"schema":2,"values":{}})).unwrap(),
            serde_json::to_vec(&json!({"schema":1,"values":{KEY:"false"}})).unwrap(),
            vec![b'x'; MAX_BYTES + 1],
        ] {
            let directory = tempfile::tempdir().unwrap();
            let store = Store::new(directory.path()).unwrap();
            std::fs::write(store.path(), &bytes).unwrap();
            let mut worker = Worker::default();
            worker
                .start(
                    1,
                    store.write(None, 1),
                    Instant::now() + Duration::from_secs(5),
                )
                .unwrap();
            assert!(finished(&mut worker).is_err());
            assert_eq!(std::fs::read(store.path()).unwrap(), bytes);
        }
    }
    #[test]
    fn stale_destination_after_preparation_cannot_overwrite_external_new_state() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::new(directory.path()).unwrap();
        write(&store, Some(true));
        let mut worker = Worker::default();
        worker
            .start(
                1,
                store.write(None, 1),
                Instant::now() + Duration::from_secs(5),
            )
            .unwrap();
        prepared(&mut worker);
        let external = b"{\"schema\":1,\"values\":{\"external\":\"new\"}}";
        std::fs::write(store.path(), external).unwrap();
        worker.authorize(1).unwrap();
        assert!(finished(&mut worker).is_err());
        assert_eq!(std::fs::read(store.path()).unwrap(), external);
    }
    #[cfg(unix)]
    #[test]
    fn final_symlink_and_nonregular_payloads_are_refused_without_touching_their_targets() {
        use std::os::unix::fs::symlink;
        for kind in ["symlink", "directory"] {
            let directory = tempfile::tempdir().unwrap();
            let store = Store::new(directory.path()).unwrap();
            let target = directory.path().join("target");
            std::fs::write(&target, b"original").unwrap();
            if kind == "symlink" {
                symlink(&target, store.path()).unwrap();
            } else {
                std::fs::create_dir(store.path()).unwrap();
            }
            let mut worker = Worker::default();
            worker
                .start(
                    1,
                    store.write(None, 1),
                    Instant::now() + Duration::from_secs(5),
                )
                .unwrap();
            assert!(finished(&mut worker).is_err());
            assert_eq!(std::fs::read(target).unwrap(), b"original");
        }
    }
}
