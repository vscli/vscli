use super::*;
impl App {
    pub(super) fn poll_extensions(&mut self) -> bool {
        // Publish readiness only after the accepted-start worker releases its
        // operation slot. Otherwise a ready UI can reject the very next action.
        if self.extension_job.as_ref().is_some_and(|job| job.startup()) {
            return false;
        }
        let Some(mut host) = self.extension_host.take() else {
            return self.poll_extension_services();
        };
        if self.extension_packages.is_empty() {
            self.extension_packages = host.packages.clone();
        }
        let before: Vec<_> = self
            .documents
            .iter()
            .chain(&self.hidden_documents)
            .map(|d| d.revision)
            .collect();
        let commands = host.commands.clone();
        let busy = host.busy();
        match host.poll_with_hidden(
            &mut self.documents,
            &mut self.hidden_documents,
            self.active,
            &self.settings,
        ) {
            Ok(messages) => {
                let changed = !messages.is_empty()
                    || busy != host.busy()
                    || commands != host.commands
                    || before
                        != self
                            .documents
                            .iter()
                            .chain(&self.hidden_documents)
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
                    if self.retain_extension_bindings(sets.clone()) {
                        self.extension_host = Some(host);
                        return self.poll_extension_prompt() || changed;
                    }
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
                self.poll_extension_services() | self.poll_extension_prompt() | changed
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
        if !self.queue_extension_command(command, &args) {
            self.execute_extension_now(command, args);
        }
    }
    pub(super) fn execute_extension_now(&mut self, command: &str, args: Option<Value>) {
        if let Some(host) = &mut self.extension_host {
            if self
                .activation
                .owners
                .get(command)
                .is_some_and(|owner| host.command_owner(command) != Some(owner.as_str()))
            {
                self.message = format!(
                    "Extension command owner conflicts with the selected contribution: {command}"
                );
                return;
            }
            match host.execute_with_hidden(
                command,
                args,
                &self.documents,
                &self.hidden_documents,
                self.active,
                &self.settings,
            ) {
                Ok(()) => self.message = format!("Running extension command: {command}"),
                Err(error) => self.message = format!("Extension command failed: {error:#}"),
            }
        } else {
            self.message = format!("Command not implemented in this alpha: {command}");
        }
    }
}
