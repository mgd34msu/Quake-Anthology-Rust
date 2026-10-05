//! Port of Quake-Anthology-TS `src/app/bootstrap/remote-application.ts`
//! (`RemoteApplication`): native remote-client application orchestration.
//!
//! The donor is a 2498-line orchestrator over subsystems that live elsewhere.
//! This port keeps every donor decision, guard, message, sequencing rule, and
//! state machine (launch validation, recorded-option mapping, seat cvar
//! seeding, command dispatch, cinematic tickets, peer admission, advance
//! boundaries, download fan-out, step branching, close aggregation) and moves
//! every subsystem operation behind [`RemoteApplicationSystems`], which the
//! wiring lane implements. Async work resolves synchronously: advance
//! boundaries and peer-admission suspensions are explicit polled state, and
//! `AggregateError` sites become [`RemoteApplicationError::Aggregate`].
//!
//! Fold notes:
//! - `ApplicationOptions` arrives as [`RemoteApplicationOptions`] with the
//!   fields this file reads; the remaining launch recipe stays host-side.
//! - `ClientBootstrap` (borrowed ownership) is fully host-side; borrowed-only
//!   queries go through systems.
//! - `RemoteSeatPeer` keeps donor bookkeeping (map, generation, closed,
//!   disconnected, userinfo, view/content presence) plus an opaque systems
//!   handle; per-peer work goes through systems.
//! - `RemoteConfiguration` arrives as [`RemoteConfigurationSummary`]; the
//!   host builds it with [`seed_seat_cvars`], which ports the donor seat
//!   cvar seeding per dialect verbatim.
//! - `remoteContentProduct` needs the product catalog, so it is a systems
//!   query; `remoteContentSelection` normalization is ported.
//! - `liveQ2Protocol` defaulting is ported.
//! - Reentrant content switches surface as poll outcomes instead of callbacks.
//! - The `run` loop keeps the 4ms floor and video-generation reset; sleeping
//!   is a systems call.
//!
//! Canonical siblings (ported; the wiring lane holds the live instances):
//! `options.ts` is `crate::options::ApplicationOptions` (`app/options.rs`),
//! `input.ts` is `crate::bootstrap::input::ApplicationInput`, `keys.ts` is
//! `crate::bootstrap::keys::ApplicationKeys`, `audio.ts` is
//! `crate::bootstrap::audio::application`, `session/*` is
//! `qa_world::session::EngineSession`, `ConfigStore` is
//! `crate::settings::config::ConfigStore`, and `PlayerProgressStore` is
//! `crate::bootstrap::player_progress::PlayerProgressStore`.
//!
//! Host seams (donor subsystems owned by the wiring lane behind
//! [`RemoteApplicationSystems`]; each names its canonical Rust home):
//! - `content.ts` is `crate::bootstrap::content` (`load_application_content`,
//!   `open_remote_application_content`); reached via `finish_open`,
//!   `select_content_mounts`, `resolve_movement`, `open_content_q3_product`,
//!   `close_loaded_content`, and `close_content_bundle`.
//! - `configuration.ts` is `crate::bootstrap::configuration`
//!   (`legacy_configuration_options`); reached via `prepare_configuration`,
//!   `publish_configuration`, and `publish_peer_configuration`, with seat cvar
//!   seeding ported as [`seed_seat_cvars`].
//! - `client-bootstrap.ts` is `crate::bootstrap::client_bootstrap`
//!   (`ClientSourcePublicationError`); reached via `dispatch_borrowed`,
//!   `borrowed_source_is_current`, and `release_borrowed_source`, with the
//!   `open_borrowed`, `open_demo_borrowed`, and `open_gtv_borrowed` entries ported.
//! - `effects.ts` is `crate::bootstrap::effects::application`
//!   (`ApplicationEffects`); reached via `present_primary_frame` (which returns
//!   unhandled effects) and `record_level_completion`.
//! - `assets.ts` has no dedicated Rust home; the wiring lane owns frontend
//!   asset setup and teardown behind `finish_open`, `refresh_images`,
//!   `read_pixels`, and `capture_next_frame`.
//! - `menu-art.ts` is `crate::bootstrap::menu_art` (`load_menu_art_image`);
//!   menu art loads during `finish_open` frontend assembly.
//! - `frame-time.ts` is `crate::bootstrap::frame_time`
//!   (`source_frame_milliseconds`); reached via `source_frame_ms`, with the 4ms
//!   run-loop floor ported in [`RemoteApplication::run`].
//! - `frame-clock.ts` is `crate::bootstrap::frame_clock` (`PresentationTime`);
//!   reached via `advance_presentation_time` and `presentation_ms`.
//! - `presentation*.ts` is `crate::bootstrap::presentation` (`seat_viewport`,
//!   `WorldSeatPresentation`) plus `presentation_scene.rs`,
//!   `presentation_state.rs`, and `selected_q3_presentation.rs`; reached via
//!   `presentation_for_actor`, the bind/sample/present/submit/retire family,
//!   and `clear_presentation`.
//! - `media/*` is `crate::bootstrap::component_media`
//!   (`presentation_audio_control`) and `crate::bootstrap::media`; reached via
//!   the `movie_*` and signon-recording seams.
//! - `network/*` is `crate::bootstrap::network` (`transport.rs`,
//!   `unified_client.rs`, `gtv_source.rs`, `recorded_source.rs`, `q2_demo.rs`,
//!   and the per-family clients and presentations); reached via
//!   `resolve_address`, `execute_advance`, `submit_primary`, the peer
//!   open/poll/drain family, and the download progress/cancel/retry family.
//! - `simulation/*` crosses the seam as [`RemotePresentationEvent`] (the
//!   consumed `SimulationPresentationEvent` subset, also referenced by
//!   `network/types.rs`); reached via `present_primary_frame` and the
//!   sample-presentation seams.
//! - `q3-client/*` is `crate::bootstrap::q3_client_app`
//!   (`ApplicationQ3Client`) and `crate::bootstrap::q3_client`; reached via
//!   `q3_command`, the input-client and browser seams, and
//!   `shutdown_primary_q3`.
//! - `ui/*` is `crate::bootstrap::ui` (`ApplicationSeatUi`); reached via
//!   `use_item_ordinal`, `controls_input`, `player_command`, and `centerview`.
//! - Renderer natives are `crate::bootstrap::renderer` (`NativeRenderer`);
//!   reached via `flush_renderer`, `read_pixels`, `capture_next_frame`, and the
//!   video-restart seams; the loading frame (`present_loading_frame`) is a
//!   renderer draw/swap plus capture drain, matching the donor.
//! - Demo recording/playback is `crate::bootstrap::demo_recording`,
//!   `demo_recording_commands.rs`, `demo_playback.rs`,
//!   `network/recorded_source.rs`, and `network/q2_demo.rs`; reached via the
//!   recording-command, recording-root, signon-recording, and
//!   `q2_demo_protocol` seams, with recorded-option mapping ported as
//!   `recorded_options`.
//! - GTV sources are `crate::bootstrap::network::gtv_source`; reached via
//!   `prepare_gtv` and `abort_gtv`, with GTV launch validation ported in
//!   `open_gtv_borrowed` and watch polling behind the `PollGtv` advance op.
//! - The content catalog is `qa_content::catalog` (`remote_content_selection`,
//!   `remote_content_product`, `expected_products`); directory normalization is
//!   ported here as [`remote_content_selection`] (string-base sync fold,
//!   donor-faithful), while product mapping stays host-side behind
//!   `content_product`, `product_expectation`, and `default_q2_selection`.
//!
//! Deliberately not listed above: the donor file never references
//! `console.ts` (console-open state is read through `ApplicationInput` focus
//! and covered by the `console_open` seam), `controller-settings.ts`,
//! `local-lobby.ts`, or `loading.ts` (the donor loading frame only touches the
//! renderer and capture, covered by `present_loading_frame`).

use qa_core::cmd::Dialect;
use qa_core::cmd_buffer::{CommandContext, CommandOrigin};
use qa_core::cvar::{flags as cvar_flags, CvarError, CvarRegistry};
use qa_core::identity::{ActorId, ClientId, SeatId, SessionId};
use std::collections::HashSet;
use thiserror::Error;

/// Remote application failure (donor `Error` + `AggregateError` sites).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RemoteApplicationError {
    /// Single failure with the donor message.
    #[error("{0}")]
    Message(String),
    /// Aggregated failures with the donor aggregate message.
    #[error("{message}: {errors:?}")]
    Aggregate {
        /// Donor aggregate message.
        message: String,
        /// Collected failure messages in donor order.
        errors: Vec<String>,
    },
}

impl From<CvarError> for RemoteApplicationError {
    fn from(error: CvarError) -> Self {
        Self::Message(error.to_string())
    }
}

/// Native protocol family (donor `"q1" | "q2" | "q3" | "qw"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RemoteFamily {
    /// NetQuake.
    Q1,
    /// Quake II.
    Q2,
    /// Quake III.
    Q3,
    /// QuakeWorld.
    Qw,
}

/// Remote network kind (donor `options.network.kind` values consumed here).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteNetworkKind {
    /// NetQuake client.
    Q1Client,
    /// QuakeWorld client.
    QwClient,
    /// Quake II client.
    Q2Client,
    /// Quake III client.
    Q3Client,
    /// Unified client.
    UnifiedClient,
    /// Offline/recorded.
    Offline,
    /// Any other network kind (server, menu, ...).
    Other(String),
}

/// Remote network endpoint (donor `options.network`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteNetworkOptions {
    /// Endpoint kind.
    pub kind: RemoteNetworkKind,
    /// Remote address text.
    pub remote: String,
}

/// Quake II protocol selection (donor `q2Protocol`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteQ2ProtocolKind {
    /// Classic 34.
    Classic,
    /// Rerelease.
    Rerelease,
    /// KEX.
    Kex,
    /// KEX single-view demo protocol (rejected for GTV).
    KexDemo,
}

/// Quake II protocol (donor `NonNullable<ApplicationOptions["q2Protocol"]>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoteQ2Protocol {
    /// Protocol kind.
    pub kind: RemoteQ2ProtocolKind,
    /// Protocol version.
    pub version: u32,
}

/// Default live Q2 protocol (donor `liveQ2Protocol`).
pub fn live_q2_protocol(rerelease: bool) -> RemoteQ2Protocol {
    if rerelease {
        RemoteQ2Protocol {
            kind: RemoteQ2ProtocolKind::Kex,
            version: 2023,
        }
    } else {
        RemoteQ2Protocol {
            kind: RemoteQ2ProtocolKind::Classic,
            version: 34,
        }
    }
}

/// Datagram transport selection (donor `networkTransport`, default UDP).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteTransportKind {
    /// UDP datagrams.
    Udp,
    /// IPX datagrams (rejected for QuakeWorld).
    Ipx,
}

/// Remote content directory selection (donor `RemoteContentSelection`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteContentSelection {
    /// Content base id.
    pub base: String,
    /// Game directory.
    pub directory: String,
}

/// Normalize a remote content selection (donor `remoteContentSelection`).
pub fn remote_content_selection(
    base: &str,
    game_directory: &str,
) -> Result<RemoteContentSelection, RemoteApplicationError> {
    let directory = game_directory.to_lowercase();
    if base == "q1-quakeworld" && (directory == "qw" || directory == "id1") {
        return Ok(RemoteContentSelection {
            base: base.to_string(),
            directory: "qw".to_string(),
        });
    }
    if (base == "q2-classic-baseq2" || base == "q2-rerelease-baseq2") && directory.is_empty() {
        return Ok(RemoteContentSelection {
            base: base.to_string(),
            directory: "baseq2".to_string(),
        });
    }
    if base == "q3-baseq3" && directory.is_empty() {
        return Ok(RemoteContentSelection {
            base: base.to_string(),
            directory: "baseq3".to_string(),
        });
    }
    let safe = !directory.is_empty()
        && directory != "."
        && !directory.contains("..")
        && directory
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '.' | '-'));
    if !safe {
        return Err(RemoteApplicationError::Message(
            "Remote game directory must be a single safe directory name".to_string(),
        ));
    }
    Ok(RemoteContentSelection {
        base: base.to_string(),
        directory,
    })
}

/// Product expectation consumed by launch validation (donor catalog subset).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteProductExpectation {
    /// Product id.
    pub id: String,
    /// Product family.
    pub family: String,
    /// Rerelease edition flag.
    pub rerelease: bool,
}

/// Remote application launch options (donor `ApplicationOptions` fields read here).
#[derive(Debug, Clone, PartialEq)]
pub struct RemoteApplicationOptions {
    /// Network endpoint.
    pub network: RemoteNetworkOptions,
    /// Product id.
    pub product: String,
    /// Character provider (`q1`/`q2`/`q3`).
    pub character: String,
    /// Character model name.
    pub character_model: String,
    /// Movement provider (`q1`/`q2`/`q3`).
    pub movement: String,
    /// Seat count (1-4).
    pub seats: u32,
    /// Map path.
    pub map: String,
    /// Content seed.
    pub seed: u64,
    /// Display gamma override.
    pub gamma: Option<f64>,
    /// Frame limit, if any.
    pub frame_limit: Option<u64>,
    /// Startup commands.
    pub startup_commands: Vec<String>,
    /// Remote content selection, if any.
    pub remote_content: Option<RemoteContentSelection>,
    /// Quake II protocol override, if any.
    pub q2_protocol: Option<RemoteQ2Protocol>,
    /// Datagram transport override, if any.
    pub network_transport: Option<RemoteTransportKind>,
    /// User content root override, if any.
    pub user_content_root: Option<String>,
    /// Dedicated server flag (rejected for remote play).
    pub dedicated: bool,
    /// Ruleset name.
    pub rules: String,
    /// Quake III product override, if any.
    pub q3_product: Option<String>,
}

/// Recorded demo resource (donor `DemoResource` fields consumed here).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteDemoKind {
    /// NetQuake demo.
    Q1,
    /// QuakeWorld demo.
    Qw,
    /// Quake II demo.
    Q2,
    /// Quake III demo.
    Q3,
}

/// Recorded playback launch (donor `{ kind: "recorded", ... }`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteRecordedLaunch {
    /// Demo kind.
    pub kind: RemoteDemoKind,
    /// Demo path for messages.
    pub path: String,
    /// Timedemo flag.
    pub timedemo: bool,
    /// Quake II playback protocol, if known.
    pub q2_protocol: Option<RemoteQ2ProtocolKind>,
}

/// GTV watch launch (donor `{ kind: "gtv", ... }`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteGtvLaunch {
    /// Watch label for messages.
    pub label: String,
    /// Rerelease header flag.
    pub rerelease: bool,
    /// Header game directory.
    pub game_directory: String,
    /// Header protocol.
    pub protocol: RemoteQ2Protocol,
}

/// Recording launch (donor `DemoLaunch | GtvLaunch`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteRecordingLaunch {
    /// Recorded demo playback.
    Recorded(RemoteRecordedLaunch),
    /// GTV watch.
    Gtv(RemoteGtvLaunch),
}

/// Remote source kind (donor `RemoteSource["kind"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteSourceKind {
    /// Live server connection.
    Live,
    /// Recorded demo playback.
    Recorded,
    /// GTV watch.
    Gtv,
}

/// Network phase (donor `ApplicationNetworkPhase`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteNetworkPhase {
    /// Challenge handshake.
    Challenging,
    /// Connecting.
    Connecting,
    /// Loading.
    Loading,
    /// Active play.
    Active,
    /// Closed.
    Closed,
    /// Rejected.
    Rejected,
}

/// Queued remote command (donor `RemoteCommand`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteCommand {
    /// Command name.
    pub name: String,
    /// Command arguments.
    pub args: Vec<String>,
    /// Target seat, if any.
    pub seat: Option<SeatId>,
    /// Invocation source, if any.
    pub source: Option<CommandContext>,
}

/// Cinematic request (donor `ScreenCinematicRequest` fields consumed here).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteCinematicRequest {
    /// Cinematic name.
    pub name: String,
    /// Loop flag.
    pub loop_play: bool,
    /// Hold flag.
    pub hold: bool,
    /// Silent flag.
    pub silent: bool,
}

/// Active cinematic ticket (donor `RemoteCinematic` without engines).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoteActiveCinematic {
    /// Request ticket that opened the movie.
    pub ticket: u64,
    /// Systems movie handle.
    pub handle: u64,
    /// Whether the movie is still current.
    pub current: bool,
}

/// Peer admission phase (donor `PeerDirectoryAdmission["phase"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerAdmissionPhase {
    /// Requested, awaiting publication.
    Requested,
    /// Published.
    Published,
}

/// Pending peer directory admission (donor `PeerDirectoryAdmission`, sync fold).
#[derive(Debug, Clone, PartialEq)]
pub struct PeerDirectoryAdmission {
    /// Admission phase.
    pub phase: PeerAdmissionPhase,
    /// Replacement options.
    pub options: RemoteApplicationOptions,
    /// Replacement content selection.
    pub selection: RemoteContentSelection,
    /// Whether the requester is suspended awaiting publication.
    pub suspended: bool,
}

/// Pending advance boundary (donor `pendingAdvance`, sync fold).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingAdvance {
    /// Whether a boundary was signalled during the advance.
    pub boundary: bool,
}

/// Advance operation (donor `advanceSource` operations).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SourceAdvanceOp {
    /// Poll the live primary network.
    PollPrimary,
    /// Poll the GTV watch and sample.
    PollGtv,
    /// Advance recorded playback.
    AdvanceRecorded {
        /// Frame milliseconds.
        frame_ms: f64,
        /// Frame number.
        frame: u64,
        /// Presentation milliseconds.
        presentation_ms: f64,
    },
}

/// Reentrant poll outcome (donor async hooks, sync fold).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PollOutcome {
    /// Requested remote content switches in donor order.
    pub content_switches: Vec<RemoteContentSelection>,
}

/// Remote seat peer bookkeeping (donor `RemoteSeatPeer` without engines).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteSeatPeer {
    /// Peer id (donor object identity).
    pub id: u64,
    /// Peer seat.
    pub seat: SeatId,
    /// Loaded map path, if any.
    pub map: Option<String>,
    /// Content generation.
    pub generation: u64,
    /// Retired flag.
    pub closed: bool,
    /// Disconnect reason, if any.
    pub disconnected: Option<String>,
    /// Last published userinfo, if any.
    pub userinfo: Option<String>,
    /// Whether the peer holds selected content.
    pub has_content: bool,
    /// Whether the peer holds loaded content.
    pub has_loaded_content: bool,
    /// Whether the peer holds a published view.
    pub has_view: bool,
    /// Whether a cinematic completion is pending.
    pub cinematic_ended: bool,
    /// Systems peer handle.
    pub handle: u64,
}

/// Configured seat (donor configuration seat subset).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteConfiguredSeat {
    /// Seat handle.
    pub id: SeatId,
    /// Client handle.
    pub client: ClientId,
}

/// Remote configuration summary (donor `RemoteConfiguration` fields read here).
pub struct RemoteConfigurationSummary {
    /// Configured seats.
    pub seats: Vec<RemoteConfiguredSeat>,
    /// Primary seat.
    pub primary: SeatId,
    /// Primary source registry.
    pub source: CvarRegistry,
    /// Per-seat source registries.
    pub seat_sources: Vec<(SeatId, CvarRegistry)>,
    /// Settings root for recording feeds.
    pub settings_root: String,
    /// Source dialect.
    pub dialect: Dialect,
}

/// Download progress entry (donor `ClientDownloadProgress` fields read here).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteDownloadProgress {
    /// File path.
    pub path: String,
    /// Download phase.
    pub phase: String,
    /// Received bytes.
    pub received: u64,
    /// Total bytes, if known.
    pub total: Option<u64>,
    /// Transport name.
    pub transport: String,
}

/// Presentation event (donor `SimulationPresentationEvent` subset).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemotePresentationEvent {
    /// Event content key.
    pub content: String,
    /// Event map key.
    pub map: String,
    /// Recipient actor, if any.
    pub recipient: Option<ActorId>,
    /// Session departure seats.
    pub departures: Vec<SeatId>,
    /// Level completion marker, if any.
    pub level_completion: Option<String>,
}

/// Unhandled effect report (donor `UnhandledApplicationEffect` subset).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteUnhandledEffect {
    /// Effect source content.
    pub content: String,
    /// Effect source kind.
    pub kind: String,
    /// Failure reason.
    pub reason: String,
}

/// Audio seat batch for session actions (donor `ApplicationAudioSeatEvents` subset).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteAudioBatch {
    /// Batch seat.
    pub seat: SeatId,
    /// Batch events.
    pub events: Vec<RemotePresentationEvent>,
}

/// Presented frame outcome (donor `step` presentation section).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RemoteFramePresentation {
    /// Presentation events for the frame.
    pub events: Vec<RemotePresentationEvent>,
    /// Unhandled effects for the frame.
    pub unhandled: Vec<RemoteUnhandledEffect>,
    /// Audio batches for session actions.
    pub batches: Vec<RemoteAudioBatch>,
}

/// Capture frame decision (donor `sourceCaptureFrame` subset).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CaptureFrameDecision {
    /// Frame milliseconds.
    pub milliseconds: f64,
    /// Whether to capture the frame.
    pub capture: bool,
}

/// Local player view (donor `LocalPlayer` subset).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteLocalPlayer {
    /// Player seat.
    pub seat: SeatId,
    /// Player actor.
    pub actor: ActorId,
}

/// Bind-seat failure (donor `bindSeat` published flag).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindSeatFailure {
    /// Whether the candidate published before failing.
    pub published: bool,
    /// Failure message.
    pub message: String,
}

/// Peer frame outcome (donor `prepareRemoteSeatFrames` per-peer work).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerFrameOutcome {
    /// Peer submitted its frame.
    Submitted,
    /// Peer skipped (inactive, stale world, or no view yet).
    Skipped,
}

/// Qport allocation range (donor `10000 + random * 30000`, floored).
pub const QPORT_BASE: u16 = 10_000;
/// Qport allocation span.
pub const QPORT_SPAN: u16 = 30_000;

/// Unwrap a script origin chain to its caller (donor `while (origin.kind === "script")`).
pub fn unwrap_origin(origin: &CommandOrigin) -> &CommandOrigin {
    let mut current = origin;
    while let CommandOrigin::Script { caller, .. } = current {
        current = caller;
    }
    current
}

/// Quake `atoi` over leading integers (donor `nativeAtoi` behavior for
/// well-formed input: optional sign plus leading ASCII digits, else zero).
pub fn remote_atoi(text: &str) -> i32 {
    let trimmed = text.trim_start();
    let (negative, digits) = match trimmed.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, trimmed.strip_prefix('+').unwrap_or(trimmed)),
    };
    let mut value: i32 = 0;
    let mut any = false;
    for c in digits.chars() {
        if !c.is_ascii_digit() {
            break;
        }
        any = true;
        value = value.saturating_mul(10).saturating_add((c as u8 - b'0') as i32);
    }
    if !any {
        return 0;
    }
    if negative {
        -value
    } else {
        value
    }
}

/// Seed seat cvars for a dialect (donor `prepareRemoteConfiguration` registrations).
pub fn seed_seat_cvars(
    dialect: Dialect,
    seat_index: u32,
    character_model: &str,
    unified_client: bool,
) -> Result<CvarRegistry, CvarError> {
    let mut cvars = CvarRegistry::new(dialect);
    let player = if seat_index == 0 {
        "Player".to_string()
    } else {
        format!("Player {}", seat_index + 1)
    };
    match dialect {
        Dialect::Q1Netquake => {
            cvars.register("name", &player, cvar_flags::ARCHIVE)?;
            cvars.register("color", "0", cvar_flags::ARCHIVE)?;
            if unified_client {
                cvars.register("password", "", cvar_flags::USER_INFO)?;
            }
        }
        Dialect::Q2Classic | Dialect::Q2Rerelease => {
            let flags = cvar_flags::ARCHIVE | cvar_flags::USER_INFO;
            let skin = match character_model {
                "female" => "female/athena",
                "cyborg" => "cyborg/oni911",
                _ => "male/grunt",
            };
            let gender = if character_model == "female" { "female" } else { "male" };
            for (name, value) in [
                ("name", player.as_str()),
                ("skin", skin),
                ("rate", "25000"),
                ("msg", "1"),
                ("hand", "0"),
                ("fov", "90"),
                ("gender", gender),
            ] {
                cvars.register(name, value, flags)?;
            }
            cvars.register("password", "", cvar_flags::USER_INFO)?;
            cvars.register("spectator", "0", cvar_flags::USER_INFO)?;
        }
        // Q3 client cvars seed through the host client-cvar initializer.
        Dialect::Q3 => {
            let _ = player;
            let _ = character_model;
        }
        Dialect::Q1Quakeworld => {
            cvars.register("cl_hightrack", "0", cvar_flags::NONE)?;
            cvars.register("cl_chasecam", "0", 0)?;
            cvars.register("rate", "2500", cvar_flags::ARCHIVE | cvar_flags::USER_INFO)?;
            cvars.register("noskins", "0", cvar_flags::ARCHIVE)?;
            cvars.register("baseskin", "base", cvar_flags::ARCHIVE)?;
            for (name, value) in [
                ("name", "unnamed"),
                ("team", ""),
                ("skin", ""),
                ("topcolor", "0"),
                ("bottomcolor", "0"),
                ("noaim", "0"),
                ("msg", "1"),
            ] {
                cvars.register(name, value, cvar_flags::ARCHIVE | cvar_flags::USER_INFO)?;
            }
            cvars.register("password", "", cvar_flags::USER_INFO)?;
        }
    }
    Ok(cvars)
}

/// Resolved remote address (donor `ApplicationNetworkAddress` subset).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteAddress {
    /// Address key for messages.
    pub key: String,
    /// IPv6 flag (bind host selection).
    pub ipv6: bool,
    /// IPX flag (rejected for QuakeWorld).
    pub ipx: bool,
    /// IPv4 flag (required for native Q3).
    pub ipv4: bool,
}

/// Pending client input (donor `clientInputs` entries, sync fold).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingClientInput {
    /// World generation that captured the event.
    pub generation: u64,
    /// Owning Q3 client handle.
    pub client: u64,
    /// Input event code.
    pub event: u32,
    /// Event seat.
    pub seat: SeatId,
}

/// Host subsystem operations for [`RemoteApplication`].
///
/// Every method is a missing-sibling seam: the wiring lane implements the
/// network stacks, content loading, frontend, input, renderer, capture,
/// browsers, recording, session, and configuration preparation behind this
/// trait while the port keeps the donor sequencing and decisions.
pub trait RemoteApplicationSystems {
    /// Current wall time in milliseconds.
    fn now_ms(&mut self) -> f64;
    /// Print a line.
    fn print(&mut self, text: &str);
    /// Sleep at least the given milliseconds.
    fn sleep_ms(&mut self, ms: f64);
    /// Random qport offset in `0..QPORT_SPAN`.
    fn random_qport_offset(&mut self) -> u16;
    /// Resolve a remote address with a default port.
    fn resolve_address(
        &mut self,
        remote: &str,
        default_port: u16,
        family: RemoteFamily,
    ) -> Result<RemoteAddress, String>;
    /// Map a content selection to its product id.
    fn content_product(&mut self, selection: &RemoteContentSelection) -> String;
    /// Look up a product expectation by id.
    fn product_expectation(&mut self, product: &str) -> Option<RemoteProductExpectation>;
    /// Default content selection for a live Q2 endpoint.
    fn default_q2_selection(&mut self, rerelease: bool) -> RemoteContentSelection;
    /// Whether the retained server browser exists (recorded guards).
    fn has_server_browser(&mut self) -> bool;
    /// Q3 product reported by opened content, if any.
    fn open_content_q3_product(&mut self, options: &RemoteApplicationOptions) -> Option<String>;
    /// Resolve movement/character providers against the catalog.
    fn resolve_movement(&mut self, options: &RemoteApplicationOptions) -> Result<RemoteApplicationOptions, String>;
    /// Finish opening: renderer, transport, browser, keys, seats, video,
    /// configuration publication, and the connecting banner.
    fn finish_open(&mut self, request: &OpenRequest) -> Result<OpenHandles, String>;
    /// Abort a failed open at the given stage, returning cleanup failures.
    fn abort_open(&mut self, stage: &str) -> Vec<String>;
    /// Session id for command contexts.
    fn session_id(&mut self) -> SessionId;
    /// Primary seat id.
    fn primary_seat_id(&mut self) -> SeatId;
    /// Prepare the remote configuration summary.
    fn prepare_configuration(
        &mut self,
        options: &RemoteApplicationOptions,
        seeds: &[(SeatId, CvarRegistry)],
    ) -> Result<RemoteConfigurationSummary, String>;
    /// Publish the prepared configuration.
    fn publish_configuration(&mut self) -> Result<(), String>;
    /// Publish a peer configuration for the given options.
    fn publish_peer_configuration(&mut self, options: &RemoteApplicationOptions) -> Result<(), String>;
    /// Register source command handlers.
    fn activate_source_commands(&mut self);
    /// Release bound settings.
    fn release_settings(&mut self);
    /// Persist source settings.
    fn save_source_settings(&mut self) -> Result<(), String>;
    /// Current primary network phase.
    fn network_phase(&mut self) -> RemoteNetworkPhase;
    /// Execute an advance operation, returning reentrant poll outcomes.
    fn execute_advance(&mut self, op: SourceAdvanceOp) -> Result<PollOutcome, String>;
    /// Submit the primary command (timed for unified networks).
    fn submit_primary(&mut self, timed: bool) -> Result<(), String>;
    /// Whether the primary connection needs a userinfo sync (Q2 active).
    fn primary_needs_userinfo_sync(&mut self) -> bool;
    /// Current primary remote userinfo, if any.
    fn primary_remote_userinfo(&mut self) -> Option<String>;
    /// Publish primary userinfo.
    fn set_primary_userinfo(&mut self, info: &str);
    /// Whether the primary network has a demo recorder.
    fn primary_has_recorder(&mut self) -> bool;
    /// Whether a peer network has a demo recorder.
    fn peer_has_recorder(&mut self, handle: u64) -> bool;
    /// Recording feed root.
    fn recording_root(&mut self) -> String;
    /// Run a standalone recording command.
    fn recording_command(&mut self, name: &str, arg: Option<&str>, context: &CommandContext) -> Result<(), String>;
    /// Stop borrowed recording.
    fn stop_borrowed_recording(&mut self) -> Result<(), String>;
    /// Stop owned recording.
    fn stop_owned_recording(&mut self) -> Result<(), String>;
    /// Attach a signon recording sink, returning an attachment handle.
    fn attach_signon_recording(&mut self) -> Result<u64, String>;
    /// Detach a signon recording sink.
    fn detach_signon_recording(&mut self, attachment: u64);
    /// Whether the source network is a live QuakeWorld connection.
    fn is_live_quakeworld(&mut self) -> bool;
    /// Open a peer connection, returning a peer handle.
    fn open_peer(&mut self, seat: &SeatId, address: &RemoteAddress, qport: u16) -> Result<u64, String>;
    /// Abort a failed peer open (transport cleanup).
    fn abort_peer(&mut self, seat: &SeatId);
    /// Whether a peer channel is active.
    fn peer_active(&mut self, handle: u64) -> bool;
    /// Poll a peer pump.
    fn poll_peer(&mut self, handle: u64, now_ms: f64);
    /// Drain ready peer publications, surfacing failures.
    fn drain_peer(&mut self, handle: u64) -> Result<(), String>;
    /// Take a pending peer disconnect reason, if any.
    fn take_peer_disconnected(&mut self, handle: u64) -> Option<String>;
    /// Current peer remote userinfo for Q2 sync, if applicable.
    fn peer_remote_userinfo(&mut self, handle: u64) -> Option<String>;
    /// Publish peer userinfo.
    fn set_peer_userinfo(&mut self, handle: u64, info: &str);
    /// Sample a peer presentation.
    fn sample_peer_presentation(&mut self, handle: u64, now_ms: f64);
    /// Prepare peer skins (QuakeWorld).
    fn prepare_peer_skins(&mut self, handle: u64) -> Result<(), String>;
    /// Whether a peer needs its seat bound (non-Q3 with output).
    fn peer_needs_bind(&mut self, handle: u64) -> bool;
    /// Bind a peer seat view.
    fn bind_peer_seat(&mut self, handle: u64) -> Result<(), String>;
    /// Submit a peer frame.
    fn submit_peer_frame(&mut self, handle: u64, now_ms: f64, wall_elapsed_ms: f64)
        -> Result<PeerFrameOutcome, String>;
    /// Retire a peer view.
    fn retire_peer_view(&mut self, handle: u64) -> Result<(), String>;
    /// Close a peer pump, channel, and content, aggregating failures.
    fn close_peer(&mut self, handle: u64) -> Vec<String>;
    /// Peer download progress.
    fn peer_download_progress(&mut self, handle: u64) -> Vec<RemoteDownloadProgress>;
    /// Cancel peer downloads.
    fn cancel_peer_downloads(&mut self, handle: u64);
    /// Retry peer downloads.
    fn retry_peer_downloads(&mut self, handle: u64) -> Result<(), String>;
    /// Drop a peer seat by index, promoting the replacement if any.
    fn drop_peer_seat(&mut self, index: u32, replacement: Option<u64>) -> Result<(), String>;
    /// Join an additional peer seat, returning its handle and identity.
    fn join_peer_seat(&mut self) -> Result<(u64, SeatId, ClientId), String>;
    /// Primary download progress.
    fn primary_download_progress(&mut self) -> Vec<RemoteDownloadProgress>;
    /// Cancel primary downloads.
    fn cancel_primary_downloads(&mut self);
    /// Retry primary downloads.
    fn retry_primary_downloads(&mut self) -> Result<(), String>;
    /// Bind the primary seat presentation.
    fn bind_primary_seat(&mut self) -> Result<bool, BindSeatFailure>;
    /// Sample the primary presentation.
    fn sample_primary_presentation(&mut self, now_ms: f64);
    /// Prepare primary skins (QuakeWorld).
    fn prepare_primary_skins(&mut self) -> Result<(), String>;
    /// Whether the primary remote has output ready.
    fn primary_output_present(&mut self) -> bool;
    /// Sync the primary Q3 command selection.
    fn sync_primary_q3_selection(&mut self);
    /// Present the primary frame, returning presentation outcomes.
    fn present_primary_frame(&mut self) -> Result<RemoteFramePresentation, String>;
    /// Record a level completion for a seat.
    fn record_level_completion(&mut self, seat: &SeatId, content: &str, map: &str, marker: &str);
    /// Open a cinematic movie, returning a movie handle.
    fn open_movie(&mut self, request: &RemoteCinematicRequest) -> Result<u64, String>;
    /// Activate a movie handle.
    fn activate_movie(&mut self, handle: u64) -> Result<(), String>;
    /// Route input to a movie handle.
    fn movie_input(&mut self, handle: u64, event: u32) -> bool;
    /// Advance a movie frame, returning whether it ended.
    fn movie_frame(&mut self, handle: u64, frame_ms: f64, frame: u64, console_open: bool) -> bool;
    /// Pause or resume a movie handle.
    fn movie_pause(&mut self, handle: u64, paused: bool);
    /// Whether a movie handle is paused.
    fn movie_paused(&mut self, handle: u64) -> bool;
    /// Close a movie bundle, aggregating failures.
    fn close_movie(&mut self, handle: u64) -> Vec<String>;
    /// Whether any console is open.
    fn console_open(&mut self) -> bool;
    /// Run a Q3 browser command.
    fn browser_command(&mut self, name: &str, args: &[String]) -> Result<bool, String>;
    /// Whether the Q3 browser exists.
    fn has_q3_browser(&mut self) -> bool;
    /// Poll browsers.
    fn poll_browsers(&mut self);
    /// Close the Q3 browser.
    fn close_q3_browser(&mut self);
    /// Run a frontend audio command.
    fn audio_command(&mut self, name: &str, args: &[String], seat: Option<&SeatId>) -> Result<bool, String>;
    /// Run a Q3 client command for a seat, if a guest owns it.
    fn q3_command(&mut self, seat: Option<&SeatId>, argv: &[String]) -> Result<bool, String>;
    /// Resolve a player actor to its presentation owner.
    fn presentation_for_actor(&mut self, actor: &ActorId) -> Result<u64, String>;
    /// Send a gameplay command for an actor.
    fn player_command(&mut self, actor: &ActorId, name: &str, args: &[String]);
    /// Recenter a player view.
    fn centerview(&mut self, actor: &ActorId);
    /// Find a UI item id for a `use` command.
    fn use_item_ordinal(&mut self, actor: &ActorId, item: &str) -> Option<i32>;
    /// Current local players.
    fn local_players(&mut self) -> Vec<RemoteLocalPlayer>;
    /// Q3 client handle capturing input for a seat, if any.
    fn input_client_for_seat(&mut self, seat: &SeatId) -> Option<u64>;
    /// Whether a Q3 client handle is still live.
    fn input_client_live(&mut self, client: u64) -> bool;
    /// Deliver deferred input to a Q3 client handle.
    fn deliver_client_input(&mut self, client: u64, event: u32) -> Result<(), String>;
    /// Route input to controls.
    fn controls_input(&mut self, event: u32) -> bool;
    /// Whether input devices are active.
    fn has_input(&mut self) -> bool;
    /// Print input device info.
    fn input_devices_info(&mut self);
    /// Restart input devices and routers.
    fn restart_input_devices(&mut self);
    /// Pump client input.
    fn pump_client_input(&mut self);
    /// Whether a quit was requested through the window.
    fn window_quit_requested(&mut self) -> bool;
    /// Whether client commands are blocked (capture readback or video restart).
    fn client_commands_blocked(&mut self) -> bool;
    /// Advance client startup, returning whether it continued.
    fn advance_client_startup(&mut self) -> Result<bool, String>;
    /// Execute pending startup scripts.
    fn execute_startup_scripts(&mut self) -> Result<(), String>;
    /// Dispatch an application request to the borrowed owner.
    fn dispatch_borrowed(
        &mut self,
        name: &str,
        args: &[String],
        seat: Option<&SeatId>,
        source: &CommandContext,
    ) -> Result<(), String>;
    /// Compute source frame milliseconds from time cvars.
    fn source_frame_ms(&mut self, elapsed_ms: f64) -> f64;
    /// Decide capture framing for the frame.
    fn capture_frame(&mut self, source_ms: f64, active: bool) -> CaptureFrameDecision;
    /// Capture the current frame.
    fn capture_frame_now(&mut self);
    /// Drain capture.
    fn drain_capture(&mut self) -> Result<(), String>;
    /// Notify capture of a world change.
    fn capture_before_world_change(&mut self) -> Result<(), String>;
    /// Close capture (owned).
    fn close_capture(&mut self) -> Result<(), String>;
    /// Activate capture (owned).
    fn activate_capture(&mut self);
    /// Advance presentation time.
    fn advance_presentation_time(&mut self, now_ms: f64, elapsed_ms: f64, frame_ms: f64);
    /// Presentation milliseconds for recorded advance.
    fn presentation_ms(&mut self) -> f64;
    /// Begin a peer frame.
    fn peer_begin_frame(&mut self, handle: u64, now_ms: f64, elapsed_ms: f64);
    /// Present a loading frame.
    fn present_loading_frame(&mut self) -> Result<(), String>;
    /// Refresh images.
    fn refresh_images(&mut self) -> Result<(), String>;
    /// Stop control haptics.
    fn stop_haptics(&mut self);
    /// Save control settings.
    fn save_control_settings(&mut self) -> Result<(), String>;
    /// Close controls.
    fn close_controls(&mut self);
    /// Whether the borrowed source currently points at this application.
    fn borrowed_source_is_current(&mut self) -> bool;
    /// Release the borrowed source publication.
    fn release_borrowed_source(&mut self) -> Result<(), String>;
    /// Shut down the primary Q3 client.
    fn shutdown_primary_q3(&mut self) -> Result<(), String>;
    /// Clear the source presentation.
    fn clear_presentation(&mut self);
    /// Disconnect the primary source connection.
    fn disconnect_source(&mut self);
    /// Close initial components.
    fn close_initial_components(&mut self) -> Result<(), String>;
    /// Close content, downloads, network, transport, controls, and frontend.
    fn close_content_bundle(&mut self) -> Vec<String>;
    /// Retire borrowed frontends, aggregating failures.
    fn retire_frontends(&mut self) -> Vec<String>;
    /// Flush the renderer with an empty batch.
    fn flush_renderer(&mut self) -> Result<(), String>;
    /// Unregister source command handlers.
    fn unregister_source_handlers(&mut self, names: &[String]);
    /// Whether retained scripts should close on shutdown.
    fn should_close_scripts(&mut self) -> bool;
    /// Close retained scripts.
    fn close_scripts(&mut self) -> Result<(), String>;
    /// Close the owned session bundle (session, renderer, image settings).
    fn close_owned_bundle(&mut self) -> Vec<String>;
    /// Close the owned server browser.
    fn close_server_browser(&mut self) -> Result<(), String>;
    /// Close loaded content.
    fn close_loaded_content(&mut self) -> Result<(), String>;
    /// Discard unpublished seat identities.
    fn discard_identities(&mut self) -> Result<(), String>;
    /// Close image settings.
    fn close_images_settings(&mut self) -> Result<(), String>;
    /// Drain the owned video restart.
    fn drain_video_restart(&mut self) -> Result<(), String>;
    /// Close the owned video restart.
    fn close_video_restart(&mut self) -> Result<(), String>;
    /// Current video restart generation.
    fn video_generation(&mut self) -> u64;
    /// Read window pixels.
    fn read_pixels(&mut self) -> Vec<u8>;
    /// Capture the next frame pixels.
    fn capture_next_frame(&mut self) -> Vec<u8>;
    /// Select remote content mounts, returning whether the product changed.
    fn select_content_mounts(&mut self, selection: &RemoteContentSelection) -> Result<bool, String>;
    /// Prepare GTV watch content for a launch.
    fn prepare_gtv(&mut self, launch: &RemoteGtvLaunch) -> Result<(), String>;
    /// Abort a prepared GTV watch.
    fn abort_gtv(&mut self);
    /// Read a Q2 playback protocol from a demo header kind.
    fn q2_demo_protocol(&mut self, recorded: &RemoteRecordedLaunch) -> Option<RemoteQ2ProtocolKind>;
}

/// Open request for [`RemoteApplicationSystems::finish_open`].
#[derive(Debug, Clone, PartialEq)]
pub struct OpenRequest {
    /// Resolved launch options.
    pub options: RemoteApplicationOptions,
    /// Borrowed ownership flag.
    pub borrowed: bool,
    /// Recording launch, if any.
    pub recording: Option<RemoteRecordingLaunch>,
    /// Resolved address, if any.
    pub address: Option<RemoteAddress>,
}

/// Opened subsystem handles (donor open sequencing state).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OpenHandles {
    /// Seat ids admitted during open, primary first.
    pub seats: [Option<SeatIdIndex>; 4],
}

/// Seat id by index (systems mint real ids; the port tracks indices).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SeatIdIndex {
    /// Seat index.
    pub index: u32,
}

/// Remote application ownership (donor `RemoteOwnership`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteOwnership {
    /// Owned subsystems.
    Owned,
    /// Borrowed from a retained client.
    Borrowed,
}

/// Native remote-client application (donor `RemoteApplication`).
pub struct RemoteApplication {
    /// Launch options (donor `launchOptions`).
    options: RemoteApplicationOptions,
    /// Native family (donor `family`).
    family: RemoteFamily,
    /// Subsystem host.
    systems: Box<dyn RemoteApplicationSystems>,
    /// Ownership.
    ownership: RemoteOwnership,
    /// Source kind.
    source_kind: RemoteSourceKind,
    /// GTV watch time override, if any.
    watch_time_ms: Option<f64>,
    /// Session id.
    session: SessionId,
    /// Primary seat id.
    seat_id: SeatId,
    /// Remote seat peers (donor `remoteSeats`).
    peers: Vec<RemoteSeatPeer>,
    /// Primary peer id, if promoted.
    primary_peer: Option<u64>,
    /// Next peer id.
    next_peer_id: u64,
    /// Queued commands (donor `commands`).
    commands: Vec<RemoteCommand>,
    /// Allocated qports (donor `qports`).
    qports: HashSet<u16>,
    /// Initial qport, if any.
    initial_qport: Option<u16>,
    /// Stopping flag.
    stopping: bool,
    /// Closing flag.
    closing: bool,
    /// Closed flag.
    closed: bool,
    /// Close already performed (donor `closeResult`).
    close_done: bool,
    /// Step reentrancy guard.
    stepping: bool,
    /// Frames presented.
    frames: u64,
    /// Elapsed milliseconds.
    elapsed_ms: f64,
    /// Active cinematic, if any.
    cinematic: Option<RemoteActiveCinematic>,
    /// Cinematic request ticket counter.
    cinematic_request: u64,
    /// Presentation events for the frame.
    source_events: Vec<RemotePresentationEvent>,
    /// Unhandled effects for the frame.
    unhandled_effects: Vec<RemoteUnhandledEffect>,
    /// Reported effect gap keys.
    reported_effect_gaps: HashSet<String>,
    /// Deferred client inputs.
    client_inputs: Vec<PendingClientInput>,
    /// World load generation.
    world_load_generation: u64,
    /// Pending peer admission, if any.
    peer_admission: Option<PeerDirectoryAdmission>,
    /// Pending advance boundary, if any.
    pending_advance: Option<PendingAdvance>,
    /// Reset frame elapsed flag.
    reset_frame_elapsed: bool,
    /// Configuration summary, if prepared.
    configuration: Option<RemoteConfigurationSummary>,
    /// Configuration published flag.
    configuration_published: bool,
    /// Primary initial userinfo for Q2 sync.
    initial_userinfo: Option<String>,
    /// QW all-skins argument.
    qw_all_skins: String,
    /// Whether the primary view is current.
    primary_view_current: bool,
}

impl std::fmt::Debug for RemoteApplication {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteApplication")
            .field("options", &self.options)
            .field("family", &self.family)
            .field("ownership", &self.ownership)
            .field("source_kind", &self.source_kind)
            .field("peers", &self.peers)
            .field("commands", &self.commands)
            .field("frames", &self.frames)
            .field("closed", &self.closed)
            .finish_non_exhaustive()
    }
}

impl RemoteApplication {
    /// Open a live application with owned subsystems (donor `open`).
    pub fn open(
        options: RemoteApplicationOptions,
        systems: Box<dyn RemoteApplicationSystems>,
    ) -> Result<Self, RemoteApplicationError> {
        Self::open_source(RemoteOwnership::Owned, options, systems, None).map_err(|failure| failure.error)
    }

    /// Open a live application borrowed from a retained client (donor `openBorrowed`).
    pub fn open_borrowed(
        options: RemoteApplicationOptions,
        systems: Box<dyn RemoteApplicationSystems>,
    ) -> Result<Self, RemoteApplicationError> {
        Self::open_source(RemoteOwnership::Borrowed, options, systems, None).map_err(|failure| failure.error)
    }

    /// Open recorded playback borrowed from a retained client (donor `openDemoBorrowed`).
    pub fn open_demo_borrowed(
        options: RemoteApplicationOptions,
        systems: Box<dyn RemoteApplicationSystems>,
        playback: RemoteRecordedLaunch,
    ) -> Result<Self, RemoteApplicationError> {
        Self::open_source(
            RemoteOwnership::Borrowed,
            options,
            systems,
            Some(RemoteRecordingLaunch::Recorded(playback)),
        )
        .map_err(|failure| failure.error)
    }

    /// Open a GTV watch borrowed from a retained client (donor `openGtvBorrowed`).
    pub fn open_gtv_borrowed(
        options: RemoteApplicationOptions,
        mut systems: Box<dyn RemoteApplicationSystems>,
        launch: RemoteGtvLaunch,
    ) -> Result<Self, RemoteApplicationError> {
        systems.prepare_gtv(&launch).map_err(RemoteApplicationError::Message)?;
        if launch.protocol.kind == RemoteQ2ProtocolKind::KexDemo {
            systems.abort_gtv();
            return Err(RemoteApplicationError::Message(
                "GTV MVD cannot use the KEX single-view demo protocol".to_string(),
            ));
        }
        let selection = match remote_content_selection(
            if launch.rerelease {
                "q2-rerelease-baseq2"
            } else {
                "q2-classic-baseq2"
            },
            &launch.game_directory,
        ) {
            Ok(selection) => selection,
            Err(error) => {
                systems.abort_gtv();
                return Err(error);
            }
        };
        let product = systems.content_product(&selection);
        let character_model = if options.character == "q2" {
            options.character_model.clone()
        } else {
            "male".to_string()
        };
        let selected = RemoteApplicationOptions {
            network: RemoteNetworkOptions {
                kind: RemoteNetworkKind::Offline,
                remote: String::new(),
            },
            seats: 1,
            dedicated: false,
            rules: "standard".to_string(),
            movement: "q2".to_string(),
            character: "q2".to_string(),
            character_model,
            product,
            remote_content: Some(selection),
            q2_protocol: Some(launch.protocol),
            ..options.clone()
        };
        match Self::open_source(
            RemoteOwnership::Borrowed,
            selected,
            systems,
            Some(RemoteRecordingLaunch::Gtv(launch)),
        ) {
            Ok(application) => Ok(application),
            Err(failure) => {
                let mut systems = failure.systems;
                systems.abort_gtv();
                Err(failure.error)
            }
        }
    }
}

/// Failed open handing the systems host back for abort cleanup.
pub struct OpenFailure {
    /// Systems host.
    pub systems: Box<dyn RemoteApplicationSystems>,
    /// Failure.
    pub error: RemoteApplicationError,
}

impl RemoteApplication {
    /// Map recorded options from launch options (donor `recordedOptions`).
    fn recorded_options(
        options: &RemoteApplicationOptions,
        resource: &RemoteRecordedLaunch,
        systems: &mut Box<dyn RemoteApplicationSystems>,
    ) -> Result<RemoteApplicationOptions, RemoteApplicationError> {
        let family = match resource.kind {
            RemoteDemoKind::Qw => "q1",
            RemoteDemoKind::Q1 => "q1",
            RemoteDemoKind::Q2 => "q2",
            RemoteDemoKind::Q3 => "q3",
        };
        let selected = systems.product_expectation(&options.product);
        let netquake_product = match selected {
            Some(product) if product.family == "q1" && !product.rerelease => product.id,
            _ => "q1-classic-id1".to_string(),
        };
        let q2 = match resource.kind {
            RemoteDemoKind::Q2 => systems.q2_demo_protocol(resource),
            _ => None,
        };
        let base = match resource.kind {
            RemoteDemoKind::Qw => "q1-quakeworld",
            RemoteDemoKind::Q2 => match q2 {
                Some(RemoteQ2ProtocolKind::Rerelease | RemoteQ2ProtocolKind::Kex | RemoteQ2ProtocolKind::KexDemo) => {
                    "q2-rerelease-baseq2"
                }
                _ => "q2-classic-baseq2",
            },
            _ => "q3-baseq3",
        };
        let selection = if resource.kind == RemoteDemoKind::Q1 {
            None
        } else if options.remote_content.as_ref().is_some_and(|prior| prior.base == base) {
            options.remote_content.clone()
        } else {
            Some(remote_content_selection(
                base,
                match resource.kind {
                    RemoteDemoKind::Qw => "qw",
                    RemoteDemoKind::Q2 => "baseq2",
                    _ => "baseq3",
                },
            )?)
        };
        let character_model = if options.character == family {
            options.character_model.clone()
        } else if family == "q1" {
            "player".to_string()
        } else if family == "q2" {
            "male".to_string()
        } else {
            "sarge".to_string()
        };
        let product = match selection.as_ref() {
            None => netquake_product,
            Some(selection) => systems.content_product(selection),
        };
        Ok(RemoteApplicationOptions {
            network: RemoteNetworkOptions {
                kind: RemoteNetworkKind::Offline,
                remote: String::new(),
            },
            seats: 1,
            dedicated: false,
            rules: "standard".to_string(),
            movement: family.to_string(),
            character: family.to_string(),
            character_model,
            product,
            remote_content: selection,
            ..options.clone()
        })
    }

    /// Shared open flow (donor `openSource`, sync fold).
    fn open_source(
        ownership: RemoteOwnership,
        mut options: RemoteApplicationOptions,
        mut systems: Box<dyn RemoteApplicationSystems>,
        recording: Option<RemoteRecordingLaunch>,
    ) -> Result<Self, OpenFailure> {
        let fail =
            |systems: Box<dyn RemoteApplicationSystems>, error: RemoteApplicationError| OpenFailure { systems, error };
        if recording.is_some() && !systems.has_server_browser() {
            return Err(fail(
                systems,
                RemoteApplicationError::Message("Recorded playback requires the retained server browser".to_string()),
            ));
        }
        if let Some(RemoteRecordingLaunch::Recorded(playback)) = recording.as_ref() {
            match Self::recorded_options(&options, playback, &mut systems) {
                Ok(mapped) => options = mapped,
                Err(error) => return Err(fail(systems, error)),
            }
        }
        if recording.is_none() && options.network.kind == RemoteNetworkKind::Q2Client {
            let base = options
                .remote_content
                .as_ref()
                .map(|selection| selection.base.as_str())
                .unwrap_or(&options.product);
            let rerelease = systems
                .product_expectation(base)
                .is_some_and(|product| product.rerelease);
            let protocol = options.q2_protocol.unwrap_or_else(|| live_q2_protocol(rerelease));
            let selection = options.remote_content.clone().unwrap_or_else(|| {
                systems.default_q2_selection(
                    protocol.kind == RemoteQ2ProtocolKind::Kex || protocol.kind == RemoteQ2ProtocolKind::Rerelease,
                )
            });
            let product = systems.content_product(&selection);
            options.q2_protocol = Some(protocol);
            options.remote_content = Some(selection);
            options.product = product;
        }
        let unified = recording.is_none() && options.network.kind == RemoteNetworkKind::UnifiedClient;
        let selected: Option<String> = match recording.as_ref() {
            Some(RemoteRecordingLaunch::Gtv(_)) => Some("q2".to_string()),
            Some(RemoteRecordingLaunch::Recorded(playback)) => Some(match playback.kind {
                RemoteDemoKind::Qw => "qw".to_string(),
                RemoteDemoKind::Q1 => "q1".to_string(),
                RemoteDemoKind::Q2 => "q2".to_string(),
                RemoteDemoKind::Q3 => "q3".to_string(),
            }),
            None => {
                if unified {
                    systems
                        .product_expectation(&options.product)
                        .map(|product| product.family)
                } else {
                    None
                }
            }
        };
        let qw = match selected.as_deref() {
            None => options.network.kind == RemoteNetworkKind::QwClient,
            Some(selected) => selected == "qw",
        };
        let q1 = match selected.as_deref() {
            None => options.network.kind == RemoteNetworkKind::Q1Client || qw,
            Some(selected) => selected == "q1" || qw,
        };
        let q3 = match selected.as_deref() {
            None => options.network.kind == RemoteNetworkKind::Q3Client,
            Some(selected) => selected == "q3",
        };
        if !unified && selected.is_none() && !q1 && !q3 && options.network.kind != RemoteNetworkKind::Q2Client {
            return Err(fail(
                systems,
                RemoteApplicationError::Message("RemoteApplication requires a native connect address".to_string()),
            ));
        }
        let family = if q1 {
            RemoteFamily::Q1
        } else if q3 {
            RemoteFamily::Q3
        } else {
            RemoteFamily::Q2
        };
        let family = if qw && matches!(family, RemoteFamily::Q1) {
            RemoteFamily::Qw
        } else {
            family
        };
        if !unified && !q1 && !q3 && !["male", "female", "cyborg"].contains(&options.character_model.as_str()) {
            return Err(fail(
                systems,
                RemoteApplicationError::Message(
                    "Remote Q2 character selection requires an installed male, female or cyborg player appearance"
                        .to_string(),
                ),
            ));
        }
        let default_port = if unified {
            27960
        } else if qw {
            27500
        } else if q1 {
            26000
        } else if q3 {
            27960
        } else if options
            .q2_protocol
            .is_some_and(|protocol| protocol.kind == RemoteQ2ProtocolKind::Kex)
        {
            5069
        } else {
            27910
        };
        let address = if recording.is_some() {
            None
        } else {
            match options.network.kind {
                RemoteNetworkKind::QwClient
                | RemoteNetworkKind::Q1Client
                | RemoteNetworkKind::Q2Client
                | RemoteNetworkKind::Q3Client
                | RemoteNetworkKind::UnifiedClient => {
                    match systems.resolve_address(&options.network.remote, default_port, family) {
                        Ok(address) => Some(address),
                        Err(error) => return Err(fail(systems, RemoteApplicationError::Message(error))),
                    }
                }
                _ => None,
            }
        };
        if recording.is_none() && address.is_none() {
            return Err(fail(
                systems,
                RemoteApplicationError::Message("Missing remote address".to_string()),
            ));
        }
        if let Some(address) = address.as_ref() {
            if qw && address.ipx {
                return Err(fail(
                    systems,
                    RemoteApplicationError::Message("QuakeWorld requires UDP".to_string()),
                ));
            }
            if q3 && !address.ipv4 {
                return Err(fail(
                    systems,
                    RemoteApplicationError::Message("Native Q3 remote requires IPv4".to_string()),
                ));
            }
        }
        if let Some(q3_product) = systems.open_content_q3_product(&options) {
            options.q3_product = Some(q3_product);
        }
        match systems.resolve_movement(&options) {
            Ok(resolved) => options = resolved,
            Err(error) => return Err(fail(systems, RemoteApplicationError::Message(error))),
        }
        let family_name = match family {
            RemoteFamily::Q1 | RemoteFamily::Qw => "q1",
            RemoteFamily::Q2 => "q2",
            RemoteFamily::Q3 => "q3",
        };
        if options.dedicated
            || options.seats < 1
            || options.seats > 4
            || !unified && (options.movement != family_name || options.character != family_name)
        {
            return Err(fail(
                systems,
                RemoteApplicationError::Message(format!(
                    "Native {family_name} remote play requires one to four graphical seats with matching movement and character providers"
                )),
            ));
        }
        let product = systems.product_expectation(&options.product);
        let product_matches = match product.as_ref() {
            None => false,
            Some(product) => {
                let q1_product_ok = !q1
                    || options.product == "q1-classic-id1"
                    || (qw
                        && options
                            .remote_content
                            .as_ref()
                            .is_some_and(|selection| options.product == systems.content_product(selection)));
                let q3_product_ok = !q3
                    || options
                        .remote_content
                        .as_ref()
                        .is_some_and(|selection| options.product == systems.content_product(selection));
                product.family == family_name
                    && !(product.rerelease && family != RemoteFamily::Q2)
                    && q1_product_ok
                    && q3_product_ok
            }
        };
        if !unified && !product_matches {
            return Err(fail(
                systems,
                RemoteApplicationError::Message(
                    "Remote application requires content matching the selected native protocol".to_string(),
                ),
            ));
        }
        let session = systems.session_id();
        let seat_id = systems.primary_seat_id();
        let source_kind = match recording.as_ref() {
            None => RemoteSourceKind::Live,
            Some(RemoteRecordingLaunch::Recorded(_)) => RemoteSourceKind::Recorded,
            Some(RemoteRecordingLaunch::Gtv(_)) => RemoteSourceKind::Gtv,
        };
        let handles = match systems.finish_open(&OpenRequest {
            options: options.clone(),
            borrowed: ownership == RemoteOwnership::Borrowed,
            recording: recording.clone(),
            address: address.clone(),
        }) {
            Ok(handles) => handles,
            Err(error) => {
                let mut failures = vec![error.clone()];
                failures.extend(systems.abort_open("finish"));
                if failures.len() > 1 {
                    return Err(fail(
                        systems,
                        RemoteApplicationError::Aggregate {
                            message: "Remote preparation and resource cleanup failed".to_string(),
                            errors: failures,
                        },
                    ));
                }
                return Err(fail(systems, RemoteApplicationError::Message(error)));
            }
        };
        let mut application = Self {
            options,
            family,
            systems,
            ownership,
            source_kind,
            watch_time_ms: None,
            session,
            seat_id,
            peers: Vec::new(),
            primary_peer: None,
            next_peer_id: 1,
            commands: Vec::new(),
            qports: HashSet::new(),
            initial_qport: None,
            stopping: false,
            closing: false,
            closed: false,
            close_done: false,
            stepping: false,
            frames: 0,
            elapsed_ms: 0.0,
            cinematic: None,
            cinematic_request: 0,
            source_events: Vec::new(),
            unhandled_effects: Vec::new(),
            reported_effect_gaps: HashSet::new(),
            client_inputs: Vec::new(),
            world_load_generation: 0,
            peer_admission: None,
            pending_advance: None,
            reset_frame_elapsed: false,
            configuration: None,
            configuration_published: false,
            initial_userinfo: None,
            qw_all_skins: String::new(),
            primary_view_current: true,
        };
        application.initial_qport = Some(application.allocate_qport());
        let _ = handles;
        Ok(application)
    }

    /// Launch options (donor `options` getter).
    pub fn options(&self) -> &RemoteApplicationOptions {
        &self.options
    }

    /// Whether the application has finished (donor `finished` getter).
    pub fn finished(&self) -> bool {
        self.stopping || self.closed || self.options.frame_limit.is_some_and(|limit| self.frames >= limit)
    }

    /// Whether client commands are blocked (donor `clientCommandsBlocked` getter).
    pub fn client_commands_blocked(&mut self) -> bool {
        self.systems.client_commands_blocked()
    }

    /// Primary seat id.
    pub fn seat_id(&self) -> &SeatId {
        &self.seat_id
    }

    /// Frames presented (donor `frameCount` getter).
    pub fn frame_count(&self) -> u64 {
        self.frames
    }

    /// Presentation time in milliseconds (donor `timeMilliseconds` getter).
    pub fn time_milliseconds(&self) -> f64 {
        if self.source_kind == RemoteSourceKind::Gtv {
            self.watch_time_ms.unwrap_or(self.elapsed_ms)
        } else {
            self.elapsed_ms
        }
    }

    /// Current network phase (donor `networkPhase` getter).
    pub fn network_phase(&mut self) -> RemoteNetworkPhase {
        self.systems.network_phase()
    }

    /// Network address key (donor `networkAddress` getter).
    pub fn network_address_key(&self) -> Result<String, RemoteApplicationError> {
        if self.source_kind != RemoteSourceKind::Live {
            return Err(RemoteApplicationError::Message(
                "Recorded source has no network address".to_string(),
            ));
        }
        Ok(self.options.network.remote.clone())
    }

    /// Presentation events for the frame (donor `presentationEvents` getter).
    pub fn presentation_events(&self) -> &[RemotePresentationEvent] {
        &self.source_events
    }

    /// Unhandled effects for the frame (donor `unhandledPresentationEffects` getter).
    pub fn unhandled_presentation_effects(&self) -> &[RemoteUnhandledEffect] {
        &self.unhandled_effects
    }

    /// Allocate a qport (donor `allocateQport`).
    pub fn allocate_qport(&mut self) -> u16 {
        loop {
            let qport = QPORT_BASE + (self.systems.random_qport_offset() % QPORT_SPAN);
            if self.qports.insert(qport) {
                return qport;
            }
        }
    }

    /// Release a qport.
    pub fn release_qport(&mut self, qport: u16) {
        self.qports.remove(&qport);
    }

    /// Start a cinematic (donor `startCinematic`, sync fold).
    pub fn start_cinematic(&mut self, request: RemoteCinematicRequest) -> Result<(), RemoteApplicationError> {
        self.stop_cinematic()?;
        let ticket = self.cinematic_request;
        let generation = self.world_load_generation;
        let handle = self
            .systems
            .open_movie(&request)
            .map_err(RemoteApplicationError::Message)?;
        if self.closed || self.closing || self.stopping || generation != self.world_load_generation {
            let failures = self.systems.close_movie(handle);
            if !failures.is_empty() {
                return Err(RemoteApplicationError::Aggregate {
                    message: "Cinematic preparation and cleanup failed".to_string(),
                    errors: failures,
                });
            }
            return Err(RemoteApplicationError::Message(
                "Cinematic belongs to a retired remote request".to_string(),
            ));
        }
        self.activate_cinematic(RemoteActiveCinematic {
            ticket,
            handle,
            current: true,
        })?;
        Ok(())
    }

    /// Activate a prepared cinematic (donor `activateCinematic`).
    pub fn activate_cinematic(&mut self, movie: RemoteActiveCinematic) -> Result<(), RemoteApplicationError> {
        if !movie.current {
            let failures = self.systems.close_movie(movie.handle);
            if !failures.is_empty() {
                return Err(RemoteApplicationError::Aggregate {
                    message: "Cinematic retirement failed".to_string(),
                    errors: failures,
                });
            }
            return Ok(());
        }
        if let Some(previous) = self.cinematic.take() {
            if previous.handle != movie.handle {
                let failures = self.systems.close_movie(previous.handle);
                if !failures.is_empty() {
                    return Err(RemoteApplicationError::Aggregate {
                        message: "Cinematic retirement failed".to_string(),
                        errors: failures,
                    });
                }
            }
        }
        if let Err(error) = self.systems.activate_movie(movie.handle) {
            let mut failures = vec![error];
            failures.extend(self.systems.close_movie(movie.handle));
            if failures.len() > 1 {
                return Err(RemoteApplicationError::Aggregate {
                    message: "Cinematic activation and cleanup failed".to_string(),
                    errors: failures,
                });
            }
            return Err(RemoteApplicationError::Message(failures.remove(0)));
        }
        self.cinematic = Some(movie);
        self.reset_frame_elapsed = true;
        Ok(())
    }

    /// Stop the active cinematic (donor `stopCinematic`).
    pub fn stop_cinematic(&mut self) -> Result<(), RemoteApplicationError> {
        self.cinematic_request += 1;
        if let Some(current) = self.cinematic.take() {
            let failures = self.systems.close_movie(current.handle);
            if !failures.is_empty() {
                return Err(RemoteApplicationError::Aggregate {
                    message: "Cinematic retirement failed".to_string(),
                    errors: failures,
                });
            }
        }
        Ok(())
    }

    /// Complete the active cinematic (donor `completeCinematic`).
    pub fn complete_cinematic(&mut self, ticket: u64) -> Result<(), RemoteApplicationError> {
        let current = match self.cinematic {
            Some(movie) if movie.ticket == ticket && movie.current => movie,
            _ => return Ok(()),
        };
        self.cinematic = None;
        let failures = self.systems.close_movie(current.handle);
        if !failures.is_empty() {
            return Err(RemoteApplicationError::Aggregate {
                message: "Cinematic retirement failed".to_string(),
                errors: failures,
            });
        }
        if self.cinematic_request == ticket {
            self.cinematic_request += 1;
            for peer in &mut self.peers {
                peer.cinematic_ended = false;
            }
        }
        Ok(())
    }
}

impl RemoteApplication {
    /// Route an input event (donor `input`).
    pub fn input(&mut self, event: u32) -> Result<bool, RemoteApplicationError> {
        if self.closed || self.closing {
            return Err(RemoteApplicationError::Message(
                "Remote application is closed".to_string(),
            ));
        }
        if let Some(movie) = self.cinematic {
            return Ok(self.systems.movie_input(movie.handle, event));
        }
        Ok(self.systems.controls_input(event))
    }

    /// Queue a remote command (donor `queueCommand`).
    pub fn queue_command(
        &mut self,
        name: &str,
        args: &[String],
        seat: Option<SeatId>,
        source: Option<CommandContext>,
    ) -> Result<(), RemoteApplicationError> {
        if self.closed || self.closing {
            return Err(RemoteApplicationError::Message(
                "Remote application is closed".to_string(),
            ));
        }
        let mut seat = seat;
        if seat.is_none() {
            if let Some(source) = source.as_ref() {
                if let CommandOrigin::LocalSeat { seat: origin_seat, .. } = unwrap_origin(&source.origin) {
                    seat = Some(origin_seat.clone());
                }
            }
        }
        self.commands.push(RemoteCommand {
            name: name.to_string(),
            args: args.to_vec(),
            seat,
            source,
        });
        Ok(())
    }

    /// Dispatch queued commands (donor `dispatchCommands`, sync fold).
    pub fn dispatch_commands(&mut self) -> Result<(), RemoteApplicationError> {
        let pending = std::mem::take(&mut self.commands);
        let mut index = 0;
        while index < pending.len() {
            if self.systems.client_commands_blocked() {
                let mut rest = pending[index..].to_vec();
                rest.extend(std::mem::take(&mut self.commands));
                self.commands = rest;
                return Ok(());
            }
            let command = &pending[index];
            let source = command
                .source
                .clone()
                .unwrap_or(CommandContext::new(self.session.clone(), CommandOrigin::LocalConsole));
            if self.ownership == RemoteOwnership::Borrowed {
                self.systems
                    .dispatch_borrowed(&command.name, &command.args, command.seat.as_ref(), &source)
                    .map_err(RemoteApplicationError::Message)?;
            } else {
                self.execute_remote_command(command.clone())?;
            }
            index += 1;
        }
        Ok(())
    }

    /// Flush client commands, publishing requested peer admissions first
    /// (donor `flushClientCommands`, sync fold).
    pub fn flush_client_commands(&mut self) -> Result<(), RemoteApplicationError> {
        if self
            .peer_admission
            .as_ref()
            .is_some_and(|admission| admission.phase == PeerAdmissionPhase::Requested)
        {
            let admission = self.peer_admission.clone().expect("checked admission");
            self.publish_peer_configuration(&admission)?;
        }
        self.dispatch_commands()
    }

    /// Advance client startup (donor `advanceClientStartup`, sync fold).
    pub fn advance_client_startup(&mut self) -> Result<bool, RemoteApplicationError> {
        self.systems
            .advance_client_startup()
            .map_err(RemoteApplicationError::Message)
    }

    /// Pump client input (donor `pumpClientInput`).
    pub fn pump_client_input(&mut self) {
        self.systems.pump_client_input();
    }

    /// Execute a remote command (donor `executeRemoteCommand`, sync fold).
    ///
    /// Failures print instead of propagating, matching the donor catch.
    pub fn execute_remote_command(&mut self, command: RemoteCommand) -> Result<(), RemoteApplicationError> {
        let context = command.source.clone();
        let print = |systems: &mut Box<dyn RemoteApplicationSystems>, text: &str| {
            systems.print(text);
        };
        let result = self.execute_remote_command_inner(&command);
        if let Err(error) = result {
            let message = match error {
                RemoteApplicationError::Message(message) => message,
                RemoteApplicationError::Aggregate { message, errors } => {
                    format!("{message}: {}", errors.join("; "))
                }
            };
            let _ = context;
            print(&mut self.systems, &format!("{message}\n"));
        }
        Ok(())
    }

    /// Inner remote command dispatch (donor `executeRemoteCommand` body).
    fn execute_remote_command_inner(&mut self, command: &RemoteCommand) -> Result<(), RemoteApplicationError> {
        if let Some(source) = command.source.as_ref() {
            if source.session != self.session {
                return Err(RemoteApplicationError::Message(
                    "Remote command belongs to another session".to_string(),
                ));
            }
            match unwrap_origin(&source.origin) {
                CommandOrigin::RemoteClient { .. } => {
                    return Err(RemoteApplicationError::Message(
                        "Remote gameplay commands require a local client".to_string(),
                    ));
                }
                CommandOrigin::LocalSeat { seat, client } => {
                    let known = self.configuration.as_ref().is_some_and(|configuration| {
                        configuration
                            .seats
                            .iter()
                            .any(|known| &known.id == seat && &known.client == client)
                    });
                    if !known {
                        return Err(RemoteApplicationError::Message(
                            "Remote command belongs to a retired local client".to_string(),
                        ));
                    }
                }
                _ => {}
            }
        }
        if let Some(seat) = command.seat.as_ref() {
            let known = self
                .configuration
                .as_ref()
                .is_some_and(|configuration| configuration.seats.iter().any(|known| &known.id == seat));
            if !known {
                return Err(RemoteApplicationError::Message(
                    "Remote command belongs to an inactive local seat".to_string(),
                ));
            }
        }
        let print = |systems: &mut Box<dyn RemoteApplicationSystems>, text: &str| systems.print(text);
        if command.name == "local_join" {
            if !command.args.is_empty() {
                return Err(RemoteApplicationError::Message("Usage: local_join".to_string()));
            }
            return self.join_remote_seat();
        }
        if command.name == "local_drop" {
            let index = if command.args.is_empty() {
                command.seat.as_ref().map(|seat| i64::from(seat.index()))
            } else {
                command
                    .args
                    .first()
                    .map(|arg| arg.parse::<i64>().map(|number| number - 1).unwrap_or(-1))
            };
            match index {
                Some(index) if command.args.len() <= 1 && index >= 0 && index <= u32::MAX as i64 => {
                    return self.drop_remote_seat(index as u32);
                }
                _ => {
                    return Err(RemoteApplicationError::Message(
                        "Usage: local_drop [player number]".to_string(),
                    ));
                }
            }
        }
        if command.name == "color" && self.family == RemoteFamily::Qw {
            let target = command.seat.clone();
            if command.args.is_empty() {
                let (top, bottom) = self.qw_colors(target.as_ref())?;
                print(&mut self.systems, &format!("\"color\" is \"{top} {bottom}\"\n"));
                return Ok(());
            }
            let top = remote_atoi(command.args.first().map(String::as_str).unwrap_or("")) & 15;
            let bottom = remote_atoi(
                command
                    .args
                    .get(1)
                    .map(String::as_str)
                    .unwrap_or(command.args.first().map(String::as_str).unwrap_or("")),
            ) & 15;
            let top = top.min(13).to_string();
            let bottom = bottom.min(13).to_string();
            self.set_qw_colors(target.as_ref(), &top, &bottom)?;
            return Ok(());
        }
        if command.name == "in_restart" || command.name == "midiinfo" {
            if !self.systems.has_input() {
                print(&mut self.systems, "Input devices are not active before signon.\n");
                return Ok(());
            }
            if command.name == "midiinfo" {
                self.systems.input_devices_info();
            } else {
                self.systems.restart_input_devices();
            }
            return Ok(());
        }
        if command.name == "record"
            || command.name == "stop"
            || command.name == "stoprecord"
            || command.name == "rerecord"
        {
            let context = command
                .source
                .clone()
                .unwrap_or(CommandContext::new(self.session.clone(), CommandOrigin::LocalConsole));
            if command.name == "stop" || command.name == "stoprecord" {
                return self
                    .systems
                    .recording_command("stop", None, &context)
                    .map_err(RemoteApplicationError::Message);
            }
            let name = command.args.first();
            if command.args.len() > 1 || command.name == "rerecord" && name.is_none() {
                return Err(RemoteApplicationError::Message(format!(
                    "Usage: {} <name>",
                    command.name
                )));
            }
            return self
                .systems
                .recording_command(&command.name, name.map(String::as_str), &context)
                .map_err(RemoteApplicationError::Message);
        }
        if command.name == "cinematic" {
            let name = command.args.first();
            let mode = command.args.get(1);
            match (name, command.args.len()) {
                (Some(name), 1 | 2) => {
                    let request = RemoteCinematicRequest {
                        name: name.clone(),
                        loop_play: mode.is_some_and(|mode| mode == "2" || mode == "loop"),
                        hold: mode.is_some_and(|mode| mode == "1" || mode == "hold"),
                        silent: false,
                    };
                    return self.start_cinematic(request);
                }
                _ => {
                    return Err(RemoteApplicationError::Message(
                        "Usage: cinematic <name> [loop|hold]".to_string(),
                    ));
                }
            }
        }
        if command.name == "cinematicpause" || command.name == "stopcinematic" {
            let Some(movie) = self.cinematic else {
                return Err(RemoteApplicationError::Message("No cinematic is playing".to_string()));
            };
            if command.name == "stopcinematic" {
                return self.stop_cinematic();
            }
            let paused = !self.systems.movie_paused(movie.handle);
            self.systems.movie_pause(movie.handle, paused);
            return Ok(());
        }
        if self.source_kind != RemoteSourceKind::Live && (command.name == "follow" || command.name == "chase") {
            let valid = command.args.len() == 1
                && command
                    .args
                    .first()
                    .is_some_and(|selected| !selected.is_empty() && selected.chars().all(|c| c.is_ascii_digit()));
            if !valid {
                return Err(RemoteApplicationError::Message(format!(
                    "Usage: {} <source player ID>; Recorded free camera is not available",
                    command.name
                )));
            }
            return self
                .systems
                .recording_command(
                    "select-player",
                    command.args.first().map(String::as_str),
                    &CommandContext::new(self.session.clone(), CommandOrigin::LocalConsole),
                )
                .map_err(RemoteApplicationError::Message);
        }
        if command.name == "demopause" {
            if self.source_kind != RemoteSourceKind::Recorded {
                return Err(RemoteApplicationError::Message(
                    "Demo pause requires active playback".to_string(),
                ));
            }
            return self
                .systems
                .recording_command(
                    "demopause",
                    None,
                    &CommandContext::new(self.session.clone(), CommandOrigin::LocalConsole),
                )
                .map_err(RemoteApplicationError::Message);
        }
        if command.name == "downloadstatus" {
            let progress = self.download_progress_for(command.seat.as_ref());
            if progress.is_empty() {
                print(&mut self.systems, "No downloads are pending.\n");
            }
            for file in &progress {
                let total = file
                    .total
                    .map(|total| total.to_string())
                    .unwrap_or_else(|| "?".to_string());
                print(
                    &mut self.systems,
                    &format!(
                        "{}: {} {}/{} bytes ({})\n",
                        file.path, file.phase, file.received, total, file.transport
                    ),
                );
            }
            return Ok(());
        }
        if command.name == "stopdownload" {
            self.cancel_downloads(command.seat.as_ref());
            print(&mut self.systems, "Downloads paused. Use retrydownload to resume.\n");
            return Ok(());
        }
        if command.name == "retrydownload" {
            return self.retry_downloads(command.seat.as_ref());
        }
        if command.name == "quit" {
            self.request_quit();
            return Ok(());
        }
        if command.name == "disconnect" {
            let peer = command.seat.as_ref().and_then(|seat| {
                self.peers
                    .iter()
                    .find(|peer| !peer.closed && peer.seat == *seat)
                    .map(|peer| peer.id)
            });
            match peer {
                None => self.request_quit(),
                Some(id) => self.retire_remote_seat(id, true)?,
            }
            return Ok(());
        }
        if self.browser_command(&command.name, &command.args)? {
            return Ok(());
        }
        if self.family == RemoteFamily::Qw && (command.name == "skins" || command.name == "allskins") {
            if command.name == "allskins" {
                self.qw_all_skins = command.args.first().cloned().unwrap_or_default();
            }
            let skins = self.qw_all_skins.clone();
            return self
                .systems
                .recording_command(
                    "refresh-skins",
                    Some(&skins),
                    &CommandContext::new(self.session.clone(), CommandOrigin::LocalConsole),
                )
                .map_err(RemoteApplicationError::Message);
        }
        if self
            .systems
            .audio_command(&command.name, &command.args, command.seat.as_ref())
            .map_err(RemoteApplicationError::Message)?
        {
            return Ok(());
        }
        if ["map", "save", "load"].contains(&command.name.as_str()) {
            return Err(RemoteApplicationError::Message(format!(
                "{} requires the authoritative server console",
                command.name
            )));
        }
        let players = self.systems.local_players();
        let player = players
            .iter()
            .find(|player| command.seat.as_ref().is_none_or(|seat| &player.seat == seat));
        let Some(player) = player else {
            return Err(RemoteApplicationError::Message(
                "Remote command requires a connected local player".to_string(),
            ));
        };
        if self.systems.network_phase() != RemoteNetworkPhase::Active {
            return Err(RemoteApplicationError::Message(
                "Remote command requires a connected local player".to_string(),
            ));
        }
        let actor = player.actor.clone();
        if command.name == "centerview" {
            self.systems.centerview(&actor);
            return Ok(());
        }
        if command.name == "use" {
            if let Some(item) = command.args.first() {
                if let Some(ordinal) = self.systems.use_item_ordinal(&actor, item) {
                    let argv = vec!["weapon".to_string(), ordinal.to_string()];
                    if self
                        .systems
                        .q3_command(command.seat.as_ref(), &argv)
                        .map_err(RemoteApplicationError::Message)?
                    {
                        return Ok(());
                    }
                }
            }
        }
        let mut argv = vec![command.name.clone()];
        argv.extend(command.args.iter().cloned());
        if self
            .systems
            .q3_command(command.seat.as_ref(), &argv)
            .map_err(RemoteApplicationError::Message)?
        {
            return Ok(());
        }
        if self.source_kind != RemoteSourceKind::Live {
            return Err(RemoteApplicationError::Message(
                "Gameplay commands require a live server".to_string(),
            ));
        }
        self.systems
            .presentation_for_actor(&actor)
            .map_err(RemoteApplicationError::Message)?;
        self.systems.player_command(&actor, &command.name, &command.args);
        Ok(())
    }

    /// Read QuakeWorld shirt/pants colors (donor `color` query).
    fn qw_colors(&mut self, seat: Option<&SeatId>) -> Result<(String, String), RemoteApplicationError> {
        let configuration = self
            .configuration
            .as_ref()
            .ok_or_else(|| RemoteApplicationError::Message("Color command has no source seat".to_string()))?;
        let cvars = match seat {
            None => &configuration.source,
            Some(seat) => configuration
                .seat_sources
                .iter()
                .find(|(id, _)| id == seat)
                .map(|(_, cvars)| cvars)
                .ok_or_else(|| RemoteApplicationError::Message("Color command has no source seat".to_string()))?,
        };
        Ok((
            cvars
                .get("topcolor")
                .map(|snapshot| snapshot.value.clone())
                .unwrap_or_default(),
            cvars
                .get("bottomcolor")
                .map(|snapshot| snapshot.value.clone())
                .unwrap_or_default(),
        ))
    }

    /// Write QuakeWorld shirt/pants colors (donor `color` set).
    fn set_qw_colors(&mut self, seat: Option<&SeatId>, top: &str, bottom: &str) -> Result<(), RemoteApplicationError> {
        let configuration = self
            .configuration
            .as_mut()
            .ok_or_else(|| RemoteApplicationError::Message("Color command has no source seat".to_string()))?;
        let cvars = match seat {
            None => &mut configuration.source,
            Some(seat) => configuration
                .seat_sources
                .iter_mut()
                .find(|(id, _)| id == seat)
                .map(|(_, cvars)| cvars)
                .ok_or_else(|| RemoteApplicationError::Message("Color command has no source seat".to_string()))?,
        };
        cvars.set("topcolor", top, true)?;
        cvars.set("bottomcolor", bottom, true)?;
        Ok(())
    }

    /// Q3 browser command dispatch (donor `browserCommand`, sync fold).
    fn browser_command(&mut self, name: &str, args: &[String]) -> Result<bool, RemoteApplicationError> {
        if !self.systems.has_q3_browser() {
            return Ok(false);
        }
        let print = |systems: &mut Box<dyn RemoteApplicationSystems>, text: &str| systems.print(text);
        match name {
            "localservers" => {
                print(&mut self.systems, "Scanning for servers on the local network...\n");
                self.systems
                    .browser_command(name, args)
                    .map_err(RemoteApplicationError::Message)?;
                Ok(true)
            }
            "globalservers" => {
                if args.len() < 2 {
                    print(
                        &mut self.systems,
                        "usage: globalservers <master# 0-1> <protocol> [keywords]\n",
                    );
                    return Ok(true);
                }
                print(&mut self.systems, "Requesting servers from the master...\n");
                self.systems
                    .browser_command(name, args)
                    .map_err(RemoteApplicationError::Message)?;
                Ok(true)
            }
            "ping" => {
                if args.len() != 1 {
                    print(&mut self.systems, "usage: ping [server]\n");
                } else {
                    self.systems
                        .browser_command(name, args)
                        .map_err(RemoteApplicationError::Message)?;
                }
                Ok(true)
            }
            "serverstatus" => {
                let server = if args.len() == 1 {
                    args.first().cloned()
                } else if self.systems.network_phase() == RemoteNetworkPhase::Active
                    && self.options.network.kind == RemoteNetworkKind::Q3Client
                {
                    Some(self.options.network.remote.clone())
                } else {
                    None
                };
                match server {
                    None => print(
                        &mut self.systems,
                        "Not connected to a server.\nUsage: serverstatus [server]\n",
                    ),
                    Some(server) => {
                        self.systems
                            .browser_command(name, &[server])
                            .map_err(RemoteApplicationError::Message)?;
                    }
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// Download progress for a seat scope (donor `downloadstatus` selection).
    fn download_progress_for(&mut self, seat: Option<&SeatId>) -> Vec<RemoteDownloadProgress> {
        match seat {
            None => self.download_progress(),
            Some(seat) if *seat == self.seat_id => self.systems.primary_download_progress(),
            Some(seat) => self
                .peers
                .iter()
                .find(|peer| !peer.closed && peer.seat == *seat)
                .map(|peer| self.systems.peer_download_progress(peer.handle))
                .unwrap_or_default(),
        }
    }

    /// Aggregate download progress (donor `downloadProgress` getter).
    pub fn download_progress(&mut self) -> Vec<RemoteDownloadProgress> {
        let mut progress = self.systems.primary_download_progress();
        for peer in self.peers.clone() {
            if !peer.closed {
                progress.extend(self.systems.peer_download_progress(peer.handle));
            }
        }
        progress
    }

    /// Cancel downloads for a seat scope (donor `cancelDownloads`).
    pub fn cancel_downloads(&mut self, seat: Option<&SeatId>) {
        if seat.is_none() || seat.is_some_and(|seat| *seat == self.seat_id) {
            self.systems.cancel_primary_downloads();
        }
        for peer in self.peers.clone() {
            if !peer.closed && (seat.is_none() || seat.is_some_and(|seat| peer.seat == *seat)) {
                self.systems.cancel_peer_downloads(peer.handle);
            }
        }
    }

    /// Retry downloads for a seat scope (donor `retryDownloads`, sync fold).
    pub fn retry_downloads(&mut self, seat: Option<&SeatId>) -> Result<(), RemoteApplicationError> {
        if seat.is_none() || seat.is_some_and(|seat| *seat == self.seat_id) {
            self.systems
                .retry_primary_downloads()
                .map_err(RemoteApplicationError::Message)?;
        }
        for peer in self.peers.clone() {
            if !peer.closed && (seat.is_none() || seat.is_some_and(|seat| peer.seat == *seat)) {
                self.systems
                    .retry_peer_downloads(peer.handle)
                    .map_err(RemoteApplicationError::Message)?;
            }
        }
        Ok(())
    }

    /// Guards for recording commands (donor `prepareRecording` validation).
    pub fn check_recording(
        &mut self,
        context: &CommandContext,
        seat: Option<&SeatId>,
    ) -> Result<Option<u64>, RemoteApplicationError> {
        if context.session != self.session {
            return Err(RemoteApplicationError::Message(
                "Recording command belongs to another session".to_string(),
            ));
        }
        if self.source_kind != RemoteSourceKind::Live || self.systems.network_phase() != RemoteNetworkPhase::Active {
            return Err(RemoteApplicationError::Message(
                "Recording requires an active live connection".to_string(),
            ));
        }
        let peer = match seat {
            None => None,
            Some(seat) => self
                .peers
                .iter()
                .find(|peer| !peer.closed && peer.seat == *seat)
                .map(|peer| peer.id),
        };
        if let Some(seat) = seat {
            if self.peers.iter().any(|peer| peer.seat == *seat && peer.closed) {
                return Err(RemoteApplicationError::Message(
                    "Recording seat has no active connection".to_string(),
                ));
            }
        }
        let recorder = match peer.and_then(|id| self.peers.iter().find(|peer| peer.id == id)) {
            None => self.systems.primary_has_recorder(),
            Some(peer) => self.systems.peer_has_recorder(peer.handle),
        };
        if !recorder {
            return Err(RemoteApplicationError::Message(
                "The selected native protocol has no demo recorder".to_string(),
            ));
        }
        Ok(peer)
    }

    /// Attach a signon recording sink (donor `attachConnectingRecording`).
    pub fn attach_signon_recording(&mut self) -> Result<SignonRecordingAttachment, RemoteApplicationError> {
        if self.closed
            || self.closing
            || self.source_kind != RemoteSourceKind::Live
            || !self.systems.is_live_quakeworld()
        {
            return Err(RemoteApplicationError::Message(
                "Signon recording requires a live QuakeWorld connection".to_string(),
            ));
        }
        let attachment = self
            .systems
            .attach_signon_recording()
            .map_err(RemoteApplicationError::Message)?;
        Ok(SignonRecordingAttachment { attachment })
    }

    /// Detach a signon recording sink.
    pub fn detach_signon_recording(&mut self, attachment: SignonRecordingAttachment) {
        self.systems.detach_signon_recording(attachment.attachment);
    }

    /// Request shutdown (donor `requestQuit`).
    pub fn request_quit(&mut self) {
        self.stopping = true;
    }

    /// Capture client input for deferred delivery (donor `clientInput` hook).
    pub fn capture_client_input(&mut self, seat: &SeatId, event: u32) -> bool {
        match self.systems.input_client_for_seat(seat) {
            Some(client) => {
                self.client_inputs.push(PendingClientInput {
                    generation: self.world_load_generation,
                    client,
                    event,
                    seat: seat.clone(),
                });
                true
            }
            None => false,
        }
    }
}

/// Attached signon recording (donor `attachConnectingRecording` disposer).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignonRecordingAttachment {
    /// Systems attachment handle.
    pub attachment: u64,
}

impl RemoteApplication {
    /// Select remote content mounts (donor `selectRemoteContent`, sync fold).
    ///
    /// Returns whether the product changed (and thus whether the caller must
    /// publish a peer configuration or suspend for one).
    pub fn select_remote_content(&mut self, selection: RemoteContentSelection) -> Result<bool, RemoteApplicationError> {
        if self.closed || self.closing {
            return Err(RemoteApplicationError::Message(
                "Remote directory selection was cancelled".to_string(),
            ));
        }
        self.world_load_generation += 1;
        let changed = self
            .systems
            .select_content_mounts(&selection)
            .map_err(RemoteApplicationError::Message)?;
        if !changed {
            return Ok(false);
        }
        let product = self.systems.content_product(&selection);
        let mut options = self.options.clone();
        options.product = product;
        options.remote_content = Some(selection.clone());
        if self.primary_peer.is_some() {
            self.publish_peer_configuration(&PeerDirectoryAdmission {
                phase: PeerAdmissionPhase::Requested,
                options,
                selection,
                suspended: false,
            })?;
            return Ok(true);
        }
        if self.peer_admission.is_some() || self.pending_advance.is_none() {
            return Err(RemoteApplicationError::Message(
                "Peer directory admission has no active source frame".to_string(),
            ));
        }
        self.peer_admission = Some(PeerDirectoryAdmission {
            phase: PeerAdmissionPhase::Requested,
            options,
            selection,
            suspended: true,
        });
        if let Some(pending) = self.pending_advance.as_mut() {
            pending.boundary = true;
        }
        Ok(true)
    }

    /// Refresh the selected remote content (donor `refreshRemoteContent`).
    pub fn refresh_remote_content(&mut self) -> Result<bool, RemoteApplicationError> {
        let selection = self.options.remote_content.clone().ok_or_else(|| {
            RemoteApplicationError::Message("Remote package refresh has no selected directory".to_string())
        })?;
        self.select_remote_content(selection)
    }

    /// Publish a requested peer configuration (donor `publishPeerConfiguration`, sync fold).
    pub fn publish_peer_configuration(
        &mut self,
        admission: &PeerDirectoryAdmission,
    ) -> Result<(), RemoteApplicationError> {
        let result = self
            .systems
            .publish_peer_configuration(&admission.options)
            .map_err(RemoteApplicationError::Message);
        match result {
            Ok(()) => {
                self.options = admission.options.clone();
                self.reset_frame_elapsed = true;
                if let Some(pending) = self.peer_admission.as_mut() {
                    pending.phase = PeerAdmissionPhase::Published;
                    pending.suspended = false;
                }
                Ok(())
            }
            Err(error) => {
                self.peer_admission = None;
                Err(error)
            }
        }
    }

    /// Resume a suspended peer admission after publication (donor suspension resume).
    pub fn resume_peer_admission(&mut self) {
        if self
            .peer_admission
            .as_ref()
            .is_some_and(|admission| admission.phase == PeerAdmissionPhase::Published)
        {
            self.peer_admission = None;
        }
    }

    /// Advance the source unless a boundary or requested admission intervenes
    /// (donor `advanceSource`, sync fold).
    pub fn advance_source(&mut self, op: SourceAdvanceOp) -> Result<bool, RemoteApplicationError> {
        if self.pending_advance.is_none() {
            self.pending_advance = Some(PendingAdvance { boundary: false });
        } else if self.peer_admission.is_some() {
            let requested = self
                .peer_admission
                .as_ref()
                .is_some_and(|admission| admission.phase == PeerAdmissionPhase::Requested);
            if requested {
                return Ok(false);
            }
            self.peer_admission = None;
            if let Some(pending) = self.pending_advance.as_mut() {
                pending.boundary = false;
            }
        }
        let outcome = self
            .systems
            .execute_advance(op)
            .map_err(RemoteApplicationError::Message)?;
        for selection in outcome.content_switches {
            self.select_remote_content(selection)?;
            if self.pending_advance.is_some_and(|pending| pending.boundary) {
                return Ok(false);
            }
        }
        if self.pending_advance.is_some_and(|pending| pending.boundary) {
            return Ok(false);
        }
        self.pending_advance = None;
        Ok(true)
    }

    /// Whether a peer world matches the primary view (donor `peerWorldCurrent`).
    pub fn peer_world_current(&self, id: u64) -> bool {
        self.peers
            .iter()
            .find(|peer| peer.id == id)
            .is_some_and(|peer| peer.map.as_deref() == Some(self.options.map.as_str()) && peer.has_loaded_content)
    }

    /// Prepare remote seat peers for extra seats (donor `prepareRemoteSeats`, sync fold).
    pub fn prepare_remote_seats(&mut self) -> Result<(), RemoteApplicationError> {
        if self.source_kind != RemoteSourceKind::Live {
            return Ok(());
        }
        let seats = self
            .configuration
            .as_ref()
            .map(|configuration| configuration.seats.len())
            .unwrap_or(1);
        if seats <= 1 {
            return Ok(());
        }
        match self.options.network.kind {
            RemoteNetworkKind::Q1Client
            | RemoteNetworkKind::QwClient
            | RemoteNetworkKind::Q2Client
            | RemoteNetworkKind::Q3Client
            | RemoteNetworkKind::UnifiedClient => {}
            _ => {
                return Err(RemoteApplicationError::Message(
                    "Remote seat requires a native server address".to_string(),
                ));
            }
        }
        let default_port = match self.family {
            RemoteFamily::Q1 => 26000,
            RemoteFamily::Qw => 27500,
            RemoteFamily::Q3 => 27960,
            RemoteFamily::Q2 => {
                if self.options.network.kind == RemoteNetworkKind::UnifiedClient {
                    27960
                } else if self
                    .options
                    .q2_protocol
                    .is_some_and(|protocol| protocol.kind == RemoteQ2ProtocolKind::Kex)
                {
                    5069
                } else {
                    27910
                }
            }
        };
        let seat_ids: Vec<SeatId> = self
            .configuration
            .as_ref()
            .map(|configuration| configuration.seats.iter().map(|seat| seat.id.clone()).collect())
            .unwrap_or_default();
        for seat in seat_ids {
            if seat == self.seat_id || self.peers.iter().any(|peer| !peer.closed && peer.seat == seat) {
                continue;
            }
            let address = self
                .systems
                .resolve_address(&self.options.network.remote, default_port, self.family)
                .map_err(RemoteApplicationError::Message)?;
            let qport = self.allocate_qport();
            match self.systems.open_peer(&seat, &address, qport) {
                Ok(handle) => {
                    let id = self.next_peer_id;
                    self.next_peer_id += 1;
                    self.peers.push(RemoteSeatPeer {
                        id,
                        seat,
                        map: None,
                        generation: 0,
                        closed: false,
                        disconnected: None,
                        userinfo: None,
                        has_content: false,
                        has_loaded_content: false,
                        has_view: false,
                        cinematic_ended: false,
                        handle,
                    });
                }
                Err(error) => {
                    self.systems.abort_peer(&seat);
                    self.release_qport(qport);
                    return Err(RemoteApplicationError::Message(error));
                }
            }
        }
        Ok(())
    }

    /// Join an additional local seat (donor `joinRemoteSeat` guards + delegation).
    pub fn join_remote_seat(&mut self) -> Result<(), RemoteApplicationError> {
        if self.source_kind != RemoteSourceKind::Live {
            return Err(RemoteApplicationError::Message(
                "Recorded playback has one recorded viewpoint".to_string(),
            ));
        }
        let seats = self
            .configuration
            .as_ref()
            .map(|configuration| configuration.seats.len())
            .unwrap_or(1);
        if seats >= 4 {
            return Err(RemoteApplicationError::Message(
                "Four local players are already connected".to_string(),
            ));
        }
        if !self.primary_view_current {
            return Err(RemoteApplicationError::Message(
                "Wait for the current player to finish connecting".to_string(),
            ));
        }
        self.systems
            .save_control_settings()
            .map_err(RemoteApplicationError::Message)?;
        let (handle, seat, client) = self.systems.join_peer_seat().map_err(RemoteApplicationError::Message)?;
        let id = self.next_peer_id;
        self.next_peer_id += 1;
        if let Some(configuration) = self.configuration.as_mut() {
            configuration.seats.push(RemoteConfiguredSeat {
                id: seat.clone(),
                client,
            });
        }
        self.peers.push(RemoteSeatPeer {
            id,
            seat,
            map: None,
            generation: 0,
            closed: false,
            disconnected: None,
            userinfo: None,
            has_content: false,
            has_loaded_content: false,
            has_view: false,
            cinematic_ended: false,
            handle,
        });
        self.options.seats += 1;
        Ok(())
    }

    /// Drop a local seat by player index (donor `dropRemoteSeat`, sync fold).
    pub fn drop_remote_seat(&mut self, index: u32) -> Result<(), RemoteApplicationError> {
        if self.source_kind != RemoteSourceKind::Live {
            return Err(RemoteApplicationError::Message(
                "Recorded playback has one recorded viewpoint".to_string(),
            ));
        }
        let seats = self
            .configuration
            .as_ref()
            .map(|configuration| configuration.seats.len())
            .unwrap_or(1);
        if seats <= 1 {
            return Err(RemoteApplicationError::Message(
                "Use disconnect to leave the last connection".to_string(),
            ));
        }
        if !self
            .configuration
            .as_ref()
            .is_some_and(|configuration| configuration.seats.iter().any(|seat| seat.id.index() == index))
            && !self.peers.iter().any(|peer| peer.seat.index() == index)
        {
            return Err(RemoteApplicationError::Message(
                "That player is not connected".to_string(),
            ));
        }
        self.systems
            .save_control_settings()
            .map_err(RemoteApplicationError::Message)?;
        let primary = self.seat_id.index() == index;
        let replacement = if primary {
            self.peers
                .iter()
                .find(|peer| {
                    !peer.closed
                        && Some(peer.id) != self.primary_peer
                        && peer.has_view
                        && peer.map.as_deref() == Some(self.options.map.as_str())
                })
                .map(|peer| peer.id)
        } else {
            None
        };
        if primary && replacement.is_none() {
            return Err(RemoteApplicationError::Message(
                "Wait for another player to finish connecting before removing this player".to_string(),
            ));
        }
        let replacement_handle =
            replacement.and_then(|id| self.peers.iter().find(|peer| peer.id == id).map(|peer| peer.handle));
        self.systems
            .drop_peer_seat(index, replacement_handle)
            .map_err(RemoteApplicationError::Message)?;
        if let Some(id) = replacement {
            self.primary_peer = Some(id);
        }
        if self.options.seats > 1 {
            self.options.seats -= 1;
        }
        if let Some(configuration) = self.configuration.as_mut() {
            configuration.seats.retain(|seat| seat.id.index() != index);
        }
        if primary && self.primary_peer.is_none() {
            if let Some(qport) = self.initial_qport.take() {
                self.release_qport(qport);
            }
        } else {
            let retire = if primary {
                self.primary_peer
            } else {
                self.peers
                    .iter()
                    .find(|peer| !peer.closed && peer.seat.index() == index)
                    .map(|peer| peer.id)
            };
            if let Some(id) = retire {
                if !(primary && Some(id) == self.primary_peer) {
                    self.retire_remote_seat(id, false)?;
                }
            }
        }
        Ok(())
    }

    /// Retire a remote seat peer (donor `retireRemoteSeat`, sync fold).
    pub fn retire_remote_seat(&mut self, id: u64, remove_input: bool) -> Result<(), RemoteApplicationError> {
        let position = self.peers.iter().position(|peer| peer.id == id);
        let Some(position) = position else {
            return Ok(());
        };
        if self.peers[position].closed {
            return Ok(());
        }
        if remove_input {
            let index = self.peers[position].seat.index();
            return self.drop_remote_seat(index);
        }
        self.peers[position].closed = true;
        let handle = self.peers[position].handle;
        let mut failures = Vec::new();
        if let Err(error) = self.systems.retire_peer_view(handle) {
            failures.push(error);
        }
        failures.extend(self.systems.close_peer(handle));
        self.peers[position].has_view = false;
        self.peers[position].has_content = false;
        self.peers[position].has_loaded_content = false;
        self.peers.remove(position);
        if failures.is_empty() {
            Ok(())
        } else {
            Err(RemoteApplicationError::Aggregate {
                message: "Remote seat retirement failed".to_string(),
                errors: failures,
            })
        }
    }

    /// Retire all peer views (donor `retireRemoteViews`, sync fold).
    pub fn retire_remote_views(&mut self) -> Result<(), RemoteApplicationError> {
        let mut failures = Vec::new();
        for peer in &mut self.peers {
            if let Err(error) = self.systems.retire_peer_view(peer.handle) {
                failures.push(error);
            }
            peer.has_view = false;
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(RemoteApplicationError::Aggregate {
                message: "Remote seat views failed to retire".to_string(),
                errors: failures,
            })
        }
    }

    /// Poll peer pumps (donor `pollRemoteSeats`).
    pub fn poll_remote_seats(&mut self, now_ms: f64) {
        for peer in self.peers.clone() {
            if !peer.closed && peer.disconnected.is_none() {
                self.systems.poll_peer(peer.handle, now_ms);
            }
        }
    }

    /// Drain ready peer publications (donor `drainSeatPublications`, sync fold).
    pub fn drain_seat_publications(&mut self) -> Result<(), RemoteApplicationError> {
        for peer in self.peers.clone() {
            self.systems
                .drain_peer(peer.handle)
                .map_err(RemoteApplicationError::Message)?;
        }
        Ok(())
    }

    /// Prepare non-primary peer frames (donor `prepareRemoteSeatFrames`, sync fold).
    pub fn prepare_remote_seat_frames(
        &mut self,
        now_ms: f64,
        wall_elapsed_ms: f64,
    ) -> Result<(), RemoteApplicationError> {
        for peer in self.peers.clone() {
            if peer.closed || Some(peer.id) == self.primary_peer {
                continue;
            }
            if let Some(reason) = self.systems.take_peer_disconnected(peer.handle) {
                let index = peer.seat.index() + 1;
                self.systems.print(&format!("Player {index}: {reason}\n"));
                self.retire_remote_seat(peer.id, true)?;
                continue;
            }
            if !self.systems.peer_active(peer.handle) || !self.peer_world_current(peer.id) {
                continue;
            }
            if let Some(info) = self.systems.peer_remote_userinfo(peer.handle) {
                let current = self
                    .peers
                    .iter()
                    .find(|candidate| candidate.id == peer.id)
                    .and_then(|candidate| candidate.userinfo.clone());
                if current.as_deref() != Some(info.as_str()) {
                    self.systems.set_peer_userinfo(peer.handle, &info);
                    if let Some(stored) = self.peers.iter_mut().find(|candidate| candidate.id == peer.id) {
                        stored.userinfo = Some(info);
                    }
                }
            }
            self.systems.sample_peer_presentation(peer.handle, now_ms);
            self.systems
                .prepare_peer_skins(peer.handle)
                .map_err(RemoteApplicationError::Message)?;
            if self.systems.peer_needs_bind(peer.handle) {
                self.systems
                    .bind_peer_seat(peer.handle)
                    .map_err(RemoteApplicationError::Message)?;
                if let Some(stored) = self.peers.iter_mut().find(|candidate| candidate.id == peer.id) {
                    stored.has_view = true;
                }
            }
            self.systems
                .submit_peer_frame(peer.handle, now_ms, wall_elapsed_ms)
                .map_err(RemoteApplicationError::Message)?;
        }
        self.poll_remote_seats(now_ms);
        Ok(())
    }
}

impl RemoteApplication {
    /// Bind the primary seat presentation (donor `bindSeat` guards + delegation).
    pub fn bind_seat(&mut self) -> Result<(), RemoteApplicationError> {
        match self.systems.bind_primary_seat() {
            Ok(_) => Ok(()),
            Err(failure) => {
                if failure.published {
                    let mut failures = vec![failure.message];
                    if let Err(error) = self.close() {
                        match error {
                            RemoteApplicationError::Message(message) => failures.push(message),
                            RemoteApplicationError::Aggregate { errors, .. } => failures.extend(errors),
                        }
                    }
                    if failures.len() > 1 {
                        return Err(RemoteApplicationError::Aggregate {
                            message: "Remote seat publication and shutdown failed".to_string(),
                            errors: failures,
                        });
                    }
                    return Err(RemoteApplicationError::Message(failures.remove(0)));
                }
                Err(RemoteApplicationError::Message(failure.message))
            }
        }
    }

    /// Present one application frame (donor `step`, sync fold).
    pub fn step(&mut self, elapsed_ms: f64) -> Result<bool, RemoteApplicationError> {
        if self.closed || self.closing {
            return Err(RemoteApplicationError::Message(
                "Remote application is closed".to_string(),
            ));
        }
        if self.stepping {
            return Err(RemoteApplicationError::Message(
                "Remote application step is already in progress".to_string(),
            ));
        }
        if !elapsed_ms.is_finite() || elapsed_ms <= 0.0 {
            return Err(RemoteApplicationError::Message(
                "Remote application step requires positive elapsed milliseconds".to_string(),
            ));
        }
        if self.ownership == RemoteOwnership::Owned {
            self.systems
                .drain_video_restart()
                .map_err(RemoteApplicationError::Message)?;
        }
        self.stepping = true;
        let result = self.step_inner(elapsed_ms);
        self.stepping = false;
        match result {
            Ok(presented) => Ok(presented),
            Err(error) => {
                let _ = self.systems.capture_before_world_change();
                Err(error)
            }
        }
    }

    /// Inner frame presentation (donor `step` body).
    fn step_inner(&mut self, mut elapsed_ms: f64) -> Result<bool, RemoteApplicationError> {
        self.systems.poll_browsers();
        self.source_events.clear();
        self.unhandled_effects.clear();
        self.drain_seat_publications()?;
        if self.ownership == RemoteOwnership::Owned {
            self.systems.pump_client_input();
            if self.systems.window_quit_requested() {
                self.request_quit();
            }
            self.flush_client_commands()?;
            if !self.systems.client_commands_blocked() {
                let continued = self.advance_client_startup()?;
                self.dispatch_commands()?;
                if !continued {
                    self.systems
                        .execute_startup_scripts()
                        .map_err(RemoteApplicationError::Message)?;
                }
            }
        }
        if self.stopping {
            self.systems
                .present_loading_frame()
                .map_err(RemoteApplicationError::Message)?;
            return Ok(false);
        }
        if self.reset_frame_elapsed {
            elapsed_ms = 1.0;
            self.reset_frame_elapsed = false;
        }
        let source_ms = self.systems.source_frame_ms(elapsed_ms);
        let active = self.systems.network_phase() == RemoteNetworkPhase::Active;
        let capture = self.systems.capture_frame(source_ms, active);
        let frame_ms = capture.milliseconds;
        if capture.capture {
            self.systems.capture_frame_now();
        }
        self.elapsed_ms += frame_ms;
        let now_ms = self.systems.now_ms();
        self.systems.advance_presentation_time(now_ms, elapsed_ms, frame_ms);
        if self.systems.primary_needs_userinfo_sync() {
            if let Some(info) = self.systems.primary_remote_userinfo() {
                let previous = match self
                    .primary_peer
                    .and_then(|id| self.peers.iter().find(|peer| peer.id == id))
                {
                    None => self.initial_userinfo.clone(),
                    Some(peer) => peer.userinfo.clone(),
                };
                if previous.as_deref() != Some(info.as_str()) {
                    self.systems.set_primary_userinfo(&info);
                    match self
                        .primary_peer
                        .and_then(|id| self.peers.iter_mut().find(|peer| peer.id == id))
                    {
                        None => self.initial_userinfo = Some(info),
                        Some(peer) => peer.userinfo = Some(info),
                    }
                }
            }
        }
        for peer in self.peers.clone() {
            if !peer.closed {
                self.systems.peer_begin_frame(peer.handle, now_ms, elapsed_ms);
            }
        }
        let advanced = match self.source_kind {
            RemoteSourceKind::Live => self.advance_source(SourceAdvanceOp::PollPrimary)?,
            RemoteSourceKind::Gtv => self.advance_source(SourceAdvanceOp::PollGtv)?,
            RemoteSourceKind::Recorded => {
                let presentation_ms = self.systems.presentation_ms();
                self.advance_source(SourceAdvanceOp::AdvanceRecorded {
                    frame_ms,
                    frame: self.frames,
                    presentation_ms: presentation_ms.trunc(),
                })?
            }
        };
        if !advanced {
            self.systems
                .present_loading_frame()
                .map_err(RemoteApplicationError::Message)?;
            return Ok(false);
        }
        self.poll_remote_seats(now_ms);
        self.drain_seat_publications()?;
        self.systems
            .prepare_primary_skins()
            .map_err(RemoteApplicationError::Message)?;
        self.frames += 1;
        let inputs = std::mem::take(&mut self.client_inputs);
        for input in inputs {
            if !self.closed
                && input.generation == self.world_load_generation
                && self.systems.input_client_live(input.client)
            {
                self.systems
                    .deliver_client_input(input.client, input.event)
                    .map_err(RemoteApplicationError::Message)?;
            }
        }
        let phase = self.systems.network_phase();
        if phase == RemoteNetworkPhase::Closed || phase == RemoteNetworkPhase::Rejected {
            let successor = self
                .peers
                .iter()
                .find(|peer| {
                    !peer.closed
                        && Some(peer.id) != self.primary_peer
                        && peer.has_view
                        && peer.map.as_deref() == Some(self.options.map.as_str())
                })
                .map(|peer| peer.id);
            match successor {
                None => {
                    self.systems.stop_haptics();
                    self.request_quit();
                    return Ok(false);
                }
                Some(_) => {
                    let index = self.seat_id.index();
                    self.drop_remote_seat(index)?;
                }
            }
        }
        if let Some(movie) = self.cinematic {
            if !movie.current {
                self.cinematic = None;
                let failures = self.systems.close_movie(movie.handle);
                if !failures.is_empty() {
                    return Err(RemoteApplicationError::Aggregate {
                        message: "Cinematic retirement failed".to_string(),
                        errors: failures,
                    });
                }
                return Ok(false);
            }
            let console_open = self.systems.console_open();
            let ended = self
                .systems
                .movie_frame(movie.handle, frame_ms, self.frames, console_open);
            let presented = !console_open;
            if presented {
                self.systems.drain_capture().map_err(RemoteApplicationError::Message)?;
            } else if self.ownership == RemoteOwnership::Owned {
                self.systems
                    .present_loading_frame()
                    .map_err(RemoteApplicationError::Message)?;
            }
            if ended {
                self.complete_cinematic(movie.ticket)?;
            }
            return Ok(false);
        }
        if self.systems.network_phase() != RemoteNetworkPhase::Active || !self.systems.primary_output_present() {
            self.systems.stop_haptics();
            if self.ownership == RemoteOwnership::Owned {
                self.dispatch_commands()?;
            }
            self.systems.refresh_images().map_err(RemoteApplicationError::Message)?;
            self.systems
                .present_loading_frame()
                .map_err(RemoteApplicationError::Message)?;
            return Ok(false);
        }
        if self.source_kind == RemoteSourceKind::Live {
            self.systems.sample_primary_presentation(now_ms);
        }
        self.bind_seat()?;
        self.systems.sync_primary_q3_selection();
        if self.source_kind == RemoteSourceKind::Live {
            let unified = self.options.network.kind == RemoteNetworkKind::UnifiedClient;
            self.systems
                .submit_primary(unified)
                .map_err(RemoteApplicationError::Message)?;
        }
        self.prepare_remote_seat_frames(now_ms, elapsed_ms)?;
        if self.ownership == RemoteOwnership::Owned {
            self.dispatch_commands()?;
        }
        if self.source_kind == RemoteSourceKind::Live && !self.advance_source(SourceAdvanceOp::PollPrimary)? {
            self.systems
                .present_loading_frame()
                .map_err(RemoteApplicationError::Message)?;
            return Ok(false);
        }
        self.systems
            .prepare_primary_skins()
            .map_err(RemoteApplicationError::Message)?;
        if self.systems.network_phase() != RemoteNetworkPhase::Active {
            self.systems.stop_haptics();
            return Ok(false);
        }
        self.bind_seat()?;
        self.systems.refresh_images().map_err(RemoteApplicationError::Message)?;
        let presentation = self
            .systems
            .present_primary_frame()
            .map_err(RemoteApplicationError::Message)?;
        self.source_events = presentation.events;
        for effect in &presentation.unhandled {
            let key = format!("{}:{}", effect.content, effect.reason);
            if self.reported_effect_gaps.insert(key) {
                self.systems
                    .print(&format!("Unresolved {} effect: {}\n", effect.kind, effect.reason));
            }
        }
        self.unhandled_effects = presentation.unhandled;
        self.source_session_actions(&presentation.batches)?;
        Ok(true)
    }

    /// Record session actions for presented batches (donor `sourceSessionActions`, sync fold).
    fn source_session_actions(&mut self, batches: &[RemoteAudioBatch]) -> Result<(), RemoteApplicationError> {
        if self.source_kind != RemoteSourceKind::Live {
            return Ok(());
        }
        let mut leaving: Vec<SeatId> = Vec::new();
        for batch in batches {
            for event in &batch.events {
                leaving.extend(event.departures.iter().cloned());
                if let Some(marker) = event.level_completion.as_ref() {
                    if event.recipient.is_none() {
                        self.systems.record_level_completion(
                            &batch.seat,
                            &event.content,
                            &self.options.map.clone(),
                            marker,
                        );
                    }
                }
            }
        }
        if !leaving.is_empty() {
            let configured: Vec<SeatId> = self
                .configuration
                .as_ref()
                .map(|configuration| configuration.seats.iter().map(|seat| seat.id.clone()).collect())
                .unwrap_or_default();
            if !configured.is_empty() && configured.iter().all(|id| leaving.contains(id)) {
                self.request_quit();
                return Ok(());
            }
        }
        for seat in leaving {
            let queued = self
                .commands
                .iter()
                .any(|command| command.name == "local_drop" && command.seat.as_ref() == Some(&seat));
            if !queued {
                self.queue_command("local_drop", &[], Some(seat), None)?;
            }
        }
        Ok(())
    }

    /// Run frames until quit, close, or the frame limit (donor `run`, sync fold).
    pub fn run(&mut self) -> Result<(), RemoteApplicationError> {
        let mut previous = self.systems.now_ms();
        while !self.stopping && !self.closed && self.options.frame_limit.is_none_or(|limit| self.frames < limit) {
            let now = self.systems.now_ms();
            let elapsed = now - previous;
            if elapsed < 4.0 {
                self.systems.sleep_ms(4.0 - elapsed);
                continue;
            }
            previous = now;
            let generation = self.systems.video_generation();
            self.step(elapsed)?;
            if self.systems.video_generation() != generation {
                previous = self.systems.now_ms();
            }
        }
        Ok(())
    }

    /// Read window pixels (donor `readPixels`).
    pub fn read_pixels(&mut self) -> Vec<u8> {
        self.systems.read_pixels()
    }

    /// Capture the next frame pixels (donor `captureNextFrame`).
    pub fn capture_next_frame(&mut self) -> Vec<u8> {
        self.systems.capture_next_frame()
    }

    /// Close the application (donor `close`, sync fold).
    pub fn close(&mut self) -> Result<(), RemoteApplicationError> {
        if self.close_done {
            return Ok(());
        }
        self.closing = true;
        self.stopping = true;
        let result = self.close_owned();
        self.close_done = true;
        result
    }

    /// Owned shutdown sequencing (donor `closeOwned`, sync fold).
    fn close_owned(&mut self) -> Result<(), RemoteApplicationError> {
        let mut errors: Vec<String> = Vec::new();
        for peer in self.peers.clone() {
            if let Err(error) = self.retire_remote_seat(peer.id, false) {
                match error {
                    RemoteApplicationError::Message(message) => errors.push(message),
                    RemoteApplicationError::Aggregate { errors: nested, .. } => errors.extend(nested),
                }
            }
        }
        if self.ownership == RemoteOwnership::Borrowed {
            if let Err(error) = self.systems.stop_borrowed_recording() {
                errors.push(error);
            }
        }
        if let Err(error) = self.systems.stop_owned_recording() {
            errors.push(error);
        }
        if self.peer_admission.take().is_some() {
            self.pending_advance = None;
        }
        if let Err(error) = self.stop_cinematic() {
            match error {
                RemoteApplicationError::Message(message) => errors.push(message),
                RemoteApplicationError::Aggregate { errors: nested, .. } => errors.extend(nested),
            }
        }
        if let Err(error) = self.systems.close_video_restart() {
            errors.push(error);
        }
        let owns_source = self.configuration_published
            && (self.ownership == RemoteOwnership::Owned || self.systems.borrowed_source_is_current());
        if owns_source {
            if let Err(error) = self.systems.save_source_settings() {
                errors.push(error);
            }
        }
        if self.ownership == RemoteOwnership::Borrowed && self.systems.borrowed_source_is_current() {
            if let Err(error) = self.systems.capture_before_world_change() {
                errors.push(error);
            }
            if let Err(error) = self.systems.release_borrowed_source() {
                errors.push(error);
            }
        }
        let closing_during_step = self.stepping;
        if closing_during_step {
            self.systems.close_q3_browser();
        }
        if self.ownership == RemoteOwnership::Owned {
            if let Err(error) = self.systems.close_capture() {
                errors.push(error);
            }
        }
        if !closing_during_step {
            if let Err(error) = self.systems.shutdown_primary_q3() {
                errors.push(error);
            }
            self.systems.close_q3_browser();
        }
        self.systems.release_settings();
        self.systems.clear_presentation();
        self.systems.disconnect_source();
        self.closed = true;
        self.stopping = true;
        self.world_load_generation += 1;
        self.client_inputs.clear();
        if let Err(error) = self.systems.close_initial_components() {
            errors.push(error);
        }
        errors.extend(self.systems.close_content_bundle());
        errors.extend(self.systems.retire_frontends());
        if let Err(error) = self.systems.flush_renderer() {
            errors.push(error);
        }
        self.systems.unregister_source_handlers(&self.source_handler_names());
        if self.ownership == RemoteOwnership::Owned || self.systems.should_close_scripts() {
            if let Err(error) = self.systems.close_scripts() {
                errors.push(error);
            }
        }
        if self.ownership == RemoteOwnership::Owned {
            errors.extend(self.systems.close_owned_bundle());
        }
        if let Err(error) = self.systems.close_server_browser() {
            errors.push(error);
        }
        if let Err(error) = self.systems.close_loaded_content() {
            errors.push(error);
        }
        if let Err(error) = self.systems.discard_identities() {
            errors.push(error);
        }
        if let Err(error) = self.systems.close_images_settings() {
            errors.push(error);
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(RemoteApplicationError::Aggregate {
                message: "Remote application shutdown failed".to_string(),
                errors,
            })
        }
    }

    /// Source handler names (donor `sourceHandlers` keys).
    fn source_handler_names(&self) -> Vec<String> {
        let mut names = vec![
            "localservers".to_string(),
            "globalservers".to_string(),
            "ping".to_string(),
            "serverstatus".to_string(),
        ];
        if self.source_kind != RemoteSourceKind::Live {
            names.push("follow".to_string());
            names.push("chase".to_string());
        }
        names
    }

    /// Prepare the remote configuration summary (donor `prepareRemoteConfiguration` seeding).
    pub fn prepare_configuration(
        &mut self,
        dialect: Dialect,
        seats: &[(SeatId, ClientId)],
    ) -> Result<(), RemoteApplicationError> {
        let unified = self.options.network.kind == RemoteNetworkKind::UnifiedClient;
        let mut seeds = Vec::new();
        for (seat, _) in seats {
            let cvars = seed_seat_cvars(dialect, seat.index(), &self.options.character_model, unified)?;
            seeds.push((seat.clone(), cvars));
        }
        let summary = self
            .systems
            .prepare_configuration(&self.options, &seeds)
            .map_err(RemoteApplicationError::Message)?;
        self.seat_id = summary.primary.clone();
        self.configuration = Some(summary);
        Ok(())
    }

    /// Publish the prepared configuration (donor `publishConfiguration` delegation).
    pub fn publish_configuration(&mut self) -> Result<(), RemoteApplicationError> {
        self.systems
            .publish_configuration()
            .map_err(RemoteApplicationError::Message)?;
        self.configuration_published = true;
        Ok(())
    }

    /// Current configuration summary, if prepared.
    pub fn configuration(&self) -> Option<&RemoteConfigurationSummary> {
        self.configuration.as_ref()
    }

    /// Remote seat peers.
    pub fn peers(&self) -> &[RemoteSeatPeer] {
        &self.peers
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;
    use std::cell::RefCell;
    use std::collections::{HashMap, HashSet, VecDeque};
    use std::rc::Rc;

    #[derive(Default)]
    struct FakePeer {
        active: bool,
        disconnected: Option<String>,
        remote_userinfo: Option<String>,
        needs_bind: bool,
        progress: Vec<RemoteDownloadProgress>,
        recorder: bool,
    }

    #[derive(Default)]
    struct FakeMovie {
        paused: bool,
        ended: bool,
    }

    struct FakeState {
        session: SessionId,
        seat: SeatId,
        now: f64,
        prints: Vec<String>,
        qport_offsets: VecDeque<u16>,
        resolve_calls: Vec<(String, u16, RemoteFamily)>,
        address: RemoteAddress,
        resolve_error: Option<String>,
        products: HashMap<(String, String), String>,
        expectations: HashMap<String, RemoteProductExpectation>,
        has_browser: bool,
        q3_product: Option<String>,
        resolve_movement_error: Option<String>,
        last_open: Option<OpenRequest>,
        finish_error: Option<String>,
        abort_failures: Vec<String>,
        config_seats: Vec<(SeatId, ClientId)>,
        config_dialect: Dialect,
        config_error: Option<String>,
        publish_config_error: Option<String>,
        publish_peer_error: Option<String>,
        source_commands: u32,
        release_settings: u32,
        save_settings_error: Option<String>,
        phase: RemoteNetworkPhase,
        advance_outcomes: VecDeque<Result<PollOutcome, String>>,
        advance_calls: Vec<SourceAdvanceOp>,
        submit_calls: Vec<bool>,
        userinfo_sync: bool,
        primary_userinfo: Option<String>,
        published_userinfo: Vec<String>,
        primary_recorder: bool,
        recording_root: String,
        recording_calls: Vec<(String, Option<String>)>,
        recording_error: Option<String>,
        stop_borrowed_error: Option<String>,
        stop_owned_error: Option<String>,
        signon_attachments: u64,
        detached: Vec<u64>,
        is_qw: bool,
        peers: HashMap<u64, FakePeer>,
        next_handle: u64,
        open_peer_error: Option<String>,
        aborted_peers: Vec<SeatId>,
        begin_calls: Vec<u64>,
        drop_calls: Vec<(u32, Option<u64>)>,
        drop_error: Option<String>,
        join_handle: u64,
        join_seat: Option<(SeatId, ClientId)>,
        join_error: Option<String>,
        primary_progress: Vec<RemoteDownloadProgress>,
        cancel_primary: u32,
        retry_primary_error: Option<String>,
        bind_primary: Result<bool, BindSeatFailure>,
        output_present: bool,
        present_frame: Result<RemoteFramePresentation, String>,
        completions: Vec<(SeatId, String, String, String)>,
        movies: HashMap<u64, FakeMovie>,
        next_movie: u64,
        movie_open_error: Option<String>,
        movie_activate_error: Option<String>,
        movie_close_failures: Vec<String>,
        movie_inputs: Vec<(u64, u32)>,
        movie_input_result: bool,
        movie_frames: Vec<(u64, f64, u64, bool)>,
        pauses: Vec<(u64, bool)>,
        console_open: bool,
        browser_calls: Vec<(String, Vec<String>)>,
        browser_results: HashMap<String, bool>,
        has_q3_browser: bool,
        browser_polls: u32,
        q3_browser_closes: u32,
        audio_calls: Vec<(String, Vec<String>)>,
        audio_results: HashMap<String, bool>,
        q3_calls: Vec<Vec<String>>,
        q3_result: bool,
        presentation_error: Option<String>,
        player_commands: Vec<(ActorId, String, Vec<String>)>,
        centerviews: Vec<ActorId>,
        use_ordinals: HashMap<String, i32>,
        players: Vec<RemoteLocalPlayer>,
        input_clients: HashMap<SeatId, u64>,
        live_clients: HashSet<u64>,
        delivered: Vec<(u64, u32)>,
        controls_result: bool,
        has_input: bool,
        info_calls: u32,
        restart_calls: u32,
        pump_calls: u32,
        window_quit: bool,
        blocked: bool,
        startup_continued: bool,
        startup_scripts: u32,
        borrowed_calls: Vec<(String, Vec<String>)>,
        source_frame_ms: Option<f64>,
        capture_decision: CaptureFrameDecision,
        capture_calls: u32,
        drain_calls: u32,
        world_changes: u32,
        capture_closes: u32,
        capture_activations: u32,
        presentation_ms: f64,
        loading_calls: u32,
        images_calls: u32,
        haptics_calls: u32,
        save_controls_error: Option<String>,
        controls_closes: u32,
        borrowed_current: bool,
        release_borrowed_error: Option<String>,
        shutdown_q3_error: Option<String>,
        presentations_cleared: u32,
        disconnects: u32,
        initial_components_error: Option<String>,
        content_bundle_failures: Vec<String>,
        retire_frontend_failures: Vec<String>,
        flush_error: Option<String>,
        unregistered: Vec<String>,
        should_close_scripts: bool,
        scripts_error: Option<String>,
        owned_bundle_failures: Vec<String>,
        browser_close_error: Option<String>,
        loaded_content_error: Option<String>,
        identities_error: Option<String>,
        images_settings_error: Option<String>,
        video_generation: u64,
        video_drain_error: Option<String>,
        video_close_error: Option<String>,
        pixels: Vec<u8>,
        mounts_changed: bool,
        mounts_error: Option<String>,
        gtv_prepared: u32,
        gtv_aborted: u32,
        q2_demo: Option<RemoteQ2ProtocolKind>,
    }

    impl FakeState {
        fn new(session: SessionId, seat: SeatId) -> Self {
            Self {
                session,
                seat: seat.clone(),
                now: 1000.0,
                prints: Vec::new(),
                qport_offsets: VecDeque::from([7, 8, 9, 10]),
                resolve_calls: Vec::new(),
                address: RemoteAddress {
                    key: "127.0.0.1:26000".to_string(),
                    ipv6: false,
                    ipx: false,
                    ipv4: true,
                },
                resolve_error: None,
                products: HashMap::new(),
                expectations: HashMap::new(),
                has_browser: true,
                q3_product: None,
                resolve_movement_error: None,
                last_open: None,
                finish_error: None,
                abort_failures: Vec::new(),
                config_seats: Vec::new(),
                config_dialect: Dialect::Q1Netquake,
                config_error: None,
                publish_config_error: None,
                publish_peer_error: None,
                source_commands: 0,
                release_settings: 0,
                save_settings_error: None,
                phase: RemoteNetworkPhase::Active,
                advance_outcomes: VecDeque::new(),
                advance_calls: Vec::new(),
                submit_calls: Vec::new(),
                userinfo_sync: false,
                primary_userinfo: None,
                published_userinfo: Vec::new(),
                primary_recorder: true,
                recording_root: "/tmp/rec".to_string(),
                recording_calls: Vec::new(),
                recording_error: None,
                stop_borrowed_error: None,
                stop_owned_error: None,
                signon_attachments: 0,
                detached: Vec::new(),
                is_qw: false,
                peers: HashMap::new(),
                next_handle: 1,
                open_peer_error: None,
                aborted_peers: Vec::new(),
                begin_calls: Vec::new(),
                drop_calls: Vec::new(),
                drop_error: None,
                join_handle: 0,
                join_seat: None,
                join_error: None,
                primary_progress: Vec::new(),
                cancel_primary: 0,
                retry_primary_error: None,
                bind_primary: Ok(true),
                output_present: true,
                present_frame: Ok(RemoteFramePresentation::default()),
                completions: Vec::new(),
                movies: HashMap::new(),
                next_movie: 1,
                movie_open_error: None,
                movie_activate_error: None,
                movie_close_failures: Vec::new(),
                movie_inputs: Vec::new(),
                movie_input_result: true,
                movie_frames: Vec::new(),
                pauses: Vec::new(),
                console_open: false,
                browser_calls: Vec::new(),
                browser_results: HashMap::new(),
                has_q3_browser: false,
                browser_polls: 0,
                q3_browser_closes: 0,
                audio_calls: Vec::new(),
                audio_results: HashMap::new(),
                q3_calls: Vec::new(),
                q3_result: false,
                presentation_error: None,
                player_commands: Vec::new(),
                centerviews: Vec::new(),
                use_ordinals: HashMap::new(),
                players: Vec::new(),
                input_clients: HashMap::new(),
                live_clients: HashSet::new(),
                delivered: Vec::new(),
                controls_result: false,
                has_input: true,
                info_calls: 0,
                restart_calls: 0,
                pump_calls: 0,
                window_quit: false,
                blocked: false,
                startup_continued: true,
                startup_scripts: 0,
                borrowed_calls: Vec::new(),
                source_frame_ms: None,
                capture_decision: CaptureFrameDecision {
                    milliseconds: 16.0,
                    capture: false,
                },
                capture_calls: 0,
                drain_calls: 0,
                world_changes: 0,
                capture_closes: 0,
                capture_activations: 0,
                presentation_ms: 5000.0,
                loading_calls: 0,
                images_calls: 0,
                haptics_calls: 0,
                save_controls_error: None,
                controls_closes: 0,
                borrowed_current: false,
                release_borrowed_error: None,
                shutdown_q3_error: None,
                presentations_cleared: 0,
                disconnects: 0,
                initial_components_error: None,
                content_bundle_failures: Vec::new(),
                retire_frontend_failures: Vec::new(),
                flush_error: None,
                unregistered: Vec::new(),
                should_close_scripts: true,
                scripts_error: None,
                owned_bundle_failures: Vec::new(),
                browser_close_error: None,
                loaded_content_error: None,
                identities_error: None,
                images_settings_error: None,
                video_generation: 3,
                video_drain_error: None,
                video_close_error: None,
                pixels: vec![1, 2, 3],
                mounts_changed: false,
                mounts_error: None,
                gtv_prepared: 0,
                gtv_aborted: 0,
                q2_demo: None,
            }
        }
    }

    struct FakeSystems {
        state: Rc<RefCell<FakeState>>,
    }

    macro_rules! fake_err {
        ($self:ident, $field:ident) => {
            if let Some(error) = $self.state.borrow().$field.clone() {
                return Err(error);
            }
        };
    }

    impl RemoteApplicationSystems for FakeSystems {
        fn now_ms(&mut self) -> f64 {
            self.state.borrow().now
        }
        fn print(&mut self, text: &str) {
            self.state.borrow_mut().prints.push(text.to_string());
        }
        fn sleep_ms(&mut self, ms: f64) {
            self.state.borrow_mut().now += ms;
        }
        fn random_qport_offset(&mut self) -> u16 {
            let mut state = self.state.borrow_mut();
            state.qport_offsets.pop_front().unwrap_or(0)
        }
        fn resolve_address(
            &mut self,
            remote: &str,
            default_port: u16,
            family: RemoteFamily,
        ) -> Result<RemoteAddress, String> {
            self.state
                .borrow_mut()
                .resolve_calls
                .push((remote.to_string(), default_port, family));
            let state = self.state.borrow();
            if let Some(error) = state.resolve_error.clone() {
                return Err(error);
            }
            Ok(state.address.clone())
        }
        fn content_product(&mut self, selection: &RemoteContentSelection) -> String {
            self.state
                .borrow()
                .products
                .get(&(selection.base.clone(), selection.directory.clone()))
                .cloned()
                .unwrap_or_else(|| format!("{}-{}", selection.base, selection.directory))
        }
        fn product_expectation(&mut self, product: &str) -> Option<RemoteProductExpectation> {
            self.state.borrow().expectations.get(product).cloned()
        }
        fn default_q2_selection(&mut self, rerelease: bool) -> RemoteContentSelection {
            RemoteContentSelection {
                base: if rerelease {
                    "q2-rerelease-baseq2".to_string()
                } else {
                    "q2-classic-baseq2".to_string()
                },
                directory: "baseq2".to_string(),
            }
        }
        fn has_server_browser(&mut self) -> bool {
            self.state.borrow().has_browser
        }
        fn open_content_q3_product(&mut self, _options: &RemoteApplicationOptions) -> Option<String> {
            self.state.borrow().q3_product.clone()
        }
        fn resolve_movement(&mut self, options: &RemoteApplicationOptions) -> Result<RemoteApplicationOptions, String> {
            fake_err!(self, resolve_movement_error);
            Ok(options.clone())
        }
        fn finish_open(&mut self, request: &OpenRequest) -> Result<OpenHandles, String> {
            self.state.borrow_mut().last_open = Some(request.clone());
            fake_err!(self, finish_error);
            Ok(OpenHandles::default())
        }
        fn abort_open(&mut self, _stage: &str) -> Vec<String> {
            self.state.borrow().abort_failures.clone()
        }
        fn session_id(&mut self) -> SessionId {
            self.state.borrow().session.clone()
        }
        fn primary_seat_id(&mut self) -> SeatId {
            self.state.borrow().seat.clone()
        }
        fn prepare_configuration(
            &mut self,
            options: &RemoteApplicationOptions,
            seeds: &[(SeatId, CvarRegistry)],
        ) -> Result<RemoteConfigurationSummary, String> {
            fake_err!(self, config_error);
            let state = self.state.borrow();
            let dialect = state.config_dialect;
            let seats: Vec<RemoteConfiguredSeat> = state
                .config_seats
                .iter()
                .map(|(seat, client)| RemoteConfiguredSeat {
                    id: seat.clone(),
                    client: client.clone(),
                })
                .collect();
            let primary = state.seat.clone();
            let character = options.character_model.clone();
            let unified = options.network.kind == RemoteNetworkKind::UnifiedClient;
            drop(state);
            let mut seat_sources = Vec::new();
            for (seat, _) in seeds {
                let cvars =
                    seed_seat_cvars(dialect, seat.index(), &character, unified).map_err(|error| error.to_string())?;
                seat_sources.push((seat.clone(), cvars));
            }
            let source =
                seed_seat_cvars(dialect, primary.index(), &character, unified).map_err(|error| error.to_string())?;
            Ok(RemoteConfigurationSummary {
                seats,
                primary,
                source,
                seat_sources,
                settings_root: "/tmp/settings".to_string(),
                dialect,
            })
        }
        fn publish_configuration(&mut self) -> Result<(), String> {
            fake_err!(self, publish_config_error);
            Ok(())
        }
        fn publish_peer_configuration(&mut self, _options: &RemoteApplicationOptions) -> Result<(), String> {
            fake_err!(self, publish_peer_error);
            Ok(())
        }
        fn activate_source_commands(&mut self) {
            self.state.borrow_mut().source_commands += 1;
        }
        fn release_settings(&mut self) {
            self.state.borrow_mut().release_settings += 1;
        }
        fn save_source_settings(&mut self) -> Result<(), String> {
            fake_err!(self, save_settings_error);
            Ok(())
        }
        fn network_phase(&mut self) -> RemoteNetworkPhase {
            self.state.borrow().phase
        }
        fn execute_advance(&mut self, op: SourceAdvanceOp) -> Result<PollOutcome, String> {
            self.state.borrow_mut().advance_calls.push(op);
            self.state
                .borrow_mut()
                .advance_outcomes
                .pop_front()
                .unwrap_or(Ok(PollOutcome::default()))
        }
        fn submit_primary(&mut self, timed: bool) -> Result<(), String> {
            self.state.borrow_mut().submit_calls.push(timed);
            Ok(())
        }
        fn primary_needs_userinfo_sync(&mut self) -> bool {
            self.state.borrow().userinfo_sync
        }
        fn primary_remote_userinfo(&mut self) -> Option<String> {
            self.state.borrow().primary_userinfo.clone()
        }
        fn set_primary_userinfo(&mut self, info: &str) {
            self.state.borrow_mut().published_userinfo.push(info.to_string());
        }
        fn primary_has_recorder(&mut self) -> bool {
            self.state.borrow().primary_recorder
        }
        fn peer_has_recorder(&mut self, handle: u64) -> bool {
            self.state.borrow().peers.get(&handle).is_some_and(|peer| peer.recorder)
        }
        fn recording_root(&mut self) -> String {
            self.state.borrow().recording_root.clone()
        }
        fn recording_command(
            &mut self,
            name: &str,
            arg: Option<&str>,
            _context: &CommandContext,
        ) -> Result<(), String> {
            self.state
                .borrow_mut()
                .recording_calls
                .push((name.to_string(), arg.map(str::to_string)));
            fake_err!(self, recording_error);
            Ok(())
        }
        fn stop_borrowed_recording(&mut self) -> Result<(), String> {
            fake_err!(self, stop_borrowed_error);
            Ok(())
        }
        fn stop_owned_recording(&mut self) -> Result<(), String> {
            fake_err!(self, stop_owned_error);
            Ok(())
        }
        fn attach_signon_recording(&mut self) -> Result<u64, String> {
            let mut state = self.state.borrow_mut();
            state.signon_attachments += 1;
            Ok(state.signon_attachments)
        }
        fn detach_signon_recording(&mut self, attachment: u64) {
            self.state.borrow_mut().detached.push(attachment);
        }
        fn is_live_quakeworld(&mut self) -> bool {
            self.state.borrow().is_qw
        }
        fn open_peer(&mut self, _seat: &SeatId, _address: &RemoteAddress, _qport: u16) -> Result<u64, String> {
            fake_err!(self, open_peer_error);
            let mut state = self.state.borrow_mut();
            let handle = state.next_handle;
            state.next_handle += 1;
            state.peers.insert(handle, FakePeer::default());
            Ok(handle)
        }
        fn abort_peer(&mut self, seat: &SeatId) {
            self.state.borrow_mut().aborted_peers.push(seat.clone());
        }
        fn peer_active(&mut self, handle: u64) -> bool {
            self.state.borrow().peers.get(&handle).is_some_and(|peer| peer.active)
        }
        fn poll_peer(&mut self, _handle: u64, _now_ms: f64) {}
        fn drain_peer(&mut self, _handle: u64) -> Result<(), String> {
            Ok(())
        }
        fn take_peer_disconnected(&mut self, handle: u64) -> Option<String> {
            self.state
                .borrow_mut()
                .peers
                .get_mut(&handle)
                .and_then(|peer| peer.disconnected.take())
        }
        fn peer_remote_userinfo(&mut self, handle: u64) -> Option<String> {
            self.state
                .borrow()
                .peers
                .get(&handle)
                .and_then(|peer| peer.remote_userinfo.clone())
        }
        fn set_peer_userinfo(&mut self, handle: u64, info: &str) {
            if let Some(peer) = self.state.borrow_mut().peers.get_mut(&handle) {
                peer.remote_userinfo = Some(info.to_string());
            }
        }
        fn sample_peer_presentation(&mut self, _handle: u64, _now_ms: f64) {}
        fn prepare_peer_skins(&mut self, _handle: u64) -> Result<(), String> {
            Ok(())
        }
        fn peer_needs_bind(&mut self, handle: u64) -> bool {
            self.state
                .borrow()
                .peers
                .get(&handle)
                .is_some_and(|peer| peer.needs_bind)
        }
        fn bind_peer_seat(&mut self, handle: u64) -> Result<(), String> {
            if let Some(peer) = self.state.borrow_mut().peers.get_mut(&handle) {
                peer.needs_bind = false;
            }
            Ok(())
        }
        fn submit_peer_frame(
            &mut self,
            _handle: u64,
            _now_ms: f64,
            _wall_elapsed_ms: f64,
        ) -> Result<PeerFrameOutcome, String> {
            Ok(PeerFrameOutcome::Submitted)
        }
        fn retire_peer_view(&mut self, _handle: u64) -> Result<(), String> {
            Ok(())
        }
        fn close_peer(&mut self, handle: u64) -> Vec<String> {
            self.state.borrow_mut().peers.remove(&handle);
            Vec::new()
        }
        fn peer_download_progress(&mut self, handle: u64) -> Vec<RemoteDownloadProgress> {
            self.state
                .borrow()
                .peers
                .get(&handle)
                .map(|peer| peer.progress.clone())
                .unwrap_or_default()
        }
        fn cancel_peer_downloads(&mut self, handle: u64) {
            if let Some(peer) = self.state.borrow_mut().peers.get_mut(&handle) {
                peer.progress.clear();
            }
        }
        fn retry_peer_downloads(&mut self, _handle: u64) -> Result<(), String> {
            Ok(())
        }
        fn drop_peer_seat(&mut self, index: u32, replacement: Option<u64>) -> Result<(), String> {
            self.state.borrow_mut().drop_calls.push((index, replacement));
            fake_err!(self, drop_error);
            Ok(())
        }
        fn join_peer_seat(&mut self) -> Result<(u64, SeatId, ClientId), String> {
            fake_err!(self, join_error);
            let mut state = self.state.borrow_mut();
            if state.join_handle == 0 {
                state.join_handle = state.next_handle;
                state.next_handle += 1;
            }
            let (seat, client) = state.join_seat.clone().expect("join seat");
            Ok((state.join_handle, seat, client))
        }
        fn primary_download_progress(&mut self) -> Vec<RemoteDownloadProgress> {
            self.state.borrow().primary_progress.clone()
        }
        fn cancel_primary_downloads(&mut self) {
            let mut state = self.state.borrow_mut();
            state.cancel_primary += 1;
            state.primary_progress.clear();
        }
        fn retry_primary_downloads(&mut self) -> Result<(), String> {
            fake_err!(self, retry_primary_error);
            Ok(())
        }
        fn bind_primary_seat(&mut self) -> Result<bool, BindSeatFailure> {
            self.state.borrow().bind_primary.clone()
        }
        fn sample_primary_presentation(&mut self, _now_ms: f64) {}
        fn prepare_primary_skins(&mut self) -> Result<(), String> {
            Ok(())
        }
        fn primary_output_present(&mut self) -> bool {
            self.state.borrow().output_present
        }
        fn sync_primary_q3_selection(&mut self) {}
        fn present_primary_frame(&mut self) -> Result<RemoteFramePresentation, String> {
            self.state.borrow().present_frame.clone()
        }
        fn record_level_completion(&mut self, seat: &SeatId, content: &str, map: &str, marker: &str) {
            self.state.borrow_mut().completions.push((
                seat.clone(),
                content.to_string(),
                map.to_string(),
                marker.to_string(),
            ));
        }
        fn open_movie(&mut self, _request: &RemoteCinematicRequest) -> Result<u64, String> {
            fake_err!(self, movie_open_error);
            let mut state = self.state.borrow_mut();
            let handle = state.next_movie;
            state.next_movie += 1;
            state.movies.insert(handle, FakeMovie::default());
            Ok(handle)
        }
        fn activate_movie(&mut self, _handle: u64) -> Result<(), String> {
            fake_err!(self, movie_activate_error);
            Ok(())
        }
        fn movie_input(&mut self, handle: u64, event: u32) -> bool {
            self.state.borrow_mut().movie_inputs.push((handle, event));
            self.state.borrow().movie_input_result
        }
        fn movie_frame(&mut self, handle: u64, frame_ms: f64, frame: u64, console_open: bool) -> bool {
            self.state
                .borrow_mut()
                .movie_frames
                .push((handle, frame_ms, frame, console_open));
            self.state.borrow().movies.get(&handle).is_some_and(|movie| movie.ended)
        }
        fn movie_pause(&mut self, handle: u64, paused: bool) {
            self.state.borrow_mut().pauses.push((handle, paused));
            if let Some(movie) = self.state.borrow_mut().movies.get_mut(&handle) {
                movie.paused = paused;
            }
        }
        fn movie_paused(&mut self, handle: u64) -> bool {
            self.state
                .borrow()
                .movies
                .get(&handle)
                .is_some_and(|movie| movie.paused)
        }
        fn close_movie(&mut self, handle: u64) -> Vec<String> {
            self.state.borrow_mut().movies.remove(&handle);
            self.state.borrow().movie_close_failures.clone()
        }
        fn console_open(&mut self) -> bool {
            self.state.borrow().console_open
        }
        fn browser_command(&mut self, name: &str, args: &[String]) -> Result<bool, String> {
            self.state
                .borrow_mut()
                .browser_calls
                .push((name.to_string(), args.to_vec()));
            Ok(self.state.borrow().browser_results.get(name).copied().unwrap_or(true))
        }
        fn has_q3_browser(&mut self) -> bool {
            self.state.borrow().has_q3_browser
        }
        fn poll_browsers(&mut self) {
            self.state.borrow_mut().browser_polls += 1;
        }
        fn close_q3_browser(&mut self) {
            self.state.borrow_mut().q3_browser_closes += 1;
        }
        fn audio_command(&mut self, name: &str, args: &[String], _seat: Option<&SeatId>) -> Result<bool, String> {
            self.state
                .borrow_mut()
                .audio_calls
                .push((name.to_string(), args.to_vec()));
            Ok(self.state.borrow().audio_results.get(name).copied().unwrap_or(false))
        }
        fn q3_command(&mut self, _seat: Option<&SeatId>, argv: &[String]) -> Result<bool, String> {
            self.state.borrow_mut().q3_calls.push(argv.to_vec());
            Ok(self.state.borrow().q3_result)
        }
        fn presentation_for_actor(&mut self, _actor: &ActorId) -> Result<u64, String> {
            fake_err!(self, presentation_error);
            Ok(1)
        }
        fn player_command(&mut self, actor: &ActorId, name: &str, args: &[String]) {
            self.state
                .borrow_mut()
                .player_commands
                .push((actor.clone(), name.to_string(), args.to_vec()));
        }
        fn centerview(&mut self, actor: &ActorId) {
            self.state.borrow_mut().centerviews.push(actor.clone());
        }
        fn use_item_ordinal(&mut self, _actor: &ActorId, item: &str) -> Option<i32> {
            self.state.borrow().use_ordinals.get(item).copied()
        }
        fn local_players(&mut self) -> Vec<RemoteLocalPlayer> {
            self.state.borrow().players.clone()
        }
        fn input_client_for_seat(&mut self, seat: &SeatId) -> Option<u64> {
            self.state.borrow().input_clients.get(seat).copied()
        }
        fn input_client_live(&mut self, client: u64) -> bool {
            self.state.borrow().live_clients.contains(&client)
        }
        fn deliver_client_input(&mut self, client: u64, event: u32) -> Result<(), String> {
            self.state.borrow_mut().delivered.push((client, event));
            Ok(())
        }
        fn controls_input(&mut self, _event: u32) -> bool {
            self.state.borrow().controls_result
        }
        fn has_input(&mut self) -> bool {
            self.state.borrow().has_input
        }
        fn input_devices_info(&mut self) {
            self.state.borrow_mut().info_calls += 1;
        }
        fn restart_input_devices(&mut self) {
            self.state.borrow_mut().restart_calls += 1;
        }
        fn pump_client_input(&mut self) {
            self.state.borrow_mut().pump_calls += 1;
        }
        fn window_quit_requested(&mut self) -> bool {
            self.state.borrow().window_quit
        }
        fn client_commands_blocked(&mut self) -> bool {
            self.state.borrow().blocked
        }
        fn advance_client_startup(&mut self) -> Result<bool, String> {
            Ok(self.state.borrow().startup_continued)
        }
        fn execute_startup_scripts(&mut self) -> Result<(), String> {
            self.state.borrow_mut().startup_scripts += 1;
            Ok(())
        }
        fn dispatch_borrowed(
            &mut self,
            name: &str,
            args: &[String],
            _seat: Option<&SeatId>,
            _source: &CommandContext,
        ) -> Result<(), String> {
            self.state
                .borrow_mut()
                .borrowed_calls
                .push((name.to_string(), args.to_vec()));
            Ok(())
        }
        fn source_frame_ms(&mut self, elapsed_ms: f64) -> f64 {
            self.state.borrow().source_frame_ms.unwrap_or(elapsed_ms)
        }
        fn capture_frame(&mut self, source_ms: f64, _active: bool) -> CaptureFrameDecision {
            let mut decision = self.state.borrow().capture_decision;
            if decision.milliseconds <= 0.0 {
                decision.milliseconds = source_ms;
            }
            decision
        }
        fn capture_frame_now(&mut self) {
            self.state.borrow_mut().capture_calls += 1;
        }
        fn drain_capture(&mut self) -> Result<(), String> {
            self.state.borrow_mut().drain_calls += 1;
            Ok(())
        }
        fn capture_before_world_change(&mut self) -> Result<(), String> {
            self.state.borrow_mut().world_changes += 1;
            Ok(())
        }
        fn close_capture(&mut self) -> Result<(), String> {
            self.state.borrow_mut().capture_closes += 1;
            Ok(())
        }
        fn activate_capture(&mut self) {
            self.state.borrow_mut().capture_activations += 1;
        }
        fn advance_presentation_time(&mut self, _now_ms: f64, _elapsed_ms: f64, _frame_ms: f64) {}
        fn presentation_ms(&mut self) -> f64 {
            self.state.borrow().presentation_ms
        }
        fn peer_begin_frame(&mut self, handle: u64, _now_ms: f64, _elapsed_ms: f64) {
            self.state.borrow_mut().begin_calls.push(handle);
        }
        fn present_loading_frame(&mut self) -> Result<(), String> {
            self.state.borrow_mut().loading_calls += 1;
            Ok(())
        }
        fn refresh_images(&mut self) -> Result<(), String> {
            self.state.borrow_mut().images_calls += 1;
            Ok(())
        }
        fn stop_haptics(&mut self) {
            self.state.borrow_mut().haptics_calls += 1;
        }
        fn save_control_settings(&mut self) -> Result<(), String> {
            fake_err!(self, save_controls_error);
            Ok(())
        }
        fn close_controls(&mut self) {
            self.state.borrow_mut().controls_closes += 1;
        }
        fn borrowed_source_is_current(&mut self) -> bool {
            self.state.borrow().borrowed_current
        }
        fn release_borrowed_source(&mut self) -> Result<(), String> {
            fake_err!(self, release_borrowed_error);
            Ok(())
        }
        fn shutdown_primary_q3(&mut self) -> Result<(), String> {
            fake_err!(self, shutdown_q3_error);
            Ok(())
        }
        fn clear_presentation(&mut self) {
            self.state.borrow_mut().presentations_cleared += 1;
        }
        fn disconnect_source(&mut self) {
            self.state.borrow_mut().disconnects += 1;
        }
        fn close_initial_components(&mut self) -> Result<(), String> {
            fake_err!(self, initial_components_error);
            Ok(())
        }
        fn close_content_bundle(&mut self) -> Vec<String> {
            self.state.borrow().content_bundle_failures.clone()
        }
        fn retire_frontends(&mut self) -> Vec<String> {
            self.state.borrow().retire_frontend_failures.clone()
        }
        fn flush_renderer(&mut self) -> Result<(), String> {
            fake_err!(self, flush_error);
            Ok(())
        }
        fn unregister_source_handlers(&mut self, names: &[String]) {
            self.state.borrow_mut().unregistered.extend(names.iter().cloned());
        }
        fn should_close_scripts(&mut self) -> bool {
            self.state.borrow().should_close_scripts
        }
        fn close_scripts(&mut self) -> Result<(), String> {
            fake_err!(self, scripts_error);
            Ok(())
        }
        fn close_owned_bundle(&mut self) -> Vec<String> {
            self.state.borrow().owned_bundle_failures.clone()
        }
        fn close_server_browser(&mut self) -> Result<(), String> {
            fake_err!(self, browser_close_error);
            Ok(())
        }
        fn close_loaded_content(&mut self) -> Result<(), String> {
            fake_err!(self, loaded_content_error);
            Ok(())
        }
        fn discard_identities(&mut self) -> Result<(), String> {
            fake_err!(self, identities_error);
            Ok(())
        }
        fn close_images_settings(&mut self) -> Result<(), String> {
            fake_err!(self, images_settings_error);
            Ok(())
        }
        fn drain_video_restart(&mut self) -> Result<(), String> {
            fake_err!(self, video_drain_error);
            Ok(())
        }
        fn close_video_restart(&mut self) -> Result<(), String> {
            fake_err!(self, video_close_error);
            Ok(())
        }
        fn video_generation(&mut self) -> u64 {
            self.state.borrow().video_generation
        }
        fn read_pixels(&mut self) -> Vec<u8> {
            self.state.borrow().pixels.clone()
        }
        fn capture_next_frame(&mut self) -> Vec<u8> {
            self.state.borrow().pixels.clone()
        }
        fn select_content_mounts(&mut self, _selection: &RemoteContentSelection) -> Result<bool, String> {
            fake_err!(self, mounts_error);
            Ok(self.state.borrow().mounts_changed)
        }
        fn prepare_gtv(&mut self, _launch: &RemoteGtvLaunch) -> Result<(), String> {
            self.state.borrow_mut().gtv_prepared += 1;
            Ok(())
        }
        fn abort_gtv(&mut self) {
            self.state.borrow_mut().gtv_aborted += 1;
        }
        fn q2_demo_protocol(&mut self, _recorded: &RemoteRecordedLaunch) -> Option<RemoteQ2ProtocolKind> {
            self.state.borrow().q2_demo
        }
    }

    struct Fixture {
        owner: IdentityOwner,
        state: Rc<RefCell<FakeState>>,
    }

    fn fixture() -> Fixture {
        let owner = IdentityOwner::create("remote-application-test").expect("owner");
        let session = owner.session().clone();
        let seat = owner.seat(0);
        let client = owner.client(1, 1);
        let mut state = FakeState::new(session, seat.clone());
        state.config_seats = vec![(seat, client)];
        state.join_seat = Some((owner.seat(1), owner.client(2, 1)));
        state.expectations.insert(
            "q1-classic-id1".to_string(),
            RemoteProductExpectation {
                id: "q1-classic-id1".to_string(),
                family: "q1".to_string(),
                rerelease: false,
            },
        );
        state.expectations.insert(
            "q2-classic-baseq2".to_string(),
            RemoteProductExpectation {
                id: "q2-classic-baseq2".to_string(),
                family: "q2".to_string(),
                rerelease: false,
            },
        );
        state.products.insert(
            ("q2-classic-baseq2".to_string(), "baseq2".to_string()),
            "q2-classic-baseq2".to_string(),
        );
        Fixture {
            owner,
            state: Rc::new(RefCell::new(state)),
        }
    }

    fn systems(fixture: &Fixture) -> Box<dyn RemoteApplicationSystems> {
        Box::new(FakeSystems {
            state: fixture.state.clone(),
        })
    }

    fn live_options() -> RemoteApplicationOptions {
        RemoteApplicationOptions {
            network: RemoteNetworkOptions {
                kind: RemoteNetworkKind::Q1Client,
                remote: "127.0.0.1:26000".to_string(),
            },
            product: "q1-classic-id1".to_string(),
            character: "q1".to_string(),
            character_model: "player".to_string(),
            movement: "q1".to_string(),
            seats: 1,
            map: "maps/e1m1.bsp".to_string(),
            seed: 7,
            gamma: None,
            frame_limit: None,
            startup_commands: Vec::new(),
            remote_content: None,
            q2_protocol: None,
            network_transport: None,
            user_content_root: None,
            dedicated: false,
            rules: "standard".to_string(),
            q3_product: None,
        }
    }

    fn open_live(fixture: &Fixture) -> RemoteApplication {
        RemoteApplication::open(live_options(), systems(fixture)).expect("open")
    }

    fn configure(app: &mut RemoteApplication, fixture: &Fixture, seats: u32, dialect: Dialect) {
        let owner = &fixture.owner;
        let list: Vec<(SeatId, ClientId)> = (0..seats)
            .map(|index| (owner.seat(index), owner.client(index + 1, 1)))
            .collect();
        fixture.state.borrow_mut().config_seats = list.clone();
        fixture.state.borrow_mut().config_dialect = dialect;
        app.prepare_configuration(dialect, &list).expect("configure");
        app.publish_configuration().expect("publish");
    }

    fn command(name: &str, args: &[&str]) -> RemoteCommand {
        RemoteCommand {
            name: name.to_string(),
            args: args.iter().map(|arg| arg.to_string()).collect(),
            seat: None,
            source: None,
        }
    }

    #[test]
    fn open_validates_address_family_and_content() {
        let fixture = fixture();
        let app = open_live(&fixture);
        assert_eq!(app.options().product, "q1-classic-id1");
        let calls = fixture.state.borrow().resolve_calls.clone();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1, 26000);

        let mut options = live_options();
        options.network.kind = RemoteNetworkKind::Other("menu".to_string());
        let error = RemoteApplication::open(options, systems(&fixture)).expect_err("error");
        assert_eq!(error.to_string(), "RemoteApplication requires a native connect address");

        let mut options = live_options();
        options.network.kind = RemoteNetworkKind::Q2Client;
        options.movement = "q2".to_string();
        options.character = "q2".to_string();
        options.character_model = "grunt".to_string();
        let error = RemoteApplication::open(options, systems(&fixture)).expect_err("error");
        assert_eq!(
            error.to_string(),
            "Remote Q2 character selection requires an installed male, female or cyborg player appearance"
        );

        let mut options = live_options();
        options.dedicated = true;
        let error = RemoteApplication::open(options, systems(&fixture)).expect_err("error");
        assert!(error.to_string().contains("one to four graphical seats"), "{error}");

        let mut options = live_options();
        options.product = "q2-classic-baseq2".to_string();
        let error = RemoteApplication::open(options, systems(&fixture)).expect_err("error");
        assert_eq!(
            error.to_string(),
            "Remote application requires content matching the selected native protocol"
        );
    }

    #[test]
    fn open_q2_selects_protocol_content_and_ports() {
        let fixture = fixture();
        let mut options = live_options();
        options.network.kind = RemoteNetworkKind::Q2Client;
        options.network.remote = "127.0.0.1:27910".to_string();
        options.product = "q2-classic-baseq2".to_string();
        options.movement = "q2".to_string();
        options.character = "q2".to_string();
        options.character_model = "male".to_string();
        let app = RemoteApplication::open(options, systems(&fixture)).expect("open");
        assert_eq!(app.options().product, "q2-classic-baseq2");
        assert_eq!(
            app.options().q2_protocol,
            Some(RemoteQ2Protocol {
                kind: RemoteQ2ProtocolKind::Classic,
                version: 34
            })
        );
        assert_eq!(fixture.state.borrow().resolve_calls[0].1, 27910);

        fixture.state.borrow_mut().expectations.insert(
            "q2-rerelease-baseq2".to_string(),
            RemoteProductExpectation {
                id: "q2-rerelease-baseq2".to_string(),
                family: "q2".to_string(),
                rerelease: true,
            },
        );
        fixture.state.borrow_mut().products.insert(
            ("q2-rerelease-baseq2".to_string(), "baseq2".to_string()),
            "q2-rerelease-baseq2".to_string(),
        );
        let mut options = live_options();
        options.network.kind = RemoteNetworkKind::Q2Client;
        options.product = "q2-rerelease-baseq2".to_string();
        options.movement = "q2".to_string();
        options.character = "q2".to_string();
        options.character_model = "male".to_string();
        options.q2_protocol = Some(RemoteQ2Protocol {
            kind: RemoteQ2ProtocolKind::Kex,
            version: 2023,
        });
        let app = RemoteApplication::open(options, systems(&fixture)).expect("open");
        assert_eq!(fixture.state.borrow().resolve_calls.last().expect("call").1, 5069);
        drop(app);
    }

    #[test]
    fn open_rejects_quakeworld_ipx_and_q3_non_ipv4() {
        let first = fixture();
        first.state.borrow_mut().address.ipx = true;
        first.state.borrow_mut().expectations.insert(
            "q1-quakeworld-qw".to_string(),
            RemoteProductExpectation {
                id: "q1-quakeworld-qw".to_string(),
                family: "q1".to_string(),
                rerelease: false,
            },
        );
        let mut options = live_options();
        options.network.kind = RemoteNetworkKind::QwClient;
        let error = RemoteApplication::open(options, systems(&first)).expect_err("error");
        assert_eq!(error.to_string(), "QuakeWorld requires UDP");

        let fixture = fixture();
        fixture.state.borrow_mut().address.ipv4 = false;
        fixture.state.borrow_mut().expectations.insert(
            "q3-baseq3".to_string(),
            RemoteProductExpectation {
                id: "q3-baseq3".to_string(),
                family: "q3".to_string(),
                rerelease: false,
            },
        );
        fixture
            .state
            .borrow_mut()
            .products
            .insert(("q3-baseq3".to_string(), "baseq3".to_string()), "q3-baseq3".to_string());
        let mut options = live_options();
        options.network.kind = RemoteNetworkKind::Q3Client;
        options.product = "q3-baseq3".to_string();
        options.movement = "q3".to_string();
        options.character = "q3".to_string();
        options.character_model = "sarge".to_string();
        options.remote_content = Some(RemoteContentSelection {
            base: "q3-baseq3".to_string(),
            directory: "baseq3".to_string(),
        });
        let error = RemoteApplication::open(options, systems(&fixture)).expect_err("error");
        assert_eq!(error.to_string(), "Native Q3 remote requires IPv4");
    }

    #[test]
    fn open_demo_maps_recorded_options() {
        let fixture = fixture();
        fixture.state.borrow_mut().products.insert(
            ("q2-classic-baseq2".to_string(), "baseq2".to_string()),
            "q2-classic-baseq2".to_string(),
        );
        let playback = RemoteRecordedLaunch {
            kind: RemoteDemoKind::Q2,
            path: "demos/test.dm2".to_string(),
            timedemo: false,
            q2_protocol: None,
        };
        let app =
            RemoteApplication::open_demo_borrowed(live_options(), systems(&fixture), playback).expect("open demo");
        assert_eq!(app.options().network.kind, RemoteNetworkKind::Offline);
        assert_eq!(app.options().seats, 1);
        assert_eq!(app.options().movement, "q2");
        assert_eq!(app.options().character_model, "male");
        assert_eq!(app.options().product, "q2-classic-baseq2");
        assert!(app.network_address_key().is_err());
    }

    #[test]
    fn open_gtv_rejects_kex_demo_and_aborts() {
        let fixture = fixture();
        let launch = RemoteGtvLaunch {
            label: "gtv:1".to_string(),
            rerelease: false,
            game_directory: "baseq2".to_string(),
            protocol: RemoteQ2Protocol {
                kind: RemoteQ2ProtocolKind::KexDemo,
                version: 1,
            },
        };
        let error = RemoteApplication::open_gtv_borrowed(live_options(), systems(&fixture), launch).expect_err("error");
        assert_eq!(
            error.to_string(),
            "GTV MVD cannot use the KEX single-view demo protocol"
        );
        let state = fixture.state.borrow();
        assert_eq!(state.gtv_prepared, 1);
        assert_eq!(state.gtv_aborted, 1);
    }

    #[test]
    fn open_gtv_maps_options() {
        let fixture = fixture();
        fixture.state.borrow_mut().products.insert(
            ("q2-classic-baseq2".to_string(), "baseq2".to_string()),
            "q2-classic-baseq2".to_string(),
        );
        let launch = RemoteGtvLaunch {
            label: "gtv:1".to_string(),
            rerelease: false,
            game_directory: "baseq2".to_string(),
            protocol: RemoteQ2Protocol {
                kind: RemoteQ2ProtocolKind::Classic,
                version: 34,
            },
        };
        let app = RemoteApplication::open_gtv_borrowed(live_options(), systems(&fixture), launch).expect("open gtv");
        assert_eq!(app.options().movement, "q2");
        assert_eq!(app.options().character_model, "male");
        assert_eq!(app.time_milliseconds(), 0.0);
    }

    #[test]
    fn content_selection_normalizes_and_rejects() {
        assert_eq!(
            remote_content_selection("q1-quakeworld", "ID1").expect("qw").directory,
            "qw"
        );
        assert_eq!(
            remote_content_selection("q2-classic-baseq2", "").expect("q2").directory,
            "baseq2"
        );
        assert_eq!(
            remote_content_selection("q3-baseq3", "").expect("q3").directory,
            "baseq3"
        );
        assert!(remote_content_selection("q3-baseq3", "../x").is_err());
        assert!(remote_content_selection("q3-baseq3", ".").is_err());
        assert!(remote_content_selection("q3-baseq3", "").is_ok());
    }

    #[test]
    fn qport_allocation_skips_collisions() {
        let fixture = fixture();
        fixture.state.borrow_mut().qport_offsets = VecDeque::from([1, 1, 2]);
        let mut app = open_live(&fixture);
        app.qports.clear();
        assert_eq!(app.allocate_qport(), QPORT_BASE + 1);
        assert_eq!(app.allocate_qport(), QPORT_BASE + 2);
        app.release_qport(QPORT_BASE + 1);
        assert!(!app.qports.contains(&(QPORT_BASE + 1)));
    }

    #[test]
    fn seed_seat_cvars_registers_per_dialect() {
        let netquake = seed_seat_cvars(Dialect::Q1Netquake, 1, "player", true).expect("q1");
        assert_eq!(netquake.get("name").expect("name").value, "Player 2");
        assert_eq!(netquake.get("color").expect("color").value, "0");
        assert!(netquake.get("password").is_some());

        let classic = seed_seat_cvars(Dialect::Q2Classic, 0, "female", false).expect("q2");
        assert_eq!(classic.get("skin").expect("skin").value, "female/athena");
        assert_eq!(classic.get("gender").expect("gender").value, "female");
        assert_eq!(classic.get("rate").expect("rate").value, "25000");

        let qw = seed_seat_cvars(Dialect::Q1Quakeworld, 0, "player", false).expect("qw");
        assert_eq!(qw.get("name").expect("name").value, "unnamed");
        assert_eq!(qw.get("rate").expect("rate").value, "2500");
        assert!(qw.get("password").is_some());
    }

    #[test]
    fn prepare_configuration_seeds_and_publishes() {
        let fixture = fixture();
        let mut app = open_live(&fixture);
        configure(&mut app, &fixture, 2, Dialect::Q2Classic);
        let configuration = app.configuration().expect("configuration");
        assert_eq!(configuration.seats.len(), 2);
        assert_eq!(configuration.seat_sources.len(), 2);
        assert_eq!(configuration.source.get("skin").expect("skin").value, "male/grunt");
    }

    #[test]
    fn cinematic_tickets_stop_complete_and_activate() {
        let fixture = fixture();
        let mut app = open_live(&fixture);
        app.start_cinematic(RemoteCinematicRequest {
            name: "intro".to_string(),
            loop_play: false,
            hold: false,
            silent: false,
        })
        .expect("start");
        assert!(app.cinematic.is_some());
        app.complete_cinematic(1).expect("complete");
        assert!(app.cinematic.is_none());

        app.start_cinematic(RemoteCinematicRequest {
            name: "intro".to_string(),
            loop_play: true,
            hold: false,
            silent: false,
        })
        .expect("start");
        app.stop_cinematic().expect("stop");
        assert!(app.cinematic.is_none());

        fixture.state.borrow_mut().movie_activate_error = Some("activate boom".to_string());
        let error = app
            .start_cinematic(RemoteCinematicRequest {
                name: "intro".to_string(),
                loop_play: false,
                hold: false,
                silent: false,
            })
            .expect_err("error");
        assert_eq!(error.to_string(), "activate boom");
    }

    #[test]
    fn queue_command_derives_seat_and_dispatches() {
        let fixture = fixture();
        let mut app = open_live(&fixture);
        configure(&mut app, &fixture, 1, Dialect::Q1Netquake);
        let seat = fixture.state.borrow().seat.clone();
        let context = CommandContext::new(
            fixture.state.borrow().session.clone(),
            CommandOrigin::Script {
                name: "test".to_string(),
                caller: Box::new(CommandOrigin::LocalSeat {
                    seat: seat.clone(),
                    client: app
                        .configuration()
                        .and_then(|config| config.seats.first().map(|seat| seat.client.clone()))
                        .expect("client"),
                }),
            },
        );
        app.queue_command("quit", &[], None, Some(context)).expect("queue");
        assert_eq!(app.commands.len(), 1);
        assert_eq!(app.commands[0].seat, Some(seat));
        app.dispatch_commands().expect("dispatch");
        assert!(app.stopping);
    }

    #[test]
    fn dispatch_defers_when_blocked_and_routes_borrowed() {
        let fixture = fixture();
        let mut app = open_live(&fixture);
        configure(&mut app, &fixture, 1, Dialect::Q1Netquake);
        fixture.state.borrow_mut().blocked = true;
        app.queue_command("quit", &[], None, None).expect("queue");
        app.dispatch_commands().expect("dispatch");
        assert_eq!(app.commands.len(), 1);
        assert!(!app.stopping);
        fixture.state.borrow_mut().blocked = false;

        let mut app = RemoteApplication::open_borrowed(live_options(), systems(&fixture)).expect("open");
        configure(&mut app, &fixture, 1, Dialect::Q1Netquake);
        app.queue_command("quit", &[], None, None).expect("queue");
        app.dispatch_commands().expect("dispatch");
        assert_eq!(
            fixture.state.borrow().borrowed_calls,
            vec![("quit".to_string(), Vec::new())]
        );
    }

    #[test]
    fn execute_command_validates_sessions_seats_and_usage() {
        let fixture = fixture();
        let mut app = open_live(&fixture);
        configure(&mut app, &fixture, 1, Dialect::Q1Netquake);
        let foreign = IdentityOwner::create("foreign").expect("owner");
        let bad = RemoteCommand {
            name: "quit".to_string(),
            args: Vec::new(),
            seat: None,
            source: Some(CommandContext::new(
                foreign.session().clone(),
                CommandOrigin::LocalConsole,
            )),
        };
        app.execute_remote_command(bad).expect("execute");
        assert!(fixture
            .state
            .borrow()
            .prints
            .iter()
            .any(|line| line.contains("another session")));
        assert!(!app.stopping);

        let stranger = foreign.seat(9);
        let bad = RemoteCommand {
            name: "quit".to_string(),
            args: Vec::new(),
            seat: Some(stranger),
            source: None,
        };
        app.execute_remote_command(bad).expect("execute");
        assert!(fixture
            .state
            .borrow()
            .prints
            .iter()
            .any(|line| line.contains("inactive local seat")));

        app.execute_remote_command(command("local_join", &["x"]))
            .expect("execute");
        assert!(fixture
            .state
            .borrow()
            .prints
            .iter()
            .any(|line| line.contains("Usage: local_join")));

        app.execute_remote_command(command("local_drop", &["x", "y"]))
            .expect("execute");
        assert!(fixture
            .state
            .borrow()
            .prints
            .iter()
            .any(|line| line.contains("Usage: local_drop")));

        app.execute_remote_command(command("map", &[])).expect("execute");
        assert!(fixture
            .state
            .borrow()
            .prints
            .iter()
            .any(|line| line.contains("authoritative server console")));
    }

    #[test]
    fn execute_cinematic_and_recording_commands() {
        let fixture = fixture();
        let mut app = open_live(&fixture);
        configure(&mut app, &fixture, 1, Dialect::Q1Netquake);
        app.execute_remote_command(command("cinematic", &[])).expect("execute");
        assert!(fixture
            .state
            .borrow()
            .prints
            .iter()
            .any(|line| line.contains("Usage: cinematic")));
        app.execute_remote_command(command("cinematic", &["intro", "loop"]))
            .expect("execute");
        assert!(app.cinematic.is_some());
        app.execute_remote_command(command("cinematicpause", &[]))
            .expect("execute");
        assert_eq!(fixture.state.borrow().pauses.len(), 1);
        app.execute_remote_command(command("stopcinematic", &[]))
            .expect("execute");
        assert!(app.cinematic.is_none());

        app.execute_remote_command(command("record", &["a", "b"]))
            .expect("execute");
        assert!(fixture
            .state
            .borrow()
            .prints
            .iter()
            .any(|line| line.contains("Usage: record")));
        app.execute_remote_command(command("rerecord", &[])).expect("execute");
        assert!(fixture
            .state
            .borrow()
            .prints
            .iter()
            .any(|line| line.contains("Usage: rerecord")));
        app.execute_remote_command(command("record", &["demo1"]))
            .expect("execute");
        app.execute_remote_command(command("stop", &[])).expect("execute");
        let calls = fixture.state.borrow().recording_calls.clone();
        assert!(calls
            .iter()
            .any(|(name, arg)| name == "record" && arg.as_deref() == Some("demo1")));
        assert!(calls.iter().any(|(name, _)| name == "stop"));
    }

    #[test]
    fn execute_recorded_only_commands() {
        let fixture = fixture();
        let playback = RemoteRecordedLaunch {
            kind: RemoteDemoKind::Q1,
            path: "demos/test.dm1".to_string(),
            timedemo: false,
            q2_protocol: None,
        };
        let mut app = RemoteApplication::open_demo_borrowed(live_options(), systems(&fixture), playback).expect("open");
        configure(&mut app, &fixture, 1, Dialect::Q1Netquake);
        app.execute_remote_command(command("follow", &["x"])).expect("execute");
        assert!(fixture
            .state
            .borrow()
            .prints
            .iter()
            .any(|line| line.contains("Usage: follow")));
        app.execute_remote_command(command("follow", &["3"])).expect("execute");
        assert!(fixture
            .state
            .borrow()
            .recording_calls
            .iter()
            .any(|(name, _)| name == "select-player"));
        app.execute_remote_command(command("demopause", &[])).expect("execute");
        assert!(fixture
            .state
            .borrow()
            .recording_calls
            .iter()
            .any(|(name, _)| name == "demopause"));

        let mut app = open_live(&fixture);
        configure(&mut app, &fixture, 1, Dialect::Q1Netquake);
        app.execute_remote_command(command("demopause", &[])).expect("execute");
        assert!(fixture
            .state
            .borrow()
            .prints
            .iter()
            .any(|line| line.contains("Demo pause requires active playback")));
    }

    #[test]
    fn execute_download_browser_and_gameplay_commands() {
        let fixture = fixture();
        fixture.state.borrow_mut().primary_progress = vec![RemoteDownloadProgress {
            path: "maps/x.bsp".to_string(),
            phase: "fetch".to_string(),
            received: 4,
            total: Some(8),
            transport: "udp".to_string(),
        }];
        fixture.state.borrow_mut().has_q3_browser = true;
        let mut app = open_live(&fixture);
        configure(&mut app, &fixture, 1, Dialect::Q1Netquake);
        app.execute_remote_command(command("downloadstatus", &[]))
            .expect("execute");
        assert!(fixture
            .state
            .borrow()
            .prints
            .iter()
            .any(|line| line.contains("maps/x.bsp: fetch 4/8")));
        app.execute_remote_command(command("stopdownload", &[]))
            .expect("execute");
        assert_eq!(fixture.state.borrow().cancel_primary, 1);
        app.execute_remote_command(command("retrydownload", &[]))
            .expect("execute");
        app.execute_remote_command(command("localservers", &[]))
            .expect("execute");
        assert!(fixture
            .state
            .borrow()
            .browser_calls
            .iter()
            .any(|(name, _)| name == "localservers"));
        app.execute_remote_command(command("globalservers", &["0"]))
            .expect("execute");
        assert!(fixture
            .state
            .borrow()
            .prints
            .iter()
            .any(|line| line.contains("usage: globalservers")));
        app.execute_remote_command(command("serverstatus", &[]))
            .expect("execute");

        let actor = fixture.owner.actor(2, 1);
        let seat = fixture.state.borrow().seat.clone();
        fixture.state.borrow_mut().players = vec![RemoteLocalPlayer {
            seat,
            actor: actor.clone(),
        }];
        fixture.state.borrow_mut().q3_result = false;
        app.execute_remote_command(command("kill", &[])).expect("execute");
        assert_eq!(fixture.state.borrow().player_commands.len(), 1);
        app.execute_remote_command(command("centerview", &[])).expect("execute");
        assert_eq!(fixture.state.borrow().centerviews, vec![actor.clone()]);
        fixture.state.borrow_mut().use_ordinals.insert("shells".to_string(), 3);
        fixture.state.borrow_mut().q3_result = true;
        app.execute_remote_command(command("use", &["shells"]))
            .expect("execute");
        assert!(fixture
            .state
            .borrow()
            .q3_calls
            .iter()
            .any(|argv| argv == &vec!["weapon".to_string(), "3".to_string()]));
    }

    #[test]
    fn execute_quakeworld_color_and_skins() {
        let fixture = fixture();
        fixture.state.borrow_mut().expectations.insert(
            "q1-classic-id1".to_string(),
            RemoteProductExpectation {
                id: "q1-classic-id1".to_string(),
                family: "q1".to_string(),
                rerelease: false,
            },
        );
        let mut options = live_options();
        options.network.kind = RemoteNetworkKind::QwClient;
        let mut app = RemoteApplication::open(options, systems(&fixture)).expect("open");
        configure(&mut app, &fixture, 1, Dialect::Q1Quakeworld);
        app.execute_remote_command(command("color", &[])).expect("execute");
        assert!(fixture
            .state
            .borrow()
            .prints
            .iter()
            .any(|line| line.contains("\"color\" is \"0 0\"")));
        app.execute_remote_command(command("color", &["14", "2"]))
            .expect("execute");
        let configuration = app.configuration().expect("configuration");
        assert_eq!(configuration.source.get("topcolor").expect("top").value, "13");
        assert_eq!(configuration.source.get("bottomcolor").expect("bottom").value, "2");
        app.execute_remote_command(command("allskins", &["x"]))
            .expect("execute");
        assert!(fixture
            .state
            .borrow()
            .recording_calls
            .iter()
            .any(|(name, _)| name == "refresh-skins"));
    }

    #[test]
    fn peers_prepare_retire_and_report() {
        let fixture = fixture();
        let mut app = open_live(&fixture);
        configure(&mut app, &fixture, 2, Dialect::Q1Netquake);
        app.prepare_remote_seats().expect("prepare");
        assert_eq!(app.peers().len(), 1);
        let id = app.peers()[0].id;
        assert!(!app.peer_world_current(id));
        app.peers.iter_mut().find(|peer| peer.id == id).expect("peer").map = Some(app.options().map.clone());
        app.peers
            .iter_mut()
            .find(|peer| peer.id == id)
            .expect("peer")
            .has_loaded_content = true;
        assert!(app.peer_world_current(id));
        app.retire_remote_views().expect("retire views");
        app.retire_remote_seat(id, false).expect("retire");
        assert!(app.peers().is_empty());
        assert!(app.retire_remote_seat(999, false).is_ok());
        assert_eq!(app.read_pixels(), vec![1, 2, 3]);
        assert_eq!(app.capture_next_frame(), vec![1, 2, 3]);
    }

    #[test]
    fn peer_open_failure_releases_qport() {
        let fixture = fixture();
        let mut app = open_live(&fixture);
        configure(&mut app, &fixture, 2, Dialect::Q1Netquake);
        fixture.state.borrow_mut().open_peer_error = Some("peer boom".to_string());
        let qports = app.qports.len();
        let error = app.prepare_remote_seats().expect_err("error");
        assert_eq!(error.to_string(), "peer boom");
        assert_eq!(app.qports.len(), qports);
        assert_eq!(fixture.state.borrow().aborted_peers.len(), 1);
    }

    #[test]
    fn content_selection_suspends_and_publishes() {
        let fixture = fixture();
        let mut app = open_live(&fixture);
        configure(&mut app, &fixture, 1, Dialect::Q1Netquake);
        fixture.state.borrow_mut().mounts_changed = true;
        fixture.state.borrow_mut().products.insert(
            ("q1-classic-id1".to_string(), "rogue".to_string()),
            "q1-classic-rogue".to_string(),
        );
        let selection = RemoteContentSelection {
            base: "q1-classic-id1".to_string(),
            directory: "rogue".to_string(),
        };
        let error = app.select_remote_content(selection.clone()).expect_err("error");
        assert_eq!(error.to_string(), "Peer directory admission has no active source frame");

        app.pending_advance = Some(PendingAdvance { boundary: false });
        assert!(app.select_remote_content(selection).expect("select"));
        assert!(app.peer_admission.as_ref().is_some_and(|admission| admission.suspended));
        assert!(app.pending_advance.is_some_and(|pending| pending.boundary));
        app.flush_client_commands().expect("flush");
        assert_eq!(app.options().product, "q1-classic-rogue");
        app.resume_peer_admission();
        assert!(app.peer_admission.is_none());
        assert!(app.refresh_remote_content().is_ok());
    }

    #[test]
    fn client_input_capture_defers_to_frame() {
        let fixture = fixture();
        let mut app = open_live(&fixture);
        configure(&mut app, &fixture, 1, Dialect::Q1Netquake);
        let seat = fixture.state.borrow().seat.clone();
        assert!(!app.capture_client_input(&seat, 9));
        fixture.state.borrow_mut().input_clients.insert(seat.clone(), 42);
        fixture.state.borrow_mut().live_clients.insert(42);
        assert!(app.capture_client_input(&seat, 9));
        assert!(app.step(16.0).expect("step"));
        assert_eq!(fixture.state.borrow().delivered, vec![(42, 9)]);
    }

    #[test]
    fn advance_source_honors_boundaries_and_admissions() {
        let fixture = fixture();
        let mut app = open_live(&fixture);
        assert!(app.advance_source(SourceAdvanceOp::PollPrimary).expect("advance"));
        assert!(app.pending_advance.is_none());
        assert_eq!(fixture.state.borrow().advance_calls, vec![SourceAdvanceOp::PollPrimary]);

        app.pending_advance = Some(PendingAdvance { boundary: false });
        app.peer_admission = Some(PeerDirectoryAdmission {
            phase: PeerAdmissionPhase::Requested,
            options: live_options(),
            selection: RemoteContentSelection {
                base: "q1-classic-id1".to_string(),
                directory: "id1".to_string(),
            },
            suspended: true,
        });
        assert!(!app.advance_source(SourceAdvanceOp::PollPrimary).expect("advance"));

        app.peer_admission.as_mut().expect("admission").phase = PeerAdmissionPhase::Published;
        assert!(app.advance_source(SourceAdvanceOp::PollPrimary).expect("advance"));
        assert!(app.peer_admission.is_none());
    }

    #[test]
    fn seats_join_and_drop_with_guards() {
        let fixture = fixture();
        let mut app = open_live(&fixture);
        configure(&mut app, &fixture, 2, Dialect::Q1Netquake);
        app.join_remote_seat().expect("join");
        assert_eq!(app.options().seats, 2);
        assert_eq!(app.peers().len(), 1);

        let error = app.drop_remote_seat(9).expect_err("error");
        assert_eq!(error.to_string(), "That player is not connected");
        let peer_index = app.peers()[0].seat.index();
        app.drop_remote_seat(peer_index).expect("drop");
        assert_eq!(app.options().seats, 1);
        let error = app.drop_remote_seat(0).expect_err("error");
        assert_eq!(error.to_string(), "Use disconnect to leave the last connection");
    }

    #[test]
    fn step_guards_frames_cinematics_and_sessions() {
        let fixture = fixture();
        let mut app = open_live(&fixture);
        configure(&mut app, &fixture, 1, Dialect::Q1Netquake);
        assert!(app.step(0.0).is_err());
        assert!(app.step(f64::NAN).is_err());
        app.stepping = true;
        assert!(app.step(16.0).is_err());
        app.stepping = false;

        assert!(app.step(16.0).expect("step"));
        assert_eq!(app.frame_count(), 1);
        assert_eq!(fixture.state.borrow().submit_calls, vec![false]);

        app.start_cinematic(RemoteCinematicRequest {
            name: "intro".to_string(),
            loop_play: false,
            hold: false,
            silent: false,
        })
        .expect("start");
        assert!(!app.step(16.0).expect("step"));
        assert_eq!(fixture.state.borrow().movie_frames.len(), 1);

        fixture.state.borrow_mut().phase = RemoteNetworkPhase::Closed;
        assert!(!app.step(16.0).expect("step"));
        assert!(app.stopping);
    }

    #[test]
    fn step_reports_effect_gaps_and_session_actions() {
        let fixture = fixture();
        let mut app = open_live(&fixture);
        configure(&mut app, &fixture, 1, Dialect::Q1Netquake);
        let seat = fixture.state.borrow().seat.clone();
        fixture.state.borrow_mut().present_frame = Ok(RemoteFramePresentation {
            events: vec![RemotePresentationEvent {
                content: "e1m1".to_string(),
                map: "maps/e1m1.bsp".to_string(),
                recipient: None,
                departures: vec![seat.clone()],
                level_completion: Some("done".to_string()),
            }],
            unhandled: vec![RemoteUnhandledEffect {
                content: "fx".to_string(),
                kind: "particle".to_string(),
                reason: "missing".to_string(),
            }],
            batches: vec![RemoteAudioBatch {
                seat: seat.clone(),
                events: vec![RemotePresentationEvent {
                    content: "e1m1".to_string(),
                    map: "maps/e1m1.bsp".to_string(),
                    recipient: None,
                    departures: vec![seat.clone()],
                    level_completion: Some("done".to_string()),
                }],
            }],
        });
        assert!(app.step(16.0).expect("step"));
        assert_eq!(app.presentation_events().len(), 1);
        assert_eq!(app.unhandled_presentation_effects().len(), 1);
        assert!(fixture
            .state
            .borrow()
            .prints
            .iter()
            .any(|line| line.contains("Unresolved particle effect: missing")));
        assert_eq!(fixture.state.borrow().completions.len(), 1);
        assert!(app.stopping);
    }

    #[test]
    fn run_loops_until_frame_limit() {
        let fixture = fixture();
        let mut options = live_options();
        options.frame_limit = Some(2);
        let mut app = RemoteApplication::open(options, systems(&fixture)).expect("open");
        configure(&mut app, &fixture, 1, Dialect::Q1Netquake);
        fixture.state.borrow_mut().now = 0.0;
        app.run().expect("run");
        assert_eq!(app.frame_count(), 2);
        assert!(app.finished());
    }

    #[test]
    fn close_aggregates_and_is_idempotent() {
        let fixture = fixture();
        let mut app = open_live(&fixture);
        configure(&mut app, &fixture, 1, Dialect::Q1Netquake);
        fixture.state.borrow_mut().content_bundle_failures = vec!["content boom".to_string()];
        fixture.state.borrow_mut().shutdown_q3_error = Some("q3 boom".to_string());
        let error = app.close().expect_err("error");
        match error {
            RemoteApplicationError::Aggregate { message, errors } => {
                assert_eq!(message, "Remote application shutdown failed");
                assert_eq!(errors, vec!["q3 boom".to_string(), "content boom".to_string()]);
            }
            _ => panic!("expected aggregate"),
        }
        assert!(app.close().is_ok());
        assert_eq!(
            fixture.state.borrow().unregistered,
            vec![
                "localservers".to_string(),
                "globalservers".to_string(),
                "ping".to_string(),
                "serverstatus".to_string(),
            ]
        );
    }

    #[test]
    fn recording_guards_and_signon_attach() {
        let fixture = fixture();
        let mut app = open_live(&fixture);
        configure(&mut app, &fixture, 1, Dialect::Q1Netquake);
        let context = CommandContext::new(fixture.state.borrow().session.clone(), CommandOrigin::LocalConsole);
        assert!(app.check_recording(&context, None).expect("check").is_none());
        fixture.state.borrow_mut().primary_recorder = false;
        let error = app.check_recording(&context, None).expect_err("error");
        assert_eq!(error.to_string(), "The selected native protocol has no demo recorder");

        let error = app.attach_signon_recording().expect_err("error");
        assert_eq!(
            error.to_string(),
            "Signon recording requires a live QuakeWorld connection"
        );
        fixture.state.borrow_mut().is_qw = true;
        let attachment = app.attach_signon_recording().expect("attach");
        app.detach_signon_recording(attachment);
        assert_eq!(fixture.state.borrow().detached, vec![attachment.attachment]);
    }

    #[test]
    fn input_routes_to_movie_controls_or_fails_closed() {
        let fixture = fixture();
        let mut app = open_live(&fixture);
        assert!(!app.input(7).expect("input"));
        app.start_cinematic(RemoteCinematicRequest {
            name: "intro".to_string(),
            loop_play: false,
            hold: false,
            silent: false,
        })
        .expect("start");
        assert!(app.input(7).expect("input"));
        assert_eq!(fixture.state.borrow().movie_inputs.len(), 1);
        app.close().expect("close");
        assert!(app.input(7).is_err());
        assert!(app.queue_command("quit", &[], None, None).is_err());
    }

    #[test]
    fn remote_helpers_cover_origins_atoi_and_protocols() {
        let origin = CommandOrigin::Script {
            name: "a".to_string(),
            caller: Box::new(CommandOrigin::LocalConsole),
        };
        assert!(matches!(unwrap_origin(&origin), CommandOrigin::LocalConsole));
        assert_eq!(remote_atoi("  -42x"), -42);
        assert_eq!(remote_atoi("+7"), 7);
        assert_eq!(remote_atoi("abc"), 0);
        assert_eq!(remote_atoi(""), 0);
        assert_eq!(
            live_q2_protocol(true),
            RemoteQ2Protocol {
                kind: RemoteQ2ProtocolKind::Kex,
                version: 2023
            }
        );
        assert_eq!(
            live_q2_protocol(false),
            RemoteQ2Protocol {
                kind: RemoteQ2ProtocolKind::Classic,
                version: 34
            }
        );
        assert_eq!(QPORT_BASE, 10_000);
        assert_eq!(QPORT_SPAN, 30_000);
    }
}
