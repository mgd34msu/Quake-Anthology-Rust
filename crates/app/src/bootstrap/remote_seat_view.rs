//! Per-channel remote view: presentation, effects, components, and Q3 guest.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/remote-seat-view.ts`
//! (`RemoteSeatViewOptions`, `RemoteSeatView`). A channel's view and PVS
//! effects borrow the retained frontend assets and output. All collaborators
//! ([`ApplicationAssets`](super::assets::ApplicationAssets) plus
//! [`effects`](super::effects),
//! [`SeatUiData`](super::presentation::SeatUiData), [`qa_guest`],
//! [`presentation`](super::presentation), [`component_media`](super::component_media),
//! [`renderer`](super::renderer), [`audio`](super::audio),
//! [`content`](super::content), [`input`](super::input)) arrive through the
//! [`RemoteSeatViewBackend`] seam; this module owns the orchestration:
//! preparation order, view-angle conversion, client-state mapping, effect
//! reporting, and aggregate cleanup with the donor's exact texts. Sync port:
//! the donor's async preparation/frames become sync backend calls.

use std::collections::HashSet;

use qa_core::identity::SeatId;
use thiserror::Error;

use super::remote_seat_source::RemoteInboundEvent;
use super::remote_seat_source::RemoteSceneHandle;
use super::remote_seat_source::RemoteSeatFamily;

/// Remote seat view failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RemoteSeatViewError {
    /// Q3 seat without admitted guest owners.
    #[error("Q3 remote seat requires its admitted guest owners")]
    Q3RequiresGuest,
    /// Frame on a closed view.
    #[error("Remote seat view is closed")]
    Closed,
    /// Stale view (donor `assertCurrent`).
    #[error("Remote seat view is stale")]
    Stale,
    /// Preparation cleanup failure (donor `AggregateError` member texts).
    #[error("Remote seat view preparation failed")]
    PreparationFailed {
        /// Member failure messages.
        errors: Vec<String>,
    },
    /// Retirement cleanup failure (donor `AggregateError` member texts).
    #[error("Remote seat view retirement failed")]
    RetirementFailed {
        /// Member failure messages.
        errors: Vec<String>,
    },
    /// Backend failure.
    #[error("{0}")]
    Backend(String),
}

/// Optional effect preload failure (donor `{ content, path, error }` row).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteViewPreloadFailure {
    /// Content id.
    pub content: String,
    /// Resource path.
    pub path: String,
    /// Failure text.
    pub error: String,
}

/// Unhandled effect (donor `{ source: { content, kind }, reason }` row).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteViewEffect {
    /// Effect source content.
    pub source_content: String,
    /// Effect source kind.
    pub source_kind: String,
    /// Reason text.
    pub reason: String,
}

/// One drained effect sound (absorbed audio payload).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteViewEffectSound {
    /// Sound name.
    pub name: String,
    /// Whether the sound targets the seat player.
    pub for_player: bool,
}

/// Prepared audio seat events (absorbed `ApplicationAudioSeatEvents`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteViewAudioEvents {
    /// Seat id.
    pub seat: SeatId,
    /// Frame number.
    pub frame: u64,
    /// Presentation events.
    pub events: Vec<RemoteInboundEvent>,
    /// Scene handle.
    pub scene: RemoteSceneHandle,
    /// Music flag (donor always false here).
    pub music: bool,
    /// Effect sounds.
    pub effect_sounds: Vec<RemoteViewEffectSound>,
}

/// View angles in degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RemoteViewAngles {
    /// Pitch.
    pub x: f64,
    /// Yaw.
    pub y: f64,
    /// Roll.
    pub z: f64,
}

/// Convert Q3 command angles to degrees (`(v << 16 >> 16) * 360 / 65536`).
#[must_use]
pub fn q3_view_angles(command: Option<[i32; 3]>) -> RemoteViewAngles {
    let Some(angles) = command else {
        return RemoteViewAngles { x: 0.0, y: 0.0, z: 0.0 };
    };
    let convert = |value: i32| f64::from((((value as u32) << 16) as i32) >> 16) * (360.0 / 65536.0);
    RemoteViewAngles {
        x: convert(angles[0]),
        y: convert(angles[1]),
        z: convert(angles[2]),
    }
}

/// Map a network phase to the Q3 client-state number (active 8, loading 6, else 5).
#[must_use]
pub const fn client_state_phase(phase: &str) -> u8 {
    match phase.as_bytes() {
        b"active" => 8,
        b"loading" => 6,
        _ => 5,
    }
}

/// View options (donor `RemoteSeatViewOptions` owner callbacks).
pub struct RemoteSeatViewOptions {
    /// Seat index callback.
    pub index: Box<dyn Fn() -> usize>,
    /// Seat count callback.
    pub count: Box<dyn Fn() -> usize>,
    /// Wall clock in milliseconds.
    pub now: Box<dyn Fn() -> f64>,
    /// Fail when the view is stale.
    pub assert_current: Box<dyn Fn() -> Result<(), RemoteSeatViewError>>,
    /// Print diagnostics.
    pub print: Box<dyn FnMut(&str)>,
    /// Presentation family.
    pub family: RemoteSeatFamily,
    /// Network phase.
    pub phase: Box<dyn Fn() -> String>,
    /// Q3 guest owners present.
    pub q3_guest: bool,
    /// Seat identity.
    pub seat: SeatId,
    /// One-based player slot for effect reports.
    pub player_slot: u32,
    /// Latest Q3 command angles, when readable.
    pub q3_command_angles: Option<[i32; 3]>,
}

/// View collaborators ([`ApplicationAssets`](super::assets::ApplicationAssets)
/// plus other landed homes behind one seam).
pub trait RemoteSeatViewBackend {
    /// Backend failure.
    type Error: std::fmt::Display;
    /// Load world assets; true means the view owns them.
    fn prepare_assets(&mut self) -> Result<bool, Self::Error>;
    /// Load the console font.
    fn load_console_font(&mut self) -> Result<(), Self::Error>;
    /// Load menu typography.
    fn load_menu_typography(&mut self) -> Result<(), Self::Error>;
    /// Preload transient effect resources.
    fn preload_effects(&mut self) -> Result<Vec<RemoteViewPreloadFailure>, Self::Error>;
    /// Open the seat UI.
    fn open_ui(&mut self) -> Result<(), Self::Error>;
    /// Create the Q3 guest client.
    fn create_q3(&mut self, angles: RemoteViewAngles, split_screen: bool, phase: u8) -> Result<(), Self::Error>;
    /// Open the seat presentation.
    fn open_presentation(&mut self) -> Result<(), Self::Error>;
    /// Publish the layout.
    fn publish_layout(&mut self, index: usize, count: usize) -> Result<(), Self::Error>;
    /// Validate the presentation against the seat.
    fn validate_presentation(&mut self) -> Result<(), Self::Error>;
    /// Attach the presentation to the seat.
    fn attach_presentation(&mut self) -> Result<(), Self::Error>;
    /// Drain presentation events.
    fn drain_presentation_events(&mut self) -> Vec<RemoteInboundEvent>;
    /// Drain unified simulation events into the local seat.
    fn deliver_simulation_events(&mut self);
    /// Receive events into effects.
    fn effects_receive(&mut self, events: &[RemoteInboundEvent]);
    /// Prepare effects for a frame; false skips preparation (Q3 family).
    fn effects_prepare(&mut self, frame: u64) -> Result<(), Self::Error>;
    /// Drain unhandled effects.
    fn drain_unhandled_effects(&mut self) -> Vec<RemoteViewEffect>;
    /// Prepare component media.
    fn components_prepare_media(&mut self, music: bool) -> Result<(), Self::Error>;
    /// Publish source events into the presentation.
    fn presentation_source_events(&mut self, events: &[RemoteInboundEvent]);
    /// Prepare the presentation for a frame.
    fn presentation_prepare(&mut self, frame: u64) -> Result<(), Self::Error>;
    /// Prepare components for a frame.
    fn components_prepare(&mut self, frame: u64) -> Result<(), Self::Error>;
    /// Drain effect sounds.
    fn drain_effect_sounds(&mut self) -> Vec<RemoteViewEffectSound>;
    /// Close components.
    fn close_components(&mut self) -> Result<(), Self::Error>;
    /// Shut down the Q3 guest.
    fn shutdown_q3(&mut self) -> Result<(), Self::Error>;
    /// Whether the seat still holds the presentation.
    fn seat_holds_presentation(&self) -> bool;
    /// Clear the seat presentation.
    fn clear_presentation(&mut self) -> Result<(), Self::Error>;
    /// Close the presentation directly.
    fn close_presentation(&mut self) -> Result<(), Self::Error>;
    /// Close effects.
    fn close_effects(&mut self) -> Result<(), Self::Error>;
    /// Close owned assets.
    fn close_assets(&mut self) -> Result<(), Self::Error>;
    /// Scene handle for audio events.
    fn scene(&self) -> RemoteSceneHandle;
}

/// A channel's view and PVS effects.
pub struct RemoteSeatView {
    options_print: Box<dyn FnMut(&str)>,
    count: Box<dyn Fn() -> usize>,
    index: Box<dyn Fn() -> usize>,
    now: Box<dyn Fn() -> f64>,
    assert_current: Box<dyn Fn() -> Result<(), RemoteSeatViewError>>,
    family: RemoteSeatFamily,
    phase: Box<dyn Fn() -> String>,
    q3_guest: bool,
    seat: SeatId,
    player_slot: u32,
    q3_command_angles: Option<[i32; 3]>,
    owns_assets: bool,
    closed: bool,
    reported_effects: HashSet<String>,
}

impl RemoteSeatView {
    /// Prepare a view over backend collaborators.
    pub fn prepare<Backend: RemoteSeatViewBackend>(
        options: RemoteSeatViewOptions,
        backend: &mut Backend,
    ) -> Result<Self, RemoteSeatViewError> {
        let mut view = Self {
            options_print: options.print,
            count: options.count,
            index: options.index,
            now: options.now,
            assert_current: options.assert_current,
            family: options.family,
            phase: options.phase,
            q3_guest: options.q3_guest,
            seat: options.seat,
            player_slot: options.player_slot,
            q3_command_angles: options.q3_command_angles,
            owns_assets: false,
            closed: false,
            reported_effects: HashSet::new(),
        };
        let prepared = view.prepare_inner(backend);
        match prepared {
            Ok(()) => Ok(view),
            Err(error) => {
                let mut failures = vec![error.to_string()];
                view.close_prepared(backend, &mut failures);
                if failures.len() != 1 {
                    return Err(RemoteSeatViewError::PreparationFailed { errors: failures });
                }
                Err(error)
            }
        }
    }

    fn prepare_inner<Backend: RemoteSeatViewBackend>(
        &mut self,
        backend: &mut Backend,
    ) -> Result<(), RemoteSeatViewError> {
        self.owns_assets = backend
            .prepare_assets()
            .map_err(|error| RemoteSeatViewError::Backend(error.to_string()))?;
        backend
            .load_console_font()
            .map_err(|error| RemoteSeatViewError::Backend(error.to_string()))?;
        backend
            .load_menu_typography()
            .map_err(|error| RemoteSeatViewError::Backend(error.to_string()))?;
        (self.assert_current)()?;
        for failure in backend
            .preload_effects()
            .map_err(|error| RemoteSeatViewError::Backend(error.to_string()))?
        {
            (self.options_print)(&format!(
                "Optional effect preload skipped: {}/{}: {}\n",
                failure.content, failure.path, failure.error
            ));
        }
        (self.assert_current)()?;
        backend
            .open_ui()
            .map_err(|error| RemoteSeatViewError::Backend(error.to_string()))?;
        if self.family == RemoteSeatFamily::Q3 {
            if !self.q3_guest {
                return Err(RemoteSeatViewError::Q3RequiresGuest);
            }
            let split_screen = (self.count)() > 1;
            backend
                .create_q3(
                    q3_view_angles(self.q3_command_angles),
                    split_screen,
                    client_state_phase(&(self.phase)()),
                )
                .map_err(|error| RemoteSeatViewError::Backend(error.to_string()))?;
            (self.assert_current)()?;
        }
        backend
            .open_presentation()
            .map_err(|error| RemoteSeatViewError::Backend(error.to_string()))?;
        backend
            .publish_layout((self.index)(), (self.count)())
            .map_err(|error| RemoteSeatViewError::Backend(error.to_string()))?;
        backend
            .validate_presentation()
            .map_err(|error| RemoteSeatViewError::Backend(error.to_string()))?;
        Ok(())
    }

    fn close_prepared<Backend: RemoteSeatViewBackend>(&mut self, backend: &mut Backend, failures: &mut Vec<String>) {
        if self.family == RemoteSeatFamily::Q3 {
            if let Err(error) = backend.shutdown_q3() {
                failures.push(error.to_string());
            }
        }
        for step in [
            backend.close_presentation().map_err(|error| error.to_string()),
            backend.close_effects().map_err(|error| error.to_string()),
        ] {
            if let Err(error) = step {
                failures.push(error);
            }
        }
        if self.owns_assets {
            if let Err(error) = backend.close_assets() {
                failures.push(error.to_string());
            }
        }
    }

    /// Publish the layout and attach the presentation.
    pub fn publish<Backend: RemoteSeatViewBackend>(
        &mut self,
        backend: &mut Backend,
        index: usize,
        count: usize,
    ) -> Result<(), RemoteSeatViewError> {
        (self.assert_current)()?;
        backend
            .publish_layout(index, count)
            .map_err(|error| RemoteSeatViewError::Backend(error.to_string()))?;
        backend
            .attach_presentation()
            .map_err(|error| RemoteSeatViewError::Backend(error.to_string()))
    }

    /// Whether the view is closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Wall clock in milliseconds.
    #[must_use]
    pub fn now(&self) -> f64 {
        (self.now)()
    }

    /// Prepare one frame, returning audio seat events.
    pub fn prepare_frame<Backend: RemoteSeatViewBackend>(
        &mut self,
        backend: &mut Backend,
        frame: u64,
        music: bool,
    ) -> Result<RemoteViewAudioEvents, RemoteSeatViewError> {
        if self.closed {
            return Err(RemoteSeatViewError::Closed);
        }
        let events = backend.drain_presentation_events();
        if self.family == RemoteSeatFamily::Unified {
            backend.deliver_simulation_events();
        }
        backend.effects_receive(&events);
        if self.family != RemoteSeatFamily::Q3 {
            backend
                .effects_prepare(frame)
                .map_err(|error| RemoteSeatViewError::Backend(error.to_string()))?;
        }
        for effect in backend.drain_unhandled_effects() {
            let key = format!("{}:{}", effect.source_content, effect.reason);
            if self.reported_effects.insert(key) {
                (self.options_print)(&format!(
                    "Player {}: unresolved {} effect: {}\n",
                    self.player_slot, effect.source_kind, effect.reason
                ));
            }
        }
        backend
            .components_prepare_media(music)
            .map_err(|error| RemoteSeatViewError::Backend(error.to_string()))?;
        backend.presentation_source_events(&events);
        backend
            .presentation_prepare(frame)
            .map_err(|error| RemoteSeatViewError::Backend(error.to_string()))?;
        backend
            .components_prepare(frame)
            .map_err(|error| RemoteSeatViewError::Backend(error.to_string()))?;
        Ok(RemoteViewAudioEvents {
            seat: self.seat.clone(),
            frame,
            events,
            scene: backend.scene(),
            music: false,
            effect_sounds: backend.drain_effect_sounds(),
        })
    }

    /// Retire the view, aggregating cleanup failures.
    pub fn close<Backend: RemoteSeatViewBackend>(&mut self, backend: &mut Backend) -> Result<(), RemoteSeatViewError> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        let mut errors = Vec::new();
        if let Err(error) = backend.close_components() {
            errors.push(error.to_string());
        }
        if self.family == RemoteSeatFamily::Q3 {
            if let Err(error) = backend.shutdown_q3() {
                errors.push(error.to_string());
            }
        }
        let cleared = if backend.seat_holds_presentation() {
            backend.clear_presentation()
        } else {
            backend.close_presentation()
        };
        if let Err(error) = cleared {
            errors.push(error.to_string());
        }
        if let Err(error) = backend.close_effects() {
            errors.push(error.to_string());
        }
        if self.owns_assets {
            if let Err(error) = backend.close_assets() {
                errors.push(error.to_string());
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(RemoteSeatViewError::RetirementFailed { errors })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use qa_core::identity::IdentityOwner;

    use super::*;

    #[test]
    fn converts_q3_view_angles() {
        assert_eq!(q3_view_angles(None), RemoteViewAngles { x: 0.0, y: 0.0, z: 0.0 });
        let angles = q3_view_angles(Some([16384, -16384, 65536 + 8192]));
        assert!((angles.x - 90.0).abs() < 1e-9);
        assert!((angles.y + 90.0).abs() < 1e-9);
        assert!((angles.z - 45.0).abs() < 1e-9);
    }

    #[test]
    fn maps_client_state_phases() {
        assert_eq!(client_state_phase("active"), 8);
        assert_eq!(client_state_phase("loading"), 6);
        assert_eq!(client_state_phase("idle"), 5);
    }

    struct FakeBackend {
        fail_ui: bool,
        unhandled: Vec<RemoteViewEffect>,
        holds: bool,
        fail_effects_close: bool,
        created_q3: Vec<(RemoteViewAngles, bool, u8)>,
    }

    impl RemoteSeatViewBackend for FakeBackend {
        type Error = String;

        fn prepare_assets(&mut self) -> Result<bool, String> {
            Ok(true)
        }

        fn load_console_font(&mut self) -> Result<(), String> {
            Ok(())
        }

        fn load_menu_typography(&mut self) -> Result<(), String> {
            Ok(())
        }

        fn preload_effects(&mut self) -> Result<Vec<RemoteViewPreloadFailure>, String> {
            Ok(vec![RemoteViewPreloadFailure {
                content: "fx".to_string(),
                path: "spark".to_string(),
                error: "missing".to_string(),
            }])
        }

        fn open_ui(&mut self) -> Result<(), String> {
            if self.fail_ui {
                return Err("ui failed".to_string());
            }
            Ok(())
        }

        fn create_q3(&mut self, angles: RemoteViewAngles, split_screen: bool, phase: u8) -> Result<(), String> {
            self.created_q3.push((angles, split_screen, phase));
            Ok(())
        }

        fn open_presentation(&mut self) -> Result<(), String> {
            Ok(())
        }

        fn publish_layout(&mut self, _index: usize, _count: usize) -> Result<(), String> {
            Ok(())
        }

        fn validate_presentation(&mut self) -> Result<(), String> {
            Ok(())
        }

        fn attach_presentation(&mut self) -> Result<(), String> {
            Ok(())
        }

        fn drain_presentation_events(&mut self) -> Vec<RemoteInboundEvent> {
            Vec::new()
        }

        fn deliver_simulation_events(&mut self) {}

        fn effects_receive(&mut self, _events: &[RemoteInboundEvent]) {}

        fn effects_prepare(&mut self, _frame: u64) -> Result<(), String> {
            Ok(())
        }

        fn drain_unhandled_effects(&mut self) -> Vec<RemoteViewEffect> {
            std::mem::take(&mut self.unhandled)
        }

        fn components_prepare_media(&mut self, _music: bool) -> Result<(), String> {
            Ok(())
        }

        fn presentation_source_events(&mut self, _events: &[RemoteInboundEvent]) {}

        fn presentation_prepare(&mut self, _frame: u64) -> Result<(), String> {
            Ok(())
        }

        fn components_prepare(&mut self, _frame: u64) -> Result<(), String> {
            Ok(())
        }

        fn drain_effect_sounds(&mut self) -> Vec<RemoteViewEffectSound> {
            Vec::new()
        }

        fn close_components(&mut self) -> Result<(), String> {
            Ok(())
        }

        fn shutdown_q3(&mut self) -> Result<(), String> {
            Ok(())
        }

        fn seat_holds_presentation(&self) -> bool {
            self.holds
        }

        fn clear_presentation(&mut self) -> Result<(), String> {
            Ok(())
        }

        fn close_presentation(&mut self) -> Result<(), String> {
            Ok(())
        }

        fn close_effects(&mut self) -> Result<(), String> {
            if self.fail_effects_close {
                return Err("effects stuck".to_string());
            }
            Ok(())
        }

        fn close_assets(&mut self) -> Result<(), String> {
            Ok(())
        }

        fn scene(&self) -> RemoteSceneHandle {
            RemoteSceneHandle { generation: 3 }
        }
    }

    fn options(
        prints: &Rc<RefCell<Vec<String>>>,
        owner: &IdentityOwner,
        family: RemoteSeatFamily,
    ) -> RemoteSeatViewOptions {
        let prints = Rc::clone(prints);
        RemoteSeatViewOptions {
            index: Box::new(|| 0),
            count: Box::new(|| 2),
            now: Box::new(|| 100.0),
            assert_current: Box::new(|| Ok(())),
            print: Box::new(move |text| prints.borrow_mut().push(text.to_string())),
            family,
            phase: Box::new(|| "active".to_string()),
            q3_guest: true,
            seat: owner.seat(0),
            player_slot: 1,
            q3_command_angles: Some([16384, 0, 0]),
        }
    }

    fn make_backend() -> FakeBackend {
        FakeBackend {
            fail_ui: false,
            unhandled: Vec::new(),
            holds: true,
            fail_effects_close: false,
            created_q3: Vec::new(),
        }
    }

    #[test]
    fn prepares_and_reports_preload_skips() {
        let prints = Rc::new(RefCell::new(Vec::new()));
        let owner = IdentityOwner::create("remote-seat-view-test").unwrap();
        let mut backend = make_backend();
        let mut view = RemoteSeatView::prepare(options(&prints, &owner, RemoteSeatFamily::Q1), &mut backend).unwrap();
        assert_eq!(
            *prints.borrow(),
            vec!["Optional effect preload skipped: fx/spark: missing\n".to_string()]
        );
        assert_eq!(view.now(), 100.0);
        view.publish(&mut backend, 0, 2).unwrap();
        let events = view.prepare_frame(&mut backend, 9, false).unwrap();
        assert_eq!(events.frame, 9);
        assert!(!events.music);
        view.close(&mut backend).unwrap();
        assert!(view.is_closed());
        view.close(&mut backend).unwrap();
    }

    #[test]
    fn q3_requires_guest_and_receives_angles() {
        let prints = Rc::new(RefCell::new(Vec::new()));
        let owner = IdentityOwner::create("remote-seat-view-q3").unwrap();
        let mut no_guest = options(&prints, &owner, RemoteSeatFamily::Q3);
        no_guest.q3_guest = false;
        let mut backend = make_backend();
        assert_eq!(
            RemoteSeatView::prepare(no_guest, &mut backend).err().unwrap(),
            RemoteSeatViewError::Q3RequiresGuest
        );
        let mut backend = make_backend();
        let view = RemoteSeatView::prepare(options(&prints, &owner, RemoteSeatFamily::Q3), &mut backend).unwrap();
        assert_eq!(backend.created_q3.len(), 1);
        assert!((backend.created_q3[0].0.x - 90.0).abs() < 1e-9);
        assert!(backend.created_q3[0].1);
        assert_eq!(backend.created_q3[0].2, 8);
        assert!(!view.is_closed());
    }

    #[test]
    fn reports_unhandled_effects_once_and_aggregates_close() {
        let prints = Rc::new(RefCell::new(Vec::new()));
        let owner = IdentityOwner::create("remote-seat-view-fx").unwrap();
        let mut backend = make_backend();
        backend.unhandled = vec![RemoteViewEffect {
            source_content: "fx".to_string(),
            source_kind: "particle".to_string(),
            reason: "no emitter".to_string(),
        }];
        let mut view = RemoteSeatView::prepare(options(&prints, &owner, RemoteSeatFamily::Q1), &mut backend).unwrap();
        view.prepare_frame(&mut backend, 1, false).unwrap();
        backend.unhandled = vec![RemoteViewEffect {
            source_content: "fx".to_string(),
            source_kind: "particle".to_string(),
            reason: "no emitter".to_string(),
        }];
        view.prepare_frame(&mut backend, 2, false).unwrap();
        let reports: Vec<_> = prints
            .borrow()
            .iter()
            .filter(|line| line.contains("unresolved"))
            .cloned()
            .collect();
        assert_eq!(
            reports,
            vec!["Player 1: unresolved particle effect: no emitter\n".to_string()]
        );
        backend.fail_effects_close = true;
        let error = view.close(&mut backend).err().unwrap();
        assert_eq!(
            error,
            RemoteSeatViewError::RetirementFailed {
                errors: vec!["effects stuck".to_string()]
            }
        );
        assert_eq!(
            view.prepare_frame(&mut backend, 3, false),
            Err(RemoteSeatViewError::Closed)
        );
    }

    #[test]
    fn preparation_failure_aggregates_cleanup() {
        let prints = Rc::new(RefCell::new(Vec::new()));
        let owner = IdentityOwner::create("remote-seat-view-fail").unwrap();
        let mut backend = make_backend();
        backend.fail_ui = true;
        backend.fail_effects_close = true;
        let error = RemoteSeatView::prepare(options(&prints, &owner, RemoteSeatFamily::Q1), &mut backend)
            .err()
            .unwrap();
        assert_eq!(
            error,
            RemoteSeatViewError::PreparationFailed {
                errors: vec!["ui failed".to_string(), "effects stuck".to_string()]
            }
        );
    }
}
