use qa_console::{
    command_text::{self, TextError, Tokens},
    conversion::Text,
};
use qa_core::{primitives::RuleSetId, text::FixedText};

#[test]
fn native_arguments_borrow_high_bytes_without_reencoding_or_sanitizing() {
    let input = b"say raw\x80 \"quoted\xff text\" tail\xfe  \t";
    for rules in RuleSetId::ALL {
        let mut tokens = Tokens::default();
        let args = command_text::tokenize_bytes(input, rules, &mut tokens).unwrap();
        assert_eq!(
            args.iter_bytes().collect::<Vec<_>>(),
            [
                b"say".as_slice(),
                b"raw\x80",
                b"quoted\xff text",
                b"tail\xfe"
            ]
        );
        assert_eq!(args.get_bytes(4), b"");
        assert_eq!(args.get_bytes(usize::MAX), b"");
        assert_eq!(args.get_bytes(1).as_ptr(), input[4..].as_ptr());
        assert_eq!(args.get_bytes(2).as_ptr(), input[10..].as_ptr());
        assert_eq!(args.tail_bytes(2), b"\"quoted\xff text\" tail\xfe  \t");
        assert_eq!(args.tail_bytes(3), b"tail\xfe");
        assert_eq!(args.tail_bytes(4), b"");
        let raw = if matches!(rules, RuleSetId::Quake2 | RuleSetId::Quake2Rerelease) {
            b"raw\x80 \"quoted\xff text\" tail\xfe".as_slice()
        } else {
            b"raw\x80 \"quoted\xff text\" tail\xfe  \t".as_slice()
        };
        assert_eq!(args.raw_bytes(), raw);
        let mut joined = FixedText::<64>::default();
        args.join_bytes(1, &mut joined).unwrap();
        assert_eq!(joined.as_bytes(), b"raw\x80 quoted\xff text tail\xfe");
    }
}

#[test]
fn utf8_wrappers_and_byte_arguments_share_all_token_spans() {
    let input = "say café \"日本語 text\" : {x} // done\nnext";
    for rules in RuleSetId::ALL {
        let mut human = Tokens::default();
        let mut native = Tokens::default();
        let human = command_text::tokenize(input, rules, &mut human).unwrap();
        let native = command_text::tokenize_bytes(input.as_bytes(), rules, &mut native).unwrap();
        assert_eq!(human.len(), native.len());
        assert_eq!(human.raw.as_bytes(), native.raw_bytes());
        for index in 0..human.len() + 1 {
            assert_eq!(human.get(index).as_bytes(), native.get_bytes(index));
            assert_eq!(human.tail(index).as_bytes(), native.tail_bytes(index));
        }
        let mut human_joined = FixedText::<128>::default();
        let mut native_joined = FixedText::<128>::default();
        human.join(1, &mut human_joined).unwrap();
        native.join_bytes(1, &mut native_joined).unwrap();
        assert_eq!(human_joined.as_bytes(), native_joined.as_bytes());
    }
}

#[test]
fn byte_tokenizer_keeps_native_punctuation_comments_and_capacity_checks() {
    let input = b"cmd \x80:x \"\xff\" //skip\nnext";
    for (rules, expected) in [
        (
            RuleSetId::Quake,
            vec![b"cmd".as_slice(), b"\x80", b":", b"x", b"\xff", b"next"],
        ),
        (
            RuleSetId::QuakeWorld,
            vec![b"cmd".as_slice(), b"\x80:x", b"\xff", b"next"],
        ),
        (
            RuleSetId::Quake2,
            vec![b"cmd".as_slice(), b"\x80:x", b"\xff", b"next"],
        ),
        (
            RuleSetId::Quake2Rerelease,
            vec![b"cmd".as_slice(), b"\x80:x", b"\xff", b"next"],
        ),
        (
            RuleSetId::Quake3,
            vec![b"cmd".as_slice(), b"\x80:x", b"\xff"],
        ),
    ] {
        let mut tokens = Tokens::default();
        assert_eq!(
            command_text::tokenize_bytes(input, rules, &mut tokens)
                .unwrap()
                .iter_bytes()
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(
            command_text::tokenize_bytes(b"cmd \x80\0bad", rules, &mut tokens).err(),
            Some(TextError::Nul)
        );
        assert_eq!(
            command_text::tokenize_bytes(&[b'x'; 8193], rules, &mut tokens).err(),
            Some(TextError::TooLong)
        );
    }
    let mut tokens = Tokens::default();
    assert_eq!(
        command_text::tokenize_bytes(&[0x80; 1024], RuleSetId::Quake, &mut tokens).err(),
        Some(TextError::TooLong)
    );
    let args = command_text::tokenize_bytes(&[0x80; 128], RuleSetId::Quake2, &mut tokens).unwrap();
    assert_eq!(args.len(), 1);
    assert_eq!(args.get_bytes(0), b"");
    let mut quoted = [0x80; 130];
    quoted[0] = b'"';
    quoted[129] = b'"';
    assert_eq!(
        command_text::tokenize_bytes(&quoted, RuleSetId::Quake2, &mut tokens).err(),
        Some(TextError::TooLong)
    );
}

#[test]
fn classic_macros_reuse_the_parser_and_keep_surrounding_native_bytes() {
    let input = b"say \x80 $one \"$two\xff\" $two \xfe";
    let mut expanded = FixedText::default();
    expanded.set_bytes(input).unwrap();
    command_text::expand_bytes(&mut expanded, &mut Tokens::default(), |name| match name {
        b"one" => Some(Text::Borrowed("$two")),
        b"two" => Some(Text::Borrowed("café")),
        _ => None,
    })
    .unwrap();
    assert_eq!(
        expanded.as_bytes(),
        b"say \x80 caf\xc3\xa9 \"$two\xff\" caf\xc3\xa9 \xfe"
    );
}

#[test]
fn byte_macros_keep_unknown_name_quote_loop_and_length_rules() {
    for (input, expected) in [
        (
            b"echo \x80 $missing \xff".as_slice(),
            Ok(b"echo \x80  \xff".as_slice()),
        ),
        (b"echo \x80 \"$loop\xff\"", Ok(b"echo \x80 \"$loop\xff\"")),
        (b"echo \x80 $loop", Err(TextError::MacroLoop)),
        (
            b"echo \x80 \"unfinished\xff",
            Err(TextError::UnmatchedQuote),
        ),
    ] {
        let mut text = FixedText::default();
        text.set_bytes(input).unwrap();
        let result = command_text::expand_bytes(&mut text, &mut Tokens::default(), |name| {
            (name == b"loop").then_some(Text::Borrowed("$loop"))
        });
        match expected {
            Ok(bytes) => {
                assert_eq!(result, Ok(()));
                assert_eq!(text.as_bytes(), bytes);
            }
            Err(error) => assert_eq!(result, Err(error)),
        }
    }
    let mut text = FixedText::default();
    text.set_bytes(&[0x80; 1024]).unwrap();
    assert_eq!(
        command_text::expand_bytes(&mut text, &mut Tokens::default(), |_| None),
        Err(TextError::TooLong)
    );
}
