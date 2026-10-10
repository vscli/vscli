//! Original same-group tab moves preserve native document, view and save identity.
use super::*;

impl App {
    pub(super) fn execute_tab_reorder_command(&mut self, command: &str) -> bool {
        let left = match command {
            "workbench.action.moveEditorLeftInGroup" => true,
            "workbench.action.moveEditorRightInGroup" => false,
            _ => return false,
        };
        let Some(member) = self.active_tab_membership() else {
            return true;
        };
        let result = (|| -> Result<_> {
            let proof = self.editor_groups.group_proof(member.group)?;
            let index = proof
                .members()
                .iter()
                .position(|current| *current == member)
                .ok_or_else(|| anyhow::anyhow!("Active tab membership changed before reorder"))?;
            let destination = if left {
                index.saturating_sub(1)
            } else {
                index.saturating_add(1)
            };
            self.editor_groups.reorder(&proof, member, destination)
        })();
        match result {
            Ok(change) if change.changed => {
                self.navigation_input_interaction();
                self.session_interaction();
                self.cancel_suggestions();
                self.clear_signature();
                self.cancel_symbols();
                // Retire only the remaining captured batch. An already approved
                // exact-origin Save→Close still owns its actual receipt and must
                // recheck original mode eligibility before that origin can close.
                let retired_batch = self.closing_group.take().is_some();
                self.apply_group_change(change);
                self.invalidate_pending_extension_commands();
                self.message = if retired_batch {
                    "Editor moved; remaining close batch retired, approved original save retained"
                        .into()
                } else if left {
                    "Editor moved left".into()
                } else {
                    "Editor moved right".into()
                };
            }
            Ok(_) => {} // No group/UI/controller mutation for an original edge noop.
            Err(error) => {
                self.message = format!("Move editor rejected; buffers retained: {error:#}")
            }
        }
        true
    }
}
