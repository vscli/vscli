//! Grapheme boundaries over rope chunks, without flattening a line or prefix.
use ropey::RopeSlice;
use unicode_segmentation::{GraphemeCursor, GraphemeIncomplete};

pub(crate) struct Graphemes<'a> {
    text: RopeSlice<'a>,
    chunk: &'a str,
    offset: usize,
    cursor: GraphemeCursor,
}

impl<'a> Graphemes<'a> {
    pub(crate) fn new(text: RopeSlice<'a>) -> Self {
        let (chunk, offset, _, _) = text.chunk_at_byte(0);
        Self {
            text,
            chunk,
            offset,
            cursor: GraphemeCursor::new(0, text.len_bytes(), true),
        }
    }
}

fn context(cursor: &mut GraphemeCursor, text: RopeSlice<'_>, end: usize) {
    let (chunk, offset, _, _) = text.chunk_at_byte(end - 1);
    cursor.provide_context(&chunk[..end - offset], offset);
}

impl<'a> Iterator for Graphemes<'a> {
    type Item = RopeSlice<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let start = self.cursor.cur_cursor();
        let end = loop {
            match self.cursor.next_boundary(self.chunk, self.offset) {
                Ok(boundary) => break boundary?,
                Err(GraphemeIncomplete::NextChunk) => {
                    let (chunk, offset, _, _) =
                        self.text.chunk_at_byte(self.offset + self.chunk.len());
                    self.chunk = chunk;
                    self.offset = offset;
                }
                Err(GraphemeIncomplete::PreContext(end)) => {
                    context(&mut self.cursor, self.text, end)
                }
                other => unreachable!("Forward cursor requested invalid context: {other:?}"),
            }
        };
        // Most graphemes fit in one leaf. Borrow that leaf directly rather than
        // walking the rope tree again for each visible ASCII character.
        Some(if start >= self.offset {
            RopeSlice::from(&self.chunk[start - self.offset..end - self.offset])
        } else {
            self.text.byte_slice(start..end)
        })
    }
}

pub(crate) fn previous_boundary(text: RopeSlice<'_>) -> usize {
    if text.len_bytes() == 0 {
        return 0;
    }
    let mut cursor = GraphemeCursor::new(text.len_bytes(), text.len_bytes(), true);
    let (mut chunk, mut offset, _, _) = text.chunk_at_byte(text.len_bytes() - 1);
    loop {
        match cursor.prev_boundary(chunk, offset) {
            Ok(boundary) => return text.byte_to_char(boundary.unwrap_or(0)),
            Err(GraphemeIncomplete::PrevChunk) => {
                let previous = text.chunk_at_byte(offset - 1);
                chunk = previous.0;
                offset = previous.1;
            }
            Err(GraphemeIncomplete::PreContext(end)) => context(&mut cursor, text, end),
            other => unreachable!("Backward cursor requested invalid context: {other:?}"),
        }
    }
}

pub(crate) fn as_text(slice: RopeSlice<'_>) -> std::borrow::Cow<'_, str> {
    match slice.as_str() {
        Some(text) => text.into(),
        None => slice.to_string().into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ropey::Rope;
    use unicode_segmentation::UnicodeSegmentation;

    #[test]
    fn chunked_boundaries_match_contiguous_unicode_in_both_directions() {
        let cases = [
            "ASCII\t\r\n".repeat(700),
            "e\u{301} 猫 👩\u{200d}💻 🇬🇧🇺🇸🇯🇵 क्\u{200d}ष \u{600}A".repeat(250),
            format!("a{}🙂z", "\u{301}".repeat(4000)),
            "🇬🇧🇺🇸🇯🇵".repeat(600),
        ];
        for text in cases {
            let mut rope = Rope::new();
            // Build several different leaf arrangements, splitting source input
            // even inside combining/ZWJ/flag sequences before rope insertion.
            for ch in text.chars() {
                rope.insert_char(rope.len_chars(), ch);
            }
            assert!(rope.chunks().count() > 1);
            let actual: Vec<_> = Graphemes::new(rope.slice(..))
                .map(|g| g.to_string())
                .collect();
            let expected: Vec<_> = text.graphemes(true).collect();
            for (index, (actual, expected)) in actual.iter().zip(&expected).enumerate() {
                assert_eq!(actual, expected, "Grapheme {index}");
            }
            assert_eq!(actual.len(), expected.len());
            let mut end = rope.len_chars();
            for grapheme in expected.iter().rev() {
                let start = previous_boundary(rope.slice(..end));
                assert_eq!(rope.slice(start..end), *grapheme);
                end = start;
            }
            assert_eq!(end, 0);
            // A cursor may arrive inside a cluster through protocol offsets.
            // Preserve the old behavior of segmenting that truncated prefix.
            for end in [1, 2, 8, rope.len_chars() / 2] {
                let prefix = rope.slice(..end).to_string();
                let last = prefix.graphemes(true).next_back().unwrap();
                assert_eq!(
                    previous_boundary(rope.slice(..end)),
                    end - last.chars().count()
                );
            }
        }
        assert_eq!(Graphemes::new(RopeSlice::from("")).next(), None);
        assert_eq!(previous_boundary(RopeSlice::from("")), 0);
    }
}
