//! Original directional focus chooses existing groups from retained topology.
use super::*;
use crate::{editor_groups::Membership, editor_layout::Direction};
use anyhow::{Context, ensure};

impl App {
    pub(super) fn execute_spatial_focus_command(&mut self, command: &str) -> bool {
        let direction = match command {
            "workbench.action.focusLeftGroup" => Direction::Left,
            "workbench.action.focusRightGroup" => Direction::Right,
            "workbench.action.focusAboveGroup" => Direction::Up,
            "workbench.action.focusBelowGroup" => Direction::Down,
            _ => return false,
        };
        if self.group_fallback {
            self.message =
                "Directional group focus unavailable in recovery overflow; buffers retained".into();
            return true;
        }
        let Some(source) = self.active_tab_membership() else {
            return true;
        };
        let result = (|| -> Result<Option<Membership>> {
            ensure!(
                self.editor_groups.groups().len() <= 4,
                "Directional focus exceeds native group bound"
            );
            let groups = self.editor_groups.proof();
            let group_ids: [Option<_>; 4] =
                std::array::from_fn(|i| self.editor_groups.groups().get(i).map(|group| group.id()));
            let mut ids = [source.group; 4];
            let mut count = 0;
            for group in group_ids.into_iter().flatten() {
                ids[count] = group;
                count += 1;
            }
            self.editor_layout.validate(&ids[..count])?;
            let candidates =
                self.editor_layout
                    .neighbor_candidates(source.group, direction, true)?;
            let recent = self.editor_groups.group_mru();
            // Match the original MRU tie-break. A never-active group ranks before
            // indexed groups; stable appearance order settles equal ranks.
            let selected = candidates.into_iter().flatten().min_by_key(|group| {
                recent
                    .iter()
                    .position(|id| id == group)
                    .map_or(0, |i| i + 1)
            });
            let target = selected
                .map(|group| -> Result<_> {
                    let target = self
                        .editor_groups
                        .group(group)
                        .context("Directional target group was closed")?
                        .active()
                        .context("Directional target group has no editor")?;
                    Ok(Membership {
                        group,
                        tab: target.id(),
                        document: target.document(),
                    })
                })
                .transpose()?;
            // Pure query/MRU lookup has no callbacks, await or filesystem work.
            // Recheck exact origin before the synchronous focus publication.
            ensure!(
                self.editor_groups.proof_current(&groups)
                    && self.editor_groups.membership_current(source),
                "Directional source membership changed"
            );
            Ok(target)
        })();
        match result {
            Ok(Some(target)) => {
                self.navigation_input_interaction();
                self.session_interaction();
                self.cancel_suggestions();
                self.clear_signature();
                self.cancel_symbols();
                // Focus does not mutate tab lists: keep the captured remaining
                // close batch and every independently approved original receipt.
                if let Err(error) = self.focus_tab(target) {
                    self.message =
                        format!("Directional focus rejected; buffers retained: {error:#}");
                }
                self.invalidate_pending_extension_commands();
                self.observe_editor_geometry();
            }
            Ok(None) => {}
            Err(error) => {
                self.message = format!("Directional focus rejected; buffers retained: {error:#}")
            }
        }
        true
    }
}
