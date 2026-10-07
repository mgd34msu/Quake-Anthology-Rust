//! Engine-owned values. Game formats and module ABIs convert at their boundaries.

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3(pub [f32; 3]);

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

#[derive(Clone, Copy, Debug, Default)]
pub struct Body {
    pub position: Vec3,
    pub velocity: Vec3,
    pub mins: Vec3,
    pub maxs: Vec3,
}

#[derive(Clone, Copy, Debug)]
pub struct Entity {
    pub id: EntityId,
    pub body: Body,
    pub next_think: Option<f64>,
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
