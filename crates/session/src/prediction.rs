//! Current-command prediction. Network command acknowledgement and correction
//! history belong to THE-821; this stores no journal or replay mechanism.
use qa_core::primitives::{PlayerState, UserCmd};
use qa_movement::{MovementResult, TraceServices};

#[derive(Default)]
pub struct Prediction {
    pub player: PlayerState,
}
impl Prediction {
    /// Copy hot movement fields only; inventory and module tails stay owned by
    /// the authoritative client. Applying a snapshot never clones an arena.
    pub fn apply_snapshot(&mut self, source: &PlayerState) {
        self.player.body = source.body;
        self.player.movement_rules = source.movement_rules;
        self.player.trace_rules = source.trace_rules;
        self.player.movement = source.movement;
        self.player.view_angles = source.view_angles;
        self.player.view_offset = source.view_offset;
    }
    pub fn advance(&mut self, command: UserCmd, trace: &mut dyn TraceServices) -> MovementResult {
        qa_movement::pmove(command, &mut self.player, trace)
    }
}
