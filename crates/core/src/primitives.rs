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
pub struct EffectId(pub u32);
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NameId(pub u32);
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModuleId(pub u16);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CallbackId(pub u16);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Think {
    pub at: f64,
    pub callback: CallbackId,
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

#[derive(Clone, Copy, Debug, Default)]
pub struct PlayerState {
    pub body: Body,
    pub view_angles: Vec3,
    pub health: i32,
    pub armor: i32,
    pub weapon: WeaponId,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct UserCmd {
    pub duration_ms: u16,
    pub view_angles: Vec3,
    pub movement: [i16; 3],
    pub buttons: u32,
    pub impulse: u8,
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
    pub amount: i32,
    pub direction: Vec3,
}
#[derive(Clone, Copy, Debug)]
pub struct SoundEvent {
    pub sound: SoundId,
    pub entity: Option<EntityId>,
    pub position: Vec3,
    pub volume: f32,
}
#[derive(Clone, Copy, Debug)]
pub struct EffectEvent {
    pub effect: EffectId,
    pub position: Vec3,
    pub direction: Vec3,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct HudState {
    pub health: i32,
    pub armor: i32,
    pub ammo: i32,
    pub weapon: WeaponId,
}
