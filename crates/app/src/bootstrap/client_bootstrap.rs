//! Client bootstrap contracts: seats, input publication, recording, and the client object.
//!
//! Port of donor `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/client-bootstrap.ts`
//! (`ClientSourcePublicationError`, `ClientBootstrapSeat`,
//! `ClientInputPublication`, `ClientRecordingFeed`, `ClientSourceLifetime`,
//! `ClientBootstrap`, `localClientAccount`). The port is synchronous; every
//! collaborator the bootstrap lane does not own arrives as an associated
//! type. Local account ids use a time-seeded generator instead of node
//! crypto, and the identity cache is keyed by session.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use qa_client::render::types::RenderCommand;
use qa_client::text::captions::ActiveCaption;
use qa_client::text::draw2d::Rect;
use qa_content::contract::ProviderReference;
use qa_content::hash::hex_lower;
use qa_core::cmd_buffer::CommandContext;
use qa_core::identity::IdentityOwner;
use qa_net::services::online::Account;

use super::config_scripts::ConsoleScriptFiles;
use super::demo_recording::DemoRecordingSeed;
use crate::options::ApplicationOptions;
use crate::settings::config::ConfigStore;

/// Publication failures after publication began.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientSourcePublicationError {
    /// Publication failures.
    pub errors: Vec<String>,
}

impl std::fmt::Display for ClientSourcePublicationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Client source failed after publication began: {}",
            self.errors.join("; ")
        )
    }
}

impl std::error::Error for ClientSourcePublicationError {}

/// One bootstrapped local seat (`ClientBootstrapSeat`).
#[derive(Debug, Clone)]
pub struct ClientBootstrapSeat<Client, Seat, Prepared> {
    /// Session client.
    pub client: Client,
    /// Session seat.
    pub seat: Seat,
    /// Prepared seat.
    pub prepared: Prepared,
}

/// Published client input (`ClientInputPublication`).
pub enum ClientInputPublication<Router, Settings, Input> {
    /// Menu input with a command retirer.
    Menu {
        /// Input router.
        router: Router,
        /// Controller settings.
        controller_settings: Settings,
        /// Retire menu commands.
        retire_commands: Box<dyn FnOnce()>,
    },
    /// World input.
    World {
        /// Application input.
        input: Input,
    },
}

/// Shared input publication cell.
pub type SharedInputPublication<Router, Settings, Input> =
    Rc<RefCell<Option<ClientInputPublication<Router, Settings, Input>>>>;

/// Recording feed (`ClientRecordingFeed`).
pub trait ClientRecordingFeed {
    /// Feed root.
    fn root(&self) -> &str;
    /// Current recording seed.
    fn seed(&mut self) -> DemoRecordingSeed;
    /// Attach a sink, returning a detach closure.
    fn attach(&mut self, sink: &mut dyn super::demo_recording::DemoRecordingSink) -> Box<dyn FnOnce()>;
}

/// Client source lifetime (`ClientSourceLifetime`, sync).
pub trait ClientSourceLifetime {
    /// Lifetime failure.
    type Error;
    /// Recording feed.
    type Feed: ClientRecordingFeed;
    /// Prepared video presentation.
    type VideoPresentation;
    /// Prepare recording for a command source.
    fn prepare_recording(&mut self, source: &CommandContext) -> Result<Self::Feed, Self::Error>;
    /// Prepare a video restart, if one is pending.
    fn prepare_video_restart(&mut self) -> Result<Option<Self::VideoPresentation>, Self::Error>;
    /// Capture map, if a capture is armed.
    fn capture_map(&self) -> Option<&str>;
    /// Prepare retirement.
    fn prepare_retirement(&mut self) -> Result<(), Self::Error>;
    /// Release settings.
    fn release_settings(&mut self);
    /// Retire the source.
    fn retire(&mut self) -> Result<(), Self::Error>;
}

/// Current configuration scripts and options.
pub struct ClientConfiguration<M> {
    /// Console script files.
    pub scripts: ConsoleScriptFiles<M>,
    /// Application options.
    pub options: ApplicationOptions,
}

/// Frontend activation (`activateFrontend` configuration).
pub struct FrontendActivation {
    /// Append to the released command buffer.
    pub release_commands: Box<dyn FnMut(&str)>,
    /// Publish the frontend.
    pub publish: Box<dyn FnOnce()>,
}

/// Borrowed client objects without final close authority (`ClientBootstrap`).
pub trait ClientBootstrap {
    /// Request failure.
    type Error;
    /// Local lobby.
    type LocalLobby;
    /// Application keys.
    type Keys;
    /// Client demo recording.
    type Recording;
    /// Video restart control.
    type VideoRestart;
    /// Music controls.
    type MusicControls;
    /// Console capture owner.
    type Capture;
    /// Session seat.
    type Seat: std::hash::Hash + Eq + Clone;
    /// Seat console.
    type Console;
    /// Engine session.
    type Session;
    /// Session client.
    type Client;
    /// Prepared seat.
    type PreparedSeat;
    /// Prepared startup.
    type PreparedStartup;
    /// Native renderer.
    type Renderer;
    /// Image settings.
    type ImageSettings;
    /// SDL controllers.
    type Controllers;
    /// Input devices.
    type InputDevices;
    /// Unified audio output.
    type Audio;
    /// Input router.
    type Router;
    /// Controller settings.
    type Settings;
    /// Application input.
    type Input;
    /// Client source lifetime.
    type Source: ClientSourceLifetime;
    /// Configuration script mounts.
    type Mounts;

    /// Local lobby, if hosted.
    fn local_lobby(&self) -> Option<&Self::LocalLobby>;
    /// Application keys.
    fn keys(&self) -> &Self::Keys;
    /// Caption draw commands.
    fn caption_commands(
        &self,
        captions: &[ActiveCaption],
        viewport: &Rect,
        time_milliseconds: f64,
    ) -> Vec<RenderCommand>;
    /// Client demo recording.
    fn recording(&self) -> &Self::Recording;
    /// Stop recording a source.
    fn stop_recording(&mut self, source: &mut Self::Source) -> Result<(), ClientSourcePublicationError>;
    /// Video restart control.
    fn video_restart(&self) -> &Self::VideoRestart;
    /// Music controls.
    fn music_controls(&self) -> &Self::MusicControls;
    /// Console capture owner.
    fn capture(&self) -> &Self::Capture;
    /// Consoles by seat.
    fn consoles(&self) -> &HashMap<Self::Seat, Self::Console>;
    /// Identity owner.
    fn identity(&self) -> &IdentityOwner;
    /// Engine session.
    fn session(&self) -> &Self::Session;
    /// Local seats.
    fn locals(&self) -> &[ClientBootstrapSeat<Self::Client, Self::Seat, Self::PreparedSeat>];
    /// Prepared startup.
    fn prepared(&self) -> &Self::PreparedStartup;
    /// Native renderer.
    fn renderer(&self) -> &Self::Renderer;
    /// Image settings.
    fn image_settings(&self) -> &Self::ImageSettings;
    /// SDL controllers.
    fn controllers(&self) -> &Self::Controllers;
    /// Input devices.
    fn input_devices(&self) -> &Self::InputDevices;
    /// Settings store.
    fn settings(&self) -> &ConfigStore;
    /// Current audio output.
    fn output(&self) -> Rc<RefCell<Option<Self::Audio>>>;
    /// Current input publication.
    fn platform(&self) -> SharedInputPublication<Self::Router, Self::Settings, Self::Input>;
    /// Current source lifetime.
    fn source(&self) -> Rc<RefCell<Option<Self::Source>>>;
    /// Current source profile.
    fn source_profile(&self) -> Rc<RefCell<Option<ProviderReference>>>;
    /// Current configuration.
    fn configuration(&self) -> Rc<RefCell<ClientConfiguration<Self::Mounts>>>;
    /// Activate the frontend.
    fn activate_frontend(&mut self, configuration: Option<FrontendActivation>);
    /// Route a console command.
    fn route_command(&mut self, name: &str, args: &[String], source: &CommandContext) -> bool;
    /// Dispatch an application request.
    fn dispatch_application_request(&mut self, request: ClientApplicationRequest) -> Result<(), Self::Error>;
    /// Whether a source is pending.
    fn has_pending_source(&self) -> bool;
}

/// Application request (`ConfigurationCommandRequest`, opaque to this lane).
#[derive(Debug, Clone)]
pub struct ClientApplicationRequest {
    /// Request payload.
    pub payload: Vec<String>,
}

static LOCAL_ACCOUNTS: LazyLock<Mutex<HashMap<qa_core::identity::SessionId, Account>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static ACCOUNT_COUNTER: AtomicU64 = AtomicU64::new(0);

fn random_account_bytes() -> [u8; 16] {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|time| time.as_nanos() as u64)
        .unwrap_or(0x1234_5678_9abc_def0);
    let count = ACCOUNT_COUNTER.fetch_add(1, Ordering::Relaxed);
    let seed = nanos
        ^ (std::process::id() as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
        ^ count.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    let mut state = seed | 1;
    let mut bytes = [0u8; 16];
    for chunk in bytes.chunks_mut(8) {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        chunk.copy_from_slice(&state.to_le_bytes());
    }
    bytes
}

/// Local account for an identity, cached by session (`localClientAccount`).
pub fn local_client_account(identity: &IdentityOwner) -> Account {
    let mut accounts = LOCAL_ACCOUNTS.lock().expect("account cache");
    if let Some(previous) = accounts.get(identity.session()) {
        return previous.clone();
    }
    let account = Account {
        id: format!("account:{}", hex_lower(&random_account_bytes())),
        name: identity.session().name().to_string(),
    };
    accounts.insert(identity.session().clone(), account.clone());
    account
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn publication_error_lists_failures() {
        let error = ClientSourcePublicationError {
            errors: vec!["first".to_string(), "second".to_string()],
        };
        assert_eq!(
            error.to_string(),
            "Client source failed after publication began: first; second"
        );
    }

    #[test]
    fn local_account_is_stable_per_identity() {
        let first = IdentityOwner::create("alpha").unwrap();
        let second = IdentityOwner::create("alpha").unwrap();
        let one = local_client_account(&first);
        let again = local_client_account(&first);
        assert_eq!(one.id, again.id);
        assert_eq!(one.name, "alpha");
        assert!(one.id.starts_with("account:"));
        assert_eq!(one.id.len(), "account:".len() + 32);
        let other = local_client_account(&second);
        assert_ne!(one.id, other.id);
    }

    #[test]
    fn input_publication_carries_retirer() {
        let retired = Rc::new(Cell::new(false));
        let retired_callback = Rc::clone(&retired);
        let publication: ClientInputPublication<String, String, String> = ClientInputPublication::Menu {
            router: "router".to_string(),
            controller_settings: "settings".to_string(),
            retire_commands: Box::new(move || retired_callback.set(true)),
        };
        match publication {
            ClientInputPublication::Menu { retire_commands, .. } => retire_commands(),
            ClientInputPublication::World { .. } => panic!("expected menu"),
        }
        assert!(retired.get());
        let world: ClientInputPublication<String, String, String> = ClientInputPublication::World {
            input: "input".to_string(),
        };
        assert!(matches!(world, ClientInputPublication::World { .. }));
    }

    #[test]
    fn request_payload_round_trips() {
        let request = ClientApplicationRequest {
            payload: vec!["vid_restart".to_string()],
        };
        assert_eq!(request.payload.len(), 1);
    }
}
