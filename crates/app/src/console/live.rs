//! Live session console: one per-dialect command buffer plus one cvar
//! registry, bound to the running server.
//!
//! Donor provenance: `src/app/bootstrap/application.ts` (console assembly),
//! `src/app/bootstrap/configuration.ts` (command/cvar registration), and
//! `src/core/commands/index.ts` (buffer) with `src/core/cvars/index.ts`
//! (registry). Behavior follows the originals (`quake/*/cmd.c`,
//! `quake-2/qcommon/cmd.c`, `quake-iii-arena/code/qcommon/cmd.c` and the
//! matching `cvar.c` files), not the donor's shapes.
//!
//! [`LiveConsole`] owns the whole console surface of one session: the core
//! [`CommandBuffer`](qa_core::cmd_buffer::CommandBuffer) with the dialect
//! builtins, the app [`ConsoleCommands`], the owner
//! [`CvarRegistry`](qa_core::cvar::CvarRegistry), per-seat userinfo
//! registries, binding tables, button-event queues, audio state, and the
//! [`LiveServerBridge`] that carries console work to the server and back.
//! Nothing here is global: every session opens its own console, and closing
//! the session drops it.
//!
//! Command handlers run inside the buffer drain, where they cannot borrow
//! the server. Handlers that need server state read the pre-drive
//! [`LiveServerSnapshot`] and push [`LiveServerOp`]s; the host applies them
//! to the live [`Server`](qa_world::server::Server) after the drive and
//! appends the result lines to the console log. Both run paths (headless
//! [`Application`](crate::application::Application) and the windowed
//! backend) drive the same console the same way.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use qa_client::audio::output::AudioOutputFormat;
use qa_client::input::commands::{
    CommandHandler as ClientCommandHandler, CommandInvocation as ClientInvocation, CommandOrigin as ClientOrigin,
    CommandRegistry as ClientCommandRegistry,
};
use qa_client::input::mouse_settings::register_mouse_settings;
use qa_client::render::debug_graph::register_debug_graph_cvars;
use qa_client::ui::settings::accessibility::register_accessibility_settings;
use qa_client::ui::settings::language::register_language_settings;
use qa_content::q3::base::shared::definitions::Product;
use qa_content::q3::presentation::config::cvar_table;
use qa_core::cmd::Dialect;
use qa_core::cmd_buffer::{CommandBuffer, CommandContext, CommandOrigin, Invocation};
use qa_core::cvar::{CvarAlias, CvarAliasConversion, CvarDocumentation, CvarRegistry};
use qa_core::identity::{SeatId, SessionId};

use super::ConsoleError;
use crate::bootstrap::audio::output_settings::{s_khz_read, s_khz_write};
use crate::bootstrap::audio::playlist_settings::MUSIC_SETTING_ALIASES;
use crate::bootstrap::gtv_commands::register_gtv_cvars;
use crate::bootstrap::image_settings::{ApplicationImageSettings, RegistryAdapter};
use crate::bootstrap::network::client_download_policy::{create_client_download_permission, ClientDownloadFamily};
use crate::bootstrap::network::socks_settings::register_socks_cvars;
use crate::bootstrap::player_userinfo::register_player_userinfo;
use crate::bootstrap::q1_client_settings::{register_q1_client_settings, Q1ClientSettingsProfile};
use crate::bootstrap::q1_source_cvars::register_q1_bot_controls;
use crate::bootstrap::q3_client::userinfo::{initialize_q3_client_cvars, Q3ClientIdentity};
use crate::bootstrap::remote_application::seed_seat_cvars;
use crate::bootstrap::shared_setting_cvars::{register_run_cvar, register_shared_client_settings};
use crate::bootstrap::startup_source::{register_startup_source_cvars, StartupSourceSelection};
use crate::options::{ApplicationOptions, GameMode, Network};

/// One server-side player row for status tables.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LivePlayerRow {
    /// Registry slot.
    pub slot: u32,
    /// Player name.
    pub name: String,
    /// Score.
    pub score: i32,
    /// Ping in milliseconds.
    pub ping: i32,
    /// Spectator flag.
    pub spectator: bool,
    /// Local seat index driving this player, if any.
    pub seat: Option<usize>,
}

/// Server state sampled before a console drive, so handlers print live
/// values without borrowing the server.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LiveServerSnapshot {
    /// Live actor count.
    pub actors: usize,
    /// Server frame number.
    pub frame: u64,
    /// Server clock in seconds.
    pub time_seconds: f64,
    /// Current map resource path.
    pub map: String,
    /// Whether the server is paused.
    pub paused: bool,
    /// Seat index to registry slot for bound seats.
    pub seat_slots: Vec<Option<u32>>,
    /// Player rows in slot order.
    pub players: Vec<LivePlayerRow>,
    /// Chat/say lines already on the server.
    pub chat: Vec<String>,
}

/// One unit of console work for the live server, applied after the drive.
#[derive(Debug, Clone, PartialEq)]
pub enum LiveServerOp {
    /// Server `say`/`say_team` line.
    Say {
        /// Sending seat, if any.
        seat: Option<usize>,
        /// Team-only flag.
        team: bool,
        /// Sender name.
        from: String,
        /// Chat text.
        text: String,
    },
    /// Suicide after an optional delay in seconds (Q2 `kill` waits 5).
    Kill {
        /// Registry slot.
        slot: u32,
        /// Delay before the suicide lands.
        delay_seconds: f64,
    },
    /// Toggle god mode on one actor.
    God {
        /// Registry slot.
        slot: u32,
    },
    /// Give items to one actor (`give`).
    Give {
        /// Registry slot.
        slot: u32,
        /// Item selector (`all`, `health`, ...).
        item: String,
        /// Amount.
        count: f64,
    },
    /// One forwarded line for the server command surface (`cmd`,
    /// unknown Q2/Q3 commands).
    Forwarded {
        /// Raw line.
        line: String,
    },
}

/// Shared bridge between console handlers and the live server.
///
/// Handlers clone the [`LiveServerBridge`] handle: they read the snapshot
/// and push ops. The host refreshes the snapshot before each drive and
/// drains the ops after it.
#[derive(Debug, Clone, Default)]
pub struct LiveServerBridge {
    shared: Rc<RefCell<LiveServerBridgeState>>,
}

/// Bridge state behind the shared handle.
#[derive(Debug, Default)]
struct LiveServerBridgeState {
    snapshot: LiveServerSnapshot,
    ops: Vec<LiveServerOp>,
}

impl LiveServerBridge {
    /// Fresh bridge with an empty snapshot and no ops.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the snapshot (host, before the drive).
    pub fn set_snapshot(&self, snapshot: LiveServerSnapshot) {
        self.shared.borrow_mut().snapshot = snapshot;
    }

    /// Read the snapshot (handlers, during the drive).
    #[must_use]
    pub fn snapshot(&self) -> LiveServerSnapshot {
        self.shared.borrow().snapshot.clone()
    }

    /// Push one op (handlers, during the drive).
    pub fn push(&self, op: LiveServerOp) {
        self.shared.borrow_mut().ops.push(op);
    }

    /// Drain queued ops (host, after the drive).
    #[must_use]
    pub fn take_ops(&self) -> Vec<LiveServerOp> {
        std::mem::take(&mut self.shared.borrow_mut().ops)
    }
}

/// Per-seat userinfo registries plus the shared owner.
///
/// Shared settings live in the owner registry (the one the command buffer
/// dispatches against). Each seat keeps its own userinfo registry holding
/// that seat's player identity (`name`, `skin`, ...), registered with the
/// seat index. Userinfo commands resolve the origin seat and address that
/// seat's registry; seat 0 shares the owner's userinfo rows.
pub struct SeatUserinfo {
    seats: Vec<CvarRegistry>,
}

impl SeatUserinfo {
    /// Open per-seat userinfo registries for `count` seats.
    pub fn open(dialect: Dialect, session: &SessionId, count: usize) -> Result<Self, ConsoleError> {
        let mut seats = Vec::with_capacity(count);
        for _ in 0..count {
            let mut registry = CvarRegistry::new(dialect);
            registry.set_session(session.clone());
            seats.push(registry);
        }
        Ok(Self { seats })
    }

    /// Number of seat registries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.seats.len()
    }

    /// Whether there are no seat registries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.seats.is_empty()
    }

    /// Borrow one seat registry.
    #[must_use]
    pub fn get(&self, seat: usize) -> Option<&CvarRegistry> {
        self.seats.get(seat)
    }

    /// Borrow one seat registry mutably.
    pub fn get_mut(&mut self, seat: usize) -> Option<&mut CvarRegistry> {
        self.seats.get_mut(seat)
    }
}

/// Script text provider for `exec`: preloaded text first, then the user
/// directory, then content mounts. Reads are synchronous, like
/// `COM_LoadHunkFile`.
#[derive(Debug, Default)]
pub struct LiveScriptProvider {
    preloaded: HashMap<String, Option<String>>,
    failed: HashMap<String, String>,
    user_dir: Option<std::path::PathBuf>,
    mounts: Option<Rc<qa_content::mounts::MountedContent>>,
}

impl LiveScriptProvider {
    /// Empty provider.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Preload script text (`None` means missing).
    pub fn set_script(&mut self, name: &str, text: Option<String>) {
        self.failed.remove(name);
        self.preloaded.insert(name.to_string(), text);
    }

    /// Preload a script read failure.
    pub fn fail_script(&mut self, name: &str, error: &str) {
        self.preloaded.remove(name);
        self.failed.insert(name.to_string(), error.to_string());
    }

    /// User directory for config/script reads and writes.
    pub fn set_user_dir(&mut self, dir: Option<std::path::PathBuf>) {
        self.user_dir = dir;
    }

    /// Content mounts for stock script reads.
    pub fn set_mounts(&mut self, mounts: Option<Rc<qa_content::mounts::MountedContent>>) {
        self.mounts = mounts;
    }

    /// Read one script: preloaded text, the user directory, then mounts.
    /// Remote-client origins never read local scripts.
    pub fn read(&self, name: &str, source: &CommandContext) -> Result<Option<String>, String> {
        if is_remote_client(&source.origin) {
            return Ok(None);
        }
        if let Some(error) = self.failed.get(name) {
            return Err(error.clone());
        }
        if let Some(text) = self.preloaded.get(name) {
            return Ok(text.clone());
        }
        if name.contains("..") {
            return Ok(None);
        }
        if let Some(dir) = self.user_dir.as_ref() {
            let path = dir.join(name);
            if let Ok(bytes) = std::fs::read(&path) {
                return Ok(Some(String::from_utf8_lossy(&bytes).into_owned()));
            }
        }
        if let Some(mounts) = self.mounts.as_ref() {
            if let Ok(Some(opened)) = mounts.open(name, |_| true) {
                return Ok(Some(String::from_utf8_lossy(&opened.bytes).into_owned()));
            }
        }
        Ok(None)
    }
}

fn is_remote_client(origin: &CommandOrigin) -> bool {
    let mut current = origin;
    while let CommandOrigin::Script { caller, .. } = current {
        current = caller;
    }
    matches!(current, CommandOrigin::RemoteClient { .. })
}

/// One `+`/`-` button edge or impulse write from a console command.
///
/// Handlers record events; the host merges them into the seat input state
/// before sampling, so console-typed buttons land the same frame.
#[derive(Debug, Clone, PartialEq)]
pub enum LiveButtonEvent {
    /// Press or release a named button from a key.
    Button {
        /// Seat id.
        seat: SeatId,
        /// Button action.
        action: qa_client::input::SourceAction,
        /// Pressing key.
        key: String,
        /// Press (`true`) or release.
        down: bool,
        /// Event time in milliseconds.
        time_ms: i64,
    },
    /// Release a named button from every key.
    Release {
        /// Seat id.
        seat: SeatId,
        /// Button action.
        action: qa_client::input::SourceAction,
        /// Event time in milliseconds.
        time_ms: i64,
    },
    /// Set the pending impulse byte.
    Impulse {
        /// Seat id.
        seat: SeatId,
        /// Impulse value.
        value: u8,
    },
    /// Wheel-mode edge (`+weaponwheel` and friends).
    Wheel {
        /// Seat id.
        seat: SeatId,
        /// Wheel mode.
        mode: qa_client::input::bindings::WheelMode,
        /// Press (`true`) or release.
        down: bool,
    },
}

/// Shared button-event queue behind the console button commands.
#[derive(Debug, Clone, Default)]
pub struct LiveButtonQueue {
    shared: Rc<RefCell<Vec<LiveButtonEvent>>>,
}

impl LiveButtonQueue {
    /// Empty queue.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one event.
    pub fn push(&self, event: LiveButtonEvent) {
        self.shared.borrow_mut().push(event);
    }

    /// Drain queued events.
    #[must_use]
    pub fn take(&self) -> Vec<LiveButtonEvent> {
        std::mem::take(&mut self.shared.borrow_mut())
    }
}

fn convert_origin(origin: &CommandOrigin) -> ClientOrigin {
    match origin {
        CommandOrigin::LocalConsole => ClientOrigin::LocalConsole,
        CommandOrigin::ServerConsole => ClientOrigin::ServerConsole,
        CommandOrigin::LocalSeat { seat, .. } => ClientOrigin::LocalSeat(seat.clone()),
        CommandOrigin::RemoteClient { .. } => ClientOrigin::Other,
        CommandOrigin::Script { caller, .. } => ClientOrigin::Script {
            caller: Box::new(convert_origin(caller)),
        },
    }
}

/// Adapts a core [`CommandBuffer`] to the client [`ClientCommandRegistry`]
/// surface, so input/binding commands register on the live program.
/// Handler errors print through the invocation printer.
pub struct LiveClientRegistry<'a> {
    buffer: &'a mut CommandBuffer,
    cvars: &'a CvarRegistry,
    contexts: &'a HashMap<SeatId, CommandContext>,
    print: Rc<dyn Fn(&str)>,
}

impl<'a> LiveClientRegistry<'a> {
    /// Adapter over a live buffer and registry.
    #[must_use]
    pub fn new(
        buffer: &'a mut CommandBuffer,
        cvars: &'a CvarRegistry,
        contexts: &'a HashMap<SeatId, CommandContext>,
        print: Rc<dyn Fn(&str)>,
    ) -> Self {
        Self {
            buffer,
            cvars,
            contexts,
            print,
        }
    }

    fn register_inner(&mut self, name: &str, handler: ClientCommandHandler) -> bool {
        let handler = Rc::new(RefCell::new(handler));
        let wrapped = Rc::new(move |invocation: &mut Invocation| {
            let call = ClientInvocation::new(
                invocation.argv.clone(),
                convert_origin(&invocation.source.origin),
                invocation.dialect,
            );
            if let Err(error) = (*handler.borrow_mut())(&call) {
                invocation.print(&format!("{error}\n"));
            }
        });
        self.buffer
            .register(name, Some(wrapped), None, self.cvars)
            .unwrap_or(false)
    }
}

impl ClientCommandRegistry for LiveClientRegistry<'_> {
    fn register_engine(&mut self, name: &str, handler: ClientCommandHandler) -> bool {
        self.register_inner(name, handler)
    }

    fn register(&mut self, name: &str, handler: ClientCommandHandler) -> bool {
        self.register_inner(name, handler)
    }

    fn unregister(&mut self, name: &str) {
        self.buffer.unregister(name);
    }

    fn exists(&self, name: &str) -> bool {
        self.buffer.exists(name)
    }

    fn append(&mut self, text: &str, seat: &SeatId) {
        let Some(context) = self.contexts.get(seat).cloned() else {
            (self.print)("Client commands require a local seat\n");
            return;
        };
        if let Err(error) = self.buffer.append(text, Some(&context), None) {
            (self.print)(&format!("{error}\n"));
        }
    }
}

/// Parameters for the live owner-registry assembly.
#[derive(Debug, Clone)]
pub struct LiveCvarParams {
    /// Stock skill level.
    pub skill: u8,
    /// Game mode.
    pub mode: GameMode,
    /// Map resource path.
    pub map: String,
    /// Catalog product id (mission-pack and match detection reads this).
    pub product: String,
    /// Dedicated server (no seats, no autosave cvar).
    pub dedicated: bool,
    /// Selected network mode.
    pub network: Network,
    /// Client capacity.
    pub max_clients: u32,
    /// Display gamma for `r_gamma`.
    pub gamma: f64,
    /// Shared audio output format.
    pub output_format: AudioOutputFormat,
    /// Default character model for userinfo skins.
    pub model: String,
}

/// Derive the Q2 match provider from a catalog product id.
fn match_provider_for(product: &str) -> &'static str {
    if product.contains("lmctf") {
        "q2:lmctf"
    } else if product.contains("-ctf") {
        "q2:ctf"
    } else {
        "q2:official"
    }
}

/// Derive the Q3 content product from a catalog product id.
fn q3_product_for(product: &str) -> Product {
    if product.contains("missionpack") {
        Product::Missionpack
    } else {
        Product::Baseq3
    }
}

/// Register the full live cvar surface into one owner registry: source
/// cvars, the Q3 cgame table, shared client settings, input, image, audio,
/// download policy, and the alias table. Every per-module registration
/// keeps its home; this is the one caller that runs them all against the
/// session registry. Registration is idempotent: options-driven rows
/// re-apply launch configuration, everything else keeps player-set
/// values, so calling it again after a map change is safe.
pub fn register_live_cvars(cvars: &mut CvarRegistry, params: &LiveCvarParams) -> Result<(), ConsoleError> {
    let dialect = cvars.dialect();
    let options = ApplicationOptions {
        skill: params.skill,
        mode: params.mode,
        map: params.map.clone(),
        product: params.product.clone(),
        dedicated: params.dedicated,
        network: params.network.clone(),
        ..ApplicationOptions::default()
    };
    let selection = StartupSourceSelection {
        source_content: params.product.clone(),
        match_provider: match_provider_for(&params.product).to_string(),
    };
    register_startup_source_cvars(cvars, &options, &selection, params.max_clients, None)
        .map_err(|error| ConsoleError::Cvar(error.to_string()))?;
    if dialect == Dialect::Q3 {
        for definition in cvar_table(q3_product_for(&params.product)) {
            cvars.register(&definition.name, &definition.default_value, definition.flags)?;
        }
    }
    register_shared_client_settings(cvars, params.output_format)?;
    register_run_cvar(cvars, dialect)?;
    register_mouse_settings(cvars)?;
    register_q1_client_settings(cvars, Q1ClientSettingsProfile::Dialect(dialect))?;
    register_q1_bot_controls(cvars)?;
    ApplicationImageSettings::register_owned(cvars, params.gamma)
        .map_err(|error| ConsoleError::Cvar(error.to_string()))?;
    {
        let mut adapter = RegistryAdapter::new(&mut *cvars);
        register_accessibility_settings(&mut adapter);
        register_language_settings(&mut adapter);
    }
    register_gtv_cvars(cvars).map_err(|error| ConsoleError::Cvar(error.to_string()))?;
    register_socks_cvars(cvars)?;
    register_debug_graph_cvars(cvars)?;
    let family = if dialect == Dialect::Q3 {
        Some(ClientDownloadFamily::Q3)
    } else if dialect.is_q2() {
        Some(ClientDownloadFamily::Q2)
    } else {
        None
    };
    if let Some(family) = family {
        create_client_download_permission(cvars, family)?;
    }
    register_live_aliases(cvars)?;
    Ok(())
}

/// Register one alias unless the name is already declared, keeping
/// live registration idempotent.
fn declare_alias(cvars: &mut CvarRegistry, alias: CvarAlias) -> Result<(), ConsoleError> {
    if cvars.get(&alias.name).is_some() {
        return Ok(());
    }
    cvars.register_alias(alias)?;
    Ok(())
}

/// Register one identity alias.
fn identity_alias(cvars: &mut CvarRegistry, name: &str, target: &str, summary: &str) -> Result<(), ConsoleError> {
    declare_alias(
        cvars,
        CvarAlias {
            name: name.to_string(),
            target: target.to_string(),
            documentation: CvarDocumentation {
                summary: summary.to_string(),
                ..CvarDocumentation::default()
            },
            conversion: CvarAliasConversion::Identity,
        },
    )
}

/// Register the live alias table: identity aliases for the audio names
/// plus the converted `s_khz` (kHz selector over `s_outputRate`) and the
/// reciprocal `gamma`/`vid_gamma` brightness names over `r_gamma`.
fn register_live_aliases(cvars: &mut CvarRegistry) -> Result<(), ConsoleError> {
    identity_alias(cvars, "s_volume", "volume", "Alias of volume.")?;
    identity_alias(cvars, "s_musicvolume", "bgmvolume", "Alias of bgmvolume.")?;
    identity_alias(cvars, "ogg_volume", "bgmvolume", "Alias of bgmvolume.")?;
    for (alias, target) in MUSIC_SETTING_ALIASES {
        identity_alias(cvars, alias, target, "Alias of the music setting.")?;
    }
    declare_alias(
        cvars,
        CvarAlias {
            name: "s_khz".to_string(),
            target: "s_outputRate".to_string(),
            documentation: CvarDocumentation {
                summary: "Source sample-rate convention for the shared output.".to_string(),
                usage: "s_khz <11|22|44|48>".to_string(),
                ..CvarDocumentation::default()
            },
            conversion: CvarAliasConversion::Converted {
                read: Rc::new(s_khz_read),
                write: Rc::new(|value| s_khz_write(value).map_err(|message| message.to_string())),
            },
        },
    )?;
    for name in ["gamma", "vid_gamma"] {
        declare_alias(
            cvars,
            CvarAlias {
                name: name.to_string(),
                target: "r_gamma".to_string(),
                documentation: CvarDocumentation {
                    summary: "Quake brightness convention: lower values brighten.".to_string(),
                    ..CvarDocumentation::default()
                },
                conversion: CvarAliasConversion::Converted {
                    read: Rc::new(|value| reciprocal_text(value, value)),
                    write: Rc::new(|value| Ok(reciprocal_text(value, "1"))),
                },
            },
        )?;
    }
    Ok(())
}

/// Reciprocal of cvar text, falling back to `fallback` when the text is
/// not a finite nonzero number.
fn reciprocal_text(value: &str, fallback: &str) -> String {
    match value.trim().parse::<f64>() {
        Ok(parsed) if parsed.is_finite() && parsed != 0.0 => format!("{}", 1.0 / parsed),
        _ => fallback.to_string(),
    }
}

/// Open per-seat userinfo registries: the seeded base rows plus the
/// identity extensions, or the Q3 client initializer on Q3.
pub fn open_seat_userinfo(
    dialect: Dialect,
    session: &SessionId,
    seats: usize,
    model: &str,
) -> Result<SeatUserinfo, ConsoleError> {
    let mut info = SeatUserinfo::open(dialect, session, seats)?;
    for index in 0..seats {
        let mut registry = seed_seat_cvars(dialect, index as u32, model, false)?;
        registry.set_session(session.clone());
        if dialect == Dialect::Q3 {
            let name = if index == 0 {
                "Player".to_string()
            } else {
                format!("Player {}", index + 1)
            };
            initialize_q3_client_cvars(
                &mut registry,
                &Q3ClientIdentity {
                    name,
                    model: model.to_string(),
                },
            )
            .map_err(|error| ConsoleError::Cvar(error.to_string()))?;
        } else {
            register_player_userinfo(&mut registry, index as u32, model)?;
        }
        info.seats[index] = registry;
    }
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_client::audio::output::DEFAULT_AUDIO_OUTPUT_FORMAT;
    use qa_core::cvar::flags;
    use qa_core::identity::IdentityOwner;

    fn params(product: &str) -> LiveCvarParams {
        LiveCvarParams {
            skill: 1,
            mode: GameMode::Singleplayer,
            map: "start".to_string(),
            product: product.to_string(),
            dedicated: false,
            network: Network::Offline,
            max_clients: 8,
            gamma: 1.0,
            output_format: DEFAULT_AUDIO_OUTPUT_FORMAT,
            model: "male".to_string(),
        }
    }

    fn assembled(dialect: Dialect, product: &str) -> CvarRegistry {
        let mut cvars = CvarRegistry::new(dialect);
        register_live_cvars(&mut cvars, &params(product)).unwrap();
        cvars
    }

    #[test]
    fn q1_owner_carries_source_and_shared_rows() {
        let cvars = assembled(Dialect::Q1Netquake, "q1-classic-id1");
        for (name, value) in [
            ("skill", "1"),
            ("deathmatch", "0"),
            ("coop", "0"),
            ("teamplay", "0"),
            ("sv_cheats", "0"),
            ("sv_aim", "0.93"),
            ("pausable", "1"),
            ("footsteps", "1"),
            ("host_framerate", "0"),
            ("viewsize", "100"),
            ("volume", "0.7"),
            ("fov", "90"),
            ("r_gamma", "1"),
            ("sensitivity", "3"),
            ("bot_minplayers", "0"),
        ] {
            assert_eq!(cvars.variable_string(name), value, "{name}");
        }
        assert!(cvars.get("timescale").is_none());
        assert!(cvars.get("g_gametype").is_none());
        assert!(cvars.get("ui_seat1_captions").is_some());
    }

    #[test]
    fn qw_owner_carries_rate_warncmd_and_maxfps() {
        let cvars = assembled(Dialect::Q1Quakeworld, "q1-quakeworld");
        assert_eq!(cvars.variable_string("sv_aim"), "2");
        assert_eq!(cvars.variable_string("cl_maxfps"), "0");
        assert_eq!(cvars.variable_string("cl_warncmd"), "0");
        assert_eq!(cvars.variable_string("rate"), "2500");
        assert_eq!(cvars.variable_string("maxspectators"), "8");
        let maxclients = cvars.get("maxclients").expect("maxclients");
        assert_eq!(maxclients.flags & flags::SERVER_INFO, flags::SERVER_INFO);
    }

    #[test]
    fn q2_owner_carries_server_graphs_and_latched_capacity() {
        let cvars = assembled(Dialect::Q2Classic, "q2-classic-baseq2");
        for name in [
            "cheats",
            "developer",
            "hostname",
            "dmflags",
            "needpass",
            "flood_msgs",
            "bob_up",
        ] {
            assert!(cvars.get(name).is_some(), "{name}");
        }
        assert_eq!(cvars.variable_string("hostname"), "noname");
        assert_eq!(cvars.variable_string("graphheight"), "32");
        assert_eq!(cvars.variable_string("netgraph"), "0");
        assert!(cvars.get("timescale").is_some());
        assert!(cvars.get("mvd_username").is_some());
        assert_eq!(cvars.variable_string("allow_download"), "1");
        assert!(cvars.get("ctfflags").is_none());
        let rerelease = assembled(Dialect::Q2Rerelease, "q2-rerelease-baseq2");
        assert!(rerelease.get("g_coop_enable_lives").is_some());
        let lmctf = assembled(Dialect::Q2Classic, "q2-classic-lmctf");
        assert!(lmctf.get("ctfflags").is_some());
        assert!(lmctf.get("maplist_file").is_some());
        let ctf = assembled(Dialect::Q2Classic, "q2-classic-ctf");
        assert!(ctf.get("capturelimit").is_some());
    }

    #[test]
    fn q3_owner_carries_engine_game_and_cgame_rows() {
        let cvars = assembled(Dialect::Q3, "q3-baseq3");
        let cheats = cvars.get("sv_cheats").expect("sv_cheats");
        assert_eq!(cheats.value, "1");
        assert_eq!(
            cheats.flags & (flags::SYSTEM_INFO | flags::READ_ONLY),
            flags::SYSTEM_INFO | flags::READ_ONLY
        );
        assert_eq!(cvars.variable_string("sv_hostname"), "noname");
        assert_eq!(cvars.variable_string("bot_pause"), "0");
        assert_eq!(
            cvars.get("bot_pause").expect("bot_pause").flags & flags::CHEAT,
            flags::CHEAT
        );
        assert_eq!(cvars.variable_string("bot_rocketjump"), "1");
        assert_eq!(cvars.variable_string("bot_interbreedbots"), "10");
        assert_eq!(cvars.variable_string("bot_predictobstacles"), "1");
        assert_eq!(cvars.variable_string("g_spSkill"), "2");
        assert_eq!(cvars.variable_string("cg_fov"), "90");
        assert_eq!(cvars.variable_string("fs_game"), "");
        assert_eq!(cvars.variable_string("com_blood"), "1");
        assert_eq!(cvars.variable_string("net_socksPort"), "1080");
        assert_eq!(cvars.variable_string("cl_allowDownload"), "0");
        assert!(cvars.get("skill").is_none());
        assert!(cvars.get("deathmatch").is_none());
        assert!(cvars.get("coop").is_none());
        let mission = assembled(Dialect::Q3, "q3-missionpack");
        assert!(mission.get("cg_fov").is_some());
    }

    #[test]
    fn aliases_write_through_and_convert() {
        let mut cvars = assembled(Dialect::Q2Classic, "q2-classic-baseq2");
        cvars.set("s_volume", "0.5", true).unwrap();
        assert_eq!(cvars.variable_string("volume"), "0.5");
        cvars.set("volume", "0.7", true).unwrap();
        assert_eq!(cvars.variable_string("s_volume"), "0.7");
        cvars.set("s_khz", "44", true).unwrap();
        assert_eq!(cvars.variable_string("s_outputRate"), "44100");
        cvars.set("s_outputRate", "11025", true).unwrap();
        assert_eq!(cvars.variable_string("s_khz"), "11");
        cvars.set("r_gamma", "2", true).unwrap();
        assert_eq!(cvars.variable_string("gamma"), "0.5");
        cvars.set("gamma", "0.5", true).unwrap();
        assert_eq!(cvars.variable_string("r_gamma"), "2");
    }

    #[test]
    fn registration_is_idempotent_and_keeps_values() {
        let mut cvars = assembled(Dialect::Q1Netquake, "q1-classic-id1");
        cvars.set("volume", "0.3", true).unwrap();
        cvars.set("skill", "3", true).unwrap();
        register_live_cvars(&mut cvars, &params("q1-classic-id1")).unwrap();
        assert_eq!(cvars.variable_string("volume"), "0.3");
        assert_eq!(cvars.variable_string("skill"), "1");
        assert_eq!(cvars.variable_string("deathmatch"), "0");
    }

    #[test]
    fn seats_carry_per_dialect_userinfo() {
        let owner = IdentityOwner::create("live-seats").unwrap();
        let session = owner.session().clone();
        let q1 = open_seat_userinfo(Dialect::Q1Netquake, &session, 2, "male").unwrap();
        assert_eq!(q1.len(), 2);
        assert_eq!(q1.get(0).unwrap().variable_string("name"), "Player");
        assert_eq!(q1.get(1).unwrap().variable_string("name"), "Player 2");
        assert!(q1.get(0).unwrap().get("_cl_name").is_some());
        assert!(q1.get(0).unwrap().get("qts_weapon_autoswitch").is_some());
        let q2 = open_seat_userinfo(Dialect::Q2Classic, &session, 1, "female").unwrap();
        assert_eq!(q2.get(0).unwrap().variable_string("gender"), "female");
        assert_eq!(q2.get(0).unwrap().variable_string("rate"), "25000");
        let q3 = open_seat_userinfo(Dialect::Q3, &session, 1, "sarge").unwrap();
        assert_eq!(q3.get(0).unwrap().variable_string("name"), "Player");
        assert_eq!(q3.get(0).unwrap().variable_string("model"), "sarge/default");
        assert_eq!(q3.get(0).unwrap().variable_string("rate"), "25000");
    }
}
