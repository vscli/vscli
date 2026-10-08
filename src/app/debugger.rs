use super::*;
use crate::debug::{Client, State};
impl App {
    pub(super) fn toggle_breakpoint(&mut self) {
        let Some(path) = self.doc().path.clone() else {
            self.message = "Save this file before setting breakpoints".into();
            return;
        };
        let line = self.doc().row() + 1;
        let points = self.breakpoints.entry(path.clone()).or_default();
        if !points.remove(&line) {
            points.insert(line);
        }
        if let Some(client) = self.debugger.as_mut()
            && let Err(e) = client.set_breakpoints(path, points.clone())
        {
            self.message = format!("Breakpoint update failed: {e:#}");
        }
    }
    pub(super) fn start_or_continue_debug(&mut self) {
        if let Some(client) = &self.debugger
            && client.state != State::Terminated
        {
            self.debug_control("continue");
            return;
        }
        let Some(config) = self.debug_configuration.clone() else {
            self.message = "Configure a debugger with --debug-adapter PROGRAM and optional --debug-arg arguments".into();
            return;
        };
        let Some(program) = config.program.clone().or_else(|| self.doc().path.clone()) else {
            self.message = "Select a saved program file or use --debug-program PATH".into();
            return;
        };
        let program = crate::document::absolute_path(&program).unwrap_or(program);
        if self.documents.iter().any(|d| {
            d.dirty()
                && d.path
                    .as_ref()
                    .is_some_and(|p| p == &program || self.breakpoints.contains_key(p))
        }) {
            self.message =
                "Save modified program and breakpoint source files before starting debugging"
                    .into();
            return;
        }
        match Client::start(
            &config,
            &program,
            &self.workspace.root,
            self.breakpoints.clone(),
        ) {
            Ok(client) => {
                self.debugger = Some(client);
                self.message = "Debugger starting…".into();
            }
            Err(e) => self.message = format!("Debugger failed to start: {e:#}"),
        }
    }
    pub(super) fn debug_control(&mut self, command: &str) {
        let Some(client) = self.debugger.as_mut() else {
            self.message = "No active debug session".into();
            return;
        };
        let result = match command {
            "disconnect" => client.stop(),
            "pause" => client.pause(),
            other => client.resume(other),
        };
        self.message = match result {
            Ok(()) => format!("Debugger: {command}"),
            Err(e) => format!("Debugger: {e:#}"),
        };
    }
    pub(super) fn poll_debugger(&mut self) -> bool {
        let Some(client) = self.debugger.as_mut() else {
            return false;
        };
        if client.state == State::Terminated {
            return false;
        }
        let previous = client.state.clone();
        match client.poll() {
            Err(e) => {
                client.state = State::Terminated;
                client.reason = format!("adapter stopped: {e:#}");
                self.message = client.reason.clone();
                true
            }
            Ok(changed) => {
                if previous != client.state {
                    self.message = format!("Debugger: {}", client.reason);
                }
                if let Some((path, line, column)) = client.location.take() {
                    if let Err(e) = self.open(&path) {
                        self.message = format!("Debugger source unavailable: {e:#}");
                    } else {
                        self.doc_mut().clear_secondary();
                        let row = line.saturating_sub(1).min(self.doc().line_count() - 1);
                        let pos = (self.doc().line_start(row) + column.saturating_sub(1))
                            .min(self.doc().line_end(row));
                        self.doc_mut().move_to(pos, false);
                        self.message = if self.doc().dirty() {
                            "Debugger stopped; source is modified, displayed lines may differ from disk".into()
                        } else {
                            "Debugger stopped · F5 continue · F10 step · Ctrl+Shift+D stack/variables".into()
                        };
                    }
                }
                changed
            }
        }
    }
    pub(super) fn debug_modal_key(
        &mut self,
        key: KeyEvent,
        mut section: usize,
        mut selected: usize,
    ) {
        match key.code {
            KeyCode::F(5) if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.debug_control("disconnect");
                return;
            }
            KeyCode::F(5) => {
                self.start_or_continue_debug();
                return;
            }
            KeyCode::F(10) => {
                self.debug_control("next");
                return;
            }
            KeyCode::F(11) => {
                self.debug_control(if key.modifiers.contains(KeyModifiers::SHIFT) {
                    "stepOut"
                } else {
                    "stepIn"
                });
                return;
            }
            KeyCode::Tab => {
                section = (section + 1) % 4;
                selected = 0;
            }
            KeyCode::BackTab => {
                section = (section + 3) % 4;
                selected = 0;
            }
            KeyCode::Up => selected = selected.saturating_sub(1),
            KeyCode::Down => selected = selected.saturating_add(1),
            KeyCode::PageUp => selected = selected.saturating_sub(10),
            KeyCode::PageDown => selected = selected.saturating_add(10),
            KeyCode::Char('e') => {
                self.start_prompt(PromptKind::DebugEvaluate, String::new());
                return;
            }
            KeyCode::Enter => {
                if let Some(client) = self.debugger.as_mut() {
                    let result = match section {
                        0 => client.select_frame(selected),
                        1 => client.expand(client.scopes.get(selected).map_or(0, |s| s.reference)),
                        2 => {
                            client.expand(client.variables.get(selected).map_or(0, |v| v.reference))
                        }
                        _ => Ok(()),
                    };
                    if let Err(e) = result {
                        self.message = e.to_string();
                    }
                    if section == 1 {
                        section = 2;
                    }
                    selected = 0;
                }
            }
            _ => {}
        }
        let count = self.debugger.as_ref().map_or(0, |d| match section {
            0 => d.frames.len(),
            1 => d.scopes.len(),
            2 => d.variables.len(),
            _ => d.console.len(),
        });
        selected = selected.min(count.saturating_sub(1));
        self.modal = Some(Modal::Debug { section, selected });
    }
}
