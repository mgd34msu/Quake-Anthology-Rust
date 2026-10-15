//! Prediction replays the connection's common history after native acknowledgement.
use qa_core::command_history::{Acknowledgement, CommandHistory};
use qa_core::primitives::{PlayerState, UserCmd};
use qa_movement::{MovementResult, TraceServices};
use qa_world::collision::TraceError;

#[derive(Default)]
pub struct Prediction {
    pub player: PlayerState,
    baseline: PlayerState,
    baseline_valid: bool,
    acknowledged: Option<Acknowledgement>,
    packet_acknowledged: Option<u64>,
    history_exhausted: bool,
    view_angles: qa_movement::ViewAngleAdmission,
    pub history_resyncs: u64,
}
impl Prediction {
    pub fn new(player: PlayerState) -> Self {
        Self {
            player,
            ..Self::default()
        }
    }
    /// Copy hot movement fields only; inventory and weapon readiness stay owned by
    /// the authoritative client. Applying a snapshot never clones an arena.
    pub fn apply_snapshot(&mut self, source: &PlayerState) {
        copy_motion(source, &mut self.player);
        self.remember_snapshot(None);
    }
    pub fn observe_command_progress(&mut self, number: Option<u64>) {
        self.packet_acknowledged = number;
    }
    pub fn packet_acknowledged(&self) -> Option<Acknowledgement> {
        self.packet_acknowledged.map(Acknowledgement::Number)
    }
    pub fn clear_history(&mut self) {
        self.baseline_valid = false;
        self.acknowledged = None;
        self.packet_acknowledged = None;
        self.history_exhausted = false;
        self.view_angles = Default::default();
    }
    pub fn remember_snapshot(&mut self, acknowledged: Option<Acknowledgement>) {
        copy_motion(&self.player, &mut self.baseline);
        self.baseline_valid = true;
        self.acknowledged = acknowledged;
        self.history_exhausted = false;
        self.view_angles = Default::default();
    }
    pub fn replay(
        &mut self,
        history: &CommandHistory,
        trace: &mut dyn TraceServices,
    ) -> Result<usize, TraceError> {
        self.view_angles = Default::default();
        let Some(latest) = history.get(0) else {
            return Ok(0);
        };
        if self.history_exhausted {
            self.advance_angles(*latest);
            return Ok(0);
        }
        if !self.baseline_valid || !self.player.prediction.position || self.acknowledged.is_none() {
            self.advance(*latest, trace)?;
            return Ok(1);
        }
        let tuning = self.player.movement.tuning;
        copy_motion(&self.baseline, &mut self.player);
        self.player.movement.tuning = tuning;
        let number = match self.acknowledged {
            Some(Acknowledgement::Number(number)) => Some(number),
            Some(Acknowledgement::ServerTime(time)) => history.number_at_time(time),
            None => None,
        };
        let pending = number.and_then(|number| history.pending_after(number).ok());
        let Some(pending) = pending else {
            self.history_resyncs = self.history_resyncs.saturating_add(1);
            self.acknowledged = None;
            self.history_exhausted = true;
            self.advance_angles(*latest);
            return Ok(0);
        };
        self.acknowledged = number.map(Acknowledgement::Number);
        let mut commands = 0;
        for command in pending {
            self.advance(*command, trace)?;
            commands += 1;
        }
        Ok(commands)
    }
    pub fn advance(
        &mut self,
        command: UserCmd,
        trace: &mut dyn TraceServices,
    ) -> Result<MovementResult, TraceError> {
        if !self.player.prediction.position {
            self.advance_angles(command);
            return Ok(MovementResult::default());
        }
        self.view_angles = Default::default();
        let options = qa_movement::MoveOptions {
            keep_view_angles: !self.player.prediction.angles,
            ..Default::default()
        };
        let result = qa_movement::pmove(
            command,
            &mut self.player,
            trace,
            options,
            qa_movement::no_boundary,
        )?;
        self.view_angles = result.view_angles;
        Ok(result)
    }
    /// An unavailable collision view still permits the native angle-only path.
    pub fn advance_angles(&mut self, command: UserCmd) {
        self.view_angles = if self.player.prediction.angles {
            qa_movement::predict_view_angles(command, &mut self.player)
        } else {
            Default::default()
        };
    }
    /// Drift extends only the angle projection that actually admitted this
    /// frame's view, including locks observed before movement changed timers.
    pub fn drift_pitch(&mut self, command: UserCmd, previous: f32, pitch: f32) {
        qa_movement::predict_pitch_drift(
            command,
            &mut self.player,
            previous,
            pitch,
            self.view_angles,
        );
    }
}

fn copy_motion(source: &PlayerState, target: &mut PlayerState) {
    target.body = source.body;
    target.movement_rules = source.movement_rules;
    target.trace_rules = source.trace_rules;
    target.movement = source.movement;
    target.native = source.native;
    target.prediction = source.prediction;
    target.view_angles = source.view_angles;
    target.view_offset = source.view_offset;
    target.ideal_pitch = source.ideal_pitch;
    target.native_clock = source.native_clock;
}
