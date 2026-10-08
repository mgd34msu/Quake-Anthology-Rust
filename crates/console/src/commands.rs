//! One host-owned command table and fixed command/alias storage for every source.
use crate::{
    command_buffer::CommandBuffer,
    command_text::{self, Arguments, TextError, Tokens},
    cvars::{Cvars, WriteError},
    text::MAX_TEXT,
    views::{Context, Source},
};
use qa_core::sys_events::{EventTime, SeatId};
use qa_core::text::FixedText;
use qa_input::{BindError, Binding, Input, Target, bindings, keys};
use std::{
    cmp::Ordering,
    fmt::{Arguments as Output, Write},
};

#[derive(Clone, Copy, Debug)]
pub enum ScriptError {
    Missing,
    TooLong,
    Read,
    Encoding,
}
pub trait Host {
    fn print(&mut self, text: Output<'_>);
    fn read_script(&mut self, path: &str, destination: &mut [u8]) -> Result<usize, ScriptError>;
    fn quit(&mut self);
    fn input(&mut self) -> &mut Input;
    fn input_time(&self) -> EventTime;
}
#[derive(Debug)]
pub enum CommandError {
    Text(TextError),
    Cvar(WriteError),
    Usage,
    UnknownCvar,
    AliasName,
    Script(ScriptError),
    Bind(BindError),
}
impl From<TextError> for CommandError {
    fn from(e: TextError) -> Self {
        Self::Text(e)
    }
}
impl From<WriteError> for CommandError {
    fn from(e: WriteError) -> Self {
        Self::Cvar(e)
    }
}
pub type CommandFn<H> =
    fn(&mut Console<H>, &mut H, &Arguments<'_>, Context) -> Result<(), CommandError>;
struct Command<H> {
    name: &'static str,
    function: CommandFn<H>,
}
struct Alias {
    name: FixedText<32>,
    text: FixedText<1024>,
}
#[derive(Default)]
struct Parser {
    line: FixedText<MAX_TEXT>,
    tokens: Tokens,
}
pub struct Console<H> {
    pub cvars: Cvars,
    buffer: CommandBuffer,
    commands: Vec<Command<H>>,
    aliases: Box<[Alias]>,
    alias_count: usize,
    parser: Option<Box<Parser>>,
    joined: FixedText<MAX_TEXT>,
    script: Box<[u8]>,
}
fn compare(a: &str, b: &str) -> Ordering {
    a.bytes()
        .map(|b| b.to_ascii_lowercase())
        .cmp(b.bytes().map(|b| b.to_ascii_lowercase()))
}
impl<H: Host> Console<H> {
    pub fn new(context: Context) -> Self {
        Self::with_alias_capacity(context, 4096)
    }
    /// Select the session's alias storage while loading; commands never grow it.
    pub fn with_alias_capacity(context: Context, alias_capacity: usize) -> Self {
        let mut console = Self {
            cvars: Cvars::with_context(context),
            buffer: CommandBuffer::new(),
            commands: Vec::with_capacity(64),
            aliases: (0..alias_capacity)
                .map(|_| Alias {
                    name: FixedText::default(),
                    text: FixedText::default(),
                })
                .collect(),
            alias_count: 0,
            parser: Some(Box::default()),
            joined: FixedText::default(),
            script: vec![0; 65535].into_boxed_slice(),
        };
        for (name, function) in [
            ("echo", Self::echo as CommandFn<H>),
            ("wait", Self::wait),
            ("alias", Self::alias),
            ("unalias", Self::unalias),
            ("exec", Self::exec),
            ("vstr", Self::vstr),
            ("set", Self::set),
            ("cmdlist", Self::cmdlist),
            ("cvarlist", Self::cvarlist),
            ("quit", Self::quit),
            ("bind", Self::bind),
            ("unbind", Self::unbind),
            ("unbindall", Self::unbind_all),
            ("bindlist", Self::bind_list),
        ] {
            console.register(name, function);
        }
        for entry in bindings::ACTION_NAMES {
            console.register(entry.press, Self::button);
            console.register(entry.release, Self::button);
        }
        console
    }
    pub fn register(&mut self, name: &'static str, function: CommandFn<H>) -> bool {
        match self.commands.binary_search_by(|c| compare(c.name, name)) {
            Ok(_) => false,
            Err(at) => {
                self.commands.insert(at, Command { name, function });
                true
            }
        }
    }
    pub fn append(&mut self, text: &str, context: Context) -> Result<(), TextError> {
        self.buffer.append(text, context)
    }
    pub fn append_line(&mut self, text: &str, context: Context) -> Result<(), TextError> {
        self.buffer.append_line(text, context)
    }
    pub fn idle(&self) -> bool {
        self.buffer.is_empty()
    }
    pub fn execute_frame(&mut self, host: &mut H) {
        // Move only the scratch pointer. Commands may mutate/insert into the
        // buffer while argv continues borrowing this console's loaded parser.
        let Some(mut parser) = self.parser.take() else {
            host.print(format_args!("Console execution already active\n"));
            return;
        };
        let mut executed = 0;
        let mut aliases = 0;
        while !self.buffer.is_empty() && self.buffer.ready() {
            if executed >= 4096 {
                host.print(format_args!("Command frame limit\n"));
                break;
            }
            let Some((context, result)) = self.buffer.next_line(&mut parser.line) else {
                break;
            };
            executed += 1;
            if let Err(error) = result {
                host.print(format_args!("Command rejected: {error:?}\n"));
                continue;
            }
            if matches!(context.source, Source::Quake2 | Source::Quake2Rerelease) {
                let raw = parser.line.as_str();
                let expanded = if raw.contains('$') {
                    command_text::expand(&mut parser.line, &mut parser.tokens, |name| {
                        self.cvars
                            .bind(name, context)
                            .and_then(|view| self.cvars.read(view).ok())
                    })
                } else if raw.len() >= 1024 {
                    Err(TextError::TooLong)
                } else if raw.bytes().filter(|&b| b == b'"').count() & 1 != 0 {
                    Err(TextError::UnmatchedQuote)
                } else {
                    Ok(())
                };
                if let Err(error) = expanded {
                    host.print(format_args!("Command rejected: {error:?}\n"));
                    continue;
                }
            }
            let args = match command_text::tokenize(
                parser.line.as_str(),
                context.source,
                &mut parser.tokens,
            ) {
                Ok(args) => args,
                Err(error) => {
                    host.print(format_args!("Command rejected: {error:?}\n"));
                    continue;
                }
            };
            let name = args.get(0);
            if name.is_empty() {
                continue;
            }
            let result = if let Ok(at) = self.commands.binary_search_by(|c| compare(c.name, name)) {
                (self.commands[at].function)(self, host, &args, context)
            } else if let Some(view) = self.cvars.bind(name, context) {
                if args.len() == 1 {
                    match self.cvars.read(view) {
                        Ok(value) => {
                            if self.cvars.private(view) {
                                host.print(format_args!("{name} is private\n"));
                            } else {
                                host.print(format_args!("{name} = \"{}\"\n", value.as_str()));
                            }
                            Ok(())
                        }
                        Err(error) => Err(CommandError::Cvar(WriteError::Conversion(error))),
                    }
                } else {
                    self.cvars.write(view, args.get(1)).map_err(Into::into)
                }
            } else if let Some(alias) = self.aliases[..self.alias_count]
                .iter()
                .find(|a| !a.name.as_str().is_empty() && a.name.as_str().eq_ignore_ascii_case(name))
            {
                aliases += 1;
                if aliases > 16 {
                    host.print(format_args!("Alias expansion limit\n"));
                    continue;
                }
                self.buffer
                    .insert(alias.text.as_str(), context)
                    .map_err(Into::into)
            } else {
                host.print(format_args!("Unknown command \"{name}\"\n"));
                Ok(())
            };
            if let Err(error) = result {
                host.print(format_args!("Command {name} rejected: {error:?}\n"));
            }
        }
        self.parser = Some(parser);
    }
    fn echo(&mut self, host: &mut H, args: &Arguments<'_>, _: Context) -> Result<(), CommandError> {
        for text in args.iter().skip(1) {
            host.print(format_args!("{text} "));
        }
        host.print(format_args!("\n"));
        Ok(())
    }
    fn bind(
        &mut self,
        host: &mut H,
        args: &Arguments<'_>,
        context: Context,
    ) -> Result<(), CommandError> {
        if args.len() < 2 {
            return Err(CommandError::Usage);
        }
        let control = keys::parse(args.get(1)).ok_or(CommandError::Bind(BindError::Control))?;
        if args.len() == 2 {
            let mut value = FixedText::<1025>::default();
            if let Some(binding) = host.input().binding(control) {
                write_binding(binding, &mut value);
                host.print(format_args!(
                    "\"{}\" = \"{}\"\n",
                    args.get(1),
                    value.as_str()
                ));
            } else {
                host.print(format_args!("\"{}\" is not bound\n", args.get(1)));
            }
            return Ok(());
        }
        args.join(2, &mut self.joined)?;
        let time = context.event_time.unwrap_or_else(|| host.input_time());
        host.input()
            .bind_text(
                control,
                self.joined.as_str(),
                time,
                &mut BindOutput {
                    buffer: &mut self.buffer,
                    context,
                },
            )
            .map_err(CommandError::Bind)
    }
    fn unbind(
        &mut self,
        host: &mut H,
        args: &Arguments<'_>,
        context: Context,
    ) -> Result<(), CommandError> {
        if args.len() != 2 {
            return Err(CommandError::Usage);
        }
        let control = keys::parse(args.get(1)).ok_or(CommandError::Bind(BindError::Control))?;
        let time = context.event_time.unwrap_or_else(|| host.input_time());
        host.input().bind(
            control,
            None,
            time,
            &mut BindOutput {
                buffer: &mut self.buffer,
                context,
            },
        );
        Ok(())
    }
    fn unbind_all(
        &mut self,
        host: &mut H,
        _: &Arguments<'_>,
        context: Context,
    ) -> Result<(), CommandError> {
        let time = context.event_time.unwrap_or_else(|| host.input_time());
        host.input().unbind_all(
            time,
            &mut BindOutput {
                buffer: &mut self.buffer,
                context,
            },
        );
        Ok(())
    }
    fn bind_list(
        &mut self,
        host: &mut H,
        _: &Arguments<'_>,
        _: Context,
    ) -> Result<(), CommandError> {
        // Copy one fixed line at a time to end the input borrow before print.
        for control in 0..1024 {
            let mut line = FixedText::<1088>::default();
            if let Some(binding) = host.input().binding(control) {
                if keys::normalize(control) != control {
                    continue;
                }
                let _ = keys::write_name(control, &mut line);
                let _ = line.write_str(" \"");
                write_binding(binding, &mut line);
                let _ = line.write_str("\"\n");
                host.print(format_args!("{}", line.as_str()));
            }
        }
        Ok(())
    }
    fn button(
        &mut self,
        host: &mut H,
        args: &Arguments<'_>,
        context: Context,
    ) -> Result<(), CommandError> {
        let name = args.get(0);
        let Some(action) = bindings::action(&name[1..]) else {
            return Err(CommandError::Usage);
        };
        let key = (!args.get(1).is_empty()).then(|| crate::numbers::integer(args.get(1)) as u16);
        let time = if args.get(2).is_empty() {
            context.event_time.unwrap_or_else(|| host.input_time())
        } else {
            EventTime(
                args.get(2)
                    .parse::<u64>()
                    .unwrap_or(0)
                    .saturating_mul(1_000_000),
            )
        };
        host.input()
            .button(context.seat, action, name.starts_with('+'), key, time);
        Ok(())
    }
    fn wait(&mut self, _: &mut H, args: &Arguments<'_>, _: Context) -> Result<(), CommandError> {
        self.buffer.wait(if args.len() == 1 {
            1
        } else {
            crate::numbers::integer(args.get(1)).max(0) as u32
        });
        Ok(())
    }
    fn alias(
        &mut self,
        host: &mut H,
        args: &Arguments<'_>,
        _: Context,
    ) -> Result<(), CommandError> {
        if args.len() == 1 {
            for alias in self.aliases[..self.alias_count]
                .iter()
                .filter(|a| !a.name.as_str().is_empty())
            {
                host.print(format_args!(
                    "{} : {}",
                    alias.name.as_str(),
                    alias.text.as_str()
                ));
            }
            return Ok(());
        }
        let name = args.get(1);
        if name.is_empty()
            || name.len() >= 32
            || name
                .bytes()
                .any(|b| b <= 32 || matches!(b, b'"' | b';' | b'\\' | b'/'))
        {
            return Err(CommandError::AliasName);
        }
        let mut text = FixedText::<1024>::default();
        args.join(2, &mut text)?;
        text.write_str("\n").map_err(|_| TextError::TooLong)?;
        let index = self.aliases[..self.alias_count]
            .iter()
            .position(|a| a.name.as_str().eq_ignore_ascii_case(name))
            .or_else(|| {
                self.aliases[..self.alias_count]
                    .iter()
                    .position(|a| a.name.as_str().is_empty())
            })
            .unwrap_or(self.alias_count);
        if index == self.aliases.len() {
            return Err(TextError::TooLong.into());
        }
        self.alias_count = self.alias_count.max(index + 1);
        let alias = &mut self.aliases[index];
        alias.name.set(name).map_err(|_| TextError::TooLong)?;
        alias.text = text;
        Ok(())
    }
    fn unalias(&mut self, _: &mut H, args: &Arguments<'_>, _: Context) -> Result<(), CommandError> {
        if args.len() != 2 {
            return Err(CommandError::Usage);
        }
        if let Some(alias) = self.aliases[..self.alias_count]
            .iter_mut()
            .find(|a| a.name.as_str().eq_ignore_ascii_case(args.get(1)))
        {
            alias.name.clear();
            alias.text.clear();
        }
        Ok(())
    }
    fn exec(
        &mut self,
        host: &mut H,
        args: &Arguments<'_>,
        context: Context,
    ) -> Result<(), CommandError> {
        if args.len() != 2 {
            return Err(CommandError::Usage);
        }
        let mut path = FixedText::<4096>::default();
        path.set(args.get(1)).map_err(|_| TextError::TooLong)?;
        if std::path::Path::new(path.as_str()).extension().is_none() {
            path.write_str(".cfg").map_err(|_| TextError::TooLong)?;
        }
        let length = host
            .read_script(path.as_str(), &mut self.script)
            .map_err(CommandError::Script)?;
        let text = std::str::from_utf8(&self.script[..length])
            .map_err(|_| CommandError::Script(ScriptError::Encoding))?;
        self.buffer.insert(text, context)?;
        Ok(())
    }
    fn vstr(
        &mut self,
        _: &mut H,
        args: &Arguments<'_>,
        context: Context,
    ) -> Result<(), CommandError> {
        if args.len() != 2 {
            return Err(CommandError::Usage);
        }
        let view = self
            .cvars
            .bind(args.get(1), context)
            .ok_or(CommandError::UnknownCvar)?;
        let text = self
            .cvars
            .read(view)
            .map_err(|e| CommandError::Cvar(WriteError::Conversion(e)))?;
        self.joined
            .set(text.as_str())
            .map_err(|_| TextError::TooLong)?;
        self.joined
            .write_str("\n")
            .map_err(|_| TextError::TooLong)?;
        self.buffer.insert(self.joined.as_str(), context)?;
        Ok(())
    }
    fn set(
        &mut self,
        _: &mut H,
        args: &Arguments<'_>,
        context: Context,
    ) -> Result<(), CommandError> {
        if args.len() < 3 {
            return Err(CommandError::Usage);
        }
        let view = self
            .cvars
            .bind(args.get(1), context)
            .ok_or(CommandError::UnknownCvar)?;
        if matches!(context.source, Source::Quake2 | Source::Quake2Rerelease) {
            if args.len() > 4 || (args.len() == 4 && !matches!(args.get(3), "u" | "s")) {
                return Err(CommandError::Usage);
            }
            if args.len() == 4 {
                self.cvars
                    .full_set(view, args.get(2), if args.get(3) == "u" { 2 } else { 4 })?;
            } else {
                self.cvars.write(view, args.get(2))?;
            }
        } else {
            args.join(2, &mut self.joined)?;
            self.cvars.write(view, self.joined.as_str())?;
        }
        Ok(())
    }
    fn cmdlist(&mut self, host: &mut H, _: &Arguments<'_>, _: Context) -> Result<(), CommandError> {
        for c in &self.commands {
            host.print(format_args!("{}\n", c.name));
        }
        Ok(())
    }
    fn cvarlist(
        &mut self,
        host: &mut H,
        _: &Arguments<'_>,
        _: Context,
    ) -> Result<(), CommandError> {
        for definition in crate::cvars_generated::DEFINITIONS {
            if definition.family_count > 1 {
                host.print(format_args!("{} [seats]\n", definition.name));
            }
        }
        for (_, name, value, definition, seat) in self.cvars.entries() {
            let indent = if seat == 0 { "" } else { "  " };
            if definition.stored {
                host.print(format_args!(
                    "{indent}{name} = \"{}\"\n",
                    if definition.policies & 2 != 0 {
                        "<private>"
                    } else {
                        value
                    }
                ));
            } else {
                host.print(format_args!("{indent}{name} (derived)\n"));
            }
        }
        Ok(())
    }
    fn quit(&mut self, host: &mut H, _: &Arguments<'_>, _: Context) -> Result<(), CommandError> {
        host.quit();
        self.buffer.clear();
        Ok(())
    }
}

fn write_binding(binding: &Binding, output: &mut impl Write) {
    let _ = output.write_str(binding.text());
}
struct BindOutput<'a> {
    buffer: &'a mut CommandBuffer,
    context: Context,
}
impl Target for BindOutput<'_> {
    fn character(&mut self, _: SeatId, _: char) {}
    fn command(&mut self, seat: SeatId, time: EventTime, text: &str) {
        let _ = self.buffer.append_line(
            text,
            Context {
                seat,
                event_time: Some(time),
                ..self.context
            },
        );
    }
}
