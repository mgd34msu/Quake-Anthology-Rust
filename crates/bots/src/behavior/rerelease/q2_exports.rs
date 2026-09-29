//! Q2 bot exports from `src/bots/behavior/rerelease/q2-exports.ts`
//! (`bot_exports.cpp`: `Bot_SetWeapon`, `Bot_TriggerEdict`,
//! `Bot_UseItem`, `Bot_GetItemID`, `Edict_ForceLookAtPoint`,
//! `Bot_PickedUpItem`).
//!
//! Source operations act through the selected game's existing
//! inventory/callback owner.

use qa_core::math::Vec3;

use crate::behavior::rerelease::math::{bvec_sub, vector_to_angles};

/// Bot client view.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2BotClientView {
    /// In use.
    pub inuse: bool,
    /// Is a bot.
    pub bot: bool,
    /// Client state.
    pub client: Option<Q2BotClientState>,
}

/// Bot client state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2BotClientState {
    /// Current weapon.
    pub current_weapon: i32,
    /// Pending weapon.
    pub pending_weapon: i32,
    /// Selected item.
    pub selected_item: i32,
    /// Origin.
    pub origin: Vec3,
    /// View offset.
    pub view_offset: Vec3,
    /// Command angles.
    pub command_angles: Vec3,
}

/// Item view.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2BotItemView {
    /// Id.
    pub id: i32,
    /// Classname.
    pub classname: Option<String>,
    /// Is a weapon.
    pub weapon: bool,
    /// Use callback name.
    pub use_callback: Option<String>,
}

/// Entity view.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2BotEntityView {
    /// In use.
    pub inuse: bool,
    /// Use callback name.
    pub use_callback: Option<String>,
    /// Touch callback name.
    pub touch_callback: Option<String>,
}

/// Q2 exports host: the selected game's inventory/callback owner.
pub trait Q2BotExportsHost {
    /// Item count.
    fn item_count(&self) -> i32;
    /// Bot view for an entity.
    fn bot(&self, entity: i32) -> Option<Q2BotClientView>;
    /// Entity view.
    fn entity(&self, entity: i32) -> Option<Q2BotEntityView>;
    /// Item view.
    fn item(&self, item: i32) -> Option<Q2BotItemView>;
    /// Inventory count.
    fn inventory(&self, entity: i32, item: i32) -> i32;
    /// Set selected item.
    fn set_selected_item(&mut self, entity: i32, item: i32);
    /// Validate selected item.
    fn validate_selected_item(&mut self, entity: i32);
    /// Disable weapon chains.
    fn disable_weapon_chains(&mut self, entity: i32);
    /// Use an item.
    fn use_item(&mut self, callback: &str, entity: i32, item: i32);
    /// Instant weapon switch (arsenal-owned).
    fn change_weapon_instantly(&mut self, entity: i32);
    /// Trigger use.
    fn trigger_use(&mut self, callback: &str, target: i32, other: i32, activator: i32);
    /// Trigger touch.
    fn trigger_touch(&mut self, callback: &str, target: i32, other: i32, other_touching_self: bool);
    /// Force look delta.
    fn force_look(&mut self, entity: i32, delta_angles: Vec3);
    /// Pickup check.
    fn picked_up_by(&self, item_entity: i32, source_client: i32) -> bool;
}

/// Set a bot's weapon (`Bot_SetWeapon`).
pub fn bot_set_weapon(host: &mut dyn Q2BotExportsHost, entity: i32, weapon: i32, instant_switch: bool) {
    if weapon <= 0 || weapon > host.item_count() {
        return;
    }
    let bot = host.bot(entity);
    let Some(bot) = bot else {
        return;
    };
    if !bot.bot {
        return;
    }
    let Some(client) = bot.client else {
        return;
    };
    if host.inventory(entity, weapon) == 0 {
        return;
    }
    if client.current_weapon == weapon || client.pending_weapon == weapon {
        return;
    }
    let item = host.item(weapon);
    let Some(item) = item else {
        return;
    };
    if !item.weapon {
        return;
    }
    let Some(callback) = item.use_callback.clone() else {
        return;
    };
    host.disable_weapon_chains(entity);
    host.use_item(&callback, entity, item.id);
    if instant_switch {
        host.change_weapon_instantly(entity);
    }
}

/// Trigger an edict (`Bot_TriggerEdict`).
pub fn bot_trigger_edict(host: &mut dyn Q2BotExportsHost, entity: i32, target: i32) {
    let bot = host.bot(entity);
    let initial = host.entity(target);
    match (bot, initial) {
        (Some(bot), Some(initial)) if bot.inuse && bot.bot && initial.inuse => {
            if let Some(callback) = initial.use_callback.clone() {
                host.trigger_use(&callback, target, entity, entity);
            }
            if let Some(reached) = host.entity(target) {
                if let Some(callback) = reached.touch_callback.clone() {
                    host.trigger_touch(&callback, target, entity, true);
                }
            }
        }
        _ => {}
    }
}

/// Use an item (`Bot_UseItem`).
pub fn bot_use_item(host: &mut dyn Q2BotExportsHost, entity: i32, item_id: i32) {
    let bot = host.bot(entity);
    match bot {
        Some(bot) if bot.inuse && bot.bot && bot.client.is_some() => {}
        _ => return,
    }
    host.set_selected_item(entity, item_id);
    host.validate_selected_item(entity);
    let selected = host
        .bot(entity)
        .and_then(|bot| bot.client.map(|client| client.selected_item));
    if selected.is_none_or(|selected| selected == 0 || selected != item_id) {
        return;
    }
    let item = host.item(item_id);
    host.set_selected_item(entity, 0);
    let Some(item) = item else {
        return;
    };
    let Some(callback) = item.use_callback.clone() else {
        return;
    };
    host.disable_weapon_chains(entity);
    host.use_item(&callback, entity, item.id);
}

/// Item id for a classname (`Bot_GetItemID`).
#[must_use]
pub fn bot_get_item_id(host: &dyn Q2BotExportsHost, classname: Option<&str>) -> i32 {
    let Some(classname) = classname else {
        return -1;
    };
    if classname.is_empty() || classname.starts_with('\0') {
        return -1;
    }
    let name = classname.split('\0').next().unwrap_or("").to_lowercase();
    if name == "none" {
        return 0;
    }
    for index in 0..host.item_count() {
        if let Some(item) = host.item(index) {
            if let Some(item_name) = item.classname {
                if !item_name.is_empty() && item_name.to_lowercase() == name {
                    return item.id;
                }
            }
        }
    }
    -1
}

/// Force look at a point (`Edict_ForceLookAtPoint`).
pub fn edict_force_look_at_point(host: &mut dyn Q2BotExportsHost, entity: i32, point: Vec3) {
    let client = host.bot(entity).and_then(|bot| bot.client);
    let Some(client) = client else {
        return;
    };
    let eye = Vec3 {
        x: client.origin.x + client.view_offset.x,
        y: client.origin.y + client.view_offset.y,
        z: client.origin.z + client.view_offset.z,
    };
    let (pitch, yaw) = vector_to_angles(bvec_sub(point, eye));
    host.force_look(
        entity,
        Vec3 {
            x: pitch - client.command_angles.x,
            y: yaw - client.command_angles.y,
            z: -client.command_angles.z,
        },
    );
}

/// Pickup check (`Bot_PickedUpItem`).
#[must_use]
pub fn bot_picked_up_item(host: &dyn Q2BotExportsHost, entity: i32, item_entity: i32) -> bool {
    host.picked_up_by(item_entity, entity - 1)
}
