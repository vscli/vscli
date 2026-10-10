use super::*;
use crate::{editing_profile::TypingOptions, snippet::Template};
use std::{collections::BTreeMap, sync::Arc};

fn select_view(document: &mut Document, passive: bool, id: u64) {
    if passive {
        document.display_view(id);
    } else {
        document.activate_view(id);
    }
}

fn pair_options() -> TypingOptions {
    TypingOptions {
        profile: crate::editing_profile::ProfileId::Cpp,
        ..Default::default()
    }
}

fn recreated_pair_history(passive: bool) {
    let mut document = Document::from_text("猫🙂\r\n");
    document.activate_view(1);
    document.move_to(2, false);
    document.activate_view(2);
    document.type_character('(', pair_options(), false).unwrap();
    document.type_character('x', pair_options(), false).unwrap();
    assert_eq!(document.text.to_string(), "猫🙂(x)\r\n");
    let retired_owner = document.session_owner.clone();
    let old_generation = document.undo.last().unwrap().pairs.generation;
    let epoch = document.text_epoch();
    let revision = document.revision;
    let save_generation = document.save_generation();
    let dirty = document.dirty();
    let undo_len = document.undo.len();
    document.activate_view(1);
    document.move_to(0, false);
    document.remove_view(2);
    select_view(&mut document, passive, 2);

    // Establish the actual clock collision without assigning a synthetic clock.
    // Ownership, rather than a numeric generation difference, must reject it.
    assert_eq!(document.pairs.generation, old_generation);
    assert_eq!(document.text_epoch(), epoch);
    assert_eq!(document.revision, revision);
    assert_eq!(document.save_generation(), save_generation);
    assert_eq!(document.dirty(), dirty);
    assert_eq!(document.undo.len(), undo_len);

    document.undo();
    assert_eq!(document.text.to_string(), "猫🙂()\r\n");
    assert_eq!(document.cursor, 3);
    // A manual closing character must insert, not regain retired overtyping.
    document.type_character(')', pair_options(), false).unwrap();
    assert_eq!(document.text.to_string(), "猫🙂())\r\n");
    assert_eq!(document.cursor, 4);
    assert!(!Arc::ptr_eq(&document.session_owner, &retired_owner));
    document.undo();
    assert_eq!(document.text.to_string(), "猫🙂()\r\n");
    document.redo();
    assert_eq!(document.text.to_string(), "猫🙂())\r\n");
    assert_eq!(document.save_generation(), save_generation);
}

#[test]
fn activated_recreated_view_does_not_restore_closed_target_pair_history() {
    recreated_pair_history(false);
}

#[test]
fn passively_recreated_view_does_not_restore_closed_target_pair_history() {
    recreated_pair_history(true);
}

fn recreated_snippet_history(passive: bool) {
    let mut document = Document::from_text("\r\n");
    document.activate_view(1);
    document
        .insert_snippet(&Template::parse("${1:x}$0").unwrap(), &BTreeMap::new())
        .unwrap();
    assert!(document.in_snippet());
    document.activate_view(2);
    document
        .insert_snippet(&Template::parse("${1:y}$0").unwrap(), &BTreeMap::new())
        .unwrap();
    document
        .type_character('z', Default::default(), false)
        .unwrap();
    assert_eq!(document.text.to_string(), "z\r\n");
    assert!(document.in_snippet());
    let retired_owner = document.session_owner.clone();
    let old_generation = document.undo.last().unwrap().snippet_generation;
    document.activate_view(1);
    // The source's real cancellation advances its copied generation to the
    // retired target's old generation. No test-only clock mutation is involved.
    assert!(document.in_snippet());
    document.cancel_snippet();
    assert_eq!(document.snippet_generation, old_generation);
    let epoch = document.text_epoch();
    let revision = document.revision;
    let save_generation = document.save_generation();
    let undo_len = document.undo.len();
    document.remove_view(2);
    select_view(&mut document, passive, 2);
    assert_eq!(document.snippet_generation, old_generation);
    assert!(!document.in_snippet());
    assert_eq!(document.text_epoch(), epoch);
    assert_eq!(document.revision, revision);
    assert_eq!(document.save_generation(), save_generation);
    assert_eq!(document.undo.len(), undo_len);

    document.undo();
    assert_eq!(document.text.to_string(), "y\r\n");
    assert_eq!(document.selection(), Some(0..1));
    assert!(!document.in_snippet());
    assert!(!Arc::ptr_eq(&document.session_owner, &retired_owner));
    assert!(!document.step_snippet(false).unwrap());
    document.redo();
    assert_eq!(document.text.to_string(), "z\r\n");
    assert!(!document.in_snippet());
    assert_eq!(document.save_generation(), save_generation);
}

#[test]
fn activated_recreated_view_does_not_restore_closed_target_snippet_history() {
    recreated_snippet_history(false);
}

#[test]
fn passively_recreated_view_does_not_restore_closed_target_snippet_history() {
    recreated_snippet_history(true);
}

#[test]
fn retained_view_switches_preserve_normal_pair_and_snippet_undo_redo() {
    let mut pairs = Document::from_text("猫🙂\r\n");
    pairs.activate_view(1);
    pairs.move_to(2, false);
    pairs.activate_view(2);
    pairs.type_character('(', pair_options(), false).unwrap();
    pairs.type_character('x', pair_options(), false).unwrap();
    let pair_owner = pairs.session_owner.clone();
    pairs.activate_view(1);
    pairs.display_view(2);
    assert!(Arc::ptr_eq(&pairs.session_owner, &pair_owner));
    pairs.undo();
    assert_eq!(pairs.text.to_string(), "猫🙂()\r\n");
    assert_eq!(pairs.cursor, 3);
    pairs.type_character(')', pair_options(), false).unwrap();
    assert_eq!(pairs.text.to_string(), "猫🙂()\r\n");
    assert_eq!(pairs.cursor, 4);
    pairs.redo();
    assert_eq!(pairs.text.to_string(), "猫🙂(x)\r\n");

    let mut snippet = Document::from_text("\r\n");
    snippet.activate_view(1);
    snippet.activate_view(2);
    snippet
        .insert_snippet(&Template::parse("${1:x}$0").unwrap(), &BTreeMap::new())
        .unwrap();
    snippet
        .type_character('y', Default::default(), false)
        .unwrap();
    let snippet_owner = snippet.session_owner.clone();
    snippet.display_view(1);
    snippet.activate_view(2);
    assert!(Arc::ptr_eq(&snippet.session_owner, &snippet_owner));
    snippet.undo();
    assert_eq!(snippet.text.to_string(), "x\r\n");
    assert!(snippet.in_snippet());
    snippet.redo();
    assert_eq!(snippet.text.to_string(), "y\r\n");
    assert!(snippet.in_snippet());
}
