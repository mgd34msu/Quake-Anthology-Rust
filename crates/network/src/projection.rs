//! One field table converts between common players and native record layouts.
//! Native ordinals and effective tuning are supplied by the module/connection.
use crate::{commands::packet::Protocol, states};
use qa_core::{
    math::{AngleShortForm, eighth_to_short, short_to_eighth},
    primitives::{
        AngleOffset, EntityAppearance, EntityId, ModelRef, MovementMagnitude, MovementMode,
        MovementTimer, NativePlayerTail, PlayerDamageSample, PlayerSignals, PlayerState,
        PlayerValueBindings, Q3StatLayout, ResourceNamespace, SoundRef, ValueBinding, ValueWidth,
        ViewWeapon, WeaponReadiness, WeaponState,
    },
};

mod entity;
pub use entity::EntityProjection;
pub use qa_core::primitives::EntityRecord;

pub const MOVEMENT_WORDS: usize = 13;
pub const PM_FLAGS_WORD: usize = 8;
pub const GRAVITY_WORD: usize = 9;

/// These are native values, never common entity, client or registry handles.
#[derive(Default)]
pub struct PlayerContext {
    /// Only this explicitly selected namespace may contribute raw native tails.
    pub namespace: Option<ResourceNamespace>,
    pub client_number: Option<u32>,
    pub ground_number: Option<u32>,
    pub weapon_number: Option<u32>,
    /// Native body presentation supplied independently of the view weapon.
    pub appearance: Option<EntityAppearance>,
    pub gravity: f32,
    pub speed: f32,
    /// Visibility/self/spectator selection belongs to the QW snapshot provider.
    pub player_info_flags: u16,
    pub command_age_ms: u32,
    /// Native entity yaw may differ from the player's command/view angles.
    pub body_yaw: f32,
}

#[derive(Clone, Copy)]
enum Source {
    Position(usize),
    Velocity(usize),
    View(usize),
    ViewOffset(usize),
    Punch(usize),
    DeltaAngle(usize),
    CommandTime,
    Timer,
    Flags,
    Mode,
    Health,
    Armor,
    Score,
    Frags,
    Grounded,
    InWater,
    WeaponTime,
    WeaponFrame,
    WeaponOffset(usize),
    WeaponAngle(usize),
    WeaponRate,
    BodyFrame,
    BodyModel,
    BodySkin,
    BodyEffects,
    Client,
    Ground,
    Weapon,
    WeaponModel,
    LoopSound,
    EventSequence,
    Event(usize),
    EventParameter(usize),
    ExternalEvent,
    ExternalParameter,
    DamageSequence,
    DamageYaw,
    DamagePitch,
    DamageCount,
    Gravity,
    Speed,
    InfoFlags,
    CommandAge,
    IdealPitch,
    BodyYaw,
    /// A native-only value lives in the common bank through a cold binding.
    NativeValue,
}
#[derive(Clone, Copy)]
enum Convert {
    Bits,
    IntegerFloat,
    FloatInteger,
    Eighth,
    ShortAngle,
    UnsignedShortAngle,
    IntegerAngle,
    TimerEighth,
    Milliseconds16,
    ScaledShort(u16),
    ScaledByte {
        scale: u16,
        nonzero: bool,
        implicit: u8,
    },
}
#[derive(Clone, Copy)]
struct Field<S = Source> {
    word: usize,
    source: S,
    convert: Convert,
}
const fn f(word: usize, source: Source, convert: Convert) -> Field {
    Field {
        word,
        source,
        convert,
    }
}
use Convert::*;
use Source::*;
static NQ: &[Field] = &[
    f(0, ViewOffset(2), Bits),
    f(1, IdealPitch, Bits),
    f(2, Punch(0), Bits),
    f(3, Velocity(0), Bits),
    f(4, Punch(1), Bits),
    f(5, Velocity(1), Bits),
    f(6, Punch(2), Bits),
    f(7, Velocity(2), Bits),
    f(8, NativeValue, Bits),
    f(9, WeaponFrame, IntegerFloat),
    f(10, Armor, IntegerFloat),
    f(11, WeaponModel, Bits),
    f(12, Health, IntegerFloat),
    f(13, NativeValue, IntegerFloat),
    f(14, NativeValue, IntegerFloat),
    f(15, NativeValue, IntegerFloat),
    f(16, NativeValue, IntegerFloat),
    f(17, NativeValue, IntegerFloat),
    f(18, Weapon, Bits),
    f(19, Grounded, Bits),
    f(20, InWater, Bits),
];
static QW: &[Field] = &[
    f(0, Position(0), Bits),
    f(1, Position(1), Bits),
    f(2, Position(2), Bits),
    f(3, BodyFrame, IntegerFloat),
    f(4, CommandAge, Bits),
    f(5, Velocity(0), Bits),
    f(6, Velocity(1), Bits),
    f(7, Velocity(2), Bits),
    f(8, BodyModel, IntegerFloat),
    f(9, BodySkin, IntegerFloat),
    f(10, BodyEffects, IntegerFloat),
    f(11, WeaponFrame, IntegerFloat),
    f(12, InfoFlags, Bits),
    f(13, BodyYaw, Bits),
];
const Q2: &[Field] = &[
    f(0, Mode, Bits),
    f(1, Position(0), Eighth),
    f(2, Position(1), Eighth),
    f(3, Position(2), Eighth),
    f(4, Velocity(0), Eighth),
    f(5, Velocity(1), Eighth),
    f(6, Velocity(2), Eighth),
    f(7, Timer, TimerEighth),
    f(PM_FLAGS_WORD, Flags, Bits),
    f(GRAVITY_WORD, Gravity, FloatInteger),
    f(10, DeltaAngle(0), ShortAngle),
    f(11, DeltaAngle(1), ShortAngle),
    f(12, DeltaAngle(2), ShortAngle),
    f(13, ViewOffset(0), Bits),
    f(14, ViewOffset(1), Bits),
    f(15, ViewOffset(2), Bits),
    f(16, View(0), Bits),
    f(17, View(1), Bits),
    f(18, View(2), Bits),
    f(19, Punch(0), Bits),
    f(20, Punch(1), Bits),
    f(21, Punch(2), Bits),
    f(22, WeaponModel, Bits),
    f(23, WeaponFrame, Bits),
    f(24, WeaponOffset(0), Bits),
    f(25, WeaponOffset(1), Bits),
    f(26, WeaponOffset(2), Bits),
    f(27, WeaponAngle(0), Bits),
    f(28, WeaponAngle(1), Bits),
    f(29, WeaponAngle(2), Bits),
    f(37, Health, Bits),
    f(41, Armor, Bits),
    f(50, Frags, Bits),
];
const fn q2_repro_fields() -> [Field; Q2.len() + 2] {
    let mut fields = [f(0, Mode, Bits); Q2.len() + 2];
    let mut i = 0;
    while i < Q2.len() {
        let mut field = Q2[i];
        field.convert = match field.source {
            Position(_) | Velocity(_) | DeltaAngle(_) => Bits,
            ViewOffset(_) => ScaledShort(16),
            Punch(_) => ScaledShort(1024),
            View(_) => ShortAngle,
            WeaponOffset(_) => ScaledShort(512),
            WeaponAngle(_) => ScaledShort(4096),
            Timer => Milliseconds16,
            _ => field.convert,
        };
        if field.word >= 36 {
            field.word += 7;
        }
        fields[i] = field;
        i += 1;
    }
    fields[Q2.len()] = f(42, Client, Bits);
    fields[Q2.len() + 1] = f(40, WeaponRate, Bits);
    fields
}
static Q2_REPRO: &[Field] = &q2_repro_fields();
static Q3: &[Field] = &[
    f(0, CommandTime, Bits),
    f(1, Position(0), Bits),
    f(2, Position(1), Bits),
    f(4, Velocity(0), Bits),
    f(5, Velocity(1), Bits),
    f(6, View(1), Bits),
    f(7, View(0), Bits),
    f(8, WeaponTime, Bits),
    f(9, Position(2), Bits),
    f(10, Velocity(2), Bits),
    f(12, Timer, Bits),
    f(13, EventSequence, Bits),
    f(16, Event(0), Bits),
    f(18, Event(1), Bits),
    f(19, Flags, Bits),
    f(20, Ground, Bits),
    f(23, ExternalEvent, Bits),
    f(24, Gravity, FloatInteger),
    f(25, Speed, FloatInteger),
    f(26, DeltaAngle(1), UnsignedShortAngle),
    f(27, ExternalParameter, Bits),
    f(28, ViewOffset(2), FloatInteger),
    f(29, DamageSequence, Bits),
    f(30, DamageYaw, Bits),
    f(31, DamagePitch, Bits),
    f(32, DamageCount, Bits),
    f(34, Mode, Bits),
    f(35, DeltaAngle(0), UnsignedShortAngle),
    f(36, DeltaAngle(2), UnsignedShortAngle),
    f(38, EventParameter(0), Bits),
    f(39, EventParameter(1), Bits),
    f(40, Client, Bits),
    f(41, Weapon, Bits),
    f(42, View(2), Bits),
    f(47, LoopSound, Bits),
    f(48, Health, Bits),
    f(51, Armor, Bits),
    f(64, Score, Bits),
];

/// A borrowed snapshot's native view target and damage, without another player bank.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayerFeedbackSample {
    pub client_number: u32,
    pub damage: PlayerDamageSample,
    pub health: i32,
}

struct FeedbackFields {
    client: Field,
    health: Field,
    sequence: Field,
    yaw: Field,
    pitch: Field,
    count: Field,
}
impl FeedbackFields {
    fn load(fields: &[Field]) -> Option<Self> {
        Some(Self {
            client: *fields.iter().find(|field| matches!(field.source, Client))?,
            health: *fields.iter().find(|field| matches!(field.source, Health))?,
            sequence: *fields
                .iter()
                .find(|field| matches!(field.source, DamageSequence))?,
            yaw: *fields
                .iter()
                .find(|field| matches!(field.source, DamageYaw))?,
            pitch: *fields
                .iter()
                .find(|field| matches!(field.source, DamagePitch))?,
            count: *fields
                .iter()
                .find(|field| matches!(field.source, DamageCount))?,
        })
    }
    fn read(&self, words: &[u32], client_limit: u32) -> Option<PlayerFeedbackSample> {
        let read = |field: Field| field.convert.import(words[field.word]);
        let client_number = read(self.client);
        (client_number < client_limit).then(|| PlayerFeedbackSample {
            client_number,
            health: read(self.health) as i32,
            damage: PlayerDamageSample {
                sequence: read(self.sequence),
                yaw: read(self.yaw),
                pitch: read(self.pitch),
                count: read(self.count),
            },
        })
    }
}

/// One walker; the native format selects data at load, not another player store.
pub struct PlayerProjection {
    protocol: Protocol,
    fields: Box<[Field]>,
    words: usize,
    modes: [u32; 7],
    received_modes: &'static [MovementMode],
    exact_modes: bool,
    flags: [u32; 8],
    flag_mask: u32,
    prediction_masks: [u32; 2],
    stats_only: bool,
    movement_only: bool,
    layout_valid: bool,
    client_limit: u32,
    feedback: Option<FeedbackFields>,
    bindings: Box<[Field<ValueBinding>]>,
    write_bindings: Box<[Field<ValueBinding>]>,
    pub dropped_bindings: usize,
}
impl PlayerProjection {
    /// Native pmove_state_t carries movement only; inventory and presentation
    /// remain in the module's player_state_t and are imported separately.
    pub fn load_movement(protocol: Protocol) -> Self {
        let mut projection = Self::load_fields(protocol, &[], false, Q3StatLayout::BASE);
        projection.movement_only = true;
        projection.fields = projection
            .fields
            .iter()
            .copied()
            .filter(|field| {
                field.word < MOVEMENT_WORDS && !matches!(field.source, Source::WeaponTime)
            })
            .collect();
        projection.feedback = None;
        projection
    }
    pub fn load(protocol: Protocol, bindings: &[(usize, ValueBinding)]) -> Self {
        Self::load_fields(protocol, bindings, false, Q3StatLayout::BASE)
    }
    /// Native array ordinals enter their explicitly reserved common bank rows.
    /// Shared hot fields remain authoritative when writing a snapshot.
    pub fn load_values(
        protocol: Protocol,
        values: PlayerValueBindings<'_>,
        layout: Q3StatLayout,
    ) -> Self {
        Self::load_value_fields(protocol, values, layout, false)
    }
    /// Native module stats enter their ordinal bank rows without importing
    /// movement/view columns, whose capability roles remain independent.
    pub fn load_stat_values(
        protocol: Protocol,
        values: PlayerValueBindings<'_>,
        layout: Q3StatLayout,
    ) -> Self {
        Self::load_value_fields(protocol, values, layout, true)
    }
    /// Q3's public playerState_t has signed32 angle offsets and exact native
    /// movement modes. Its wire adapter retains its own narrower conversions.
    pub fn load_native_q3(values: PlayerValueBindings<'_>, layout: Q3StatLayout) -> Self {
        let mut projection = Self::load_value_fields(Protocol::Quake3_68, values, layout, false);
        for field in &mut projection.fields {
            if matches!(field.source, DeltaAngle(_)) {
                field.convert = IntegerAngle;
            }
        }
        projection.exact_modes = true;
        projection
    }
    fn load_value_fields(
        protocol: Protocol,
        values: PlayerValueBindings<'_>,
        layout: Q3StatLayout,
        stats_only: bool,
    ) -> Self {
        let (_, spans) = states::player_value_layout(protocol);
        let bindings: Vec<_> = values
            .arrays
            .into_iter()
            .enumerate()
            .flat_map(|(array, bindings)| {
                bindings
                    .iter()
                    .copied()
                    .enumerate()
                    .filter_map(move |(index, binding)| {
                        let word = if array == 0 {
                            states::player_stat_word(protocol, index)
                        } else {
                            spans[array].range().nth(index)
                        }?;
                        Some((word, binding))
                    })
            })
            .chain(
                values
                    .items
                    .filter(|_| protocol == Protocol::NetQuake15)
                    .map(|binding| (8, binding)),
            )
            .collect();
        let mut projection = Self::load_fields(protocol, &bindings, stats_only, layout);
        projection.dropped_bindings += values.iter().count() - bindings.len();
        for field in &mut projection.bindings {
            if protocol == Protocol::NetQuake15 {
                field.convert = NQ
                    .iter()
                    .find(|native| native.word == field.word)
                    .map_or(Bits, |native| native.convert);
            }
        }
        projection.write_bindings = projection.bindings.clone();
        if !stats_only {
            projection.write_bindings = projection
                .bindings
                .iter()
                .copied()
                .filter(|binding| {
                    !projection.fields.iter().any(|field| {
                        field.word == binding.word
                            && !matches!(field.source, NativeValue)
                            && !(protocol == Protocol::NetQuake15 && matches!(field.source, Weapon))
                    })
                })
                .collect();
        }
        projection
    }
    fn load_fields(
        protocol: Protocol,
        bindings: &[(usize, ValueBinding)],
        stats_only: bool,
        layout: Q3StatLayout,
    ) -> Self {
        use MovementMode as M;
        let (fields, words, modes, received_modes, flags) = match protocol {
            Protocol::NetQuake15 => (NQ, states::NQ_PLAYER_WORDS, [0; 7], &[][..], [0; 8]),
            Protocol::QuakeWorld28 => (QW, states::QW_PLAYER_WORDS, [0; 7], &[][..], [0; 8]),
            Protocol::Quake2_34 => (
                Q2,
                states::Q2_PLAYER_WORDS,
                [0, 1, 1, 1, 2, 3, 4],
                &[M::Walk, M::Spectator, M::Dead, M::Gib, M::Frozen][..],
                [1, 2, 4, 8, 16, 32, 0, 0],
            ),
            Protocol::Quake2Repro1038 => (
                Q2_REPRO,
                states::Q2_REPRO_PLAYER_WORDS,
                [0, 1, 2, 3, 4, 5, 6],
                &[
                    M::Walk,
                    M::Fly,
                    M::Noclip,
                    M::Spectator,
                    M::Dead,
                    M::Gib,
                    M::Frozen,
                ][..],
                [1, 2, 4, 8, 16, 32, 0, 1024],
            ),
            Protocol::Quake3_68 => (
                Q3,
                states::PLAYER_WORDS,
                [0, 1, 1, 2, 3, 3, 4],
                &[
                    M::Walk,
                    M::Noclip,
                    M::Spectator,
                    M::Dead,
                    M::Frozen,
                    M::Frozen,
                    M::Frozen,
                ][..],
                [1, 2, 0, 256, 32, 0, 64, 0],
            ),
        };
        let admitted: Box<[_]> = bindings
            .iter()
            .copied()
            .filter(|(word, _)| *word < words)
            .map(|(word, source)| Field {
                word,
                source,
                convert: Bits,
            })
            .collect();
        let layout_valid = protocol != Protocol::Quake3_68
            || layout.fits(states::player_value_layout(protocol).1[0].count as usize);
        let stat_first = states::player_value_layout(protocol).1[0].first as usize;
        let fields: Box<[_]> = fields
            .iter()
            .copied()
            .filter(|field| !stats_only || matches!(field.source, Health | Armor | Score | Frags))
            .map(|mut field| {
                if protocol == Protocol::Quake3_68 && layout_valid {
                    match field.source {
                        Health => field.word = stat_first + usize::from(layout.health),
                        Armor => field.word = stat_first + usize::from(layout.armor),
                        _ => {}
                    }
                }
                field
            })
            .collect();
        let feedback = if layout_valid {
            FeedbackFields::load(&fields)
        } else {
            None
        };
        Self {
            protocol,
            fields,
            words,
            modes,
            received_modes,
            exact_modes: false,
            flags,
            flag_mask: flags.into_iter().fold(0, |mask, flag| mask | flag),
            prediction_masks: match protocol {
                Protocol::Quake2_34 => [64, 0],
                Protocol::Quake2Repro1038 => [64, 256],
                Protocol::Quake3_68 => [4096; 2],
                _ => [0; 2],
            },
            stats_only,
            movement_only: false,
            layout_valid,
            client_limit: if protocol == Protocol::Quake2Repro1038 {
                256
            } else {
                64
            },
            feedback,
            dropped_bindings: bindings.len() - admitted.len(),
            write_bindings: admitted.clone(),
            bindings: admitted,
        }
    }
    pub fn words(&self) -> usize {
        self.words
    }
    pub fn movement_flag_mask(&self) -> u32 {
        self.flag_mask
    }

    /// Read the load-selected view/damage fields from a retained native frame.
    /// Native and wire conversion share the same descriptors as full import.
    pub fn damage_feedback(&self, words: &[u32]) -> Option<PlayerFeedbackSample> {
        self.feedback
            .as_ref()?
            .read(words.get(..self.words)?, self.client_limit)
    }

    /// The native view target is independent of this connection's callback id.
    pub fn client_number(&self, words: &[u32]) -> Option<u32> {
        let words = words.get(..self.words)?;
        self.fields.iter().find_map(|field| {
            if matches!(field.source, Client) {
                let number = field.convert.import(words[field.word]);
                (number < self.client_limit).then_some(number)
            } else {
                None
            }
        })
    }

    /// Presentation reads the same native field table without importing stats
    /// into a second player bank or allocating temporary value capacity.
    pub fn view_weapon(&self, words: &[u32], namespace: ResourceNamespace) -> Option<ViewWeapon> {
        let words = words.get(..self.words)?;
        let mut weapon = ViewWeapon::default();
        let mut present = false;
        for field in &self.fields {
            present |= self.apply_weapon(
                field.source,
                field.convert.import(words[field.word]),
                &mut weapon,
                namespace,
            );
        }
        present.then_some(weapon)
    }

    /// Native Q2 player interpolation rejects a teleport or a changed POV.
    /// Raw flag semantics remain at this protocol boundary.
    pub fn can_interpolate_player(&self, from: &[u32], to: &[u32]) -> bool {
        let (Some(from), Some(to)) = (from.get(..self.words), to.get(..self.words)) else {
            return false;
        };
        if !matches!(
            self.protocol,
            Protocol::Quake2_34 | Protocol::Quake2Repro1038
        ) {
            return true;
        }
        for field in &self.fields {
            match field.source {
                Position(_) => {
                    let old = f32::from_bits(field.convert.import(from[field.word]));
                    let new = f32::from_bits(field.convert.import(to[field.word]));
                    let distance = (new - old).abs();
                    if !distance.is_finite() || distance > 256.0 {
                        return false;
                    }
                }
                Client if from[field.word] != to[field.word] => return false,
                _ => {}
            }
        }
        self.protocol != Protocol::Quake2Repro1038 || (from[35] ^ to[35]) & (1 << 4) == 0
    }

    fn apply_weapon(
        &self,
        source: Source,
        value: u32,
        weapon: &mut ViewWeapon,
        namespace: ResourceNamespace,
    ) -> bool {
        match source {
            WeaponFrame => weapon.frame = value as i32,
            WeaponOffset(i) => weapon.offset.0[i] = f32::from_bits(value),
            WeaponAngle(i) => weapon.angles.0[i] = f32::from_bits(value),
            WeaponRate => weapon.rate = value as i32,
            WeaponModel => {
                let model = if self.protocol == Protocol::Quake2Repro1038 {
                    weapon.skin = (value >> 13) as i32;
                    value & 8191
                } else {
                    value
                };
                weapon.model = ModelRef::native(namespace, model);
            }
            _ => return false,
        }
        true
    }

    /// The module can supply fields absent from the common hot columns through
    /// load-resolved numeric bindings. Last binding wins in registration order.
    pub fn reduce(
        &self,
        player: &PlayerState,
        context: &PlayerContext,
        out: &mut [u32],
        mut resolve_model: impl FnMut(ModelRef) -> Option<u32>,
        mut resolve_sound: impl FnMut(SoundRef) -> Option<u32>,
    ) -> bool {
        if !self.layout_valid {
            return false;
        }
        let Some(out) = out.get_mut(..self.words) else {
            return false;
        };
        out.fill(0);
        let native = player.native_for(self.protocol.rule_set(), context.namespace);
        for field in &self.fields {
            out[field.word] = if let DeltaAngle(axis) = field.source {
                field
                    .convert
                    .export_angle(player.movement.delta_angles.0[axis])
            } else {
                let word = self.value(
                    field.source,
                    player,
                    context,
                    &mut resolve_model,
                    &mut resolve_sound,
                );
                native
                    .and_then(|tail| match field.source {
                        Flags => Some((tail.flags & !self.flag_mask) | word),
                        Gravity => tail.gravity.map(|value| value as u32),
                        Speed => tail.speed.map(|value| value as u32),
                        Client => tail.client_number,
                        Ground
                            if player.movement.grounded
                                && context.ground_number.is_none()
                                && player.movement.ground.is_none() =>
                        {
                            tail.ground_number
                        }
                        Weapon => tail.weapon_number,
                        _ => tail
                            .signals
                            .and_then(|signals| field.source.signal_value(signals)),
                    })
                    .unwrap_or_else(|| field.convert.export(word))
            };
        }
        for field in &self.write_bindings {
            if self.protocol == Protocol::NetQuake15
                && (context.weapon_number.is_some()
                    || native.is_some_and(|tail| tail.weapon_number.is_some()))
                && NQ
                    .iter()
                    .any(|native| native.word == field.word && matches!(native.source, Weapon))
            {
                continue;
            }
            if let Some(value) = field.source.export(&player.values) {
                let value = if field.source.width == ValueWidth::Signed16 {
                    value as i16 as i32 as u32
                } else {
                    value
                };
                out[field.word] = field.convert.export(value);
            }
        }
        true
    }

    /// Apply words returned by the existing native decoder, not the writer's
    /// input words (NQ's QC float stats decode to native signed integers).
    /// Roles, untransmitted fields and owned arenas remain unchanged. Ordinals
    /// stay in boundary context; the caller resolves ground in its namespace.
    /// Effective native tuning enters the same shared capability columns used by
    /// SERVER and accepted CLIENT packets. Raw words retain their source tail.
    pub fn apply(
        &self,
        words: &[u32],
        player: &mut PlayerState,
        context: &mut PlayerContext,
        namespace: ResourceNamespace,
        resolve_ground: impl FnMut(u32) -> Option<EntityId>,
    ) -> bool {
        self.apply_fields(words, player, context, namespace, resolve_ground, false)
    }
    /// Engine-owned movement retains its body/view state while a native module
    /// publishes numeric arrays and weapon readiness through this same walker.
    pub fn apply_stats(
        &self,
        words: &[u32],
        player: &mut PlayerState,
        namespace: ResourceNamespace,
    ) -> bool {
        self.apply_fields(
            words,
            player,
            &mut PlayerContext::default(),
            namespace,
            |_| None,
            true,
        )
    }
    fn apply_fields(
        &self,
        words: &[u32],
        player: &mut PlayerState,
        context: &mut PlayerContext,
        namespace: ResourceNamespace,
        mut resolve_ground: impl FnMut(u32) -> Option<EntityId>,
        stats_only: bool,
    ) -> bool {
        let stats_only = stats_only || self.stats_only;
        if !self.layout_valid {
            return false;
        }
        let Some(words) = words.get(..self.words) else {
            return false;
        };
        if self
            .bindings
            .iter()
            .any(|field| field.source.id.0 as usize >= player.values.capacity())
        {
            return false;
        }
        let mut native = NativePlayerTail::new(self.protocol.rule_set(), namespace);
        if !stats_only {
            context.namespace = Some(namespace);
            player.prediction = Default::default();
        }
        for field in &self.fields {
            if stats_only && !matches!(field.source, Health | Armor | Score | Frags | WeaponTime) {
                continue;
            }
            let word = words[field.word];
            let value = if matches!(field.source, DeltaAngle(_)) {
                word
            } else {
                field.convert.import(word)
            };
            match field.source {
                Position(i) => player.body.position.0[i] = f32::from_bits(value),
                Velocity(i) => player.body.velocity.0[i] = f32::from_bits(value),
                View(i) => player.view_angles.0[i] = f32::from_bits(value),
                ViewOffset(i) => player.view_offset.0[i] = f32::from_bits(value),
                Punch(i) => player.punch_angles.0[i] = f32::from_bits(value),
                DeltaAngle(i) => {
                    player.movement.delta_angles.0[i] = field.convert.import_angle(word)
                }
                CommandTime => player.movement.command_time_ms = value as i32,
                Timer => player.movement.remaining_ms = value,
                Flags => {
                    native.flags = word;
                    player.prediction.position = word & self.prediction_masks[0] == 0;
                    player.prediction.angles = word & self.prediction_masks[1] == 0;
                    if self.flags[0] != 0 {
                        player.movement.ducked = value & self.flags[0] != 0;
                    }
                    if self.flags[1] != 0 {
                        player.movement.jump_held = value & self.flags[1] != 0;
                    }
                    if self.flags[2] != 0 {
                        player.movement.grounded = value & self.flags[2] != 0;
                        // Q2 transmits contact presence, not a ground ordinal.
                        player.movement.ground = None;
                    }
                    for (timer, flag) in [
                        MovementTimer::WATER_JUMP,
                        MovementTimer::LAND,
                        MovementTimer::TELEPORT,
                        MovementTimer::KNOCKBACK,
                        MovementTimer::TRICK,
                    ]
                    .into_iter()
                    .zip(&self.flags[3..])
                    {
                        if *flag != 0 {
                            player.movement.timer.0 = (player.movement.timer.0 & !timer.0)
                                | if value & flag != 0 { timer.0 } else { 0 };
                        }
                    }
                }
                Mode => {
                    // Preserve a caller's Fly/Noclip/Gib choice when the wire
                    // cannot distinguish it from another common mode.
                    if self.exact_modes || self.native_mode(player.movement.mode) != value {
                        player.movement.mode = self
                            .received_modes
                            .get(value as usize)
                            .copied()
                            .unwrap_or(MovementMode::Frozen);
                    }
                }
                Health => player.health = value as i32,
                Armor => player.armor = value as i32,
                Score => player.score = value as i32,
                Frags => player.frags = value as i32,
                Grounded => {
                    player.movement.grounded = value != 0;
                    player.movement.ground = None;
                }
                InWater => {
                    // One native bit cannot supply the exact three-level probe.
                    player.movement.water_level = if value != 0 {
                        player.movement.water_level.max(2)
                    } else {
                        player.movement.water_level.min(1)
                    };
                }
                WeaponTime => {
                    player.weapon_state = Some(WeaponState {
                        source: self.protocol.rule_set(),
                        namespace,
                        readiness: WeaponReadiness::RemainingMilliseconds(value as i32),
                    });
                }
                WeaponFrame | WeaponOffset(_) | WeaponAngle(_) | WeaponRate | WeaponModel => {
                    self.apply_weapon(field.source, value, &mut player.view_weapon, namespace);
                }
                LoopSound => player.loop_sound = SoundRef::native(namespace, value),
                EventSequence | Event(_) | EventParameter(_) | ExternalEvent
                | ExternalParameter | DamageSequence | DamageYaw | DamagePitch | DamageCount => {
                    let signals = native.signals.get_or_insert_with(PlayerSignals::default);
                    field.source.apply_signal(value, signals);
                }
                BodyFrame | BodyModel | BodySkin | BodyEffects => {
                    let body = context.appearance.get_or_insert_with(|| EntityAppearance {
                        source: self.protocol.rule_set(),
                        ..Default::default()
                    });
                    match field.source {
                        BodyFrame => body.frame = value as i32,
                        BodyModel => body.models[0] = ModelRef::native(namespace, value),
                        BodySkin => body.skin = value as i32,
                        BodyEffects => body.native.effects = u64::from(value),
                        _ => {}
                    }
                }
                Client => {
                    native.client_number = Some(word);
                    context.client_number = (value < self.client_limit).then_some(value);
                }
                Ground => {
                    native.ground_number = Some(word);
                    context.ground_number = (value < 1023).then_some(value);
                    player.movement.grounded = context.ground_number.is_some();
                    player.movement.ground = context.ground_number.and_then(&mut resolve_ground);
                }
                Weapon => {
                    native.weapon_number = Some(word);
                    context.weapon_number = Some(value);
                }
                Gravity => {
                    native.gravity = Some(word as i32);
                    context.gravity = f32::from_bits(value);
                    let tuning = &mut player.movement.tuning;
                    tuning.gravity = Some(MovementMagnitude::native(
                        context.gravity,
                        tuning.gravity_multiplier,
                    ));
                }
                Speed => {
                    native.speed = Some(word as i32);
                    context.speed = f32::from_bits(value);
                    let tuning = &mut player.movement.tuning;
                    tuning.max_speed = Some(MovementMagnitude::native(
                        context.speed,
                        tuning.speed_multiplier,
                    ));
                }
                InfoFlags => context.player_info_flags = value as u16,
                CommandAge => context.command_age_ms = value,
                IdealPitch => player.ideal_pitch = f32::from_bits(value),
                // QW body yaw is an input to command sanitization, not a field
                // received in playerinfo. Its decoded placeholder is unused.
                BodyYaw | NativeValue => {}
            }
        }
        if !stats_only && !self.movement_only {
            player.native = Some(native);
        }
        for field in &self.bindings {
            field
                .source
                .import(&mut player.values, field.convert.import(words[field.word]));
        }
        true
    }

    fn native_mode(&self, mode: MovementMode) -> u32 {
        self.modes[match mode {
            MovementMode::Walk => 0,
            MovementMode::Fly => 1,
            MovementMode::Noclip => 2,
            MovementMode::Spectator => 3,
            MovementMode::Dead => 4,
            MovementMode::Gib => 5,
            MovementMode::Frozen => 6,
        }]
    }

    fn value(
        &self,
        source: Source,
        p: &PlayerState,
        c: &PlayerContext,
        resolve_model: &mut impl FnMut(ModelRef) -> Option<u32>,
        resolve_sound: &mut impl FnMut(SoundRef) -> Option<u32>,
    ) -> u32 {
        match source {
            Position(i) => p.body.position.0[i].to_bits(),
            Velocity(i) => p.body.velocity.0[i].to_bits(),
            View(i) => p.view_angles.0[i].to_bits(),
            ViewOffset(i) => p.view_offset.0[i].to_bits(),
            Punch(i) => p.punch_angles.0[i].to_bits(),
            DeltaAngle(i) => p.movement.delta_angles.0[i].degrees().to_bits(),
            CommandTime => p.movement.command_time_ms as u32,
            Timer => p.movement.remaining_ms,
            Flags => {
                let enabled = [
                    p.movement.ducked,
                    p.movement.jump_held,
                    p.movement.grounded,
                    p.movement.timer.contains(MovementTimer::WATER_JUMP),
                    p.movement.timer.contains(MovementTimer::LAND),
                    p.movement.timer.contains(MovementTimer::TELEPORT),
                    p.movement.timer.contains(MovementTimer::KNOCKBACK),
                    p.movement.timer.contains(MovementTimer::TRICK),
                ];
                enabled
                    .into_iter()
                    .zip(self.flags)
                    .fold(0, |bits, (on, flag)| bits | if on { flag } else { 0 })
            }
            Mode => self.native_mode(p.movement.mode),
            Health => p.health as u32,
            Armor => p.armor as u32,
            Score => p.score as u32,
            Frags => p.frags as u32,
            Grounded => u32::from(p.movement.grounded),
            InWater => u32::from(p.movement.water_level >= 2),
            WeaponTime => match p.weapon_state.map(|state| state.readiness) {
                Some(WeaponReadiness::RemainingMilliseconds(time)) => time as u32,
                _ => 0,
            },
            WeaponFrame => p.view_weapon.frame as u32,
            WeaponOffset(i) => p.view_weapon.offset.0[i].to_bits(),
            WeaponAngle(i) => p.view_weapon.angles.0[i].to_bits(),
            BodyFrame => c.appearance.map_or(0, |a| a.frame as u32),
            BodyModel => c
                .appearance
                .and_then(|a| a.models[0])
                .and_then(resolve_model)
                .filter(|&n| n <= 255)
                .unwrap_or(0),
            BodySkin => c.appearance.map_or(0, |a| a.skin as u32),
            BodyEffects => c
                .appearance
                .filter(|a| {
                    matches!(
                        a.source,
                        qa_core::primitives::RuleSetId::Quake
                            | qa_core::primitives::RuleSetId::QuakeWorld
                    )
                })
                .map_or(0, |a| a.native.effects as u32 & 255),
            WeaponRate => {
                if p.view_weapon.rate == 10 {
                    0
                } else {
                    p.view_weapon.rate as u32
                }
            }
            Client => c
                .client_number
                .filter(|&n| n < self.client_limit)
                .unwrap_or(0),
            Ground => c.ground_number.filter(|&n| n < 1024).unwrap_or(1023),
            Weapon => c.weapon_number.unwrap_or(0),
            WeaponModel => {
                let limit = if self.protocol == Protocol::Quake2Repro1038 {
                    8191
                } else {
                    255
                };
                let model = p
                    .view_weapon
                    .model
                    .and_then(resolve_model)
                    .filter(|&n| n <= limit)
                    .unwrap_or(0);
                if self.protocol == Protocol::Quake2Repro1038 {
                    model | ((p.view_weapon.skin as u32 & 7) << 13)
                } else {
                    model
                }
            }
            LoopSound => p.loop_sound.and_then(resolve_sound).unwrap_or(0),
            EventSequence | Event(_) | EventParameter(_) | ExternalEvent | ExternalParameter
            | DamageSequence | DamageYaw | DamagePitch | DamageCount => 0,
            Gravity => c.gravity.to_bits(),
            Speed => c.speed.to_bits(),
            InfoFlags => u32::from(c.player_info_flags),
            CommandAge => c.command_age_ms.min(255),
            IdealPitch => p.ideal_pitch.to_bits(),
            BodyYaw => c.body_yaw.to_bits(),
            NativeValue => 0,
        }
    }
}

impl Source {
    fn signal_value(self, signals: PlayerSignals) -> Option<u32> {
        Some(match self {
            EventSequence => signals.sequence,
            Event(i) => signals.events[i].code,
            EventParameter(i) => signals.events[i].parameter,
            ExternalEvent => signals.external.code,
            ExternalParameter => signals.external.parameter,
            DamageSequence => signals.damage.sequence,
            DamageYaw => signals.damage.yaw,
            DamagePitch => signals.damage.pitch,
            DamageCount => signals.damage.count,
            _ => return None,
        })
    }

    fn apply_signal(self, value: u32, signals: &mut PlayerSignals) {
        match self {
            EventSequence => signals.sequence = value,
            Event(i) => signals.events[i].code = value,
            EventParameter(i) => signals.events[i].parameter = value,
            ExternalEvent => signals.external.code = value,
            ExternalParameter => signals.external.parameter = value,
            DamageSequence => signals.damage.sequence = value,
            DamageYaw => signals.damage.yaw = value,
            DamagePitch => signals.damage.pitch = value,
            DamageCount => signals.damage.count = value,
            _ => unreachable!("the signal branch only contains signal sources"),
        }
    }
}

impl Convert {
    fn export(self, word: u32) -> u32 {
        match self {
            Bits => word,
            IntegerFloat => (word as i32 as f32).to_bits(),
            FloatInteger => f32::from_bits(word) as i32 as u32,
            Eighth => eighth_to_short(f32::from_bits(word)) as i32 as u32,
            ShortAngle | UnsignedShortAngle | IntegerAngle => {
                self.export_angle(AngleOffset::Degrees(f32::from_bits(word)))
            }
            TimerEighth => (word / 8).min(255),
            Milliseconds16 => word.min(u32::from(u16::MAX)),
            ScaledShort(scale) => {
                let scaled = f32::from_bits(word) * f32::from(scale);
                (scaled.clamp(f32::from(i16::MIN), f32::from(i16::MAX)) as i32) as u32
            }
            ScaledByte {
                scale,
                nonzero,
                implicit,
            } => {
                let value = f32::from_bits(word);
                let mut packed = (value * f32::from(scale)).clamp(0., 255.) as u32;
                if nonzero && value != 0.0 {
                    packed = packed.max(1);
                }
                if implicit != 0 && packed == u32::from(implicit) {
                    0
                } else {
                    packed
                }
            }
        }
    }
    fn import(self, word: u32) -> u32 {
        match self {
            Bits | IntegerFloat => word,
            FloatInteger => (word as i32 as f32).to_bits(),
            Eighth => short_to_eighth(word as i16).to_bits(),
            ShortAngle | UnsignedShortAngle | IntegerAngle => {
                self.import_angle(word).degrees().to_bits()
            }
            TimerEighth => u32::from(word as u8) * 8,
            Milliseconds16 => u32::from(word as u16),
            ScaledShort(scale) => (f32::from(word as i16) / f32::from(scale)).to_bits(),
            ScaledByte { scale, .. } => (word as u8 as f32 / f32::from(scale)).to_bits(),
        }
    }
    fn import_angle(self, word: u32) -> AngleOffset {
        match self {
            ShortAngle => AngleOffset::Units(word as i16 as i32),
            UnsignedShortAngle => AngleOffset::Units(word as u16 as i32),
            IntegerAngle => AngleOffset::Units(word as i32),
            _ => AngleOffset::Degrees(f32::from_bits(self.import(word))),
        }
    }
    fn export_angle(self, angle: AngleOffset) -> u32 {
        match self {
            ShortAngle => angle.units(AngleShortForm::MultiplyDivide) as i16 as i32 as u32,
            UnsignedShortAngle => angle.units(AngleShortForm::MultiplyDivide) as u16 as u32,
            IntegerAngle => angle.units(AngleShortForm::Factored) as u32,
            _ => self.export(angle.degrees().to_bits()),
        }
    }
}
