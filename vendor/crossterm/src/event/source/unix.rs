#[cfg(feature = "use-dev-tty")]
pub(crate) mod tty;

#[cfg(not(feature = "use-dev-tty"))]
pub(crate) mod mio;

#[cfg(feature = "use-dev-tty")]
pub(crate) use self::tty::UnixInternalEventSource;

#[cfg(not(feature = "use-dev-tty"))]
pub(crate) use self::mio::UnixInternalEventSource;

// One legacy Escape may immediately precede another sequence's Escape prefix.
// Consume only the pending key; the current byte still belongs to the parser.
fn take_pending_escape(buffer: &mut Vec<u8>, next: u8) -> Option<crate::event::InternalEvent> {
    if next == b'\x1B' && buffer == b"\x1B" {
        buffer.clear();
        Some(crate::event::InternalEvent::Event(
            crate::event::Event::Key(crate::event::KeyCode::Esc.into()),
        ))
    } else {
        None
    }
}

#[cfg(test)]
mod adjacent_escape_tests {
    #[cfg(not(feature = "use-dev-tty"))]
    use super::mio::Parser;
    #[cfg(feature = "use-dev-tty")]
    use super::tty::Parser;
    use crate::event::{Event, InternalEvent, KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> InternalEvent {
        InternalEvent::Event(Event::Key(code.into()))
    }
    fn modified(code: KeyCode, modifiers: KeyModifiers) -> InternalEvent {
        InternalEvent::Event(Event::Key(KeyEvent::new(code, modifiers)))
    }
    fn parse(chunks: &[(&[u8], bool)]) -> Vec<InternalEvent> {
        let mut parser = Parser::default();
        for (bytes, more) in chunks {
            parser.advance(bytes, *more);
        }
        parser.collect()
    }

    #[test]
    fn escape_then_f1_in_one_read_retains_following_command_input_and_enter() {
        let bytes = b"\x1B\x1BOPLanguage: Parameter Hints\r";
        let mut expected = vec![key(KeyCode::Esc), key(KeyCode::F(1))];
        for character in "Language: Parameter Hints".chars() {
            expected.push(modified(
                KeyCode::Char(character),
                if character.is_uppercase() {
                    KeyModifiers::SHIFT
                } else {
                    KeyModifiers::NONE
                },
            ));
        }
        expected.push(key(KeyCode::Enter));
        assert_eq!(parse(&[(bytes, false)]), expected);
    }

    #[test]
    fn escape_then_f1_survives_every_buffer_split_with_positive_more_proof() {
        let bytes = b"\x1B\x1BOPx\r";
        let expected = vec![
            key(KeyCode::Esc),
            key(KeyCode::F(1)),
            key(KeyCode::Char('x')),
            key(KeyCode::Enter),
        ];
        for split in 1..bytes.len() {
            assert_eq!(
                parse(&[(&bytes[..split], true), (&bytes[split..], false)]),
                expected,
                "split {split} must not consume the next escape prefix"
            );
        }
        // A standalone Escape read is already positively decoded as a key.
        assert_eq!(parse(&[(b"\x1B", false), (b"\x1BOPx\r", false)]), expected);
    }

    #[test]
    fn repeated_escape_keys_are_not_collapsed_and_partial_ss3_remains_valid() {
        for count in 1..=8 {
            let bytes = vec![b'\x1B'; count];
            let expected = vec![key(KeyCode::Esc); count];
            assert_eq!(parse(&[(&bytes, false)]), expected);
        }
        assert_eq!(
            parse(&[(b"\x1B\x1B", true), (b"\x1B", false)]),
            vec![key(KeyCode::Esc); 3]
        );
        assert_eq!(
            parse(&[(b"\x1BO", false), (b"P", false)]),
            vec![key(KeyCode::F(1))]
        );
        assert_eq!(
            parse(&[(b"\x1B", true), (b"O", true), (b"P", false)]),
            vec![key(KeyCode::F(1))]
        );
    }

    #[test]
    fn csi_and_bracketed_paste_preserve_embedded_adjacent_escapes() {
        assert_eq!(
            parse(&[(b"\x1B\x1B[D", false)]),
            vec![key(KeyCode::Esc), key(KeyCode::Left)]
        );
        #[cfg(feature = "bracketed-paste")]
        {
            let bytes = "\u{1b}\u{1b}[200~x\u{1b}\u{1b}OP猫🙂\u{1b}[201~".as_bytes();
            let expected = vec![
                key(KeyCode::Esc),
                InternalEvent::Event(Event::Paste("x\u{1b}\u{1b}OP猫🙂".into())),
            ];
            assert_eq!(parse(&[(bytes, false)]), expected);
            for split in 1..bytes.len() {
                assert_eq!(
                    parse(&[(&bytes[..split], true), (&bytes[split..], false)]),
                    expected
                );
            }
        }
    }

    #[test]
    fn legacy_alt_keys_and_enhanced_alt_escape_keep_their_modifiers() {
        assert_eq!(
            parse(&[(b"\x1Bc", false)]),
            vec![modified(KeyCode::Char('c'), KeyModifiers::ALT)]
        );
        assert_eq!(
            parse(&[(b"\x1BH", false)]),
            vec![modified(
                KeyCode::Char('H'),
                KeyModifiers::ALT | KeyModifiers::SHIFT
            )]
        );
        assert_eq!(
            parse(&[(b"\x1B\x14", false)]),
            vec![modified(
                KeyCode::Char('t'),
                KeyModifiers::ALT | KeyModifiers::CONTROL
            )]
        );
        assert_eq!(
            parse(&[(b"\x1B[27;3u", false)]),
            vec![modified(KeyCode::Esc, KeyModifiers::ALT)]
        );
    }

    #[test]
    fn ordinary_utf8_sequences_still_decode_across_every_byte_split() {
        let bytes = "猫🙂Ü".as_bytes();
        let expected = vec![
            key(KeyCode::Char('猫')),
            key(KeyCode::Char('🙂')),
            modified(KeyCode::Char('Ü'), KeyModifiers::SHIFT),
        ];
        for split in 1..bytes.len() {
            assert_eq!(
                parse(&[(&bytes[..split], true), (&bytes[split..], false)]),
                expected
            );
        }
    }

    #[test]
    fn adjacent_escape_at_a_full_tty_batch_boundary_preserves_all_events() {
        let mut first = vec![b'x'; 1023];
        first.push(b'\x1B');
        let mut expected = vec![key(KeyCode::Char('x')); 1023];
        expected.extend([key(KeyCode::Esc), key(KeyCode::F(1)), key(KeyCode::Enter)]);
        let actual = parse(&[(&first, true), (b"\x1BOP\r", false)]);
        assert_eq!(actual.len(), 1026);
        assert_eq!(actual, expected);
    }
}
