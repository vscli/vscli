use super::*;
use crate::tasks::{self, PreparedTask, Task, Variables};
impl App {
    pub(super) fn show_tasks(&mut self, build: bool, label: Option<&str>) {
        let tasks = match tasks::load(&self.workspace.root) {
            Ok(tasks) => tasks,
            Err(e) => {
                self.message = format!("Cannot load tasks: {e:#}");
                return;
            }
        };
        if let Some(label) = label {
            if let Some(task) = tasks.iter().find(|t| t.label == label) {
                self.prepare_task(task);
            } else {
                self.message = format!("No task named {label}");
            }
            return;
        }
        let tasks = if build {
            let defaults: Vec<_> = tasks
                .iter()
                .filter(|t| tasks::is_default_build(t))
                .collect();
            if defaults.len() == 1 {
                self.prepare_task(defaults[0]);
                return;
            }
            tasks
                .into_iter()
                .filter(tasks::is_build)
                .collect::<Vec<_>>()
        } else {
            tasks
        };
        if tasks.is_empty() {
            self.message = "No matching tasks in .vscode/tasks.json".into();
        } else {
            self.modal = Some(Modal::Tasks { tasks, selected: 0 });
        }
    }
    pub(super) fn prepare_task(&mut self, task: &Task) {
        let selected = self
            .active_document()
            .and_then(Document::selected_text)
            .unwrap_or_default();
        let vars = Variables {
            root: &self.workspace.root,
            file: self.active_document().and_then(|d| d.path.as_deref()),
            line: self.active_document().map(|d| d.row() + 1),
            selected: &selected,
        };
        match tasks::prepare(task, &vars) {
            Ok(task) if self.trusted_tasks => self.run_task(task),
            Ok(task) => self.modal = Some(Modal::ConfirmTask(task)),
            Err(e) => self.message = format!("Cannot run task: {e:#}"),
        }
    }
    pub(super) fn run_task(&mut self, task: PreparedTask) {
        if self.terminals.len() >= 8 {
            self.message = "Terminal limit reached (8); close an unused terminal first".into();
            return;
        }
        match crate::terminal::Session::spawn(
            task.command,
            format!("Task: {}", task.label),
            12,
            self.editor_area.width.max(40),
        ) {
            Ok(terminal) => {
                self.terminals.push(terminal);
                self.active_terminal = self.terminals.len() - 1;
                self.terminal_visible = true;
                self.focus = Focus::Terminal;
                self.message = if task.notice.is_empty() {
                    format!("Running task: {}", task.label)
                } else {
                    task.notice
                };
            }
            Err(e) => self.message = format!("Task failed to start: {e:#}"),
        }
    }
}
