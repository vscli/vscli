//! Pure, bounded autosave scheduling. Disk work and save ownership belong to App.
use anyhow::{Result, ensure};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    time::{Duration, Instant},
};

pub const MAX_DELAY_MS: u32 = 86_400_000;
pub const DEFAULT_DELAY_MS: u32 = 1000;
const MODELS: usize = 128;
const PATH_BYTES: usize = 4096;
const TOTAL_PATH_BYTES: usize = 512 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Policy {
    #[default]
    Off,
    AfterDelay {
        delay_ms: u32,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelProof {
    pub id: u64,
    pub path: Option<PathBuf>,
    pub text_epoch: u64,
    pub save_generation: u64,
    pub dirty: bool,
    pub policy: Policy,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    proof: ModelProof,
    changed_at: Instant,
    suppressed: bool,
}
impl Entry {
    fn due(&self, now: Instant) -> bool {
        let Policy::AfterDelay { delay_ms } = self.proof.policy else {
            return false;
        };
        self.proof.dirty
            && self.proof.path.is_some()
            && !self.suppressed
            && now
                .checked_duration_since(self.changed_at)
                .is_some_and(|elapsed| elapsed >= Duration::from_millis(delay_ms.into()))
    }
}
#[derive(Default, Debug)]
pub struct Scheduler {
    entries: BTreeMap<u64, Entry>,
    last_selected: Option<u64>,
    observed_at: Option<Instant>,
}
impl Scheduler {
    /// Observe the complete retained inventory, including hidden file models.
    /// Validate everything before replacing any deadline or suppression proof.
    pub fn observe(&mut self, now: Instant, models: &[ModelProof]) -> Result<()> {
        ensure!(
            models.len() <= MODELS,
            "Autosave exceeds 128 retained models"
        );
        ensure!(
            self.observed_at.is_none_or(|previous| now >= previous),
            "Autosave clock moved backwards"
        );
        let mut ids = BTreeSet::new();
        let mut bytes = 0usize;
        for model in models {
            ensure!(
                ids.insert(model.id),
                "Autosave inventory has duplicate model IDs"
            );
            if let Policy::AfterDelay { delay_ms } = model.policy {
                ensure!(
                    delay_ms <= MAX_DELAY_MS,
                    "Autosave delay exceeds native 24-hour limit"
                );
            }
            if let Some(path) = &model.path {
                let name = path.as_os_str();
                ensure!(
                    !name.is_empty() && name.len() <= PATH_BYTES && path.file_name().is_some(),
                    "Autosave model path must contain 1–4096 bytes and a filename"
                );
                ensure!(
                    !name.as_encoded_bytes().contains(&0),
                    "Autosave model path contains NUL"
                );
                bytes += name.len();
                ensure!(
                    bytes <= TOTAL_PATH_BYTES,
                    "Autosave model paths exceed 512 KiB"
                );
            }
        }
        self.entries.retain(|id, _| ids.contains(id));
        for model in models {
            if let Some(entry) = self.entries.get_mut(&model.id) {
                if entry.proof == *model {
                    continue;
                }
                if entry.proof.path != model.path
                    || entry.proof.text_epoch != model.text_epoch
                    || entry.proof.policy != model.policy
                    || (!entry.proof.dirty && model.dirty)
                {
                    entry.changed_at = now;
                }
                // A committed older snapshot changes the save generation,
                // clearing suppression without restarting newer-text debounce.
                entry.suppressed = false;
                if entry.proof.path != model.path {
                    entry.proof.path.clone_from(&model.path);
                }
                entry.proof.text_epoch = model.text_epoch;
                entry.proof.save_generation = model.save_generation;
                entry.proof.dirty = model.dirty;
                entry.proof.policy = model.policy;
            } else {
                self.entries.insert(
                    model.id,
                    Entry {
                        proof: model.clone(),
                        changed_at: now,
                        suppressed: false,
                    },
                );
            }
        }
        self.observed_at = Some(now);
        Ok(())
    }
    /// Call only when the actual save worker is available. Returns metadata,
    /// never a text snapshot; App recaptures current text when dispatching.
    /// Selection reserves this exact proof, preventing repeated queued saves.
    pub fn take_due(&mut self, now: Instant) -> Option<ModelProof> {
        if self.observed_at.is_some_and(|previous| now < previous) {
            return None;
        }
        let eligible = |(_, entry): &(&u64, &Entry)| entry.due(now);
        let selected = self
            .entries
            .iter()
            .filter(eligible)
            .find(|(id, _)| self.last_selected.is_none_or(|previous| **id > previous))
            .or_else(|| self.entries.iter().find(eligible))
            .map(|(id, _)| *id)?;
        let entry = self.entries.get_mut(&selected).unwrap();
        entry.suppressed = true;
        self.last_selected = Some(selected);
        Some(entry.proof.clone())
    }
    /// A failed manual or automatic save suppresses only its exact live proof.
    /// A delayed old failure cannot disable newer edits or a fresh disk baseline.
    pub fn failed(&mut self, proof: &ModelProof) -> bool {
        let Some(entry) = self
            .entries
            .get_mut(&proof.id)
            .filter(|entry| entry.proof == *proof)
        else {
            return false;
        };
        entry.suppressed = true;
        true
    }
    /// Retire all scheduling proofs when the owning workspace/profile changes.
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn model(id: u64) -> ModelProof {
        ModelProof {
            id,
            path: Some(format!("model-{id}.cpp").into()),
            text_epoch: 1,
            save_generation: 0,
            dirty: true,
            policy: Policy::AfterDelay { delay_ms: 1000 },
        }
    }
    fn at(start: Instant, millis: u64) -> Instant {
        start + Duration::from_millis(millis)
    }
    #[test]
    fn unchanged_observations_do_not_restart_deadlines_and_edit_undo_does() {
        let start = Instant::now();
        let mut scheduler = Scheduler::default();
        let mut proof = model(1);
        scheduler
            .observe(start, std::slice::from_ref(&proof))
            .unwrap();
        scheduler
            .observe(at(start, 999), std::slice::from_ref(&proof))
            .unwrap();
        assert!(scheduler.take_due(at(start, 999)).is_none());
        assert_eq!(scheduler.take_due(at(start, 1000)), Some(proof.clone()));
        assert!(scheduler.take_due(at(start, 2000)).is_none());
        proof.text_epoch += 1;
        scheduler
            .observe(at(start, 2100), std::slice::from_ref(&proof))
            .unwrap();
        proof.text_epoch += 1; // Undo returns equal text but has a new epoch.
        scheduler
            .observe(at(start, 2500), std::slice::from_ref(&proof))
            .unwrap();
        assert!(scheduler.take_due(at(start, 3499)).is_none());
        assert_eq!(scheduler.take_due(at(start, 3500)), Some(proof));
    }
    #[test]
    fn held_worker_accumulates_no_snapshots_and_dispatches_latest_live_proof() {
        let start = Instant::now();
        let mut scheduler = Scheduler::default();
        let mut proof = model(1);
        scheduler
            .observe(start, std::slice::from_ref(&proof))
            .unwrap();
        let original = scheduler.take_due(at(start, 1000)).unwrap();
        // Actual worker is held: App continues observing but never calls take_due.
        for epoch in 2..=256 {
            proof.text_epoch = epoch;
            scheduler
                .observe(at(start, 1000 + epoch), std::slice::from_ref(&proof))
                .unwrap();
        }
        assert_eq!(scheduler.entries.len(), 1);
        assert!(!scheduler.failed(&original));
        assert!(scheduler.take_due(at(start, 2255)).is_none());
        assert_eq!(scheduler.take_due(at(start, 2256)), Some(proof));
        assert!(scheduler.take_due(at(start, 5000)).is_none());
    }
    #[test]
    fn fair_selection_covers_hidden_and_visible_models_despite_one_frequent_editor() {
        let start = Instant::now();
        let mut models: Vec<_> = (1..=MODELS as u64).rev().map(model).collect();
        for proof in &mut models {
            proof.policy = Policy::AfterDelay { delay_ms: 0 };
        }
        let mut scheduler = Scheduler::default();
        scheduler.observe(start, &models).unwrap();
        let mut served = Vec::new();
        for expected in 1..=MODELS as u64 {
            let selected = scheduler.take_due(start).unwrap();
            assert_eq!(selected.id, expected);
            served.push(selected.id);
            models.last_mut().unwrap().text_epoch += 1;
            scheduler.observe(start, &models).unwrap();
        }
        assert_eq!(served.len(), MODELS);
        assert_eq!(scheduler.take_due(start).unwrap().id, 1);
    }
    #[test]
    fn failure_suppression_is_exact_and_path_policy_epoch_or_save_change_retires_it() {
        let start = Instant::now();
        let mut proof = model(1);
        proof.policy = Policy::AfterDelay { delay_ms: 0 };
        let mut scheduler = Scheduler::default();
        scheduler
            .observe(start, std::slice::from_ref(&proof))
            .unwrap();
        assert!(scheduler.failed(&proof));
        assert!(scheduler.take_due(start).is_none());
        for change in 0..4 {
            let old = proof.clone();
            match change {
                0 => proof.path = Some("different.cpp".into()),
                1 => proof.text_epoch += 1,
                2 => proof.save_generation += 1,
                _ => proof.policy = Policy::AfterDelay { delay_ms: 1 },
            }
            scheduler
                .observe(start, std::slice::from_ref(&proof))
                .unwrap();
            assert!(!scheduler.failed(&old));
            assert_eq!(scheduler.take_due(at(start, 1)), Some(proof.clone()));
            assert!(scheduler.failed(&proof));
            assert!(scheduler.take_due(at(start, 2)).is_none());
        }
    }
    #[test]
    fn old_commit_clears_suppression_without_delaying_newer_dirty_text() {
        let start = Instant::now();
        let mut proof = model(1);
        let mut scheduler = Scheduler::default();
        scheduler
            .observe(start, std::slice::from_ref(&proof))
            .unwrap();
        let old = scheduler.take_due(at(start, 1000)).unwrap();
        proof.text_epoch += 1;
        scheduler
            .observe(at(start, 1100), std::slice::from_ref(&proof))
            .unwrap();
        assert!(scheduler.failed(&proof));
        proof.save_generation += 1; // Older authorized write settles at1900.
        scheduler
            .observe(at(start, 1900), std::slice::from_ref(&proof))
            .unwrap();
        assert!(!scheduler.failed(&old));
        assert!(scheduler.take_due(at(start, 2099)).is_none());
        assert_eq!(scheduler.take_due(at(start, 2100)), Some(proof));
    }
    #[test]
    fn clean_untitled_disabled_removed_models_never_schedule_and_profile_clear_retires_state() {
        let start = Instant::now();
        let mut models: Vec<_> = (1..=4).map(model).collect();
        models[0].dirty = false;
        models[1].path = None;
        models[2].policy = Policy::Off;
        let mut scheduler = Scheduler::default();
        scheduler.observe(start, &models).unwrap();
        assert_eq!(scheduler.take_due(at(start, 1000)).unwrap().id, 4);
        scheduler.observe(at(start, 1000), &models[..3]).unwrap();
        assert!(scheduler.take_due(at(start, 2000)).is_none());
        scheduler.clear();
        assert!(scheduler.entries.is_empty());
        assert!(scheduler.observed_at.is_none());
        assert!(scheduler.last_selected.is_none());
    }
    #[test]
    fn invalid_inventory_never_mutates_existing_deadline_or_suppression() {
        let start = Instant::now();
        let mut scheduler = Scheduler::default();
        let proof = model(1);
        scheduler
            .observe(start, std::slice::from_ref(&proof))
            .unwrap();
        let before = scheduler.entries.clone();
        let mut invalids = vec![
            (0..=MODELS as u64).map(model).collect::<Vec<_>>(),
            vec![proof.clone(), proof.clone()],
        ];
        for path in [
            PathBuf::new(),
            "x".repeat(PATH_BYTES + 1).into(),
            "bad\0path".into(),
        ] {
            let mut invalid = proof.clone();
            invalid.path = Some(path);
            invalids.push(vec![invalid]);
        }
        let mut invalid = proof.clone();
        invalid.policy = Policy::AfterDelay {
            delay_ms: MAX_DELAY_MS + 1,
        };
        invalids.push(vec![invalid]);
        for models in invalids {
            assert!(scheduler.observe(at(start, 500), &models).is_err());
            assert_eq!(scheduler.entries, before);
            assert_eq!(scheduler.observed_at, Some(start));
            assert!(scheduler.last_selected.is_none());
        }
        assert_eq!(scheduler.take_due(at(start, 1000)), Some(proof));
        assert!(
            scheduler
                .observe(start - Duration::from_millis(1), &[])
                .is_err()
        );
        assert_eq!(scheduler.entries.len(), 1);
    }
    #[test]
    fn exact_path_and_delay_limits_are_admitted_without_io() {
        let start = Instant::now();
        let mut models: Vec<_> = (0..MODELS as u64).map(model).collect();
        for proof in &mut models {
            proof.path = Some("x".repeat(PATH_BYTES).into());
            proof.policy = Policy::AfterDelay {
                delay_ms: MAX_DELAY_MS,
            };
        }
        let mut scheduler = Scheduler::default();
        scheduler.observe(start, &models).unwrap();
        assert!(
            scheduler
                .take_due(at(start, MAX_DELAY_MS as u64 - 1))
                .is_none()
        );
        assert_eq!(
            scheduler
                .take_due(at(start, MAX_DELAY_MS.into()))
                .unwrap()
                .id,
            0
        );
    }
}
