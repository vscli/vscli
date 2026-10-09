use super::*;
use crate::extensions::{NativePrompt, PromptType};
impl App {
    pub(super) fn poll_extension_prompt(&mut self) -> bool {
        if let Some(Prompt {
            kind: PromptKind::Extension(request),
            ..
        }) = &self.prompt
        {
            let valid = self
                .extension_host
                .as_ref()
                .and_then(|host| host.prompt())
                .is_some_and(|current| {
                    current.id == request.id
                        && current.spec.session == request.spec.session
                        && current.spec.owner == request.spec.owner
                });
            if !valid {
                self.prompt = None;
                return true;
            }
        }
        if self.prompt.is_some() || self.modal.is_some() {
            return false;
        }
        let Some(request) = self
            .extension_host
            .as_ref()
            .and_then(|host| host.prompt())
            .cloned()
        else {
            return false;
        };
        let value = request.spec.value.clone();
        self.start_prompt(PromptKind::Extension(Box::new(request)), value);
        true
    }
    pub(super) fn cancel_extension_prompt(&mut self) {
        if !matches!(
            self.prompt.as_ref().map(|p| &p.kind),
            Some(PromptKind::Extension(_))
        ) {
            return;
        }
        let prompt = self.prompt.take().unwrap();
        if let PromptKind::Extension(request) = prompt.kind {
            self.reply_extension_prompt(&request, Value::Null);
        }
    }
    fn reply_extension_prompt(&mut self, request: &NativePrompt, answer: Value) {
        if let Some(host) = &mut self.extension_host
            && let Err(error) = host.answer_prompt(
                request.spec.session,
                &request.spec.owner,
                &request.id,
                answer,
            )
        {
            self.message = format!("Extension prompt failed: {error:#}");
        }
    }
    pub(super) fn accept_extension_prompt(
        &mut self,
        request: Box<NativePrompt>,
        text: String,
        selected: usize,
    ) {
        let answer = match request.spec.kind {
            PromptType::InputBox => json!(text),
            PromptType::QuickPick => {
                let matches = request.matches(&text);
                let Some((index, _)) = matches.get(selected.min(matches.len().saturating_sub(1)))
                else {
                    self.prompt = Some(Prompt::new(PromptKind::Extension(request), text));
                    return;
                };
                json!(index)
            }
        };
        self.reply_extension_prompt(&request, answer);
    }
}
