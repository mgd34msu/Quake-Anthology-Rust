use qa_console::{
    command_buffer::CommandBuffer,
    command_text::{self, TextError},
    commands::{CommandError, Console, Host},
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
    fn read_script(&mut self, path: &str) -> Result<String, String> {
        match path {
            "inner.cfg" => Ok("mark inside\n".into()),
            "outer.cfg" => Ok("mark before; exec inner; mark after\n".into()),
            _ => Err("missing".into()),
        }
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
    host.marks.push(args.tail(1));
    Ok(())
}

#[test]
fn source_tokenizers_keep_original_quotes_comments_punctuation_and_limits() {
    let text = "echo a\"b c\" x:y {q} //end\nnext";
    assert_eq!(
        command_text::tokenize(text, Source::Quake).unwrap().values,
        ["echo", "a\"b", "c\"", "x", ":", "y", "{", "q", "}", "next"]
    );
    assert_eq!(
        command_text::tokenize(text, Source::Quake2).unwrap().values,
        ["echo", "a\"b", "c\"", "x:y", "{q}", "next"]
    );
    assert_eq!(
        command_text::tokenize(text, Source::Quake3).unwrap().values,
        ["echo", "a", "b c", "x:y", "{q}"]
    );
    assert_eq!(
        command_text::tokenize("echo /* skip */one\"two\" //rest", Source::Quake3)
            .unwrap()
            .values,
        ["echo", "one", "two"]
    );
    for source in Source::ALL {
        assert_eq!(
            command_text::tokenize("echo \"unfinished", source)
                .unwrap()
                .values,
            ["echo", "unfinished"]
        );
        let many = "x ".repeat(1100);
        assert_eq!(
            command_text::tokenize(&many, source).unwrap().values.len(),
            if source == Source::Quake3 { 1024 } else { 80 }
        );
    }
}

#[test]
fn q2_macros_repeat_outside_quotes_and_reject_only_the_bad_command() {
    let values = |name: &str| {
        Some(
            match name {
                "a" => "$b",
                "b" => "3",
                "loop" => "$loop",
                _ => "",
            }
            .to_owned(),
        )
    };
    assert_eq!(
        command_text::expand("echo $a \"$a\"", values).unwrap(),
        "echo 3 \"$a\""
    );
    assert_eq!(
        command_text::expand("echo $loop", values),
        Err(TextError::MacroLoop)
    );
    assert_eq!(
        command_text::expand("echo \"unfinished", values),
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
        while let Some(line) = buffer.next_line() {
            assert_eq!(line.context, context);
            if !line.text.trim().is_empty() {
                lines.push(line.text);
            }
        }
        assert_eq!(lines, ["echo first", "echo hello", " echo \"a;b\""]);
        buffer.append("kept\n", context).unwrap();
        assert_eq!(
            buffer.append(&"x".repeat(65536), context),
            Err(TextError::TooLong)
        );
        assert_eq!(buffer.next_line().unwrap().text, "kept");
    }
    let q1 = context(Source::Quake);
    let mut buffer = CommandBuffer::new();
    buffer.append("fix\n", q1).unwrap();
    buffer.insert("pre", q1).unwrap();
    assert_eq!(buffer.next_line().unwrap().text, "prefix");
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
