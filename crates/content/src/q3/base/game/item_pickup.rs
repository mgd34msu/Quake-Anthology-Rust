//! Quake III base/game: item pickup.
//!
//! Donor provenance: `src/content/q3/base/game/item-pickup.ts`.

use qa_core::math::{dot3, length3, normalize3, sub3, Vec3};

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::mirrors_game_items::*;

// ---------------------------------------------------------------------------
// item-pickup.ts: Pickup_* functions
// ---------------------------------------------------------------------------

/// Ammo respawn seconds (`RESPAWN_AMMO`).
pub const RESPAWN_AMMO: i32 = 40;

pub(crate) const RESPAWN_ARMOR: i32 = 25;

pub(crate) const RESPAWN_HEALTH: i32 = 35;

pub(crate) const RESPAWN_HOLDABLE: i32 = 60;

pub(crate) const RESPAWN_MEGAHEALTH: i32 = 35;

pub(crate) const RESPAWN_POWERUP: i32 = 120;

pub(crate) const EF_KAMIKAZE: i32 = 0x200;

pub(crate) const PLAYEREVENT_DENIED_REWARD: i32 = 0x0001;

/// Weapon pickup context (`WeaponPickupContext`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeaponPickupContext {
    /// Game type.
    pub game_type: i32,
    /// Weapon respawn seconds.
    pub weapon_respawn_seconds: i32,
    /// Team weapon respawn seconds.
    pub team_weapon_respawn_seconds: i32,
}

/// Powerup sight trace (`PowerupSightTrace`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PowerupSightTrace {
    /// Fraction travelled.
    pub fraction: f32,
}

/// Powerup pickup context (`PowerupPickupContext`).
pub struct PowerupPickupContext<'a> {
    /// Current time.
    pub time: i32,
    /// Game type.
    pub game_type: i32,
    /// Solid line-of-sight trace.
    pub trace_solid_line: &'a mut dyn FnMut(Vec3, Vec3) -> PowerupSightTrace,
}

/// Item pickup context (`ItemPickupContext`).
pub struct ItemPickupContext<'a> {
    /// Game type.
    pub game_type: i32,
    /// Weapon respawn seconds.
    pub weapon_respawn_seconds: i32,
    /// Team weapon respawn seconds.
    pub team_weapon_respawn_seconds: i32,
    /// Current time.
    pub time: i32,
    /// Solid line-of-sight trace.
    pub trace_solid_line: &'a mut dyn FnMut(Vec3, Vec3) -> PowerupSightTrace,
    /// Handicap string for a client number.
    pub handicap_for_client: &'a mut dyn FnMut(i32) -> String,
}

impl<'a> ItemPickupContext<'a> {
    fn weapon_context(&self) -> WeaponPickupContext {
        WeaponPickupContext {
            game_type: self.game_type,
            weapon_respawn_seconds: self.weapon_respawn_seconds,
            team_weapon_respawn_seconds: self.team_weapon_respawn_seconds,
        }
    }
}

pub(crate) struct PickupInput {
    item: ItemDefinition,
    item_index: usize,
    product: Product,
}

pub(crate) fn pickup_input(
    pool: &EntityPool,
    items: &dyn ItemTable,
    item_slot: Slot,
    other_slot: Slot,
) -> Q3GameItemsResult<PickupInput> {
    let other = pool
        .get(other_slot)
        .ok_or_else(|| invalid("item pickup requires a client entity"))?;
    if other.client.is_none() {
        return Err(invalid("item pickup requires a client entity"));
    }
    let product = other.client.as_ref().expect("client checked").ps.product;
    let holder = pool
        .get(item_slot)
        .ok_or_else(|| invalid("item pickup requires an item definition"))?;
    let item = holder
        .item
        .clone()
        .ok_or_else(|| invalid("item pickup requires an item definition"))?;
    let item_index = items
        .index_of(product, &item)
        .ok_or_else(|| invalid(format!("item does not belong to {}", product.as_str())))?;
    Ok(PickupInput {
        item,
        item_index,
        product,
    })
}

pub(crate) fn has_guard(pool: &EntityPool, items: &dyn ItemTable, slot: Slot) -> Q3GameItemsResult<bool> {
    let entity = pool
        .get(slot)
        .ok_or_else(|| invalid("guard check requires a client entity"))?;
    let client = entity
        .client
        .as_ref()
        .ok_or_else(|| invalid("guard check requires a client entity"))?;
    if client.ps.product == Product::Baseq3 {
        return Ok(false);
    }
    let index = client.ps.stats.get(MissionpackStatIndex::PERSISTANT_POWERUP)? as usize;
    Ok(items.item_at(Product::Missionpack, index)?.powerup_tag()? == Powerup::Guard)
}

pub(crate) fn add_ammo_to(client: &mut GameClient, weapon: Weapon, count: i32) -> Q3GameItemsResult<()> {
    let total = client.ps.ammo.get(weapon as usize)?.wrapping_add(count);
    client
        .ps
        .ammo
        .set(weapon as usize, if total > 200 { 200 } else { total })
}

/// Add ammo to a client entity (`addAmmo`).
pub fn add_ammo(pool: &mut EntityPool, other_slot: Slot, weapon: Weapon, count: i32) -> Q3GameItemsResult<()> {
    pool.require_owned(other_slot)?;
    let entity = &mut pool.entities[other_slot];
    let client = entity.client_mut()?;
    add_ammo_to(client, weapon, count)
}

/// Weapon pickup quantity input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeaponPickupQuantity {
    /// Entity count override.
    pub count: i32,
    /// Definition quantity.
    pub quantity: i32,
    /// Whether dropped.
    pub dropped: bool,
    /// Game type.
    pub game_type: i32,
    /// Current ammo.
    pub current_ammo: i32,
}

/// Weapon pickup quantity (`q3WeaponPickupQuantity`).
#[must_use]
pub fn q3_weapon_pickup_quantity(input: &WeaponPickupQuantity) -> i32 {
    if input.count < 0 {
        return 0;
    }
    let quantity = if input.count != 0 { input.count } else { input.quantity };
    if !input.dropped && input.game_type != GameType::Team as i32 {
        if input.current_ammo < quantity {
            quantity - input.current_ammo
        } else {
            1
        }
    } else {
        quantity
    }
}

/// Weapon respawn seconds (`q3WeaponRespawnSeconds`).
#[must_use]
pub fn q3_weapon_respawn_seconds(context: &WeaponPickupContext) -> i32 {
    if context.game_type == GameType::Team as i32 {
        context.team_weapon_respawn_seconds
    } else {
        context.weapon_respawn_seconds
    }
}

/// Item respawn seconds (`q3ItemRespawnSeconds`).
pub fn q3_item_respawn_seconds(item: &ItemDefinition, context: &WeaponPickupContext) -> Q3GameItemsResult<i32> {
    match item.item_type() {
        ItemType::Weapon => Ok(q3_weapon_respawn_seconds(context)),
        ItemType::Ammo => Ok(RESPAWN_AMMO),
        ItemType::Armor => Ok(RESPAWN_ARMOR),
        ItemType::Health => Ok(if item.quantity == 100 {
            RESPAWN_MEGAHEALTH
        } else {
            RESPAWN_HEALTH
        }),
        ItemType::Holdable => Ok(RESPAWN_HOLDABLE),
        ItemType::Powerup => Ok(RESPAWN_POWERUP),
        ItemType::PersistantPowerup => Ok(-1),
        ItemType::Team => Err(invalid("team objective lifecycle requires its original pickup handler")),
        ItemType::Bad => Err(invalid("invalid item has no pickup lifecycle")),
    }
}

/// Pick up ammo (`pickupAmmo`).
pub fn pickup_ammo(
    pool: &mut EntityPool,
    items: &dyn ItemTable,
    item_slot: Slot,
    other_slot: Slot,
) -> Q3GameItemsResult<i32> {
    let input = pickup_input(pool, items, item_slot, other_slot)?;
    if input.item.item_type() != ItemType::Ammo {
        return Err(invalid("Pickup_Ammo requires an ammo item"));
    }
    let count = pool.at(item_slot)?.count;
    let quantity = if count != 0 { count } else { input.item.quantity };
    let weapon = input.item.weapon_tag()?;
    pool.require_owned(other_slot)?;
    add_ammo_to(pool.entities[other_slot].client_mut()?, weapon, quantity)?;
    Ok(RESPAWN_AMMO)
}

/// Pick up a weapon (`pickupWeapon`).
pub fn pickup_weapon(
    pool: &mut EntityPool,
    items: &dyn ItemTable,
    item_slot: Slot,
    other_slot: Slot,
    context: &WeaponPickupContext,
) -> Q3GameItemsResult<i32> {
    let input = pickup_input(pool, items, item_slot, other_slot)?;
    if input.item.item_type() != ItemType::Weapon {
        return Err(invalid("Pickup_Weapon requires a weapon item"));
    }
    let holder = pool.at(item_slot)?;
    let count = holder.count;
    let dropped = holder.flags & GameFlags::DROPPED_ITEM != 0;
    let weapon = input.item.weapon_tag()?;
    pool.require_owned(other_slot)?;
    let entity = &mut pool.entities[other_slot];
    let client = entity.client_mut()?;
    let current = client.ps.ammo.get(weapon as usize)?;
    let quantity = q3_weapon_pickup_quantity(&WeaponPickupQuantity {
        count,
        quantity: input.item.quantity,
        dropped,
        game_type: context.game_type,
        current_ammo: current,
    });
    let schema = stat_schema(client.ps.product);
    let armed = client.ps.stats.get(schema.weapons)?;
    client.ps.stats.set(schema.weapons, armed | (1 << weapon as i32))?;
    add_ammo_to(client, weapon, quantity)?;
    if weapon == Weapon::GrapplingHook {
        client.ps.ammo.set(weapon as usize, -1)?;
    }
    Ok(q3_weapon_respawn_seconds(context))
}

/// Pick up health (`pickupHealth`).
pub fn pickup_health(
    pool: &mut EntityPool,
    items: &dyn ItemTable,
    item_slot: Slot,
    other_slot: Slot,
) -> Q3GameItemsResult<i32> {
    let input = pickup_input(pool, items, item_slot, other_slot)?;
    if input.item.item_type() != ItemType::Health {
        return Err(invalid("Pickup_Health requires a health item"));
    }
    let count = pool.at(item_slot)?.count;
    let quantity = if count != 0 { count } else { input.item.quantity };
    let guard = has_guard(pool, items, other_slot)?;
    pool.require_owned(other_slot)?;
    let entity = &mut pool.entities[other_slot];
    let client = entity.client_mut()?;
    let schema = stat_schema(client.ps.product);
    let max_health = client.ps.stats.get(schema.max_health)?;
    let limit = if guard || (input.item.quantity != 5 && input.item.quantity != 100) {
        max_health
    } else {
        max_health.wrapping_mul(2)
    };
    entity.health = entity.health.wrapping_add(quantity);
    if entity.health > limit {
        entity.health = limit;
    }
    let health = entity.health;
    entity.client_mut()?.ps.stats.set(schema.health, health)?;
    Ok(if input.item.quantity == 100 {
        RESPAWN_MEGAHEALTH
    } else {
        RESPAWN_HEALTH
    })
}

/// Pick up armor (`pickupArmor`).
pub fn pickup_armor(
    pool: &mut EntityPool,
    items: &dyn ItemTable,
    item_slot: Slot,
    other_slot: Slot,
) -> Q3GameItemsResult<i32> {
    let input = pickup_input(pool, items, item_slot, other_slot)?;
    if input.item.item_type() != ItemType::Armor {
        return Err(invalid("Pickup_Armor requires an armor item"));
    }
    let guard = has_guard(pool, items, other_slot)?;
    pool.require_owned(other_slot)?;
    let client = pool.entities[other_slot].client_mut()?;
    let schema = stat_schema(client.ps.product);
    let armor = client.ps.stats.get(schema.armor)?.wrapping_add(input.item.quantity);
    let max_health = client.ps.stats.get(schema.max_health)?;
    let limit = if guard { max_health } else { max_health.wrapping_mul(2) };
    client
        .ps
        .stats
        .set(schema.armor, if armor > limit { limit } else { armor })?;
    Ok(RESPAWN_ARMOR)
}

/// Pick up a holdable (`pickupHoldable`).
pub fn pickup_holdable(
    pool: &mut EntityPool,
    items: &dyn ItemTable,
    item_slot: Slot,
    other_slot: Slot,
) -> Q3GameItemsResult<i32> {
    let input = pickup_input(pool, items, item_slot, other_slot)?;
    if input.item.item_type() != ItemType::Holdable {
        return Err(invalid("Pickup_Holdable requires a holdable item"));
    }
    pool.require_owned(other_slot)?;
    let client = pool.entities[other_slot].client_mut()?;
    let schema = stat_schema(client.ps.product);
    client.ps.stats.set(schema.holdable_item, input.item_index as i32)?;
    if input.item.holdable_tag()? == Holdable::Kamikaze {
        client.ps.e_flags |= EF_KAMIKAZE;
    }
    Ok(RESPAWN_HOLDABLE)
}

pub(crate) fn source_handicap(raw_handicap: &str) -> f32 {
    let parsed = game_atof(raw_handicap);
    if parsed <= 0.0 || parsed > 100.0 {
        100.0
    } else {
        parsed
    }
}

/// Pick up a persistent powerup (`pickupPersistentPowerup`).
pub fn pickup_persistent_powerup(
    pool: &mut EntityPool,
    items: &dyn ItemTable,
    item_slot: Slot,
    other_slot: Slot,
    raw_handicap: &str,
) -> Q3GameItemsResult<i32> {
    let input = pickup_input(pool, items, item_slot, other_slot)?;
    if input.product != Product::Missionpack {
        return Err(invalid("persistent powerups require missionpack player state"));
    }
    if input.item.item_type() != ItemType::PersistantPowerup {
        return Err(invalid("Pickup_PersistantPowerup requires a persistent powerup item"));
    }
    pool.require_owned(other_slot)?;
    let entity = &mut pool.entities[other_slot];
    let client = entity.client_mut()?;
    client
        .ps
        .stats
        .set(MissionpackStatIndex::PERSISTANT_POWERUP, input.item_index as i32)?;
    client.persistant_powerup = Some(item_slot);
    let handicap = source_handicap(raw_handicap);
    let player_maximum = handicap.trunc() as i32;
    match input.item.powerup_tag()? {
        Powerup::Guard => {
            let maximum = (2.0 * handicap).trunc() as i32;
            entity.health = maximum;
            let client = entity.client_mut()?;
            client.ps.stats.set(MissionpackStatIndex::HEALTH, maximum)?;
            client.ps.stats.set(MissionpackStatIndex::MAX_HEALTH, maximum)?;
            client.ps.stats.set(MissionpackStatIndex::ARMOR, maximum)?;
            client.pers.max_health = maximum;
        }
        Powerup::Scout => {
            let client = entity.client_mut()?;
            client.pers.max_health = player_maximum;
            client.ps.stats.set(MissionpackStatIndex::ARMOR, 0)?;
        }
        Powerup::Doubler => {
            entity.client_mut()?.pers.max_health = player_maximum;
        }
        Powerup::AmmoRegen => {
            let client = entity.client_mut()?;
            client.pers.max_health = player_maximum;
            for index in 0..client.ammo_times.len() {
                client.ammo_times.set(index, 0)?;
            }
        }
        _ => {
            entity.client_mut()?.pers.max_health = player_maximum;
        }
    }
    Ok(-1)
}

/// Pick up a powerup (`pickupPowerup`).
pub fn pickup_powerup(
    pool: &mut EntityPool,
    items: &dyn ItemTable,
    item_slot: Slot,
    other_slot: Slot,
    context: &mut PowerupPickupContext<'_>,
) -> Q3GameItemsResult<i32> {
    let input = pickup_input(pool, items, item_slot, other_slot)?;
    if input.item.item_type() != ItemType::Powerup {
        return Err(invalid("Pickup_Powerup requires a powerup item"));
    }
    let count = pool.at(item_slot)?.count;
    let quantity = if count != 0 { count } else { input.item.quantity };
    let tag = input.item.powerup_tag()? as usize;
    pool.require_owned(other_slot)?;
    {
        let client = pool.entities[other_slot].client_mut()?;
        let mut expiration = client.ps.powerups.get(tag)?;
        if expiration == 0 {
            expiration = context.time - context.time % 1000;
        }
        client
            .ps
            .powerups
            .set(tag, expiration.wrapping_add(quantity.wrapping_mul(1000)))?;
    }
    let item_base = pool.at(item_slot)?.s.pos.base;
    let picker_team = pool
        .at(other_slot)?
        .client
        .as_ref()
        .expect("client checked")
        .sess
        .session_team;
    let candidates: Vec<Slot> = (0..pool.num_entities())
        .filter(|slot| *slot != other_slot && pool.get(*slot).is_some_and(|e| e.client.is_some()))
        .collect();
    for slot in candidates {
        let candidate = pool.at(slot)?;
        let client = candidate.client.as_ref().expect("client scanned");
        if client.pers.connected == ConnectionState::Disconnected {
            continue;
        }
        let schema = stat_schema(client.ps.product);
        if client.ps.stats.get(schema.health)? <= 0 {
            continue;
        }
        if context.game_type >= GameType::Team as i32 && client.sess.session_team == picker_team {
            continue;
        }
        let delta = sub3(item_base, client.ps.origin);
        if length3(delta) > 192.0 {
            continue;
        }
        let direction = normalize3(delta);
        if dot3(direction, qvm_angle_vectors(client.ps.viewangles).forward) < 0.4 {
            continue;
        }
        let start = client.ps.origin;
        if (context.trace_solid_line)(start, item_base).fraction != 1.0 {
            continue;
        }
        let candidate = pool.at_mut(slot)?;
        let client = candidate.client_mut()?;
        let events = client.ps.persistant.get(PersistentIndex::PLAYEREVENTS)?;
        client
            .ps
            .persistant
            .set(PersistentIndex::PLAYEREVENTS, events ^ PLAYEREVENT_DENIED_REWARD)?;
    }
    Ok(RESPAWN_POWERUP)
}

/// Pick up any item (`pickupItem`).
pub fn pickup_item(
    pool: &mut EntityPool,
    items: &dyn ItemTable,
    item_slot: Slot,
    other_slot: Slot,
    context: &mut ItemPickupContext<'_>,
) -> Q3GameItemsResult<i32> {
    let input = pickup_input(pool, items, item_slot, other_slot)?;
    match input.item.item_type() {
        ItemType::Weapon => {
            let weapon = context.weapon_context();
            pickup_weapon(pool, items, item_slot, other_slot, &weapon)
        }
        ItemType::Ammo => pickup_ammo(pool, items, item_slot, other_slot),
        ItemType::Armor => pickup_armor(pool, items, item_slot, other_slot),
        ItemType::Health => pickup_health(pool, items, item_slot, other_slot),
        ItemType::Powerup => {
            let mut powerup = PowerupPickupContext {
                time: context.time,
                game_type: context.game_type,
                trace_solid_line: &mut *context.trace_solid_line,
            };
            pickup_powerup(pool, items, item_slot, other_slot, &mut powerup)
        }
        ItemType::Holdable => pickup_holdable(pool, items, item_slot, other_slot),
        ItemType::PersistantPowerup => {
            let client_num = pool
                .at(other_slot)?
                .client
                .as_ref()
                .expect("client checked")
                .ps
                .client_num;
            let handicap = (context.handicap_for_client)(client_num);
            pickup_persistent_powerup(pool, items, item_slot, other_slot, &handicap)
        }
        ItemType::Team => Err(invalid(
            "Pickup_Item: IT_TEAM requires the team objective pickup handler",
        )),
        ItemType::Bad => Err(invalid("Pickup_Item: IT_BAD cannot be picked up")),
    }
}
