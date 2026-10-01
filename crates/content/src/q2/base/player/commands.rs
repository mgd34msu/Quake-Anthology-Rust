//! Q2 client commands (`src/content/q2/base/player/commands.ts`).

use std::collections::BTreeMap;

use qa_core::identity::ActorId;

use crate::contract::{InventoryCountPolicy, InventoryEntry, SourceCounterArithmetic};
use crate::q2::foundation::host::{Q2Edition, Q2GameServices, Q2Mode};
use crate::q2::foundation::items::Q2ConsoleGive;
use crate::q2::foundation::weapons::player::Q2WeaponSelection;
use crate::q2::foundation::weapons::player::{can_drop_weapon, registered_weapon_definitions, request_weapon};
use crate::q2::support::contracts::CombatTraitChanges;

use super::index::{
    can_drop_coop_stay_items, chase_player, environment_damage, player_hooks, player_items, send_scoreboard,
    Q2Intermission,
};
use super::types::{Q2PlayerContext, Q2PlayerEvent, Q2PrintLevel, Q2ScoreRow};

/// Command documentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Q2CommandDocumentation {
    /// Summary.
    pub summary: &'static str,
    /// Usage.
    pub usage: &'static str,
    /// Examples.
    pub examples: &'static [&'static str],
}

/// Client command handler.
///
/// The donor also receives the players module; the rules, states and
/// submodules it reads live on the game services here.
pub type Q2CommandHandler = fn(&mut Q2PlayerContext, &[String], &str);

/// Client command definition (`q2ClientCommands` entry).
#[derive(Debug, Clone, Copy)]
pub struct Q2ClientCommandDefinition {
    /// Name.
    pub name: &'static str,
    /// Documentation.
    pub documentation: Q2CommandDocumentation,
    /// Whether allowed during intermission.
    pub intermission: bool,
    /// Handler.
    pub run: Q2CommandHandler,
}

/// Parse a command integer prefix (JS `parseInt` semantics).
fn parse_command_prefix(value: &str) -> Option<i64> {
    let text = value.trim_start();
    let (sign, digits) = match text.strip_prefix('-') {
        Some(rest) => (-1i64, rest),
        None => (1i64, text.strip_prefix('+').unwrap_or(text)),
    };
    let mut parsed: i64 = 0;
    let mut any = false;
    for ch in digits.chars() {
        match ch.to_digit(10) {
            Some(digit) => {
                any = true;
                parsed = parsed.saturating_mul(10).saturating_add(i64::from(digit));
            }
            None => break,
        }
    }
    if any {
        Some(sign * parsed)
    } else {
        None
    }
}

/// Parse a command integer (JS `parseInt || 0` semantics).
pub fn parse_command_int(value: Option<&str>) -> i32 {
    parse_command_prefix(value.unwrap_or("0"))
        .unwrap_or(0)
        .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

/// Read a userinfo value (`q2Userinfo` lookup).
pub fn userinfo_value(userinfo: &str, key: &str) -> Option<String> {
    let source = userinfo.strip_prefix('\\').unwrap_or(userinfo);
    let fields: Vec<&str> = source.split('\\').collect();
    let mut index = 0;
    while index + 1 < fields.len() {
        if fields[index] == key {
            return Some(fields[index + 1].to_string());
        }
        index += 2;
    }
    None
}

/// Resolve a skin team (`team`).
fn team(game: &Q2GameServices, skin: &str) -> String {
    match skin.find('/') {
        None => skin.to_string(),
        Some(slash) => {
            if game.options.deathmatch_flags & 64 != 0 {
                skin[..slash].to_string()
            } else {
                skin[slash + 1..].to_string()
            }
        }
    }
}

/// Emit a high print.
fn print(context: &mut Q2PlayerContext, text: String) {
    let emit = player_hooks(context.game).emit;
    emit(Q2PlayerEvent::Print {
        target: Some(context.actor.clone()),
        level: Q2PrintLevel::High,
        text,
    });
}

/// Publish the inventory display (`publishInventory`).
fn publish_inventory(context: &mut Q2PlayerContext) {
    let actor = context.actor.clone();
    let entries = context.game.host.inventory().entries(&actor);
    let state = context.game.players.states[&actor].clone();
    let labels: Vec<super::types::Q2ItemLabel> = player_items(context.game)
        .list(context.game)
        .into_iter()
        .map(|item| super::types::Q2ItemLabel {
            item: item.id.clone(),
            name: item.name.clone(),
        })
        .collect();
    let emit = player_hooks(context.game).emit;
    emit(Q2PlayerEvent::Inventory {
        actor,
        entries,
        visible: Some(state.show_inventory),
        selected: Some(state.selected_item.clone()),
        labels: Some(labels),
    });
}

/// Selection filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Q2SelectFilter {
    /// All usable items.
    All,
    /// Weapons.
    Weapon,
    /// Powers.
    Power,
}

/// Select an item (`select`).
fn select(context: &mut Q2PlayerContext, direction: i32, filter: Q2SelectFilter) {
    let actor = context.actor.clone();
    let list = player_items(context.game).list(context.game);
    if list.is_empty() {
        context
            .game
            .players
            .states
            .get_mut(&actor)
            .expect("Q2 player has not been admitted")
            .selected_item = None;
        return;
    }
    let current = context.game.players.states[&actor].selected_item.clone();
    let first = list
        .iter()
        .position(|item| Some(&item.id) == current.as_ref())
        .map_or(-1, |index| index as i32);
    let len = list.len() as i32;
    for step in 1..=len {
        let item = &list[((first + direction * step + len * 2) % len) as usize];
        let held = context.game.host.inventory().count(&actor, &item.id) > 0.0;
        let wanted = filter == Q2SelectFilter::All
            || filter == Q2SelectFilter::Weapon && item.kind == crate::q2::foundation::items::Q2ItemKind::Weapon
            || filter == Q2SelectFilter::Power && item.kind == crate::q2::foundation::items::Q2ItemKind::Power;
        if item.usable && held && wanted {
            context
                .game
                .players
                .states
                .get_mut(&actor)
                .expect("Q2 player has not been admitted")
                .selected_item = Some(item.id.clone());
            return;
        }
    }
    context
        .game
        .players
        .states
        .get_mut(&actor)
        .expect("Q2 player has not been admitted")
        .selected_item = None;
}

/// Use an item (`use`).
fn use_item(context: &mut Q2PlayerContext, value: &str) {
    let actor = context.actor.clone();
    let item = player_items(context.game).lookup(context.game, value);
    let Some(item) = item else {
        print(context, format!("unknown item: {value}\n"));
        return;
    };
    if !item.usable {
        print(context, "Item is not usable.\n".to_string());
        return;
    }
    if context.game.host.inventory().count(&actor, &item.id) == 0.0 {
        print(context, format!("Out of item: {}\n", item.name));
        return;
    }
    let weapon = registered_weapon_definitions(context.game)
        .into_iter()
        .find(|weapon| weapon.item == item.id);
    if let Some(weapon) = weapon {
        let owned = context.game.owned_of(actor.clone());
        match request_weapon(context.game, &owned, &weapon.name, false) {
            Q2WeaponSelection::NoAmmo | Q2WeaponSelection::NotEnoughAmmo => {
                let ammo = weapon.ammo.as_deref().unwrap_or("ammo");
                print(context, format!("Not enough {ammo} for {}.\n", item.name));
                return;
            }
            _ => {}
        }
    } else {
        let owned = context.game.owned_of(actor.clone());
        player_items(context.game).use_inventory_item(&owned, &item.id, context.game, 30.0);
    }
    context
        .game
        .players
        .states
        .get_mut(&actor)
        .expect("Q2 player has not been admitted")
        .selected_item = Some(item.id);
}

/// Drop an item (`drop`).
fn drop_item(context: &mut Q2PlayerContext, value: &str) {
    let actor = context.actor.clone();
    let item = player_items(context.game).lookup(context.game, value);
    let Some(item) = item else {
        print(context, format!("unknown item: {value}\n"));
        return;
    };
    let can_drop = context
        .game
        .players
        .overrides
        .can_drop_coop_stay_items
        .map_or_else(can_drop_coop_stay_items, |can_drop| can_drop(context.game));
    if !item.droppable || context.game.options.mode == Q2Mode::Coop && item.stay_coop && !can_drop {
        print(context, "Item is not dropable.\n".to_string());
        return;
    }
    let count = context.game.host.inventory().count(&actor, &item.id);
    if count == 0.0 {
        print(context, format!("Out of item: {}\n", item.name));
        return;
    }
    let weapon = registered_weapon_definitions(context.game)
        .into_iter()
        .find(|weapon| weapon.item == item.id);
    if let Some(weapon) = weapon {
        let owned = context.game.owned_of(actor.clone());
        if !can_drop_weapon(context.game, &owned, &weapon.name) {
            print(context, "Can't drop current weapon\n".to_string());
            return;
        }
    }
    let grenade = context
        .game
        .weapons
        .states
        .get(&actor)
        .and_then(|state| state.weapon.clone())
        == Some("grenades".to_string())
        && item.id == "q2:ammo_grenades";
    let quantity = if item.kind == crate::q2::foundation::items::Q2ItemKind::Ammo {
        item.quantity.min(count)
    } else {
        1.0
    };
    if grenade && count - quantity <= 0.0 {
        print(context, "Can't drop current weapon\n".to_string());
        return;
    }
    let dropped = player_items(context.game).drop(
        actor.clone(),
        context.game,
        &item.id,
        &crate::q2::foundation::items::Q2DropOptions {
            immediate_touch: false,
            player_death: false,
            yaw_offset: None,
            expires_at: None,
        },
    );
    if let Some(dropped) = dropped {
        context.game.require_entity_mut(&dropped).count = quantity as i32;
        let owned = context.game.owned_of(actor);
        context.game.host.inventory().consume(&owned, &item.id, quantity);
    }
}

/// Cycle weapons (`weaponCycle`).
fn weapon_cycle(context: &mut Q2PlayerContext, direction: i32) {
    let actor = context.actor.clone();
    let current = context
        .game
        .weapons
        .states
        .get(&actor)
        .and_then(|state| state.weapon.clone());
    let definitions = registered_weapon_definitions(context.game);
    if definitions.is_empty() {
        return;
    }
    let index = definitions
        .iter()
        .position(|weapon| Some(&weapon.name) == current.as_ref())
        .map_or(-1, |index| index as i32);
    let len = definitions.len() as i32;
    let owned = context.game.owned_of(actor);
    for step in 1..=len {
        let weapon = &definitions[((index + direction * step + len * 2) % len) as usize];
        if request_weapon(context.game, &owned, &weapon.name, false) == Q2WeaponSelection::Selected {
            break;
        }
    }
}

/// Whether chat is allowed (`q2ChatAllowed`).
pub fn q2_chat_allowed(context: &mut Q2PlayerContext) -> bool {
    let now = context.game.host.now();
    let rules = context.game.players.rules.clone();
    if rules.flood_messages != 0 {
        let actor = context.actor.clone();
        let state = context.game.players.states[&actor].clone();
        if now < state.flood_lock_until {
            print(
                context,
                format!(
                    "You can't talk for {} more seconds\n",
                    (state.flood_lock_until - now).trunc() as i32
                ),
            );
            return false;
        }
        let back = i64::from(rules.flood_messages).min(10);
        let previous = if back <= 0 {
            None
        } else if state.flood_times.len() >= back as usize {
            state.flood_times.get(state.flood_times.len() - back as usize).copied()
        } else {
            state.flood_times.first().copied()
        };
        if state.flood_times.len() >= rules.flood_messages as usize
            && previous.is_some_and(|previous| now - previous < rules.flood_seconds)
        {
            context
                .game
                .players
                .states
                .get_mut(&actor)
                .expect("Q2 player has not been admitted")
                .flood_lock_until = now + rules.flood_wait_seconds;
            print(
                context,
                format!(
                    "Flood protection:  You can't talk for {} seconds.\n",
                    rules.flood_wait_seconds.trunc() as i32
                ),
            );
            return false;
        }
        let entry = context
            .game
            .players
            .states
            .get_mut(&actor)
            .expect("Q2 player has not been admitted");
        entry.flood_times.push(now);
        if entry.flood_times.len() > 10 {
            entry.flood_times.remove(0);
        }
    }
    true
}

/// Say command (`say`).
fn say(context: &mut Q2PlayerContext, args: &[String], team_only: bool) {
    if args.is_empty() || !q2_chat_allowed(context) {
        return;
    }
    let actor = context.actor.clone();
    let is_team = team_only && context.game.options.deathmatch_flags & (64 | 128) != 0;
    let mut words = args.join(" ");
    if words.starts_with('"') {
        words = if words.ends_with('"') && words.len() > 1 {
            words[1..words.len() - 1].to_string()
        } else {
            words[1..].to_string()
        };
    }
    let from = context.game.players.states[&actor].name.clone();
    let skin = context.game.players.states[&actor].skin.clone();
    let message: String = format!("{}: {words}", if is_team { format!("({from})") } else { from })
        .chars()
        .take(150)
        .collect::<String>()
        + "\n";
    let recipients: Vec<ActorId> = context
        .game
        .players
        .states
        .iter()
        .filter(|(_, other)| {
            other.connected && (!is_team || team(context.game, &other.skin) == team(context.game, &skin))
        })
        .map(|(other, _)| other.clone())
        .collect();
    let emit = player_hooks(context.game).emit;
    for recipient in recipients {
        emit(Q2PlayerEvent::Print {
            target: Some(recipient),
            level: Q2PrintLevel::Chat,
            text: message.clone(),
        });
    }
    if !context.game.players.states.contains_key(&actor) {
        panic!("Chat actor disconnected during synchronous command");
    }
}

/// Chat command (`say`/`say_team`).
fn chat_command(context: &mut Q2PlayerContext, args: &[String], command: &str) {
    say(context, args, command == "say_team");
}

/// List players command (`players`/`playerlist`).
fn list_players_command(context: &mut Q2PlayerContext, _args: &[String], command: &str) {
    let mut rows: Vec<(i32, i32, f64, String, bool, i32)> = context
        .game
        .players
        .states
        .values()
        .filter(|state| state.connected)
        .map(|state| {
            (
                state.score,
                state.slot,
                state.entered_at,
                state.name.clone(),
                state.spectator,
                state.ping,
            )
        })
        .collect();
    if command == "players" {
        rows.sort_by(|left, right| left.0.cmp(&right.0));
    } else {
        rows.sort_by(|left, right| left.1.cmp(&right.1));
    }
    let mut message = String::new();
    for (score, _slot, entered_at, name, spectator, ping) in &rows {
        let seconds = (context.game.host.now() - entered_at).trunc() as i32;
        let line = if command == "players" {
            format!("{score:>3} {name}\n")
        } else {
            format!(
                "{:02}:{:02} {ping:>4} {score:>3} {name}{}\n",
                seconds / 60,
                seconds % 60,
                if *spectator { " (spectator)" } else { "" }
            )
        };
        if message.len() + line.len() > 1280 {
            message.push_str("...\n");
            break;
        }
        message.push_str(&line);
    }
    if command == "players" {
        message.push_str(&format!("\n{} players\n", rows.len()));
    }
    print(context, message);
}

/// Score command (`score`).
fn score_command(context: &mut Q2PlayerContext, _args: &[String], _command: &str) {
    let actor = context.actor.clone();
    {
        let entry = context
            .game
            .players
            .states
            .get_mut(&actor)
            .expect("Q2 player has not been admitted");
        entry.show_inventory = false;
        entry.show_help = false;
        entry.show_scores = !entry.show_scores;
    }
    publish_inventory(context);
    if context.game.players.states[&actor].show_scores && context.game.options.mode != Q2Mode::Singleplayer {
        send_scoreboard(&actor, context.game, true);
    }
}

/// Help command (`help`).
fn help_command(context: &mut Q2PlayerContext, _args: &[String], _command: &str) {
    let actor = context.actor.clone();
    {
        let entry = context
            .game
            .players
            .states
            .get_mut(&actor)
            .expect("Q2 player has not been admitted");
        entry.show_inventory = false;
        entry.show_scores = false;
    }
    publish_inventory(context);
    if context.game.options.mode == Q2Mode::Deathmatch {
        context
            .game
            .players
            .states
            .get_mut(&actor)
            .expect("Q2 player has not been admitted")
            .show_scores = true;
        send_scoreboard(&actor, context.game, true);
    } else {
        let visible = !context.game.players.states[&actor].show_help;
        context
            .game
            .players
            .states
            .get_mut(&actor)
            .expect("Q2 player has not been admitted")
            .show_help = visible;
        let emit = player_hooks(context.game).emit;
        let actor = context.actor.clone();
        emit(Q2PlayerEvent::Help { actor, visible });
    }
}

/// Use command (`use`).
fn use_command(context: &mut Q2PlayerContext, args: &[String], _command: &str) {
    use_item(context, &args.join(" "));
}

/// Drop command (`drop`).
fn drop_command(context: &mut Q2PlayerContext, args: &[String], _command: &str) {
    drop_item(context, &args.join(" "));
}

/// Inventory command (`inven`).
fn inventory_command(context: &mut Q2PlayerContext, _args: &[String], _command: &str) {
    let actor = context.actor.clone();
    {
        let entry = context
            .game
            .players
            .states
            .get_mut(&actor)
            .expect("Q2 player has not been admitted");
        entry.show_scores = false;
        entry.show_help = false;
        entry.show_inventory = !entry.show_inventory;
    }
    publish_inventory(context);
}

/// Select command (`invnext` family).
fn select_command(context: &mut Q2PlayerContext, _args: &[String], command: &str) {
    let actor = context.actor.clone();
    if context.game.players.states[&actor].chase_target.is_some() {
        chase_player(actor, context.game, if command.starts_with("invnext") { 1 } else { -1 });
    } else {
        select(
            context,
            if command.starts_with("invnext") { 1 } else { -1 },
            if command.ends_with('w') {
                Q2SelectFilter::Weapon
            } else if command.ends_with('p') {
                Q2SelectFilter::Power
            } else {
                Q2SelectFilter::All
            },
        );
        if context.game.players.states[&actor].show_inventory {
            publish_inventory(context);
        }
    }
}

/// Selected-item command (`invuse`/`invdrop`).
fn selected_item_command(context: &mut Q2PlayerContext, _args: &[String], command: &str) {
    let actor = context.actor.clone();
    let selected = context.game.players.states[&actor].selected_item.clone();
    let empty = selected
        .as_ref()
        .is_none_or(|item| context.game.host.inventory().count(&actor, item) == 0.0);
    if empty {
        select(context, 1, Q2SelectFilter::All);
    }
    let selected = context.game.players.states[&actor].selected_item.clone();
    match selected {
        None => print(context, "No item to use.\n".to_string()),
        Some(selected) => {
            if command == "invuse" {
                use_item(context, &selected);
            } else {
                drop_item(context, &selected);
            }
        }
    }
}

/// Previous weapon command (`weapprev`).
fn previous_weapon_command(context: &mut Q2PlayerContext, _args: &[String], _command: &str) {
    weapon_cycle(context, 1);
}

/// Next weapon command (`weapnext`).
fn next_weapon_command(context: &mut Q2PlayerContext, _args: &[String], _command: &str) {
    weapon_cycle(context, -1);
}

/// Last weapon command (`weaplast`).
fn last_weapon_command(context: &mut Q2PlayerContext, _args: &[String], _command: &str) {
    let actor = context.actor.clone();
    let last = context
        .game
        .weapons
        .states
        .get(&actor)
        .and_then(|state| state.last_weapon.clone());
    if let Some(last) = last {
        let owned = context.game.owned_of(actor);
        request_weapon(context.game, &owned, &last, false);
    }
}

/// Kill command (`kill`).
fn kill_command(context: &mut Q2PlayerContext, _args: &[String], _command: &str) {
    let actor = context.actor.clone();
    let now = context.game.host.now();
    let forbidden = context.game.options.edition == Q2Edition::Rerelease
        && context.game.players.states[&actor].spectator
        || now - context.game.players.states[&actor].respawn_time < 5.0;
    if forbidden {
        return;
    }
    context
        .game
        .players
        .states
        .get_mut(&actor)
        .expect("Q2 player has not been admitted")
        .god = false;
    context.game.require_entity_mut(&actor).flags &= !16;
    let owned = context.game.owned_of(actor.clone());
    context.game.host.combat().set_traits(
        &owned,
        &CombatTraitChanges {
            can_take_damage: None,
            mass: None,
            invulnerable: Some(false),
            team: None,
            no_knockback: None,
        },
    );
    let health = context
        .game
        .host
        .combat()
        .read(&actor)
        .map_or(0.0, |combat| combat.health);
    environment_damage(actor, context.game, 1.0f64.max(health) + 1.0, 23, 32);
}

/// Put-away command (`putaway`).
fn put_away_command(context: &mut Q2PlayerContext, _args: &[String], _command: &str) {
    let actor = context.actor.clone();
    let entry = context
        .game
        .players
        .states
        .get_mut(&actor)
        .expect("Q2 player has not been admitted");
    entry.show_scores = false;
    entry.show_help = false;
    entry.show_inventory = false;
}

/// Gesture command (`wave`).
fn gesture_command(context: &mut Q2PlayerContext, args: &[String], _command: &str) {
    let actor = context.actor.clone();
    let movement = (player_hooks(context.game).movement)(actor.clone());
    if movement.ducked || context.game.players.states[&actor].animation_priority > 1 {
        return;
    }
    let wave = args.first().and_then(|argument| parse_command_prefix(argument));
    let animations: &[(&str, i32, i32)] = &[
        ("flipoff", 72, 83),
        ("salute", 84, 94),
        ("taunt", 95, 111),
        ("wave", 112, 122),
        ("point", 123, 134),
    ];
    let animation = wave
        .and_then(|wave| {
            if wave >= 0 {
                animations.get(wave as usize).copied()
            } else {
                None
            }
        })
        .unwrap_or(animations[4]);
    context
        .game
        .players
        .states
        .get_mut(&actor)
        .expect("Q2 player has not been admitted")
        .animation_priority = 1;
    context.game.require_entity_mut(&actor).frame = animation.1 - 1;
    context
        .game
        .players
        .states
        .get_mut(&actor)
        .expect("Q2 player has not been admitted")
        .animation_end = animation.2;
    print(context, format!("{}\n", animation.0));
}

/// Whether cheats are allowed (`q2CheatsAllowed`).
pub fn q2_cheats_allowed(context: &mut Q2PlayerContext) -> bool {
    let multiplayer = if context.game.options.edition == Q2Edition::Rerelease {
        context.game.options.max_clients > 1
    } else {
        context.game.options.mode == Q2Mode::Deathmatch
    };
    if multiplayer && !context.game.players.rules.cheats {
        print(
            context,
            "You must run the server with '+set cheats 1' to enable this command.\n".to_string(),
        );
        return false;
    }
    true
}

/// Cheat command (`god`/`notarget`/`noclip`/`target`/`give`).
fn cheat_command(context: &mut Q2PlayerContext, args: &[String], command: &str) {
    if !q2_cheats_allowed(context) {
        return;
    }
    let actor = context.actor.clone();
    match command {
        "god" => {
            let god = !context.game.players.states[&actor].god;
            context
                .game
                .players
                .states
                .get_mut(&actor)
                .expect("Q2 player has not been admitted")
                .god = god;
            context.game.require_entity_mut(&actor).flags ^= 16;
            let owned = context.game.owned_of(actor);
            context.game.host.combat().set_traits(
                &owned,
                &CombatTraitChanges {
                    can_take_damage: None,
                    mass: None,
                    invulnerable: Some(god),
                    team: None,
                    no_knockback: None,
                },
            );
            print(context, format!("godmode {}\n", if god { "ON" } else { "OFF" }));
        }
        "notarget" => {
            let notarget = !context.game.players.states[&actor].notarget;
            context
                .game
                .players
                .states
                .get_mut(&actor)
                .expect("Q2 player has not been admitted")
                .notarget = notarget;
            context.game.require_entity_mut(&actor).flags ^= 32;
            print(context, format!("notarget {}\n", if notarget { "ON" } else { "OFF" }));
        }
        "noclip" => {
            let noclip = !context.game.players.states[&actor].noclip;
            context
                .game
                .players
                .states
                .get_mut(&actor)
                .expect("Q2 player has not been admitted")
                .noclip = noclip;
            (player_hooks(context.game).set_movement)(
                actor,
                super::types::Q2PlayerMovementChange::Noclip { enabled: noclip },
            );
            print(context, format!("noclip {}\n", if noclip { "ON" } else { "OFF" }));
        }
        "target" => {
            let targets = context.game.targets(&args.join(" "));
            for target in targets {
                context
                    .game
                    .dispatch_use(target, Some(actor.clone()), Some(actor.clone()));
            }
        }
        _ => give_command(context, args),
    }
}

/// Give command (`give`).
fn give_command(context: &mut Q2PlayerContext, args: &[String]) {
    let actor = context.actor.clone();
    let requested = args.join(" ").to_lowercase();
    let all = requested == "all";
    let rerelease = context.game.options.edition == Q2Edition::Rerelease;
    if all || args.first().is_some_and(|first| first.to_lowercase() == "health") {
        let amount = if args.len() == 2 {
            parse_command_int(args.get(1).map(String::as_str)) as f64
        } else {
            context.game.require_entity(&actor).max_health
        };
        let owned = context.game.owned_of(actor.clone());
        context.game.host.combat().set_health(&owned, amount);
        if !all {
            return;
        }
    }
    if all || requested == "weapons" {
        let granted = player_hooks(context.game)
            .grant_selected_arsenal
            .is_some_and(|grant| grant(actor.clone(), super::types::Q2ArsenalCategory::Weapons));
        if !granted {
            let ids: Vec<String> = player_items(context.game)
                .list(context.game)
                .into_iter()
                .filter(|item| item.weapon && item.console_give != Q2ConsoleGive::InventoryOnly)
                .map(|item| item.id.clone())
                .collect();
            for id in ids {
                let count = context.game.host.inventory().count(&actor, &id);
                write_inventory(context, &id, count + 1.0);
            }
        }
        if !all {
            return;
        }
    }
    if all || requested == "ammo" {
        let granted = player_hooks(context.game)
            .grant_selected_arsenal
            .is_some_and(|grant| grant(actor.clone(), super::types::Q2ArsenalCategory::Ammo));
        if !granted {
            if all && rerelease {
                pickup_command(context, "item_pack");
            }
            let ids: Vec<String> = player_items(context.game)
                .list(context.game)
                .into_iter()
                .filter(|item| item.kind == crate::q2::foundation::items::Q2ItemKind::Ammo)
                .map(|item| item.id.clone())
                .collect();
            for id in ids {
                let entry = context
                    .game
                    .host
                    .inventory()
                    .entries(&actor)
                    .into_iter()
                    .find(|entry| entry.item == id);
                let capacity = player_items(context.game)
                    .lookup(context.game, &id)
                    .map_or(0.0, |found| found.capacity);
                write_inventory(
                    context,
                    &id,
                    (entry.as_ref().map_or(0.0, |entry| entry.count) + 1000.0)
                        .min(entry.map_or(capacity, |entry| entry.capacity)),
                );
            }
        }
        if !all {
            return;
        }
    }
    if all || requested == "armor" {
        let owned = context.game.owned_of(actor.clone());
        context.game.host.combat().set_regular_armor(
            &owned,
            &crate::contract::RegularArmorState::Q2 {
                points: 200.0,
                normal_protection: 0.8,
                energy_protection: 0.6,
                item: "q2:item_armor_body".to_string(),
            },
        );
        if !all {
            return;
        }
    }
    if all || !rerelease && requested == "power shield" {
        pickup_command(context, "item_power_shield");
        if !all {
            return;
        }
    }
    if all {
        let items: Vec<(String, crate::q2::foundation::items::Q2ItemKind, Q2ConsoleGive, String)> =
            player_items(context.game)
                .list(context.game)
                .into_iter()
                .map(|item| (item.id.clone(), item.kind, item.console_give, item.classname.clone()))
                .collect();
        for (id, kind, give, classname) in items {
            if matches!(
                kind,
                crate::q2::foundation::items::Q2ItemKind::Weapon
                    | crate::q2::foundation::items::Q2ItemKind::Ammo
                    | crate::q2::foundation::items::Q2ItemKind::Armor
                    | crate::q2::foundation::items::Q2ItemKind::Shard
            ) || give == Q2ConsoleGive::InventoryOnly
            {
                continue;
            }
            if rerelease
                && (give == Q2ConsoleGive::Forbidden
                    || give == Q2ConsoleGive::IndividualOnly
                    || kind == crate::q2::foundation::items::Q2ItemKind::Health
                    || kind == crate::q2::foundation::items::Q2ItemKind::MaximumHealth
                        && classname != "item_adrenaline")
            {
                continue;
            }
            write_inventory(
                context,
                &id,
                if rerelease && kind == crate::q2::foundation::items::Q2ItemKind::Key {
                    8.0
                } else {
                    1.0
                },
            );
        }
        if rerelease {
            context.game.require_entity_mut(&actor).power_cubes = 0xff;
            check_power_armor_after_give(context);
        }
        return;
    }
    let first = args.first().map_or("", String::as_str).to_lowercase();
    let catalog = player_items(context.game).list(context.game);
    let item = catalog
        .iter()
        .find(|item| item.name.to_lowercase() == requested)
        .or_else(|| catalog.iter().find(|item| item.name.to_lowercase() == first))
        .or_else(|| {
            if rerelease {
                catalog
                    .iter()
                    .find(|item| item.classname.to_lowercase() == first || item.id.to_lowercase() == first)
            } else {
                None
            }
        })
        .cloned();
    let Some(item) = item else {
        if player_hooks(context.game)
            .give_selected_item
            .is_some_and(|give| give(actor.clone(), args))
        {
            return;
        }
        print(context, "unknown item\n".to_string());
        return;
    };
    if rerelease && item.console_give == Q2ConsoleGive::Forbidden {
        print(context, "Item is not giveable.\n".to_string());
        return;
    }
    if (item.weapon || item.kind == crate::q2::foundation::items::Q2ItemKind::Ammo)
        && !player_items(context.game).maps_supply(context.game, &item.id)
        && player_hooks(context.game).give_selected_item.is_some_and(|give| {
            let mut grant = vec![item.id.clone()];
            if item.kind == crate::q2::foundation::items::Q2ItemKind::Ammo && args.len() == 2 {
                grant.push(args[1].clone());
            }
            give(actor.clone(), &grant)
        })
    {
        return;
    }
    if item.console_give == Q2ConsoleGive::InventoryOnly {
        if rerelease {
            write_inventory(context, &item.id.clone(), 1.0);
        } else {
            print(context, "non-pickup item\n".to_string());
        }
    } else if item.kind == crate::q2::foundation::items::Q2ItemKind::Ammo {
        let owned = context.game.owned_of(actor);
        let amount = if args.len() == 2 {
            Some(f64::from(parse_command_int(args.get(1).map(String::as_str))))
        } else {
            None
        };
        player_items(context.game).give_ammo_count(&owned, context.game, &item.id.clone(), amount);
    } else {
        let classname = item.classname.clone();
        pickup_command(context, &classname);
    }
}

/// Write inventory for give (`write`).
fn write_inventory(context: &mut Q2PlayerContext, item: &str, count: f64) {
    let actor = context.actor.clone();
    let entry = context
        .game
        .host
        .inventory()
        .entries(&actor)
        .into_iter()
        .find(|entry| entry.item == item);
    let capacity = entry.as_ref().map_or_else(
        || {
            player_items(context.game)
                .lookup(context.game, item)
                .map_or(0.0, |found| found.capacity)
        },
        |entry| entry.capacity,
    );
    let owned = context.game.owned_of(actor);
    context.game.host.inventory().configure(
        &owned,
        &InventoryEntry {
            item: item.to_string(),
            count,
            capacity,
            count_policy: Some(InventoryCountPolicy::SourceCounter(SourceCounterArithmetic::Int32)),
        },
    );
}

/// Spawn a pickup for give (`pickup`).
fn pickup_command(context: &mut Q2PlayerContext, classname: &str) {
    let actor = context.actor.clone();
    let temporary = context.game.create(classname, BTreeMap::new());
    if !player_items(context.game).spawn(temporary.clone(), context.game)
        || !context.game.host.actors().is_live(&temporary)
    {
        return;
    }
    context.game.cancel_actor(temporary.clone());
    player_items(context.game).touch(temporary.clone(), context.game, actor);
    if context.game.host.actors().is_live(&temporary) {
        context.game.remove_actor(temporary);
    }
}

/// Restore power armor selection after give (`checkPowerArmorAfterGive`).
fn check_power_armor_after_give(context: &mut Q2PlayerContext) {
    let actor = context.actor.clone();
    let userinfo = context.game.players.states[&actor].userinfo.clone();
    let fields: Vec<&str> = userinfo.split('\\').collect();
    let mut automatic = -1;
    let mut index = 1;
    while index < fields.len() {
        if fields[index] == "autoshield" {
            automatic = parse_command_int(fields.get(index + 1).copied());
        }
        index += 2;
    }
    let cells = context
        .game
        .host
        .inventory()
        .count(&actor, &"q2:ammo_cells".to_string());
    let flags = context.game.require_entity(&actor).flags;
    let enough = cells != 0.0 && (automatic < 0 || flags & 0x40000000 != 0 && cells > automatic as f64);
    let active = context
        .game
        .host
        .combat()
        .read(&actor)
        .is_some_and(|combat| !matches!(combat.armor.powered, crate::contract::PoweredProtectionState::None));
    let shield = if context
        .game
        .host
        .inventory()
        .count(&actor, &"q2:item_power_shield".to_string())
        != 0.0
    {
        "q2:item_power_shield"
    } else {
        "q2:item_power_screen"
    };
    if active && !enough || !active && automatic != -1 && enough {
        let owned = context.game.owned_of(actor);
        player_items(context.game).use_inventory_item(&owned, shield, context.game, 30.0);
    }
}

/// Score rows snapshot.
pub fn score_rows(game: &Q2GameServices) -> Vec<Q2ScoreRow> {
    let now = game.host.now();
    let mut rows: Vec<(i32, i32, f64, String, bool, i32)> = game
        .players
        .states
        .values()
        .filter(|state| state.connected && !state.spectator)
        .map(|state| {
            (
                state.score,
                state.slot,
                state.entered_at,
                state.name.clone(),
                state.spectator,
                state.ping,
            )
        })
        .collect();
    rows.sort_by(|left, right| right.0.cmp(&left.0).then(left.1.cmp(&right.1)));
    rows.into_iter()
        .take(12)
        .map(|(score, slot, entered_at, name, spectator, ping)| Q2ScoreRow {
            slot,
            name,
            score,
            ping: ping.min(999),
            minutes: ((now - entered_at) / 60.0).trunc() as i32,
            spectator,
        })
        .collect()
}

/// Client command table (`q2ClientCommands`).
pub const Q2_CLIENT_COMMANDS: &[Q2ClientCommandDefinition] = &[
    Q2ClientCommandDefinition {
        name: "say",
        documentation: Q2CommandDocumentation {
            summary: "Send a chat message.",
            usage: "say <message>",
            examples: &[],
        },
        intermission: true,
        run: chat_command,
    },
    Q2ClientCommandDefinition {
        name: "say_team",
        documentation: Q2CommandDocumentation {
            summary: "Send chat to your team when model or skin teams are enabled.",
            usage: "say_team <message>",
            examples: &[],
        },
        intermission: true,
        run: chat_command,
    },
    Q2ClientCommandDefinition {
        name: "players",
        documentation: Q2CommandDocumentation {
            summary: "List connected players sorted by score.",
            usage: "players",
            examples: &[],
        },
        intermission: true,
        run: list_players_command,
    },
    Q2ClientCommandDefinition {
        name: "playerlist",
        documentation: Q2CommandDocumentation {
            summary: "List connected players with time, ping, score, and spectator status.",
            usage: "playerlist",
            examples: &[],
        },
        intermission: true,
        run: list_players_command,
    },
    Q2ClientCommandDefinition {
        name: "score",
        documentation: Q2CommandDocumentation {
            summary: "Toggle the scoreboard.",
            usage: "score",
            examples: &[],
        },
        intermission: true,
        run: score_command,
    },
    Q2ClientCommandDefinition {
        name: "help",
        documentation: Q2CommandDocumentation {
            summary: "Toggle mission help, or show the deathmatch scoreboard.",
            usage: "help",
            examples: &[],
        },
        intermission: true,
        run: help_command,
    },
    Q2ClientCommandDefinition {
        name: "use",
        documentation: Q2CommandDocumentation {
            summary: "Use an inventory item or select a weapon by name.",
            usage: "use <item name>",
            examples: &[],
        },
        intermission: false,
        run: use_command,
    },
    Q2ClientCommandDefinition {
        name: "drop",
        documentation: Q2CommandDocumentation {
            summary: "Drop an inventory item by name.",
            usage: "drop <item name>",
            examples: &[],
        },
        intermission: false,
        run: drop_command,
    },
    Q2ClientCommandDefinition {
        name: "inven",
        documentation: Q2CommandDocumentation {
            summary: "Toggle the inventory display.",
            usage: "inven",
            examples: &[],
        },
        intermission: false,
        run: inventory_command,
    },
    Q2ClientCommandDefinition {
        name: "invnext",
        documentation: Q2CommandDocumentation {
            summary: "Select the next usable item, or cycle chase targets.",
            usage: "invnext",
            examples: &[],
        },
        intermission: false,
        run: select_command,
    },
    Q2ClientCommandDefinition {
        name: "invprev",
        documentation: Q2CommandDocumentation {
            summary: "Select the previous usable item, or cycle chase targets.",
            usage: "invprev",
            examples: &[],
        },
        intermission: false,
        run: select_command,
    },
    Q2ClientCommandDefinition {
        name: "invnextw",
        documentation: Q2CommandDocumentation {
            summary: "Select the next weapon, or cycle chase targets.",
            usage: "invnextw",
            examples: &[],
        },
        intermission: false,
        run: select_command,
    },
    Q2ClientCommandDefinition {
        name: "invprevw",
        documentation: Q2CommandDocumentation {
            summary: "Select the previous weapon, or cycle chase targets.",
            usage: "invprevw",
            examples: &[],
        },
        intermission: false,
        run: select_command,
    },
    Q2ClientCommandDefinition {
        name: "invnextp",
        documentation: Q2CommandDocumentation {
            summary: "Select the next powerup, or cycle chase targets.",
            usage: "invnextp",
            examples: &[],
        },
        intermission: false,
        run: select_command,
    },
    Q2ClientCommandDefinition {
        name: "invprevp",
        documentation: Q2CommandDocumentation {
            summary: "Select the previous powerup, or cycle chase targets.",
            usage: "invprevp",
            examples: &[],
        },
        intermission: false,
        run: select_command,
    },
    Q2ClientCommandDefinition {
        name: "invuse",
        documentation: Q2CommandDocumentation {
            summary: "Use the selected inventory item.",
            usage: "invuse",
            examples: &[],
        },
        intermission: false,
        run: selected_item_command,
    },
    Q2ClientCommandDefinition {
        name: "invdrop",
        documentation: Q2CommandDocumentation {
            summary: "Drop the selected inventory item.",
            usage: "invdrop",
            examples: &[],
        },
        intermission: false,
        run: selected_item_command,
    },
    Q2ClientCommandDefinition {
        name: "weapprev",
        documentation: Q2CommandDocumentation {
            summary: "Select the previous available weapon.",
            usage: "weapprev",
            examples: &[],
        },
        intermission: false,
        run: previous_weapon_command,
    },
    Q2ClientCommandDefinition {
        name: "weapnext",
        documentation: Q2CommandDocumentation {
            summary: "Select the next available weapon.",
            usage: "weapnext",
            examples: &[],
        },
        intermission: false,
        run: next_weapon_command,
    },
    Q2ClientCommandDefinition {
        name: "weaplast",
        documentation: Q2CommandDocumentation {
            summary: "Select the last weapon used.",
            usage: "weaplast",
            examples: &[],
        },
        intermission: false,
        run: last_weapon_command,
    },
    Q2ClientCommandDefinition {
        name: "kill",
        documentation: Q2CommandDocumentation {
            summary: "Kill yourself after the five-second respawn delay.",
            usage: "kill",
            examples: &[],
        },
        intermission: false,
        run: kill_command,
    },
    Q2ClientCommandDefinition {
        name: "putaway",
        documentation: Q2CommandDocumentation {
            summary: "Close score, help, and inventory displays.",
            usage: "putaway",
            examples: &[],
        },
        intermission: false,
        run: put_away_command,
    },
    Q2ClientCommandDefinition {
        name: "wave",
        documentation: Q2CommandDocumentation {
            summary: "Play a gesture: 0 flipoff, 1 salute, 2 taunt, 3 wave, 4 point.",
            usage: "wave [0-4]",
            examples: &[],
        },
        intermission: false,
        run: gesture_command,
    },
    Q2ClientCommandDefinition {
        name: "god",
        documentation: Q2CommandDocumentation {
            summary: "Toggle invulnerability; deathmatch requires cheats.",
            usage: "god",
            examples: &[],
        },
        intermission: false,
        run: cheat_command,
    },
    Q2ClientCommandDefinition {
        name: "notarget",
        documentation: Q2CommandDocumentation {
            summary: "Toggle monster targeting immunity; deathmatch requires cheats.",
            usage: "notarget",
            examples: &[],
        },
        intermission: false,
        run: cheat_command,
    },
    Q2ClientCommandDefinition {
        name: "noclip",
        documentation: Q2CommandDocumentation {
            summary: "Toggle movement through walls; deathmatch requires cheats.",
            usage: "noclip",
            examples: &[],
        },
        intermission: false,
        run: cheat_command,
    },
    Q2ClientCommandDefinition {
        name: "give",
        documentation: Q2CommandDocumentation {
            summary: "Give items, health, weapons, ammo, or armor; deathmatch requires cheats.",
            usage: "give <all|health [amount]|weapons|ammo|armor|item name [amount]>",
            examples: &[],
        },
        intermission: false,
        run: cheat_command,
    },
    Q2ClientCommandDefinition {
        name: "target",
        documentation: Q2CommandDocumentation {
            summary: "Activate entities with the supplied target name; deathmatch requires cheats.",
            usage: "target <targetname>",
            examples: &[],
        },
        intermission: false,
        run: cheat_command,
    },
];

/// Run a client command (`runQ2ClientCommand`).
pub fn run_q2_client_command(context: &mut Q2PlayerContext, source_command: &str, args: &[String]) -> bool {
    let command = source_command.to_lowercase();
    let actor = context.actor.clone();
    if player_hooks(context.game)
        .command
        .is_some_and(|command_hook| command_hook(actor.clone(), context.game, &command, args))
    {
        return true;
    }
    let definition = Q2_CLIENT_COMMANDS.iter().find(|definition| definition.name == command);
    if !matches!(context.game.players.intermission, Q2Intermission::Playing)
        && !definition.is_some_and(|definition| definition.intermission)
    {
        return true;
    }
    match definition {
        None => {
            let mut said = vec![source_command.to_string()];
            said.extend(args.iter().cloned());
            say(context, &said, false);
        }
        Some(definition) => (definition.run)(context, args, &command),
    }
    true
}
