use qa_formats::{FormatError, text::Tokens};

#[test]
fn data_words_borrow_quoted_paths_without_treating_punctuation_as_commands() {
    let bytes = b"\"music/intro theme.ogg\" \"music/loop,//literal.wav\" //path /*path*/";
    let mut words = Tokens::words(bytes);
    let first = words.next().unwrap().unwrap();
    assert_eq!(first, b"music/intro theme.ogg");
    assert_eq!(first.as_ptr(), bytes[1..].as_ptr());
    assert_eq!(
        words.next().unwrap(),
        Some(&b"music/loop,//literal.wav"[..])
    );
    assert_eq!(words.next().unwrap(), Some(&b"//path"[..]));
    assert_eq!(words.next().unwrap(), Some(&b"/*path*/"[..]));
    assert_eq!(words.next().unwrap(), None);
    assert_eq!(
        Tokens::words(b"\"unfinished").next(),
        Err(FormatError::Truncated)
    );
}

#[test]
fn existing_format_comment_rules_still_skip_line_and_block_comments() {
    let mut words = Tokens::new(b"//ignored\nfirst /* ignored */ second");
    assert_eq!(words.next().unwrap(), Some(&b"first"[..]));
    assert_eq!(words.next().unwrap(), Some(&b"second"[..]));
    assert_eq!(words.next().unwrap(), None);
}
