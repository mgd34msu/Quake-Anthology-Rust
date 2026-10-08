//! Engine-owned values. Game formats and module ABIs convert at their boundaries.

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3(pub [f32; 3]);

impl Vec3 {
    pub fn dot(self, other: Self) -> f32 {
        self.0[0] * other.0[0] + self.0[1] * other.0[1] + self.0[2] * other.0[2]
    }

    pub fn lerp(self, end: Self, fraction: f32) -> Self {
        Self(std::array::from_fn(|axis| {
            self.0[axis] + fraction * (end.0[axis] - self.0[axis])
        }))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
    Z,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Plane {
    pub normal: Vec3,
    pub distance: f32,
    pub axis: Option<Axis>,
}

#[derive(Clone, Copy, Debug)]
pub struct ClipNode {
    pub plane: u32,
    pub children: [i32; 2],
}

impl Plane {
    pub fn signed_distance(self, point: Vec3) -> f32 {
        let coordinate = match self.axis {
            Some(Axis::X) => point.0[0],
            Some(Axis::Y) => point.0[1],
            Some(Axis::Z) => point.0[2],
            None => self.normal.dot(point),
        };
        coordinate - self.distance
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntityId {
    pub slot: u32,
    pub generation: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ItemId(pub u32);
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WeaponId(pub u32);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CvarHandle(pub u32);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SoundId(pub u32);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum PcmChannels {
    Mono = 1,
    Stereo = 2,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pcm {
    pub rate: std::num::NonZeroU32,
    pub channels: PcmChannels,
    pub samples: Vec<i16>,
    pub loop_start: Option<usize>,
}
impl Pcm {
    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels as usize
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EffectId(pub u32);
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NameId(pub u32);
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModuleId(pub u16);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CallbackId(pub u16);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClientId(pub u8);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PowerupId(pub u16);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextId {
    pub slot: u16,
    pub generation: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Think {
    pub at: f64,
    pub callback: CallbackId,
}

#[derive(Clone, Copy, Debug)]
pub enum CallbackCall {
    Think {
        entity: EntityId,
        time: f64,
    },
    Touch {
        entity: EntityId,
        other: EntityId,
    },
    Use {
        entity: EntityId,
        activator: EntityId,
    },
    Blocked {
        entity: EntityId,
        other: EntityId,
    },
    Pain {
        event: DamageEvent,
        taken: i32,
        knockback: i32,
    },
    Die {
        event: DamageEvent,
        taken: i32,
    },
}

impl CallbackCall {
    pub fn entity(self) -> EntityId {
        match self {
            Self::Think { entity, .. }
            | Self::Touch { entity, .. }
            | Self::Use { entity, .. }
            | Self::Blocked { entity, .. } => entity,
            Self::Pain { event, .. } | Self::Die { event, .. } => event.target,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Body {
    pub position: Vec3,
    pub velocity: Vec3,
    pub mins: Vec3,
    pub maxs: Vec3,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Bounds {
    pub mins: Vec3,
    pub maxs: Vec3,
}

impl Bounds {
    pub fn overlaps(self, other: Self) -> bool {
        (0..3).all(|axis| {
            self.mins.0[axis] <= other.maxs.0[axis] && self.maxs.0[axis] >= other.mins.0[axis]
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Entity {
    pub id: EntityId,
    pub body: Body,
    pub next_think: Option<Think>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum PlayerTail {
    #[default]
    None,
    Q1 {
        attack_finished: f64,
        water_jump_until: f64,
        teleport_hold_until: f64,
    },
    Q2 {
        weapon_frame: i32,
        movement_time: u8,
    },
    Q3 {
        weapon_time: i32,
        movement_time: i32,
        command_time: i32,
    },
}

#[derive(Debug, Default)]
pub struct PlayerState {
    pub body: Body,
    pub view_angles: Vec3,
    pub health: i32,
    pub armor: i32,
    pub armor_absorption: f32,
    pub armor_energy_absorption: f32,
    pub armor_type: ItemId,
    pub weapon: WeaponId,
    pub pending_weapon: Option<WeaponId>,
    pub inventory: Box<[i32]>,
    pub item_acquired_at: Box<[f64]>,
    pub powerup_until: Box<[f64]>,
    pub collectibles: u64,
    pub view_offset: Vec3,
    pub punch_angles: Vec3,
    pub flags: u32,
    pub score: i32,
    pub frags: i32,
    pub tail: PlayerTail,
}

impl PlayerState {
    pub fn with_capacity(items: usize, powerups: usize) -> Self {
        Self {
            inventory: vec![0; items].into_boxed_slice(),
            item_acquired_at: vec![0.0; items].into_boxed_slice(),
            powerup_until: vec![0.0; powerups].into_boxed_slice(),
            ..Self::default()
        }
    }

    pub fn reset(&mut self) {
        self.inventory.fill(0);
        self.item_acquired_at.fill(0.0);
        self.powerup_until.fill(0.0);
        let inventory = std::mem::take(&mut self.inventory);
        let item_acquired_at = std::mem::take(&mut self.item_acquired_at);
        let powerup_until = std::mem::take(&mut self.powerup_until);
        *self = Self {
            inventory,
            item_acquired_at,
            powerup_until,
            ..Self::default()
        };
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct UserCmd {
    pub duration_ms: u16,
    pub server_time_ms: i32,
    pub view_angles: Vec3,
    pub movement: [i16; 3],
    pub buttons: u32,
    pub impulse: u8,
    pub light_level: u8,
    pub weapon: Option<WeaponId>,
}

pub mod buttons {
    pub const ATTACK: u32 = 1;
    pub const JUMP: u32 = 2;
    pub const USE: u32 = 4;
    pub const CROUCH: u32 = 8;
    pub const WALK: u32 = 16;
    pub const ANY: u32 = 32;
    pub const TALK: u32 = 64;
    pub const GESTURE: u32 = 128;
    pub const AFFIRMATIVE: u32 = 256;
    pub const NEGATIVE: u32 = 512;
    pub const GETFLAG: u32 = 1024;
    pub const GUARDBASE: u32 = 2048;
    pub const PATROL: u32 = 4096;
    pub const FOLLOWME: u32 = 8192;
}

#[derive(Clone, Copy, Debug)]
pub struct Item {
    pub id: ItemId,
    pub quantity: i32,
}
#[derive(Clone, Copy, Debug)]
pub struct Weapon {
    pub id: WeaponId,
    pub ammo: Option<ItemId>,
}
#[derive(Clone, Copy, Debug)]
pub struct DamageEvent {
    pub target: EntityId,
    pub attacker: Option<EntityId>,
    pub inflictor: Option<EntityId>,
    pub amount: f32,
    pub knockback: i32,
    pub direction: Option<Vec3>,
    pub point: Vec3,
    pub flags: DamageFlags,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct DamageFlags(pub u32);
impl DamageFlags {
    pub const RADIUS: u32 = 1;
    pub const NO_ARMOR: u32 = 2;
    pub const NO_KNOCKBACK: u32 = 4;
    pub const NO_PROTECTION: u32 = 8;
    pub const ENERGY: u32 = 16;
    pub const FALLING: u32 = 32;
    pub fn contains(self, flag: u32) -> bool {
        self.0 & flag != 0
    }
}
#[derive(Clone, Copy, Debug)]
pub struct SoundEvent {
    pub sound: SoundId,
    pub entity: Option<EntityId>,
    pub channel: u16,
    pub position: Vec3,
    pub volume: f32,
    pub attenuation: f32,
    pub action: SoundAction,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SoundAction {
    #[default]
    Play,
    StartLoop,
    Stop,
}
#[derive(Clone, Copy, Debug)]
pub struct EffectEvent {
    pub effect: EffectId,
    pub position: Vec3,
    pub direction: Vec3,
    pub count: u16,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrintKind {
    Console,
    Notify,
    Center,
    Chat,
    Layout,
}
#[derive(Clone, Copy, Debug)]
pub struct PrintEvent {
    pub client: Option<ClientId>,
    pub kind: PrintKind,
    pub text: TextId,
}
#[derive(Clone, Copy, Debug)]
pub struct HudLine {
    pub text: TextId,
    pub started_at: f64,
    pub until: f64,
}
#[derive(Debug, Default)]
pub struct HudState {
    pub health: i32,
    pub armor: i32,
    pub ammo: i32,
    pub weapon: WeaponId,
    pub armor_type: ItemId,
    pub frags: i32,
    pub score: i32,
    pub item_counts: Box<[i32]>,
    pub item_acquired_at: Box<[f64]>,
    pub owned_items: Box<[u64]>,
    pub owned_weapons: Box<[u64]>,
    pub powerup_until: Box<[f64]>,
    pub collectibles: u64,
    pub layout: NameId,
    pub layout_text: Option<TextId>,
    pub centerprint: Option<HudLine>,
    pub notify: [Option<HudLine>; 4],
}

impl HudState {
    pub fn with_capacity(items: usize, powerups: usize, weapons: usize) -> Self {
        Self {
            item_counts: vec![0; items].into_boxed_slice(),
            item_acquired_at: vec![0.0; items].into_boxed_slice(),
            owned_items: vec![0; items.div_ceil(64)].into_boxed_slice(),
            owned_weapons: vec![0; weapons.div_ceil(64)].into_boxed_slice(),
            powerup_until: vec![0.0; powerups].into_boxed_slice(),
            ..Self::default()
        }
    }

    pub fn reset(&mut self) {
        self.item_counts.fill(0);
        self.item_acquired_at.fill(0.0);
        self.owned_items.fill(0);
        self.owned_weapons.fill(0);
        self.powerup_until.fill(0.0);
        *self = Self {
            item_counts: std::mem::take(&mut self.item_counts),
            item_acquired_at: std::mem::take(&mut self.item_acquired_at),
            owned_items: std::mem::take(&mut self.owned_items),
            owned_weapons: std::mem::take(&mut self.owned_weapons),
            powerup_until: std::mem::take(&mut self.powerup_until),
            ..Self::default()
        };
    }
}
