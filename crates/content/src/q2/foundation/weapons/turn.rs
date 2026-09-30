//! Weapon turn state (`src/content/q2/foundation/weapons/turn.ts`).
//!
//! Native ClientThink / ClientBeginServerFrame weapon command
//! continuation.

/// Weapon turn state (`Q2WeaponTurnState`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Q2WeaponTurnState {
    /// Buttons.
    pub buttons: i32,
    /// Latched buttons.
    pub latched_buttons: i32,
    /// Weapon thunk.
    pub weapon_thunk: bool,
}

/// Latch weapon buttons (`latchQ2WeaponButtons`).
pub fn latch_q2_weapon_buttons(state: &mut Q2WeaponTurnState, buttons: i32) {
    state.latched_buttons |= buttons & !state.buttons;
    state.buttons = buttons;
}

/// Early weapon turn (`earlyQ2WeaponTurn`).
pub fn early_q2_weapon_turn(state: &mut Q2WeaponTurnState, tick: &mut dyn FnMut(bool)) {
    if state.latched_buttons & 1 != 0 && !state.weapon_thunk {
        state.weapon_thunk = true;
        tick(true);
    }
}

/// Begin weapon turn (`beginQ2WeaponTurn`).
pub fn begin_q2_weapon_turn(state: &mut Q2WeaponTurnState, allowed: bool, tick: &mut dyn FnMut(bool)) {
    if allowed && !state.weapon_thunk {
        tick(state.latched_buttons & 1 != 0);
    } else {
        state.weapon_thunk = false;
    }
}
