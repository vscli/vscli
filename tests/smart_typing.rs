use std::collections::BTreeMap;
use vscli::{
    document::{Document, Selection},
    editing_profile::{
        AutoClosing, AutoIndent, LexicalContext, PairHandling, ProfileId, Surround, TypingOptions,
    },
    snippet::Template,
};

fn options(profile: ProfileId) -> TypingOptions {
    TypingOptions {
        profile,
        ..TypingOptions::default()
    }
}
fn select(doc: &mut Document, anchor: usize, cursor: usize) {
    doc.set_selections(vec![Selection {
        anchor: Some(anchor),
        cursor,
        desired_column: None,
    }]);
}
fn cursors(doc: &Document) -> Vec<(usize, usize)> {
    doc.selections()
        .iter()
        .map(|selection| {
            (
                selection.anchor.unwrap_or(selection.cursor),
                selection.cursor,
            )
        })
        .collect()
}
fn text_epoch(doc: &mut Document) -> u64 {
    doc.typing_contexts(&[doc.cursor], ProfileId::Cpp)[0].text_epoch
}

#[test]
fn generated_pair_skip_and_paired_delete_have_reference_undo_endpoints() {
    let typing = options(ProfileId::Cpp);
    let mut doc = Document::from_text("猫🙂 \r\n");
    doc.move_to(3, false);
    doc.type_character('(', typing, true).unwrap();
    assert_eq!(doc.text.to_string(), "猫🙂 ()\r\n");
    assert_eq!(doc.cursor, 4);
    let revision = doc.revision;
    doc.type_character(')', typing, true).unwrap();
    assert_eq!(doc.revision, revision);
    assert_eq!(doc.cursor, 5);
    doc.undo();
    assert_eq!(doc.text.to_string(), "猫🙂 \r\n");
    assert_eq!(doc.cursor, 3);
    doc.redo();
    assert_eq!(doc.text.to_string(), "猫🙂 ()\r\n");
    assert_eq!(doc.cursor, 5);

    let mut doc = Document::from_text("\r\n");
    doc.type_character('(', typing, true).unwrap();
    doc.backspace_with_options(typing, false).unwrap();
    assert_eq!(doc.text.to_string(), "\r\n");
    doc.undo();
    assert_eq!(doc.text.to_string(), "()\r\n");
    assert_eq!(doc.cursor, 1);
    doc.backspace_with_options(typing, false).unwrap();
    assert_eq!(
        doc.text.to_string(),
        "\r\n",
        "Undo must restore generated ownership"
    );
}

#[test]
fn multi_cursor_movement_retains_each_owned_pair_until_the_final_carets_leave() {
    let typing = options(ProfileId::Cpp);
    let original = "猫🙂 \r\n猫🙂 \r\n";
    let filled = "猫🙂 (x)\r\n猫🙂 (x)\r\n";
    let empty = "猫🙂 ()\r\n猫🙂 ()\r\n";
    let root = tempfile::tempdir().unwrap();
    for delete in [false, true] {
        let path = root
            .path()
            .join(if delete { "delete.cpp" } else { "skip.cpp" });
        std::fs::write(&path, original).unwrap();
        let mut doc = Document::open(&path).unwrap();
        let id = doc.id;
        // Reversed primary/secondary order must not privilege the first mark.
        doc.set_selections(vec![Selection::caret(8), Selection::caret(3)]);
        let initial = doc.selections();
        doc.type_character('(', typing, false).unwrap();
        doc.type_character('x', typing, false).unwrap();
        assert_eq!(doc.text.to_string(), filled);
        let inside = doc.selections();
        let revision = doc.revision;
        let epoch = text_epoch(&mut doc);
        assert!(doc.navigate_cursors("cursorLeft", false, 20));
        assert!(doc.navigate_cursors("cursorRight", false, 20));
        assert_eq!(doc.selections(), inside);
        assert_eq!((doc.revision, text_epoch(&mut doc)), (revision, epoch));
        if delete {
            doc.backspace_with_options(typing, false).unwrap();
            assert_eq!(doc.text.to_string(), empty);
            let pair_cursors = doc.selections();
            doc.backspace_with_options(typing, false).unwrap();
            assert_eq!(doc.text.to_string(), original);
            assert_eq!(doc.selections(), initial);
            doc.undo();
            assert_eq!(doc.text.to_string(), empty);
            assert_eq!(doc.selections(), pair_cursors);
            // Restored ownership allows the same atomic paired deletion again.
            doc.backspace_with_options(typing, false).unwrap();
            assert_eq!(doc.text.to_string(), original);
            doc.undo();
            assert_eq!(doc.text.to_string(), empty);
            doc.redo();
            assert_eq!(doc.text.to_string(), original);
        } else {
            doc.type_character(')', typing, false).unwrap();
            assert_eq!(doc.text.to_string(), filled);
            assert_eq!((doc.revision, text_epoch(&mut doc)), (revision, epoch));
            let outside = doc.selections();
            assert_eq!(cursors(&doc), [(14, 14), (6, 6)]);
            doc.undo();
            assert_eq!(doc.text.to_string(), empty);
            doc.redo();
            assert_eq!(doc.text.to_string(), filled);
            assert_eq!(doc.selections(), outside);
        }
        assert_eq!(doc.id, id);
        assert_eq!(std::fs::read(&path).unwrap(), original.as_bytes());
        doc.save().unwrap();
        assert_eq!(
            std::fs::read(&path).unwrap(),
            doc.text.to_string().as_bytes()
        );
    }
}

#[test]
fn multi_cursor_departure_retires_pairs_even_after_return_and_undo() {
    let typing = options(ProfileId::Cpp);
    let mut doc = Document::from_text("猫🙂 \r\n猫🙂 \r\n");
    doc.set_selections(vec![Selection::caret(8), Selection::caret(3)]);
    doc.type_character('(', typing, false).unwrap();
    doc.type_character('x', typing, false).unwrap();
    assert!(doc.navigate_cursors("cursorEnd", false, 20));
    assert_eq!(cursors(&doc), [(14, 14), (6, 6)]);
    assert!(doc.navigate_cursors("cursorLeft", false, 20));
    doc.type_character(')', typing, false).unwrap();
    assert_eq!(doc.text.to_string(), "猫🙂 (x))\r\n猫🙂 (x))\r\n");
    doc.undo();
    assert_eq!(doc.text.to_string(), "猫🙂 (x)\r\n猫🙂 (x)\r\n");
    doc.backspace_with_options(typing, false).unwrap();
    assert_eq!(doc.text.to_string(), "猫🙂 ()\r\n猫🙂 ()\r\n");
    doc.backspace_with_options(typing, false).unwrap();
    assert_eq!(doc.text.to_string(), "猫🙂 )\r\n猫🙂 )\r\n");
}

#[test]
fn oversized_public_secondary_selections_reject_before_staging_and_preserve_history() {
    let typing = options(ProfileId::Cpp);
    let mut doc = Document::from_text("猫🙂 \r\n");
    doc.move_to(3, false);
    doc.type_character('(', typing, false).unwrap();
    doc.type_character('x', typing, false).unwrap();
    doc.undo();
    let before = doc.text.to_string();
    let cursor = doc.cursor;
    let revision = doc.revision;
    let epoch = text_epoch(&mut doc);
    let invalid = vec![Selection::caret(cursor); 10_000];
    doc.secondary = invalid.clone();
    for operation in ["type", "newline", "lineBreakInsert", "backspace"] {
        let result = match operation {
            "type" => doc.type_character(')', typing, false),
            "newline" => doc.newline_with_options(typing),
            "lineBreakInsert" => doc.line_break_with_options(typing),
            "backspace" => doc.backspace_with_options(typing, false),
            _ => unreachable!(),
        };
        assert!(result.unwrap_err().to_string().contains("10,000 cursors"));
        assert_eq!(doc.text.to_string(), before);
        assert_eq!(doc.cursor, cursor);
        assert_eq!(doc.secondary, invalid);
        assert_eq!((doc.revision, text_epoch(&mut doc)), (revision, epoch));
    }
    doc.secondary.clear();
    doc.redo();
    assert_eq!(doc.text.to_string(), "猫🙂 (x)\r\n");
    doc.undo();
    doc.backspace_with_options(typing, false).unwrap();
    assert_eq!(
        doc.text.to_string(),
        "猫🙂 \r\n",
        "rejection preserves pair ownership"
    );
}

#[test]
fn manual_pasted_and_replaced_pairs_never_acquire_generated_ownership() {
    let typing = options(ProfileId::Cpp);
    for pasted in [false, true] {
        let mut doc = Document::from_text(if pasted { "\r\n" } else { "()\r\n" });
        if pasted {
            doc.insert("()", false);
        }
        doc.move_to(1, false);
        doc.type_character(')', typing, true).unwrap();
        assert_eq!(doc.text.to_string(), "())\r\n");
        doc.undo();
        doc.backspace_with_options(typing, false).unwrap();
        assert_eq!(doc.text.to_string(), ")\r\n");
    }
    let mut doc = Document::from_text("\r\n");
    doc.type_character('(', typing, true).unwrap();
    doc.apply_changes(vec![(1..2, ")".into())]);
    doc.move_to(1, false);
    doc.backspace_with_options(typing, false).unwrap();
    assert_eq!(
        doc.text.to_string(),
        ")\r\n",
        "replacement of a generated delimiter retires it even if bytes match"
    );
}

#[test]
fn surround_normalizes_direction_and_preserves_adjacent_selection_identity_and_history() {
    let typing = options(ProfileId::Cpp);
    let mut doc = Document::from_text("猫🙂 value\r\n");
    select(&mut doc, 8, 3);
    doc.type_character('(', typing, true).unwrap();
    assert_eq!(doc.text.to_string(), "猫🙂 (value)\r\n");
    assert_eq!(cursors(&doc), [(4, 9)]);
    doc.undo();
    assert_eq!(doc.text.to_string(), "猫🙂 value\r\n");
    assert_eq!(cursors(&doc), [(8, 3)]);

    let mut doc = Document::from_text("abcd\r\n");
    doc.set_selections(vec![
        Selection {
            cursor: 4,
            anchor: Some(2),
            desired_column: None,
        },
        Selection {
            cursor: 2,
            anchor: Some(0),
            desired_column: None,
        },
    ]);
    let before = doc.selections();
    doc.type_character('(', typing, true).unwrap();
    assert_eq!(doc.text.to_string(), "(ab)(cd)\r\n");
    assert_eq!(cursors(&doc), [(5, 7), (1, 3)]);
    doc.undo();
    assert_eq!(doc.selections(), before);
    assert_eq!(doc.text.to_string(), "abcd\r\n");
}

#[test]
fn mixed_cursors_fall_back_as_one_literal_gesture_and_large_surround_does_not_copy_content() {
    let typing = options(ProfileId::Cpp);
    let mut doc = Document::from_text("one\r\ntwo\r\n");
    doc.set_selections(vec![
        Selection {
            anchor: Some(0),
            cursor: 3,
            desired_column: None,
        },
        Selection::caret(8),
    ]);
    let before = doc.selections();
    doc.type_character('(', typing, true).unwrap();
    assert_eq!(doc.text.to_string(), "(\r\ntwo(\r\n");
    assert_eq!(cursors(&doc), [(1, 1), (7, 7)]);
    doc.undo();
    assert_eq!(doc.selections(), before);
    assert_eq!(doc.text.to_string(), "one\r\ntwo\r\n");

    let text = format!("猫{}\r\n", "x".repeat(5 * 1024 * 1024));
    let mut doc = Document::from_text(&text);
    let end = doc.len() - 2;
    select(&mut doc, 0, end);
    doc.type_character('<', typing, true).unwrap();
    assert_eq!(doc.text.len_bytes(), text.len() + 2);
    assert_eq!(doc.text.char(0), '<');
    assert_eq!(doc.text.char(doc.len() - 3), '>');
    doc.undo();
    assert_eq!(doc.text.to_string(), text);
}

#[test]
fn language_pair_restrictions_use_current_context_and_exact_cpp_raw_delimiters() {
    for (profile, text, cursor, typed, expected) in [
        (
            ProfileId::Cpp,
            "\"hello world\"\r\n",
            6,
            '(',
            "\"hello() world\"\r\n",
        ),
        (
            ProfileId::Cpp,
            "// hello world\r\n",
            8,
            '(',
            "// hello() world\r\n",
        ),
        (
            ProfileId::Json,
            "\"hello world\"\r\n",
            6,
            '(',
            "\"hello( world\"\r\n",
        ),
        (
            ProfileId::Cpp,
            "// hello world\r\n",
            8,
            '"',
            "// hello\" world\r\n",
        ),
        (
            ProfileId::Cpp,
            "R\"tag(hello world)tag\"\r\n",
            11,
            '"',
            "R\"tag(hello\" world)tag\"\r\n",
        ),
        (
            ProfileId::Cpp,
            "/* hello */\r\n",
            3,
            '\'',
            "/* 'hello */\r\n",
        ),
    ] {
        let mut doc = Document::from_text(text);
        doc.move_to(cursor, false);
        doc.type_character(typed, options(profile), true).unwrap();
        assert_eq!(doc.text.to_string(), expected);
        doc.undo();
        assert_eq!(doc.text.to_string(), text);
    }
    let mut doc =
        Document::from_text("R\"abcdefghijklmnop(猫\" )fake\" 🙂)abcdefghijklmnop\" \r\n");
    let interior = doc.text.to_string().find("🙂").unwrap();
    let interior = doc.text.byte_to_char(interior);
    let after = doc.len() - 3;
    let contexts = doc.typing_contexts(&[interior, after], ProfileId::Cpp);
    assert_eq!(contexts[0].context, LexicalContext::String);
    assert_eq!(contexts[1].context, LexicalContext::Code);
    let mut invalid = Document::from_text("R\"abcdefghijklmnopq(x)abcdefghijklmnopq\" ");
    let end = invalid.len();
    assert_eq!(
        invalid.typing_contexts(&[end], ProfileId::Cpp)[0].context,
        LexicalContext::Unknown
    );
}

#[test]
fn context_checkpoints_reject_edit_undo_and_obey_cold_scan_limit() {
    let mut doc = Document::from_text(&format!("{}\"text\" \r\n", " ".repeat(64 * 1024 + 8)));
    let end = doc.len() - 2;
    let cold = doc.typing_contexts(&[end], ProfileId::Cpp)[0];
    assert_eq!(cold.context, LexicalContext::Unknown);
    let warm = doc.typing_contexts(&[end], ProfileId::Cpp)[0];
    assert_eq!(warm.context, LexicalContext::Code);
    let revision = doc.revision;
    doc.apply_changes(vec![(0..0, "/*".into())]);
    let changed_end = doc.len() - 2;
    let changed = doc.typing_contexts(&[changed_end], ProfileId::Cpp)[0];
    assert_eq!(changed.context, LexicalContext::Unknown);
    assert_eq!(
        doc.typing_contexts(&[changed_end], ProfileId::Cpp)[0].context,
        LexicalContext::Comment
    );
    doc.undo();
    assert_eq!(doc.revision, revision);
    let undone = doc.typing_contexts(&[end], ProfileId::Cpp)[0];
    assert!(undone.text_epoch > warm.text_epoch);
    assert_eq!(undone.context, LexicalContext::Unknown);
    assert_eq!(
        doc.typing_contexts(&[end], ProfileId::Cpp)[0].context,
        LexicalContext::Code
    );
    assert_eq!(
        doc.typing_contexts(&[end], ProfileId::Unsupported)[0].context,
        LexicalContext::Unknown
    );

    let mut doc = Document::from_text("// continued \\\r\nstill comment\r\ncode");
    let middle = doc.line_start(1) + 6;
    let end = doc.len();
    let proofs = doc.typing_contexts(&[middle, end], ProfileId::Cpp);
    assert_eq!(proofs[0].context, LexicalContext::Comment);
    assert_eq!(proofs[1].context, LexicalContext::Code);
}

#[test]
fn newline_and_explicit_line_break_keep_reference_eol_and_distinct_cursor_behavior() {
    let typing = options(ProfileId::Cpp);
    for explicit in [false, true] {
        let mut doc = Document::from_text("{}\r\n");
        doc.move_to(1, false);
        if explicit {
            doc.line_break_with_options(typing).unwrap();
        } else {
            doc.newline_with_options(typing).unwrap();
        }
        assert_eq!(doc.text.to_string(), "{\r\n    \r\n}\r\n");
        assert_eq!(doc.cursor, if explicit { 1 } else { 7 });
        doc.undo();
        assert_eq!(doc.text.to_string(), "{}\r\n");
        assert_eq!(doc.cursor, 1);
    }
    let mut doc = Document::from_text("\t{}\n");
    doc.set_indentation(8, false);
    doc.move_to(2, false);
    doc.newline_with_options(typing).unwrap();
    assert_eq!(doc.text.to_string(), "\t{\n\t\t\n\t}\n");
    assert_eq!(doc.cursor, 5);
    doc.undo();
    let keep = TypingOptions {
        indent: AutoIndent::Keep,
        ..typing
    };
    doc.newline_with_options(keep).unwrap();
    assert_eq!(doc.text.to_string(), "\t{\n\t}\n");
    doc.undo();
    let none = TypingOptions {
        indent: AutoIndent::None,
        ..typing
    };
    doc.newline_with_options(none).unwrap();
    assert_eq!(doc.text.to_string(), "\t{\n}\n");
}

#[test]
fn multi_cursor_pairs_and_indentation_preserve_caller_order_with_one_undo() {
    let typing = options(ProfileId::Cpp);
    let mut doc = Document::from_text("one\r\ntwo\r\n");
    doc.set_selections(vec![Selection::caret(8), Selection::caret(3)]);
    let before = doc.selections();
    doc.type_character('(', typing, true).unwrap();
    assert_eq!(doc.text.to_string(), "one()\r\ntwo()\r\n");
    assert_eq!(cursors(&doc), [(11, 11), (4, 4)]);
    doc.type_character(')', typing, true).unwrap();
    assert_eq!(cursors(&doc), [(12, 12), (5, 5)]);
    doc.undo();
    assert_eq!(doc.text.to_string(), "one\r\ntwo\r\n");
    assert_eq!(doc.selections(), before);

    let mut doc = Document::from_text("{}\r\n  {}\r\n");
    doc.set_selections(vec![Selection::caret(7), Selection::caret(1)]);
    let before = doc.selections();
    doc.newline_with_options(typing).unwrap();
    assert_eq!(
        doc.text.to_string(),
        "{\r\n    \r\n}\r\n  {\r\n    \r\n  }\r\n"
    );
    doc.undo();
    assert_eq!(doc.text.to_string(), "{}\r\n  {}\r\n");
    assert_eq!(doc.selections(), before);
}

#[test]
fn shared_views_do_not_inherit_pair_ownership_and_owned_undo_preserves_dirty_disk() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("main.cpp");
    std::fs::write(&path, "猫🙂 \r\n").unwrap();
    let mut doc = Document::open(&path).unwrap();
    let id = doc.id;
    doc.activate_view(10);
    doc.move_to(3, false);
    doc.insert("dirty ", false);
    let dirty = doc.text.to_string();
    doc.type_character('(', options(ProfileId::Cpp), true)
        .unwrap();
    let pair = doc.text.to_string();
    doc.activate_view(20);
    let cursor = doc.cursor;
    doc.type_character(')', options(ProfileId::Cpp), true)
        .unwrap();
    assert_eq!(doc.text.to_string(), pair.replacen("()", "())", 1));
    doc.undo();
    assert_eq!(doc.text.to_string(), pair);
    assert_eq!(doc.cursor, cursor);
    doc.activate_view(10);
    doc.type_character(')', options(ProfileId::Cpp), true)
        .unwrap();
    assert_eq!(doc.text.to_string(), pair);
    doc.undo();
    assert_eq!(doc.text.to_string(), dirty);
    assert_eq!(doc.id, id);
    assert!(doc.dirty());
    assert_eq!(std::fs::read(&path).unwrap(), "猫🙂 \r\n".as_bytes());
    doc.redo();
    doc.save().unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), pair.as_bytes());
}

#[test]
fn explicit_retirement_and_disabled_settings_do_not_revive_generated_pairs() {
    let typing = options(ProfileId::Cpp);
    let mut doc = Document::from_text("\r\n");
    doc.type_character('(', typing, true).unwrap();
    doc.retire_typing_pairs();
    doc.undo();
    doc.redo();
    doc.move_to(1, false);
    doc.type_character(')', typing, true).unwrap();
    assert_eq!(doc.text.to_string(), "())\r\n");
    let off = TypingOptions {
        brackets: AutoClosing::Never,
        quotes: AutoClosing::Never,
        delete: PairHandling::Never,
        overtype: PairHandling::Never,
        surround: Surround::Never,
        ..typing
    };
    let mut doc = Document::from_text("\r\n");
    doc.type_character('(', off, true).unwrap();
    assert_eq!(doc.text.to_string(), "(\r\n");
    let mut doc = Document::from_text("abc\n");
    select(&mut doc, 0, 3);
    doc.type_character('(', off, true).unwrap();
    assert_eq!(doc.text.to_string(), "(\n");

    let mut doc = Document::from_text("\r\n");
    doc.type_character('(', typing, true).unwrap();
    doc.type_character(')', off, true).unwrap();
    assert_eq!(doc.text.to_string(), "())\r\n");
    doc.undo();
    assert_eq!(doc.text.to_string(), "()\r\n");
    doc.type_character(')', typing, true).unwrap();
    assert_eq!(
        doc.text.to_string(),
        "())\r\n",
        "returning to equal options must not revive retired generation"
    );

    let mut doc = Document::from_text("\r\n");
    doc.type_character('(', typing, true).unwrap();
    doc.type_character(')', options(ProfileId::Json), true)
        .unwrap();
    assert_eq!(
        doc.text.to_string(),
        "())\r\n",
        "a different profile must not inherit owned delimiters"
    );
}

#[test]
fn rejected_gesture_preserves_all_bytes_selections_undo_and_redo() {
    let mut doc = Document::from_text("x\r\n");
    doc.insert("dirty", false);
    doc.undo();
    let before = doc.text.to_string();
    let revision = doc.revision;
    doc.secondary = vec![Selection::caret(doc.len() + 1)];
    let selections = doc.selections();
    assert!(
        doc.type_character('(', options(ProfileId::Cpp), true)
            .is_err()
    );
    assert_eq!(doc.text.to_string(), before);
    assert_eq!(doc.selections(), selections);
    assert_eq!(doc.revision, revision);
    doc.secondary.clear();
    doc.redo();
    assert_eq!(doc.text.to_string(), "dirtyx\r\n");

    let mut doc = Document::from_text(&" ".repeat(64 * 1024 + 1));
    doc.move_to(doc.len(), false);
    let selections = doc.selections();
    let revision = doc.revision;
    assert!(doc.newline_with_options(options(ProfileId::Cpp)).is_err());
    assert_eq!(doc.text.len_bytes(), 64 * 1024 + 1);
    assert_eq!(doc.selections(), selections);
    assert_eq!(doc.revision, revision);
}

#[test]
fn linked_snippet_typing_remains_literal_and_linked() {
    let mut doc = Document::from_text("\r\n");
    doc.insert_snippet(&Template::parse("${1:x} + $1$0").unwrap(), &BTreeMap::new())
        .unwrap();
    doc.type_character('(', options(ProfileId::Cpp), true)
        .unwrap();
    assert_eq!(doc.text.to_string(), "( + (\r\n");
    assert!(doc.in_snippet());
    doc.type_character(')', options(ProfileId::Cpp), true)
        .unwrap();
    assert_eq!(doc.text.to_string(), "() + ()\r\n");
    doc.undo();
    assert_eq!(doc.text.to_string(), "x + x\r\n");

    let mut doc = Document::from_text("\r\n");
    doc.insert_snippet(
        &Template::parse("${1:()} + $1$0").unwrap(),
        &BTreeMap::new(),
    )
    .unwrap();
    doc.set_selections(vec![Selection::caret(1), Selection::caret(6)]);
    assert!(doc.in_snippet());
    doc.newline_with_options(options(ProfileId::Cpp)).unwrap();
    assert_eq!(doc.text.to_string(), "(\r\n) + (\r\n)\r\n");
    assert!(doc.in_snippet());
    doc.undo();
    assert_eq!(doc.text.to_string(), "() + ()\r\n");
}

#[test]
fn successful_save_as_retires_provenance_but_failed_save_as_does_not() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("main.cpp");
    let target = root.path().join("main.json");
    std::fs::write(&source, "\r\n").unwrap();
    std::fs::write(&target, "existing\r\n").unwrap();
    let mut doc = Document::open(&source).unwrap();
    let id = doc.id;
    doc.type_character('(', options(ProfileId::Cpp), true)
        .unwrap();
    assert!(doc.save_to(&target, false).is_err());
    doc.backspace_with_options(options(ProfileId::Cpp), false)
        .unwrap();
    assert_eq!(doc.text.to_string(), "\r\n");
    doc.undo();
    assert_eq!(doc.text.to_string(), "()\r\n");
    doc.save_to(&target, true).unwrap();
    doc.save_to(&source, true).unwrap();
    doc.undo();
    doc.redo();
    doc.move_to(1, false);
    doc.type_character(')', options(ProfileId::Cpp), true)
        .unwrap();
    assert_eq!(doc.text.to_string(), "())\r\n");
    assert_eq!(doc.id, id);
    assert_eq!(std::fs::read(&source).unwrap(), b"()\r\n");
    assert_eq!(std::fs::read(&target).unwrap(), b"()\r\n");
}

#[test]
fn whole_set_pair_capacity_falls_back_to_literal_without_partial_pairing() {
    let mut doc = Document::from_text(&"x\n".repeat(4097));
    let before = (0..4097)
        .map(|row| Selection::caret(row * 2 + 1))
        .collect::<Vec<_>>();
    doc.set_selections(before.clone());
    doc.type_character('(', options(ProfileId::Cpp), true)
        .unwrap();
    assert_eq!(doc.text.to_string(), "x(\n".repeat(4097));
    doc.undo();
    assert_eq!(doc.text.to_string(), "x\n".repeat(4097));
    assert_eq!(doc.selections(), before);
}

#[test]
fn maximum_generated_pair_set_keeps_permuted_caller_order_through_skip_delete_and_undo() {
    let typing = options(ProfileId::Cpp);
    let original = "猫🙂 \r\n".repeat(4096);
    let paired = "猫🙂 ()\r\n".repeat(4096);
    for delete in [false, true] {
        let mut doc = Document::from_text(&original);
        // Odd multiplication permutes all 4096 rows without sorting caller order.
        let initial = (0..4096)
            .map(|index| Selection::caret(((index * 2053) % 4096) * 5 + 3))
            .collect::<Vec<_>>();
        doc.set_selections(initial.clone());
        doc.type_character('(', typing, false).unwrap();
        assert_eq!(doc.text.to_string(), paired);
        let mut inside = doc.selections();
        inside.rotate_left(173);
        doc.set_selections(inside.clone());
        assert_eq!(doc.selections(), inside);
        if delete {
            doc.backspace_with_options(typing, false).unwrap();
            assert_eq!(doc.text.to_string(), original);
            doc.undo();
            assert_eq!(doc.text.to_string(), paired);
            assert_eq!(doc.selections(), inside);
            doc.backspace_with_options(typing, false).unwrap();
            assert_eq!(doc.text.to_string(), original);
            doc.undo();
            doc.redo();
            assert_eq!(doc.text.to_string(), original);
        } else {
            let revision = doc.revision;
            let epoch = text_epoch(&mut doc);
            doc.type_character(')', typing, false).unwrap();
            assert_eq!(doc.text.to_string(), paired);
            assert_eq!((doc.revision, text_epoch(&mut doc)), (revision, epoch));
            let outside = inside
                .iter()
                .map(|selection| Selection::caret(selection.cursor + 1))
                .collect::<Vec<_>>();
            assert_eq!(doc.selections(), outside);
            doc.undo();
            assert_eq!(doc.text.to_string(), original);
            assert_eq!(doc.selections(), initial);
            doc.redo();
            assert_eq!(doc.text.to_string(), paired);
            assert_eq!(doc.selections(), outside);
        }
    }
}

#[test]
fn size_and_nul_rejection_preserve_redo_and_selections() {
    let mut doc = Document::from_text(&"x".repeat(vscli::document::MAX_FILE_BYTES as usize));
    doc.insert("old", false);
    doc.undo();
    let selections = doc.selections();
    let revision = doc.revision;
    assert!(
        doc.type_character('(', options(ProfileId::Cpp), true)
            .is_err()
    );
    assert!(
        doc.type_character('\0', options(ProfileId::Cpp), true)
            .is_err()
    );
    assert_eq!(
        doc.text.len_bytes(),
        vscli::document::MAX_FILE_BYTES as usize
    );
    assert_eq!(doc.selections(), selections);
    assert_eq!(doc.revision, revision);
    doc.redo();
    assert_eq!(
        doc.text.len_bytes(),
        vscli::document::MAX_FILE_BYTES as usize + 3
    );
    assert_eq!(doc.text.slice(0..3).to_string(), "old");
}

#[test]
fn configured_pair_policies_do_not_invent_angle_closers_or_unsupported_language_pairs() {
    let typing = options(ProfileId::Cpp);
    let whitespace = TypingOptions {
        brackets: AutoClosing::BeforeWhitespace,
        ..typing
    };
    let mut doc = Document::from_text(">\r\n");
    doc.type_character('(', whitespace, true).unwrap();
    assert_eq!(doc.text.to_string(), "(>\r\n");
    let mut doc = Document::from_text(")\r\n");
    doc.type_character('(', whitespace, true).unwrap();
    assert_eq!(doc.text.to_string(), "())\r\n");
    let mut doc = Document::from_text("word\r\n");
    doc.type_character(
        '(',
        TypingOptions {
            brackets: AutoClosing::Always,
            ..typing
        },
        true,
    )
    .unwrap();
    assert_eq!(doc.text.to_string(), "()word\r\n");
    let mut doc = Document::from_text(")\r\n");
    doc.type_character(
        ')',
        TypingOptions {
            overtype: PairHandling::Always,
            ..options(ProfileId::Unsupported)
        },
        true,
    )
    .unwrap();
    assert_eq!(doc.text.to_string(), "))\r\n");
    let mut doc = Document::from_text("()\r\n");
    doc.move_to(1, false);
    doc.type_character(
        ')',
        TypingOptions {
            overtype: PairHandling::Always,
            ..typing
        },
        true,
    )
    .unwrap();
    assert_eq!(doc.text.to_string(), "()\r\n");
    assert_eq!(doc.cursor, 2);
}
