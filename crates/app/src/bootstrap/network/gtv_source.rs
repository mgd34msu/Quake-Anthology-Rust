//! Quake II GTV remote source.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/network/gtv-source.ts`
//! (`GtvRemoteSource`, `GtvSourceOptions`). One native DATA message waits for
//! the retained presentation while TCP supplies backpressure. The donor is
//! asynchronous; this port resolves every step inline over the synchronous
//! [`GtvConnection`](qa_net::q2_svc::GtvConnection): `prepare` performs the
//! blocking hello handshake, starts the stream with the donor's default
//! buffering, and pumps the non-blocking connection until the first DATA
//! (bounded by the caller timeout, since a sync port has no abort signal).
//! MVD decoding reuses [`MvdPlayback`](qa_net::q2_svc::MvdPlayback) and
//! records decode through
//! [`Q2ClientReceiver`](super::q2_client_receiver::Q2ClientReceiver).

use std::time::{Duration, Instant};

use qa_net::msg::MsgReader;
use qa_net::q2_net::Q2ServerEvent;
use qa_net::q2_svc::{GtvConnection, GtvConnectionOptions, GtvEvent, GtvIdentity, MvdPlayback};
use qa_net::q2_variants::{read_mvd_header, MvdHeader};
use thiserror::Error;

use super::q2_client_receiver::{Q2ClientReceiver, Q2ClientReceiverError, Q2ClientReceiverHost, Q2DemoReceiverSource};
use super::q2_demo::{DelegatedVisibility, Q2MvdPresentation, Q2MvdView};
use super::types::ApplicationNetworkPhase;

/// GTV source failure.
#[derive(Debug, Error)]
pub enum GtvSourceError {
    /// Policy or protocol failure with donor text.
    #[error("{0}")]
    Message(String),
    /// Wire failure.
    #[error(transparent)]
    Net(#[from] qa_net::q2_net::Q2NetError),
    /// Receiver failure.
    #[error(transparent)]
    Receiver(#[from] Q2ClientReceiverError),
    /// MVD profile failure.
    #[error(transparent)]
    Variant(#[from] qa_net::q2_variants::VariantError),
}

/// GTV source options (`GtvSourceOptions`).
///
/// `timeout` bounds both the blocking hello handshake and the first-DATA
/// wait; the donor caller's abort signal has no sync equivalent.
pub struct GtvSourceOptions {
    /// Remote host.
    pub host: String,
    /// Remote port.
    pub port: u16,
    /// Viewer identity.
    pub identity: GtvIdentity,
    /// Handshake and admission timeout.
    pub timeout: Duration,
}

impl GtvSourceOptions {
    /// Build options with a 30-second timeout.
    pub fn new(host: String, port: u16, identity: GtvIdentity) -> Self {
        Self {
            host,
            port,
            identity,
            timeout: Duration::from_secs(30),
        }
    }
}

/// Admitted presentation owner.
struct GtvAdmitted<H> {
    /// Record receiver.
    receiver: Q2ClientReceiver<H, Q2DemoReceiverSource>,
    /// MVD decoder.
    decoder: MvdPlayback,
    /// Viewer binding.
    view: Box<dyn Q2MvdView>,
}

/// GTV remote source (`GtvRemoteSource`).
pub struct GtvRemoteSource<H> {
    connection: Option<GtvConnection>,
    admitted: Option<GtvAdmitted<H>>,
    pending: Option<Vec<u8>>,
    first_header: Option<MvdHeader>,
    ended: bool,
    polling: bool,
    failure: Option<String>,
}

impl<H: Q2ClientReceiverHost> GtvRemoteSource<H> {
    /// Connect, start the stream, and wait for the first DATA (`prepare`).
    pub fn prepare(options: &GtvSourceOptions) -> Result<Self, GtvSourceError> {
        let mut connection = GtvConnection::connect(&GtvConnectionOptions {
            host: options.host.clone(),
            port: options.port,
            identity: options.identity.clone(),
            timeout: options.timeout,
        })
        .map_err(|error| GtvSourceError::Message(error.to_string()))?;
        if let Err(error) = connection.start(10) {
            connection.close();
            return Err(GtvSourceError::Message(error.to_string()));
        }
        let mut source = Self {
            connection: Some(connection),
            admitted: None,
            pending: None,
            first_header: None,
            ended: false,
            polling: false,
            failure: None,
        };
        let deadline = Instant::now() + options.timeout;
        loop {
            let events = match source.connection.as_mut().expect("GTV connection").poll() {
                Ok(events) => events,
                Err(error) => {
                    source.close();
                    return Err(GtvSourceError::Message(error.to_string()));
                }
            };
            let mut failed = None;
            for event in events {
                if let Err(error) = source.receive(event) {
                    failed = Some(error);
                    break;
                }
            }
            if let Some(error) = failed {
                source.close();
                return Err(error);
            }
            if source.first_header.is_some() {
                return Ok(source);
            }
            if source.ended {
                let reason = source
                    .failure
                    .clone()
                    .unwrap_or_else(|| "GTV source closed before admission".to_string());
                return Err(GtvSourceError::Message(reason));
            }
            if Instant::now() > deadline {
                source.close();
                return Err(GtvSourceError::Message(
                    "GTV source timed out before admission".to_string(),
                ));
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// Admitted MVD header (`header`).
    pub fn header(&self) -> Result<&MvdHeader, GtvSourceError> {
        self.first_header
            .as_ref()
            .ok_or_else(|| GtvSourceError::Message("GTV source has no admitted header".to_string()))
    }

    /// Source phase (`phase`).
    #[must_use]
    pub fn phase(&self) -> ApplicationNetworkPhase {
        if self.ended {
            ApplicationNetworkPhase::Closed
        } else {
            self.admitted
                .as_ref()
                .map_or(ApplicationNetworkPhase::Loading, |admitted| admitted.receiver.phase())
        }
    }

    /// Recorded time in milliseconds (`time`).
    #[must_use]
    pub fn time(&self) -> Option<f64> {
        self.admitted
            .as_ref()
            .and_then(|admitted| admitted.receiver.recorded_time_milliseconds())
    }

    /// Admit the presentation owner (`bind`).
    pub fn bind(&mut self, host: H, presentation: Q2MvdPresentation) -> Result<(), GtvSourceError> {
        if self.ended || self.admitted.is_some() || self.first_header.is_none() {
            return Err(GtvSourceError::Message(
                "GTV presentation admission is not available".to_string(),
            ));
        }
        let decoder = MvdPlayback::new(DelegatedVisibility(presentation.visibility))?;
        let receiver = Q2ClientReceiver::new(host, Q2DemoReceiverSource)?;
        self.admitted = Some(GtvAdmitted {
            receiver,
            decoder,
            view: presentation.select_view,
        });
        Ok(())
    }

    /// Handle one connection event (`receive`).
    fn receive(&mut self, event: GtvEvent) -> Result<(), GtvSourceError> {
        if self.ended {
            return Ok(());
        }
        match event {
            GtvEvent::Closed { reason } => {
                self.failure = Some(reason);
                self.close();
            }
            GtvEvent::Data { bytes } => {
                if self.pending.is_some() {
                    return Err(GtvSourceError::Message(
                        "GTV DATA dispatch overlapped its consumer".to_string(),
                    ));
                }
                if self.first_header.is_none() {
                    self.first_header = Some(read_mvd_header(&mut MsgReader::new(&bytes))?);
                }
                self.pending = Some(bytes);
            }
            _ => {}
        }
        Ok(())
    }

    /// Decode the pending DATA message (`poll`).
    pub fn poll(&mut self) -> Result<(), GtvSourceError> {
        if let Some(connection) = self.connection.as_mut() {
            let events = connection
                .poll()
                .map_err(|error| GtvSourceError::Message(error.to_string()))?;
            for event in events {
                self.receive(event)?;
            }
        }
        if let Some(failure) = self.failure.clone() {
            return Err(GtvSourceError::Message(failure));
        }
        if self.ended || self.pending.is_none() {
            return Ok(());
        }
        if self.polling {
            return Err(GtvSourceError::Message(
                "GTV source poll is already in progress".to_string(),
            ));
        }
        if self.admitted.is_none() {
            return Err(GtvSourceError::Message(
                "GTV source has no presentation owner".to_string(),
            ));
        }
        self.polling = true;
        let outcome = self.decode_pending();
        self.polling = false;
        self.pending = None;
        if let Err(error) = outcome {
            self.close();
            return Err(error);
        }
        Ok(())
    }

    /// Decode loop body.
    fn decode_pending(&mut self) -> Result<(), GtvSourceError> {
        let bytes = self.pending.clone().expect("pending GTV payload");
        let admitted = self.admitted.as_mut().expect("GTV presentation owner");
        for record in admitted.decoder.read(&bytes)? {
            if matches!(record.event, Q2ServerEvent::Frame { .. }) {
                let selected = admitted.decoder.selected_player();
                admitted.view.select_view(selected);
            }
            admitted.receiver.receive_records(vec![record], 0, false)?;
            if self.ended {
                return Ok(());
            }
        }
        Ok(())
    }

    /// Select the viewed player (`selectPlayer`).
    pub fn select_player(&mut self, clientnum: i32) -> Result<(), GtvSourceError> {
        if self.admitted.is_none() || self.ended {
            return Err(GtvSourceError::Message("GTV source has no active viewer".to_string()));
        }
        let number = u8::try_from(clientnum)
            .map_err(|_| GtvSourceError::Message("GTV source has no active viewer".to_string()))?;
        self.admitted
            .as_mut()
            .expect("GTV presentation owner")
            .decoder
            .select_player(number)?;
        Ok(())
    }

    /// Close the source (`close`).
    pub fn close(&mut self) {
        if self.ended {
            return;
        }
        self.ended = true;
        if let Some(admitted) = self.admitted.as_mut() {
            admitted.receiver.close();
        }
        if let Some(connection) = self.connection.as_mut() {
            connection.close();
        }
        self.connection = None;
        self.pending = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_net::q2::{PlayerState, PmoveState, MAX_STATS_STORAGE};
    use qa_net::q2_net::Q2ServerMessageOptions;
    use qa_net::q2_svc::{MvdCapture, MvdChannel, MvdEncoder, MvdVisibility};
    use std::collections::BTreeMap;

    use crate::bootstrap::demo_recording::Q2ProtocolIdentity;

    struct GtvHost;

    impl Q2ClientReceiverHost for GtvHost {
        fn protocol(&self) -> Q2ProtocolIdentity {
            Q2ProtocolIdentity::Classic
        }

        fn message_options(&self) -> Q2ServerMessageOptions {
            Q2ServerMessageOptions::default()
        }

        fn server_data(&mut self, _data: &qa_net::q2_net::Q2ServerData, _assert_current: &dyn Fn()) {}

        fn game_state(&mut self, _state: &super::super::types::Q2ApplicationGameState) {}

        fn frame(
            &mut self,
            _frame: &qa_net::q2_net::Q2WireFrame,
            _records: &[qa_net::q2_net::Q2ServerRecord],
            _now: u64,
        ) {
        }

        fn records(&mut self, _records: &[qa_net::q2_net::Q2ServerRecord]) {}

        fn disconnected(&mut self, _reason: &str) {}

        fn print(&mut self, _text: &str) {}
    }

    struct PassthroughVisibility;

    impl MvdVisibility for PassthroughVisibility {
        fn entities(
            &self,
            entities: &[qa_net::q2::EntityState],
            _player: &PlayerState,
            _portal_bits: &[u8],
        ) -> Vec<qa_net::q2::EntityState> {
            entities.to_vec()
        }

        fn visible(&self, _leaf: u16, _channel: MvdChannel, _player: &PlayerState, _portal_bits: &[u8]) -> bool {
            true
        }

        fn area_bits(&self, _player: &PlayerState, _portal_bits: &[u8]) -> Vec<u8> {
            Vec::new()
        }

        fn sound_audible(&self, _origin: [f64; 3], _player: &PlayerState, _portal_bits: &[u8]) -> bool {
            true
        }

        fn sound_origin(&self, entity: &qa_net::q2::EntityState) -> [f64; 3] {
            entity.origin
        }
    }

    fn player_state() -> PlayerState {
        PlayerState {
            clientnum: 0,
            pmove: PmoveState {
                pm_type: 0,
                origin: [0, 0, 0],
                velocity: [0, 0, 0],
                pm_flags: 0,
                pm_time: 0,
                gravity: 0,
                delta_angles: [0, 0, 0],
                viewheight: 22,
                origin_f: [0.0, 0.0, 0.0],
                velocity_f: [0.0, 0.0, 0.0],
                delta_angles_f: [0.0, 0.0, 0.0],
                delta_angle_float: false,
            },
            viewangles: [0.0, 0.0, 0.0],
            viewoffset: [0.0, 0.0, 22.0],
            kick_angles: [0.0, 0.0, 0.0],
            gunangles: [0.0, 0.0, 0.0],
            gunoffset: [0.0, 0.0, 0.0],
            gunindex: 0,
            gunskin: 0,
            gunframe: 0,
            gunrate: 0,
            blend: [0.0, 0.0, 0.0, 0.0],
            damage_blend: [0.0, 0.0, 0.0, 0.0],
            fov: 90,
            rdflags: 0,
            stats: [0; MAX_STATS_STORAGE],
            team_id: 0,
            fog: Default::default(),
        }
    }

    fn gamestate_message() -> Vec<u8> {
        let mut encoder = MvdEncoder::new();
        let mut players = BTreeMap::new();
        players.insert(0u8, player_state());
        let capture = MvdCapture {
            revision: 2010,
            flags: 0,
            servercount: 2,
            gamedir: "baseq2".to_string(),
            dummy: 0,
            config_strings: BTreeMap::from([(0u16, "base1".to_string()), (30u16, "8".to_string())]),
            portal_bits: Vec::new(),
            players,
            entities: Vec::new(),
            messages: Vec::new(),
        };
        encoder.capture(&capture).expect("capture").remove(0)
    }

    fn presentation() -> Q2MvdPresentation {
        Q2MvdPresentation {
            visibility: Box::new(PassthroughVisibility),
            select_view: Box::new(()),
        }
    }

    fn unwrap_err<T>(result: Result<T, GtvSourceError>, what: &str) -> GtvSourceError {
        match result {
            Err(error) => error,
            Ok(_) => panic!("expected {what} error"),
        }
    }

    fn unbound() -> GtvRemoteSource<GtvHost> {
        GtvRemoteSource {
            connection: None,
            admitted: None,
            pending: None,
            first_header: None,
            ended: false,
            polling: false,
            failure: None,
        }
    }

    #[test]
    fn admission_requires_a_header() {
        let mut source = unbound();
        assert_eq!(source.phase(), ApplicationNetworkPhase::Loading);
        let error = unwrap_err(source.bind(GtvHost, presentation()), "bind");
        assert_eq!(error.to_string(), "GTV presentation admission is not available");
        let error = unwrap_err(source.header().cloned(), "header");
        assert_eq!(error.to_string(), "GTV source has no admitted header");
        let error = unwrap_err(source.select_player(0), "select");
        assert_eq!(error.to_string(), "GTV source has no active viewer");
    }

    #[test]
    fn data_admits_and_decodes_through_the_receiver() {
        let gamestate = gamestate_message();
        let mut source = unbound();
        source
            .receive(GtvEvent::Data {
                bytes: gamestate.clone(),
            })
            .expect("data");
        assert_eq!(source.header().expect("header").servercount, 2);
        source.bind(GtvHost, presentation()).expect("bind");
        source.poll().expect("poll");
        assert!(source.pending.is_none());
        // The gamestate precache activates demo sources (no downloads gate it).
        assert_eq!(source.phase(), ApplicationNetworkPhase::Active);
        source.select_player(0).expect("select");
    }

    #[test]
    fn overlapped_data_fails_with_donor_text() {
        let gamestate = gamestate_message();
        let mut source = unbound();
        source
            .receive(GtvEvent::Data {
                bytes: gamestate.clone(),
            })
            .expect("first");
        let error = unwrap_err(source.receive(GtvEvent::Data { bytes: gamestate }), "overlap");
        assert_eq!(error.to_string(), "GTV DATA dispatch overlapped its consumer");
    }

    #[test]
    fn poll_without_owner_fails_with_donor_text() {
        let gamestate = gamestate_message();
        let mut source = unbound();
        source.receive(GtvEvent::Data { bytes: gamestate }).expect("data");
        let error = unwrap_err(source.poll(), "owner");
        assert_eq!(error.to_string(), "GTV source has no presentation owner");
    }

    #[test]
    fn closed_events_retire_the_source() {
        let mut source = unbound();
        source
            .receive(GtvEvent::Closed {
                reason: "bye".to_string(),
            })
            .expect("closed");
        assert_eq!(source.phase(), ApplicationNetworkPhase::Closed);
        assert!(source.time().is_none());
        let error = unwrap_err(source.poll(), "failure");
        assert_eq!(error.to_string(), "bye");
        source.close();
        assert_eq!(source.phase(), ApplicationNetworkPhase::Closed);
    }
}
