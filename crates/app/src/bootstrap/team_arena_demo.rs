//! Team Arena postgame demo selection.
//!
//! Donor: `src/app/bootstrap/team-arena-demo.ts` (`teamArenaDemo`).
//! The async `exists` probe becomes a sync `FnMut`.

use thiserror::Error;

/// Team Arena demo selection failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TeamArenaDemoError {
    /// Map, game type, or protocol failed validation.
    #[error("Invalid Team Arena demo selection")]
    BadSelection,
}

/// Selected postgame demo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamArenaDemo {
    /// Demo name (`{map}_{gameType}`).
    pub name: String,
    /// Demo path (`demos/{name}.dm_{protocol}`).
    pub path: String,
}

/// Source postgame demos use the selected map, game type and protocol.
pub fn team_arena_demo(
    map: &str,
    game_type: i32,
    protocol: i32,
    exists: &mut dyn FnMut(&str) -> bool,
) -> Result<Option<TeamArenaDemo>, TeamArenaDemoError> {
    let valid_map = !map.is_empty()
        && map
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '/' || c == '-')
        && !map.split('/').any(|part| part.is_empty() || part == "..");
    if !valid_map || game_type < 0 || protocol < 0 {
        return Err(TeamArenaDemoError::BadSelection);
    }
    let name = format!("{map}_{game_type}");
    let path = format!("demos/{name}.dm_{protocol}");
    if exists(&path) {
        Ok(Some(TeamArenaDemo { name, path }))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_happy_path() {
        let mut exists = |path: &str| path == "demos/q3tourney6_4.dm_68";
        let demo = team_arena_demo("q3tourney6", 4, 68, &mut exists).unwrap().unwrap();
        assert_eq!(demo.name, "q3tourney6_4");
        assert_eq!(demo.path, "demos/q3tourney6_4.dm_68");
    }

    #[test]
    fn demo_missing_returns_none() {
        let mut exists = |_: &str| false;
        assert!(team_arena_demo("map/one-2_3", 0, 0, &mut exists).unwrap().is_none());
    }

    #[test]
    fn demo_rejects_bad_map() {
        let mut exists = |_: &str| true;
        for map in ["", "a b", "a..b/c", "../x", "a//b", "a/./b", "máp", "a\0b"] {
            assert_eq!(
                team_arena_demo(map, 0, 0, &mut exists),
                Err(TeamArenaDemoError::BadSelection),
                "map {map:?}"
            );
        }
    }

    #[test]
    fn demo_rejects_negative_numbers() {
        let mut exists = |_: &str| true;
        assert_eq!(
            team_arena_demo("map", -1, 68, &mut exists),
            Err(TeamArenaDemoError::BadSelection)
        );
        assert_eq!(
            team_arena_demo("map", 4, -1, &mut exists),
            Err(TeamArenaDemoError::BadSelection)
        );
    }
}
