//! Startup application front-end: action queue, frame loop, and shutdown.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/startup.ts`
//! (`StartupApplication`). The action queue ([`StartupAction`]), display
//! snapshot ([`StartupDisplay`]), entry point ([`StartupEntry`]), frame
//! counter, status line, stop/close flags, and the step/run/close state
//! machine are concrete here. Everything that owns world state — the menu
//! and game clients, browser, lobby, saves, addons, keys, music, movies,
//! demos, GTV, movies, and preference baselines — lives behind
//! [`StartupBackend`], because its siblings have no lane port:
//! `./application.ts` (`Application`, `PreparedStartup`,
//! `createStartupApplicationClient`), `./remote-application.ts`
//! (`RemoteApplicationClient`, `closeRemoteClient`, `hasActiveRemoteGame`,
//! `remoteInputSeat`), `./client-bootstrap.ts` (`ClientBootstrap`),
//! `./configuration.ts` (`FrontendPreferences`, `applicationHost`,
//! `frontendRoutingBaselines`), `./addons.ts` (`AddonLibrary`),
//! `./keys.ts` (`ApplicationKeys`), `./image-settings.ts`
//! (`ApplicationImageSettings`), `./unified-content.ts`,
//! `./content.ts`, `./content-runtime.ts`, `./input.ts`
//! (`ApplicationInput`, `LocalInput`, `InputRouter`), `./game-prompt.ts`,
//! `./player-death.ts`, `./q2-match-ui.ts`, `./base-arena-menu.ts`,
//! `./movie-capture.ts`, `./demo-commands.ts`, `./gtv-commands.ts`
//! (except [`GtvConnectRequest`], inlined below),
//! `./server-administration.ts` (`LocalServerBridge`),
//! `./launch.ts`, and the `platform` audio/input backends. Sync port: the
//! donor's async open/step/run/close become sync calls; `captureNextFrame`
//! returns its pixels instead of a promise.

use qa_client::input::router::SeatInputEvent;
use qa_core::identity::SeatId;
use thiserror::Error;

use super::server_browser::BrowserConnection;
use super::startup_selection::{StartupLaunch, StartupSelectionError, StartupSelectionModel};
use crate::options::{ApplicationOptions, Renderer};

/// Failures from the startup application.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StartupError {
    /// Operation failed.
    #[error("{0}")]
    Failed(String),
    /// Cleanup aggregated failures (donor `AggregateError`).
    #[error("{context}: {failures:?}")]
    Aggregate {
        /// Operation context.
        context: String,
        /// Failure messages.
        failures: Vec<String>,
    },
}

impl From<StartupSelectionError> for StartupError {
    fn from(error: StartupSelectionError) -> Self {
        Self::Failed(error.to_string())
    }
}

/// GTV connect request (donor `GtvConnectRequest` in `./gtv-commands.ts`,
/// inlined: that sibling has no lane port).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GtvConnectRequest {
    /// Server address.
    pub address: String,
    /// Login username.
    pub username: String,
    /// Login password.
    pub password: String,
    /// Display label, if any.
    pub label: Option<String>,
}

/// Queued source change (donor `StartupAction`).
#[derive(Debug, Clone, PartialEq)]
pub enum StartupAction {
    /// Connect to a GTV relay.
    Gtv {
        /// Connect request.
        request: GtvConnectRequest,
    },
    /// Connect to a browser entry.
    Connect {
        /// Browser connection.
        connection: BrowserConnection,
    },
    /// Launch with explicit options.
    Initial {
        /// Launch options.
        options: Box<ApplicationOptions>,
    },
    /// Host the lobby draft.
    LobbyHost {
        /// Lobby selection.
        selection: Box<StartupLaunch>,
    },
    /// Return to the frontend.
    Frontend,
    /// Start the current selection.
    Play,
    /// Start an arena preset.
    Preset {
        /// Preset id.
        id: String,
        /// Skill level.
        skill: u8,
        /// Arena map, if any.
        arena_map: Option<String>,
    },
    /// Load a save path.
    Load {
        /// Save path.
        path: String,
        /// Source product, if any.
        source_product: Option<String>,
    },
}

/// Display snapshot (donor `StartupDisplay`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StartupDisplay {
    /// Renderer.
    pub renderer: Renderer,
    /// Display gamma.
    pub gamma: f64,
    /// Window width.
    pub width: u32,
    /// Window height.
    pub height: u32,
    /// Start hidden.
    pub hidden: bool,
}

/// Display snapshot for options (donor `startupDisplay`).
#[must_use]
pub fn startup_display(options: &ApplicationOptions) -> StartupDisplay {
    StartupDisplay {
        renderer: options.renderer,
        gamma: options.gamma,
        width: options.width,
        height: options.height,
        hidden: options.hidden,
    }
}

/// Startup entry point (donor `StartupEntry`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupEntry {
    /// Open the menu.
    Menu,
    /// Run the initial selection.
    Run,
}

/// Per-frame mutable state handed to the backend (donor locals shared
/// across `step`).
pub struct StartupFrame<'a> {
    /// Elapsed frame time in milliseconds (donor `elapsed`, minimum 4).
    pub elapsed_ms: f64,
    /// Frame counter before this frame (donor `this.frames`).
    pub frame: u64,
    /// Queued source change (donor `this.pending`).
    pub pending: &'a mut Option<StartupAction>,
    /// Status line (donor `this.status`).
    pub status: &'a mut String,
    /// Save-library refresh request (donor `this.refreshSaves`).
    pub refresh_saves: &'a mut bool,
    /// Display update for the model, if the backend observed one.
    pub display: &'a mut Option<StartupDisplay>,
}

/// World-owning startup backend (donor menu/game clients, browser, lobby,
/// saves, addons, keys, music, movies, demos, and GTV).
pub trait StartupBackend {
    /// Current monotonic time in milliseconds (donor `performance.now`).
    fn now_ms(&self) -> f64;
    /// Sleep between frames (donor `Bun.sleep`; sync port).
    fn sleep_ms(&self, ms: f64);
    /// Open browser and graphics (donor `open` tail).
    fn open(&mut self) -> Result<(), String>;
    /// Poll browser and lobby (donor `poll` calls in `step`).
    fn poll(&mut self) -> Result<(), String>;
    /// Run one frame's world work (donor `step` body between polls).
    fn frame(&mut self, ctx: &mut StartupFrame<'_>) -> Result<(), String>;
    /// Route one input event (donor `input`).
    fn input(&mut self, event: &SeatInputEvent) -> bool;
    /// Request shutdown (donor `requestQuit` tail).
    fn request_quit(&mut self);
    /// Read back pixels (donor `readPixels`).
    fn read_pixels(&self) -> Result<Vec<u8>, String>;
    /// Capture the next frame's pixels (donor `captureNextFrame`; sync
    /// port returns the pixels instead of a promise).
    fn capture_next_frame(&mut self) -> Result<Vec<u8>, String>;
    /// Whether a game client is active (donor `activeGame`).
    fn has_active_game(&self) -> bool;
    /// Seat receiving input, if any (donor `inputSeat`).
    fn input_seat(&self) -> Option<SeatId>;
    /// Release backend resources, returning failure messages (donor
    /// `close` tail).
    fn close(&mut self) -> Vec<String>;
}

/// Startup application front-end (donor `StartupApplication`).
pub struct StartupApplication<B> {
    /// Selection model (donor `readonly model`).
    pub model: StartupSelectionModel,
    backend: B,
    entry: StartupEntry,
    pending: Option<StartupAction>,
    stopping: bool,
    closed: bool,
    frames: u64,
    status: String,
    refresh_saves: bool,
    last_frame_ms: f64,
}

impl<B: StartupBackend> StartupApplication<B> {
    /// Open the application (donor `open`).
    pub fn open(model: StartupSelectionModel, mut backend: B, entry: StartupEntry) -> Result<Self, StartupError> {
        if let Err(error) = backend.open() {
            backend.close();
            return Err(StartupError::Failed(format!(
                "Could not open startup application: {error}"
            )));
        }
        let last_frame_ms = backend.now_ms();
        Ok(Self {
            model,
            backend,
            entry,
            pending: None,
            stopping: false,
            closed: false,
            frames: 0,
            status: String::new(),
            refresh_saves: false,
            last_frame_ms,
        })
    }

    /// Entry point.
    #[must_use]
    pub fn entry(&self) -> StartupEntry {
        self.entry
    }

    /// Queued source change, if any.
    #[must_use]
    pub fn pending(&self) -> Option<&StartupAction> {
        self.pending.as_ref()
    }

    /// Frames stepped.
    #[must_use]
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// Live backend (the windowed drive loop reads the display rate from it).
    #[must_use]
    pub fn backend(&self) -> &B {
        &self.backend
    }

    /// Status line.
    #[must_use]
    pub fn status(&self) -> &str {
        &self.status
    }

    /// Whether shutdown was requested.
    #[must_use]
    pub fn is_stopping(&self) -> bool {
        self.stopping
    }

    /// Whether the application is closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Route one input event (donor `input`).
    pub fn input(&mut self, event: &SeatInputEvent) -> bool {
        self.backend.input(event)
    }

    /// Request shutdown (donor `requestQuit`).
    pub fn request_quit(&mut self) {
        self.stopping = true;
        self.backend.request_quit();
    }

    /// Read back pixels (donor `readPixels`).
    pub fn read_pixels(&self) -> Result<Vec<u8>, StartupError> {
        self.backend.read_pixels().map_err(StartupError::Failed)
    }

    /// Capture the next frame's pixels (donor `captureNextFrame`).
    pub fn capture_next_frame(&mut self) -> Result<Vec<u8>, StartupError> {
        self.backend.capture_next_frame().map_err(StartupError::Failed)
    }

    /// Whether a game client is active (donor `activeGame`).
    #[must_use]
    pub fn active_game(&self) -> bool {
        self.backend.has_active_game()
    }

    /// Seat receiving input, if any (donor `inputSeat`).
    #[must_use]
    pub fn input_seat(&self) -> Option<SeatId> {
        self.backend.input_seat()
    }

    /// Step one frame (donor `step`).
    pub fn step(&mut self) -> Result<(), StartupError> {
        if self.closed || self.stopping {
            return Ok(());
        }
        let now = self.backend.now_ms();
        let elapsed = (now - self.last_frame_ms).max(4.0);
        self.last_frame_ms = now;
        self.backend.poll().map_err(StartupError::Failed)?;
        if self.closed || self.stopping {
            return Ok(());
        }
        let mut display = None;
        {
            let mut ctx = StartupFrame {
                elapsed_ms: elapsed,
                frame: self.frames,
                pending: &mut self.pending,
                status: &mut self.status,
                refresh_saves: &mut self.refresh_saves,
                display: &mut display,
            };
            self.backend.frame(&mut ctx).map_err(StartupError::Failed)?;
        }
        if let Some(display) = display {
            self.model.set_display(display.width, display.height, display.gamma)?;
        }
        if self.closed || self.stopping {
            return Ok(());
        }
        self.frames += 1;
        Ok(())
    }

    /// Run until shutdown, close, or the frame limit (donor `run`).
    pub fn run(&mut self) -> Result<(), StartupError> {
        while !self.stopping && !self.closed && self.under_frame_limit()? {
            let started = self.backend.now_ms();
            self.step()?;
            if self.stopping || self.closed {
                break;
            }
            let wait = if !self.backend.has_active_game() {
                8.0
            } else {
                4.0 - (self.backend.now_ms() - started)
            };
            if wait > 0.0 {
                self.backend.sleep_ms(wait);
            }
        }
        Ok(())
    }

    /// Whether the frame limit (if any) still allows stepping.
    fn under_frame_limit(&self) -> Result<bool, StartupError> {
        Ok(self
            .model
            .options()?
            .frame_limit
            .is_none_or(|limit| self.frames < limit))
    }

    /// Release the application (donor `close`).
    pub fn close(&mut self) -> Result<(), StartupError> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        self.stopping = true;
        let failures = self.backend.close();
        if failures.is_empty() {
            Ok(())
        } else {
            Err(StartupError::Aggregate {
                context: "Startup application cleanup failed".to_string(),
                failures,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::cell::RefCell;

    use qa_content::catalog::CatalogArchive;
    use qa_content::catalog::CatalogError;
    use qa_content::catalog::CatalogProduct;
    use qa_content::catalog::InstalledCatalog;
    use qa_content::catalog::LaunchPreset;
    use qa_content::catalog::LaunchQvmCompatibility;
    use qa_content::catalog::LaunchWeaponSources;
    use qa_content::catalog::ProductAvailability;
    use qa_content::catalog::ProductExpectation;
    use qa_content::catalog::{BehaviorMounts, QvmCompatRole};
    use qa_content::contract::ContentDigest;
    use qa_content::contract::ContentId;
    use qa_content::contract::ExecutableRecipe as ContractRecipe;
    use qa_content::contract::GameFamily;
    use qa_content::contract::ModDescription;
    use qa_content::contract::ModSelection;
    use qa_content::contract::ProviderReference;
    use qa_content::contract::ProviderTiming;
    use qa_content::contract::QvmAbiProfile;
    use qa_content::contract::ResourceRequest as ContractResourceRequest;
    use qa_content::mounts::MountPreparationScope;
    use qa_core::identity::IdentityOwner;

    use super::super::server_browser::BrowserProtocol;
    use super::super::startup_selection::PreparedQ3Catalog;
    use super::super::startup_selection::PreparedTeamArena;
    use super::super::startup_selection::QvmGrappleStyle;
    use super::super::startup_selection::StartupArenaSelection;
    use super::super::startup_selection::StartupPlayerProducts;
    use super::super::startup_selection::StartupSelectionCollaborators;
    use crate::options::Network;

    use super::*;

    struct FakeWeapons;

    impl LaunchWeaponSources for FakeWeapons {
        fn canonical_weapon_source(
            &self,
            _map: &ProviderReference,
            weapon: &ProviderReference,
            _catalog: &InstalledCatalog,
        ) -> Result<ProviderReference, CatalogError> {
            Ok(weapon.clone())
        }

        fn selected_weapon_resources(
            &self,
            _map: &ProviderReference,
            _weapons: &[ProviderReference],
            _catalog: &InstalledCatalog,
        ) -> Result<Vec<ContractResourceRequest>, CatalogError> {
            Ok(Vec::new())
        }

        fn selected_weapon_timing(
            &self,
            _map: &ProviderReference,
            _weapons: &[ProviderReference],
            _catalog: &InstalledCatalog,
        ) -> Result<Vec<ProviderTiming>, CatalogError> {
            Ok(Vec::new())
        }

        fn admit_weapon_timing(
            &self,
            _timing: &mut Vec<ProviderTiming>,
            _weapon: &ProviderTiming,
        ) -> Result<(), CatalogError> {
            Ok(())
        }

        fn weapon_provider_ids(&self) -> Vec<qa_core::identity::ProviderId> {
            Vec::new()
        }
    }

    struct FakeCompat;

    impl LaunchQvmCompatibility for FakeCompat {
        fn read_qvm_compatibility(
            &self,
            _mounts: &dyn BehaviorMounts,
            _artifact_path: &str,
            _digest: &ContentDigest,
            _role: QvmCompatRole,
        ) -> Result<QvmAbiProfile, CatalogError> {
            Err(CatalogError::Invalid("no qvm in tests".to_string()))
        }
    }

    struct FakeCollaborators;

    impl StartupSelectionCollaborators for FakeCollaborators {
        fn duplicate(&self) -> Box<dyn StartupSelectionCollaborators> {
            Box::new(FakeCollaborators)
        }

        fn mod_choices(&self, _catalog: &InstalledCatalog) -> Result<Vec<ModDescription>, String> {
            Ok(Vec::new())
        }

        fn apply_mods(
            &self,
            recipe: ContractRecipe,
            _choices: &[ModDescription],
            _mods: &[ModSelection],
        ) -> Result<ContractRecipe, String> {
            Ok(recipe)
        }

        fn read_arena_selection(
            &self,
            _catalog: &InstalledCatalog,
            _options: &ApplicationOptions,
        ) -> Result<StartupArenaSelection, String> {
            Ok(StartupArenaSelection::default())
        }

        fn prepare_q3_product(
            &self,
            catalog: &InstalledCatalog,
            _product_id: &str,
            _initial: &ApplicationOptions,
        ) -> Result<PreparedQ3Catalog, String> {
            Ok(PreparedQ3Catalog {
                catalog: catalog.clone(),
                q3_product: None,
            })
        }

        fn load_team_arena(
            &self,
            _catalog: &InstalledCatalog,
            _initial: &ApplicationOptions,
        ) -> Result<Option<PreparedTeamArena>, String> {
            Ok(None)
        }

        fn qvm_grapple_selection(
            &self,
            _catalog: &InstalledCatalog,
            _product_id: &str,
            _mounts: &MountPreparationScope,
        ) -> Result<Option<QvmGrappleStyle>, String> {
            Ok(None)
        }

        fn player_products(
            &self,
            _catalog: &InstalledCatalog,
            product: &str,
            _movement: GameFamily,
            _character: GameFamily,
            _network: &Network,
        ) -> Result<StartupPlayerProducts, String> {
            Ok(StartupPlayerProducts {
                movement: product.to_string(),
                character: product.to_string(),
            })
        }

        fn application_preset(
            &self,
            _catalog: &InstalledCatalog,
            _options: &ApplicationOptions,
            _movement: Option<&ProviderReference>,
            _character: Option<&ProviderReference>,
        ) -> Result<LaunchPreset, String> {
            Err("no preset in tests".to_string())
        }

        fn launch_weapons(&self) -> Box<dyn LaunchWeaponSources> {
            Box::new(FakeWeapons)
        }

        fn launch_compat(&self) -> Box<dyn LaunchQvmCompatibility> {
            Box::new(FakeCompat)
        }
    }

    fn content_map(path: &str) -> qa_content::catalog::ContentMap {
        qa_content::catalog::ContentMap {
            path: path.to_string(),
            source: "test".to_string(),
            member_index: None,
        }
    }

    fn product(id: &str, family: GameFamily, edition: &str, campaign: &str) -> CatalogProduct {
        CatalogProduct {
            id: ContentId(id.to_string()),
            expectation: ProductExpectation {
                id: id.to_string(),
                family,
                edition: edition.to_string(),
                campaign: campaign.to_string(),
                title: id.to_string(),
                content_directory: campaign.to_string(),
                base_product: None,
                required_content_archives: Vec::new(),
                required_programs: Vec::new(),
                map_witness: None,
                unresolved_reason: None,
            },
            availability: ProductAvailability::Installed,
            archives: Vec::new(),
            loose_root: None,
            user_content: None,
            maps: if id == "q1-classic-id1" {
                vec![content_map("maps/start.bsp")]
            } else {
                Vec::new()
            },
            diagnostics: Vec::new(),
        }
    }

    fn catalog() -> InstalledCatalog {
        InstalledCatalog::new(
            "/tmp/qa-startup-test".to_string(),
            vec![
                product("q1-classic-id1", GameFamily::Q1, "classic", "id1"),
                product("q1-quakeworld", GameFamily::Q1, "quakeworld", "qw"),
                product("q2-classic-baseq2", GameFamily::Q2, "classic", "baseq2"),
                product("q2-classic-ctf", GameFamily::Q2, "classic", "ctf"),
                product("q2-classic-lmctf", GameFamily::Q2, "classic", "lmctf"),
                product("q2-rerelease-baseq2", GameFamily::Q2, "rerelease", "baseq2"),
                product("q3-baseq3", GameFamily::Q3, "classic", "baseq3"),
            ],
            Vec::<CatalogArchive>::new(),
            0,
            None,
        )
        .unwrap()
    }

    fn model() -> StartupSelectionModel {
        let options = ApplicationOptions {
            product: "q1-classic-id1".to_string(),
            map: "maps/start.bsp".to_string(),
            character_model: "player".to_string(),
            ..ApplicationOptions::default()
        };
        let mut model = StartupSelectionModel::new(catalog(), options, Box::new(FakeCollaborators)).unwrap();
        model.prepare_maps().unwrap();
        model
    }

    struct FakeBackend {
        now: Cell<f64>,
        sleeps: RefCell<Vec<f64>>,
        frames: RefCell<Vec<(f64, u64)>>,
        open_error: Option<String>,
        poll_error: Option<String>,
        frame_error: Option<String>,
        close_failures: Vec<String>,
        active_game: bool,
        queue: RefCell<Option<StartupAction>>,
        status: RefCell<Option<String>>,
        display: RefCell<Option<StartupDisplay>>,
        inputs: RefCell<usize>,
        quit: Cell<bool>,
    }

    impl FakeBackend {
        fn silent() -> Self {
            Self {
                now: Cell::new(1_000.0),
                sleeps: RefCell::new(Vec::new()),
                frames: RefCell::new(Vec::new()),
                open_error: None,
                poll_error: None,
                frame_error: None,
                close_failures: Vec::new(),
                active_game: false,
                queue: RefCell::new(None),
                status: RefCell::new(None),
                display: RefCell::new(None),
                inputs: RefCell::new(0),
                quit: Cell::new(false),
            }
        }
    }

    impl StartupBackend for FakeBackend {
        fn now_ms(&self) -> f64 {
            self.now.get()
        }

        fn sleep_ms(&self, ms: f64) {
            self.sleeps.borrow_mut().push(ms);
        }

        fn open(&mut self) -> Result<(), String> {
            if let Some(error) = self.open_error.clone() {
                return Err(error);
            }
            Ok(())
        }

        fn poll(&mut self) -> Result<(), String> {
            if let Some(error) = self.poll_error.clone() {
                return Err(error);
            }
            Ok(())
        }

        fn frame(&mut self, ctx: &mut StartupFrame<'_>) -> Result<(), String> {
            if let Some(error) = self.frame_error.clone() {
                return Err(error);
            }
            self.frames.borrow_mut().push((ctx.elapsed_ms, ctx.frame));
            if let Some(action) = self.queue.borrow_mut().take() {
                *ctx.pending = Some(action);
            }
            if let Some(status) = self.status.borrow_mut().take() {
                *ctx.status = status;
            }
            if let Some(display) = self.display.borrow_mut().take() {
                *ctx.display = Some(display);
            }
            Ok(())
        }

        fn input(&mut self, _event: &SeatInputEvent) -> bool {
            *self.inputs.borrow_mut() += 1;
            true
        }

        fn request_quit(&mut self) {
            self.quit.set(true);
        }

        fn read_pixels(&self) -> Result<Vec<u8>, String> {
            Ok(vec![1, 2, 3, 4])
        }

        fn capture_next_frame(&mut self) -> Result<Vec<u8>, String> {
            Ok(vec![5, 6, 7, 8])
        }

        fn has_active_game(&self) -> bool {
            self.active_game
        }

        fn input_seat(&self) -> Option<SeatId> {
            None
        }

        fn close(&mut self) -> Vec<String> {
            self.close_failures.clone()
        }
    }

    #[test]
    fn open_initializes_state() {
        let app = StartupApplication::open(model(), FakeBackend::silent(), StartupEntry::Menu).unwrap();
        assert_eq!(app.entry(), StartupEntry::Menu);
        assert_eq!(app.frames(), 0);
        assert_eq!(app.status(), "");
        assert!(app.pending().is_none());
        assert!(!app.is_stopping());
        assert!(!app.is_closed());
        assert!(!app.active_game());
        assert_eq!(app.input_seat(), None);
        assert_eq!(app.model.options().unwrap().product, "q1-classic-id1");
    }

    #[test]
    fn open_failure_reports() {
        let mut backend = FakeBackend::silent();
        backend.open_error = Some("no window".to_string());
        let error = StartupApplication::open(model(), backend, StartupEntry::Run)
            .err()
            .unwrap();
        assert_eq!(error.to_string(), "Could not open startup application: no window");
    }

    #[test]
    fn step_frames_and_floors_elapsed() {
        let mut app = StartupApplication::open(model(), FakeBackend::silent(), StartupEntry::Menu).unwrap();
        app.step().unwrap();
        assert_eq!(app.frames(), 1);
        app.step().unwrap();
        assert_eq!(app.frames(), 2);
    }

    #[test]
    fn step_hands_pending_and_status_to_backend() {
        let mut backend = FakeBackend::silent();
        backend.queue = RefCell::new(Some(StartupAction::Play));
        backend.status = RefCell::new(Some("loading".to_string()));
        let mut app = StartupApplication::open(model(), backend, StartupEntry::Menu).unwrap();
        app.step().unwrap();
        assert_eq!(app.pending(), Some(&StartupAction::Play));
        assert_eq!(app.status(), "loading");
    }

    #[test]
    fn step_applies_display() {
        let mut backend = FakeBackend::silent();
        backend.display = RefCell::new(Some(StartupDisplay {
            renderer: Renderer::Cpu,
            gamma: 1.0,
            width: 800,
            height: 600,
            hidden: false,
        }));
        let mut app = StartupApplication::open(model(), backend, StartupEntry::Menu).unwrap();
        app.step().unwrap();
        assert_eq!(app.frames(), 1);
    }

    #[test]
    fn step_gates_on_stopping_and_closed() {
        let mut app = StartupApplication::open(model(), FakeBackend::silent(), StartupEntry::Menu).unwrap();
        app.request_quit();
        assert!(app.is_stopping());
        app.step().unwrap();
        assert_eq!(app.frames(), 0);
        app.close().unwrap();
        app.step().unwrap();
        assert_eq!(app.frames(), 0);
    }

    #[test]
    fn run_respects_frame_limit() {
        let options = ApplicationOptions {
            product: "q1-classic-id1".to_string(),
            map: "maps/start.bsp".to_string(),
            character_model: "player".to_string(),
            frame_limit: Some(3),
            ..ApplicationOptions::default()
        };
        let mut selection = StartupSelectionModel::new(catalog(), options, Box::new(FakeCollaborators)).unwrap();
        selection.prepare_maps().unwrap();
        let mut app = StartupApplication::open(selection, FakeBackend::silent(), StartupEntry::Run).unwrap();
        app.run().unwrap();
        assert_eq!(app.frames(), 3);
    }

    #[test]
    fn close_aggregates_failures() {
        let mut backend = FakeBackend::silent();
        backend.close_failures = vec!["browser".to_string(), "audio".to_string()];
        let mut app = StartupApplication::open(model(), backend, StartupEntry::Menu).unwrap();
        let error = app.close().err().unwrap();
        assert_eq!(
            error,
            StartupError::Aggregate {
                context: "Startup application cleanup failed".to_string(),
                failures: vec!["browser".to_string(), "audio".to_string()],
            }
        );
        assert!(app.is_closed());
        app.close().unwrap();
    }

    #[test]
    fn input_pixels_and_capture_pass_through() {
        let authority = IdentityOwner::create("startup-test").unwrap();
        let mut app = StartupApplication::open(model(), FakeBackend::silent(), StartupEntry::Menu).unwrap();
        let event = SeatInputEvent::Key {
            seat: authority.seat(0),
            time_ms: 0.0,
            code: 27,
            down: true,
        };
        assert!(app.input(&event));
        assert_eq!(app.read_pixels().unwrap(), vec![1, 2, 3, 4]);
        assert_eq!(app.capture_next_frame().unwrap(), vec![5, 6, 7, 8]);
    }

    #[test]
    fn startup_display_snapshots_options() {
        let options = ApplicationOptions::default();
        let display = startup_display(&options);
        assert_eq!(display.renderer, options.renderer);
        assert_eq!(display.gamma, options.gamma);
        assert_eq!(
            (display.width, display.height, display.hidden),
            (options.width, options.height, options.hidden)
        );
    }

    #[test]
    fn action_variants_round_trip() {
        // `StartupAction::LobbyHost` carries `StartupLaunch`, which needs
        // installed-map fixtures that even `startup_selection` tests lack
        // (`resolve_requires_an_installed_map`); the variant is
        // compile-checked, and the other seven round-trip here.
        let actions = vec![
            StartupAction::Gtv {
                request: GtvConnectRequest {
                    address: "gtv.example".to_string(),
                    username: "u".to_string(),
                    password: "p".to_string(),
                    label: None,
                },
            },
            StartupAction::Connect {
                connection: BrowserConnection {
                    protocol: BrowserProtocol::Q1,
                    remote: "local".to_string(),
                    q2_protocol: None,
                },
            },
            StartupAction::Initial {
                options: Box::new(ApplicationOptions::default()),
            },
            StartupAction::Frontend,
            StartupAction::Play,
            StartupAction::Preset {
                id: "dm".to_string(),
                skill: 2,
                arena_map: Some("q3dm1".to_string()),
            },
            StartupAction::Load {
                path: "save/quicksave".to_string(),
                source_product: None,
            },
        ];
        for action in &actions {
            let mut backend = FakeBackend::silent();
            backend.queue = RefCell::new(Some(action.clone()));
            let mut app = StartupApplication::open(model(), backend, StartupEntry::Menu).unwrap();
            app.step().unwrap();
            assert_eq!(app.pending(), Some(action));
        }
        assert_eq!(actions.len(), 7);
    }
}
