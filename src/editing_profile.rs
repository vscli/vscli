//! Native, declarative typing profiles. These are a qualified subset of the
//! pinned language configurations, not an extension language-configuration API.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AutoClosing {
    Always,
    #[default]
    LanguageDefined,
    BeforeWhitespace,
    Never,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PairHandling {
    Always,
    #[default]
    Auto,
    Never,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Surround {
    #[default]
    LanguageDefined,
    Quotes,
    Brackets,
    Never,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AutoIndent {
    None,
    Keep,
    #[default]
    Brackets,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TypingOptions {
    pub profile: ProfileId,
    pub brackets: AutoClosing,
    pub quotes: AutoClosing,
    pub delete: PairHandling,
    pub overtype: PairHandling,
    pub surround: Surround,
    pub indent: AutoIndent,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProfileId {
    Cpp,
    Json,
    #[default]
    Plaintext,
    Unsupported,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LexicalContext {
    Code,
    String,
    Comment,
    Unknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContextProof {
    pub document: u64,
    pub text_epoch: u64,
    pub profile: ProfileId,
    pub byte: usize,
    pub context: LexicalContext,
}
impl ProfileId {
    pub fn for_language(language: &str) -> Self {
        match language {
            "c" | "cpp" => Self::Cpp,
            "json" | "jsonc" => Self::Json,
            "plaintext" => Self::Plaintext,
            _ => Self::Unsupported,
        }
    }
    pub(crate) fn pair(self, open: char) -> Option<Pair> {
        let close = match open {
            '(' => ')',
            '[' => ']',
            '{' => '}',
            '\'' | '"' => open,
            '`' if self == Self::Json => open,
            _ => return None,
        };
        let (not_string, not_comment) = match self {
            Self::Cpp => (matches!(open, '\'' | '"'), open == '\''),
            Self::Json => (true, matches!(open, '"' | '`')),
            Self::Plaintext | Self::Unsupported => return None,
        };
        Some(Pair {
            open,
            close,
            not_string,
            not_comment,
        })
    }
    pub(crate) fn surround(self, open: char) -> Option<char> {
        if self == Self::Cpp && open == '<' {
            return Some('>');
        }
        self.pair(open).map(|pair| pair.close)
    }
    pub(crate) fn closing(self, ch: char) -> bool {
        ['(', '[', '{', '\'', '"', '`']
            .into_iter()
            .any(|open| self.pair(open).is_some_and(|pair| pair.close == ch))
    }
    pub(crate) fn indent_pair(self, open: char, close: Option<char>) -> bool {
        matches!(
            (self, open, close),
            (Self::Cpp, '{', None | Some('}'))
                | (Self::Cpp, '[', None | Some(']'))
                | (Self::Cpp, '(', None | Some(')'))
                | (Self::Json, '{', None | Some('}'))
                | (Self::Json, '[', None | Some(']'))
        )
    }
}
#[derive(Clone, Copy)]
pub(crate) struct Pair {
    pub open: char,
    pub close: char,
    pub not_string: bool,
    pub not_comment: bool,
}
pub(crate) fn quote(ch: char) -> bool {
    matches!(ch, '\'' | '"' | '`')
}
pub(crate) fn word(ch: char) -> bool {
    !ch.is_whitespace() && !"`~!@#$%^&*()-=+[{]}\\|;:'\",.<>/?".contains(ch)
}
pub(crate) fn before(ch: char, policy: AutoClosing) -> bool {
    match policy {
        AutoClosing::Always => true,
        AutoClosing::BeforeWhitespace => ch.is_whitespace(),
        AutoClosing::LanguageDefined => ";:.,=}])> \n\r\t".contains(ch),
        AutoClosing::Never => false,
    }
}
