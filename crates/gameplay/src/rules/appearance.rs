//! Source-native appearance decoding, independent of presentation and physics.
use qa_core::{
    math,
    names::compare_folded,
    primitives::{EntityAppearance, ModelUse, RuleSetId, Vec3},
};

/// Native model cells can select resources or the configured player binding.
/// Translucency belongs to a linked cell rather than its resource identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelInput {
    Resource(u32),
    LinkedTranslucent(u32),
    Player,
}

/// Filename-selected capabilities are cached at registration/configuration.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModelFeatures {
    pub linked_shell: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayerModelRules {
    pub weapon_limit: usize,
    pub weapon_start: usize,
    pub require_icon: bool,
    pub partial_base: bool,
    pub female_skin_fallback: bool,
    pub restricted_paths: bool,
    pub skip_player_model_gap: bool,
    pub folded_weapon_fallback: bool,
    weapon_mask: u32,
}
impl PlayerModelRules {
    /// RR reserves the upper bytes for height/team/POI. Classic retains the
    /// entire shifted weapon value, which the caller checks against its limit.
    pub fn player_selection(self, skin: i32) -> (usize, usize) {
        (
            (skin as u32 & 255) as usize,
            ((skin >> 8) as u32 & self.weapon_mask) as usize,
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelCellPresentation {
    pub model: ModelInput,
    pub skin: i32,
    pub capabilities: AppearanceCapabilities,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AppearanceCapabilities {
    pub viewer_model: bool,
    pub weapon_model: bool,
    pub depth_hack: bool,
    pub fullbright: bool,
    pub infrared_visible: bool,
    /// Use the current origin/old_origin frame path instead of interpolating
    /// origins: Q2 RF_FRAMELERP and rerelease RF_NO_ORIGIN_LERP.
    pub frame_lerp: bool,
    /// Classic Q2 applies transmitted origin motion inside alias blending.
    /// Rerelease bit 6 instead suppresses origin interpolation completely.
    pub alias_motion: bool,
    pub beam_radius: Option<f32>,
    pub beam_interpolate: bool,
    pub state_old_frame: bool,
    pub alpha: f32,
    pub inline_opacity: Option<f32>,
    pub scale: f32,
    /// Nonzero means residual native presentation is not implemented by this
    /// projection. The original bits remain in the shared appearance record.
    pub unsupported: u64,
}

/// Native animation capabilities, selected at the connection boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WeaponInterpolation {
    None,
    Snapshot,
    Rate,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum LinkedModels {
    Inherited,
    Opaque,
    CustomAlpha,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct NativeRules {
    random_model_sync: bool,
    player_model: Option<u32>,
    player_models: Option<PlayerModelRules>,
    secondary_model_alpha: Option<f32>,
    linked_shell: Option<&'static [u8]>,
    linked_models: LinkedModels,
    viewer: u32,
    weapon: u32,
    depth: u32,
    fullbright: u32,
    infrared_visible: u32,
    translucent: u32,
    inline_translucent_alpha: Option<f32>,
    frame_lerp: u32,
    alias_motion: bool,
    old_frame: u32,
    beam: u32,
    beam_integer_half: bool,
    beam_interpolate: bool,
    bfg: u64,
    plasma: u64,
    sphere: u64,
    tracker: u64,
    native_alpha: bool,
    explicit_alpha: bool,
    zero_scale: bool,
    alias_light: Option<Vec3>,
    alias_pitch_reversed: bool,
    gl_alias_roll_reversed: bool,
    interpolation_events: &'static [u32],
    interpolation_distance: Option<i32>,
    player_interpolation_events: bool,
    sequence_step: Option<u32>,
    weapon_interpolation: WeaponInterpolation,
    world_zero: bool,
    command_body_angles: bool,
    indexed_external_pcx: bool,
    software_alias: bool,
    snapshot_event_base: Option<u32>,
}
const NEUTRAL: NativeRules = NativeRules {
    random_model_sync: false,
    player_model: None,
    player_models: None,
    secondary_model_alpha: None,
    linked_shell: None,
    linked_models: LinkedModels::Inherited,
    viewer: 0,
    weapon: 0,
    depth: 0,
    fullbright: 0,
    infrared_visible: 0,
    translucent: 0,
    inline_translucent_alpha: None,
    frame_lerp: 0,
    alias_motion: false,
    old_frame: 0,
    beam: 0,
    beam_integer_half: false,
    beam_interpolate: false,
    bfg: 0,
    plasma: 0,
    sphere: 0,
    tracker: 0,
    native_alpha: false,
    explicit_alpha: false,
    zero_scale: false,
    alias_light: None,
    alias_pitch_reversed: false,
    gl_alias_roll_reversed: false,
    interpolation_events: &[],
    interpolation_distance: None,
    player_interpolation_events: false,
    sequence_step: None,
    weapon_interpolation: WeaponInterpolation::None,
    world_zero: false,
    command_body_angles: false,
    indexed_external_pcx: false,
    software_alias: false,
    snapshot_event_base: None,
};
// Original quake-2/game/q_shared.h render/effects values. Rerelease game.h
// retains these bits, renames bit6 NO_ORIGIN_LERP and adds OLD_FRAME_LERP.
const Q2: NativeRules = NativeRules {
    player_model: Some(255),
    player_models: Some(PlayerModelRules {
        weapon_limit: 20,
        weapon_start: 1,
        require_icon: true,
        partial_base: false,
        female_skin_fallback: false,
        restricted_paths: false,
        skip_player_model_gap: false,
        folded_weapon_fallback: false,
        weapon_mask: u32::MAX,
    }),
    secondary_model_alpha: Some(0.32),
    linked_models: LinkedModels::Opaque,
    viewer: 2,
    weapon: 4,
    fullbright: 8,
    infrared_visible: 0x8000,
    depth: 16,
    translucent: 32,
    inline_translucent_alpha: Some(0.25),
    frame_lerp: 64,
    alias_motion: true,
    beam: 128,
    beam_integer_half: true,
    bfg: 1 << 7,
    plasma: 1 << 24,
    sphere: 1 << 28,
    tracker: 1 << 31,
    native_alpha: true,
    alias_light: Some(Vec3([-1.0, 0.0, 0.0])),
    gl_alias_roll_reversed: true,
    interpolation_events: &[6, 7],
    interpolation_distance: Some(512),
    sequence_step: Some(1),
    weapon_interpolation: WeaponInterpolation::Snapshot,
    world_zero: true,
    indexed_external_pcx: true,
    software_alias: true,
    ..NEUTRAL
};
const RULES: [NativeRules; 5] = [
    NativeRules {
        random_model_sync: true,
        alias_pitch_reversed: true,
        world_zero: true,
        ..NEUTRAL
    },
    NativeRules {
        alias_pitch_reversed: true,
        world_zero: true,
        command_body_angles: true,
        ..NEUTRAL
    },
    Q2,
    NativeRules {
        player_models: Some(PlayerModelRules {
            weapon_limit: 256,
            weapon_start: 2,
            require_icon: false,
            partial_base: true,
            female_skin_fallback: true,
            restricted_paths: true,
            skip_player_model_gap: true,
            folded_weapon_fallback: true,
            weapon_mask: 255,
        }),
        secondary_model_alpha: None,
        linked_shell: Some(b"models/items/shell/tris.md2"),
        linked_models: LinkedModels::CustomAlpha,
        old_frame: 1 << 22,
        alias_motion: false,
        beam_integer_half: false,
        beam_interpolate: true,
        explicit_alpha: true,
        inline_translucent_alpha: None,
        zero_scale: true,
        player_interpolation_events: true,
        weapon_interpolation: WeaponInterpolation::Rate,
        ..Q2
    },
    // Q3 entity eFlags are not refEntity renderfx. An actual client-scene
    // boundary must supply a supported mapping before interpreting those bits.
    NativeRules {
        snapshot_event_base: Some(13),
        ..NEUTRAL
    },
];

/// Cached source semantics. These never choose a model's presentation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AppearancePolicy {
    rules: NativeRules,
    smooth_clients: bool,
}
pub fn policy(source: RuleSetId) -> AppearancePolicy {
    AppearancePolicy {
        rules: RULES[source as usize],
        smooth_clients: false,
    }
}

/// Resource-list rows have no cell use. Entity and view-weapon publication
/// select native interpretation explicitly without changing the raw reference.
pub fn model_input(source: RuleSetId, ordinal: u32, usage: Option<ModelUse>) -> ModelInput {
    model_input_rules(RULES[source as usize], ordinal, usage)
}

fn model_input_rules(rules: NativeRules, ordinal: u32, usage: Option<ModelUse>) -> ModelInput {
    if rules.player_model == Some(ordinal) && matches!(usage, Some(ModelUse::Entity(0 | 1))) {
        ModelInput::Player
    } else if rules.secondary_model_alpha.is_some()
        && usage == Some(ModelUse::Entity(1))
        && ordinal <= u8::MAX as u32
        && ordinal & 0x80 != 0
    {
        ModelInput::LinkedTranslucent(ordinal & 0x7f)
    } else {
        ModelInput::Resource(ordinal)
    }
}

impl AppearancePolicy {
    pub fn random_model_sync(self) -> bool {
        self.rules.random_model_sync
    }
    pub fn player_models(self) -> Option<PlayerModelRules> {
        self.rules.player_models
    }
    pub fn model_features(self, path: &[u8]) -> ModelFeatures {
        ModelFeatures {
            linked_shell: self
                .rules
                .linked_shell
                .is_some_and(|name| compare_folded(name, path).is_eq()),
        }
    }

    /// Linked model resets happen after the entity's common pose and frame
    /// have been selected. They affect only this model's draw properties.
    pub fn model_cell(
        self,
        appearance: &EntityAppearance,
        cell: usize,
        base: AppearanceCapabilities,
        features: ModelFeatures,
    ) -> Option<ModelCellPresentation> {
        let reference = (*appearance.models.get(cell)?)?;
        let model = model_input_rules(self.rules, reference.ordinal, Some(ModelUse::Entity(cell)));
        let mut capabilities = base;
        let mut skin = if model == ModelInput::Player {
            0
        } else {
            appearance.skin
        };
        if cell != 0 && self.rules.linked_models != LinkedModels::Inherited {
            skin = 0;
            capabilities.viewer_model = false;
            capabilities.weapon_model = false;
            capabilities.depth_hack = false;
            capabilities.fullbright = false;
            capabilities.infrared_visible = false;
            capabilities.alias_motion = false;
            capabilities.beam_radius = None;
            capabilities.alpha = match self.rules.linked_models {
                LinkedModels::CustomAlpha if appearance.alpha != 0.0 => appearance.alpha,
                _ => 1.0,
            };
            capabilities.inline_opacity = (capabilities.alpha != 1.0).then_some(capabilities.alpha);
            if let ModelInput::LinkedTranslucent(_) = model {
                capabilities.alpha = self.rules.secondary_model_alpha?;
                capabilities.inline_opacity = self.rules.inline_translucent_alpha;
            } else if cell == 1 && features.linked_shell && self.rules.linked_shell.is_some() {
                capabilities.alpha *= 0.32;
                capabilities.inline_opacity = Some(capabilities.alpha);
            }
        }
        Some(ModelCellPresentation {
            model,
            skin,
            capabilities,
        })
    }

    pub fn with_smooth_clients(mut self, enabled: bool) -> Self {
        self.smooth_clients = enabled;
        self
    }
    pub fn smooth_clients(self) -> bool {
        self.smooth_clients
    }
    /// Standalone temporary events need no model, sound or ordinary event word.
    /// Empty module anchors still do not become received entity poses.
    pub fn snapshot_present(self, appearance: &EntityAppearance) -> bool {
        self.rules
            .snapshot_event_base
            .is_some_and(|base| appearance.native.entity_type > base)
            || appearance.models.iter().any(Option::is_some)
            || appearance.native.effects != 0
            || appearance.native.event != 0
            || appearance.loop_sound.is_some()
            || appearance.presentation.is_some()
    }
    pub fn adjacent(self, current: u32, previous: u32) -> bool {
        self.rules
            .sequence_step
            .is_none_or(|step| current.wrapping_sub(previous) == step)
    }
    pub fn world_zero(self) -> bool {
        self.rules.world_zero
    }
    pub fn weapon_interpolation(self) -> WeaponInterpolation {
        self.rules.weapon_interpolation
    }
    pub fn player_event_check(self) -> bool {
        self.rules.player_interpolation_events
    }
}

/// Alias basis and light conversion, selected once for model presentation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AliasPose {
    pitch_reversed: bool,
    gl_roll_reversed: bool,
    light: Option<Vec3>,
}
impl AliasPose {
    pub fn angles(self, mut angles: Vec3, software: bool) -> Vec3 {
        if self.pitch_reversed {
            angles.0[0] = -angles.0[0];
        }
        if !software && self.gl_roll_reversed {
            angles.0[2] = -angles.0[2];
        }
        angles
    }
    pub fn light_direction(self, angles: Vec3) -> Option<Vec3> {
        let light = self.light?;
        let basis = math::angle_vectors(angles);
        Some(Vec3([
            light.dot(basis.forward),
            -light.dot(basis.right),
            light.dot(basis.up),
        ]))
    }
}
// Original ref_soft/r_polyse.c infrared palette substitution.
const INFRARED_MAP: [u8; 256] = [
    79, 78, 77, 76, 75, 74, 73, 72, 71, 70, 69, 68, 67, 66, 65, 64, 64, 65, 66, 67, 68, 69, 70, 71,
    72, 73, 74, 75, 76, 77, 78, 79, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 78, 79,
    208, 208, 208, 208, 208, 208, 208, 208, 64, 66, 68, 70, 72, 74, 76, 78, 64, 65, 66, 67, 68, 69,
    70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77,
    78, 79, 64, 66, 68, 70, 72, 74, 76, 78, 68, 67, 66, 65, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73,
    74, 75, 76, 76, 77, 77, 78, 78, 79, 79, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77,
    78, 79, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 64, 65, 66, 67, 68, 69,
    70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77,
    78, 79, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 64, 65, 66, 67, 68, 69,
    70, 71, 64, 65, 66, 67, 68, 69, 70, 71, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77,
    78, 79, 208, 208, 64, 64, 70, 71, 72, 64, 66, 68, 70, 64, 65, 66, 67, 68,
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelPresentation {
    pub pose: AliasPose,
    pub software_alias: bool,
    pub fullbright_light: [f32; 2],
    pub alpha_cutoffs: [f32; 2],
    pub infrared_map: Option<&'static [u8; 256]>,
}
pub fn model_presentation(source: RuleSetId) -> ModelPresentation {
    let rules = RULES[source as usize];
    ModelPresentation {
        pose: AliasPose {
            pitch_reversed: rules.alias_pitch_reversed,
            gl_roll_reversed: rules.gl_alias_roll_reversed,
            light: rules.alias_light,
        },
        software_alias: rules.software_alias,
        fullbright_light: [128.0, 64.0],
        alpha_cutoffs: [0.33, 0.66],
        infrared_map: (rules.infrared_visible != 0).then_some(&INFRARED_MAP),
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelSource {
    pub indexed_external_pcx: bool,
}
pub fn model_source(source: RuleSetId) -> ModelSource {
    ModelSource {
        indexed_external_pcx: RULES[source as usize].indexed_external_pcx,
    }
}

/// Values from the common cvar table, bound before entering the scene path.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AliasRoll {
    pub angle: f32,
    pub speed: f32,
}
impl AliasRoll {
    /// Native QW V_CalcRoll, shared by server command and client presentation.
    pub fn calculate(self, angles: Vec3, velocity: Vec3) -> f32 {
        let side = velocity.dot(math::angle_vectors(angles).right);
        let sign = if side < 0.0 { -1.0 } else { 1.0 };
        let magnitude = side.abs();
        let side = if magnitude < self.speed {
            magnitude * self.angle / self.speed
        } else {
            self.angle
        };
        side * sign
    }
}
/// Convert native command angles into body angles before model presentation.
/// QW playerinfo contains command angles; packetentities contain body angles.
impl AppearancePolicy {
    pub fn body_angles(
        self,
        mut angles: Vec3,
        velocity: Vec3,
        qw_player: bool,
        roll: AliasRoll,
    ) -> Vec3 {
        if self.rules.command_body_angles && qw_player {
            // QW CL_LinkPlayers and V_CalcRoll retain their native operation order.
            angles.0[0] = -angles.0[0] / 3.0;
            angles.0[2] = 0.0;
            angles.0[2] = roll.calculate(angles, velocity) * 4.0;
        }
        angles
    }

    /// Q2 packet entities invalidate interpolation on native discontinuities.
    /// Classic cl_ents.c uses integer abs on each floating displacement.
    pub fn can_interpolate(
        self,
        previous: &EntityAppearance,
        current: &EntityAppearance,
        old: Vec3,
        new: Vec3,
    ) -> bool {
        if previous.source != current.source {
            return false;
        }
        let rules = self.rules;
        let Some(distance) = rules.interpolation_distance else {
            return true;
        };
        previous.models == current.models
            && !self.is_interpolation_event(current)
            && (new - old)
                .0
                .iter()
                .all(|delta| delta.is_finite() && (*delta as i32).unsigned_abs() <= distance as u32)
    }

    pub fn is_interpolation_event(self, current: &EntityAppearance) -> bool {
        self.rules
            .interpolation_events
            .contains(&current.native.event)
    }
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

impl AppearancePolicy {
    pub fn project(self, appearance: &EntityAppearance) -> AppearanceCapabilities {
        let rules = self.rules;
        let flags = appearance.native.render_flags;
        let supported = rules.viewer
            | rules.weapon
            | rules.depth
            | rules.fullbright
            | rules.infrared_visible
            | rules.translucent
            | rules.frame_lerp
            | rules.old_frame;
        let supported = supported | rules.beam;
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
            infrared_visible: flags & rules.infrared_visible != 0,
            frame_lerp: flags & rules.frame_lerp != 0,
            alias_motion: flags & rules.frame_lerp != 0 && rules.alias_motion,
            beam_radius: (flags & rules.beam != 0).then(|| {
                if rules.beam_integer_half {
                    (appearance.frame / 2) as f32
                } else {
                    appearance.frame as f32 * 0.5
                }
            }),
            beam_interpolate: rules.beam_interpolate,
            state_old_frame: flags & rules.old_frame != 0,
            alpha,
            inline_opacity: if flags & rules.translucent != 0 {
                Some(rules.inline_translucent_alpha.unwrap_or(alpha))
            } else if !rules.native_alpha && alpha != 1.0 {
                Some(alpha)
            } else {
                None
            },
            scale: if rules.zero_scale && appearance.scale == 0.0 {
                1.0
            } else {
                appearance.scale
            },
            unsupported: appearance.native.effects | u64::from(flags & !supported),
        }
    }
}
