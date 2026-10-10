use std::sync::Arc;
use vscli::{
    document::{CommentOperation, Document, Selection},
    editing_profile::{AutoClosing, AutoIndent, PairHandling, ProfileId, TypingOptions},
    language_configuration::{Comments, Configuration, Delimiter, Identity, Pair},
};

fn configuration(language: &str) -> Configuration {
    Configuration {
        identity: Identity {
            owner: "test.typing".into(),
            version: "1.0.0".into(),
            package_path: "/package".into(),
            archive_sha256: "archive".into(),
            configuration_path: "language.json".into(),
            content_sha256: "a".repeat(64),
            composition_sha256: "b".repeat(64),
        },
        language: language.into(),
        auto_closing_pairs: None,
        surrounding_pairs: None,
        auto_close_before: None,
        brackets: None,
        comments: None,
    }
}
fn pair(open: char, close: char) -> Pair {
    Pair {
        open,
        close,
        not_string: false,
        not_comment: false,
    }
}
fn options(profile: ProfileId) -> TypingOptions {
    TypingOptions {
        profile,
        ..TypingOptions::default()
    }
}
fn bind(doc: &mut Document, configuration: Configuration) {
    doc.set_language_configuration(Some(Arc::new(configuration)))
        .unwrap();
}
fn comments(language: &str) -> Configuration {
    Configuration {
        comments: Some(Comments {
            line_comment: Some("※".into()),
            block_comment: Some(("«".into(), "»".into())),
        }),
        ..configuration(language)
    }
}
fn select(doc: &mut Document, anchor: usize, cursor: usize) {
    doc.set_selections(vec![Selection {
        anchor: Some(anchor),
        cursor,
        desired_column: None,
    }]);
}
fn undo_redo(doc: &mut Document, before: &str, original: &[Selection], after: &str) {
    let final_selections = doc.selections();
    doc.undo();
    assert_eq!(doc.text.to_string(), before);
    assert_eq!(doc.selections(), original);
    doc.redo();
    assert_eq!(doc.text.to_string(), after);
    assert_eq!(doc.selections(), final_selections);
}

#[test]
fn unrestricted_configured_pairs_work_without_unknown_language_lexical_claims() {
    let mut doc = Document::from_text("\r\n");
    bind(
        &mut doc,
        Configuration {
            auto_closing_pairs: Some(vec![
                pair('«', '»'),
                Pair {
                    not_string: true,
                    ..pair('"', '"')
                },
            ]),
            ..configuration("rust")
        },
    );
    let typing = options(ProfileId::Unsupported);
    doc.type_character('«', typing, false).unwrap();
    assert_eq!(doc.text.to_string(), "«»\r\n");
    doc.type_character('»', typing, false).unwrap();
    assert_eq!(doc.cursor, 2);
    doc.type_character('"', typing, false).unwrap();
    assert_eq!(doc.text.to_string(), "«»\"\r\n");
}

#[test]
fn brackets_only_unknown_configuration_derives_pairs_and_surround_without_overriding_builtins() {
    let config = Configuration {
        brackets: Some(vec![Delimiter {
            open: '«',
            close: '»',
        }]),
        ..configuration("fixture-native-language")
    };
    let typing = options(ProfileId::Unsupported);
    let mut doc = Document::from_text("\r\n");
    bind(&mut doc, config.clone());
    doc.type_character('«', typing, false).unwrap();
    assert_eq!(doc.text.to_string(), "«»\r\n");
    doc.backspace_with_options(typing, false).unwrap();
    assert_eq!(doc.text.to_string(), "\r\n");
    doc.undo();
    doc.type_character('»', typing, false).unwrap();
    assert_eq!(doc.cursor, 2);
    assert_eq!(doc.text.to_string(), "«»\r\n");

    let mut doc = Document::from_text("猫🙂");
    bind(&mut doc, config.clone());
    select(&mut doc, 2, 0);
    let original = doc.selections();
    doc.type_character('«', typing, false).unwrap();
    assert_eq!(doc.text.to_string(), "«猫🙂»");
    assert_eq!(doc.anchor, Some(1));
    assert_eq!(doc.cursor, 3);
    undo_redo(&mut doc, "猫🙂", &original, "«猫🙂»");

    for profile in [ProfileId::Cpp, ProfileId::Json] {
        let mut doc = Document::from_text("\r\n");
        bind(&mut doc, config.clone());
        doc.type_character('«', options(profile), false).unwrap();
        assert_eq!(doc.text.to_string(), "«\r\n");
        let mut doc = Document::from_text("\r\n");
        bind(&mut doc, config.clone());
        doc.type_character('{', options(profile), false).unwrap();
        assert_eq!(doc.text.to_string(), "{}\r\n");
    }
    let mut doc = Document::from_text("\r\n");
    bind(
        &mut doc,
        Configuration {
            auto_closing_pairs: Some(Vec::new()),
            ..config
        },
    );
    doc.type_character('«', typing, false).unwrap();
    assert_eq!(doc.text.to_string(), "«\r\n");
}

#[test]
fn configured_pair_restrictions_use_exact_builtin_lexical_proofs() {
    let config = Configuration {
        auto_closing_pairs: Some(vec![Pair {
            not_string: true,
            not_comment: true,
            ..pair('«', '»')
        }]),
        ..configuration("cpp")
    };
    for (text, cursor, expected) in [
        ("\r\n", 0, "«»\r\n"),
        ("\" \"\r\n", 1, "\"« \"\r\n"),
        ("// \r\n", 3, "// «\r\n"),
        ("R\"tag( )tag\"\r\n", 7, "R\"tag( «)tag\"\r\n"),
    ] {
        let mut doc = Document::from_text(text);
        bind(&mut doc, config.clone());
        doc.move_to(cursor, false);
        doc.type_character('«', options(ProfileId::Cpp), false)
            .unwrap();
        assert_eq!(doc.text.to_string(), expected);
    }
}

#[test]
fn omitted_surround_keeps_explicit_cpp_builtin_but_derives_unknown_table_and_empty_disables() {
    let config = Configuration {
        auto_closing_pairs: Some(vec![pair('«', '»')]),
        ..configuration("cpp")
    };
    let mut doc = Document::from_text("猫🙂");
    bind(&mut doc, config.clone());
    select(&mut doc, 2, 0);
    doc.type_character('<', options(ProfileId::Cpp), false)
        .unwrap();
    assert_eq!(doc.text.to_string(), "<猫🙂>");
    let mut doc = Document::from_text("猫🙂");
    bind(&mut doc, config);
    select(&mut doc, 0, 2);
    doc.type_character('«', options(ProfileId::Cpp), false)
        .unwrap();
    assert_eq!(doc.text.to_string(), "«");
    let mut doc = Document::from_text("猫🙂");
    bind(
        &mut doc,
        Configuration {
            auto_closing_pairs: Some(vec![pair('«', '»')]),
            ..configuration("rust")
        },
    );
    select(&mut doc, 2, 0);
    let original = doc.selections();
    doc.type_character('«', options(ProfileId::Unsupported), false)
        .unwrap();
    assert_eq!(doc.text.to_string(), "«猫🙂»");
    assert_eq!(doc.anchor, Some(1));
    assert_eq!(doc.cursor, 3);
    undo_redo(&mut doc, "猫🙂", &original, "«猫🙂»");
    let mut doc = Document::from_text("\r\n");
    bind(
        &mut doc,
        Configuration {
            auto_closing_pairs: Some(Vec::new()),
            surrounding_pairs: Some(Vec::new()),
            ..configuration("cpp")
        },
    );
    doc.type_character('(', options(ProfileId::Cpp), false)
        .unwrap();
    assert_eq!(doc.text.to_string(), "(\r\n");
}

#[test]
fn auto_close_before_and_settings_policy_preserve_fieldwise_precedence() {
    let config = Configuration {
        auto_closing_pairs: Some(vec![pair('«', '»')]),
        auto_close_before: Some("!".into()),
        ..configuration("rust")
    };
    for (text, policy, expected) in [
        ("!", AutoClosing::LanguageDefined, "«»!"),
        (" ", AutoClosing::LanguageDefined, "« "),
        (" ", AutoClosing::BeforeWhitespace, "«» "),
        ("x", AutoClosing::Always, "«»x"),
        ("!", AutoClosing::Never, "«!"),
    ] {
        let mut doc = Document::from_text(text);
        bind(&mut doc, config.clone());
        doc.type_character(
            '«',
            TypingOptions {
                brackets: policy,
                ..options(ProfileId::Unsupported)
            },
            false,
        )
        .unwrap();
        assert_eq!(doc.text.to_string(), expected);
    }
}

#[test]
fn configured_multi_pair_delete_skip_and_literal_insertions_preserve_owned_undo() {
    let config = Configuration {
        auto_closing_pairs: Some(vec![pair('«', '»')]),
        ..configuration("rust")
    };
    let typing = options(ProfileId::Unsupported);
    let before = "猫🙂\r\nabc\r\n";
    let mut doc = Document::from_text(before);
    bind(&mut doc, config.clone());
    doc.set_selections(vec![Selection::caret(7), Selection::caret(2)]);
    let original = doc.selections();
    doc.type_character('«', typing, false).unwrap();
    doc.backspace_with_options(typing, false).unwrap();
    assert_eq!(doc.text.to_string(), before);
    doc.undo();
    assert_eq!(doc.text.to_string(), "猫🙂«»\r\nabc«»\r\n");
    doc.type_character('»', typing, false).unwrap();
    assert_eq!(doc.text.to_string(), "猫🙂«»\r\nabc«»\r\n");
    doc.undo();
    assert_eq!(doc.text.to_string(), before);
    assert_eq!(doc.selections(), original);
    for literal in [true, false] {
        let mut doc = Document::from_text("\r\n");
        bind(&mut doc, config.clone());
        if literal {
            doc.insert("«»", false);
        } else {
            doc.apply_changes(vec![(0..0, "«»".into())]);
        }
        doc.move_to(1, false);
        doc.type_character('»', typing, false).unwrap();
        assert_eq!(doc.text.to_string(), "«»»\r\n");
    }
}

#[test]
fn equal_snapshot_preserves_ownership_but_source_changes_and_a_b_a_never_revive_it() {
    let typing = options(ProfileId::Unsupported);
    let a = Configuration {
        auto_closing_pairs: Some(vec![pair('«', '»')]),
        ..configuration("rust")
    };
    let mut doc = Document::from_text("\r\n");
    bind(&mut doc, a.clone());
    doc.type_character('«', typing, false).unwrap();
    bind(&mut doc, a.clone());
    doc.type_character('»', typing, false).unwrap();
    assert_eq!(doc.text.to_string(), "«»\r\n");
    doc.undo();
    doc.redo();
    doc.move_to(1, false);
    let mut b = a.clone();
    b.identity.version = "2.0.0".into();
    bind(&mut doc, b);
    bind(&mut doc, a);
    doc.undo();
    doc.redo();
    doc.move_to(1, false);
    doc.type_character('»', typing, false).unwrap();
    assert_eq!(doc.text.to_string(), "«»»\r\n");
}

#[test]
fn invalid_direct_configuration_rejects_before_retirement_and_preserves_redo() {
    let typing = options(ProfileId::Unsupported);
    let a = Configuration {
        auto_closing_pairs: Some(vec![pair('«', '»')]),
        ..configuration("rust")
    };
    for invalid in [
        Configuration {
            auto_closing_pairs: Some(vec![pair('(', ')'); 65]),
            ..a.clone()
        },
        Configuration {
            auto_close_before: Some("x".repeat(257)),
            ..a.clone()
        },
        Configuration {
            comments: Some(Comments {
                line_comment: Some("\0".into()),
                block_comment: None,
            }),
            ..a.clone()
        },
    ] {
        let mut doc = Document::from_text("\r\n");
        bind(&mut doc, a.clone());
        doc.type_character('«', typing, false).unwrap();
        doc.insert("x", false);
        doc.undo();
        let revision = doc.revision;
        let original = doc.selections();
        assert!(
            doc.set_language_configuration(Some(Arc::new(invalid)))
                .is_err()
        );
        assert_eq!(doc.revision, revision);
        assert_eq!(doc.selections(), original);
        doc.type_character('»', typing, false).unwrap();
        assert_eq!(doc.text.to_string(), "«»\r\n");
        doc.redo();
        assert_eq!(doc.text.to_string(), "«x»\r\n");
    }
}

#[test]
fn configured_bracket_enter_uses_current_code_proof_and_empty_disables_basic_rule() {
    for brackets in [
        vec![Delimiter {
            open: '<',
            close: '>',
        }],
        Vec::new(),
    ] {
        let mut doc = Document::from_text("<>\r\n");
        bind(
            &mut doc,
            Configuration {
                brackets: Some(brackets.clone()),
                ..configuration("cpp")
            },
        );
        doc.move_to(1, false);
        doc.newline_with_options(TypingOptions {
            indent: AutoIndent::Brackets,
            ..options(ProfileId::Cpp)
        })
        .unwrap();
        assert_eq!(
            doc.text.to_string(),
            if brackets.is_empty() {
                "<\r\n>\r\n"
            } else {
                "<\r\n    \r\n>\r\n"
            }
        );
    }
}

#[test]
fn configured_line_comments_align_and_preserve_crlf_unicode_dirty_disk_and_undo() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("main.rs");
    let before = "  猫🙂\r\n    run();\r\n\r\n";
    std::fs::write(&path, before).unwrap();
    let mut doc = Document::open(&path).unwrap();
    let id = doc.id;
    bind(&mut doc, comments("rust"));
    let end = doc.len() - 2;
    select(&mut doc, end, 0);
    let original = doc.selections();
    doc.comment_lines(CommentOperation::Toggle).unwrap();
    let after = "※   猫🙂\r\n※     run();\r\n\r\n";
    assert_eq!(doc.text.to_string(), after);
    assert!(doc.dirty());
    assert_eq!(doc.id, id);
    assert_eq!(std::fs::read(&path).unwrap(), before.as_bytes());
    undo_redo(&mut doc, before, &original, after);
    doc.save().unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), after.as_bytes());
    doc.comment_lines(CommentOperation::Toggle).unwrap();
    assert_eq!(doc.text.to_string(), before);
}

#[test]
fn line_command_reference_boundaries_and_sorted_primary_restore_original_order_on_undo() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("main.cpp");
    let before = "α();\r\nβ();\r\n";
    std::fs::write(&path, before).unwrap();
    let mut doc = Document::open(&path).unwrap();
    doc.set_selections(vec![Selection::caret(7), Selection::caret(1)]);
    let original = doc.selections();
    doc.comment_lines(CommentOperation::Toggle).unwrap();
    let after = "// α();\r\n// β();\r\n";
    assert_eq!(doc.text.to_string(), after);
    assert_eq!(
        doc.selections(),
        vec![Selection::caret(4), Selection::caret(13)]
    );
    assert_eq!(std::fs::read(&path).unwrap(), before.as_bytes());
    undo_redo(&mut doc, before, &original, after);
    doc.save().unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), after.as_bytes());

    for (anchor, cursor, expected_anchor, expected_cursor) in [(0, 16, 3, 22), (16, 0, 22, 3)] {
        let before = "  α();\r\n    β();\r\n";
        let mut doc = Document::from_text(before);
        doc.path = Some(path.clone());
        select(&mut doc, anchor, cursor);
        let original = doc.selections();
        doc.comment_lines(CommentOperation::Toggle).unwrap();
        assert_eq!(doc.text.to_string(), "//   α();\r\n//     β();\r\n");
        assert_eq!(doc.anchor, Some(expected_anchor));
        assert_eq!(doc.cursor, expected_cursor);
        undo_redo(&mut doc, before, &original, "//   α();\r\n//     β();\r\n");
    }
}

#[test]
fn add_remove_comments_and_explicit_missing_delimiters_preserve_policy_and_history() {
    let mut doc = Document::from_text("※ old\r\n\r\n");
    bind(&mut doc, comments("rust"));
    doc.comment_lines(CommentOperation::Add).unwrap();
    assert_eq!(doc.text.to_string(), "※ ※ old\r\n\r\n");
    doc.comment_lines(CommentOperation::Remove).unwrap();
    assert_eq!(doc.text.to_string(), "※ old\r\n\r\n");
    let mut config = comments("rust");
    config.comments.as_mut().unwrap().line_comment = None;
    config.comments.as_mut().unwrap().block_comment = None;
    bind(&mut doc, config);
    let before = doc.text.to_string();
    let revision = doc.revision;
    doc.comment_lines(CommentOperation::Toggle).unwrap();
    assert_eq!(doc.text.to_string(), before);
    assert_eq!(doc.revision, revision);
    doc.undo();
    assert_eq!(doc.text.to_string(), "※ ※ old\r\n\r\n");
}

#[test]
fn add_on_all_blank_lines_is_noop_and_preserves_the_prior_edit_undo_redo() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("main.cpp");
    let initial = "猫🙂\r\n";
    std::fs::write(&path, initial).unwrap();
    for seed in ["\r\n", "\t  ", "\r\n \t\r\n  "] {
        let mut doc = Document::open(&path).unwrap();
        let initial_end = doc.len();
        doc.move_to(initial_end, false);
        doc.insert(seed, false);
        let text = doc.text.to_string();
        let revision = doc.revision;
        let dirty = doc.dirty();
        let id = doc.id;
        let end = doc.len();
        if seed.contains(' ') && seed.contains('\r') {
            let start = doc.line_start(1);
            select(&mut doc, end, start);
        }
        let selection = doc.selections();
        let cursor = doc.cursor;
        let epoch = doc.typing_contexts(&[cursor], ProfileId::Cpp)[0].text_epoch;
        doc.comment_lines(CommentOperation::Add).unwrap();
        assert_eq!(doc.text.to_string().as_bytes(), text.as_bytes());
        assert_eq!(doc.selections(), selection);
        assert_eq!(doc.revision, revision);
        assert_eq!(
            doc.typing_contexts(&[cursor], ProfileId::Cpp)[0].text_epoch,
            epoch
        );
        assert_eq!(doc.dirty(), dirty);
        assert_eq!(doc.id, id);
        assert_eq!(std::fs::read(&path).unwrap(), initial.as_bytes());
        doc.undo();
        assert_eq!(doc.text.to_string(), initial);
        // The seed snapshot was captured after moving to the original EOF.
        assert_eq!(doc.cursor, initial.chars().count());
        assert!(!doc.dirty());
        doc.redo();
        assert_eq!(doc.text.to_string(), text);
        assert_eq!(doc.selections(), selection);
        assert!(doc.dirty());
        assert_eq!(doc.id, id);
        assert_eq!(std::fs::read(&path).unwrap(), initial.as_bytes());
    }
}

#[test]
fn block_only_line_command_wraps_full_lines_and_line_only_block_command_is_noop() {
    let mut config = comments("rust");
    config.comments.as_mut().unwrap().line_comment = None;
    let mut doc = Document::from_text("  猫🙂\r\n");
    bind(&mut doc, config);
    doc.move_to(3, false);
    let original = doc.selections();
    doc.comment_lines(CommentOperation::Toggle).unwrap();
    assert_eq!(doc.text.to_string(), "  « 猫🙂 »\r\n");
    undo_redo(&mut doc, "  猫🙂\r\n", &original, "  « 猫🙂 »\r\n");
    let mut config = comments("rust");
    config.comments.as_mut().unwrap().block_comment = None;
    let mut doc = Document::from_text("猫🙂\r\n");
    bind(&mut doc, config);
    doc.insert("dirty", false);
    doc.undo();
    doc.toggle_block_comment().unwrap();
    assert_eq!(doc.text.to_string(), "猫🙂\r\n");
    doc.redo();
    assert_eq!(doc.text.to_string(), "dirty猫🙂\r\n");
}

#[test]
fn block_wrapping_adjacent_reversed_selections_and_empty_caret_have_atomic_history() {
    let mut doc = Document::from_text("猫🙂ab\r\n");
    bind(&mut doc, comments("rust"));
    doc.set_selections(vec![
        Selection {
            anchor: Some(4),
            cursor: 2,
            desired_column: None,
        },
        Selection {
            anchor: Some(2),
            cursor: 0,
            desired_column: None,
        },
    ]);
    let original = doc.selections();
    doc.toggle_block_comment().unwrap();
    assert_eq!(doc.text.to_string(), "« 猫🙂 »« ab »\r\n");
    assert!(doc.selections()[0].cursor > doc.selections()[1].cursor);
    undo_redo(&mut doc, "猫🙂ab\r\n", &original, "« 猫🙂 »« ab »\r\n");
    doc.toggle_block_comment().unwrap();
    assert_eq!(doc.text.to_string(), "猫🙂ab\r\n");
    let mut doc = Document::from_text("\r\n");
    bind(&mut doc, comments("rust"));
    let original = doc.selections();
    doc.toggle_block_comment().unwrap();
    assert_eq!(doc.text.to_string(), "«  »\r\n");
    assert_eq!(doc.cursor, 2);
    undo_redo(&mut doc, "\r\n", &original, "«  »\r\n");
    doc.toggle_block_comment().unwrap();
    assert_eq!(doc.text.to_string(), "\r\n");
}

#[test]
fn expanded_block_removal_and_comment_budget_reject_without_mutating_redo() {
    for long in [false, true] {
        let text = if long {
            format!("{}\r\n", " ".repeat(64 * 1024 + 1))
        } else {
            "« ab »\r\n".into()
        };
        let mut doc = Document::from_text(&text);
        bind(&mut doc, comments("rust"));
        doc.move_to(doc.len(), false);
        doc.insert("dirty", false);
        doc.undo();
        if long {
            doc.move_to(0, false);
        } else {
            doc.set_selections(vec![Selection::caret(2), Selection::caret(3)]);
        }
        let revision = doc.revision;
        let original = doc.selections();
        let result = if long {
            doc.comment_lines(CommentOperation::Toggle)
        } else {
            doc.toggle_block_comment()
        };
        assert!(result.is_err());
        assert_eq!(doc.text.to_string(), text);
        assert_eq!(doc.revision, revision);
        assert_eq!(doc.selections(), original);
        doc.redo();
        assert_eq!(doc.text.to_string(), text + "dirty");
    }
}

#[test]
fn source_transition_retires_all_shared_views_without_changing_unsaved_model_identity() {
    let typing = options(ProfileId::Unsupported);
    let a = Configuration {
        auto_closing_pairs: Some(vec![pair('«', '»')]),
        ..configuration("rust")
    };
    let mut doc = Document::from_text("\r\n");
    let id = doc.id;
    bind(&mut doc, a.clone());
    doc.activate_view(10);
    doc.type_character('«', typing, false).unwrap();
    doc.activate_view(20);
    doc.move_to(1, false);
    let mut b = a.clone();
    b.identity.composition_sha256 = "c".repeat(64);
    bind(&mut doc, b);
    bind(&mut doc, a);
    doc.activate_view(10);
    doc.type_character('»', typing, false).unwrap();
    assert_eq!(doc.text.to_string(), "«»»\r\n");
    assert_eq!(doc.id, id);
    assert!(doc.dirty());
    doc.undo();
    doc.activate_view(20);
    doc.move_to(1, false);
    doc.type_character(
        '»',
        TypingOptions {
            overtype: PairHandling::Auto,
            ..typing
        },
        false,
    )
    .unwrap();
    assert_eq!(doc.text.to_string(), "«»»\r\n");
}
