//! Check the patched cursor against Unicode's expected boundaries, including
//! context requests at every scalar seam. Contiguous iteration alone does not
//! exercise the chunked API used for rope navigation.
use unicode_segmentation::{GraphemeCursor, GraphemeIncomplete};

#[allow(dead_code)]
#[rustfmt::skip]
#[path = "../vendor/unicode-segmentation/tests/testdata/mod.rs"]
mod corpus;

fn check(text: &str, expected: &[&str], extended: bool) {
    let chunks: Vec<_> = text
        .char_indices()
        .map(|(offset, ch)| (offset, &text[offset..offset + ch.len_utf8()]))
        .collect();
    let mut boundaries = vec![0];
    for grapheme in expected {
        boundaries.push(boundaries.last().unwrap() + grapheme.len());
    }
    assert_eq!(expected.concat(), text);

    let supply_context = |cursor: &mut GraphemeCursor, end: usize| {
        let (offset, chunk) = chunks.iter().find(|(i, s)| i + s.len() == end).unwrap();
        cursor.provide_context(chunk, *offset);
    };
    for forward in [true, false] {
        let mut cursor =
            GraphemeCursor::new(if forward { 0 } else { text.len() }, text.len(), extended);
        let mut index = if forward { 0 } else { chunks.len() - 1 };
        let mut actual = vec![cursor.cur_cursor()];
        loop {
            let (offset, chunk) = chunks[index];
            let result = if forward {
                cursor.next_boundary(chunk, offset)
            } else {
                cursor.prev_boundary(chunk, offset)
            };
            match result {
                Ok(Some(boundary)) => actual.push(boundary),
                Ok(None) => break,
                Err(GraphemeIncomplete::NextChunk) => index += 1,
                Err(GraphemeIncomplete::PrevChunk) => index -= 1,
                Err(GraphemeIncomplete::PreContext(end)) => supply_context(&mut cursor, end),
                Err(other) => panic!("Invalid cursor request: {other:?}"),
            }
        }
        if !forward {
            actual.reverse();
        }
        assert_eq!(
            actual, boundaries,
            "{text:?}, extended={extended}, forward={forward}"
        );
    }
    for &(offset, chunk) in &chunks {
        let mut cursor = GraphemeCursor::new(offset, text.len(), extended);
        let actual = loop {
            match cursor.is_boundary(chunk, offset) {
                Ok(value) => break value,
                Err(GraphemeIncomplete::PreContext(end)) => supply_context(&mut cursor, end),
                Err(other) => panic!("Invalid boundary request: {other:?}"),
            }
        };
        assert_eq!(
            actual,
            boundaries.contains(&offset),
            "{text:?} at {offset}, extended={extended}"
        );
    }
}

#[test]
fn unicode_17_boundaries_across_scalar_chunks() {
    for &(text, expected) in corpus::TEST_SAME {
        check(text, expected, true);
        check(text, expected, false);
    }
    for &(text, extended, legacy) in corpus::TEST_DIFF {
        check(text, extended, true);
        check(text, legacy, false);
    }
}

#[test]
fn cached_regional_indicator_count_survives_chunk_changes() {
    check("a🇬🇧🇺🇸🇯", &["a", "🇬🇧", "🇺🇸", "🇯"], true);
}
