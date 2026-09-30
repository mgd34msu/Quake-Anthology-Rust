//! Quake II `SV_Map`-style travel expression parsing.
//!
//! Sync port of donor `src/app/bootstrap/q2-travel.ts`: `SV_Map` consumes
//! the next-server suffix before the spawn point and unit marker. The
//! donor's regular expressions are implemented as explicit byte checks
//! (no new workspace dependencies).

use thiserror::Error;

/// Failure to parse a Q2 travel expression.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Q2TravelError {
    /// One `+`-separated destination is empty.
    #[error("Q2 travel has an empty destination")]
    EmptyDestination,
    /// A destination name has invalid characters or shape.
    #[error("Invalid Q2 travel destination: {0}")]
    BadDestination(String),
    /// A `$spawn` suffix has invalid characters.
    #[error("Invalid Q2 spawn point: {0}")]
    BadSpawnPoint(String),
    /// The expression has no destination at all.
    #[error("Q2 travel has no destination")]
    NoDestination,
}

/// What a travel destination loads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2TravelKind {
    /// A map.
    Map,
    /// A `.cin` cinematic.
    Cinematic,
    /// A `.pcx` picture.
    Picture,
    /// A `.dm2` demo.
    Demo,
}

/// One parsed travel destination with its chained suffix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2TravelTarget {
    /// Destination kind.
    pub kind: Q2TravelKind,
    /// Destination name (unit marker stripped).
    pub name: String,
    /// Spawn point after `$` (possibly empty).
    pub spawn_point: String,
    /// The level started a new unit (`*` prefix).
    pub new_unit: bool,
    /// Chained next destination (`+`-suffix).
    pub next: Option<Box<Q2TravelTarget>>,
}

fn valid_stem(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'/' | b'-'))
}

/// Parse a `+`-chained travel expression.
pub fn parse_q2_travel(expression: &str) -> Result<Q2TravelTarget, Q2TravelError> {
    let mut next: Option<Box<Q2TravelTarget>> = None;
    for part in expression.split('+').rev() {
        if part.is_empty() {
            return Err(Q2TravelError::EmptyDestination);
        }
        let (level, spawn_point) = match part.find('$') {
            None => (part, ""),
            Some(dollar) => (&part[..dollar], &part[dollar + 1..]),
        };
        let (new_unit, name) = match level.strip_prefix('*') {
            Some(name) => (true, name),
            None => (false, level),
        };
        let stem = [".cin", ".pcx", ".dm2"]
            .iter()
            .find_map(|extension| name.strip_suffix(*extension).filter(|stem| !stem.is_empty()))
            .unwrap_or(name);
        if !valid_stem(stem) || name.starts_with('/') || name.contains("//") {
            return Err(Q2TravelError::BadDestination(name.to_string()));
        }
        if !spawn_point
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            return Err(Q2TravelError::BadSpawnPoint(spawn_point.to_string()));
        }
        let kind = if name.ends_with(".cin") {
            Q2TravelKind::Cinematic
        } else if name.ends_with(".pcx") {
            Q2TravelKind::Picture
        } else if name.ends_with(".dm2") {
            Q2TravelKind::Demo
        } else {
            Q2TravelKind::Map
        };
        next = Some(Box::new(Q2TravelTarget {
            kind,
            name: name.to_string(),
            spawn_point: spawn_point.to_string(),
            new_unit,
            next,
        }));
    }
    next.map(|target| *target).ok_or(Q2TravelError::NoDestination)
}

/// Render the authored `SV_Map` suffix as a source `nextserver` command;
/// empty when the target has no chained suffix. The rebuilt expression is
/// re-parsed like the donor, so a malformed chain errors.
pub fn q2_next_server_command(target: &Q2TravelTarget) -> Result<String, Q2TravelError> {
    if target.next.is_none() {
        return Ok(String::new());
    }
    let mut parts = Vec::new();
    let mut cursor = target.next.as_deref();
    while let Some(next) = cursor {
        let mut part = String::new();
        if next.new_unit {
            part.push('*');
        }
        part.push_str(&next.name);
        if !next.spawn_point.is_empty() {
            part.push('$');
            part.push_str(&next.spawn_point);
        }
        parts.push(part);
        cursor = next.next.as_deref();
    }
    let expression = parts.join("+");
    parse_q2_travel(&expression)?;
    Ok(format!("gamemap \"{expression}\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_maps_and_chains() {
        let target = parse_q2_travel("base1$start+*base2+intro.cin").unwrap();
        assert_eq!(target.kind, Q2TravelKind::Map);
        assert_eq!(target.name, "base1");
        assert_eq!(target.spawn_point, "start");
        assert!(!target.new_unit);
        let second = target.next.as_deref().unwrap();
        assert_eq!(second.name, "base2");
        assert!(second.new_unit);
        let third = second.next.as_deref().unwrap();
        assert_eq!(third.kind, Q2TravelKind::Cinematic);
        assert_eq!(third.name, "intro.cin");
        assert!(third.next.is_none());
        assert_eq!(parse_q2_travel("shot.pcx").unwrap().kind, Q2TravelKind::Picture);
        assert_eq!(parse_q2_travel("demo1.dm2").unwrap().kind, Q2TravelKind::Demo);
    }

    #[test]
    fn rejects_bad_destinations() {
        assert_eq!(parse_q2_travel("").unwrap_err(), Q2TravelError::EmptyDestination);
        assert_eq!(
            parse_q2_travel("base1++base2").unwrap_err(),
            Q2TravelError::EmptyDestination
        );
        assert_eq!(
            parse_q2_travel("has space").unwrap_err(),
            Q2TravelError::BadDestination("has space".to_string())
        );
        assert_eq!(
            parse_q2_travel("/leading").unwrap_err(),
            Q2TravelError::BadDestination("/leading".to_string())
        );
        assert_eq!(
            parse_q2_travel("a//b").unwrap_err(),
            Q2TravelError::BadDestination("a//b".to_string())
        );
        assert_eq!(
            parse_q2_travel(".cin").unwrap_err(),
            Q2TravelError::BadDestination(".cin".to_string())
        );
        assert_eq!(
            parse_q2_travel("boss.intro.cin").unwrap_err(),
            Q2TravelError::BadDestination("boss.intro.cin".to_string())
        );
        assert_eq!(
            parse_q2_travel("base1$bad!").unwrap_err(),
            Q2TravelError::BadSpawnPoint("bad!".to_string())
        );
    }

    #[test]
    fn next_server_command_round_trips() {
        let single = parse_q2_travel("base1").unwrap();
        assert_eq!(q2_next_server_command(&single).unwrap(), "");
        let chained = parse_q2_travel("base1$start+*base2$x+demo1.dm2").unwrap();
        assert_eq!(
            q2_next_server_command(&chained).unwrap(),
            "gamemap \"*base2$x+demo1.dm2\""
        );
        let broken = Q2TravelTarget {
            kind: Q2TravelKind::Map,
            name: "base1".to_string(),
            spawn_point: String::new(),
            new_unit: false,
            next: Some(Box::new(Q2TravelTarget {
                kind: Q2TravelKind::Map,
                name: "bad name".to_string(),
                spawn_point: String::new(),
                new_unit: false,
                next: None,
            })),
        };
        assert!(q2_next_server_command(&broken).is_err());
    }
}
