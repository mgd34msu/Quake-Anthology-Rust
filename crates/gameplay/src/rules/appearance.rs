//! Source-native appearance decoding, independent of presentation and physics.
use qa_core::{
    math,
    primitives::{EntityAppearance, RuleSetId, Vec3},
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AppearanceCapabilities {
    pub viewer_model: bool,
    pub weapon_model: bool,
    pub depth_hack: bool,
    pub fullbright: bool,
    /// Use the current origin/old_origin frame path instead of interpolating
    /// origins: Q2 RF_FRAMELERP and rerelease RF_NO_ORIGIN_LERP.
    pub frame_lerp: bool,
    pub state_old_frame: bool,
    pub alpha: f32,
    pub scale: f32,
    /// Nonzero means residual native presentation is not implemented by this
    /// projection. The original bits remain in the shared appearance record.
    pub unsupported: u64,
    pub drawable: bool,
}

#[derive(Clone, Copy)]
struct NativeRules {
    viewer: u32,
    weapon: u32,
    depth: u32,
    fullbright: u32,
    translucent: u32,
    frame_lerp: u32,
    old_frame: u32,
    beam: u32,
    bfg: u64,
    plasma: u64,
    sphere: u64,
    tracker: u64,
    native_alpha: bool,
    explicit_alpha: bool,
    zero_scale: bool,
    alias_light: Option<Vec3>,
    interpolation_events: [u32; 2],
    interpolation_distance: Option<i32>,
}
const NEUTRAL: NativeRules = NativeRules {
    viewer: 0,
    weapon: 0,
    depth: 0,
    fullbright: 0,
    translucent: 0,
    frame_lerp: 0,
    old_frame: 0,
    beam: 0,
    bfg: 0,
    plasma: 0,
    sphere: 0,
    tracker: 0,
    native_alpha: false,
    explicit_alpha: false,
    zero_scale: false,
    alias_light: None,
    interpolation_events: [0; 2],
    interpolation_distance: None,
};
// Original quake-2/game/q_shared.h render/effects values. Rerelease game.h
// retains these bits, renames bit6 NO_ORIGIN_LERP and adds OLD_FRAME_LERP.
const Q2: NativeRules = NativeRules {
    viewer: 2,
    weapon: 4,
    fullbright: 8,
    depth: 16,
    translucent: 32,
    frame_lerp: 64,
    beam: 128,
    bfg: 1 << 7,
    plasma: 1 << 24,
    sphere: 1 << 28,
    tracker: 1 << 31,
    native_alpha: true,
    alias_light: Some(Vec3([-1.0, 0.0, 0.0])),
    interpolation_events: [6, 7],
    interpolation_distance: Some(512),
    ..NEUTRAL
};
const RULES: [NativeRules; 5] = [
    NEUTRAL,
    NEUTRAL,
    Q2,
    NativeRules {
        old_frame: 1 << 22,
        explicit_alpha: true,
        zero_scale: true,
        ..Q2
    },
    // Q3 entity eFlags are not refEntity renderfx. An actual client-scene
    // boundary must supply a supported mapping before interpreting those bits.
    NEUTRAL,
];

/// Native Q2 software alias light, rotated with the original dot/negate order.
pub fn alias_light_direction(source: RuleSetId, angles: Vec3) -> Option<Vec3> {
    let light = RULES[source as usize].alias_light?;
    let basis = math::angle_vectors(angles);
    Some(Vec3([
        light.dot(basis.forward),
        -light.dot(basis.right),
        light.dot(basis.up),
    ]))
}

/// Q2 packet entities invalidate interpolation on native discontinuities.
/// Classic cl_ents.c uses integer abs on each floating displacement.
pub fn can_interpolate(
    previous: &EntityAppearance,
    current: &EntityAppearance,
    old: Vec3,
    new: Vec3,
) -> bool {
    if previous.source != current.source {
        return false;
    }
    let rules = RULES[current.source as usize];
    let Some(distance) = rules.interpolation_distance else {
        return true;
    };
    previous.models == current.models
        && !rules.interpolation_events.contains(&current.native.event)
        && (new - old)
            .0
            .iter()
            .all(|delta| delta.is_finite() && (*delta as i32).unsigned_abs() <= distance as u32)
}

fn native_alpha(flags: u32, effects: u64, rules: NativeRules) -> f32 {
    // Original cl_ents.c only assigns the black-hole alpha when renderfx is
    // exactly RF_TRANSLUCENT; additional combinations retain the zeroed alpha.
    let mut alpha = if flags == rules.translucent {
        0.7
    } else if flags & rules.translucent != 0 {
        0.0
    } else {
        1.0
    };
    if flags & rules.beam != 0 || effects & rules.bfg != 0 {
        alpha = 0.3;
    }
    if effects & rules.plasma != 0 {
        alpha = 0.6;
    }
    if effects & rules.sphere != 0 {
        alpha = if effects & rules.tracker != 0 {
            0.6
        } else {
            0.3
        };
    }
    alpha
}

pub fn project(appearance: &EntityAppearance) -> AppearanceCapabilities {
    let rules = RULES[appearance.source as usize];
    let flags = appearance.native.render_flags;
    let supported = rules.viewer
        | rules.weapon
        | rules.depth
        | rules.fullbright
        | rules.translucent
        | rules.frame_lerp
        | rules.old_frame;
    let alpha = if rules.native_alpha && !(rules.explicit_alpha && appearance.alpha != 0.0) {
        native_alpha(flags, appearance.native.effects, rules)
    } else {
        appearance.alpha
    };
    AppearanceCapabilities {
        viewer_model: flags & rules.viewer != 0,
        weapon_model: flags & rules.weapon != 0,
        depth_hack: flags & rules.depth != 0,
        fullbright: flags & rules.fullbright != 0,
        frame_lerp: flags & rules.frame_lerp != 0,
        state_old_frame: flags & rules.old_frame != 0,
        alpha,
        scale: if rules.zero_scale && appearance.scale == 0.0 {
            1.0
        } else {
            appearance.scale
        },
        unsupported: appearance.native.effects | u64::from(flags & !supported),
        drawable: flags & rules.beam == 0,
    }
}
