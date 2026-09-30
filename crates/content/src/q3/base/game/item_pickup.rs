//! Quake III base/game: item pickup.
//!
//! Donor provenance: `src/content/q3/base/game/item-pickup.ts`.

use qa_core::math::{angle_vectors, dot3, length3, normalize3, sub3, Vec3};

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::items_core::*;
use crate::q3::base::game::numeric::game_atof;
use crate::q3::base::game::state::{ConnectionState, GameFlags};
use crate::q3::base::shared::definitions::{
    stat_schema, GameType, Holdable, ItemType, MissionpackStatIndex, PersistentIndex, Powerup, Product, StatSchema,
    Weapon,
};

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
    let index = client
        .ps
        .stats
        .get(MissionpackStatIndex::StatPersistantPowerup as usize)? as usize;
    Ok(items.item_at(Product::Missionpack, index)?.powerup_tag()? == Powerup::PwGuard)
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
    if !input.dropped && input.game_type != GameType::GtTeam as i32 {
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
    if context.game_type == GameType::GtTeam as i32 {
        context.team_weapon_respawn_seconds
    } else {
        context.weapon_respawn_seconds
    }
}

/// Item respawn seconds (`q3ItemRespawnSeconds`).
pub fn q3_item_respawn_seconds(item: &ItemDefinition, context: &WeaponPickupContext) -> Q3GameItemsResult<i32> {
    match item.item_type() {
        ItemType::ItWeapon => Ok(q3_weapon_respawn_seconds(context)),
        ItemType::ItAmmo => Ok(RESPAWN_AMMO),
        ItemType::ItArmor => Ok(RESPAWN_ARMOR),
        ItemType::ItHealth => Ok(if item.quantity == 100 {
            RESPAWN_MEGAHEALTH
        } else {
            RESPAWN_HEALTH
        }),
        ItemType::ItHoldable => Ok(RESPAWN_HOLDABLE),
        ItemType::ItPowerup => Ok(RESPAWN_POWERUP),
        ItemType::ItPersistantPowerup => Ok(-1),
        ItemType::ItTeam => Err(invalid("team objective lifecycle requires its original pickup handler")),
        ItemType::ItBad => Err(invalid("invalid item has no pickup lifecycle")),
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
    if input.item.item_type() != ItemType::ItAmmo {
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
    if input.item.item_type() != ItemType::ItWeapon {
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
    let weapons = match stat_schema(client.ps.product) {
        StatSchema::Base(layout) => layout.weapons,
        StatSchema::Missionpack(layout) => layout.weapons,
    } as usize;
    let armed = client.ps.stats.get(weapons)?;
    client.ps.stats.set(weapons, armed | (1 << weapon as i32))?;
    add_ammo_to(client, weapon, quantity)?;
    if weapon == Weapon::WpGrapplingHook {
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
    if input.item.item_type() != ItemType::ItHealth {
        return Err(invalid("Pickup_Health requires a health item"));
    }
    let count = pool.at(item_slot)?.count;
    let quantity = if count != 0 { count } else { input.item.quantity };
    let guard = has_guard(pool, items, other_slot)?;
    pool.require_owned(other_slot)?;
    let entity = &mut pool.entities[other_slot];
    let client = entity.client_mut()?;
    let (health_slot, max_health_slot) = match stat_schema(client.ps.product) {
        StatSchema::Base(layout) => (layout.health, layout.max_health),
        StatSchema::Missionpack(layout) => (layout.health, layout.max_health),
    };
    let max_health = client.ps.stats.get(max_health_slot as usize)?;
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
    entity.client_mut()?.ps.stats.set(health_slot as usize, health)?;
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
    if input.item.item_type() != ItemType::ItArmor {
        return Err(invalid("Pickup_Armor requires an armor item"));
    }
    let guard = has_guard(pool, items, other_slot)?;
    pool.require_owned(other_slot)?;
    let client = pool.entities[other_slot].client_mut()?;
    let (armor_slot, max_health_slot) = match stat_schema(client.ps.product) {
        StatSchema::Base(layout) => (layout.armor, layout.max_health),
        StatSchema::Missionpack(layout) => (layout.armor, layout.max_health),
    };
    let armor = client
        .ps
        .stats
        .get(armor_slot as usize)?
        .wrapping_add(input.item.quantity);
    let max_health = client.ps.stats.get(max_health_slot as usize)?;
    let limit = if guard { max_health } else { max_health.wrapping_mul(2) };
    client
        .ps
        .stats
        .set(armor_slot as usize, if armor > limit { limit } else { armor })?;
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
    if input.item.item_type() != ItemType::ItHoldable {
        return Err(invalid("Pickup_Holdable requires a holdable item"));
    }
    pool.require_owned(other_slot)?;
    let client = pool.entities[other_slot].client_mut()?;
    let holdable_item = match stat_schema(client.ps.product) {
        StatSchema::Base(layout) => layout.holdable_item,
        StatSchema::Missionpack(layout) => layout.holdable_item,
    } as usize;
    client.ps.stats.set(holdable_item, input.item_index as i32)?;
    if input.item.holdable_tag()? == Holdable::HiKamikaze {
        client.ps.e_flags |= EF_KAMIKAZE;
    }
    Ok(RESPAWN_HOLDABLE)
}

pub(crate) fn source_handicap(raw_handicap: &str) -> f32 {
    let parsed = game_atof(raw_handicap).unwrap_or(0.0);
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
    if input.item.item_type() != ItemType::ItPersistantPowerup {
        return Err(invalid("Pickup_PersistantPowerup requires a persistent powerup item"));
    }
    pool.require_owned(other_slot)?;
    let entity = &mut pool.entities[other_slot];
    let client = entity.client_mut()?;
    client.ps.stats.set(
        MissionpackStatIndex::StatPersistantPowerup as usize,
        input.item_index as i32,
    )?;
    client.persistant_powerup = Some(item_slot);
    let handicap = source_handicap(raw_handicap);
    let player_maximum = handicap.trunc() as i32;
    match input.item.powerup_tag()? {
        Powerup::PwGuard => {
            let maximum = (2.0 * handicap).trunc() as i32;
            entity.health = maximum;
            let client = entity.client_mut()?;
            client
                .ps
                .stats
                .set(MissionpackStatIndex::StatHealth as usize, maximum)?;
            client
                .ps
                .stats
                .set(MissionpackStatIndex::StatMaxHealth as usize, maximum)?;
            client.ps.stats.set(MissionpackStatIndex::StatArmor as usize, maximum)?;
            client.pers.max_health = maximum;
        }
        Powerup::PwScout => {
            let client = entity.client_mut()?;
            client.pers.max_health = player_maximum;
            client.ps.stats.set(MissionpackStatIndex::StatArmor as usize, 0)?;
        }
        Powerup::PwDoubler => {
            entity.client_mut()?.pers.max_health = player_maximum;
        }
        Powerup::PwAmmoregen => {
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
    if input.item.item_type() != ItemType::ItPowerup {
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
        let health_slot = match stat_schema(client.ps.product) {
            StatSchema::Base(layout) => layout.health,
            StatSchema::Missionpack(layout) => layout.health,
        } as usize;
        if client.ps.stats.get(health_slot)? <= 0 {
            continue;
        }
        if context.game_type >= GameType::GtTeam as i32 && client.sess.session_team == picker_team {
            continue;
        }
        let delta = sub3(item_base, client.ps.origin);
        if length3(delta) > 192.0 {
            continue;
        }
        let direction = normalize3(delta);
        if dot3(direction, angle_vectors(client.ps.viewangles).forward) < 0.4 {
            continue;
        }
        let start = client.ps.origin;
        if (context.trace_solid_line)(start, item_base).fraction != 1.0 {
            continue;
        }
        let candidate = pool.at_mut(slot)?;
        let client = candidate.client_mut()?;
        let events = client.ps.persistant.get(PersistentIndex::PersPlayerevents as usize)?;
        client.ps.persistant.set(
            PersistentIndex::PersPlayerevents as usize,
            events ^ PLAYEREVENT_DENIED_REWARD,
        )?;
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
        ItemType::ItWeapon => {
            let weapon = context.weapon_context();
            pickup_weapon(pool, items, item_slot, other_slot, &weapon)
        }
        ItemType::ItAmmo => pickup_ammo(pool, items, item_slot, other_slot),
        ItemType::ItArmor => pickup_armor(pool, items, item_slot, other_slot),
        ItemType::ItHealth => pickup_health(pool, items, item_slot, other_slot),
        ItemType::ItPowerup => {
            let mut powerup = PowerupPickupContext {
                time: context.time,
                game_type: context.game_type,
                trace_solid_line: &mut *context.trace_solid_line,
            };
            pickup_powerup(pool, items, item_slot, other_slot, &mut powerup)
        }
        ItemType::ItHoldable => pickup_holdable(pool, items, item_slot, other_slot),
        ItemType::ItPersistantPowerup => {
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
        ItemType::ItTeam => Err(invalid(
            "Pickup_Item: IT_TEAM requires the team objective pickup handler",
        )),
        ItemType::ItBad => Err(invalid("Pickup_Item: IT_BAD cannot be picked up")),
    }
}

#[cfg(test)]
mod tests {
    use qa_core::math::{vec3, Vec3};

    use super::*;
    use crate::q3::base::game::items_core::test_support::*;
    use crate::q3::base::game::state::ConnectionState;
    use crate::q3::base::shared::definitions::*;

    #[test]
    fn item_table_counts_and_schema() {
        assert_eq!(weapon_count(Product::Baseq3), 11);
        assert_eq!(weapon_count(Product::Missionpack), 14);
        let baseq3 = stat_schema(Product::Baseq3);
        let StatSchema::Base(base) = baseq3 else {
            panic!("baseq3 stat layout");
        };
        assert_eq!(base.weapons, 2);
        let missionpack = stat_schema(Product::Missionpack);
        let StatSchema::Missionpack(mission) = missionpack else {
            panic!("missionpack stat layout");
        };
        assert_eq!(mission.weapons, 3);
    }

    #[test]
    fn item_tables_and_quantities() {
        let items = test_items();
        let weapon_ctx = WeaponPickupContext {
            game_type: GameType::GtFfa as i32,
            weapon_respawn_seconds: 5,
            team_weapon_respawn_seconds: 30,
        };
        assert_eq!(q3_item_respawn_seconds(&items.list[1], &weapon_ctx).unwrap(), 5);
        assert_eq!(
            q3_item_respawn_seconds(&items.list[4], &weapon_ctx).unwrap(),
            RESPAWN_AMMO
        );
        assert_eq!(q3_item_respawn_seconds(&items.list[5], &weapon_ctx).unwrap(), 25);
        assert_eq!(q3_item_respawn_seconds(&items.list[6], &weapon_ctx).unwrap(), 35);
        assert_eq!(q3_item_respawn_seconds(&items.list[7], &weapon_ctx).unwrap(), 35);
        assert_eq!(q3_item_respawn_seconds(&items.list[8], &weapon_ctx).unwrap(), 120);
        assert_eq!(q3_item_respawn_seconds(&items.list[9], &weapon_ctx).unwrap(), 60);
        assert_eq!(q3_item_respawn_seconds(&items.list[10], &weapon_ctx).unwrap(), -1);
        assert!(q3_item_respawn_seconds(&items.list[11], &weapon_ctx).is_err());
        assert!(q3_item_respawn_seconds(&items.list[0], &weapon_ctx).is_err());
        let team_ctx = WeaponPickupContext {
            game_type: GameType::GtTeam as i32,
            ..weapon_ctx
        };
        assert_eq!(q3_weapon_respawn_seconds(&team_ctx), 30);
        assert_eq!(
            q3_weapon_pickup_quantity(&WeaponPickupQuantity {
                count: -1,
                quantity: 10,
                dropped: false,
                game_type: 0,
                current_ammo: 0
            }),
            0
        );
        assert_eq!(
            q3_weapon_pickup_quantity(&WeaponPickupQuantity {
                count: 0,
                quantity: 10,
                dropped: false,
                game_type: 0,
                current_ammo: 3
            }),
            7
        );
        assert_eq!(
            q3_weapon_pickup_quantity(&WeaponPickupQuantity {
                count: 0,
                quantity: 10,
                dropped: false,
                game_type: 0,
                current_ammo: 50
            }),
            1
        );
        assert_eq!(
            q3_weapon_pickup_quantity(&WeaponPickupQuantity {
                count: 0,
                quantity: 10,
                dropped: true,
                game_type: 0,
                current_ammo: 3
            }),
            10
        );
    }

    #[test]
    fn pickup_flows_cover_all_types() {
        let items = test_items();
        let mut pool = EntityPool::new(Product::Baseq3);
        let player = player_slot(&mut pool, Product::Baseq3);
        let ammo = item_slot(&mut pool, &items, 4);
        assert_eq!(pickup_ammo(&mut pool, &items, ammo, player).unwrap(), 40);
        assert_eq!(
            pool.at(player)
                .unwrap()
                .client
                .as_ref()
                .unwrap()
                .ps
                .ammo
                .get(Weapon::WpRocketLauncher as usize)
                .unwrap(),
            5
        );
        let weapon = item_slot(&mut pool, &items, 1);
        let ctx = WeaponPickupContext {
            game_type: GameType::GtFfa as i32,
            weapon_respawn_seconds: 5,
            team_weapon_respawn_seconds: 30,
        };
        assert_eq!(pickup_weapon(&mut pool, &items, weapon, player, &ctx).unwrap(), 5);
        let client = pool.at(player).unwrap().client.as_ref().unwrap();
        assert_ne!(
            client.ps.stats.get(2).unwrap() & (1 << Weapon::WpRocketLauncher as i32),
            0
        );
        let armor = item_slot(&mut pool, &items, 5);
        pool.at_mut(player)
            .unwrap()
            .client
            .as_mut()
            .unwrap()
            .ps
            .stats
            .set(6, 100)
            .unwrap();
        assert_eq!(pickup_armor(&mut pool, &items, armor, player).unwrap(), 25);
        let health = item_slot(&mut pool, &items, 6);
        pool.at_mut(player).unwrap().health = 90;
        assert_eq!(pickup_health(&mut pool, &items, health, player).unwrap(), 35);
        assert_eq!(pool.at(player).unwrap().health, 95);
        let holdable = item_slot(&mut pool, &items, 9);
        assert_eq!(pickup_holdable(&mut pool, &items, holdable, player).unwrap(), 60);
        let client = pool.at(player).unwrap().client.as_ref().unwrap();
        assert_eq!(client.ps.stats.get(1).unwrap(), 9);
        assert_ne!(client.ps.e_flags & 0x200, 0);
        let mut trace = |_a: Vec3, _b: Vec3| PowerupSightTrace { fraction: 1.0 };
        let mut powerup_ctx = PowerupPickupContext {
            time: 1234,
            game_type: GameType::GtFfa as i32,
            trace_solid_line: &mut trace,
        };
        let powerup = item_slot(&mut pool, &items, 8);
        assert_eq!(
            pickup_powerup(&mut pool, &items, powerup, player, &mut powerup_ctx).unwrap(),
            120
        );
        let client = pool.at(player).unwrap().client.as_ref().unwrap();
        assert_eq!(client.ps.powerups.get(Powerup::PwQuad as usize).unwrap(), 1000 + 30_000);
        let mut handicap = |_client: i32| "100".to_string();
        let mut sight = |_a: Vec3, _b: Vec3| PowerupSightTrace { fraction: 1.0 };
        let mut full = ItemPickupContext {
            game_type: GameType::GtFfa as i32,
            weapon_respawn_seconds: 5,
            team_weapon_respawn_seconds: 30,
            time: 2000,
            trace_solid_line: &mut sight,
            handicap_for_client: &mut handicap,
        };
        let ammo2 = item_slot(&mut pool, &items, 4);
        assert_eq!(pickup_item(&mut pool, &items, ammo2, player, &mut full).unwrap(), 40);
        let flag = item_slot(&mut pool, &items, 11);
        assert!(pickup_item(&mut pool, &items, flag, player, &mut full).is_err());
    }

    #[test]
    fn persistent_powerup_and_denied_reward() {
        let items = test_items();
        let mut pool = EntityPool::new(Product::Missionpack);
        let player = player_slot(&mut pool, Product::Missionpack);
        let guard = item_slot(&mut pool, &items, 10);
        assert_eq!(
            pickup_persistent_powerup(&mut pool, &items, guard, player, "80").unwrap(),
            -1
        );
        let client = pool.at(player).unwrap().client.as_ref().unwrap();
        assert_eq!(client.ps.stats.get(2).unwrap(), 10);
        assert_eq!(client.pers.max_health, 160);
        assert_eq!(pool.at(player).unwrap().health, 160);
        // Witness facing the pickup sees a denied reward toggle.
        let witness = player_slot(&mut pool, Product::Missionpack);
        pool.at_mut(witness).unwrap().client.as_mut().unwrap().pers.connected = ConnectionState::Connected;
        pool.at_mut(witness)
            .unwrap()
            .client
            .as_mut()
            .unwrap()
            .ps
            .stats
            .set(0, 100)
            .unwrap();
        pool.at_mut(witness).unwrap().client.as_mut().unwrap().ps.origin = vec3(100.0, 0.0, 0.0);
        pool.at_mut(witness).unwrap().client.as_mut().unwrap().ps.viewangles = vec3(0.0, 180.0, 0.0);
        let powerup = item_slot(&mut pool, &items, 8);
        pool.at_mut(powerup).unwrap().s.pos.base = vec3(0.0, 0.0, 0.0);
        let mut trace = |_a: Vec3, _b: Vec3| PowerupSightTrace { fraction: 1.0 };
        let mut ctx = PowerupPickupContext {
            time: 2000,
            game_type: GameType::GtFfa as i32,
            trace_solid_line: &mut trace,
        };
        pickup_powerup(&mut pool, &items, powerup, player, &mut ctx).unwrap();
        let witness_client = pool.at(witness).unwrap().client.as_ref().unwrap();
        assert_eq!(witness_client.ps.persistant.get(5).unwrap(), 1);
        // Baseq3 rejects persistent powerups.
        let mut base_pool = EntityPool::new(Product::Baseq3);
        let base_player = player_slot(&mut base_pool, Product::Baseq3);
        let base_guard = item_slot(&mut base_pool, &items, 10);
        assert!(pickup_persistent_powerup(&mut base_pool, &items, base_guard, base_player, "100").is_err());
    }
}
