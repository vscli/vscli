use super::*;
#[derive(Default)]
pub struct State {
    pub sequence: String,
    pub context: HashMap<String, Value>,
    pub source: &'static str,
    chord: Option<String>,
}
impl App {
    pub(super) fn open_keyboard_inspector(&mut self) {
        self.keyboard = State {
            context: self.context(),
            source: if self.focus == Focus::Editor {
                "Editor"
            } else if self.focus == Focus::Explorer {
                "Explorer"
            } else if self.focus == Focus::Outline {
                "Outline"
            } else if self.focus == Focus::Terminal {
                "Terminal"
            } else {
                "Workbench"
            },
            ..State::default()
        };
        self.last_key = None;
        self.modal = Some(Modal::Inspector);
    }
    pub(super) fn inspect_key(&mut self, event: KeyEvent) {
        self.last_key = Some(event);
        let token = keys::token(event);
        if event.kind == KeyEventKind::Release {
            self.keyboard.sequence = token;
            return;
        }
        self.keyboard.sequence = self
            .keyboard
            .chord
            .take()
            .filter(|prefix| prefix.len() + token.len() < 256)
            .map_or_else(|| token.clone(), |prefix| format!("{prefix} {token}"));
        if matches!(
            self.keymap
                .resolve(&self.keyboard.sequence, &self.keyboard.context),
            Resolution::Chord
        ) {
            self.keyboard.chord = Some(self.keyboard.sequence.clone());
        }
    }
}
