//! Weapon damage multiplier (`src/content/q2/foundation/weapons/damage.ts`).

use qa_core::identity::ActorId;

use crate::q2::foundation::host::Q2GameServices;
use crate::q2::foundation::weapons::types::Q2WeaponInput;

/// Weapon damage multiplier (`q2WeaponDamageMultiplier`).
pub fn q2_weapon_damage_multiplier(actor: &ActorId, input: &Q2WeaponInput, now: f64, game: &mut Q2GameServices) -> f64 {
    let quad = input.quad_until > now;
    let engine = game
        .weapons
        .engine
        .as_mut()
        .unwrap_or_else(|| panic!("Q2 weapons require a session engine"));
    (if quad { engine.quad_multiplier(actor) } else { 1.0 })
        * (if input.double_until > now && !(quad && input.no_stack_double) {
            2.0
        } else {
            1.0
        })
        * engine.source_damage_multiplier(actor)
}
