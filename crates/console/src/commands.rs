//! One host-owned command table and fixed command/alias storage for every source.
use crate::{
    command_buffer::CommandBuffer,
    command_text::{self, Arguments, ParsedCommand, TextError},
    cvars::{Cvars, WriteError},
    text::MAX_TEXT,
    views::{CommandProducer, Context, RuleSetId},
};
use qa_core::sys_events::{EventTime, KeySource, SeatId, key_catchers};
use qa_core::text::FixedText;
use qa_core::{
    names::{NamesError, compare_folded},
    primitives::NameId,
};
use qa_input::{BindError, Input, Target, bindings, keys};
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
    fn print_bytes(&mut self, text: &[u8]) {
        match std::str::from_utf8(text) {
            Ok(text) => self.print(format_args!("{text}")),
            Err(_) => self.print(format_args!("Console byte output is unavailable\n")),
        }
    }
    fn reject_input(&mut self, _count: u64) {}
    fn read_script(&mut self, path: &str, destination: &mut [u8]) -> Result<usize, ScriptError>;
    fn quit(&mut self);
    fn input(&mut self) -> &mut Input;
    fn input_time(&self) -> EventTime;
    fn command_producer_live(&self, _producer: CommandProducer) -> bool {
        false
    }
}
#[derive(Debug)]
pub enum CommandError {
    Text(TextError),
    Names(NamesError),
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
enum Command<H> {
    Function(CommandFn<H>),
    Button(qa_input::Action),
}
impl<H> Copy for Command<H> {}
impl<H> Clone for Command<H> {
    fn clone(&self) -> Self {
        *self
    }
}
struct Alias {
    name: Option<NameId>,
    producer: Option<CommandProducer>,
    next_same_name: Option<usize>,
    text: FixedText<1024>,
}
pub struct Console<H> {
    pub cvars: Cvars,
    pub editor: crate::editor::Editor,
    pub scrollback: crate::presentation::Scrollback,
    buffer: CommandBuffer,
    commands: Vec<NameId>,
    command_by_name: Box<[Option<Command<H>>]>,
    aliases: Box<[Alias]>,
    alias_by_name: Box<[Option<usize>]>,
    alias_count: usize,
    parser: Option<Box<ParsedCommand>>,
    joined: FixedText<MAX_TEXT>,
    script: Box<[u8]>,
}
impl<H: Host> Console<H> {
    pub fn new(context: Context) -> Result<Self, NamesError> {
        Self::with_alias_capacity(context, 4096)
    }
    /// Select the session's alias storage while loading; commands never grow it.
    pub fn with_alias_capacity(
        context: Context,
        alias_capacity: usize,
    ) -> Result<Self, NamesError> {
        let cvars = Cvars::with_context(context)?;
        let command_by_name = vec![None; cvars.names.capacity()].into_boxed_slice();
        let alias_by_name = vec![None; cvars.names.capacity()].into_boxed_slice();
        let mut console = Self {
            cvars,
            editor: crate::editor::Editor::default(),
            scrollback: crate::presentation::Scrollback::default(),
            command_by_name,
            alias_by_name,
            buffer: CommandBuffer::new(),
            commands: Vec::with_capacity(64),
            aliases: (0..alias_capacity)
                .map(|_| Alias {
                    name: None,
                    producer: None,
                    next_same_name: None,
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
            ("seta", Self::set_flags::<1>),
            ("setu", Self::set_flags::<2>),
            ("sets", Self::set_flags::<4>),
            ("cmdlist", Self::cmdlist),
            ("cvarlist", Self::cvarlist),
            ("quit", Self::quit),
            ("bind", Self::bind),
            ("unbind", Self::unbind),
            ("unbindall", Self::unbind_all),
            ("bindlist", Self::bind_list),
            ("centerview", Self::center_view),
            ("toggleconsole", Self::toggle_console),
        ] {
            console.register(name, function);
        }
        for entry in bindings::ACTION_NAMES {
            for name in [entry.press, entry.release] {
                console.register_entry(name, Command::Button(entry.action));
            }
        }
        Ok(console)
    }
    pub fn register(&mut self, name: &'static str, function: CommandFn<H>) -> bool {
        self.register_entry(name, Command::Function(function))
    }
    fn register_entry(&mut self, name: &'static str, command: Command<H>) -> bool {
        let Ok(exact) = self.cvars.names.intern(name.as_bytes()) else {
            return false;
        };
        let Some(id) = self.cvars.names.folded(exact) else {
            return false;
        };
        if self.command_by_name[id.0 as usize].is_some() {
            return false;
        }
        let names = &self.cvars.names;
        let at = self.commands.partition_point(|&id| {
            compare_folded(names.get(id).unwrap_or_default(), name.as_bytes()) == Ordering::Less
        });
        self.commands.insert(at, exact);
        self.command_by_name[id.0 as usize] = Some(command);
        true
    }

    pub fn append(&mut self, text: &str, context: Context) -> Result<(), TextError> {
        self.buffer.append(text, context)
    }
    /// Borrow the existing table and buffer for a module call. No module owns
    /// another console or queues commands outside this buffer.
    pub fn module_parts(&mut self) -> (&mut Cvars, &mut CommandBuffer, &mut crate::editor::Editor) {
        (&mut self.cvars, &mut self.buffer, &mut self.editor)
    }
    pub fn append_line(&mut self, text: &str, context: Context) -> Result<(), TextError> {
        self.append_line_bytes(text.as_bytes(), context)
    }
    pub fn append_line_bytes(&mut self, text: &[u8], context: Context) -> Result<(), TextError> {
        self.buffer.append_line_bytes(text, context)
    }
    pub fn discard_acquisition(&mut self, source: KeySource, producer: Option<CommandProducer>) {
        self.buffer.discard_acquisition(source, producer);
    }
    pub fn discard_producer(
        &mut self,
        host: &mut H,
        producer: CommandProducer,
    ) -> Result<(), CommandError> {
        self.retain_producers(host, |current| current != producer)
    }
    /// Cold publication or teardown keeps only the admitted module lifetimes.
    /// Human text and aliases have no producer and retain their order.
    pub fn retain_producers(
        &mut self,
        host: &mut H,
        mut retained: impl FnMut(CommandProducer) -> bool,
    ) -> Result<(), CommandError> {
        let time = host.input_time();
        let mut output = BindOutput::new(&mut self.buffer, self.cvars.context());
        host.input()
            .retain_producers(&mut retained, time, &mut output);
        let result = output.finish(host);
        self.buffer.retain_producers(&mut retained);
        for index in 0..self.alias_count {
            if self.aliases[index]
                .producer
                .is_some_and(|producer| !retained(producer))
            {
                self.remove_alias(index);
            }
        }
        result
    }
    fn alias_index(&self, name: NameId, producer: Option<CommandProducer>) -> Option<usize> {
        let mut current = self.alias_by_name[name.0 as usize];
        while let Some(index) = current {
            let alias = &self.aliases[index];
            if alias.producer == producer {
                return Some(index);
            }
            current = alias.next_same_name;
        }
        None
    }
    fn remove_alias(&mut self, index: usize) {
        let Some(name) = self.aliases[index]
            .name
            .and_then(|name| self.cvars.names.folded(name))
        else {
            return;
        };
        let head = &mut self.alias_by_name[name.0 as usize];
        if *head == Some(index) {
            *head = self.aliases[index].next_same_name;
        } else {
            let mut current = *head;
            while let Some(previous) = current {
                if self.aliases[previous].next_same_name == Some(index) {
                    self.aliases[previous].next_same_name = self.aliases[index].next_same_name;
                    break;
                }
                current = self.aliases[previous].next_same_name;
            }
        }
        let alias = &mut self.aliases[index];
        alias.name = None;
        alias.producer = None;
        alias.next_same_name = None;
        alias.text.clear();
    }
    fn reject_expansion(&mut self, error: TextError, context: Context, host: &mut H) {
        if error == TextError::TooLong
            && let Some(source) = context.key_source
        {
            self.discard_acquisition(source, context.producer);
            let time = context.event_time.unwrap_or_else(|| host.input_time());
            host.input().cancel_source(source, context.producer, time);
            host.reject_input(1);
        }
    }
    /// The window editor submits through the same buffer as stdin, binds and
    /// module commands. Failed admission keeps the editable line intact.
    pub fn edit_key(
        &mut self,
        seat: SeatId,
        control: u16,
        down: bool,
        repeat: bool,
        time: EventTime,
        mut print: impl FnMut(Output<'_>),
    ) -> Result<bool, TextError> {
        match self.editor.key(seat, control, down, repeat) {
            crate::editor::EditAction::Ignored => Ok(false),
            crate::editor::EditAction::Caught => Ok(true),
            crate::editor::EditAction::Submit => {
                let context = Context {
                    seat,
                    event_time: Some(time),
                    key_source: None,
                    producer: None,
                    ..self.cvars.context()
                };
                self.buffer.append_line(self.editor.line(), context)?;
                print(format_args!("]{}\n", self.editor.line()));
                self.editor.submitted();
                Ok(true)
            }
        }
    }

    pub fn sync_editor_input(&self, input: &mut Input, time: EventTime) {
        let owner = self.editor.seat();
        for seat in SeatId::ALL {
            input.set_catcher(seat, key_catchers::CONSOLE, owner == Some(seat), time);
        }
        if let Some(seat) = owner {
            input.set_text_seat(seat);
        }
    }

    fn toggle_console(
        console: &mut Self,
        host: &mut H,
        _: &Arguments<'_>,
        context: Context,
    ) -> Result<(), CommandError> {
        console.editor.toggle(context.seat);
        let time = context.event_time.unwrap_or(host.input_time());
        console.sync_editor_input(host.input(), time);
        Ok(())
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
        while !self.buffer.is_empty() {
            if let Some(producer) = self
                .buffer
                .wait_context()
                .and_then(|context| context.producer)
                && !host.command_producer_live(producer)
            {
                let _ = self.discard_producer(host, producer);
                continue;
            }
            if !self.buffer.ready() {
                break;
            }
            if executed >= 4096 {
                host.print(format_args!("Command frame limit\n"));
                break;
            }
            let Some((context, result)) = self.buffer.next_line(&mut parser.line) else {
                break;
            };
            executed += 1;
            if let Some(producer) = context.producer
                && !host.command_producer_live(producer)
            {
                let _ = self.discard_producer(host, producer);
                continue;
            }
            if let Err(error) = result {
                self.reject_expansion(error, context, host);
                host.print(format_args!("Command rejected: {error:?}\n"));
                continue;
            }
            if matches!(
                context.source,
                RuleSetId::Quake2 | RuleSetId::Quake2Rerelease
            ) {
                let raw = parser.line.as_bytes();
                let expanded = if raw.contains(&b'$') {
                    command_text::expand_engine_bytes(
                        &mut parser.line,
                        &mut parser.tokens,
                        |name| self.cvars.macro_value(name, context),
                    )
                } else if raw.len() >= crate::text::MAX_TEXT {
                    Err(TextError::TooLong)
                } else if raw.iter().filter(|&&b| b == b'"').count() & 1 != 0 {
                    Err(TextError::UnmatchedQuote)
                } else {
                    Ok(())
                };
                if let Err(error) = expanded {
                    self.reject_expansion(error, context, host);
                    host.print(format_args!("Command rejected: {error:?}\n"));
                    continue;
                }
            }
            let args = match command_text::tokenize_engine_bytes(
                parser.line.as_bytes(),
                context.source,
                &mut parser.tokens,
            ) {
                Ok(args) => args,
                Err(error) => {
                    self.reject_expansion(error, context, host);
                    host.print(format_args!("Command rejected: {error:?}\n"));
                    continue;
                }
            };
            let Ok(name) = std::str::from_utf8(args.get_bytes(0)) else {
                host.print(format_args!("Command name is not UTF-8\n"));
                continue;
            };
            if name.is_empty() {
                continue;
            }
            let id = self.cvars.lookup_name(name);
            let result =
                if let Some(command) = id.and_then(|id| self.command_by_name[id.0 as usize]) {
                    match command {
                        Command::Function(function) => function(self, host, &args, context),
                        Command::Button(action) => self.button(host, &args, context, action),
                    }
                } else if let Some(view) = id.and_then(|id| self.cvars.bind_id(id, context)) {
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
                        match std::str::from_utf8(args.get_bytes(1)) {
                            Ok(value) => self.cvars.write(view, value).map_err(Into::into),
                            Err(_) => Err(CommandError::Usage),
                        }
                    }
                } else if let Some(alias) = id
                    .and_then(|id| self.alias_index(id, context.producer))
                    .map(|index| &self.aliases[index])
                {
                    aliases += 1;
                    if aliases > 16 {
                        host.print(format_args!("Alias expansion limit\n"));
                        continue;
                    }
                    let result = self.buffer.insert_bytes(alias.text.as_bytes(), context);
                    if let Err(error) = result {
                        self.reject_expansion(error, context, host);
                    }
                    result.map_err(Into::into)
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
        let mut argument = FixedText::<{ crate::text::MAX_TEXT + 1 }>::default();
        for text in args.iter_bytes().skip(1) {
            argument.clear();
            argument
                .append_bytes(text)
                .map_err(|_| TextError::TooLong)?;
            argument
                .append_bytes(b" ")
                .map_err(|_| TextError::TooLong)?;
            host.print_bytes(argument.as_bytes());
        }
        host.print_bytes(b"\n");
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
            if let Some(binding) = host.input().binding(control) {
                let mut value = FixedText::<1088>::default();
                let _ = write!(value, "\"{}\" = \"", args.get(1));
                let _ = value.append_bytes(binding.bytes());
                let _ = value.write_str("\"\n");
                host.print_bytes(value.as_bytes());
            } else {
                host.print(format_args!("\"{}\" is not bound\n", args.get(1)));
            }
            return Ok(());
        }
        args.join_bytes(2, &mut self.joined)?;
        let time = context.event_time.unwrap_or_else(|| host.input_time());
        let mut output = BindOutput::new(&mut self.buffer, context);
        if self.joined.as_bytes().is_empty() {
            host.input().bind(control, None, time, &mut output);
        } else {
            host.input()
                .bind_bytes_owned(
                    control,
                    self.joined.as_bytes(),
                    context.producer,
                    time,
                    &mut output,
                )
                .map_err(CommandError::Bind)?;
        }
        output.finish(host)
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
        let mut output = BindOutput::new(&mut self.buffer, context);
        host.input().bind(control, None, time, &mut output);
        output.finish(host)
    }
    fn unbind_all(
        &mut self,
        host: &mut H,
        _: &Arguments<'_>,
        context: Context,
    ) -> Result<(), CommandError> {
        let time = context.event_time.unwrap_or_else(|| host.input_time());
        let mut output = BindOutput::new(&mut self.buffer, context);
        host.input().unbind_all(time, &mut output);
        output.finish(host)
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
                let _ = line.append_bytes(binding.bytes());
                let _ = line.write_str("\"\n");
                host.print_bytes(line.as_bytes());
            }
        }
        Ok(())
    }
    fn button(
        &mut self,
        host: &mut H,
        args: &Arguments<'_>,
        context: Context,
        action: qa_input::Action,
    ) -> Result<(), CommandError> {
        let name = args.get(0);
        let key = context.key_source.or_else(|| {
            (!args.get(1).is_empty()).then(|| qa_core::sys_events::KeySource {
                device: qa_core::sys_events::DeviceId::Keyboard(0),
                control: crate::numbers::integer(args.get(1)) as u16,
            })
        });
        let time = if context.key_source.is_some() || args.get(2).is_empty() {
            context.event_time.unwrap_or_else(|| host.input_time())
        } else {
            EventTime::from_milliseconds(crate::numbers::integer(args.get(2)) as u32 as u64)
        };
        host.input().button(
            context.seat,
            action,
            name.starts_with('+'),
            key,
            context.producer,
            time,
        );
        Ok(())
    }
    fn center_view(
        &mut self,
        host: &mut H,
        _: &Arguments<'_>,
        context: Context,
    ) -> Result<(), CommandError> {
        host.input().center_view(context.seat);
        Ok(())
    }
    fn wait(
        &mut self,
        _: &mut H,
        args: &Arguments<'_>,
        context: Context,
    ) -> Result<(), CommandError> {
        self.buffer.wait(
            if args.len() == 1 {
                1
            } else {
                crate::numbers::integer(args.get(1)).max(0) as u32
            },
            context,
        );
        Ok(())
    }
    fn alias(
        &mut self,
        host: &mut H,
        args: &Arguments<'_>,
        context: Context,
    ) -> Result<(), CommandError> {
        if args.len() == 1 {
            for alias in self.aliases[..self.alias_count]
                .iter()
                .filter(|a| a.name.is_some() && a.producer == context.producer)
            {
                host.print(format_args!(
                    "{} : ",
                    alias
                        .name
                        .and_then(|id| self.cvars.names.get(id))
                        .and_then(|bytes| std::str::from_utf8(bytes).ok())
                        .unwrap_or_default()
                ));
                host.print_bytes(alias.text.as_bytes());
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
        args.join_bytes(2, &mut text)?;
        text.write_str("\n").map_err(|_| TextError::TooLong)?;
        let folded = self.cvars.lookup_name(name);
        let previous = folded.and_then(|id| self.alias_index(id, context.producer));
        let index = previous
            .or_else(|| {
                self.aliases[..self.alias_count]
                    .iter()
                    .position(|a| a.name.is_none())
            })
            .unwrap_or(self.alias_count);
        if index == self.aliases.len() {
            return Err(TextError::TooLong.into());
        }
        let exact = self
            .cvars
            .names
            .intern(name.as_bytes())
            .map_err(CommandError::Names)?;
        let folded = self
            .cvars
            .names
            .folded(exact)
            .ok_or(NamesError::Capacity)
            .map_err(CommandError::Names)?;
        self.alias_count = self.alias_count.max(index + 1);
        let next_same_name = if previous.is_some() {
            self.aliases[index].next_same_name
        } else {
            self.alias_by_name[folded.0 as usize]
        };
        if previous.is_none() {
            self.alias_by_name[folded.0 as usize] = Some(index);
        }
        let alias = &mut self.aliases[index];
        alias.name = Some(exact);
        alias.producer = context.producer;
        alias.next_same_name = next_same_name;
        alias.text = text;
        Ok(())
    }
    fn unalias(
        &mut self,
        _: &mut H,
        args: &Arguments<'_>,
        context: Context,
    ) -> Result<(), CommandError> {
        if args.len() != 2 {
            return Err(CommandError::Usage);
        }
        let folded = self.cvars.lookup_name(args.get(1));
        if let Some(index) = folded.and_then(|id| self.alias_index(id, context.producer)) {
            self.remove_alias(index);
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
        if let Err(error) = self.buffer.insert(text, context) {
            self.reject_expansion(error, context, host);
            return Err(error.into());
        }
        Ok(())
    }
    fn vstr(
        &mut self,
        host: &mut H,
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
        if let Err(error) = self.buffer.insert(self.joined.as_str(), context) {
            self.reject_expansion(error, context, host);
            return Err(error.into());
        }
        Ok(())
    }
    fn set(
        &mut self,
        _: &mut H,
        args: &Arguments<'_>,
        context: Context,
    ) -> Result<(), CommandError> {
        self.set_value(args, context, 0)
    }
    fn set_flags<const FLAGS: u32>(
        &mut self,
        _: &mut H,
        args: &Arguments<'_>,
        context: Context,
    ) -> Result<(), CommandError> {
        if args.len() != 3 {
            return Err(CommandError::Usage);
        }
        self.set_value(args, context, FLAGS)
    }
    fn set_value(
        &mut self,
        args: &Arguments<'_>,
        context: Context,
        added_flags: u32,
    ) -> Result<(), CommandError> {
        if args.len() < 3 {
            return Err(CommandError::Usage);
        }
        let q2 = matches!(
            context.source,
            RuleSetId::Quake2 | RuleSetId::Quake2Rerelease
        );
        let full_flags = if q2 {
            if args.len() > 4 || (args.len() == 4 && !matches!(args.get(3), "u" | "s")) {
                return Err(CommandError::Usage);
            }
            (args.len() == 4).then(|| if args.get(3) == "u" { 2 } else { 4 })
        } else {
            args.join(2, &mut self.joined)?;
            None
        };
        let value = if q2 {
            std::str::from_utf8(args.get_bytes(2)).map_err(|_| CommandError::Usage)?
        } else {
            std::str::from_utf8(self.joined.as_bytes()).map_err(|_| CommandError::Usage)?
        };
        let name = std::str::from_utf8(args.get_bytes(1)).map_err(|_| CommandError::Usage)?;
        self.cvars
            .set_user(name, value, context, full_flags, added_flags)?;
        Ok(())
    }
    fn cmdlist(&mut self, host: &mut H, _: &Arguments<'_>, _: Context) -> Result<(), CommandError> {
        for c in &self.commands {
            host.print(format_args!(
                "{}\n",
                self.cvars
                    .names
                    .get(*c)
                    .and_then(|bytes| std::str::from_utf8(bytes).ok())
                    .unwrap_or_default()
            ));
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
            if definition.is_none_or(|d| d.stored) {
                host.print(format_args!(
                    "{indent}{name} = \"{}\"\n",
                    if definition.is_some_and(|d| d.policies & 2 != 0) {
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

pub struct BindOutput<'a> {
    buffer: &'a mut CommandBuffer,
    context: Context,
    rejected: Option<TextError>,
    rejected_count: u64,
}
impl Target for BindOutput<'_> {
    fn character(&mut self, _: SeatId, _: char) {}
    fn command(
        &mut self,
        seat: SeatId,
        time: EventTime,
        key_source: Option<qa_core::sys_events::KeySource>,
        producer: Option<CommandProducer>,
        text: &str,
    ) -> bool {
        self.command_bytes(seat, time, key_source, producer, text.as_bytes())
    }
    fn command_bytes(
        &mut self,
        seat: SeatId,
        time: EventTime,
        key_source: Option<qa_core::sys_events::KeySource>,
        producer: Option<CommandProducer>,
        text: &[u8],
    ) -> bool {
        let result = self.buffer.append_line_bytes(
            text,
            Context {
                seat,
                key_source,
                producer,
                event_time: Some(time),
                ..self.context
            },
        );
        if let Err(error) = result {
            self.rejected.get_or_insert(error);
            self.rejected_count += 1;
        }
        result.is_ok()
    }
    fn cancel_commands(&mut self, source: KeySource, producer: Option<CommandProducer>) {
        self.buffer.discard_acquisition(source, producer);
    }
}

impl<'a> BindOutput<'a> {
    pub fn new(buffer: &'a mut CommandBuffer, context: Context) -> Self {
        Self {
            buffer,
            context,
            rejected: None,
            rejected_count: 0,
        }
    }
    pub fn outcome(self) -> (u64, Result<(), TextError>) {
        (self.rejected_count, self.rejected.map_or(Ok(()), Err))
    }
    fn finish(self, host: &mut impl Host) -> Result<(), CommandError> {
        let (count, result) = self.outcome();
        host.reject_input(count);
        result.map_err(CommandError::Text)
    }
}
