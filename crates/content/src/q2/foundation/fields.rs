//! Spawn-field readers (`src/content/q2/foundation/fields.ts`).
//!
//! Vector math is reused from [`qa_core::math`] (`add3`, `sub3`,
//! `scale3`, `dot3`, `length3`, `normalize3`); this module only keeps the
//! spawn-text readers, the entity parser and the spawn gate.

use qa_core::math::Vec3;
use qa_core::numeric::{native_atof, native_atoi};

use super::host::{Q2Edition, Q2GameOptions, Q2SpawnFields};
use crate::q2::support::misc::{parse_q2_token, ParseState};

/// Shared zero vector (donor `zero`).
pub const ZERO: Vec3 = Vec3 {
    x: 0.0,
    y: 0.0,
    z: 0.0,
};

/// Read a float spawn field (`numberField`).
pub fn number_field(fields: &Q2SpawnFields, key: &str, fallback: f64) -> f64 {
    match fields.values.get(key) {
        None => fallback,
        Some(value) => native_atof(value)
            .unwrap_or_else(|_| panic!("Q2 spawn field {key} is not source text")),
    }
}

/// Read an integer spawn field (`integerField`).
pub fn integer_field(fields: &Q2SpawnFields, key: &str, fallback: i32) -> i32 {
    match fields.values.get(key) {
        None => fallback,
        Some(value) => native_atoi(value)
            .unwrap_or_else(|_| panic!("Q2 spawn field {key} is not source text")),
    }
}

/// Read a vector spawn field (`vectorField`).
pub fn vector_field(fields: &Q2SpawnFields, key: &str) -> Vec3 {
    let text = fields.values.get(key).map_or("", String::as_str);
    let mut parts = text.split_whitespace();
    let component = |part: Option<&str>| native_atof(part.unwrap_or("0")).unwrap_or(0.0) as f32;
    Vec3 {
        x: component(parts.next()),
        y: component(parts.next()),
        z: component(parts.next()),
    }
}

/// `ED_NewString` backslash handling: `\n` becomes LF, any other escape
/// introducer collapses to one backslash; a trailing lone backslash stays.
fn entity_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(char) = chars.next() {
        if char != '\\' {
            out.push(char);
            continue;
        }
        match chars.next() {
            None => out.push('\\'),
            Some('n') => out.push('\n'),
            Some(_) => out.push('\\'),
        }
    }
    out
}

enum EntityToken {
    Eof,
    Token(String),
}

/// Skip whitespace and `//` comments, then parse one token.
///
/// `COM_Parse`'s data pointer distinguishes EOF from a valid empty quoted
/// token, mirrored here by the `EntityToken` kind.
fn entity_token(state: &mut ParseState, limit: usize) -> EntityToken {
    loop {
        while state.index < state.data.len() {
            let unit = state.data[state.index];
            if unit == 0 || unit > 32 {
                break;
            }
            state.index += 1;
        }
        if state.index >= state.data.len() || state.data[state.index] == 0 {
            return EntityToken::Eof;
        }
        let slash = state.data[state.index] == 47 && state.data.get(state.index + 1) == Some(&47);
        if !slash {
            break;
        }
        while state.index < state.data.len() && state.data[state.index] != 0 && state.data[state.index] != 10 {
            state.index += 1;
        }
    }
    EntityToken::Token(parse_q2_token(state, limit))
}

/// Parse authored entities (`parseQ2Entities`).
pub fn parse_q2_entities(source: &str, edition: Q2Edition) -> Vec<Q2SpawnFields> {
    let mut state = ParseState::new(source);
    let mut entities = Vec::new();
    let limit = match edition {
        Q2Edition::Classic => 128,
        Q2Edition::Rerelease => 512,
    };
    loop {
        let token = entity_token(&mut state, limit);
        match token {
            EntityToken::Eof => break,
            EntityToken::Token(value) => {
                if value != "{" {
                    panic!("Q2 entity {}: expected opening brace", entities.len());
                }
            }
        }
        let mut values = std::collections::BTreeMap::new();
        loop {
            let key = entity_token(&mut state, limit);
            let key = match key {
                EntityToken::Eof => panic!("Q2 entity {}: EOF without closing brace", entities.len()),
                EntityToken::Token(value) => value,
            };
            if key == "}" {
                break;
            }
            let value = entity_token(&mut state, limit);
            match value {
                EntityToken::Eof => panic!("Q2 entity {}: missing value for {key}", entities.len()),
                EntityToken::Token(value) => {
                    if value == "}" {
                        panic!("Q2 entity {}: missing value for {key}", entities.len());
                    }
                    values.insert(key, entity_string(&value));
                }
            }
        }
        let ordinal = entities.len() as i32;
        let classname = values.get("classname").cloned().unwrap_or_default();
        entities.push(Q2SpawnFields {
            ordinal,
            classname,
            values,
        });
    }
    entities
}

/// Whether a spawn is inhibited by skill or mode (`inhibitQ2Spawn`).
pub fn inhibit_q2_spawn(fields: &Q2SpawnFields, options: &Q2GameOptions) -> bool {
    if fields.classname == "worldspawn" {
        return false;
    }
    let flags = integer_field(fields, "spawnflags", 0);
    if options.mode == super::host::Q2Mode::Deathmatch {
        return flags & 2048 != 0;
    }
    if options.edition == Q2Edition::Rerelease {
        if options.mode == super::host::Q2Mode::Coop && flags & 4096 != 0 {
            return true;
        }
        if options.mode != super::host::Q2Mode::Coop && flags & 16384 != 0 {
            return true;
        }
    }
    flags
        & (match options.skill {
            0 => 256,
            1 => 512,
            _ => 1024,
        })
        != 0
}

/// Door movedir from angles (`movedir`).
pub fn movedir(angles: Vec3) -> Vec3 {
    if angles.x == 0.0 && angles.y == -1.0 && angles.z == 0.0 {
        return Vec3 { x: 0.0, y: 0.0, z: 1.0 };
    }
    if angles.x == 0.0 && angles.y == -2.0 && angles.z == 0.0 {
        return Vec3 { x: 0.0, y: 0.0, z: -1.0 };
    }
    let yaw = f64::from(angles.y) * std::f64::consts::PI / 180.0;
    let pitch = f64::from(angles.x) * std::f64::consts::PI / 180.0;
    Vec3 {
        x: (pitch.cos() * yaw.cos()) as f32,
        y: (pitch.cos() * yaw.sin()) as f32,
        z: (-pitch.sin()) as f32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::BTreeMap;

    use qa_core::identity::ProviderId;

    use super::super::host::{Q2Edition, Q2Mode};

    fn options(mode: Q2Mode, skill: u8, edition: Q2Edition) -> Q2GameOptions {
        Q2GameOptions {
            edition,
            map_name: "base1".to_string(),
            skill,
            mode,
            deathmatch_flags: 0,
            max_clients: 1,
            provider: ProviderId::new("q2", "baseq2"),
            damage_powerup_owner: None,
            source_damage_modifier: None,
            campaign: ProviderId::new("q2", "campaign"),
            combat_provider: ProviderId::new("q2", "combat"),
            inventory_provider: ProviderId::new("q2", "inventory"),
            movement_provider: ProviderId::new("q2", "movement"),
        }
    }

    fn fields(classname: &str, pairs: &[(&str, &str)]) -> Q2SpawnFields {
        Q2SpawnFields {
            ordinal: 0,
            classname: classname.to_string(),
            values: pairs
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect::<BTreeMap<_, _>>(),
        }
    }

    #[test]
    fn parses_entities_with_escapes() {
        let parsed = parse_q2_entities(
            "{\n\"classname\" \"light\"\n\"message\" \"a\\\\nb\\\\tc\\\\\"\n}\n{\n\"classname\" \"worldspawn\"\n}",
            Q2Edition::Classic,
        );
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].classname, "light");
        assert_eq!(parsed[0].values.get("message").map(String::as_str), Some("a\nb\\c\\"));
        assert_eq!(parsed[1].ordinal, 1);
    }

    #[test]
    fn inhibits_spawns_by_skill_and_mode() {
        let easy = fields("monster_soldier", &[("spawnflags", "256")]);
        assert!(inhibit_q2_spawn(&easy, &options(Q2Mode::Singleplayer, 0, Q2Edition::Classic)));
        assert!(!inhibit_q2_spawn(&easy, &options(Q2Mode::Singleplayer, 1, Q2Edition::Classic)));
        let no_dm = fields("weapon_shotgun", &[("spawnflags", "2048")]);
        assert!(inhibit_q2_spawn(&no_dm, &options(Q2Mode::Deathmatch, 1, Q2Edition::Classic)));
        let world = fields("worldspawn", &[("spawnflags", "2048")]);
        assert!(!inhibit_q2_spawn(&world, &options(Q2Mode::Deathmatch, 1, Q2Edition::Classic)));
    }

    #[test]
    fn reads_numbers_vectors_and_movedir() {
        let field_values = fields("func_door", &[("speed", "100"), ("angle", "45"), ("origin", "1 2 3")]);
        assert_eq!(number_field(&field_values, "speed", 0.0), 100.0);
        assert_eq!(number_field(&field_values, "missing", 7.0), 7.0);
        assert_eq!(integer_field(&field_values, "angle", 0), 45);
        assert_eq!(
            vector_field(&field_values, "origin"),
            Vec3 { x: 1.0, y: 2.0, z: 3.0 }
        );
        let up = movedir(Vec3 { x: 0.0, y: -1.0, z: 0.0 });
        assert_eq!(up, Vec3 { x: 0.0, y: 0.0, z: 1.0 });
    }
}
