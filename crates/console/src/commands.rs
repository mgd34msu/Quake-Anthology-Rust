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
    names::{NameTable, NamesError, compare_folded},
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
    External,
}
impl<H> Copy for Command<H> {}
impl<H> Clone for Command<H> {
    fn clone(&self) -> Self {
        *self
    }
}
struct CommandSlot<H> {
    command: Option<Command<H>>,
    completion: Option<usize>,
    alias: Option<usize>,
}
impl<H> CommandSlot<H> {
    fn new() -> Self {
        Self {
            command: None,
            completion: None,
            alias: None,
        }
    }
}
struct OwnedName<T> {
    name: Option<NameId>,
    producer: Option<CommandProducer>,
    next: Option<usize>,
    value: T,
}
struct OwnedNameSlot {
    index: usize,
    replace: bool,
}
struct OwnedNames<T> {
    rows: Box<[OwnedName<T>]>,
    len: usize,
}
impl<T: Default> OwnedNames<T> {
    fn load(capacity: usize) -> Self {
        Self {
            rows: (0..capacity)
                .map(|_| OwnedName {
                    name: None,
                    producer: None,
                    next: None,
                    value: T::default(),
                })
                .collect(),
            len: 0,
        }
    }
    fn index(&self, mut head: Option<usize>, producer: Option<CommandProducer>) -> Option<usize> {
        while let Some(index) = head {
            let row = &self.rows[index];
            if row.producer == producer {
                return Some(index);
            }
            head = row.next;
        }
        None
    }
    fn prepare(
        &self,
        head: Option<usize>,
        producer: Option<CommandProducer>,
    ) -> Result<OwnedNameSlot, NamesError> {
        let previous = self.index(head, producer);
        let index = previous
            .or_else(|| {
                self.rows[..self.len]
                    .iter()
                    .position(|row| row.name.is_none())
            })
            .unwrap_or(self.len);
        if index >= self.rows.len() {
            return Err(NamesError::Capacity);
        }
        Ok(OwnedNameSlot {
            index,
            replace: previous.is_some(),
        })
    }
    fn set(
        &mut self,
        head: &mut Option<usize>,
        name: NameId,
        producer: Option<CommandProducer>,
        value: T,
        slot: OwnedNameSlot,
    ) {
        let row = &mut self.rows[slot.index];
        let next = if slot.replace { row.next } else { *head };
        *row = OwnedName {
            name: Some(name),
            producer,
            next,
            value,
        };
        self.len = self.len.max(slot.index + 1);
        if !slot.replace {
            *head = Some(slot.index);
        }
    }
    fn remove(&mut self, head: &mut Option<usize>, index: usize) {
        let next = self.rows[index].next;
        if *head == Some(index) {
            *head = next;
        } else {
            let mut current = *head;
            while let Some(previous) = current {
                if self.rows[previous].next == Some(index) {
                    self.rows[previous].next = next;
                    break;
                }
                current = self.rows[previous].next;
            }
        }
        self.rows[index] = OwnedName {
            name: None,
            producer: None,
            next: None,
            value: T::default(),
        };
    }
    fn retain<E>(
        &mut self,
        names: &NameTable,
        entries: &mut [E],
        head: fn(&mut E) -> &mut Option<usize>,
        retained: &mut impl FnMut(CommandProducer) -> bool,
    ) {
        for index in 0..self.len {
            if self.rows[index]
                .producer
                .is_some_and(|producer| !retained(producer))
                && let Some(name) = self.rows[index].name.and_then(|name| names.folded(name))
            {
                self.remove(head(&mut entries[name.0 as usize]), index);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandStep {
    Continue,
    Fallback,
    External(NameId),
    Done,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandReply {
    Handled,
    Unhandled,
}
#[derive(Clone, Copy)]
enum CommandMode {
    Single,
    Drain,
}
#[derive(Clone, Copy)]
enum CommandPhase {
    Read,
    AwaitFallback,
    AwaitExternal(NameId),
    Replied(CommandReply),
    Done,
}
/// Owned progress; the caller retains its parsed scratch across fallback calls.
pub struct CommandRun {
    mode: CommandMode,
    phase: CommandPhase,
    context: Context,
    executed: u32,
    aliases: u32,
}
impl CommandRun {
    pub fn single(context: Context) -> Self {
        Self {
            mode: CommandMode::Single,
            phase: CommandPhase::Read,
            context,
            executed: 0,
            aliases: 0,
        }
    }
    pub fn drain() -> Self {
        Self {
            mode: CommandMode::Drain,
            ..Self::single(Context::default())
        }
    }
    pub fn context(&self) -> Context {
        self.context
    }
    /// Accept exactly one reply to the outstanding fallback request.
    pub fn reply(&mut self, reply: CommandReply) -> bool {
        if !matches!(
            self.phase,
            CommandPhase::AwaitFallback | CommandPhase::AwaitExternal(_)
        ) {
            return false;
        }
        self.phase = CommandPhase::Replied(reply);
        true
    }
    fn complete_command(&mut self) -> CommandStep {
        match self.mode {
            CommandMode::Single => {
                self.phase = CommandPhase::Done;
                CommandStep::Done
            }
            CommandMode::Drain => {
                self.phase = CommandPhase::Read;
                CommandStep::Continue
            }
        }
    }
}
pub struct Console<H> {
    pub cvars: Cvars,
    pub editor: crate::editor::Editor,
    pub scrollback: crate::presentation::Scrollback,
    buffer: CommandBuffer,
    commands: Vec<NameId>,
    command_by_name: Box<[CommandSlot<H>]>,
    completions: OwnedNames<()>,
    aliases: OwnedNames<FixedText<1024>>,
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
        let command_capacity = cvars.names.capacity();
        let command_by_name = (0..command_capacity).map(|_| CommandSlot::new()).collect();
        let mut console = Self {
            cvars,
            editor: crate::editor::Editor::default(),
            scrollback: crate::presentation::Scrollback::default(),
            command_by_name,
            completions: OwnedNames::load(command_capacity),
            buffer: CommandBuffer::new(),
            commands: Vec::with_capacity(command_capacity),
            aliases: OwnedNames::load(alias_capacity),
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
                let _ = console.register_entry(name, Command::Button(entry.action));
            }
        }
        Ok(console)
    }
    pub fn register(&mut self, name: &'static str, function: CommandFn<H>) -> bool {
        self.register_entry(name, Command::Function(function))
            .is_some()
    }
    /// An application-owned continuation still resolves through this table,
    /// before cvars and aliases, and shares the ordinary command reply boundary.
    /// Reattaching a host to the same console retains the registered identity.
    pub fn register_external(&mut self, name: &'static str) -> Option<NameId> {
        self.register_entry(name, Command::External)
    }
    fn register_entry(&mut self, name: &'static str, command: Command<H>) -> Option<NameId> {
        let exact = self.cvars.names.intern(name.as_bytes()).ok()?;
        let id = self.cvars.names.folded(exact)?;
        let slot = &mut self.command_by_name[id.0 as usize];
        if let Some(previous) = slot.command {
            return matches!((previous, command), (Command::External, Command::External))
                .then_some(id);
        }
        if slot.completion.is_some() {
            return None;
        }
        slot.command = Some(command);
        self.insert_command_name(exact);
        Some(id)
    }
    fn insert_command_name(&mut self, exact: NameId) {
        let names = &self.cvars.names;
        let name = names.get(exact).unwrap_or_default();
        let at = self.commands.partition_point(|&id| {
            compare_folded(names.get(id).unwrap_or_default(), name) == Ordering::Less
        });
        self.commands.insert(at, exact);
    }
    /// A native null-function command advertises its name without taking
    /// precedence over cvars, aliases or module fallback.
    pub fn register_completion(
        &mut self,
        name: &[u8],
        producer: CommandProducer,
    ) -> Result<(), NamesError> {
        let previous = self
            .cvars
            .names
            .find_folded(name)
            .map(|id| &self.command_by_name[id.0 as usize]);
        if previous.is_some_and(|slot| slot.command.is_some()) {
            return Ok(());
        }
        let head = previous.and_then(|slot| slot.completion);
        let prepared = self.completions.prepare(head, Some(producer))?;
        let exact = self.cvars.names.intern(name)?;
        let id = self.cvars.names.folded(exact).ok_or(NamesError::Capacity)?;
        self.completions.set(
            &mut self.command_by_name[id.0 as usize].completion,
            exact,
            Some(producer),
            (),
            prepared,
        );
        if head.is_none() {
            self.insert_command_name(exact);
        }
        Ok(())
    }
    /// A module cannot remove an engine command or another live producer's
    /// registration for the same folded name.
    pub fn remove_completion(&mut self, name: &[u8], producer: CommandProducer) -> bool {
        let Some(id) = self.cvars.names.find_folded(name) else {
            return false;
        };
        let slot = &mut self.command_by_name[id.0 as usize];
        let Some(index) = self.completions.index(slot.completion, Some(producer)) else {
            return false;
        };
        self.completions.remove(&mut slot.completion, index);
        self.retain_command_names();
        true
    }
    fn retain_command_names(&mut self) {
        let names = &self.cvars.names;
        let entries = &self.command_by_name;
        self.commands.retain(|&id| {
            names.folded(id).is_some_and(|id| {
                let slot = &entries[id.0 as usize];
                slot.command.is_some() || slot.completion.is_some()
            })
        });
    }

    pub fn append(&mut self, text: &str, context: Context) -> Result<(), TextError> {
        self.buffer.append(text, context)
    }
    pub fn begin_frame(&mut self) {
        self.buffer.begin_frame();
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
        self.completions.retain(
            &self.cvars.names,
            &mut self.command_by_name,
            |slot| &mut slot.completion,
            &mut retained,
        );
        self.aliases.retain(
            &self.cvars.names,
            &mut self.command_by_name,
            |slot| &mut slot.alias,
            &mut retained,
        );
        self.retain_command_names();
        result
    }
    fn alias_index(&self, name: NameId, producer: Option<CommandProducer>) -> Option<usize> {
        self.aliases
            .index(self.command_by_name[name.0 as usize].alias, producer)
    }
    fn remove_alias(&mut self, index: usize) {
        if let Some(name) = self.aliases.rows[index]
            .name
            .and_then(|name| self.cvars.names.folded(name))
        {
            self.aliases
                .remove(&mut self.command_by_name[name.0 as usize].alias, index);
        }
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
        let Some(mut parser) = self.parser.take() else {
            host.print(format_args!("Console execution already active\n"));
            return;
        };
        let mut run = CommandRun::drain();
        loop {
            match self.step(host, &mut run, &mut parser) {
                CommandStep::Continue => {}
                CommandStep::Fallback => {
                    run.reply(CommandReply::Unhandled);
                }
                CommandStep::External(_) => {
                    host.print(format_args!(
                        "Command {} is unavailable in this context\n",
                        parser.arguments().get(0)
                    ));
                    run.reply(CommandReply::Handled);
                }
                CommandStep::Done => break,
            }
        }
        self.parser = Some(parser);
    }

    /// Run one shared dispatch boundary, ending all console borrows on fallback.
    pub fn step(
        &mut self,
        host: &mut H,
        run: &mut CommandRun,
        parser: &mut ParsedCommand,
    ) -> CommandStep {
        if matches!(
            run.phase,
            CommandPhase::AwaitFallback | CommandPhase::AwaitExternal(_) | CommandPhase::Replied(_)
        ) && let Some(producer) = run.context.producer
            && !host.command_producer_live(producer)
        {
            let _ = self.discard_producer(host, producer);
            return run.complete_command();
        }
        match run.phase {
            CommandPhase::Done => return CommandStep::Done,
            CommandPhase::AwaitFallback => return CommandStep::Fallback,
            CommandPhase::AwaitExternal(id) => return CommandStep::External(id),
            CommandPhase::Replied(reply) => {
                if reply == CommandReply::Unhandled {
                    host.print(format_args!(
                        "Unknown command \"{}\"\n",
                        parser.arguments().get(0)
                    ));
                }
                return run.complete_command();
            }
            CommandPhase::Read => {}
        }
        let context = if matches!(run.mode, CommandMode::Drain) {
            if self.buffer.is_empty() {
                run.phase = CommandPhase::Done;
                return CommandStep::Done;
            }
            if let Some(producer) = self
                .buffer
                .wait_context()
                .and_then(|context| context.producer)
                && !host.command_producer_live(producer)
            {
                let _ = self.discard_producer(host, producer);
                return CommandStep::Continue;
            }
            if !self.buffer.ready() {
                run.phase = CommandPhase::Done;
                return CommandStep::Done;
            }
            if run.executed >= 4096 {
                host.print(format_args!("Command frame limit\n"));
                run.phase = CommandPhase::Done;
                return CommandStep::Done;
            }
            parser.clear();
            let Some((context, result)) = self.buffer.next_line(&mut parser.line) else {
                run.phase = CommandPhase::Done;
                return CommandStep::Done;
            };
            run.executed += 1;
            run.context = context;
            if let Some(producer) = context.producer
                && !host.command_producer_live(producer)
            {
                let _ = self.discard_producer(host, producer);
                return CommandStep::Continue;
            }
            if let Err(error) = result {
                self.reject_expansion(error, context, host);
                host.print(format_args!("Command rejected: {error:?}\n"));
                return CommandStep::Continue;
            }
            context
        } else {
            run.executed += 1;
            if let Some(producer) = run.context.producer
                && !host.command_producer_live(producer)
            {
                let _ = self.discard_producer(host, producer);
                return run.complete_command();
            }
            run.context
        };
        if matches!(
            context.source,
            RuleSetId::Quake2 | RuleSetId::Quake2Rerelease
        ) {
            let raw = parser.line.as_bytes();
            let expanded = if raw.contains(&b'$') {
                command_text::expand_engine_bytes(&mut parser.line, &mut parser.tokens, |name| {
                    self.cvars.macro_value(name, context)
                })
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
                return run.complete_command();
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
                return run.complete_command();
            }
        };
        let Ok(name) = std::str::from_utf8(args.get_bytes(0)) else {
            host.print(format_args!("Command name is not UTF-8\n"));
            return run.complete_command();
        };
        if name.is_empty() {
            return run.complete_command();
        }
        let id = self.cvars.lookup_name(name);
        let result = if let Some((id, command)) = id.and_then(|id| {
            self.command_by_name[id.0 as usize]
                .command
                .map(|command| (id, command))
        }) {
            match command {
                Command::Function(function) => function(self, host, &args, context),
                Command::Button(action) => self.button(host, &args, context, action),
                Command::External => {
                    run.phase = CommandPhase::AwaitExternal(id);
                    return CommandStep::External(id);
                }
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
            .map(|index| &self.aliases.rows[index])
        {
            run.aliases += 1;
            if run.aliases > 16 {
                host.print(format_args!("Alias expansion limit\n"));
                return run.complete_command();
            }
            let result = self.buffer.insert_bytes(alias.value.as_bytes(), context);
            if let Err(error) = result {
                self.reject_expansion(error, context, host);
            }
            result.map_err(Into::into)
        } else {
            run.phase = CommandPhase::AwaitFallback;
            return CommandStep::Fallback;
        };
        if let Err(error) = result {
            host.print(format_args!("Command {name} rejected: {error:?}\n"));
        }
        run.complete_command()
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
            for alias in self.aliases.rows[..self.aliases.len]
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
                host.print_bytes(alias.value.as_bytes());
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
        let head = self
            .cvars
            .lookup_name(name)
            .and_then(|id| self.command_by_name[id.0 as usize].alias);
        let prepared = self
            .aliases
            .prepare(head, context.producer)
            .map_err(|_| CommandError::Text(TextError::TooLong))?;
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
        self.aliases.set(
            &mut self.command_by_name[folded.0 as usize].alias,
            exact,
            context.producer,
            text,
            prepared,
        );
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
            host.print_bytes(self.cvars.names.get(*c).unwrap_or_default());
            host.print_bytes(b"\n");
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
