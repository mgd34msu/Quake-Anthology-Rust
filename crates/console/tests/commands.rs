use qa_console::{
    command_buffer::CommandBuffer,
    command_text::{self, TextError},
    commands::{CommandError, Console, Host, ScriptError},
    cvars_generated::DEFINITIONS,
    views::{Context, Source},
};
use std::fmt::{Arguments, Write};

#[derive(Default)]
struct Capture {
    output: String,
    quit: bool,
    marks: Vec<String>,
}
impl Host for Capture {
    fn print(&mut self, text: Arguments<'_>) {
        self.output.write_fmt(text).unwrap();
    }
    fn read_script(&mut self, path: &str, destination: &mut [u8]) -> Result<usize, ScriptError> {
        let text = match path {
            "inner.cfg" => "mark inside\n",
            "outer.cfg" => "mark before; exec inner; mark after\n",
            _ => return Err(ScriptError::Missing),
        };
        destination[..text.len()].copy_from_slice(text.as_bytes());
        Ok(text.len())
    }
    fn quit(&mut self) {
        self.quit = true;
    }
}
fn context(source: Source) -> Context {
    Context {
        source,
        ..Context::default()
    }
}
fn mark(
    _: &mut Console<Capture>,
    host: &mut Capture,
    args: &command_text::Arguments<'_>,
    _: Context,
) -> Result<(), CommandError> {
    host.marks.push(args.tail(1).to_owned());
    Ok(())
}

fn tokens(text: &str, source: Source) -> Vec<String> {
    let mut storage = command_text::Tokens::default();
    command_text::tokenize(text, source, &mut storage)
        .unwrap()
        .iter()
        .map(str::to_owned)
        .collect()
}
fn expanded(
    text: &str,
    values: impl FnMut(&str) -> Option<qa_console::conversion::Text<'static>>,
) -> Result<String, TextError> {
    let mut output = qa_console::text::FixedText::default();
    output.set(text).unwrap();
    command_text::expand(&mut output, &mut command_text::Tokens::default(), values)?;
    Ok(output.as_str().to_owned())
}
#[test]
fn source_tokenizers_keep_original_quotes_comments_punctuation_and_limits() {
    let text = "echo a\"b c\" x:y {q} //end\nnext";
    assert_eq!(
        tokens(text, Source::Quake),
        ["echo", "a\"b", "c\"", "x", ":", "y", "{", "q", "}", "next"]
    );
    assert_eq!(
        tokens(text, Source::Quake2),
        ["echo", "a\"b", "c\"", "x:y", "{q}", "next"]
    );
    assert_eq!(
        tokens(text, Source::Quake3),
        ["echo", "a", "b c", "x:y", "{q}"]
    );
    assert_eq!(
        tokens("echo /* skip */one\"two\" //rest", Source::Quake3),
        ["echo", "one", "two"]
    );
    for source in Source::ALL {
        assert_eq!(tokens("echo \"unfinished", source), ["echo", "unfinished"]);
        let many = "x ".repeat(1100);
        assert_eq!(
            tokens(&many, source).len(),
            if source == Source::Quake3 { 1024 } else { 80 }
        );
    }
}

#[test]
fn q2_macros_repeat_outside_quotes_and_reject_only_the_bad_command() {
    let values = |name: &str| {
        Some(qa_console::conversion::Text::Borrowed(match name {
            "a" => "$b",
            "b" => "3",
            "loop" => "$loop",
            _ => "",
        }))
    };
    assert_eq!(expanded("echo $a \"$a\"", values).unwrap(), "echo 3 \"$a\"");
    assert_eq!(expanded("echo $loop", values), Err(TextError::MacroLoop));
    assert_eq!(
        expanded("echo \"unfinished", values),
        Err(TextError::UnmatchedQuote)
    );
}

#[test]
fn buffer_preserves_append_insert_quotes_source_context_and_overflow_atomicity() {
    for source in Source::ALL {
        let context = context(source);
        let mut buffer = CommandBuffer::new();
        buffer.append("echo he", context).unwrap();
        buffer.append("llo; echo \"a;b\"\n", context).unwrap();
        buffer.insert("echo first\n", context).unwrap();
        let mut lines = Vec::new();
        let mut line = qa_console::text::FixedText::<65536>::default();
        while let Some((source, result)) = buffer.next_line(&mut line) {
            result.unwrap();
            assert_eq!(source, context);
            if !line.as_str().trim().is_empty() {
                lines.push(line.as_str().to_owned());
            }
        }
        assert_eq!(lines, ["echo first", "echo hello", " echo \"a;b\""]);
        buffer.append("kept\n", context).unwrap();
        assert_eq!(
            buffer.append(&"x".repeat(65536), context),
            Err(TextError::TooLong)
        );
        assert_eq!(
            {
                buffer.next_line(&mut line).unwrap().1.unwrap();
                line.as_str()
            },
            "kept"
        );
    }
    let q1 = context(Source::Quake);
    let mut buffer = CommandBuffer::new();
    buffer.append("fix\n", q1).unwrap();
    buffer.insert("pre", q1).unwrap();
    let mut line = qa_console::text::FixedText::<65536>::default();
    buffer.next_line(&mut line).unwrap().1.unwrap();
    assert_eq!(line.as_str(), "prefix");
}

#[test]
fn one_table_executes_alias_wait_nested_exec_and_cvars_in_every_source() {
    for source in Source::ALL {
        let context = context(source);
        let mut console = Console::<Capture>::new(context);
        let mut host = Capture::default();
        assert!(console.register("mark", mark));
        console.append("alias a \"mark hi; wait; mark there\"; a; fov 110; cg_fov; exec outer; quit; mark unreachable\n", context).unwrap();
        console.execute_frame(&mut host);
        assert_eq!(host.marks, ["hi"]);
        assert!(!console.idle());
        console.execute_frame(&mut host);
        assert_eq!(host.marks, ["hi", "there", "before", "inside", "after"]);
        assert_eq!(
            console.cvars.value(console.cvars.find("cg_fov").unwrap()),
            110.0
        );
        assert!(host.quit && console.idle());
    }
}

#[test]
fn unknown_text_and_recursive_alias_stay_scoped_and_never_become_chat() {
    let mut console = Console::<Capture>::new(Context::default());
    let mut host = Capture::default();
    console.register("mark", mark);
    console
        .append(
            "alias loop loop; loop; unknown_text; mark reached\n",
            Context::default(),
        )
        .unwrap();
    console.execute_frame(&mut host);
    assert!(console.idle());
    assert!(!host.quit);
    assert_eq!(host.marks, ["reached"]);
}

#[test]
fn cvarlist_has_every_owner_definition_once_with_seat_values_indented() {
    let mut console = Console::<Capture>::new(Context::default());
    let mut host = Capture::default();
    console.append("cvarlist\n", Context::default()).unwrap();
    console.execute_frame(&mut host);
    let mut names: Vec<_> = host
        .output
        .lines()
        .filter(|l| !l.starts_with(' '))
        .filter_map(|l| l.split_whitespace().next())
        .collect();
    let mut expected: Vec<_> = DEFINITIONS.iter().map(|d| d.name).collect();
    names.sort_unstable();
    expected.sort_unstable();
    assert_eq!(names, expected);
    assert!(host.output.contains("  ui_seat1_language"));
}

#[test]
fn bare_assignment_uses_argv_one_and_set_retains_native_source_rules() {
    for source in Source::ALL {
        let context = context(source);
        let mut console = Console::<Capture>::new(context);
        let mut host = Capture::default();
        console
            .append("fov 100 ignored; sensitivity \"3 4\" ignored\n", context)
            .unwrap();
        console.execute_frame(&mut host);
        assert_eq!(
            console.cvars.text(console.cvars.find("cg_fov").unwrap()),
            "100"
        );
        assert_eq!(
            console
                .cvars
                .text(console.cvars.find("sensitivity").unwrap()),
            "3 4"
        );
        console
            .append("set sensitivity \"a b\" c\n", context)
            .unwrap();
        console.execute_frame(&mut host);
        if matches!(source, Source::Quake2 | Source::Quake2Rerelease) {
            assert_eq!(
                console
                    .cvars
                    .text(console.cvars.find("sensitivity").unwrap()),
                "3 4"
            );
            assert!(host.output.contains("Usage"));
            console
                .append("set sensitivity \"a b\" u\n", context)
                .unwrap();
            console.execute_frame(&mut host);
            let view = console.cvars.bind("sensitivity", context).unwrap();
            assert_eq!(console.cvars.text(view.canonical()), "a b");
            assert_eq!(console.cvars.flags(view), 2);
            let q3 = console
                .cvars
                .bind("sensitivity", Context::default())
                .unwrap();
            assert_ne!(console.cvars.flags(q3), 2);
        } else {
            assert_eq!(
                console
                    .cvars
                    .text(console.cvars.find("sensitivity").unwrap()),
                "a b c"
            );
        }
    }
}

#[test]
fn buffer_spans_preserve_mixed_sources_after_front_inserts_and_oversized_lines() {
    let mut buffer = CommandBuffer::new();
    let mut line = qa_console::text::FixedText::<8192>::default();
    let q1 = context(Source::Quake);
    let q2 = context(Source::Quake2);
    let q3 = context(Source::Quake3);
    buffer.append("q1", q1).unwrap();
    buffer.append("q2\n", q2).unwrap();
    buffer.insert("first", q3).unwrap();
    for (text, source) in [("first", q3), ("q1", q1), ("q2", q2)] {
        let (actual, result) = buffer.next_line(&mut line).unwrap();
        result.unwrap();
        assert_eq!((line.as_str(), actual), (text, source));
    }
    buffer.append(&"x".repeat(8193), q1).unwrap();
    buffer.append("kept\n", q2).unwrap();
    assert_eq!(
        buffer.next_line(&mut line).unwrap(),
        (q1, Err(TextError::TooLong))
    );
    buffer.next_line(&mut line).unwrap().1.unwrap();
    assert_eq!(line.as_str(), "kept");
    assert!(buffer.is_empty());
}
