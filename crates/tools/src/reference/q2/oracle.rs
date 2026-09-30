//! Q2 source-derived oracle (donor `tools/reference/q2/oracle.ts`).
//!
//! Evaluates the Q2 transcriptions: frame clocks, think scheduling, frame
//! order, ammo pickup, armor absorption, cross-unit flags, save-field
//! projection, flechette defaults, Q64 configuration, and command
//! predicates. Inputs validate like the Q1 oracle (known kinds, exact
//! fields), while numeric behavior follows the donor exactly: binary32
//! stores are `as f32` round-trips, bit operations apply JavaScript
//! `ToInt32`/`ToUint32` coercion, and `Math.min` propagates NaN.

use std::collections::{HashMap, HashSet};

use crate::error::ToolsError;
use crate::json::{render_number, Json};
use crate::reference::q2::sources::source_text;

/// Think-scheduling family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThinkFamily {
    /// Classic binary32-second scheduling with a 1 ms lookahead.
    Classic,
    /// Rerelease integer-millisecond scheduling without lookahead.
    Rerelease,
}

/// Armor kind selecting a protection row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArmorKind {
    /// Jacket armor.
    Jacket,
    /// Combat armor.
    Combat,
    /// Body armor.
    Body,
}

/// Think-scheduling input (donor `think` argument).
#[derive(Debug, Clone, Copy)]
pub struct ThinkInput {
    /// Scheduling family.
    pub family: ThinkFamily,
    /// Current time.
    pub now: f64,
    /// Scheduled think time.
    pub nextthink: f64,
    /// Reschedule written by the callback.
    pub reschedule: f64,
}

/// Command-predicate input.
#[derive(Debug, Clone, Copy)]
pub struct CommandPredicatesInput {
    /// Classic vertical command.
    pub upmove: f64,
    /// Rerelease button byte.
    pub buttons: f64,
    /// Whether Q64 physics are enabled.
    pub n64_physics: bool,
}

/// Oracle input.
#[derive(Debug, Clone)]
pub enum Q2Input {
    /// Classic and rerelease frame clocks.
    Clock {
        /// Classic frames to convert to binary32 seconds.
        classic_frames: Vec<f64>,
        /// Rerelease millisecond step.
        rerelease_step_ms: f64,
        /// Rerelease frame count.
        rerelease_frames: f64,
    },
    /// One think-scheduling decision.
    Think(ThinkInput),
    /// Classic frame-actor order with authored mutations.
    FrameOrder {
        /// Whether slot 2 deletes slot 3.
        delete_later_actor: bool,
        /// Whether slot 2 appends slot 4.
        spawn_actor: bool,
    },
    /// One ammo pickup through `Touch_Item`.
    Pickup {
        /// Starting inventory.
        inventory: f64,
        /// Pickup capacity.
        capacity: f64,
        /// Pickup quantity.
        quantity: f64,
        /// Whether targets were already used.
        targets_used: bool,
    },
    /// One armor absorption through `CheckArmor`.
    Armor {
        /// Incoming damage.
        damage: f64,
        /// Remaining armor inventory.
        inventory: f64,
        /// Armor kind.
        armor: ArmorKind,
        /// Whether the damage is energy damage.
        energy: bool,
        /// Whether armor is bypassed (`DAMAGE_NO_ARMOR`).
        bypass: bool,
    },
    /// Cross-unit trigger/target flag evaluation.
    CrossUnit {
        /// Current unit flags.
        flags: f64,
        /// Trigger bits.
        trigger: f64,
        /// Required bits.
        required: f64,
    },
    /// Save-field projection for one native struct.
    SaveFields {
        /// Native struct name (validated against source, not here).
        save_struct: String,
        /// Candidate fields in input order.
        fields: Vec<(String, Json)>,
    },
    /// Flechette default extracted from raw source.
    FlechetteDefault,
    /// Q64 worldspawn configuration block.
    Q64Config {
        /// Whether the build is N64.
        is_n64: bool,
        /// Whether the game is deathmatch.
        deathmatch: bool,
        /// Server physics state before the block.
        initial_n64_physics: bool,
        /// Air acceleration cvar.
        airacceleration: f64,
    },
    /// Native command predicates.
    CommandPredicates(CommandPredicatesInput),
}

fn object(value: &Json) -> Result<&[(String, Json)], ToolsError> {
    value.as_object().ok_or_else(|| ToolsError::parse("Expected an object"))
}

fn fields(value: &[(String, Json)], expected: &[&str]) -> Result<(), ToolsError> {
    for (key, _) in value {
        if !expected.contains(&key.as_str()) {
            return Err(ToolsError::parse(format!("Unexpected field {key}")));
        }
    }
    for key in expected {
        if !value.iter().any(|(present, _)| present == key) {
            return Err(ToolsError::parse(format!("Missing field {key}")));
        }
    }
    Ok(())
}

fn field<'a>(value: &'a [(String, Json)], key: &str) -> &'a Json {
    value.iter().find(|(present, _)| present == key).map(|(_, found)| found).expect("checked field")
}

fn kind_of(value: &[(String, Json)]) -> Option<&str> {
    value.iter().find(|(present, _)| present == "kind").and_then(|(_, found)| found.as_str())
}

fn number(value: &Json) -> Result<f64, ToolsError> {
    value.as_f64().ok_or_else(|| ToolsError::parse("Expected a number"))
}

fn boolean(value: &Json) -> Result<bool, ToolsError> {
    value.as_bool().ok_or_else(|| ToolsError::parse("Expected a boolean"))
}

fn text(value: &Json) -> Result<&str, ToolsError> {
    value.as_str().ok_or_else(|| ToolsError::parse("Expected a string"))
}

/// Binary32 store: `Math.fround` round-to-nearest ties-to-even.
fn fround(value: f64) -> f64 {
    f64::from(value as f32)
}

/// JavaScript `ToInt32` coercion for bit operations.
fn to_int32(value: f64) -> i32 {
    if !value.is_finite() {
        return 0;
    }
    ((value.trunc() % 4_294_967_296.0) as i64) as u32 as i32
}

/// JavaScript `ToUint32` coercion for bit operations.
fn to_uint32(value: f64) -> u32 {
    if !value.is_finite() {
        return 0;
    }
    ((value.trunc() % 4_294_967_296.0) as i64) as u32
}

/// JavaScript `Math.min`: NaN propagates (unlike `f64::min`).
fn js_min(left: f64, right: f64) -> f64 {
    if left.is_nan() || right.is_nan() {
        f64::NAN
    } else {
        left.min(right)
    }
}

/// Parse and validate an oracle input document.
pub fn parse_q2_input(value: &Json) -> Result<Q2Input, ToolsError> {
    let item = object(value)?;
    match kind_of(item) {
        Some("clock") => {
            fields(item, &["kind", "classicFrames", "rereleaseStepMs", "rereleaseFrames"])?;
            let raw = field(item, "classicFrames")
                .as_array()
                .ok_or_else(|| ToolsError::parse("Expected classic frames array"))?;
            let mut classic_frames = Vec::with_capacity(raw.len());
            for entry in raw {
                classic_frames.push(number(entry)?);
            }
            Ok(Q2Input::Clock {
                classic_frames,
                rerelease_step_ms: number(field(item, "rereleaseStepMs"))?,
                rerelease_frames: number(field(item, "rereleaseFrames"))?,
            })
        }
        Some("think") => {
            fields(item, &["kind", "family", "now", "nextthink", "reschedule"])?;
            let family = match text(field(item, "family"))? {
                "classic" => ThinkFamily::Classic,
                "rerelease" => ThinkFamily::Rerelease,
                _ => return Err(ToolsError::parse("Unknown think family")),
            };
            Ok(Q2Input::Think(ThinkInput {
                family,
                now: number(field(item, "now"))?,
                nextthink: number(field(item, "nextthink"))?,
                reschedule: number(field(item, "reschedule"))?,
            }))
        }
        Some("frame-order") => {
            fields(item, &["kind", "deleteLaterActor", "spawnActor"])?;
            Ok(Q2Input::FrameOrder {
                delete_later_actor: boolean(field(item, "deleteLaterActor"))?,
                spawn_actor: boolean(field(item, "spawnActor"))?,
            })
        }
        Some("pickup") => {
            fields(item, &["kind", "inventory", "capacity", "quantity", "targetsUsed"])?;
            Ok(Q2Input::Pickup {
                inventory: number(field(item, "inventory"))?,
                capacity: number(field(item, "capacity"))?,
                quantity: number(field(item, "quantity"))?,
                targets_used: boolean(field(item, "targetsUsed"))?,
            })
        }
        Some("armor") => {
            fields(item, &["kind", "damage", "inventory", "armor", "energy", "bypass"])?;
            let armor = match text(field(item, "armor"))? {
                "jacket" => ArmorKind::Jacket,
                "combat" => ArmorKind::Combat,
                "body" => ArmorKind::Body,
                _ => return Err(ToolsError::parse("Unknown armor kind")),
            };
            Ok(Q2Input::Armor {
                damage: number(field(item, "damage"))?,
                inventory: number(field(item, "inventory"))?,
                armor,
                energy: boolean(field(item, "energy"))?,
                bypass: boolean(field(item, "bypass"))?,
            })
        }
        Some("cross-unit") => {
            fields(item, &["kind", "flags", "trigger", "required"])?;
            Ok(Q2Input::CrossUnit {
                flags: number(field(item, "flags"))?,
                trigger: number(field(item, "trigger"))?,
                required: number(field(item, "required"))?,
            })
        }
        Some("save-fields") => {
            fields(item, &["kind", "struct", "fields"])?;
            let raw = object(field(item, "fields"))?;
            Ok(Q2Input::SaveFields { save_struct: text(field(item, "struct"))?.to_owned(), fields: raw.to_vec() })
        }
        Some("flechette-default") => {
            fields(item, &["kind"])?;
            Ok(Q2Input::FlechetteDefault)
        }
        Some("q64-config") => {
            fields(item, &["kind", "isN64", "deathmatch", "initialN64Physics", "airacceleration"])?;
            Ok(Q2Input::Q64Config {
                is_n64: boolean(field(item, "isN64"))?,
                deathmatch: boolean(field(item, "deathmatch"))?,
                initial_n64_physics: boolean(field(item, "initialN64Physics"))?,
                airacceleration: number(field(item, "airacceleration"))?,
            })
        }
        Some("command-predicates") => {
            fields(item, &["kind", "upmove", "buttons", "n64Physics"])?;
            Ok(Q2Input::CommandPredicates(CommandPredicatesInput {
                upmove: number(field(item, "upmove"))?,
                buttons: number(field(item, "buttons"))?,
                n64_physics: boolean(field(item, "n64Physics"))?,
            }))
        }
        _ => Err(ToolsError::parse("Unknown Q2 oracle input kind")),
    }
}

/// Classic frame time: `FRAMETIME` is an unsuffixed double literal while
/// `level.time` is a float store.
#[must_use]
pub fn classic_time(frame: f64) -> f64 {
    fround(frame * 0.1)
}

fn event(name: &str, pairs: Vec<(&str, Json)>) -> Json {
    let mut entries = vec![("event".to_owned(), Json::string(name))];
    entries.extend(pairs.into_iter().map(|(key, value)| (key.to_owned(), value)));
    Json::object(entries)
}

/// Evaluate one think-scheduling decision.
#[must_use]
pub fn think(input: &ThinkInput) -> Json {
    let classic = matches!(input.family, ThinkFamily::Classic);
    let now = if classic { fround(input.now) } else { input.now };
    let mut nextthink = if classic { fround(input.nextthink) } else { input.nextthink };
    let mut trace = Vec::new();
    let deadline = if classic { now + 0.001 } else { now };
    let may_continue_physics = nextthink <= 0.0 || nextthink > deadline;
    if !may_continue_physics {
        trace.push(event("think.enter", vec![("nextthink", Json::float(0.0))]));
        nextthink = if classic { fround(input.reschedule) } else { input.reschedule };
        trace.push(event("think.return", vec![("nextthink", Json::float(nextthink))]));
    }
    Json::object(vec![
        ("now".to_owned(), Json::float(now)),
        ("nextthink".to_owned(), Json::float(nextthink)),
        ("mayContinuePhysics".to_owned(), Json::boolean(may_continue_physics)),
        ("trace".to_owned(), Json::array(trace)),
    ])
}

fn frame_order(delete_later_actor: bool, spawn_actor: bool) -> Json {
    let mut trace: Vec<Json> =
        ["level.framenum++", "level.time=frame*0.1", "AI_SetSightClient"].into_iter().map(Json::string).collect();
    let mut actors = vec![0, 1, 2, 3];
    let mut active: HashSet<i64> = actors.iter().copied().collect();
    let mut index = 0;
    while index < actors.len() {
        let actor = actors[index];
        index += 1;
        if !active.contains(&actor) {
            continue;
        }
        trace.push(event("current_entity+old_origin", vec![("actor", Json::int(actor))]));
        if actor == 1 {
            trace.push(event("ClientBeginServerFrame", vec![("actor", Json::int(actor))]));
            continue;
        }
        trace.push(event("G_RunEntity.enter", vec![("actor", Json::int(actor))]));
        if actor == 2 {
            if delete_later_actor {
                active.remove(&3);
                trace.push(event("authored.delete", vec![("actor", Json::int(3))]));
            }
            if spawn_actor {
                actors.push(4);
                active.insert(4);
                trace.push(event("authored.append", vec![("actor", Json::int(4))]));
            }
        }
        trace.push(event("G_RunEntity.return", vec![("actor", Json::int(actor))]));
    }
    trace.push(Json::string("CheckDMRules"));
    trace.push(Json::string("ClientEndServerFrames"));
    Json::object(vec![("trace".to_owned(), Json::array(trace))])
}

fn pickup(inventory: f64, capacity: f64, quantity: f64, targets_used: bool) -> Json {
    let mut current = inventory;
    let mut trace = Vec::new();
    trace.push(event("Pickup_Ammo.enter", vec![("inventory", Json::float(current))]));
    let taken = if current == capacity {
        trace.push(event(
            "Pickup_Ammo.return",
            vec![("taken", Json::boolean(false)), ("inventory", Json::float(current))],
        ));
        false
    } else {
        current = js_min(current + quantity, capacity);
        trace.push(event(
            "Pickup_Ammo.return",
            vec![("taken", Json::boolean(true)), ("inventory", Json::float(current))],
        ));
        true
    };
    trace.push(event("Touch_Item.observes-return", vec![("taken", Json::boolean(taken))]));
    if taken {
        trace.push(Json::string("pickup.feedback"));
    }
    let mut used = targets_used;
    if !used {
        trace.push(event(
            "G_UseTargets.enter",
            vec![("inventory", Json::float(current)), ("targetsUsed", Json::boolean(targets_used))],
        ));
        trace.push(Json::string("G_UseTargets.return"));
        used = true;
        trace.push(Json::string("ITEM_TARGETS_USED=set"));
    }
    let freed = if taken {
        trace.push(Json::string("G_FreeEdict"));
        true
    } else {
        false
    };
    Json::object(vec![
        ("taken".to_owned(), Json::boolean(taken)),
        ("targetsUsed".to_owned(), Json::boolean(used)),
        ("freed".to_owned(), Json::boolean(freed)),
        ("inventory".to_owned(), Json::float(current)),
        ("trace".to_owned(), Json::array(trace)),
    ])
}

fn armor(damage: f64, inventory: f64, armor: ArmorKind, energy: bool, bypass: bool) -> Json {
    let (normal, energy_protection) = match armor {
        ArmorKind::Jacket => (0.3, 0.0),
        ArmorKind::Combat => (0.6, 0.3),
        ArmorKind::Body => (0.8, 0.6),
    };
    let protection = fround(if energy { energy_protection } else { normal });
    let absorbed = if bypass { 0.0 } else { js_min(fround(protection * damage).ceil(), inventory) };
    Json::object(vec![
        ("absorbed".to_owned(), Json::float(absorbed)),
        ("remainingArmor".to_owned(), Json::float(inventory - absorbed)),
        ("remainingDamage".to_owned(), Json::float(damage - absorbed)),
        (
            "effect".to_owned(),
            if absorbed == 0.0 { Json::Null } else { Json::string("SpawnDamage") },
        ),
    ])
}

/// Declared `FIELD_AUTO` names for one save struct (donor
/// `/FIELD_AUTO\(\s*([^()]+?)\s*\)/g` over the `DECLARE_SAVE_STRUCT` span).
pub fn declared_save_fields(text: &str, save_struct: &str) -> Result<Vec<String>, ToolsError> {
    let marker = format!("#define DECLARE_SAVE_STRUCT {save_struct}\n");
    let start = text.find(&marker).ok_or_else(|| ToolsError::invalid(format!("Q2 save structure missing: {save_struct}")))?;
    let end = text[start..]
        .find("#undef DECLARE_SAVE_STRUCT")
        .map(|offset| start + offset)
        .ok_or_else(|| ToolsError::invalid(format!("Q2 save structure unterminated: {save_struct}")))?;
    let span = &text[start..end];
    let mut names = Vec::new();
    let mut cursor = 0;
    while let Some(found) = span[cursor..].find("FIELD_AUTO(") {
        let inner_start = cursor + found + "FIELD_AUTO(".len();
        let rest = &span[inner_start..];
        let close = rest.find(')');
        let open = rest.find('(');
        match close {
            Some(end) if open.is_none_or(|index| index > end) => {
                let name = rest[..end].trim();
                if name.is_empty() {
                    cursor = inner_start;
                } else {
                    names.push(name.to_owned());
                    cursor = inner_start + end + 1;
                }
            }
            _ => {
                cursor = inner_start;
            }
        }
    }
    Ok(names)
}

fn save_fields(save_struct: &str, fields: &[(String, Json)], sources: &HashMap<String, String>) -> Result<Json, ToolsError> {
    let text = source_text(sources, "rereleaseSave")?;
    let selected: HashSet<String> = declared_save_fields(text, save_struct)?.into_iter().collect();
    let mut retained = Vec::new();
    let mut omitted = Vec::new();
    for (name, value) in fields {
        if selected.contains(name) {
            retained.push((name.clone(), value.clone()));
        } else {
            omitted.push(Json::string(name));
        }
    }
    Ok(Json::object(vec![
        ("retained".to_owned(), Json::object(retained)),
        ("omitted".to_owned(), Json::array(omitted)),
    ]))
}

fn word_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// `AMMO_*` enumerator names inside the `ammo_t` definition (donor
/// `/\bAMMO_[A-Z]+\b/g` over the enum body).
fn ammo_names(shared: &str) -> Result<Vec<String>, ToolsError> {
    let marker = "enum ammo_t : uint8_t";
    let start = shared.find(marker).ok_or_else(|| ToolsError::invalid("Q2 ammo_t definition missing"))?;
    let after = &shared[start + marker.len()..];
    let open = after.find('{').ok_or_else(|| ToolsError::invalid("Q2 ammo_t definition missing"))?;
    let body_start = start + marker.len() + open + 1;
    let close = shared[body_start..].find('}').ok_or_else(|| ToolsError::invalid("Q2 ammo_t definition missing"))?;
    let body = &shared[body_start..body_start + close];
    let bytes = body.as_bytes();
    let mut names = Vec::new();
    let mut index = 0;
    while index + "AMMO_".len() <= bytes.len() {
        if body[index..].starts_with("AMMO_")
            && (index == 0 || !word_char(bytes[index - 1]))
            && bytes[index + "AMMO_".len()].is_ascii_uppercase()
        {
            let mut end = index + "AMMO_".len() + 1;
            while end < bytes.len() && bytes[end].is_ascii_uppercase() {
                end += 1;
            }
            if end >= bytes.len() || !word_char(bytes[end]) {
                names.push(body[index..end].to_owned());
            }
            index = end;
        } else {
            index += 1;
        }
    }
    if names.is_empty() {
        return Err(ToolsError::invalid("Q2 ammo_t definition missing"));
    }
    Ok(names)
}

fn flechette_default(sources: &HashMap<String, String>) -> Result<Json, ToolsError> {
    let names = ammo_names(source_text(sources, "rereleaseShared")?)?;
    let index = names.iter().position(|name| name == "AMMO_FLECHETTES");
    let count = names.iter().position(|name| name == "AMMO_MAX");
    let source = source_text(sources, "rereleaseClient")?;
    let assignment = source
        .find("max_ammo[AMMO_FLECHETTES] = ")
        .and_then(|found| {
            let digits: String = source[found + "max_ammo[AMMO_FLECHETTES] = ".len()..]
                .chars()
                .take_while(|ch| ch.is_ascii_digit())
                .collect();
            let after = &source[found + "max_ammo[AMMO_FLECHETTES] = ".len() + digits.len()..];
            if !digits.is_empty() && after.starts_with(';') {
                digits.parse::<f64>().ok()
            } else {
                None
            }
        });
    match (index, count, assignment) {
        (Some(index), Some(count), Some(assignment)) => Ok(Json::object(vec![
            ("arrayIndex".to_owned(), Json::uint(index as u64)),
            ("arrayLength".to_owned(), Json::uint(count as u64)),
            ("defaultCapacity".to_owned(), Json::float(assignment)),
            ("savedField".to_owned(), Json::string("max_ammo")),
        ])),
        _ => Err(ToolsError::invalid("Q2 flechette initialization missing")),
    }
}

fn q64_config(is_n64: bool, deathmatch: bool, initial_n64_physics: bool, airacceleration: f64) -> Json {
    let mut trace = Vec::new();
    let mut server_n64_physics = initial_n64_physics;
    if is_n64 && !deathmatch {
        trace.push(event(
            "configstring",
            vec![("name", Json::string("CONFIG_N64_PHYSICS")), ("value", Json::string("1"))],
        ));
        server_n64_physics = true;
        trace.push(event("server.pm_config.n64_physics", vec![("value", Json::boolean(true))]));
    }
    trace.push(Json::string("G_InitStatusbar"));
    trace.push(event(
        "configstring",
        vec![("name", Json::string("CS_AIRACCEL")), ("value", Json::string(render_number(airacceleration)))]),
    );
    trace.push(event("server.pm_config.airaccel", vec![("value", Json::float(airacceleration))]));
    Json::object(vec![
        ("serverN64Physics".to_owned(), Json::boolean(server_n64_physics)),
        ("serverAiracceleration".to_owned(), Json::float(airacceleration)),
        ("trace".to_owned(), Json::array(trace)),
    ])
}

/// Evaluate the native command predicates.
#[must_use]
pub fn command_predicates(input: &CommandPredicatesInput) -> Json {
    let buttons = to_int32(input.buttons);
    Json::object(vec![
        ("classicHoldingJump".to_owned(), Json::boolean(input.upmove >= 10.0)),
        ("classicGroundedDuckBranch".to_owned(), Json::boolean(input.upmove < 0.0)),
        ("rereleaseHoldingJump".to_owned(), Json::boolean(buttons & 8 != 0)),
        (
            "rereleaseGroundedDuckBranch".to_owned(),
            Json::boolean(buttons & 16 != 0 && !input.n64_physics),
        ),
    ])
}

/// Evaluate an oracle input document against verified sources.
pub fn evaluate(input: &Json, sources: &HashMap<String, String>) -> Result<Json, ToolsError> {
    match parse_q2_input(input)? {
        Q2Input::Clock { classic_frames, rerelease_step_ms, rerelease_frames } => {
            let mut rerelease_times_ms = Vec::new();
            let mut now = 0.0;
            let mut frame = 0.0;
            while frame < rerelease_frames {
                now += rerelease_step_ms;
                rerelease_times_ms.push(Json::float(now));
                frame += 1.0;
            }
            Ok(Json::object(vec![
                (
                    "classicTimesSeconds".to_owned(),
                    Json::array(classic_frames.iter().map(|frame| Json::float(classic_time(*frame))).collect()),
                ),
                ("rereleaseTimesMs".to_owned(), Json::array(rerelease_times_ms)),
            ]))
        }
        Q2Input::Think(input) => Ok(think(&input)),
        Q2Input::FrameOrder { delete_later_actor, spawn_actor } => Ok(frame_order(delete_later_actor, spawn_actor)),
        Q2Input::Pickup { inventory, capacity, quantity, targets_used } => {
            Ok(pickup(inventory, capacity, quantity, targets_used))
        }
        Q2Input::Armor { damage, inventory, armor: kind, energy, bypass } => {
            Ok(armor(damage, inventory, kind, energy, bypass))
        }
        Q2Input::CrossUnit { flags, trigger, required } => {
            let combined = to_uint32(flags) | to_uint32(trigger);
            let satisfied = required == f64::from(combined & 0xffff00ff & to_uint32(required));
            let steps: &[&str] = if satisfied {
                &["flags|=trigger", "G_FreeEdict(trigger)", "G_UseTargets(target,target)", "G_FreeEdict(target)"]
            } else {
                &["flags|=trigger", "G_FreeEdict(trigger)"]
            };
            Ok(Json::object(vec![
                ("flags".to_owned(), Json::uint(u64::from(combined))),
                ("satisfied".to_owned(), Json::boolean(satisfied)),
                ("trace".to_owned(), Json::array(steps.iter().map(|step| Json::string(*step)).collect())),
            ]))
        }
        Q2Input::SaveFields { save_struct, fields } => save_fields(&save_struct, &fields, sources),
        Q2Input::FlechetteDefault => flechette_default(sources),
        Q2Input::Q64Config { is_n64, deathmatch, initial_n64_physics, airacceleration } => {
            Ok(q64_config(is_n64, deathmatch, initial_n64_physics, airacceleration))
        }
        Q2Input::CommandPredicates(input) => Ok(command_predicates(&input)),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::json::{deep_strict_equal, parse_json};
    use crate::reference::environment::quake_typescript_root;
    use crate::reference::q2::cases::q2_cases;
    use crate::reference::q2::sources::{load_verified_sources, source_text};

    use super::*;

    fn verified_text() -> HashMap<String, String> {
        load_verified_sources(&quake_typescript_root()).expect("verified sources").text
    }

    #[test]
    fn reference_cases_evaluate_to_expectations() {
        let sources = verified_text();
        let cases = q2_cases().expect("cases");
        assert_eq!(cases.len(), 32);
        for reference in &cases {
            let actual = evaluate(&reference.input, &sources).expect(&format!("evaluate {}", reference.id));
            assert!(deep_strict_equal(&actual, &reference.expected), "{}", reference.id);
            assert!(!reference.sources.is_empty(), "{}", reference.id);
            for location in &reference.sources {
                assert!(location.first_line >= 1, "{}", reference.id);
                assert!(location.last_line >= location.first_line, "{}", reference.id);
                let line_count = source_text(&sources, location.source.as_str()).expect("source").split('\n').count();
                assert!(u64::from(location.last_line) <= line_count as u64, "{}", reference.id);
            }
        }
    }

    #[test]
    fn classic_time_crosschecks_binary32_storage() {
        for frame in 0..=10_000 {
            let stored = f64::from((f64::from(frame) / 10.0) as f32);
            assert_eq!(classic_time(f64::from(frame)), stored, "frame {frame}");
        }
        let mut incremented = 0.0;
        for _ in 0..10 {
            incremented = fround(incremented + 0.1);
        }
        assert_ne!(incremented, classic_time(10.0));
    }

    #[test]
    fn adjacent_deadlines_straddle_the_classic_epsilon() {
        let above = f64::from(f32::from_bits((1.001_f32).to_bits()));
        let below = f64::from(f32::from_bits((1.001_f32).to_bits() - 1));
        assert!(below < 1.0 + 0.001);
        assert!(above > 1.0 + 0.001);
        let fired = think(&ThinkInput { family: ThinkFamily::Classic, now: 1.0, nextthink: below, reschedule: 2.0 });
        let expected = parse_json(
            r#"{"now": 1, "nextthink": 2, "mayContinuePhysics": false, "trace": [{"event": "think.enter", "nextthink": 0}, {"event": "think.return", "nextthink": 2}]}"#,
        )
        .unwrap();
        assert!(deep_strict_equal(&fired, &expected), "{}", fired.render());
        let pending = think(&ThinkInput { family: ThinkFamily::Classic, now: 1.0, nextthink: above, reschedule: 2.0 });
        assert_eq!(pending.get("mayContinuePhysics").and_then(Json::as_bool), Some(true));
        assert_eq!(pending.get("nextthink").and_then(Json::as_f64), Some(above));
        assert_eq!(pending.get("trace").and_then(Json::as_array).map(<[Json]>::len), Some(0));
    }

    #[test]
    fn armor_matches_stored_float_equation() {
        let sources = verified_text();
        let stored = f64::from(0.3_f32);
        for damage in 0..=1000 {
            let absorbed = f64::from((stored * f64::from(damage)) as f32).ceil().min(50.0);
            let input = parse_json(&format!(
                r#"{{"kind": "armor", "damage": {damage}, "inventory": 50, "armor": "jacket", "energy": false, "bypass": false}}"#,
            ))
            .unwrap();
            let actual = evaluate(&input, &sources).unwrap();
            let expected = parse_json(&format!(
                r#"{{"absorbed": {absorbed}, "remainingArmor": {}, "remainingDamage": {}, "effect": {}}}"#,
                50.0 - absorbed,
                f64::from(damage) - absorbed,
                if absorbed == 0.0 { "null" } else { "\"SpawnDamage\"" },
            ))
            .unwrap();
            assert!(deep_strict_equal(&actual, &expected), "damage {damage}: {}", actual.render());
        }
        assert_eq!((stored * 10.0).ceil(), 4.0);
    }

    #[test]
    fn all_button_bytes_preserve_native_predicates() {
        for buttons in 0..256 {
            let jump_bit = buttons / 8 % 2 == 1;
            let crouch_bit = buttons / 16 % 2 == 1;
            let up = command_predicates(&CommandPredicatesInput { upmove: 10.0, buttons: f64::from(buttons), n64_physics: false });
            let expected = parse_json(&format!(
                r#"{{"classicHoldingJump": true, "classicGroundedDuckBranch": false, "rereleaseHoldingJump": {jump_bit}, "rereleaseGroundedDuckBranch": {crouch_bit}}}"#,
            ))
            .unwrap();
            assert!(deep_strict_equal(&up, &expected), "buttons {buttons}");
            let down = command_predicates(&CommandPredicatesInput { upmove: -1.0, buttons: f64::from(buttons), n64_physics: true });
            let expected = parse_json(&format!(
                r#"{{"classicHoldingJump": false, "classicGroundedDuckBranch": true, "rereleaseHoldingJump": {jump_bit}, "rereleaseGroundedDuckBranch": false}}"#,
            ))
            .unwrap();
            assert!(deep_strict_equal(&down, &expected), "buttons {buttons}");
        }
    }

    #[test]
    fn save_fields_come_from_the_original_declaration() {
        let sources = verified_text();
        let text = source_text(&sources, "rereleaseSave").unwrap();
        assert!(declared_save_fields(text, "level_locals_t").unwrap().contains(&"current_poi_stage".to_owned()));
        assert!(declared_save_fields(text, "edict_t").unwrap().contains(&"fog.density".to_owned()));
        assert!(declared_save_fields(text, "edict_t").unwrap().contains(&"bmodel_anim.enabled".to_owned()));
        assert!(declared_save_fields(text, "client_persistant_t").unwrap().contains(&"max_ammo".to_owned()));
        assert!(!declared_save_fields(text, "client_persistant_t").unwrap().contains(&"max_flechettes".to_owned()));
        let error = declared_save_fields(text, "not_a_native_struct").expect_err("missing struct");
        assert!(error.to_string().contains("Q2 save structure missing"), "{error}");
    }

    #[test]
    fn converts_like_javascript_bit_operations() {
        assert_eq!(to_int32(2147483648.0), -2147483648);
        assert_eq!(to_int32(-1.0), -1);
        assert_eq!(to_int32(3.9), 3);
        assert_eq!(to_int32(f64::NAN), 0);
        assert_eq!(to_int32(f64::INFINITY), 0);
        assert_eq!(to_uint32(2147483648.0), 2147483648);
        assert_eq!(to_uint32(-1.0), 4294967295);
        assert_eq!(to_uint32(3.9), 3);
        assert_eq!(to_uint32(f64::NAN), 0);
        assert!(js_min(f64::NAN, 1.0).is_nan());
        assert_eq!(js_min(2.0, 1.0), 1.0);
    }

    #[test]
    fn rejects_unknown_inputs() {
        assert!(parse_q2_input(&parse_json(r#"{"kind": "nope"}"#).unwrap()).is_err());
        assert!(parse_q2_input(&parse_json(r#"{"kind": "think", "family": "n64", "now": 0, "nextthink": 0, "reschedule": 0}"#).unwrap()).is_err());
        assert!(parse_q2_input(&parse_json(r#"{"kind": "armor", "damage": 0, "inventory": 0, "armor": "plate", "energy": false, "bypass": false}"#).unwrap()).is_err());
        assert!(parse_q2_input(&parse_json(r#"{"kind": "clock", "classicFrames": [], "rereleaseStepMs": 25}"#).unwrap()).is_err());
    }

    #[test]
    fn save_field_scan_matches_pattern_edges() {
        let text = "#define DECLARE_SAVE_STRUCT demo\nFIELD_AUTO( alpha )\nFIELD_AUTO(beta)\nFIELD_AUTO()\nFIELD_AUTO(gamma(delta))\nFIELD_AUTO( epsilon )\n#undef DECLARE_SAVE_STRUCT\n";
        assert_eq!(declared_save_fields(text, "demo").unwrap(), vec!["alpha".to_owned(), "beta".to_owned(), "epsilon".to_owned()]);
        assert!(declared_save_fields(text, "other").is_err());
        assert!(declared_save_fields("#define DECLARE_SAVE_STRUCT demo\n", "demo").is_err());
    }
}
