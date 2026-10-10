use vscli::{
    document::{Document, Selection},
    editing_profile::{AutoIndent, ProfileId, TypingOptions},
};

const MODES: [AutoIndent; 5] = [
    AutoIndent::None,
    AutoIndent::Keep,
    AutoIndent::Brackets,
    AutoIndent::Advanced,
    AutoIndent::Full,
];
fn options(profile: ProfileId, indent: AutoIndent) -> TypingOptions {
    TypingOptions {
        profile,
        indent,
        ..TypingOptions::default()
    }
}
fn end_of(doc: &mut Document, row: usize) {
    doc.move_to(doc.line_end(row), false);
}
fn assert_undo_redo(doc: &mut Document, before: &str, selections: &[Selection], after: &str) {
    let endpoints = doc.selections();
    doc.undo();
    assert_eq!(doc.text.to_string(), before);
    assert_eq!(doc.selections(), selections);
    doc.redo();
    assert_eq!(doc.text.to_string(), after);
    assert_eq!(doc.selections(), endpoints);
}

#[test]
fn single_electric_closers_align_fresh_code_in_every_mode_with_exact_disk_undo() {
    let directory = tempfile::tempdir().unwrap();
    for profile in [ProfileId::Cpp, ProfileId::Json] {
        for mode in MODES {
            let before = "猫🙂\r\n\t{\r\n \t  \r\n";
            let after = "猫🙂\r\n\t{\r\n    }\r\n";
            let mut doc = Document::from_text(before);
            doc.set_indentation(4, true);
            end_of(&mut doc, 2);
            let selections = doc.selections();
            doc.type_character('}', options(profile, mode), false)
                .unwrap();
            assert_eq!(doc.text.to_string(), after, "{profile:?}/{mode:?}");
            assert_undo_redo(&mut doc, before, &selections, after);
            let path = directory.path().join(format!("{profile:?}-{mode:?}.txt"));
            doc.save_to(&path, false).unwrap();
            assert_eq!(std::fs::read(&path).unwrap(), after.as_bytes());
        }
    }
}

#[test]
fn advanced_cpp_textual_control_body_rules_include_comments_and_ascii_word_boundary() {
    for header in [
        "if(ok)",
        "elseif(ok)",
        "else if(check(\"(\")) )",
        "else",
        "for(;;)",
        "while(ok)",
    ] {
        for body in ["run();", "// note{", "/* note{", "iffy();"] {
            for mode in [AutoIndent::Advanced, AutoIndent::Full] {
                let before = format!("\t{header}\r\n \t  {body}\r\n");
                let mut doc = Document::from_text(&before);
                doc.set_indentation(4, false);
                end_of(&mut doc, 1);
                let selections = doc.selections();
                doc.newline_with_options(options(ProfileId::Cpp, mode))
                    .unwrap();
                let after = format!("\t{header}\r\n \t  {body}\r\n\t\r\n");
                assert_eq!(doc.text.to_string(), after, "{header}/{body}/{mode:?}");
                assert_undo_redo(&mut doc, &before, &selections, &after);
            }
        }
    }
    for (header, body) in [
        ("else  if(ok)", "run();"),
        ("if(ok)", "if猫();"),
        ("if(ok)", "ifé();"),
        ("if(ok)", "if(ok);"),
        ("if(ok)", "{ run(); }"),
    ] {
        let before = format!("{header}\r\n    {body}\r\n");
        let mut doc = Document::from_text(&before);
        end_of(&mut doc, 1);
        doc.newline_with_options(options(ProfileId::Cpp, AutoIndent::Full))
            .unwrap();
        assert_eq!(
            doc.text.to_string(),
            format!("{header}\r\n    {body}\r\n    \r\n")
        );
    }
}

#[test]
fn cpp_modes_and_physical_blank_line_do_not_apply_advanced_outdent() {
    for mode in MODES {
        let mut doc = Document::from_text("if(ok)\r\n    run();\r\n");
        end_of(&mut doc, 1);
        doc.newline_with_options(options(ProfileId::Cpp, mode))
            .unwrap();
        let indent = if matches!(mode, AutoIndent::Keep | AutoIndent::Brackets) {
            "    "
        } else {
            ""
        };
        assert_eq!(
            doc.text.to_string(),
            format!("if(ok)\r\n    run();\r\n{indent}\r\n")
        );
    }
    for text in ["if(ok)\r\n\r\n    run();\r\n", "if(ok)\r\n    \r\n"] {
        let mut doc = Document::from_text(text);
        let row = doc.line_count() - 2;
        end_of(&mut doc, row);
        doc.newline_with_options(options(ProfileId::Cpp, AutoIndent::Full))
            .unwrap();
        assert_eq!(
            doc.text.to_string(),
            format!("{}\r\n    \r\n", text.trim_end_matches("\r\n"))
        );
    }
}

#[test]
fn json_full_enter_dedents_existing_closer_and_explicit_line_break_retains_position() {
    let before = "{\r\n    \"猫🙂\": 1   }\r\n";
    for explicit in [false, true] {
        let mut doc = Document::from_text(before);
        doc.move_to(doc.line_end(1) - 4, false);
        let original = doc.cursor;
        let selections = doc.selections();
        if explicit {
            doc.line_break_with_options(options(ProfileId::Json, AutoIndent::Full))
                .unwrap();
        } else {
            doc.newline_with_options(options(ProfileId::Json, AutoIndent::Full))
                .unwrap();
        }
        // Spaces belonging to content after the caret remain: only original
        // leading indentation can be consumed by the Full range expansion.
        let after = "{\r\n    \"猫🙂\": 1\r\n   }\r\n";
        assert_eq!(doc.text.to_string(), after);
        assert_eq!(doc.cursor, if explicit { original } else { original + 2 });
        assert_undo_redo(&mut doc, before, &selections, after);
    }
}

#[test]
fn json_full_multi_closer_preserves_permuted_caller_order_one_undo_and_other_modes_literal() {
    let before = "{\r\n        \r\n}\r\n[\r\n        \r\n]\r\n";
    for mode in MODES {
        let mut doc = Document::from_text(before);
        doc.set_selections(vec![
            Selection::caret(doc.line_end(4)),
            Selection::caret(doc.line_end(1)),
        ]);
        let selections = doc.selections();
        doc.type_character('}', options(ProfileId::Json, mode), false)
            .unwrap();
        let after = if mode == AutoIndent::Full {
            "{\r\n}\r\n}\r\n[\r\n}\r\n]\r\n"
        } else {
            "{\r\n        }\r\n}\r\n[\r\n        }\r\n]\r\n"
        };
        assert_eq!(doc.text.to_string(), after, "{mode:?}");
        assert!(doc.selections()[0].cursor > doc.selections()[1].cursor);
        assert_undo_redo(&mut doc, before, &selections, after);
    }
    let mut doc = Document::from_text(before);
    doc.set_selections(vec![
        Selection::caret(doc.line_end(4)),
        Selection::caret(doc.line_end(1)),
    ]);
    doc.type_character('}', options(ProfileId::Cpp, AutoIndent::Full), false)
        .unwrap();
    assert_eq!(
        doc.text.to_string(),
        "{\r\n        }\r\n}\r\n[\r\n        }\r\n]\r\n"
    );
}

#[test]
fn generated_close_skips_before_electric_and_multiline_electric_groups_with_enter() {
    let typing = options(ProfileId::Cpp, AutoIndent::Full);
    let mut doc = Document::from_text("    \r\n");
    doc.move_to(4, false);
    doc.type_character('{', typing, false).unwrap();
    doc.type_character('}', typing, false).unwrap();
    assert_eq!(doc.text.to_string(), "    {}\r\n");
    assert_eq!(doc.cursor, 6);
    doc.undo();
    assert_eq!(doc.text.to_string(), "    \r\n");
    doc.redo();
    assert_eq!(doc.cursor, 6);

    let mut doc = Document::from_text("    \r\n");
    doc.move_to(4, false);
    doc.type_character('{', typing, false).unwrap();
    let paired = doc.selections();
    doc.newline_with_options(typing).unwrap();
    doc.type_character('}', typing, true).unwrap();
    assert_eq!(doc.text.to_string(), "    {\r\n    }\r\n    }\r\n");
    assert_eq!(doc.cursor, 12);
    assert_undo_redo(
        &mut doc,
        "    {}\r\n",
        &paired,
        "    {\r\n    }\r\n    }\r\n",
    );
}

#[test]
fn lexical_proof_ignores_escaped_quotes_raw_strings_comments_and_same_line_openers() {
    for (profile, text) in [
        (ProfileId::Cpp, "{\r\n/*\r\n    \r\n*/\r\n"),
        (ProfileId::Cpp, "{\r\nR\"tag(\r\n    \r\n)tag\"\r\n"),
        (ProfileId::Json, "{\r\n    \"text \\\" {\r\n"),
        (ProfileId::Cpp, "    {    \r\n"),
    ] {
        let mut doc = Document::from_text(text);
        let row = if profile == ProfileId::Cpp && doc.line_count() > 3 {
            2
        } else if doc.line_count() > 2 {
            1
        } else {
            0
        };
        end_of(&mut doc, row);
        let at = doc.cursor;
        doc.type_character('}', options(profile, AutoIndent::Full), false)
            .unwrap();
        let mut expected = text.to_owned();
        let byte = text
            .char_indices()
            .nth(at)
            .map_or(text.len(), |(byte, _)| byte);
        expected.insert(byte, '}');
        assert_eq!(doc.text.to_string(), expected);
    }
    let mut doc = Document::from_text("{\r\n    \"text \\\" {\" : 1,\r\n        \r\n");
    end_of(&mut doc, 2);
    doc.type_character('}', options(ProfileId::Json, AutoIndent::Keep), false)
        .unwrap();
    assert_eq!(
        doc.text.to_string(),
        "{\r\n    \"text \\\" {\" : 1,\r\n}\r\n"
    );
}

#[test]
fn expanded_same_line_edits_reject_atomically_and_preserve_redo() {
    for newline in [false, true] {
        let before = if newline {
            "{\r\n    }\r\n"
        } else {
            "{\r\n    \r\n}\r\n"
        };
        let mut doc = Document::from_text(before);
        doc.move_to(doc.len(), false);
        doc.insert("猫🙂", false);
        doc.undo();
        doc.set_selections(vec![Selection::caret(4), Selection::caret(5)]);
        let selections = doc.selections();
        let revision = doc.revision;
        let result = if newline {
            doc.newline_with_options(options(ProfileId::Json, AutoIndent::Full))
        } else {
            doc.type_character('}', options(ProfileId::Json, AutoIndent::Full), false)
        };
        assert!(
            result.is_err(),
            "newline={newline}, text={:?}, selections={:?}",
            doc.text.to_string(),
            doc.selections()
        );
        assert_eq!(doc.text.to_string(), before);
        assert_eq!(doc.revision, revision);
        assert_eq!(doc.selections(), selections);
        doc.redo();
        assert_eq!(doc.text.to_string(), format!("{before}猫🙂"));
    }
}

#[test]
fn line_token_and_scan_caps_fall_back_without_partial_dedent_or_document_flattening() {
    for text in [
        format!("{{\r\n{}    \r\n", "\r\n".repeat(128)),
        format!("{{{}\r\n    \r\n", "()".repeat(4097)),
        format!("{{{}\r\n    \r\n", "x".repeat(64 * 1024)),
    ] {
        let mut doc = Document::from_text(&text);
        let row = doc.line_count() - 2;
        end_of(&mut doc, row);
        let selections = doc.selections();
        let at = doc.cursor;
        doc.type_character('}', options(ProfileId::Cpp, AutoIndent::Full), false)
            .unwrap();
        let mut expected = text.clone();
        expected.insert(at, '}');
        assert_eq!(doc.text.to_string(), expected);
        assert_undo_redo(&mut doc, &text, &selections, &expected);
    }
}

#[test]
fn interior_crlf_carets_and_selection_ends_fall_back_without_panics() {
    for profile in [ProfileId::Cpp, ProfileId::Json] {
        for close in [false, true] {
            let mut doc = Document::from_text("\r\n");
            doc.move_to(1, false);
            let selections = doc.selections();
            if close {
                doc.type_character('}', options(profile, AutoIndent::Full), false)
                    .unwrap();
            } else {
                doc.newline_with_options(options(profile, AutoIndent::Full))
                    .unwrap();
            }
            let after = if close { "\r}\n" } else { "\r\r\n\n" };
            assert_eq!(doc.text.to_string(), after);
            assert_undo_redo(&mut doc, "\r\n", &selections, after);
        }
        let mut doc = Document::from_text("{\r\n");
        doc.set_selections(vec![Selection {
            anchor: Some(1),
            cursor: 2,
            desired_column: None,
        }]);
        let selections = doc.selections();
        doc.newline_with_options(options(profile, AutoIndent::Full))
            .unwrap();
        assert_undo_redo(&mut doc, "{\r\n", &selections, "{\r\n\n");
    }
}

#[test]
fn explicit_ungrouped_typing_and_option_changes_keep_indentation_history_separate() {
    let typing = options(ProfileId::Cpp, AutoIndent::Full);
    for change_options in [false, true] {
        let mut doc = Document::from_text("    \r\n");
        doc.move_to(4, false);
        doc.type_character('{', typing, false).unwrap();
        doc.newline_with_options(typing).unwrap();
        let before = doc.text.to_string();
        let selections = doc.selections();
        let next = if change_options {
            options(ProfileId::Cpp, AutoIndent::Keep)
        } else {
            typing
        };
        doc.type_character('}', next, change_options).unwrap();
        let after = doc.text.to_string();
        assert_undo_redo(&mut doc, &before, &selections, &after);
    }
}

#[test]
fn current_epoch_proofs_do_not_survive_edit_undo() {
    let typing = options(ProfileId::Cpp, AutoIndent::Full);
    let mut doc = Document::from_text("{\r\n        \r\n");
    end_of(&mut doc, 1);
    doc.type_character('}', typing, false).unwrap();
    doc.undo();
    doc.apply_changes(vec![(0..0, "/*".to_owned())]);
    end_of(&mut doc, 1);
    doc.type_character('}', typing, false).unwrap();
    assert_eq!(doc.text.to_string(), "/*{\r\n        }\r\n");
    doc.undo();
    doc.undo();
    assert_eq!(doc.text.to_string(), "{\r\n        \r\n");
    end_of(&mut doc, 1);
    doc.type_character('}', typing, false).unwrap();
    assert_eq!(doc.text.to_string(), "{\r\n}\r\n");
}

#[test]
fn shared_view_prefix_edits_keep_dirty_model_identity_disk_and_owned_undo_endpoints() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("main.cpp");
    let disk = "猫🙂\r\n{\r\n        \r\n";
    std::fs::write(&path, disk).unwrap();
    let mut doc = Document::open(&path).unwrap();
    let id = doc.id;
    doc.activate_view(11);
    doc.move_to(2, false);
    doc.insert(" dirty", false);
    end_of(&mut doc, 2);
    let dirty = doc.text.to_string();
    let selections = doc.selections();
    doc.activate_view(22);
    doc.type_character('}', options(ProfileId::Cpp, AutoIndent::Full), false)
        .unwrap();
    let edited = "猫🙂 dirty\r\n{\r\n}\r\n";
    assert_eq!(doc.text.to_string(), edited);
    assert_eq!(doc.view_state(Some(11)).cursor, doc.cursor);
    assert!(doc.dirty());
    assert_eq!(std::fs::read(&path).unwrap(), disk.as_bytes());
    assert_undo_redo(&mut doc, &dirty, &selections, edited);
    assert_eq!(doc.id, id);
    doc.save().unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), edited.as_bytes());
}
