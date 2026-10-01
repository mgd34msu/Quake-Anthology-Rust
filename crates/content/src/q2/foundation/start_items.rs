//! Q2 start items (`src/content/q2/foundation/start-items.ts`).
//!
//! Gameplay logic adapted from id Software's Quake II game and the
//! rerelease game DLL (GPL-2.0-or-later).

/// Start item (`Q2StartItem`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Q2StartItem {
    /// Classname.
    pub classname: String,
    /// Count.
    pub count: i64,
}

/// Parse a `parseInt` decimal prefix (`Number.parseInt`).
fn parse_count(text: &str) -> Option<i64> {
    let text = text.trim_start();
    let (negative, text) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    let digits: String = text.chars().take_while(|char| char.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    let mut value: i64 = 0;
    for char in digits.chars() {
        value = value.checked_mul(10)?.checked_add(i64::from(char as u8 - b'0'))?;
    }
    Some(if negative { value.checked_neg()? } else { value })
}

/// Parse Q2 start items (`parseQ2StartItems`).
pub fn parse_q2_start_items(expression: &str) -> Vec<Q2StartItem> {
    expression
        .split(';')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            let space = value.find(char::is_whitespace);
            let (classname, count) = match space {
                None => (value, 1),
                Some(index) => (
                    &value[..index],
                    parse_count(&value[index + 1..]).unwrap_or_else(|| panic!("Invalid Q2 starting item: {value}")),
                ),
            };
            if classname.is_empty()
                || !classname
                    .chars()
                    .all(|char| char.is_ascii_alphanumeric() || char == '_')
            {
                panic!("Invalid Q2 starting item: {value}");
            }
            Q2StartItem {
                classname: classname.to_string(),
                count,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::parse_q2_start_items;

    #[test]
    fn parses_counts() {
        let items = parse_q2_start_items("weapon_shotgun; weapon_rocketlauncher 5");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].classname, "weapon_shotgun");
        assert_eq!(items[0].count, 1);
        assert_eq!(items[1].classname, "weapon_rocketlauncher");
        assert_eq!(items[1].count, 5);
    }

    #[test]
    #[should_panic(expected = "Invalid Q2 starting item")]
    fn rejects_bad_classname() {
        parse_q2_start_items("weapon-shotgun");
    }
}
