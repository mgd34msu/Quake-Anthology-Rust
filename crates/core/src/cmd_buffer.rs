//! Buffered command dispatch ported from `src/core/commands/index.ts`
//! (Quake `cmd.c`, Quake II `qcommon/cmd.c`, Quake III `cmd.c`), with
//! documentation shapes from `src/core/commands/documentation.ts` and the
//! wildcard matcher from `src/core/commands/filter.ts`.
//!
//! This module owns the queue layer above the text primitives in
//! [`crate::cmd`]: text appends and inserts, console aliases, `wait` and
//! multi-frame wait countdowns, `exec` script reads with completion
//! callbacks, the Quake II overflow queue, and the program revision counter.
//! Queue entries and script origins carry their [`CommandContext`] through
//! insertions, nested execution, and waits, so alternating seats keep their
//! own source identity.
//!
//! Script reads resolve through [`BufferServices`]. A read that is not ready
//! yet parks the drain (`Pending`) and resumes in order once the host
//! supplies the text with [`CommandBuffer::resolve_pending_script`]; that is
//! the synchronous form of the donor's asynchronous host queue, with the
//! same per-frame ordering and no threads involved.

use std::collections::VecDeque;
use std::rc::Rc;

use thiserror::Error;

use crate::cmd::{
    ascii_fold, command_separator_offset_bytes, expand_command_macros, source_command_text, tokenize_command, CmdError,
    Dialect, EngineText, TextMode,
};
use crate::cvar::{cvar_value_text, flags, q2_flags, CvarRegistry, SetCommandKind};
use crate::identity::{ClientId, SeatId, SessionId};
use crate::numeric::{native_atof, native_atoi};

/// Documented command help (donor `CommandDocumentation`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandDocumentation {
    /// One-line summary.
    pub summary: String,
    /// Usage line.
    pub usage: String,
    /// Example invocations.
    pub examples: Vec<String>,
    /// Allowed values, when the command takes an enum.
    pub allowed_values: Option<Vec<String>>,
}

/// Port of the donor `sourceFilter` (`Com_Filter`): prefix matching with
/// non-backtracking `*` runs, `?` wildcards, and `[...]` classes.
pub fn source_filter(filter_text: &str, name_text: &str, case_sensitive: bool) -> Result<bool, CmdError> {
    let filter: Vec<char> = source_command_text(filter_text).chars().collect();
    let name: Vec<char> = source_command_text(name_text).chars().collect();
    let byte = |text: &[char], index: usize| -> Result<u32, CmdError> {
        if index > text.len() {
            return Err(CmdError::TokenizedOverflow);
        }
        Ok(if index == text.len() { 0 } else { text[index] as u32 })
    };
    // Signed char 0xff promotes to EOF, the one defined negative input.
    let fold = |value: u32| -> i32 {
        if case_sensitive {
            if value > 127 {
                value as i32 - 256
            } else {
                value as i32
            }
        } else if value == 255 {
            -1
        } else if (97..=122).contains(&value) {
            value as i32 - 32
        } else {
            value as i32
        }
    };
    let mut pattern = 0;
    let mut cursor = 0;
    while byte(&filter, pattern)? != 0 {
        if byte(&filter, pattern)? == 42 {
            pattern += 1;
            let start = pattern;
            while byte(&filter, pattern)? != 0 && byte(&filter, pattern)? != 42 && byte(&filter, pattern)? != 63 {
                pattern += 1;
            }
            let segment: String = filter[start..pattern].iter().collect();
            if segment.len() >= 1024 {
                return Err(CmdError::TokenOverflow(
                    "Source filter star run exceeds its scratch buffer".to_string(),
                ));
            }
            if !segment.is_empty() {
                let segment: Vec<char> = segment.chars().collect();
                let mut found: Option<usize> = None;
                if segment.len() <= name.len() + 1 {
                    for index in cursor..=name.len().saturating_sub(segment.len()) {
                        if index < cursor {
                            continue;
                        }
                        let mut matches = true;
                        for (offset, expected) in segment.iter().enumerate() {
                            if fold(byte(&name, index + offset)?) != fold(*expected as u32) {
                                matches = false;
                                break;
                            }
                        }
                        if matches {
                            found = Some(index);
                            break;
                        }
                    }
                }
                let Some(found) = found else {
                    return Ok(false);
                };
                cursor = found + segment.len();
            }
        } else if byte(&filter, pattern)? == 63 {
            byte(&name, cursor)?;
            pattern += 1;
            cursor += 1;
        } else if byte(&filter, pattern)? == 91 && byte(&filter, pattern + 1)? == 91 {
            pattern += 1;
        } else if byte(&filter, pattern)? == 91 {
            pattern += 1;
            let mut found = false;
            while byte(&filter, pattern)? != 0 && !found {
                if byte(&filter, pattern)? == 93 && byte(&filter, pattern + 1)? != 93 {
                    break;
                }
                if byte(&filter, pattern + 1)? == 45
                    && byte(&filter, pattern + 2)? != 0
                    && (byte(&filter, pattern + 2)? != 93 || byte(&filter, pattern + 3)? == 93)
                {
                    let current = fold(byte(&name, cursor)?);
                    if current >= fold(byte(&filter, pattern)?) && current <= fold(byte(&filter, pattern + 2)?) {
                        found = true;
                    }
                    pattern += 3;
                } else {
                    if fold(byte(&filter, pattern)?) == fold(byte(&name, cursor)?) {
                        found = true;
                    }
                    pattern += 1;
                }
            }
            if !found {
                return Ok(false);
            }
            while byte(&filter, pattern)? != 0 {
                if byte(&filter, pattern)? == 93 && byte(&filter, pattern + 1)? != 93 {
                    break;
                }
                pattern += 1;
            }
            pattern += 1;
            cursor += 1;
        } else {
            if fold(byte(&filter, pattern)?) != fold(byte(&name, cursor)?) {
                return Ok(false);
            }
            pattern += 1;
            cursor += 1;
        }
    }
    Ok(true)
}

/// Where a queued command came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandOrigin {
    /// Local console input.
    LocalConsole,
    /// Server console input.
    ServerConsole,
    /// Local seat input.
    LocalSeat {
        /// Seat handle.
        seat: SeatId,
        /// Client handle.
        client: ClientId,
    },
    /// Remote client input.
    RemoteClient {
        /// Client handle.
        client: ClientId,
    },
    /// Script text executing on behalf of a caller.
    Script {
        /// Script name.
        name: String,
        /// Caller origin.
        caller: Box<CommandOrigin>,
    },
}

/// Execution context retained by every queued chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandContext {
    /// Owning session.
    pub session: SessionId,
    /// Input origin.
    pub origin: CommandOrigin,
}

impl CommandContext {
    /// Build a context for a session and origin.
    #[must_use]
    pub fn new(session: SessionId, origin: CommandOrigin) -> Self {
        Self { session, origin }
    }
}

fn same_origin(left: &CommandOrigin, right: &CommandOrigin) -> bool {
    match (left, right) {
        (CommandOrigin::LocalConsole, CommandOrigin::LocalConsole)
        | (CommandOrigin::ServerConsole, CommandOrigin::ServerConsole) => true,
        (
            CommandOrigin::LocalSeat { seat, client },
            CommandOrigin::LocalSeat {
                seat: other_seat,
                client: other_client,
            },
        ) => seat == other_seat && client == other_client,
        (CommandOrigin::RemoteClient { client }, CommandOrigin::RemoteClient { client: other }) => client == other,
        (
            CommandOrigin::Script { name, caller },
            CommandOrigin::Script {
                name: other_name,
                caller: other_caller,
            },
        ) => name == other_name && same_origin(caller, other_caller),
        _ => false,
    }
}

fn source_client(origin: &CommandOrigin) -> Option<&ClientId> {
    let mut current = origin;
    while let CommandOrigin::Script { caller, .. } = current {
        current = caller;
    }
    match current {
        CommandOrigin::LocalSeat { client, .. } | CommandOrigin::RemoteClient { client } => Some(client),
        _ => None,
    }
}

/// Outcome of one script read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptResult {
    /// The script text executed.
    Completed,
    /// No script text exists under the name.
    Missing,
    /// The read failed.
    Failed(String),
}

/// Completion event delivered after a script's text drains.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptCompletion {
    /// Script name.
    pub name: String,
    /// Script execution context.
    pub source: CommandContext,
    /// Read outcome.
    pub result: ScriptResult,
}

/// Host answer to a script read request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptRead {
    /// The read settled immediately (`None` means missing).
    Ready(Option<String>),
    /// The read is not ready; the drain parks until the host resolves it.
    Pending,
    /// The read failed immediately.
    Failed(String),
}

/// Snapshot of one invocation for host-owned routing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForwardedCommand {
    /// Invocation context.
    pub source: CommandContext,
    /// Whether the line reached dispatch without alias expansion.
    pub direct: bool,
    /// Dispatch dialect.
    pub dialect: Dialect,
    /// Argument vector.
    pub argv: Vec<String>,
    /// Raw text after the first token.
    pub args_text: String,
    /// Raw segment text.
    pub raw: String,
}

/// Host services behind buffered dispatch.
pub trait BufferServices {
    /// Read a script by name.
    fn read_script(&mut self, name: &str, source: &CommandContext) -> ScriptRead;
    /// Forward a line to the server (Quake II/III unknown commands, `cmd`).
    fn forward_to_server(&mut self, command: &ForwardedCommand);
    /// Handle a command the buffer does not own; `registered` reports
    /// whether the buffer has a non-builtin entry under the name.
    fn external_command(&mut self, command: &ForwardedCommand, registered: bool) -> bool {
        let _ = (command, registered);
        false
    }
    /// Quake III client-game fallback.
    fn client_game(&mut self, command: &ForwardedCommand) -> bool {
        let _ = command;
        false
    }
    /// Quake III server-game fallback.
    fn server_game(&mut self, command: &ForwardedCommand) -> bool {
        let _ = command;
        false
    }
    /// Quake III UI fallback.
    fn ui_game(&mut self, command: &ForwardedCommand) -> bool {
        let _ = command;
        false
    }
    /// Gate one invocation; `false` consumes the line without running it.
    fn allow_command(&mut self, command: &ForwardedCommand) -> bool {
        let _ = command;
        true
    }
}

/// Services with no scripts, no server, and no fallbacks.
#[derive(Debug, Default)]
pub struct NullBufferServices {
    forwarded: Vec<ForwardedCommand>,
}

impl NullBufferServices {
    /// Open empty services.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Lines forwarded so far.
    #[must_use]
    pub fn forwarded(&self) -> &[ForwardedCommand] {
        &self.forwarded
    }
}

impl BufferServices for NullBufferServices {
    fn read_script(&mut self, _name: &str, _source: &CommandContext) -> ScriptRead {
        ScriptRead::Ready(None)
    }

    fn forward_to_server(&mut self, command: &ForwardedCommand) {
        self.forwarded.push(command.clone());
    }
}

/// Per-frame hooks for the synchronous host driver.
pub trait FrameHooks {
    /// Whether the drain keeps consuming chunks.
    fn should_continue(&mut self) -> bool {
        true
    }

    /// Runs after every dispatched line, before the next chunk is read.
    fn after_dispatch(&mut self) {}
}

/// Failure of a buffered command operation.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BufferError {
    /// Command text failed validation.
    #[error(transparent)]
    Command(#[from] CmdError),
    /// A cvar operation failed.
    #[error("{0}")]
    Cvar(String),
    /// A buffer or line limit is not a positive integer.
    #[error("{0} must be a positive integer")]
    BadLimit(String),
    /// Command input belongs to another session.
    #[error("Command input belongs to another session")]
    SessionMismatch,
    /// A registry of another dialect was passed to the buffer.
    #[error("Commands and cvars require the same dialect")]
    DialectMismatch,
    /// A disconnected client's queued work is gone.
    #[error("Command client is disconnected")]
    DisconnectedClient,
    /// A drain started while the buffer was already draining.
    #[error("{0}")]
    Reentrant(String),
    /// A command line overflowed its line buffer.
    #[error("Command line overflows source line buffer")]
    LineOverflow,
    /// An insertion overflowed the buffer.
    #[error("Cbuf_InsertText overflows source sizebuf")]
    InsertOverflow,
    /// The overflow queue was used outside Quake II.
    #[error("Deferred command buffers belong to Quake II")]
    DeferDialect,
    /// Overflow-queue text did not fit the buffer.
    #[error("Deferred commands overflow source sizebuf")]
    DeferOverflow,
    /// A replacement buffer cannot hold the pending program.
    #[error("Replacement command buffer cannot hold pending commands")]
    ReplacementOverflow,
    /// Quake III has no console aliases.
    #[error("Quake III uses vstr instead of console aliases")]
    Q3Alias,
    /// An alias body overflowed its buffer.
    #[error("Alias body overflows source cmd[1024]")]
    AliasOverflow,
    /// A Quake I registration has a null callback.
    #[error("Quake I command registration has a null callback")]
    NullCallback,
    /// A `set` value overflowed its combined buffer.
    #[error("Cvar_Set overflows source combined buffer")]
    SetOverflow,
    /// A command batch exceeded its bounds.
    #[error("{0}")]
    BadBatch(String),
}

impl From<crate::cvar::CvarError> for BufferError {
    fn from(error: crate::cvar::CvarError) -> Self {
        Self::Cvar(error.to_string())
    }
}

/// Construction options for [`CommandBuffer`].
#[derive(Debug, Clone, Default)]
pub struct BufferOptions {
    /// Maximum queued bytes (16384 for Quake III, 8192 otherwise).
    pub max_buffer_length: Option<usize>,
    /// Maximum line bytes (1024).
    pub max_command_length: Option<usize>,
    /// Register the donor builtins (`wait`, `exec`, `echo`, ...).
    pub builtins: Option<bool>,
    /// Startup text consumed by `stuffcmds`.
    pub startup_command_text: Option<String>,
    /// Command line words consumed by `stuffcmds`.
    pub command_line: Vec<String>,
}

impl BufferOptions {
    /// Options with donor builtins enabled.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

fn positive_limit(value: Option<usize>, fallback: usize, label: &str) -> Result<usize, BufferError> {
    let limit = value.unwrap_or(fallback);
    if limit < 1 {
        return Err(BufferError::BadLimit(label.to_string()));
    }
    Ok(limit)
}

#[derive(Clone)]
struct RegisteredEntry {
    name: String,
    handler: Option<CommandHandler>,
    documentation: Option<CommandDocumentation>,
    builtin: bool,
}

/// Command handler. Handlers run inside the dispatch frame and reach the
/// queue, the registry, and the host through [`Invocation`]; closures keep
/// their own state behind shared handles.
pub type CommandHandler = Rc<dyn for<'b, 'c, 's> Fn(&mut Invocation<'b, 'c, 's>)>;

type Printer = Box<dyn FnMut(&str, Option<&CommandContext>)>;
type ScriptListener = Box<dyn FnMut(&ScriptCompletion)>;

#[derive(Debug, Clone)]
struct AliasEntry {
    name: String,
    value: String,
    text_mode: TextMode,
    dialect: Dialect,
}

#[derive(Debug, Clone)]
enum CommandChunk {
    Text {
        dialect: Dialect,
        text: EngineText,
        source: CommandContext,
        direct: bool,
        text_mode: TextMode,
    },
    Completion {
        event: ScriptCompletion,
        dialect: Dialect,
        text_mode: TextMode,
    },
}

#[derive(Debug, Clone)]
struct ExecutionFrame {
    dialect: Dialect,
    source: CommandContext,
    text_mode: TextMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PendingState {
    Waiting,
    Ready(Option<String>),
    Failed(String),
}

#[derive(Debug, Clone)]
struct PendingScript {
    name: String,
    source: CommandContext,
    dialect: Dialect,
    text_mode: TextMode,
    state: PendingState,
}

/// One live command invocation.
pub struct Invocation<'b, 'c, 's> {
    /// Invocation context.
    pub source: CommandContext,
    /// Whether the line reached dispatch without alias expansion.
    pub direct: bool,
    /// Dispatch dialect.
    pub dialect: Dialect,
    /// Argument vector.
    pub argv: Vec<String>,
    /// Raw text after the first token.
    pub args_text: String,
    /// Raw segment text.
    pub raw: String,
    buffer: &'b mut CommandBuffer,
    cvars: &'c mut CvarRegistry,
    services: &'s mut dyn BufferServices,
}

impl<'b, 'c, 's> Invocation<'b, 'c, 's> {
    /// Arguments after the command name.
    #[must_use]
    pub fn args(&self) -> &[String] {
        if self.argv.is_empty() {
            &[]
        } else {
            &self.argv[1..]
        }
    }

    /// Queue text behind the pending program.
    pub fn append(&mut self, text: &str) -> Result<(), BufferError> {
        let (dialect, text_mode) = self.buffer.frame_mode();
        let source = self.source.clone();
        self.buffer.append_for(text, source, false, text_mode, dialect)
    }

    /// Queue text ahead of the pending program.
    pub fn insert(&mut self, text: &str) -> Result<(), BufferError> {
        let (dialect, text_mode) = self.buffer.frame_mode();
        let source = self.source.clone();
        self.buffer.insert_for(text, source, false, None, text_mode, dialect)
    }

    /// Dispatch text immediately inside the current frame.
    pub fn execute_now(&mut self, text: &str) -> Result<usize, BufferError> {
        if text.is_empty() {
            return self.buffer.execute(self.cvars, self.services);
        }
        let value = source_command_text(text);
        let frame = self.buffer.frames.last().cloned().unwrap_or_else(|| ExecutionFrame {
            dialect: self.buffer.dialect,
            source: self.buffer.context.clone(),
            text_mode: TextMode::Source,
        });
        self.buffer.dispatch(
            value,
            frame.source,
            false,
            frame.text_mode,
            frame.dialect,
            self.cvars,
            self.services,
        )
    }

    /// Run `exec` for this invocation.
    pub fn execute_script(&mut self) -> Result<(), BufferError> {
        let argv = self.argv.clone();
        let raw = self.raw.clone();
        let source = self.source.clone();
        let dialect = self.dialect;
        self.buffer.execute_script(&argv, &raw, &source, dialect, self.services)
    }

    /// Forward this invocation to the server.
    pub fn forward_to_server(&mut self) {
        let command = self.forwarded();
        self.services.forward_to_server(&command);
    }

    /// Print through the buffer printer.
    pub fn print(&mut self, text: &str) {
        let source = self.source.clone();
        self.buffer.print(text, Some(&source));
    }

    /// Borrow the invocation registry.
    pub fn cvars(&mut self) -> &mut CvarRegistry {
        self.cvars
    }

    fn forwarded(&self) -> ForwardedCommand {
        ForwardedCommand {
            source: self.source.clone(),
            direct: self.direct,
            dialect: self.dialect,
            argv: self.argv.clone(),
            args_text: self.args_text.clone(),
            raw: self.raw.clone(),
        }
    }
}

/// Buffered command queue.
pub struct CommandBuffer {
    dialect: Dialect,
    context: CommandContext,
    handlers: Vec<RegisteredEntry>,
    builtin_dialect: Option<Dialect>,
    aliases: Vec<AliasEntry>,
    chunks: VecDeque<CommandChunk>,
    deferred: Vec<CommandChunk>,
    wait_frames: i32,
    wait_dialect: Option<Dialect>,
    wait_source: Option<CommandContext>,
    pending_script: Option<PendingScript>,
    frames: Vec<ExecutionFrame>,
    tokens: Vec<String>,
    alias_count: u32,
    maximum_buffer: usize,
    maximum_command: usize,
    startup_command_text: Option<String>,
    command_line: Vec<String>,
    printer: Printer,
    on_script_complete: Option<ScriptListener>,
    script_listeners: Vec<(u64, ScriptListener)>,
    next_listener: u64,
    retired_clients: Vec<ClientId>,
    revision: u64,
    async_draining: bool,
    with_builtins: bool,
    explicit_maximum: Option<usize>,
}

impl CommandBuffer {
    /// Open a buffer for a dialect and owning context.
    pub fn new(dialect: Dialect, context: CommandContext, options: BufferOptions) -> Result<Self, BufferError> {
        let maximum_buffer = positive_limit(
            options.max_buffer_length,
            if dialect == Dialect::Q3 { 16384 } else { 8192 },
            "maxBufferLength",
        )?;
        let maximum_command = positive_limit(options.max_command_length, 1024, "maxCommandLength")?;
        let with_builtins = options.builtins.unwrap_or(true);
        let mut buffer = Self {
            dialect,
            context,
            handlers: Vec::new(),
            builtin_dialect: None,
            aliases: Vec::new(),
            chunks: VecDeque::new(),
            deferred: Vec::new(),
            wait_frames: 0,
            wait_dialect: None,
            wait_source: None,
            pending_script: None,
            frames: Vec::new(),
            tokens: Vec::new(),
            alias_count: 0,
            maximum_buffer,
            maximum_command,
            startup_command_text: options.startup_command_text,
            command_line: options.command_line,
            printer: Box::new(|_, _| {}),
            on_script_complete: None,
            script_listeners: Vec::new(),
            next_listener: 1,
            retired_clients: Vec::new(),
            revision: 0,
            async_draining: false,
            with_builtins,
            explicit_maximum: options.max_buffer_length,
        };
        if with_builtins {
            buffer.ensure_builtins(dialect);
        }
        Ok(buffer)
    }

    /// Buffer dialect.
    #[must_use]
    pub fn dialect(&self) -> Dialect {
        self.dialect
    }

    /// Owning context.
    #[must_use]
    pub fn context(&self) -> &CommandContext {
        &self.context
    }

    /// Program revision: every queue, alias, wait, drain, and dispatch
    /// mutation bumps it, so hosts can detect program changes cheaply.
    #[must_use]
    pub fn program_revision(&self) -> u64 {
        self.revision
    }

    /// Whether chunks or a parked script read are waiting.
    #[must_use]
    pub fn has_pending_commands(&self) -> bool {
        !self.chunks.is_empty() || self.pending_script.is_some()
    }

    /// Queued text ahead of the drain.
    #[must_use]
    pub fn pending_text(&self) -> String {
        self.chunks
            .iter()
            .filter_map(|chunk| match chunk {
                CommandChunk::Text { text, .. } => Some(text.to_display()),
                CommandChunk::Completion { .. } => None,
            })
            .collect()
    }

    /// Queued engine bytes ahead of the drain: every byte counts once.
    fn pending_len(&self) -> usize {
        self.chunks
            .iter()
            .filter_map(|chunk| match chunk {
                CommandChunk::Text { text, .. } => Some(text.len()),
                CommandChunk::Completion { .. } => None,
            })
            .sum()
    }

    /// Text held in the Quake II overflow queue.
    #[must_use]
    pub fn deferred_text(&self) -> String {
        self.deferred
            .iter()
            .filter_map(|chunk| match chunk {
                CommandChunk::Text { text, .. } => Some(text.to_display()),
                CommandChunk::Completion { .. } => None,
            })
            .collect()
    }

    /// Engine bytes held in the Quake II overflow queue.
    fn deferred_len(&self) -> usize {
        self.deferred
            .iter()
            .filter_map(|chunk| match chunk {
                CommandChunk::Text { text, .. } => Some(text.len()),
                CommandChunk::Completion { .. } => None,
            })
            .sum()
    }

    /// Argument vector of the most recently dispatched line.
    #[must_use]
    pub fn tokenized_arguments(&self) -> &[String] {
        &self.tokens
    }

    /// Maximum queued bytes.
    #[must_use]
    pub fn maximum_buffer_length(&self) -> usize {
        self.maximum_buffer
    }

    /// Maximum line bytes.
    #[must_use]
    pub fn maximum_command_length(&self) -> usize {
        self.maximum_command
    }

    /// Context of the running frame, if any.
    #[must_use]
    pub fn execution_context(&self) -> Option<&CommandContext> {
        self.frames.last().map(|frame| &frame.source)
    }

    /// Name of the parked script read, if any.
    #[must_use]
    pub fn pending_script_name(&self) -> Option<&str> {
        self.pending_script.as_ref().map(|read| read.name.as_str())
    }

    /// Install the print sink (default drops output).
    pub fn set_printer(&mut self, printer: impl FnMut(&str, Option<&CommandContext>) + 'static) {
        self.printer = Box::new(printer);
    }

    /// Install the primary script-completion callback.
    pub fn set_on_script_complete(&mut self, listener: Option<impl FnMut(&ScriptCompletion) + 'static>) {
        self.on_script_complete = listener.map(|listener| {
            let boxed: Box<dyn FnMut(&ScriptCompletion)> = Box::new(listener);
            boxed
        });
    }

    /// Bind an extra script-completion listener; returns its binding id.
    pub fn bind_script_completion(&mut self, listener: impl FnMut(&ScriptCompletion) + 'static) -> u64 {
        let id = self.next_listener;
        self.next_listener += 1;
        self.script_listeners.push((id, Box::new(listener)));
        id
    }

    /// Release a script-completion binding.
    pub fn unbind_script_completion(&mut self, id: u64) -> bool {
        let before = self.script_listeners.len();
        self.script_listeners.retain(|(known, _)| *known != id);
        self.script_listeners.len() != before
    }

    fn print(&mut self, text: &str, source: Option<&CommandContext>) {
        (self.printer)(text, source);
    }

    fn execution_dialect(&self) -> Dialect {
        self.frames.last().map_or(self.dialect, |frame| frame.dialect)
    }

    fn frame_mode(&self) -> (Dialect, TextMode) {
        self.frames.last().map_or((self.dialect, TextMode::Source), |frame| {
            (frame.dialect, frame.text_mode)
        })
    }

    /// Register a command; `None` installs a fallback name. Returns false
    /// when the name is taken or, outside Quake III, already a live cvar.
    pub fn register(
        &mut self,
        name_input: &str,
        handler: Option<CommandHandler>,
        documentation: Option<CommandDocumentation>,
        cvars: &CvarRegistry,
    ) -> Result<bool, BufferError> {
        let name = source_command_text(name_input);
        if self.exists(&name) {
            if handler.is_some() || self.execution_dialect() != Dialect::Q3 {
                let source = self.frame_source();
                self.print(&format!("Cmd_AddCommand: {name} already defined\n"), source.as_ref());
            }
            return Ok(false);
        }
        if self.execution_dialect() != Dialect::Q3 && !cvars.variable_string(&name).is_empty() {
            let source = self.frame_source();
            self.print(
                &format!("Cmd_AddCommand: {name} already defined as a var\n"),
                source.as_ref(),
            );
            return Ok(false);
        }
        self.handlers.push(RegisteredEntry {
            name,
            handler,
            documentation,
            builtin: false,
        });
        Ok(true)
    }

    /// Whether a command is registered.
    #[must_use]
    pub fn exists(&self, name_input: &str) -> bool {
        let name = source_command_text(name_input);
        self.handlers.iter().any(|entry| entry.name == name)
    }

    /// Remove a command; returns whether one was registered.
    pub fn unregister(&mut self, name_input: &str) -> bool {
        let name = source_command_text(name_input);
        let before = self.handlers.len();
        self.handlers.retain(|entry| entry.name != name);
        self.handlers.len() != before
    }

    /// Registered command names, newest first.
    #[must_use]
    pub fn registered_names(&self) -> Vec<String> {
        self.handlers.iter().rev().map(|entry| entry.name.clone()).collect()
    }

    /// Documentation for a registered command (ASCII-folded).
    #[must_use]
    pub fn command_documentation(&self, name: &str) -> Option<&CommandDocumentation> {
        let folded = ascii_fold(name);
        self.handlers
            .iter()
            .find(|entry| ascii_fold(&entry.name) == folded)
            .and_then(|entry| entry.documentation.as_ref())
    }

    /// Visit every registered command name.
    pub fn complete_names(&self, visitor: &mut dyn FnMut(&str)) {
        for entry in &self.handlers {
            visitor(&entry.name);
        }
    }

    /// Complete a partial command name (plus Quake II/QuakeWorld aliases).
    #[must_use]
    pub fn complete(&self, partial_input: &str) -> Option<String> {
        let partial = source_command_text(partial_input);
        if partial.is_empty() {
            return None;
        }
        let dialect = self.execution_dialect();
        let mut names = self.registered_names();
        if dialect.is_q2() || dialect == Dialect::Q1Quakeworld {
            names.extend(self.aliases.iter().map(|alias| alias.name.clone()));
        }
        if dialect != Dialect::Q1Netquake && names.iter().any(|name| name == &partial) {
            return Some(partial);
        }
        names.into_iter().find(|name| {
            if dialect == Dialect::Q3 {
                ascii_fold(name).starts_with(&ascii_fold(&partial))
            } else {
                name.starts_with(&partial)
            }
        })
    }

    /// Define or replace an alias. Quake III rejects aliases in favor of
    /// `vstr`.
    pub fn define_alias(&mut self, name_input: &str, text_input: &str) -> Result<bool, BufferError> {
        if self.execution_dialect() == Dialect::Q3 {
            return Err(BufferError::Q3Alias);
        }
        let name_text = EngineText::from(name_input);
        let text = source_command_text(text_input);
        if name_text.len() >= 32 {
            let source = self.frame_source();
            self.print("Alias name is too long\n", source.as_ref());
            return Ok(false);
        }
        let name = name_text.to_display();
        self.revision += 1;
        let (dialect, text_mode) = self.frame_mode();
        if let Some(existing) = self.aliases.iter_mut().find(|alias| alias.name == name) {
            existing.value = text;
            existing.text_mode = text_mode;
            existing.dialect = dialect;
        } else {
            self.aliases.insert(
                0,
                AliasEntry {
                    name,
                    value: text,
                    text_mode,
                    dialect,
                },
            );
        }
        Ok(true)
    }

    /// Alias expansion text (ASCII-folded).
    #[must_use]
    pub fn alias_value(&self, name: &str) -> Option<&str> {
        let folded = ascii_fold(name);
        self.aliases
            .iter()
            .find(|alias| ascii_fold(&alias.name) == folded)
            .map(|alias| alias.value.as_str())
    }

    /// Alias names (`vstr` replaces aliases on Quake III, so none there).
    #[must_use]
    pub fn alias_names(&self) -> Vec<String> {
        if self.execution_dialect() == Dialect::Q3 {
            return Vec::new();
        }
        self.aliases.iter().map(|alias| alias.name.clone()).collect()
    }

    fn frame_source(&self) -> Option<CommandContext> {
        self.frames.last().map(|frame| frame.source.clone())
    }

    fn input_context(&self, source: Option<&CommandContext>) -> Result<CommandContext, BufferError> {
        let context = source
            .cloned()
            .or_else(|| self.frame_source())
            .unwrap_or_else(|| self.context.clone());
        if context.session != self.context.session {
            return Err(BufferError::SessionMismatch);
        }
        if let Some(client) = source_client(&context.origin) {
            if self.retired_clients.contains(client) {
                return Err(BufferError::DisconnectedClient);
            }
        }
        Ok(context)
    }

    fn input_text_mode(&self, source: &CommandContext, direct: bool) -> TextMode {
        match source.origin {
            CommandOrigin::LocalSeat { .. } | CommandOrigin::LocalConsole => self
                .frames
                .last()
                .map_or(if direct { TextMode::Console } else { TextMode::Source }, |frame| {
                    frame.text_mode
                }),
            _ => TextMode::Source,
        }
    }

    /// Queue text behind the pending program.
    pub fn append(
        &mut self,
        text: &str,
        source: Option<&CommandContext>,
        dialect: Option<Dialect>,
    ) -> Result<(), BufferError> {
        let context = self.input_context(source)?;
        let direct = self.frames.is_empty() && !matches!(context.origin, CommandOrigin::Script { .. });
        let text_mode = self.input_text_mode(&context, direct);
        let dialect = dialect.unwrap_or_else(|| self.execution_dialect());
        self.append_for(text, context, direct, text_mode, dialect)
    }

    /// Queue text ahead of the pending program.
    pub fn insert(
        &mut self,
        text: &str,
        source: Option<&CommandContext>,
        dialect: Option<Dialect>,
    ) -> Result<(), BufferError> {
        let context = self.input_context(source)?;
        let direct = self.frames.is_empty() && !matches!(context.origin, CommandOrigin::Script { .. });
        let text_mode = self.input_text_mode(&context, direct);
        let dialect = dialect.unwrap_or_else(|| self.execution_dialect());
        self.insert_for(text, context, direct, None, text_mode, dialect)
    }

    fn append_for(
        &mut self,
        input: &str,
        source: CommandContext,
        direct: bool,
        text_mode: TextMode,
        dialect: Dialect,
    ) -> Result<(), BufferError> {
        let text = EngineText::from(input);
        let limit = self
            .explicit_maximum
            .unwrap_or(if dialect == Dialect::Q3 { 16384 } else { 8192 });
        if self.pending_len() + text.len() >= limit {
            self.print("Cbuf_AddText: overflow\n", None);
            return Ok(());
        }
        if !text.is_empty() {
            self.chunks.push_back(CommandChunk::Text {
                dialect,
                text,
                source,
                direct,
                text_mode,
            });
            self.revision += 1;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_for(
        &mut self,
        input: &str,
        source: CommandContext,
        direct: bool,
        completion: Option<ScriptCompletion>,
        text_mode: TextMode,
        dialect: Dialect,
    ) -> Result<(), BufferError> {
        let mut text = EngineText::from(input);
        if dialect == Dialect::Q1Quakeworld || dialect == Dialect::Q3 {
            text.push(b'\n');
        }
        let limit = self
            .explicit_maximum
            .unwrap_or(if dialect == Dialect::Q3 { 16384 } else { 8192 });
        if dialect == Dialect::Q3 {
            if self.pending_len() + text.len() > limit {
                self.print("Cbuf_InsertText overflowed\n", None);
                return Ok(());
            }
        } else {
            if text.len() >= limit {
                self.print("Cbuf_AddText: overflow\n", None);
                return Ok(());
            }
            if self.pending_len() + text.len() > limit {
                return Err(BufferError::InsertOverflow);
            }
        }
        if completion.is_some() || !text.is_empty() {
            self.revision += 1;
        }
        if let Some(event) = completion {
            self.chunks.push_front(CommandChunk::Completion {
                event,
                dialect,
                text_mode,
            });
        }
        if !text.is_empty() {
            self.chunks.push_front(CommandChunk::Text {
                dialect,
                text,
                source,
                direct,
                text_mode,
            });
        }
        Ok(())
    }

    /// Move the pending program into the Quake II overflow queue.
    pub fn copy_to_defer(&mut self) -> Result<(), BufferError> {
        if !self.execution_dialect().is_q2() {
            return Err(BufferError::DeferDialect);
        }
        self.revision += 1;
        self.deferred = std::mem::take(&mut self.chunks).into_iter().collect();
        Ok(())
    }

    /// Move the Quake II overflow queue ahead of the pending program.
    pub fn insert_from_defer(&mut self) -> Result<(), BufferError> {
        if !self.execution_dialect().is_q2() {
            return Err(BufferError::DeferDialect);
        }
        if self.pending_len() + self.deferred_len() > self.maximum_buffer {
            return Err(BufferError::DeferOverflow);
        }
        self.revision += 1;
        let mut restored: VecDeque<CommandChunk> = std::mem::take(&mut self.deferred).into_iter().collect();
        restored.append(&mut self.chunks);
        self.chunks = restored;
        Ok(())
    }

    /// Take over another buffer's pending program (same session only).
    pub fn copy_pending_from(&mut self, previous: &CommandBuffer) -> Result<(), BufferError> {
        if !self.frames.is_empty() || !previous.frames.is_empty() || self.async_draining {
            return Err(BufferError::Reentrant(
                "Cannot copy commands during execution".to_string(),
            ));
        }
        if self.context.session != previous.context.session {
            return Err(BufferError::SessionMismatch);
        }
        if previous.pending_len() + previous.deferred_len() >= self.maximum_buffer {
            return Err(BufferError::ReplacementOverflow);
        }
        self.chunks = previous.chunks.clone();
        self.deferred = previous.deferred.clone();
        self.wait_frames = previous.wait_frames;
        self.wait_dialect = previous.wait_dialect;
        self.wait_source.clone_from(&previous.wait_source);
        self.pending_script.clone_from(&previous.pending_script);
        self.alias_count = previous.alias_count;
        self.tokens.clone_from(&previous.tokens);
        self.startup_command_text.clone_from(&previous.startup_command_text);
        self.aliases.clone_from(&previous.aliases);
        self.retired_clients.clone_from(&previous.retired_clients);
        self.revision += 1;
        Ok(())
    }

    /// Drop every chunk, parked read, and wait owned by a disconnected
    /// client, so a seat's next occupant never inherits them.
    pub fn discard_client(&mut self, client: &ClientId) {
        self.retired_clients.push(client.clone());
        let owned = |source: &CommandContext| source_client(&source.origin) == Some(client);
        self.chunks.retain(|chunk| match chunk {
            CommandChunk::Text { source, .. } => !owned(source),
            CommandChunk::Completion { event, .. } => !owned(&event.source),
        });
        self.deferred.retain(|chunk| match chunk {
            CommandChunk::Text { source, .. } => !owned(source),
            CommandChunk::Completion { event, .. } => !owned(&event.source),
        });
        if self.pending_script.as_ref().is_some_and(|read| owned(&read.source)) {
            self.pending_script = None;
        }
        if self.wait_source.as_ref().is_some_and(owned) {
            self.wait_frames = 0;
            self.wait_dialect = None;
            self.wait_source = None;
        }
        self.revision += 1;
    }

    /// Supply the text of the parked script read (`None` means missing).
    pub fn resolve_pending_script(&mut self, text: Option<String>) {
        if let Some(read) = self.pending_script.as_mut() {
            read.state = PendingState::Ready(text);
            self.revision += 1;
        }
    }

    /// Fail the parked script read.
    pub fn fail_pending_script(&mut self, error: String) {
        if let Some(read) = self.pending_script.as_mut() {
            read.state = PendingState::Failed(error);
            self.revision += 1;
        }
    }

    fn check_registry(&self, cvars: &CvarRegistry) -> Result<(), BufferError> {
        if cvars.dialect() != self.dialect {
            return Err(BufferError::DialectMismatch);
        }
        Ok(())
    }

    /// Drain one frame: lines run until the queue empties, a `wait`
    /// boundary lands, or a script read parks.
    pub fn execute(
        &mut self,
        cvars: &mut CvarRegistry,
        services: &mut dyn BufferServices,
    ) -> Result<usize, BufferError> {
        if self.async_draining {
            return Err(BufferError::Reentrant(
                "Command buffer is already draining asynchronously".to_string(),
            ));
        }
        self.check_registry(cvars)?;
        self.alias_count = 0;
        self.drain(cvars, services, None)
    }

    /// Drain one frame with host hooks: `after_dispatch` runs after every
    /// dispatched line. This is the synchronous form of the donor host
    /// queue drain, with the same ordering and no threads.
    pub fn execute_hooked(
        &mut self,
        cvars: &mut CvarRegistry,
        services: &mut dyn BufferServices,
        hooks: &mut dyn FrameHooks,
    ) -> Result<usize, BufferError> {
        if self.async_draining || !self.frames.is_empty() {
            return Err(BufferError::Reentrant(
                "Command buffer is already executing".to_string(),
            ));
        }
        self.check_registry(cvars)?;
        self.async_draining = true;
        self.alias_count = 0;
        let outcome = self.drain(cvars, services, Some(hooks));
        self.async_draining = false;
        outcome
    }

    /// Advance one host frame: the overflow queue rejoins first, then an
    /// empty queue ticks a pending multi-frame wait down, otherwise the
    /// frame drains.
    pub fn advance_program_frame(
        &mut self,
        cvars: &mut CvarRegistry,
        services: &mut dyn BufferServices,
    ) -> Result<usize, BufferError> {
        if self.async_draining || !self.frames.is_empty() {
            return Err(BufferError::Reentrant(
                "Command buffer is already executing".to_string(),
            ));
        }
        if !self.deferred.is_empty() {
            self.insert_from_defer()?;
        }
        if self.chunks.is_empty() && self.pending_script.is_none() && self.wait_frames != 0 {
            self.revision += 1;
            self.wait_frames -= 1;
            return Ok(0);
        }
        self.execute(cvars, services)
    }

    /// Dispatch text immediately (`None`/empty drains the queue instead).
    pub fn execute_now(
        &mut self,
        text: Option<&str>,
        source: Option<&CommandContext>,
        dialect: Option<Dialect>,
        cvars: &mut CvarRegistry,
        services: &mut dyn BufferServices,
    ) -> Result<usize, BufferError> {
        let value = text.map_or_else(String::new, source_command_text);
        if value.is_empty() {
            return self.execute(cvars, services);
        }
        self.check_registry(cvars)?;
        let context = self.input_context(source)?;
        let direct = self.frames.is_empty();
        let text_mode = self.input_text_mode(&context, direct);
        let dialect = dialect.unwrap_or_else(|| self.execution_dialect());
        self.dispatch(value, context, direct, text_mode, dialect, cvars, services)
    }

    fn drain(
        &mut self,
        cvars: &mut CvarRegistry,
        services: &mut dyn BufferServices,
        mut hooks: Option<&mut dyn FrameHooks>,
    ) -> Result<usize, BufferError> {
        let mut executed = 0;
        loop {
            if hooks.as_mut().is_some_and(|hooks| !hooks.should_continue()) {
                break;
            }
            if self.pending_script.is_some() {
                let state = self.pending_script.as_ref().map(|read| read.state.clone());
                match state {
                    Some(PendingState::Waiting) | None => return Ok(executed),
                    Some(PendingState::Ready(text)) => {
                        let read = self.pending_script.take().expect("parked script read vanished");
                        self.revision += 1;
                        self.insert_script(&read.name, text, &read.source, read.dialect, read.text_mode)?;
                        continue;
                    }
                    Some(PendingState::Failed(error)) => {
                        let read = self.pending_script.take().expect("parked script read vanished");
                        self.revision += 1;
                        self.print(&format!("couldn't exec {}: {error}\n", read.name), Some(&read.source));
                        let event = Self::completion(&read.name, &read.source, ScriptResult::Failed(error));
                        self.chunks.push_front(CommandChunk::Completion {
                            event,
                            dialect: read.dialect,
                            text_mode: read.text_mode,
                        });
                        continue;
                    }
                }
            }
            if self.wait_dialect == Some(Dialect::Q3) && self.wait_frames != 0 {
                self.revision += 1;
                self.wait_frames -= 1;
                break;
            }
            let Some(first) = self.chunks.front().cloned() else {
                break;
            };
            match first {
                CommandChunk::Completion {
                    event,
                    dialect,
                    text_mode,
                } => {
                    self.revision += 1;
                    self.chunks.pop_front();
                    self.frames.push(ExecutionFrame {
                        dialect,
                        source: event.source.clone(),
                        text_mode,
                    });
                    self.ensure_builtins(dialect);
                    if let Some(listener) = self.on_script_complete.as_mut() {
                        listener(&event);
                    }
                    for (_, listener) in &mut self.script_listeners {
                        listener(&event);
                    }
                    self.pop_frame();
                }
                CommandChunk::Text {
                    dialect,
                    source,
                    direct,
                    text_mode,
                    ..
                } => {
                    let joined = self.joined_text();
                    let mut offset = command_separator_offset_bytes(joined.as_bytes(), dialect);
                    if offset >= self.maximum_command {
                        if dialect != Dialect::Q3 {
                            return Err(BufferError::LineOverflow);
                        }
                        offset = self.maximum_command - 1;
                    }
                    let line = EngineText::from_bytes(&joined.as_bytes()[..offset]).to_display();
                    let consumed = if offset == joined.len() { offset } else { offset + 1 };
                    self.consume(consumed);
                    let count = self.dispatch(line, source, direct, text_mode, dialect, cvars, services)?;
                    if count != 0 {
                        executed += count;
                        if let Some(hooks) = hooks.as_mut() {
                            hooks.after_dispatch();
                        }
                    }
                    if self.wait_dialect != Some(Dialect::Q3) && self.wait_frames != 0 {
                        self.revision += 1;
                        self.wait_frames = 0;
                        break;
                    }
                }
            }
        }
        Ok(executed)
    }

    /// Merge the leading run of chunks into one scannable buffer. Quake II
    /// script text joins the caller's following bytes across completion
    /// nodes; every other boundary stops the run.
    fn joined_text(&self) -> EngineText {
        let Some(CommandChunk::Text {
            dialect,
            source,
            direct,
            text_mode,
            ..
        }) = self.chunks.front()
        else {
            return EngineText::new();
        };
        let dialect = *dialect;
        let direct = *direct;
        let text_mode = *text_mode;
        let mut origin = source.origin.clone();
        let mut resumed_caller = false;
        let mut text = EngineText::new();
        for chunk in &self.chunks {
            match chunk {
                CommandChunk::Completion { event, .. } => {
                    if !dialect.is_q2()
                        || !matches!(origin, CommandOrigin::Script { .. })
                        || event.result != ScriptResult::Completed
                        || !same_origin(&event.source.origin, &origin)
                    {
                        break;
                    }
                    let CommandOrigin::Script { caller, .. } = &origin else {
                        break;
                    };
                    origin = (**caller).clone();
                    resumed_caller = true;
                }
                CommandChunk::Text {
                    dialect: next_dialect,
                    text: next_text,
                    source: next_source,
                    direct: next_direct,
                    text_mode: next_mode,
                } => {
                    if *next_dialect != dialect || *next_mode != text_mode {
                        break;
                    }
                    if (!resumed_caller && *next_direct != direct) || !same_origin(&next_source.origin, &origin) {
                        break;
                    }
                    text.push_text(next_text);
                }
            }
        }
        text
    }

    fn consume(&mut self, mut count: usize) {
        if count > 0 {
            self.revision += 1;
        }
        let mut index = 0;
        while count > 0 {
            let Some(chunk) = self.chunks.get(index).cloned() else {
                return;
            };
            match chunk {
                CommandChunk::Completion { .. } => {
                    index += 1;
                }
                CommandChunk::Text { mut text, .. } => {
                    if text.len() > count {
                        let rest = text.split_off(count);
                        if let Some(CommandChunk::Text { text: slot, .. }) = self.chunks.get_mut(index) {
                            *slot = rest;
                        }
                        return;
                    }
                    self.chunks.remove(index);
                    count -= text.len();
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn dispatch(
        &mut self,
        raw: String,
        source: CommandContext,
        direct: bool,
        text_mode: TextMode,
        dialect: Dialect,
        cvars: &mut CvarRegistry,
        services: &mut dyn BufferServices,
    ) -> Result<usize, BufferError> {
        self.revision += 1;
        let expanded = if dialect.is_q2() {
            let expanded = expand_command_macros(
                &raw,
                &|name| {
                    let variable = cvars.get(name);
                    if cvars.dialect().is_q2()
                        && variable
                            .as_ref()
                            .is_some_and(|variable| variable.flags & q2_flags::PRIVATE != 0)
                    {
                        String::new()
                    } else {
                        variable.map_or_else(String::new, |variable| variable.value)
                    }
                },
                &mut |text| self.print(text, None),
                text_mode,
            )?;
            let Some(expanded) = expanded else {
                self.tokens = Vec::new();
                return Ok(0);
            };
            expanded
        } else {
            raw.clone()
        };
        let tokens = tokenize_command(&expanded, dialect, text_mode)?;
        let Some(name) = tokens.argv.first().cloned() else {
            self.tokens = tokens.argv;
            return Ok(0);
        };
        self.tokens = tokens.argv.clone();
        let direct = direct
            && matches!(
                source.origin,
                CommandOrigin::LocalConsole | CommandOrigin::LocalSeat { .. }
            );
        self.frames.push(ExecutionFrame {
            dialect,
            source: source.clone(),
            text_mode,
        });
        self.ensure_builtins(dialect);
        let forwarded = ForwardedCommand {
            source: source.clone(),
            direct,
            dialect,
            argv: tokens.argv.clone(),
            args_text: tokens.args_text.clone(),
            raw: raw.clone(),
        };
        let outcome = self.dispatch_inner(&forwarded, &name, cvars, services);
        self.pop_frame();
        self.flush_notifications(cvars);
        outcome
    }

    fn dispatch_inner(
        &mut self,
        forwarded: &ForwardedCommand,
        name: &str,
        cvars: &mut CvarRegistry,
        services: &mut dyn BufferServices,
    ) -> Result<usize, BufferError> {
        if !services.allow_command(forwarded) {
            return Ok(1);
        }
        let folded = ascii_fold(name);
        let selected = self
            .handlers
            .iter()
            .rposition(|entry| ascii_fold(&entry.name) == folded);
        let selected_builtin = selected.map(|index| self.handlers[index].builtin).unwrap_or(false);
        if !selected_builtin && services.external_command(forwarded, selected.is_some()) {
            return Ok(1);
        }
        if let Some(index) = selected {
            if self.execution_dialect() == Dialect::Q3 {
                let entry = self.handlers.remove(index);
                self.handlers.push(entry);
            }
            let handler = self
                .handlers
                .iter()
                .rev()
                .find(|entry| ascii_fold(&entry.name) == folded)
                .and_then(|entry| entry.handler.clone());
            match handler {
                Some(handler) => {
                    let mut invocation = Invocation {
                        source: forwarded.source.clone(),
                        direct: forwarded.direct,
                        dialect: forwarded.dialect,
                        argv: forwarded.argv.clone(),
                        args_text: forwarded.args_text.clone(),
                        raw: forwarded.raw.clone(),
                        buffer: self,
                        cvars,
                        services,
                    };
                    handler(&mut invocation);
                    return Ok(1);
                }
                None => {
                    if self.execution_dialect().is_q2() {
                        let source = forwarded.source.clone();
                        let dialect = self.execution_dialect();
                        let text_mode = self.frames.last().map_or(TextMode::Source, |frame| frame.text_mode);
                        let line = format!("cmd {}", forwarded.raw);
                        return self.dispatch(line, source, false, text_mode, dialect, cvars, services);
                    }
                    if self.execution_dialect() == Dialect::Q3 {
                        self.fallback(forwarded, cvars, services)?;
                        return Ok(1);
                    }
                    return Err(BufferError::NullCallback);
                }
            }
        }
        if self.execution_dialect() != Dialect::Q3 {
            let alias = self
                .aliases
                .iter()
                .find(|alias| ascii_fold(&alias.name) == folded)
                .cloned();
            if let Some(alias) = alias {
                if self.execution_dialect().is_q2() {
                    self.alias_count += 1;
                    if self.alias_count == 16 {
                        let source = forwarded.source.clone();
                        self.print("ALIAS_LOOP_COUNT\n", Some(&source));
                        return Ok(1);
                    }
                }
                let source = forwarded.source.clone();
                self.insert_for(&alias.value, source, false, None, alias.text_mode, alias.dialect)?;
                return Ok(1);
            }
        }
        self.fallback(forwarded, cvars, services)?;
        Ok(1)
    }

    fn fallback(
        &mut self,
        command: &ForwardedCommand,
        cvars: &mut CvarRegistry,
        services: &mut dyn BufferServices,
    ) -> Result<(), BufferError> {
        let name = command.argv.first().cloned().unwrap_or_default();
        if let Some(variable) = cvars.get(&name) {
            match command.argv.get(1) {
                None => {
                    if self.execution_dialect() == Dialect::Q3 {
                        self.print(
                            &format!(
                                "\"{}\" is:\"{}^7\" default:\"{}^7\"\n",
                                variable.name, variable.value, variable.reset_value
                            ),
                            Some(&command.source),
                        );
                        if let Some(latched) = variable.latched_value {
                            self.print(&format!("latched: \"{latched}\"\n"), Some(&command.source));
                        }
                    } else {
                        self.print(
                            &format!("\"{}\" is \"{}\"\n", variable.name, variable.value),
                            Some(&command.source),
                        );
                    }
                }
                Some(value) => {
                    cvars.set(&variable.name, value, false)?;
                }
            }
            return Ok(());
        }
        if self.execution_dialect() == Dialect::Q3
            && (services.client_game(command) || services.server_game(command) || services.ui_game(command))
        {
            return Ok(());
        }
        if self.execution_dialect() == Dialect::Q3 || self.execution_dialect().is_q2() {
            services.forward_to_server(command);
            return Ok(());
        }
        let loud = |name: &str| {
            cvars
                .get(name)
                .is_some_and(|variable| variable.numeric_value != 0.0 && !variable.numeric_value.is_nan())
        };
        if self.execution_dialect() != Dialect::Q1Quakeworld || loud("cl_warncmd") || loud("developer") {
            self.print(&format!("Unknown command \"{name}\"\n"), Some(&command.source));
        }
        Ok(())
    }

    fn flush_notifications(&mut self, cvars: &mut CvarRegistry) {
        for line in cvars.take_notifications() {
            let source = self.frame_source();
            self.print(&line, source.as_ref());
        }
    }

    fn pop_frame(&mut self) {
        self.frames.pop();
        let dialect = self.execution_dialect();
        self.ensure_builtins(dialect);
    }

    fn completion(name: &str, caller: &CommandContext, result: ScriptResult) -> ScriptCompletion {
        ScriptCompletion {
            name: name.to_string(),
            source: CommandContext {
                session: caller.session.clone(),
                origin: CommandOrigin::Script {
                    name: name.to_string(),
                    caller: Box::new(caller.origin.clone()),
                },
            },
            result,
        }
    }

    fn insert_script(
        &mut self,
        filename: &str,
        file: Option<String>,
        caller: &CommandContext,
        dialect: Dialect,
        text_mode: TextMode,
    ) -> Result<(), BufferError> {
        let Some(file) = file else {
            self.print(&format!("couldn't exec {filename}\n"), Some(caller));
            let event = Self::completion(filename, caller, ScriptResult::Missing);
            self.chunks.push_front(CommandChunk::Completion {
                event,
                dialect,
                text_mode,
            });
            return Ok(());
        };
        self.print(&format!("execing {filename}\n"), Some(caller));
        let mut text = source_command_text(&file);
        if dialect.is_q1() && !text.ends_with('\n') {
            text.push('\n');
        }
        let completion = Self::completion(filename, caller, ScriptResult::Completed);
        let source = completion.source.clone();
        self.insert_for(&text, source, false, Some(completion), text_mode, dialect)
    }

    fn execute_script(
        &mut self,
        argv: &[String],
        raw: &str,
        source: &CommandContext,
        dialect: Dialect,
        services: &mut dyn BufferServices,
    ) -> Result<(), BufferError> {
        if argv.len() != 2 {
            self.print("exec <filename> : execute a script file\n", Some(source));
            return Ok(());
        }
        let text_mode = self.frames.last().map_or(TextMode::Source, |frame| frame.text_mode);
        if self.pending_script.is_some() {
            let line = format!("{raw}\n");
            self.insert_for(&line, source.clone(), false, None, text_mode, dialect)?;
            return Ok(());
        }
        let requested = argv.get(1).cloned().unwrap_or_default();
        let leaf = requested.rsplit('/').next().unwrap_or(&requested);
        let filename = if dialect == Dialect::Q3 && !leaf.contains('.') {
            format!("{requested}.cfg")
        } else {
            requested
        };
        match services.read_script(&filename, source) {
            ScriptRead::Ready(text) => {
                self.insert_script(&filename, text, source, dialect, text_mode)?;
            }
            ScriptRead::Pending => {
                self.pending_script = Some(PendingScript {
                    name: filename,
                    source: source.clone(),
                    dialect,
                    text_mode,
                    state: PendingState::Waiting,
                });
                self.revision += 1;
            }
            ScriptRead::Failed(error) => {
                self.print(&format!("couldn't exec {filename}: {error}\n"), Some(source));
                let event = Self::completion(&filename, source, ScriptResult::Failed(error));
                self.chunks.push_front(CommandChunk::Completion {
                    event,
                    dialect,
                    text_mode,
                });
            }
        }
        Ok(())
    }
}

impl CommandBuffer {
    fn ensure_builtins(&mut self, dialect: Dialect) {
        if !self.with_builtins || self.builtin_dialect == Some(dialect) {
            return;
        }
        self.handlers.retain(|entry| !entry.builtin);
        self.builtin_dialect = Some(dialect);
        self.install_builtins(dialect);
    }

    fn install_builtins(&mut self, dialect: Dialect) {
        let mut ordered: Vec<(&str, CommandHandler, Option<CommandDocumentation>)> = Vec::new();
        let mut extra: Vec<(&str, CommandHandler, Option<CommandDocumentation>)> = Vec::new();
        let order: &[&str] = if dialect == Dialect::Q1Netquake {
            &["stuffcmds", "exec", "echo", "alias", "cmd", "wait"]
        } else if dialect == Dialect::Q1Quakeworld {
            &["stuffcmds", "exec", "echo", "alias", "wait", "cmd"]
        } else if dialect.is_q2() {
            &["cmdlist", "exec", "echo", "alias", "wait", "set", "cvarlist", "cmd"]
        } else {
            &[
                "toggle",
                "set",
                "sets",
                "setu",
                "seta",
                "reset",
                "cvarlist",
                "cvar_restart",
                "cmdlist",
                "exec",
                "vstr",
                "echo",
                "wait",
                "cmd",
            ]
        };
        let push = |name: &'static str,
                    handler: CommandHandler,
                    documentation: Option<CommandDocumentation>,
                    table: &mut Vec<(&str, CommandHandler, Option<CommandDocumentation>)>| {
            table.push((name, handler, documentation));
        };
        push(
            "wait",
            Rc::new(|inv: &mut Invocation| {
                inv.buffer.revision += 1;
                inv.buffer.wait_dialect = Some(inv.dialect);
                inv.buffer.wait_source = Some(inv.source.clone());
                inv.buffer.wait_frames = if inv.dialect == Dialect::Q3 && inv.argv.len() == 2 {
                    native_atoi(inv.argv.get(1).map_or("", String::as_str))
                } else {
                    1
                };
            }),
            None,
            &mut ordered,
        );
        push(
            "echo",
            Rc::new(|inv: &mut Invocation| {
                let gap = if inv.args().is_empty() { "" } else { " " };
                inv.print(&format!("{}{gap}\n", inv.args().join(" ")));
            }),
            Some(CommandDocumentation {
                summary: "Print text to the console.".to_string(),
                usage: "echo <text>".to_string(),
                examples: vec!["echo hello".to_string()],
                allowed_values: None,
            }),
            &mut ordered,
        );
        push(
            "cmd",
            Rc::new(|inv: &mut Invocation| {
                inv.forward_to_server();
            }),
            None,
            &mut ordered,
        );
        push(
            "exec",
            Rc::new(|inv: &mut Invocation| {
                let _ = inv.execute_script();
            }),
            None,
            &mut ordered,
        );
        if dialect != Dialect::Q3 {
            push(
                "alias",
                Rc::new(|inv: &mut Invocation| {
                    Self::alias_command(inv);
                }),
                None,
                &mut ordered,
            );
        }
        if dialect.is_q1() {
            push(
                "stuffcmds",
                Rc::new(|inv: &mut Invocation| {
                    Self::stuffcmds_command(inv);
                }),
                None,
                &mut ordered,
            );
        }
        push(
            "set",
            Rc::new(|inv: &mut Invocation| {
                Self::set_command(inv, 0);
            }),
            Some(CommandDocumentation {
                summary: "Set a console variable.".to_string(),
                usage: "set <variable> <value>".to_string(),
                examples: vec!["set name \"Player\"".to_string()],
                allowed_values: None,
            }),
            &mut ordered,
        );
        push(
            "cmdlist",
            Rc::new(|inv: &mut Invocation| {
                Self::cmdlist_command(inv);
            }),
            Some(CommandDocumentation {
                summary: "List registered console commands.".to_string(),
                usage: if dialect == Dialect::Q3 {
                    "cmdlist [pattern]"
                } else {
                    "cmdlist"
                }
                .to_string(),
                examples: vec!["cmdlist".to_string()],
                allowed_values: None,
            }),
            &mut ordered,
        );
        push(
            "cvarlist",
            Rc::new(|inv: &mut Invocation| {
                Self::cvarlist_command(inv);
            }),
            Some(CommandDocumentation {
                summary: "List visible console variables and their current values.".to_string(),
                usage: if dialect == Dialect::Q3 {
                    "cvarlist [pattern]"
                } else {
                    "cvarlist"
                }
                .to_string(),
                examples: vec!["cvarlist".to_string()],
                allowed_values: None,
            }),
            &mut ordered,
        );
        for name in ["inc", "dec"] {
            push(
                name,
                Rc::new(move |inv: &mut Invocation| {
                    Self::adjust_command(inv, name);
                }),
                None,
                &mut extra,
            );
        }
        push(
            "resetall",
            Rc::new(|inv: &mut Invocation| {
                for variable in inv.cvars().snapshots(0) {
                    if inv.buffer.canonical_name(&variable.name) != variable.name {
                        continue;
                    }
                    let _ = inv.cvars().reset_console(&variable.name, true);
                }
            }),
            None,
            &mut extra,
        );
        push(
            "vstr",
            Rc::new(|inv: &mut Invocation| {
                if inv.argv.len() != 2 {
                    inv.print("vstr <variablename> : execute a variable command\n");
                    return;
                }
                let name = inv.argv.get(1).cloned().unwrap_or_default();
                let value = inv
                    .cvars()
                    .get(&name)
                    .map_or_else(String::new, |variable| variable.value);
                let _ = inv.insert(&format!("{value}\n"));
            }),
            Some(CommandDocumentation {
                summary: "Execute the invoking owner's variable as commands in this command buffer.".to_string(),
                usage: "vstr <variablename>".to_string(),
                examples: vec!["vstr nextmap".to_string()],
                allowed_values: None,
            }),
            &mut extra,
        );
        for (name, kind) in [
            ("seta", SetCommandKind::Archive),
            ("setu", SetCommandKind::Userinfo),
            ("sets", SetCommandKind::Serverinfo),
        ] {
            push(
                name,
                Rc::new(move |inv: &mut Invocation| {
                    let variable = inv.argv.get(1).cloned().unwrap_or_default();
                    if inv.argv.get(1).is_none()
                        || inv.argv.len() < 3
                        || inv.dialect == Dialect::Q3 && inv.argv.len() != 3
                    {
                        inv.print(&format!("Usage: {name} <variable> <value>\n"));
                        return;
                    }
                    if inv.cvars().dialect() == Dialect::Q3 {
                        let flag = match kind {
                            SetCommandKind::Archive => flags::ARCHIVE,
                            SetCommandKind::Userinfo => flags::USER_INFO,
                            SetCommandKind::Serverinfo => flags::SERVER_INFO,
                        };
                        Self::set_command(inv, flag);
                        return;
                    }
                    let value = inv.args()[1..].join(" ");
                    let _ = inv.cvars().set_command_flags(&variable, &value, kind);
                }),
                None,
                &mut extra,
            );
        }
        push(
            "toggle",
            Rc::new(|inv: &mut Invocation| {
                Self::toggle_command(inv);
            }),
            None,
            &mut extra,
        );
        push(
            "reset",
            Rc::new(|inv: &mut Invocation| {
                if inv.argv.len() < 2 || inv.dialect == Dialect::Q3 && inv.argv.len() != 2 {
                    inv.print("reset <variable> : reset a cvar\n");
                    return;
                }
                let name = inv.argv.get(1).cloned().unwrap_or_default();
                let _ = inv.cvars().reset_console(&name, false);
            }),
            None,
            &mut extra,
        );
        push(
            "cvar_restart",
            Rc::new(|inv: &mut Invocation| {
                if inv.cvars().dialect() == Dialect::Q3 {
                    let _ = inv.cvars().reset_all();
                }
            }),
            None,
            &mut extra,
        );
        let mut sequence: Vec<(&str, CommandHandler, Option<CommandDocumentation>)> = Vec::new();
        for name in order {
            if let Some(position) = ordered.iter().position(|(known, _, _)| known == name) {
                sequence.push(ordered.remove(position));
            } else if let Some(position) = extra.iter().position(|(known, _, _)| known == name) {
                sequence.push(extra.remove(position));
            }
        }
        sequence.extend(ordered);
        sequence.extend(extra);
        for (name, handler, documentation) in sequence {
            if self.exists(name) {
                continue;
            }
            self.handlers.push(RegisteredEntry {
                name: name.to_string(),
                handler: Some(handler),
                documentation,
                builtin: true,
            });
        }
    }

    fn canonical_name(&self, name: &str) -> String {
        if self.dialect == Dialect::Q3 {
            ascii_fold(name)
        } else {
            name.to_string()
        }
    }

    fn alias_command(inv: &mut Invocation) {
        let Some(name) = inv.argv.get(1).cloned() else {
            inv.print("Current alias commands:\n");
            for alias in inv.buffer.aliases.clone() {
                inv.print(&format!("{} : {}\n", alias.name, alias.value));
            }
            return;
        };
        let rest = &inv.argv.get(2..).unwrap_or(&[]);
        let gap = if inv.dialect.is_q1() && !rest.is_empty() {
            " "
        } else {
            ""
        };
        let joined = rest.join(" ");
        let text = format!("{joined}{gap}\n");
        if EngineText::from(text.as_str()).len() >= 1024 {
            inv.print("Alias body overflows source cmd[1024]\n");
            return;
        }
        let _ = inv.buffer.define_alias(&name, &text);
    }

    fn stuffcmds_command(inv: &mut Invocation) {
        if inv.dialect == Dialect::Q1Netquake && inv.argv.len() != 1 {
            inv.print("stuffcmds : execute command line parameters\n");
            return;
        }
        if let Some(startup) = inv.buffer.startup_command_text.clone() {
            let _ = inv.insert(&startup);
            return;
        }
        let text = inv
            .buffer
            .command_line
            .iter()
            .skip(1)
            .cloned()
            .collect::<Vec<_>>()
            .join(" ");
        let chars: Vec<char> = text.chars().collect();
        let mut script = String::new();
        let mut offset = 0;
        while offset < chars.len() {
            if chars[offset] != '+' {
                offset += 1;
                continue;
            }
            offset += 1;
            let start = offset;
            while offset < chars.len() && chars[offset] != '+' && chars[offset] != '-' {
                offset += 1;
            }
            script.push_str(&chars[start..offset].iter().collect::<String>());
            script.push('\n');
        }
        if !script.is_empty() {
            let _ = inv.insert(&script);
        }
    }

    fn cmdlist_command(inv: &mut Invocation) {
        let pattern = if inv.dialect == Dialect::Q3 {
            inv.argv.get(1).cloned()
        } else {
            None
        };
        let names: Vec<String> = inv
            .buffer
            .registered_names()
            .into_iter()
            .filter(|name| {
                pattern
                    .as_ref()
                    .is_none_or(|pattern| source_filter(pattern, name, false).unwrap_or(false))
            })
            .collect();
        for name in &names {
            inv.print(&format!("{name}\n"));
        }
        inv.print(&format!("{} commands\n", names.len()));
    }

    fn cvarlist_command(inv: &mut Invocation) {
        let pattern = if inv.dialect == Dialect::Q3 {
            inv.argv.get(1).cloned()
        } else {
            None
        };
        let variables = inv.cvars().snapshots(0);
        let mut shown = 0;
        for variable in &variables {
            if pattern
                .as_ref()
                .is_some_and(|pattern| !source_filter(pattern, &variable.name, false).unwrap_or(false))
            {
                continue;
            }
            shown += 1;
            let marker = |mask: u32, letter: &'static str| -> &'static str {
                if variable.flags & mask != 0 {
                    letter
                } else {
                    " "
                }
            };
            let markers = if inv.dialect == Dialect::Q3 {
                format!(
                    "{}{}{}{}{}{}{}",
                    marker(4, "S"),
                    marker(2, "U"),
                    marker(64, "R"),
                    marker(16, "I"),
                    marker(1, "A"),
                    marker(32, "L"),
                    marker(512, "C")
                )
            } else {
                format!(
                    "{}{}{}{}",
                    marker(1, "*"),
                    marker(2, "U"),
                    marker(4, "S"),
                    if variable.flags & 8 != 0 { "-" } else { marker(16, "L") }
                )
            };
            inv.print(&format!("{markers} {} \"{}\"\n", variable.name, variable.value));
        }
        if inv.dialect == Dialect::Q3 {
            inv.print(&format!("\n{shown} total cvars\n{} cvar indexes\n", variables.len()));
        } else {
            inv.print(&format!("{shown} cvars\n"));
        }
    }

    fn adjust_command(inv: &mut Invocation, name: &str) {
        let Some(variable_name) = inv.argv.get(1).cloned() else {
            inv.print(&format!("Usage: {name} <variable> [value]\n"));
            return;
        };
        let Some(variable) = inv.cvars().get(&variable_name) else {
            inv.print(&format!("{variable_name} is not a variable\n"));
            return;
        };
        if !is_decimal_text(&variable.value) {
            inv.print(&format!(
                "\"{}\" is \"{}\", can't {name}\n",
                variable.name, variable.value
            ));
            return;
        }
        let amount = inv.argv.get(2).map_or(1.0, |text| native_atof(text) as f32);
        let value = variable.numeric_value + if name == "dec" { -amount } else { amount };
        let text = if value == variable.numeric_value {
            variable.value.clone()
        } else if !value.is_finite() {
            if value.is_nan() {
                "nan".to_string()
            } else if value < 0.0 {
                "-inf".to_string()
            } else {
                "inf".to_string()
            }
        } else if value - value.floor() < 1e-6 {
            format!("{}", value.round())
        } else {
            cvar_value_text(f64::from(value), false).unwrap_or_else(|_| format!("{value:.6}"))
        };
        let truncated: String = text.chars().take(31).collect();
        let _ = inv.cvars().set_console(&variable.name, &truncated);
    }

    fn toggle_command(inv: &mut Invocation) {
        let Some(name) = inv.argv.get(1).cloned() else {
            inv.print("Usage: toggle <variable> [values]\n");
            return;
        };
        if inv.dialect == Dialect::Q3 && inv.argv.len() == 2 {
            let current = inv.cvars().variable_value(&name);
            let _ = inv
                .cvars()
                .set(&name, if current.trunc() == 0.0 { "1" } else { "0" }, false);
            return;
        }
        let Some(variable) = inv.cvars().get(&name) else {
            inv.print(&format!("{name} is not a variable\n"));
            return;
        };
        let values: Vec<String> = inv.argv.iter().skip(2).cloned().collect();
        if values.is_empty() {
            if variable.value == "0" || variable.value == "1" {
                let _ = inv
                    .cvars()
                    .set_console(&name, if variable.value == "0" { "1" } else { "0" });
            } else {
                inv.print(&format!("\"{name}\" is \"{}\", can't toggle\n", variable.value));
            }
            return;
        }
        let index = values
            .iter()
            .position(|value| ascii_fold(value) == ascii_fold(&variable.value));
        let next = index.and_then(|index| values.get((index + 1) % values.len()));
        match next {
            Some(next) => {
                let _ = inv.cvars().set_console(&name, next);
            }
            None => {
                inv.print(&format!("\"{name}\" is \"{}\", can't cycle\n", variable.value));
            }
        }
    }

    fn set_command(inv: &mut Invocation, command_flags: u32) {
        if inv.argv.len() < 3 || inv.dialect.is_q2() && inv.argv.len() > 4 || command_flags != 0 && inv.argv.len() != 3
        {
            inv.print("set <variable> <value>\n");
            return;
        }
        let name = inv.argv.get(1).cloned().unwrap_or_default();
        let value = inv.argv.get(2).cloned().unwrap_or_default();
        if inv.dialect.is_q2() && inv.argv.len() == 4 {
            let requested = inv.argv.get(3).cloned().unwrap_or_default();
            if requested != "u" && requested != "s" {
                inv.print("flags can only be 'u' or 's'\n");
                return;
            }
            let _ = inv.cvars().full_set(
                &name,
                &value,
                if requested == "u" {
                    q2_flags::USER_INFO
                } else {
                    q2_flags::SERVER_INFO
                },
            );
            return;
        }
        let mut combined = value;
        if inv.dialect == Dialect::Q3 {
            combined = String::new();
            let mut length = 0usize;
            let total = inv.argv.len();
            for (offset, argument) in inv.argv.iter().enumerate().skip(2) {
                let source_length = EngineText::from(argument.as_str()).len().saturating_sub(1);
                if length + source_length >= 1022 {
                    break;
                }
                combined.push_str(argument);
                if offset + 1 != total {
                    combined.push(' ');
                }
                if EngineText::from(combined.as_str()).len() >= 1024 {
                    inv.print("Cvar_Set overflows source combined buffer\n");
                    return;
                }
                length += source_length;
            }
        }
        if inv.cvars().set(&name, &combined, false).is_ok() {
            let _ = inv.cvars().add_flags(&name, command_flags);
        }
    }
}

fn is_decimal_text(value: &str) -> bool {
    let text = value.strip_prefix('-').unwrap_or(value);
    if text.is_empty() {
        return false;
    }
    // Donor `/^-?(?:[0-9]+(?:\.[0-9]*)?|\.[0-9]*)$/`: digits with an
    // optional point, or a bare point run.
    let mut digits = 0;
    let mut points = 0;
    for byte in text.bytes() {
        if byte.is_ascii_digit() {
            digits += 1;
        } else if byte == b'.' {
            points += 1;
        } else {
            return false;
        }
    }
    if points > 1 {
        return false;
    }
    digits > 0 || points > 0
}
