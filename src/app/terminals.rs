use super::*;
impl App {
    pub(super) fn new_terminal(&mut self) {
        if self.terminals.len() >= 8 {
            self.message = "Terminal limit reached (8); kill an unused terminal first".into();
            return;
        }
        match crate::terminal::Session::shell(
            &self.workspace.root,
            12,
            self.editor_area.width.max(40),
        ) {
            Ok(terminal) => {
                self.terminals.push(terminal);
                self.active_terminal = self.terminals.len() - 1;
                self.terminal_visible = true;
                self.focus = Focus::Terminal;
                self.message = "Terminal ready · Ctrl+` toggles · Ctrl+1 focuses editor".into();
            }
            Err(error) => self.message = format!("Terminal failed: {error:#}"),
        }
    }
    pub(super) fn kill_terminal(&mut self) {
        if self.terminals.is_empty() {
            return;
        }
        if let Err(error) = self.terminals[self.active_terminal].kill() {
            self.message = format!("Could not terminate terminal: {error:#}");
            return;
        }
        self.terminals.remove(self.active_terminal);
        self.active_terminal = self
            .active_terminal
            .min(self.terminals.len().saturating_sub(1));
        if self.terminals.is_empty() {
            self.terminal_visible = false;
            self.focus = Focus::Editor;
        }
    }
}
