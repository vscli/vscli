use super::*;

impl App {
    pub(super) fn type_editor_text(&mut self, text: &str, grouped: bool) {
        let options = self.settings.typing(self.language());
        let mut characters = text.chars();
        let Some(character) = characters.next() else {
            return;
        };
        if characters.next().is_some() {
            // Programmatic bulk insertion and paste retain literal semantics.
            self.doc_mut().insert(text, false);
            return;
        }
        let result = if character == '\n' || character == '\r' {
            self.doc_mut().newline_with_options(options)
        } else {
            self.doc_mut().type_character(character, options, grouped)
        };
        if let Err(error) = result {
            self.message = format!("Typing failed: {error:#}");
        }
    }

    pub(super) fn backspace_editor(&mut self, word: bool) {
        let options = self.settings.typing(self.language());
        if let Err(error) = self.doc_mut().backspace_with_options(options, word) {
            self.message = format!("Deletion failed: {error:#}");
        }
    }
}
