//! Authored campaign start selection.
//!
//! Donor: `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/authored-start.ts`
//! (`authoredCampaignStart`).
//! Only a selected authored start runs its intro and starting inventory;
//! arbitrary maps remain direct.

use qa_content::catalog::AuthoredStartMap;
use qa_content::q2::foundation::start_items::parse_q2_start_items;
use thiserror::Error;

use super::q2_travel::{parse_q2_travel, Q2TravelError, Q2TravelTarget};

/// Authored campaign start failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AuthoredStartError {
    /// Starting inventory entry is invalid.
    #[error("Invalid Q2 starting item: {0}")]
    InvalidStartItem(String),
    /// Start BSP travel expression is invalid.
    #[error(transparent)]
    Travel(#[from] Q2TravelError),
}

/// Authored campaign start (`AuthoredCampaignStart`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoredCampaignStart {
    /// Travel destination parsed from the start BSP.
    pub target: Q2TravelTarget,
    /// Starting inventory expression.
    pub start_items: String,
}

fn valid_start_items(expression: &str) -> Result<(), AuthoredStartError> {
    for value in expression.split(';').map(str::trim).filter(|value| !value.is_empty()) {
        let space = value.find(char::is_whitespace);
        let (classname, count) = match space {
            None => (value, "1"),
            Some(index) => (&value[..index], value[index + 1..].trim_start()),
        };
        if classname.is_empty() || !classname.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(AuthoredStartError::InvalidStartItem(value.to_string()));
        }
        let digits = count
            .strip_prefix(['+', '-'])
            .unwrap_or(count)
            .chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>();
        if digits.is_empty() {
            return Err(AuthoredStartError::InvalidStartItem(value.to_string()));
        }
        let mut magnitude: i64 = 0;
        for digit in digits.chars() {
            magnitude = magnitude
                .checked_mul(10)
                .and_then(|scaled| scaled.checked_add(i64::from(digit as u8 - b'0')))
                .ok_or_else(|| AuthoredStartError::InvalidStartItem(value.to_string()))?;
        }
        if magnitude > (1i64 << 53) {
            return Err(AuthoredStartError::InvalidStartItem(value.to_string()));
        }
    }
    Ok(())
}

/// Resolve the authored start for a selected map (`authoredCampaignStart`).
/// Callers pass `catalog.starts`; an empty slice (or a missing catalog)
/// resolves to `None`. Returns `None` when the map has no authored start.
pub fn authored_campaign_start(
    starts: &[AuthoredStartMap],
    selected_map: &str,
) -> Result<Option<AuthoredCampaignStart>, AuthoredStartError> {
    let stripped = selected_map
        .strip_prefix("maps/")
        .unwrap_or(selected_map)
        .strip_suffix(".bsp")
        .unwrap_or_else(|| selected_map.strip_prefix("maps/").unwrap_or(selected_map));
    let normalized = format!("maps/{stripped}.bsp");
    let Some(start) = starts.iter().find(|entry| entry.path == normalized) else {
        return Ok(None);
    };
    valid_start_items(&start.start_items)?;
    parse_q2_start_items(&start.start_items);
    Ok(Some(AuthoredCampaignStart {
        target: parse_q2_travel(&start.bsp)?,
        start_items: start.start_items.clone(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn starts() -> Vec<AuthoredStartMap> {
        vec![AuthoredStartMap {
            episode: "e1".to_string(),
            bsp: "base1".to_string(),
            path: "maps/base1.bsp".to_string(),
            title: "Base".to_string(),
            start_items: "weapon_shotgun; weapon_rocketlauncher 5".to_string(),
            singleplayer: true,
            cooperative: false,
            capture_the_flag: false,
        }]
    }

    #[test]
    fn resolves_authored_start_for_map_variants() {
        let catalog = starts();
        for selected in ["base1", "maps/base1", "maps/base1.bsp", "base1.bsp"] {
            let start = authored_campaign_start(&catalog, selected).unwrap().unwrap();
            assert_eq!(start.target.name, "base1");
            assert_eq!(start.start_items, "weapon_shotgun; weapon_rocketlauncher 5");
        }
    }

    #[test]
    fn arbitrary_maps_and_missing_catalogs_stay_direct() {
        let catalog = starts();
        assert_eq!(authored_campaign_start(&catalog, "maps/unknown.bsp").unwrap(), None);
        assert_eq!(authored_campaign_start(&[], "base1").unwrap(), None);
    }

    #[test]
    fn rejects_invalid_start_items() {
        let mut catalog = starts();
        catalog[0].start_items = "not an item!".to_string();
        assert_eq!(
            authored_campaign_start(&catalog, "base1").unwrap_err(),
            AuthoredStartError::InvalidStartItem("not an item!".to_string())
        );
    }
}
