use super::*;
use crate::git::{Action, Job, Output};
impl App {
    pub(super) fn start_git(&mut self, action: Action) {
        if self.git_job.is_some() {
            self.message = "A Git operation is already running".into();
            return;
        }
        self.git_job = Some(Job::start(self.workspace.root.clone(), action));
        self.message = "Git operation running…".into();
    }
    pub(super) fn poll_git(&mut self) -> bool {
        let Some(result) = self.git_job.as_mut().and_then(Job::poll) else {
            return false;
        };
        self.git_job = None;
        match result {
            Err(error) => {
                self.message = format!("Git failed: {error}");
                if self.git_status.is_none() && matches!(self.modal, Some(Modal::Git)) {
                    self.modal = None;
                }
            }
            Ok(Output::Status(status)) => {
                self.git_selected = self
                    .git_selected
                    .min(status.entries.len().saturating_sub(1));
                self.git_status = Some(status);
                self.message = "Git status refreshed · saved working tree contents".into();
            }
            Ok(Output::Text { title, text }) => {
                self.modal = Some(Modal::Text {
                    title,
                    text,
                    scroll: 0,
                })
            }
            Ok(Output::Changed(message)) => {
                self.start_git(Action::Status);
                self.message = message;
                self.modal = Some(Modal::Git);
            }
        }
        true
    }
    pub(super) fn git_file_action(&mut self, action: &str) {
        let Some(status) = &self.git_status else {
            self.message = "Open Source Control first".into();
            return;
        };
        let Some(entry) = status.entries.get(self.git_selected) else {
            return;
        };
        let relative = entry.path.clone();
        let path = status.root.join(&relative);
        if action == "open" {
            self.modal = None;
            if let Err(error) = self.open(&path) {
                self.message = format!("Open failed: {error:#}");
            }
            return;
        }
        if action == "stage"
            && self
                .documents
                .iter()
                .any(|d| d.path.as_ref() == Some(&path) && d.dirty())
        {
            self.message = "Save this buffer before staging its file".into();
            return;
        }
        let action = match action {
            "stage" => Action::Stage(relative),
            "unstage" => Action::Unstage(relative),
            "diff" => Action::Diff {
                path: relative,
                staged: false,
            },
            "staged_diff" => Action::Diff {
                path: relative,
                staged: true,
            },
            _ => return,
        };
        self.start_git(action);
    }
}
