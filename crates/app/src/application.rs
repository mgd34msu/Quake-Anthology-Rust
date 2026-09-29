//! Host application: the fixed-timestep main loop driving the headless
//! server plus the headless client core.
//!
//! Donor provenance: `src/app/bootstrap/application.ts` (`openApplication`,
//! `requestQuit`, `run`, `close`), `src/app/bootstrap/frame-time.ts`
//! (frame pacing), and `src/main.ts` (dedicated vs windowed assembly).
//! Each host frame queues one synthetic command per bound seat, ticks the
//! [`Server`](qa_world::server::Server) with the configured fixed step,
//! submits the bodies as scene entities to a
//! [`RendererBackend`](qa_client::render::RendererBackend), and refreshes
//! the [`ChannelPool`](qa_client::audio::ChannelPool) spatialization from
//! live body positions. Per-frame scratch (`scratch_entities`) is cleared
//! and reused; the loop makes no per-frame allocations of its own beyond
//! what the world and client APIs return.

use qa_client::audio::ChannelPool;
use qa_client::prediction::CommandRing;
use qa_client::render::{FrameStats as RenderFrameStats, ModelPose, RenderView, RendererBackend, SceneEntity};
use qa_client::view::{CameraClip, ModelTransform, Rect, SceneCamera};
use qa_core::cmd::Dialect;
use qa_core::cmd_buffer::{CommandContext, CommandOrigin};
use qa_core::cvar::CvarRegistry;
use qa_core::identity::IdentityOwner;
use qa_core::math::{vec3, vec4, Axis, Vec3};
use qa_core::rng::Qrand;
use qa_core::time::SourceTime;
use qa_world::client::{apply_scalar, ClientCommand, ClientFamily, ScalarInput};
use qa_world::server::{NullLogic, Server};

use crate::console::commands::{register_console_commands, ConsoleCommandServices, ConsoleCommands};
use crate::console::queue::ConsoleQueue;
use crate::error::AppError;
use crate::startup::{load_stub_map, open_server, spawn_stub_map, StartupConfig};

/// Registry slot sentinel for an unbound seat.
pub const UNBOUND_SEAT: u32 = u32::MAX;

/// One host frame's outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostFrame {
    /// Host frame number (0-based).
    pub frame: u64,
    /// Server fixed frames executed this host frame.
    pub server_frames: u32,
    /// Server events produced this host frame.
    pub server_events: usize,
    /// Renderer submission counts.
    pub render: RenderFrameStats,
}

/// Totals for a completed run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunStats {
    /// Host frames executed.
    pub frames: u64,
    /// Server fixed frames executed.
    pub ticks: u64,
    /// Live actors at the end of the run.
    pub entities: usize,
    /// Renderer frames submitted.
    pub render_frames: u64,
}

/// One headless local seat: command history plus its bound actor slot.
#[derive(Debug)]
pub struct HeadlessSeat {
    /// Registry slot of the seat's actor, or [`UNBOUND_SEAT`].
    pub slot: u32,
    history: CommandRing<ClientCommand>,
}

impl HeadlessSeat {
    fn new(family: ClientFamily, slot: u32) -> Self {
        Self {
            slot,
            history: CommandRing::new(ClientCommand::zero(family)),
        }
    }

    /// Newest recorded command number.
    #[must_use]
    pub fn current_number(&self) -> i32 {
        self.history.current_number()
    }
}

/// Host application over a renderer backend.
pub struct Application<R: RendererBackend> {
    server: Server<NullLogic>,
    renderer: R,
    audio: ChannelPool,
    seats: Vec<HeadlessSeat>,
    rng: Qrand,
    step: SourceTime,
    frame_limit: Option<u64>,
    frames: u64,
    ticks: u64,
    render_frames: u64,
    finished: bool,
    width: i32,
    height: i32,
    client_family: ClientFamily,
    scratch_entities: Vec<SceneEntity>,
    console_owner: IdentityOwner,
    console_queue: ConsoleQueue,
    console_commands: ConsoleCommands,
    console_cvars: CvarRegistry,
    console_log: Vec<String>,
    console_forwarded: Vec<String>,
    map_name: String,
}

/// Headless console services: prints and server forwards land in the
/// application log vectors.
struct AppConsoleServices<'a> {
    log: &'a mut Vec<String>,
    forwarded: &'a mut Vec<String>,
    map: &'a str,
}

impl ConsoleCommandServices for AppConsoleServices<'_> {
    fn print(&mut self, text: &str) {
        self.log.push(text.to_string());
    }

    fn forward_to_server(&mut self, line: &str) {
        self.forwarded.push(line.to_string());
    }

    fn toggle_console(&mut self) {}

    fn clear_console(&mut self) {
        self.log.clear();
    }

    fn message_mode(&mut self, _team: bool) {}

    fn can_chat(&self) -> bool {
        false
    }

    fn console_dump(&self) -> String {
        self.log.concat()
    }

    fn write_file(&mut self, _path: &str, _contents: &str) -> Result<(), String> {
        Err("Headless application has no writable console root".to_string())
    }

    fn configuration_text(&mut self, _argv: &[String]) -> String {
        String::new()
    }

    fn map_name(&self) -> String {
        self.map.to_string()
    }
}

fn console_dialect_for(family: ClientFamily) -> Dialect {
    match family {
        ClientFamily::Q1Netquake => Dialect::Q1Netquake,
        ClientFamily::Q1Quakeworld => Dialect::Q1Quakeworld,
        ClientFamily::Q2Classic => Dialect::Q2Classic,
        ClientFamily::Q2Rerelease => Dialect::Q2Rerelease,
        ClientFamily::Q3 => Dialect::Q3,
    }
}

impl<R: RendererBackend> Application<R> {
    /// Assemble an application: open the server, spawn the stub map, and
    /// bind seats to player actors in spawn order (seat `i` takes the
    /// `i`-th player start; missing actors leave the seat unbound).
    pub fn open(config: &StartupConfig, renderer: R) -> Result<Self, AppError> {
        let mut server = open_server(config)?;
        let stub = load_stub_map(&config.map);
        let actors = spawn_stub_map(&mut server, &stub)?;
        let mut seats = Vec::with_capacity(config.seats as usize);
        for index in 0..config.seats {
            let slot = actors
                .get(index as usize + 1)
                .map_or(UNBOUND_SEAT, |actor| actor.id().slot());
            seats.push(HeadlessSeat::new(config.client_family, slot));
        }
        let console_owner = IdentityOwner::create(&format!("{}:console", config.session_name))
            .map_err(|error| AppError::Startup(error.to_string()))?;
        let dialect = console_dialect_for(config.client_family);
        let console_queue = ConsoleQueue::new(
            dialect,
            CommandContext::new(console_owner.session().clone(), CommandOrigin::LocalConsole),
        )
        .map_err(|error| AppError::Console(error.to_string()))?;
        let mut console_commands = ConsoleCommands::new();
        register_console_commands(&mut console_commands);
        Ok(Self {
            server,
            renderer,
            audio: ChannelPool::new(),
            seats,
            rng: Qrand::new(config.seed as i32, 0),
            step: config.step,
            frame_limit: config.frame_limit,
            frames: 0,
            ticks: 0,
            render_frames: 0,
            finished: false,
            width: config.width as i32,
            height: config.height as i32,
            client_family: config.client_family,
            scratch_entities: Vec::with_capacity(actors.len()),
            console_owner,
            console_queue,
            console_commands,
            console_cvars: CvarRegistry::new(dialect),
            console_log: Vec::new(),
            console_forwarded: Vec::new(),
            map_name: config.map.clone(),
        })
    }

    /// Request shutdown (donor `requestQuit`); the run loop stops after the
    /// current frame.
    pub fn request_quit(&mut self) {
        self.finished = true;
    }

    /// Whether the application has finished.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// Borrow the server.
    #[must_use]
    pub fn server(&self) -> &Server<NullLogic> {
        &self.server
    }

    /// Borrow the server mutably.
    pub fn server_mut(&mut self) -> &mut Server<NullLogic> {
        &mut self.server
    }

    /// Attach a bot command source to the server tick.
    pub fn set_bot_source(&mut self, source: Option<Box<dyn qa_world::server::BotCommandSource>>) {
        self.server.set_bot_source(source);
    }

    /// Borrow the renderer.
    #[must_use]
    pub fn renderer(&self) -> &R {
        &self.renderer
    }

    /// Borrow the audio channel pool.
    #[must_use]
    pub fn audio(&self) -> &ChannelPool {
        &self.audio
    }

    /// Local seats.
    #[must_use]
    pub fn seats(&self) -> &[HeadlessSeat] {
        &self.seats
    }

    /// Completed host frames.
    #[must_use]
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// Executed server fixed frames.
    #[must_use]
    pub fn ticks(&self) -> u64 {
        self.ticks
    }

    /// Queue console text on the host program; [`Application::step_frame`]
    /// drains one frame of it per host frame.
    pub fn submit_console(&mut self, text: &str) -> Result<(), AppError> {
        self.console_queue
            .submit(text)
            .map_err(|error| AppError::Console(error.to_string()))
    }

    /// Queue console text as a local seat's input.
    pub fn submit_console_as_seat(&mut self, seat: usize, text: &str) -> Result<(), AppError> {
        if seat >= self.seats.len() {
            return Err(AppError::Console(format!("No console seat {seat}")));
        }
        let source = CommandContext::new(
            self.console_owner.session().clone(),
            CommandOrigin::LocalSeat {
                seat: self.console_owner.seat(seat as u32),
                client: self.console_owner.client(seat as u32, 0),
            },
        );
        self.console_queue
            .submit_as(text, &source)
            .map_err(|error| AppError::Console(error.to_string()))
    }

    /// Supply `exec` script text (`None` means missing).
    pub fn provide_console_script(&mut self, name: &str, text: Option<String>) {
        self.console_queue.set_script(name, text);
    }

    /// Park an `exec` script name until [`Application::provide_console_script`]
    /// supplies it.
    pub fn stage_console_script(&mut self, name: &str) {
        self.console_queue.stage_pending_script(name);
    }

    /// Console output lines collected so far.
    #[must_use]
    pub fn console_log(&self) -> &[String] {
        &self.console_log
    }

    /// Lines the console forwarded to the server so far.
    #[must_use]
    pub fn console_forwarded(&self) -> &[String] {
        &self.console_forwarded
    }

    /// Console program revision.
    #[must_use]
    pub fn console_program_revision(&self) -> u64 {
        self.console_queue.buffer().program_revision()
    }

    /// Whether the console program still holds queued work.
    #[must_use]
    pub fn console_has_pending(&self) -> bool {
        self.console_queue.buffer().has_pending_commands()
    }

    /// Current console variable text, or empty when unregistered.
    #[must_use]
    pub fn console_cvar(&self, name: &str) -> String {
        self.console_cvars.variable_string(name)
    }

    /// Run host frames until quit is requested or the frame limit lands.
    pub fn run(&mut self) -> Result<RunStats, AppError> {
        while !self.finished {
            self.step_frame()?;
        }
        Ok(RunStats {
            frames: self.frames,
            ticks: self.ticks,
            entities: self.server.simulation().actor_count(),
            render_frames: self.render_frames,
        })
    }

    /// Run one host frame: console drain, seat commands, server tick,
    /// scene submission, and audio refresh.
    pub fn step_frame(&mut self) -> Result<HostFrame, AppError> {
        if self.finished {
            return Err(AppError::Finished);
        }
        self.drive_console()?;
        let frame = self.frames;
        for seat in &mut self.seats {
            if seat.slot == UNBOUND_SEAT {
                continue;
            }
            let command = synthetic_command(self.client_family, frame, &mut self.rng);
            seat.history.append(command);
            self.server.queue_client(seat.slot, command)?;
        }
        let tick = self.server.tick(self.step)?;
        self.ticks += u64::from(tick.frames);

        self.submit_scene();
        self.refresh_audio();

        self.frames += 1;
        if self.frame_limit.is_some_and(|limit| self.frames >= limit) {
            self.finished = true;
        }
        Ok(HostFrame {
            frame,
            server_frames: tick.frames,
            server_events: tick.events.len(),
            render: RenderFrameStats {
                entities: self.scratch_entities.len(),
                particles: 0,
                decals: 0,
                lights: 0,
            },
        })
    }

    fn drive_console(&mut self) -> Result<(), AppError> {
        let mut services = AppConsoleServices {
            log: &mut self.console_log,
            forwarded: &mut self.console_forwarded,
            map: &self.map_name,
        };
        self.console_queue
            .drive_frame(
                &mut self.console_commands,
                &mut self.console_cvars,
                &mut services,
                &mut || {},
            )
            .map_err(|error| AppError::Console(error.to_string()))?;
        Ok(())
    }

    fn submit_scene(&mut self) {
        self.scratch_entities.clear();
        let axis = identity_axis();
        for actor in self.server.simulation().body_actors() {
            let Some(state) = self.server.simulation().body_state(&actor) else {
                continue;
            };
            self.scratch_entities.push(SceneEntity {
                entity_number: actor.slot(),
                model: 0,
                transform: ModelTransform {
                    origin: state.origin,
                    axis,
                    scale: 1.0,
                },
                previous_origin: state.origin,
                pose: ModelPose::default(),
                skin: 0,
                color: vec4(1.0, 1.0, 1.0, 1.0),
                flags: 0,
                lighting_origin: None,
                shadow_plane: None,
                opacity: None,
            });
        }
        let view = RenderView {
            camera: SceneCamera {
                origin: listener_origin(self),
                axis,
                projection: identity_matrix(),
                viewport: Rect {
                    x: 0,
                    y: 0,
                    width: self.width,
                    height: self.height,
                },
                clip: CameraClip::None,
            },
            time_ms: self.server.simulation().frame().time.as_milliseconds_truncated(),
            flags: 0,
        };
        self.renderer.begin_frame(&view);
        self.renderer.submit_entities(&self.scratch_entities);
        self.renderer.submit_particles(&[]);
        self.renderer.submit_decals(&[]);
        self.renderer.submit_lights(&[]);
        self.renderer.end_frame();
        self.render_frames += 1;
    }

    fn refresh_audio(&mut self) {
        for actor in self.server.simulation().body_actors() {
            if let Some(state) = self.server.simulation().body_state(&actor) {
                let _ = self.audio.set_entity_position(actor.slot(), state.origin);
            }
        }
        self.audio.set_listener(0, listener_origin(self), identity_axis());
        self.audio.update_volumes();
    }
}

fn identity_axis() -> Axis {
    [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)]
}

fn identity_matrix() -> [f32; 16] {
    [
        1.0, 0.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0, //
        0.0, 0.0, 0.0, 1.0, //
    ]
}

fn listener_origin<R: RendererBackend>(application: &Application<R>) -> Vec3 {
    application
        .seats
        .first()
        .and_then(|seat| {
            if seat.slot == UNBOUND_SEAT {
                return None;
            }
            application.server.simulation().actor_by_slot(seat.slot)
        })
        .and_then(|actor| application.server.simulation().body_state(actor.id()))
        .map_or(vec3(0.0, 0.0, 0.0), |state| state.origin)
}

/// Deterministic synthetic input for headless seats: a forward ramp over a
/// 21-frame cycle, a small seeded strafe jitter, attack every 30 frames,
/// and jump every 45 frames.
fn synthetic_command(family: ClientFamily, frame: u64, rng: &mut Qrand) -> ClientCommand {
    let base = ClientCommand::zero(family);
    let forward = (frame % 21) as f64 / 10.0 - 1.0;
    let command = apply_scalar(&base, ScalarInput::ForwardMove, forward).unwrap_or(base);
    let command = apply_scalar(&command, ScalarInput::SideMove, rng.next_centered() * 0.25).unwrap_or(command);
    let command = if frame.is_multiple_of(30) {
        apply_scalar(&command, ScalarInput::Attack, 1.0).unwrap_or(command)
    } else {
        command
    };
    if frame.is_multiple_of(45) {
        apply_scalar(&command, ScalarInput::Jump, 1.0).unwrap_or(command)
    } else {
        command
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::{parse_application_command, ApplicationCommand};
    use qa_client::render::NullRenderer;

    fn config(words: &[&str]) -> StartupConfig {
        let argv: Vec<String> = words.iter().map(|word| (*word).to_string()).collect();
        let options = match parse_application_command(&argv).unwrap() {
            ApplicationCommand::Run { options } | ApplicationCommand::Menu { options } => options,
            other => panic!("expected run or menu, got {other:?}"),
        };
        StartupConfig::from_options(&options).unwrap()
    }

    #[test]
    fn seats_bind_to_player_actors() {
        let application = Application::open(&config(&["--movement", "q1"]), NullRenderer::new()).unwrap();
        assert_eq!(application.seats().len(), 1);
        assert_ne!(application.seats()[0].slot, UNBOUND_SEAT);

        let application =
            Application::open(&config(&["--dedicated", "--movement", "q1"]), NullRenderer::new()).unwrap();
        assert!(application.seats().is_empty());
        assert!(!application.is_finished());
    }

    #[test]
    fn frame_limit_stops_the_loop() {
        let mut application =
            Application::open(&config(&["--movement", "q1", "--frames", "5"]), NullRenderer::new()).unwrap();
        let stats = application.run().unwrap();
        assert_eq!(stats.frames, 5);
        assert_eq!(stats.render_frames, 5);
        assert!(stats.ticks >= 5);
        assert_eq!(stats.entities, 5);
        assert!(application.is_finished());
        assert_eq!(application.step_frame(), Err(AppError::Finished));
    }

    #[test]
    fn request_quit_stops_after_current_frame() {
        let mut application = Application::open(&config(&["--movement", "q1"]), NullRenderer::new()).unwrap();
        let first = application.step_frame().unwrap();
        assert_eq!(first.frame, 0);
        assert!(first.server_frames >= 1);
        assert!(!application.is_finished());
        application.request_quit();
        let stats = application.run().unwrap();
        assert_eq!(stats.frames, 1);
    }

    #[test]
    fn loop_drives_server_renderer_and_audio() {
        let mut application =
            Application::open(&config(&["--movement", "q3", "--seed", "7"]), NullRenderer::new()).unwrap();
        let frame = application.step_frame().unwrap();
        assert_eq!(application.ticks(), u64::from(frame.server_frames));
        assert_eq!(frame.render.entities, 5);
        assert_eq!(application.renderer().frames(), 1);
        assert_eq!(application.renderer().entities.len(), 5);
        assert_eq!(application.seats()[0].current_number(), 1);
        assert!(application.server().simulation().frame().frame >= 1);
    }

    #[test]
    fn synthetic_input_is_deterministic() {
        let mut first = Qrand::new(9, 0);
        let mut second = Qrand::new(9, 0);
        for frame in 0..60 {
            let left = synthetic_command(ClientFamily::Q1Netquake, frame, &mut first);
            let right = synthetic_command(ClientFamily::Q1Netquake, frame, &mut second);
            assert_eq!(left, right);
        }
        let mut rng = Qrand::new(9, 0);
        let attack = synthetic_command(ClientFamily::Q1Netquake, 0, &mut rng);
        assert!(attack.attack_pressed());
        let rest = synthetic_command(ClientFamily::Q1Netquake, 1, &mut rng);
        assert!(!rest.attack_pressed());
    }
}
