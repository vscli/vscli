use super::*;
impl App {
    pub(super) fn poll_extensions(&mut self) -> bool {
        // Publish readiness only after the accepted-start worker releases its
        // operation slot. Otherwise a ready UI can reject the very next action.
        if self.extension_job.as_ref().is_some_and(|job| job.startup()) {
            return false;
        }
        let Some(mut host) = self.extension_host.take() else {
            return false;
        };
        if self.extension_packages.is_empty() {
            self.extension_packages = host.packages.clone();
        }
        let before: Vec<_> = self.documents.iter().map(|d| d.revision).collect();
        let commands = host.commands.clone();
        let busy = host.busy();
        match host.poll(&mut self.documents, self.active, &self.settings) {
            Ok(messages) => {
                let changed = !messages.is_empty()
                    || busy != host.busy()
                    || commands != host.commands
                    || before
                        != self
                            .documents
                            .iter()
                            .map(|d| d.revision)
                            .collect::<Vec<_>>();
                if messages.is_empty()
                    && busy
                    && !host.busy()
                    && self.message.starts_with("Running extension command:")
                {
                    self.message = "Extension command completed".into();
                }
                for message in messages {
                    self.message = message;
                }
                // Package validation is independent; one malformed contribution
                // cannot discard another selected package's defaults.
                if let Some(sets) = host.binding_sets.take() {
                    host.keybindings = None;
                    match self.keymap.set_extension_binding_sets(sets) {
                        Ok(errors) if !errors.is_empty() => {
                            self.message = format!(
                                "Extension commands ready; keybindings rejected: {}",
                                errors.join("; ")
                            )
                        }
                        Err(error) => {
                            self.message =
                                format!("Extension commands ready; keybindings rejected: {error:#}")
                        }
                        _ => {}
                    }
                }
                self.extension_host = Some(host);
                self.poll_extension_prompt() || changed
            }
            Err(error) => {
                self.keymap.clear_extension_bindings();
                self.retire_extension_host(host);
                self.message = format!("Extension host stopped: {error:#}");
                true
            }
        }
    }
    pub(super) fn execute_extension(&mut self, command: &str, args: Option<Value>) {
        if let Some(host) = &mut self.extension_host {
            match host.execute(command, args, &self.documents, self.active, &self.settings) {
                Ok(()) => self.message = format!("Running extension command: {command}"),
                Err(error) => self.message = format!("Extension command failed: {error:#}"),
            }
        } else {
            self.message = format!("Command not implemented in this alpha: {command}");
        }
    }
}
