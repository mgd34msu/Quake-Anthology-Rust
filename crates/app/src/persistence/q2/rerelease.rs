//! Quake II rerelease source saves ported from `src/persistence/q2-rerelease.ts`.
//!
//! `g_save.cpp` JSON: dotted `FIELD_AUTO` names stay flat keys. Game and
//! level envelopes plus the fog, height-fog, brush-animation, POI, and
//! ammo-capacity field helpers operate directly on [`SourceJson`]
//! records like the donor.

use std::collections::HashMap;

use qa_core::math::Vec3;
use qa_world::save::json::{
    parse_source_json, source_bool, source_number, source_object, write_source_json, SaveNumber, SourceJson,
};

use super::super::PersistenceError;

fn save_error(path: &str, message: &str) -> PersistenceError {
    PersistenceError::BadSave(format!("{path}: {message}"))
}

/// Rerelease game save.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2RereleaseGameSave {
    /// Root record.
    pub root: SourceJson,
    /// Game record.
    pub game: SourceJson,
    /// Client records.
    pub clients: Vec<SourceJson>,
}

/// Rerelease level save.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2RereleaseLevelSave {
    /// Root record.
    pub root: SourceJson,
    /// Level record.
    pub level: SourceJson,
    /// Entity records by slot.
    pub entities: HashMap<u32, SourceJson>,
}

fn root_of(text: &str) -> Result<SourceJson, PersistenceError> {
    let parsed = parse_source_json(text)?;
    let root = source_object(Some(&parsed), "q2-rerelease")?.clone();
    if source_number(root.get("save_version"), "save_version", 0.0)? != 1.0 {
        return Err(save_error("q2-rerelease", "unsupported game save version"));
    }
    Ok(root)
}

/// Decode a rerelease game save.
pub fn decode_q2_rerelease_game(text: &str) -> Result<Q2RereleaseGameSave, PersistenceError> {
    let root = root_of(text)?;
    let game = source_object(root.get("game"), "game")?.clone();
    let clients = match root.get("clients") {
        Some(SourceJson::Array(clients)) => clients
            .iter()
            .enumerate()
            .map(|(index, client)| source_object(Some(client), &format!("clients[{index}]")).cloned())
            .collect::<Result<Vec<_>, _>>()?,
        _ => return Err(save_error("clients", "expected client array")),
    };
    Ok(Q2RereleaseGameSave { root, game, clients })
}

/// Encode a rerelease game save.
pub fn encode_q2_rerelease_game(save: &Q2RereleaseGameSave) -> String {
    let mut root = save.root.clone();
    let _ = root.set("game", save.game.clone());
    let _ = root.set("clients", SourceJson::Array(save.clients.clone()));
    write_source_json(&root)
}

/// Decode a rerelease level save.
pub fn decode_q2_rerelease_level(text: &str) -> Result<Q2RereleaseLevelSave, PersistenceError> {
    let root = root_of(text)?;
    let level = source_object(root.get("level"), "level")?.clone();
    let records = source_object(root.get("entities"), "entities")?.clone();
    let mut entities = HashMap::new();
    for key in records.keys() {
        let slot: u32 = key.parse().unwrap_or(u32::MAX);
        if key != slot.to_string() {
            return Err(save_error(&format!("entities.{key}"), "invalid source entity index"));
        }
        let record = source_object(records.get(&key), &format!("entities.{key}"))?.clone();
        entities.insert(slot, record);
    }
    Ok(Q2RereleaseLevelSave { root, level, entities })
}

/// Encode a rerelease level save.
pub fn encode_q2_rerelease_level(save: &Q2RereleaseLevelSave) -> String {
    let mut root = save.root.clone();
    let mut entities = SourceJson::Object(Vec::new());
    let mut slots: Vec<u32> = save.entities.keys().copied().collect();
    slots.sort_unstable();
    for slot in slots {
        let _ = entities.set(&slot.to_string(), save.entities[&slot].clone());
    }
    let _ = root.set("level", save.level.clone());
    let _ = root.set("entities", entities);
    write_source_json(&root)
}

fn vector(value: Option<&SourceJson>, path: &str) -> Result<Vec3, PersistenceError> {
    match value {
        None => Ok(Vec3 { x: 0.0, y: 0.0, z: 0.0 }),
        Some(SourceJson::Array(items)) if items.len() == 3 => Ok(Vec3 {
            #[allow(clippy::cast_possible_truncation)]
            x: source_number(Some(&items[0]), path, 0.0)? as f32,
            #[allow(clippy::cast_possible_truncation)]
            y: source_number(Some(&items[1]), path, 0.0)? as f32,
            #[allow(clippy::cast_possible_truncation)]
            z: source_number(Some(&items[2]), path, 0.0)? as f32,
        }),
        _ => Err(save_error(path, "expected source vec3")),
    }
}

fn vector_value(value: Vec3) -> SourceJson {
    SourceJson::Array(vec![
        SourceJson::Number(SaveNumber::from_f64(f64::from(value.x))),
        SourceJson::Number(SaveNumber::from_f64(f64::from(value.y))),
        SourceJson::Number(SaveNumber::from_f64(f64::from(value.z))),
    ])
}

fn number_value(value: f64) -> SourceJson {
    SourceJson::Number(SaveNumber::from_f64(value))
}

/// Saved fog state.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2FogSave {
    /// Color.
    pub color: Vec3,
    /// Density.
    pub density: f64,
    /// Disabled color.
    pub color_off: Vec3,
    /// Disabled density.
    pub density_off: f64,
    /// Sky factor.
    pub sky_factor: f64,
    /// Disabled sky factor.
    pub sky_factor_off: f64,
}

/// Read fog fields from a record.
pub fn read_q2_fog(record: &SourceJson) -> Result<Q2FogSave, PersistenceError> {
    Ok(Q2FogSave {
        color: vector(record.get("fog.color"), "fog.color")?,
        density: source_number(record.get("fog.density"), "fog.density", 0.0)?,
        color_off: vector(record.get("fog.color_off"), "fog.color_off")?,
        density_off: source_number(record.get("fog.density_off"), "fog.density_off", 0.0)?,
        sky_factor: source_number(record.get("fog.sky_factor"), "fog.sky_factor", 0.0)?,
        sky_factor_off: source_number(record.get("fog.sky_factor_off"), "fog.sky_factor_off", 0.0)?,
    })
}

/// Write fog fields into a record.
pub fn write_q2_fog(record: &mut SourceJson, fog: &Q2FogSave) -> Result<(), PersistenceError> {
    record.set("fog.color", vector_value(fog.color))?;
    record.set("fog.density", number_value(fog.density))?;
    record.set("fog.color_off", vector_value(fog.color_off))?;
    record.set("fog.density_off", number_value(fog.density_off))?;
    record.set("fog.sky_factor", number_value(fog.sky_factor))?;
    record.set("fog.sky_factor_off", number_value(fog.sky_factor_off))?;
    Ok(())
}

/// Saved height-fog state.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2HeightFogSave {
    /// Falloff.
    pub falloff: f64,
    /// Density.
    pub density: f64,
    /// Start color.
    pub start_color: Vec3,
    /// Start distance.
    pub start_distance: f64,
    /// End color.
    pub end_color: Vec3,
    /// End distance.
    pub end_distance: f64,
    /// Disabled falloff.
    pub falloff_off: f64,
    /// Disabled density.
    pub density_off: f64,
    /// Disabled start color.
    pub start_color_off: Vec3,
    /// Disabled start distance.
    pub start_distance_off: f64,
    /// Disabled end color.
    pub end_color_off: Vec3,
    /// Disabled end distance.
    pub end_distance_off: f64,
}

/// Read height-fog fields from a record.
pub fn read_q2_height_fog(record: &SourceJson) -> Result<Q2HeightFogSave, PersistenceError> {
    let number = |key: &str| {
        source_number(
            record.get(&format!("heightfog.{key}")),
            &format!("heightfog.{key}"),
            0.0,
        )
    };
    let vec = |key: &str| vector(record.get(&format!("heightfog.{key}")), &format!("heightfog.{key}"));
    Ok(Q2HeightFogSave {
        falloff: number("falloff")?,
        density: number("density")?,
        start_color: vec("start_color")?,
        start_distance: number("start_dist")?,
        end_color: vec("end_color")?,
        end_distance: number("end_dist")?,
        falloff_off: number("falloff_off")?,
        density_off: number("density_off")?,
        start_color_off: vec("start_color_off")?,
        start_distance_off: number("start_dist_off")?,
        end_color_off: vec("end_color_off")?,
        end_distance_off: number("end_dist_off")?,
    })
}

/// Write height-fog fields into a record.
pub fn write_q2_height_fog(record: &mut SourceJson, fog: &Q2HeightFogSave) -> Result<(), PersistenceError> {
    for (key, value) in [
        ("falloff", fog.falloff),
        ("density", fog.density),
        ("start_dist", fog.start_distance),
        ("end_dist", fog.end_distance),
        ("falloff_off", fog.falloff_off),
        ("density_off", fog.density_off),
        ("start_dist_off", fog.start_distance_off),
        ("end_dist_off", fog.end_distance_off),
    ] {
        record.set(&format!("heightfog.{key}"), number_value(value))?;
    }
    for (key, value) in [
        ("start_color", fog.start_color),
        ("end_color", fog.end_color),
        ("start_color_off", fog.start_color_off),
        ("end_color_off", fog.end_color_off),
    ] {
        record.set(&format!("heightfog.{key}"), vector_value(value))?;
    }
    Ok(())
}

/// Saved brush-model animation.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2BrushAnimationSave {
    /// Start frame.
    pub start: f64,
    /// End frame.
    pub end: f64,
    /// Style.
    pub style: f64,
    /// Speed.
    pub speed: f64,
    /// No wrap.
    pub no_wrap: bool,
    /// Alternate start.
    pub alternate_start: f64,
    /// Alternate end.
    pub alternate_end: f64,
    /// Alternate style.
    pub alternate_style: f64,
    /// Alternate speed.
    pub alternate_speed: f64,
    /// Alternate no wrap.
    pub alternate_no_wrap: bool,
    /// Enabled.
    pub enabled: bool,
    /// Alternate.
    pub alternate: bool,
    /// Currently alternate.
    pub currently_alternate: bool,
    /// Next tick.
    pub next_tick: f64,
}

/// Read brush-animation fields from a record.
pub fn read_q2_brush_animation(record: &SourceJson) -> Result<Q2BrushAnimationSave, PersistenceError> {
    let number = |key: &str| {
        source_number(
            record.get(&format!("bmodel_anim.{key}")),
            &format!("bmodel_anim.{key}"),
            0.0,
        )
    };
    let flag = |key: &str| source_bool(record.get(&format!("bmodel_anim.{key}")), &format!("bmodel_anim.{key}"));
    Ok(Q2BrushAnimationSave {
        start: number("start")?,
        end: number("end")?,
        style: number("style")?,
        speed: number("speed")?,
        no_wrap: flag("nowrap")?,
        alternate_start: number("alt_start")?,
        alternate_end: number("alt_end")?,
        alternate_style: number("alt_style")?,
        alternate_speed: number("alt_speed")?,
        alternate_no_wrap: flag("alt_nowrap")?,
        enabled: flag("enabled")?,
        alternate: flag("alternate")?,
        currently_alternate: flag("currently_alternate")?,
        next_tick: number("next_tick")?,
    })
}

/// Write brush-animation fields into a record.
pub fn write_q2_brush_animation(
    record: &mut SourceJson,
    animation: &Q2BrushAnimationSave,
) -> Result<(), PersistenceError> {
    for (key, value) in [
        ("start", animation.start),
        ("end", animation.end),
        ("style", animation.style),
        ("speed", animation.speed),
        ("alt_start", animation.alternate_start),
        ("alt_end", animation.alternate_end),
        ("alt_style", animation.alternate_style),
        ("alt_speed", animation.alternate_speed),
        ("next_tick", animation.next_tick),
    ] {
        record.set(&format!("bmodel_anim.{key}"), number_value(value))?;
    }
    for (key, value) in [
        ("nowrap", animation.no_wrap),
        ("alt_nowrap", animation.alternate_no_wrap),
        ("enabled", animation.enabled),
        ("alternate", animation.alternate),
        ("currently_alternate", animation.currently_alternate),
    ] {
        record.set(&format!("bmodel_anim.{key}"), SourceJson::Bool(value))?;
    }
    Ok(())
}

/// Saved point of interest.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2PoiSave {
    /// Valid flag.
    pub valid: bool,
    /// Point.
    pub point: Vec3,
    /// Stage.
    pub stage: f64,
    /// Image.
    pub image: f64,
    /// Dynamic entity.
    pub dynamic_entity: Option<f64>,
}

/// Read POI fields from a level record.
pub fn read_q2_poi(level: &SourceJson) -> Result<Q2PoiSave, PersistenceError> {
    Ok(Q2PoiSave {
        valid: source_bool(level.get("valid_poi"), "valid_poi")?,
        point: vector(level.get("current_poi"), "current_poi")?,
        stage: source_number(level.get("current_poi_stage"), "current_poi_stage", 0.0)?,
        image: source_number(level.get("current_poi_image"), "current_poi_image", 0.0)?,
        dynamic_entity: match level.get("current_dynamic_poi") {
            None | Some(SourceJson::Null) => None,
            value => Some(source_number(value, "current_dynamic_poi", 0.0)?),
        },
    })
}

/// Write POI fields into a level record.
pub fn write_q2_poi(level: &mut SourceJson, poi: &Q2PoiSave) -> Result<(), PersistenceError> {
    level.set("valid_poi", SourceJson::Bool(poi.valid))?;
    level.set("current_poi", vector_value(poi.point))?;
    level.set("current_poi_stage", number_value(poi.stage))?;
    level.set("current_poi_image", number_value(poi.image))?;
    level.set(
        "current_dynamic_poi",
        poi.dynamic_entity.map_or(SourceJson::Null, number_value),
    )?;
    Ok(())
}

/// Read ammo capacities from a persistent record.
pub fn read_q2_ammo_capacity(persistent: &SourceJson) -> Result<Vec<f64>, PersistenceError> {
    match persistent.get("max_ammo") {
        None => Ok(Vec::new()),
        Some(SourceJson::Array(values)) => values
            .iter()
            .enumerate()
            .map(|(index, value)| source_number(Some(value), &format!("max_ammo[{index}]"), 0.0))
            .collect::<Result<Vec<_>, _>>()
            .map_err(PersistenceError::from),
        _ => Err(save_error("max_ammo", "expected ammo capacity array")),
    }
}

/// Write ammo capacities into a persistent record.
pub fn write_q2_ammo_capacity(persistent: &mut SourceJson, values: &[f64]) -> Result<(), PersistenceError> {
    persistent.set(
        "max_ammo",
        SourceJson::Array(values.iter().map(|value| number_value(*value)).collect()),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelopes_round_trip() {
        let game_text = r#"{"save_version":1,"game":{"map":"base1"},"clients":[{"health":100}]}"#;
        let game = decode_q2_rerelease_game(game_text).unwrap();
        assert_eq!(encode_q2_rerelease_game(&game), game_text);
        let level_text =
            r#"{"save_version":1,"level":{"map":"base1"},"entities":{"0":{"classname":"worldspawn"},"2":{}}}"#;
        let level = decode_q2_rerelease_level(level_text).unwrap();
        assert_eq!(level.entities.len(), 2);
        assert_eq!(encode_q2_rerelease_level(&level), level_text);
        assert!(decode_q2_rerelease_game(r#"{"save_version":2,"game":{},"clients":[]}"#).is_err());
        assert!(decode_q2_rerelease_level(r#"{"save_version":1,"level":{},"entities":{"x":{}}}"#).is_err());
    }

    #[test]
    fn field_helpers_round_trip() {
        let mut record = SourceJson::Object(Vec::new());
        let fog = Q2FogSave {
            color: Vec3 { x: 1.0, y: 0.0, z: 0.0 },
            density: 0.5,
            color_off: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            density_off: 0.0,
            sky_factor: 1.0,
            sky_factor_off: 0.0,
        };
        write_q2_fog(&mut record, &fog).unwrap();
        assert_eq!(read_q2_fog(&record).unwrap(), fog);
        // Missing fields default like the donor.
        let empty = SourceJson::Object(Vec::new());
        assert_eq!(read_q2_fog(&empty).unwrap().density, 0.0);
        let mut level = SourceJson::Object(Vec::new());
        let poi = Q2PoiSave {
            valid: true,
            point: Vec3 { x: 1.0, y: 2.0, z: 3.0 },
            stage: 2.0,
            image: 4.0,
            dynamic_entity: None,
        };
        write_q2_poi(&mut level, &poi).unwrap();
        assert_eq!(read_q2_poi(&level).unwrap(), poi);
        let mut persistent = SourceJson::Object(Vec::new());
        write_q2_ammo_capacity(&mut persistent, &[50.0, 10.0]).unwrap();
        assert_eq!(read_q2_ammo_capacity(&persistent).unwrap(), vec![50.0, 10.0]);
        assert!(read_q2_ammo_capacity(&SourceJson::Object(vec![(
            "max_ammo".to_string(),
            SourceJson::Bool(true)
        )]))
        .is_err());
        let animation = Q2BrushAnimationSave {
            start: 0.0,
            end: 9.0,
            style: 1.0,
            speed: 2.0,
            no_wrap: true,
            alternate_start: 0.0,
            alternate_end: 0.0,
            alternate_style: 0.0,
            alternate_speed: 0.0,
            alternate_no_wrap: false,
            enabled: true,
            alternate: false,
            currently_alternate: false,
            next_tick: 3.0,
        };
        write_q2_brush_animation(&mut record, &animation).unwrap();
        assert_eq!(read_q2_brush_animation(&record).unwrap(), animation);
    }
}
