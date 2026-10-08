use super::*;
impl App {
    pub(super) fn poll_extensions(&mut self) -> bool {
        let Some(mut host) = self.extension_host.take() else {
            return false;
        };
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
                if let Some(bindings) = host.keybindings.take()
                    && let Err(error) = self.keymap.set_extension_bindings(bindings)
                {
                    self.message =
                        format!("Extension commands ready; keybindings rejected: {error:#}");
                }
                self.extension_host = Some(host);
                changed
            }
            Err(error) => {
                self.keymap.clear_extension_bindings();
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
