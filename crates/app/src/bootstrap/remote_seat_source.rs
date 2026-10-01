//! Live remote source channel: presentation, network, and content transfers.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/remote-seat-source.ts`
//! (`RemoteSeatPresentation`, `RemoteSeatNetwork`, `RemoteSeatSourceHooks`,
//! `RemoteSeatSourceOptions`, `RemoteSeatSource`). One live source channel
//! and its content transfers; frontend ownership stays with the caller. The
//! five per-family network/presentation pairs plus download receivers,
//! content selection, and transports are unported siblings, so they arrive
//! as the [`RemoteSeatNetwork`]/[`RemoteSeatPresentation`] traits, the
//! [`RemoteSeatSourceHooks`] seam, and small download-manager traits. Sync
//! port: the donor's async content transfers become sync hook calls with
//! the same generation guards and error texts.

use qa_core::cmd::Dialect;
use qa_core::cmd_buffer::CommandOrigin;
use qa_core::cvar::CvarRegistry;
use qa_core::identity::ClientId;
use qa_core::identity::SeatId;
use qa_core::identity::SessionId;
use thiserror::Error;

/// Remote seat source failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RemoteSeatSourceError {
    /// QuakeWorld over IPX.
    #[error("QuakeWorld requires UDP")]
    QuakeWorldRequiresUdp,
    /// Native Q3 remote over neither IPv4 nor IPX.
    #[error("Native Q3 remote requires IPv4")]
    Q3RequiresIpv4,
    /// Q2 remote without a download policy.
    #[error("Q2 remote client has no download policy")]
    Q2RequiresDownloadPolicy,
    /// Component command without a unified channel.
    #[error("Component has no unified channel")]
    ComponentRequiresUnified,
    /// QW download before server data.
    #[error("QW download before serverdata")]
    QwDownloadBeforeServerData,
    /// Q3 download without a content owner.
    #[error("Q3 download has no content owner")]
    Q3DownloadWithoutOwner,
    /// QW download after a newer selection.
    #[error("QW directory selection was cancelled")]
    QwSelectionCancelled,
    /// Q3 download on a retired connection.
    #[error("Q3 package download belongs to a retired connection")]
    Q3RetiredConnection,
    /// Q3 content selection replaced mid-transfer.
    #[error("Q3 package content selection was replaced")]
    Q3SelectionReplaced,
    /// Q3 download replaced mid-transfer.
    #[error("Q3 package download was replaced")]
    Q3DownloadReplaced,
    /// Q3 package refresh cancelled.
    #[error("Q3 package refresh was cancelled")]
    Q3RefreshCancelled,
    /// Q3 download setup cancelled.
    #[error("Q3 download setup was cancelled")]
    Q3SetupCancelled,
    /// Q3 world load without selected content.
    #[error("Q3 world loading requires its selected content owner")]
    Q3RequiresContent,
    /// Q3 world load cancelled.
    #[error("Remote Q3 content loading was cancelled")]
    Q3LoadCancelled,
    /// Operation on a retired source.
    #[error("Remote seat source is retired")]
    Retired,
    /// Hook failure.
    #[error("{0}")]
    Hook(String),
    /// Network failure.
    #[error("{0}")]
    Network(String),
}

/// Remote source family (donor `RemoteSeatSourceOptions["family"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RemoteSeatFamily {
    /// Unified native channel.
    Unified,
    /// NetQuake.
    Q1,
    /// QuakeWorld.
    Qw,
    /// Quake II.
    Q2,
    /// Quake III.
    Q3,
}

/// Remote address kind (absorbed `ApplicationNetworkAddress["kind"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RemoteAddressKind {
    /// IPv4.
    Ipv4,
    /// IPv6.
    Ipv6,
    /// IPX.
    Ipx,
    /// UDP socket address.
    Udp,
}

/// Absorbed admitted-actor identity (donor `ActorId`, unmintable here).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RemoteActorId {
    /// Session token.
    pub session: u64,
    /// Registry slot.
    pub slot: u32,
    /// Slot generation.
    pub generation: u32,
}

/// Player admitted to a remote presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemotePlayer {
    /// Admitted actor.
    pub actor: RemoteActorId,
}

/// One seat-audience simulation event (absorbed donor event shape).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteSeatEvent {
    /// Opaque event payload marker retained for the seat.
    pub name: String,
    /// Retargeted seat audience.
    pub seat: SeatId,
}

/// Remote snapshot header (absorbed donor snapshot identity).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteSnapshot {
    /// Owning session.
    pub session: SessionId,
    /// Frame number.
    pub frame: u64,
}

/// Simulation output delivered to a remote seat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteSimulationOutput {
    /// Snapshot header.
    pub snapshot: RemoteSnapshot,
    /// Seat-audience events.
    pub events: Vec<RemoteSeatEvent>,
}

/// Raw inbound event before seat filtering (donor `SimulationOutput["events"]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteInboundEvent {
    /// Event name.
    pub name: String,
    /// Target seat, when the event targets one seat.
    pub seat: Option<SeatId>,
    /// Target client, when the event targets one client.
    pub client: Option<ClientId>,
    /// Broadcast event.
    pub broadcast: bool,
}

impl RemoteInboundEvent {
    /// Whether the event targets a seat (donor `eventTargetsSeat`).
    #[must_use]
    pub fn targets_seat(&self, seat: &SeatId, client: &ClientId) -> bool {
        if self.broadcast {
            return true;
        }
        match (&self.seat, &self.client) {
            (Some(event_seat), Some(event_client)) => event_seat == seat && event_client == client,
            (Some(event_seat), None) => event_seat == seat,
            (None, Some(event_client)) => event_client == client,
            (None, None) => false,
        }
    }

    /// Retarget to a seat audience (donor `audience: { kind: "seat" }`).
    #[must_use]
    pub fn for_seat(&self, seat: &SeatId) -> RemoteSeatEvent {
        RemoteSeatEvent {
            name: self.name.clone(),
            seat: seat.clone(),
        }
    }
}

/// Inbound simulation output before seat filtering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteInboundOutput {
    /// Snapshot header.
    pub snapshot: RemoteSnapshot,
    /// Unfiltered events.
    pub events: Vec<RemoteInboundEvent>,
}

/// Actor command submitted through a remote channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteActorCommand {
    /// Command source origin.
    pub source: CommandOrigin,
    /// Command actor.
    pub actor: RemoteActorId,
}

/// Presentation scene handle (absorbed per-family scene queries).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RemoteSceneHandle {
    /// Scene generation marker.
    pub generation: u64,
}

/// Remote presentation (donor `RemoteSeatPresentation` union).
pub trait RemoteSeatPresentation {
    /// Owning session client id.
    fn client_id(&self) -> ClientId;
    /// Admitted player, if any.
    fn player(&self) -> Option<RemotePlayer>;
    /// Whether an actor is the seat player.
    fn is_player(&self, actor: &RemoteActorId) -> bool;
    /// Presentation scene handle.
    fn scene(&self) -> RemoteSceneHandle;
    /// Latest output frame number, when presented.
    fn output_frame(&self) -> Option<u64>;
    /// Presentation family.
    fn family(&self) -> RemoteSeatFamily;
}

/// Remote client network (donor `RemoteSeatNetwork` union).
pub trait RemoteSeatNetwork {
    /// Network phase (`"active"`, `"loading"`, ...).
    fn phase(&self) -> String;
    /// Whether the channel submits timed commands (unified family).
    fn timed_submission(&self) -> bool {
        false
    }
    /// Poll the connection at a wall timestamp.
    fn poll(&mut self, wall_now_ms: f64);
    /// Submit commands; timed channels also consume the frame elapsed time.
    fn submit(&mut self, commands: &[RemoteActorCommand], wall_now_ms: f64, frame_elapsed_ms: f64);
    /// Send a console command.
    fn command(&mut self, text: &str);
    /// Close the connection.
    fn close(&mut self) -> Result<(), String> {
        Ok(())
    }
}

/// Frame-time source (absorbed `frame-time.ts`, outside this wave).
pub trait RemoteFrameTimer {
    /// Source frame milliseconds for a wall step.
    fn source_frame_milliseconds(&self, dialect: Dialect, wall_elapsed_ms: f64, cvars: &CvarRegistry) -> f64;
}

/// Download progress row (donor `ClientDownloadProgress`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteDownloadProgress {
    /// File name.
    pub name: String,
    /// Bytes received.
    pub received: u64,
    /// Total bytes.
    pub total: u64,
}

/// QuakeWorld download receiver (absorbed `QwDownloadReceiver`).
pub trait QwDownloadManager {
    /// Current progress rows.
    fn progress(&self) -> Vec<RemoteDownloadProgress>;
    /// Cancel pending downloads.
    fn cancel(&mut self);
    /// Retry pending downloads.
    fn retry(&mut self);
    /// Close the receiver.
    fn close(&mut self);
}

/// Q3 download receiver (absorbed `Q3ApplicationClientDownloads`).
pub trait Q3DownloadManager {
    /// Current progress rows.
    fn progress(&self) -> Vec<RemoteDownloadProgress>;
    /// Cancel pending downloads.
    fn cancel(&mut self);
    /// Retry pending downloads.
    fn retry(&mut self);
    /// Close the receiver.
    fn close(&mut self);
}

/// Q3 content owner (absorbed `Q3ClientContent`).
pub trait Q3ContentOwner {
    /// Pure command for a server id, if referenced.
    fn referenced_pure_command(&self, server_id: u32) -> Option<String>;
    /// Close the content owner.
    fn close(&mut self);
}

/// Content hooks behind a remote source (donor `RemoteSeatSourceHooks`).
pub trait RemoteSeatSourceHooks {
    /// Publish one simulation output.
    fn publish(&mut self, output: &RemoteSimulationOutput);
    /// Report a disconnect.
    fn disconnected(&mut self, reason: &str);
    /// Print diagnostics.
    fn print(&mut self, text: &str);
    /// Whether a demo is recording.
    fn recording(&self) -> bool;
}

/// Remote source construction (donor `RemoteSeatSourceOptions`).
pub struct RemoteSeatSourceOptions<Network, Presentation> {
    /// Source family.
    pub family: RemoteSeatFamily,
    /// Remote address kind.
    pub address_kind: RemoteAddressKind,
    /// Client qport.
    pub qport: u32,
    /// Whether a Q2 download policy is installed.
    pub download_policy: bool,
    /// Presentation owner.
    pub remote: Presentation,
    /// Network owner.
    pub network: Network,
}

/// One live source channel and its content transfers.
pub struct RemoteSeatSource<Network, Presentation> {
    /// Presentation owner.
    pub remote: Presentation,
    /// Network owner.
    pub network: Network,
    family: RemoteSeatFamily,
    qport: u32,
    qw_downloads: Option<Box<dyn QwDownloadManager>>,
    q3_downloads: Option<Box<dyn Q3DownloadManager>>,
    q3_content: Option<Box<dyn Q3ContentOwner>>,
    download_generation: u64,
    closed: bool,
    all_skins: String,
}

impl<Network: RemoteSeatNetwork, Presentation: RemoteSeatPresentation> RemoteSeatSource<Network, Presentation> {
    /// Open a source channel, validating the family transport.
    pub fn open(options: RemoteSeatSourceOptions<Network, Presentation>) -> Result<Self, RemoteSeatSourceError> {
        match options.family {
            RemoteSeatFamily::Qw if options.address_kind == RemoteAddressKind::Ipx => {
                return Err(RemoteSeatSourceError::QuakeWorldRequiresUdp);
            }
            RemoteSeatFamily::Q3
                if !matches!(options.address_kind, RemoteAddressKind::Ipv4 | RemoteAddressKind::Ipx) =>
            {
                return Err(RemoteSeatSourceError::Q3RequiresIpv4);
            }
            RemoteSeatFamily::Q2 if !options.download_policy => {
                return Err(RemoteSeatSourceError::Q2RequiresDownloadPolicy);
            }
            _ => {}
        }
        Ok(Self {
            remote: options.remote,
            network: options.network,
            family: options.family,
            qport: options.qport,
            qw_downloads: None,
            q3_downloads: None,
            q3_content: None,
            download_generation: 0,
            closed: false,
            all_skins: String::new(),
        })
    }

    /// Source family.
    #[must_use]
    pub fn family(&self) -> RemoteSeatFamily {
        self.family
    }

    /// Client qport.
    #[must_use]
    pub fn qport(&self) -> u32 {
        self.qport
    }

    /// Whether the source is closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Remember the shared all-skins value (donor `setAllSkins`).
    pub fn set_all_skins(&mut self, value: &str) {
        self.all_skins = value.to_string();
    }

    /// Shared all-skins value.
    #[must_use]
    pub fn all_skins(&self) -> &str {
        &self.all_skins
    }

    /// Pure command for a server id, when Q3 content is loaded.
    #[must_use]
    pub fn pure_command(&self, server_id: u32) -> Option<String> {
        self.q3_content
            .as_ref()
            .and_then(|content| content.referenced_pure_command(server_id))
    }

    /// Current download progress rows.
    pub fn download_progress(&self) -> Vec<RemoteDownloadProgress> {
        if self.family == RemoteSeatFamily::Q2 {
            return Vec::new();
        }
        if let Some(qw) = &self.qw_downloads {
            return qw.progress();
        }
        if let Some(q3) = &self.q3_downloads {
            return q3.progress();
        }
        Vec::new()
    }

    /// Cancel pending downloads.
    pub fn cancel_downloads(&mut self) {
        if let Some(qw) = self.qw_downloads.as_mut() {
            qw.cancel();
        }
        if let Some(q3) = self.q3_downloads.as_mut() {
            q3.cancel();
        }
    }

    /// Retry pending downloads.
    pub fn retry_downloads(&mut self) {
        if let Some(qw) = self.qw_downloads.as_mut() {
            qw.retry();
        }
        if let Some(q3) = self.q3_downloads.as_mut() {
            q3.retry();
        }
    }

    /// Attach a QuakeWorld download receiver (donor `prepareQwDownloads` sink).
    pub fn attach_qw_downloads(&mut self, downloads: Box<dyn QwDownloadManager>) {
        self.close_qw_downloads();
        self.qw_downloads = Some(downloads);
    }

    /// Require the QuakeWorld download receiver.
    pub fn require_qw_downloads(&mut self) -> Result<&mut dyn QwDownloadManager, RemoteSeatSourceError> {
        if self.qw_downloads.is_none() {
            return Err(RemoteSeatSourceError::QwDownloadBeforeServerData);
        }
        Ok(self
            .qw_downloads
            .as_mut()
            .map(|downloads| downloads.as_mut())
            .expect("checked above") as &mut dyn QwDownloadManager)
    }

    /// Attach a Q3 download receiver (donor `prepareQ3Downloads` sink).
    pub fn attach_q3_downloads(&mut self, downloads: Box<dyn Q3DownloadManager>) {
        self.close_q3_downloads();
        self.q3_downloads = Some(downloads);
    }

    /// Require the Q3 download receiver.
    pub fn require_q3_downloads(&mut self) -> Result<&mut dyn Q3DownloadManager, RemoteSeatSourceError> {
        if self.q3_downloads.is_none() {
            return Err(RemoteSeatSourceError::Q3DownloadWithoutOwner);
        }
        Ok(self
            .q3_downloads
            .as_mut()
            .map(|downloads| downloads.as_mut())
            .expect("checked above") as &mut dyn Q3DownloadManager)
    }

    /// Attach a Q3 content owner, closing the previous one.
    pub fn attach_q3_content(&mut self, content: Box<dyn Q3ContentOwner>) {
        if let Some(mut previous) = self.q3_content.take() {
            previous.close();
        }
        self.q3_content = Some(content);
    }

    /// Close the QuakeWorld receiver and invalidate its generation.
    pub fn close_qw_downloads(&mut self) {
        self.download_generation += 1;
        if let Some(mut downloads) = self.qw_downloads.take() {
            downloads.close();
        }
    }

    /// Close the Q3 receiver.
    pub fn close_q3_downloads(&mut self) {
        if let Some(mut downloads) = self.q3_downloads.take() {
            downloads.close();
        }
    }

    /// Current download generation (donor selection guard).
    #[must_use]
    pub fn download_generation(&self) -> u64 {
        self.download_generation
    }

    /// Fail when the source is retired (donor `assertOpen`).
    pub fn assert_open(&self) -> Result<(), RemoteSeatSourceError> {
        if self.closed {
            return Err(RemoteSeatSourceError::Retired);
        }
        Ok(())
    }

    /// Send a console command through the network.
    pub fn send_command(&mut self, text: &str) -> Result<(), RemoteSeatSourceError> {
        self.assert_open()?;
        self.network.command(text);
        Ok(())
    }

    /// Close the source, receivers, and content exactly once.
    pub fn close(&mut self) -> Result<(), RemoteSeatSourceError> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        self.close_qw_downloads();
        self.close_q3_downloads();
        let network = self.network.close();
        if let Some(mut content) = self.q3_content.take() {
            content.close();
        }
        network.map_err(RemoteSeatSourceError::Network)
    }
}

#[cfg(test)]
mod tests {
    use qa_core::identity::IdentityOwner;

    use super::*;

    struct FakeNetwork {
        phase: String,
        timed: bool,
        submitted: Vec<(Vec<RemoteActorCommand>, f64, f64)>,
        commands: Vec<String>,
        closed: bool,
    }

    impl RemoteSeatNetwork for FakeNetwork {
        fn phase(&self) -> String {
            self.phase.clone()
        }

        fn timed_submission(&self) -> bool {
            self.timed
        }

        fn poll(&mut self, _wall_now_ms: f64) {}

        fn submit(&mut self, commands: &[RemoteActorCommand], wall_now_ms: f64, frame_elapsed_ms: f64) {
            self.submitted.push((commands.to_vec(), wall_now_ms, frame_elapsed_ms));
        }

        fn command(&mut self, text: &str) {
            self.commands.push(text.to_string());
        }

        fn close(&mut self) -> Result<(), String> {
            self.closed = true;
            Ok(())
        }
    }

    struct FakePresentation {
        client: ClientId,
        player: Option<RemotePlayer>,
    }

    impl RemoteSeatPresentation for FakePresentation {
        fn client_id(&self) -> ClientId {
            self.client.clone()
        }

        fn player(&self) -> Option<RemotePlayer> {
            self.player
        }

        fn is_player(&self, actor: &RemoteActorId) -> bool {
            self.player.is_some_and(|player| player.actor == *actor)
        }

        fn scene(&self) -> RemoteSceneHandle {
            RemoteSceneHandle { generation: 1 }
        }

        fn output_frame(&self) -> Option<u64> {
            Some(7)
        }

        fn family(&self) -> RemoteSeatFamily {
            RemoteSeatFamily::Q3
        }
    }

    struct FakeQw {
        progress: Vec<RemoteDownloadProgress>,
        cancelled: bool,
        retried: bool,
        closed: bool,
    }

    impl QwDownloadManager for FakeQw {
        fn progress(&self) -> Vec<RemoteDownloadProgress> {
            self.progress.clone()
        }

        fn cancel(&mut self) {
            self.cancelled = true;
        }

        fn retry(&mut self) {
            self.retried = true;
        }

        fn close(&mut self) {
            self.closed = true;
        }
    }

    fn owner() -> IdentityOwner {
        IdentityOwner::create("remote-seat-source-test").unwrap()
    }

    fn source(
        family: RemoteSeatFamily,
        address_kind: RemoteAddressKind,
        download_policy: bool,
    ) -> RemoteSeatSource<FakeNetwork, FakePresentation> {
        let ident = owner();
        RemoteSeatSource::open(RemoteSeatSourceOptions {
            family,
            address_kind,
            qport: 27960,
            download_policy,
            remote: FakePresentation {
                client: ident.client(0, 0),
                player: None,
            },
            network: FakeNetwork {
                phase: "active".to_string(),
                timed: false,
                submitted: Vec::new(),
                commands: Vec::new(),
                closed: false,
            },
        })
        .unwrap()
    }

    #[test]
    fn open_validates_family_transports() {
        assert_eq!(
            source(RemoteSeatFamily::Qw, RemoteAddressKind::Udp, false).qport(),
            27960
        );
        assert_eq!(
            RemoteSeatSource::open(RemoteSeatSourceOptions {
                family: RemoteSeatFamily::Qw,
                address_kind: RemoteAddressKind::Ipx,
                qport: 1,
                download_policy: false,
                remote: FakePresentation {
                    client: owner().client(0, 0),
                    player: None
                },
                network: FakeNetwork {
                    phase: String::new(),
                    timed: false,
                    submitted: Vec::new(),
                    commands: Vec::new(),
                    closed: false,
                },
            })
            .err()
            .unwrap(),
            RemoteSeatSourceError::QuakeWorldRequiresUdp
        );
        let ident = owner();
        assert_eq!(
            RemoteSeatSource::open(RemoteSeatSourceOptions {
                family: RemoteSeatFamily::Q3,
                address_kind: RemoteAddressKind::Udp,
                qport: 1,
                download_policy: false,
                remote: FakePresentation {
                    client: ident.client(0, 0),
                    player: None
                },
                network: FakeNetwork {
                    phase: String::new(),
                    timed: false,
                    submitted: Vec::new(),
                    commands: Vec::new(),
                    closed: false,
                },
            })
            .err()
            .unwrap(),
            RemoteSeatSourceError::Q3RequiresIpv4
        );
        let ident = owner();
        assert_eq!(
            RemoteSeatSource::open(RemoteSeatSourceOptions {
                family: RemoteSeatFamily::Q2,
                address_kind: RemoteAddressKind::Udp,
                qport: 1,
                download_policy: false,
                remote: FakePresentation {
                    client: ident.client(0, 0),
                    player: None
                },
                network: FakeNetwork {
                    phase: String::new(),
                    timed: false,
                    submitted: Vec::new(),
                    commands: Vec::new(),
                    closed: false,
                },
            })
            .err()
            .unwrap(),
            RemoteSeatSourceError::Q2RequiresDownloadPolicy
        );
    }

    #[test]
    fn downloads_require_attach_and_report_progress() {
        let mut source = source(RemoteSeatFamily::Qw, RemoteAddressKind::Udp, false);
        assert_eq!(
            source.require_qw_downloads().err().unwrap(),
            RemoteSeatSourceError::QwDownloadBeforeServerData
        );
        assert_eq!(
            source.require_q3_downloads().err().unwrap(),
            RemoteSeatSourceError::Q3DownloadWithoutOwner
        );
        assert!(source.download_progress().is_empty());
        source.attach_qw_downloads(Box::new(FakeQw {
            progress: vec![RemoteDownloadProgress {
                name: "pak0.pk3".to_string(),
                received: 1,
                total: 2,
            }],
            cancelled: false,
            retried: false,
            closed: false,
        }));
        assert_eq!(source.download_progress().len(), 1);
        source.cancel_downloads();
        source.retry_downloads();
        source.close_qw_downloads();
        assert!(source.download_progress().is_empty());
        assert!(source.download_generation() > 0);
    }

    #[test]
    fn close_retires_once_and_blocks_commands() {
        let mut source = source(RemoteSeatFamily::Q3, RemoteAddressKind::Ipv4, false);
        source.set_all_skins("all");
        assert_eq!(source.all_skins(), "all");
        assert!(source.pure_command(1).is_none());
        source.send_command("status").unwrap();
        source.close().unwrap();
        assert!(source.is_closed());
        assert_eq!(source.send_command("status"), Err(RemoteSeatSourceError::Retired));
        source.close().unwrap();
    }

    #[test]
    fn inbound_events_filter_and_retarget() {
        let owner = owner();
        let seat = owner.seat(0);
        let client = owner.client(0, 0);
        let other = owner.seat(1);
        let direct = RemoteInboundEvent {
            name: "hit".to_string(),
            seat: Some(seat.clone()),
            client: Some(client.clone()),
            broadcast: false,
        };
        assert!(direct.targets_seat(&seat, &client));
        assert!(!direct.targets_seat(&other, &client));
        let broadcast = RemoteInboundEvent {
            name: "msg".to_string(),
            seat: None,
            client: None,
            broadcast: true,
        };
        assert!(broadcast.targets_seat(&other, &client));
        let retargeted = direct.for_seat(&seat);
        assert_eq!(retargeted.seat, seat);
    }
}
