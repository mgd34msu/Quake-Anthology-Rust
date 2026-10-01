//! Weapon projection (`src/content/q2/foundation/weapons/projection.ts`).

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use crate::q2::foundation::host::{Q2Edition, Q2GameServices, Q2TraceRequest};
use crate::q2::foundation::weapons::types::{PLAYER_CONTENTS, PROJECTILE_MASK, SHOT_MASK, WeaponHand};
use crate::q2::foundation::weapons::vectors::{angle_vectors, vector_angles};
use crate::q2::support::contracts::TraceFamily;

/// Actor view for projection (`Q2ActorView`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2ActorView {
    /// Weapon hand.
    pub hand: WeaponHand,
    /// View height.
    pub view_height: f64,
    /// Whether players collide.
    pub players_collide: bool,
}

/// Actor shot mask (`q2ActorShotMask`).
pub fn q2_actor_shot_mask(game: &Q2GameServices, players_collide: bool) -> i32 {
    if game.options.edition == Q2Edition::Classic {
        return SHOT_MASK;
    }
    if players_collide {
        PROJECTILE_MASK
    } else {
        PROJECTILE_MASK & !PLAYER_CONTENTS
    }
}

/// Project an actor muzzle (`projectQ2Actor`).
pub fn project_q2_actor(
    actor: &ActorId,
    game: &mut Q2GameServices,
    view: &Q2ActorView,
    angles: Vec3,
    offset: Vec3,
) -> (Vec3, Vec3) {
    use qa_core::math::{add3, normalize3, scale3, sub3};

    let axes = angle_vectors(angles);
    let side = match view.hand {
        WeaponHand::Left => -offset.y,
        WeaponHand::Center => 0.0,
        WeaponHand::Right => offset.y,
    };
    let body = game
        .host
        .bodies()
        .read(actor)
        .unwrap_or_else(|| panic!("Q2 projection requires a shared actor body"));
    let origin = body.origin;
    if game.options.edition == Q2Edition::Classic {
        let start = add3(
            add3(add3(origin, scale3(axes.forward, offset.x)), scale3(axes.right, side)),
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: (view.view_height as f32) + offset.z,
            },
        );
        return (start, axes.forward);
    }
    let eye = add3(origin, Vec3 {
        x: 0.0,
        y: 0.0,
        z: view.view_height as f32,
    });
    let start = add3(
        add3(add3(eye, scale3(axes.forward, offset.x)), scale3(axes.right, side)),
        scale3(axes.up, offset.z),
    );
    let trace = game.host.trace(&Q2TraceRequest {
        start: eye,
        end: add3(eye, scale3(axes.forward, 8192.0)),
        bounds: None,
        ignore: Some(actor.clone()),
        mask: q2_actor_shot_mask(game, view.players_collide) & !0x4000000,
        exclude: Vec::new(),
    });
    let contents = match &trace.family {
        TraceFamily::Q1 { .. } => 0,
        TraceFamily::Q2(fields) => fields.contents,
        TraceFamily::Q3 { contents, .. } => *contents,
    };
    let close = !matches!(trace.family, TraceFamily::Q1 { .. })
        && contents & (0x2000000 | PLAYER_CONTENTS) != 0
        && trace.fraction * 8192.0 < 128.0;
    let direction = if trace.start_solid || close {
        axes.forward
    } else {
        normalize3(sub3(trace.end, start))
    };
    let _ = vector_angles;
    (start, direction)
}
