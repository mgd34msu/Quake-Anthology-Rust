//! Q2 rerelease inventory roster resolution through the native API.
//!
//! Donor: `src/compat/q2/rerelease/semantics.ts` — bridges declared item
//! identities into source inventory slots via `Bot_GetItemID`.

use qa_world::combat::ItemId;
use thiserror::Error;

/// Roster resolution failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SemanticsError {
    /// Native item roster mismatch.
    #[error("Native item roster mismatch: {0}")]
    RosterMismatch(String),
    /// Source inventory declaration requires exactly one remaining slot.
    #[error("Source inventory declaration requires exactly one remaining slot")]
    BadRemaining,
    /// Native inventory roster differs from the declared source storage.
    #[error("Native inventory roster differs from the declared source storage")]
    RosterStorageMismatch,
}

/// Inventory capacity rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RosterCapacity {
    /// Ammunition capacity slot.
    Ammo {
        /// `max_ammo` source index.
        source_index: usize,
    },
    /// Fixed capacity.
    Fixed {
        /// Count.
        count: i32,
    },
}

/// Source slot declaration for one roster row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RosterSource {
    /// Resolve through `Bot_GetItemID` with a classname.
    Classname {
        /// Item classname.
        name: String,
    },
    /// Explicit source index.
    Index {
        /// Source index.
        index: usize,
    },
    /// The single unnamed remaining slot.
    Remaining,
}

/// One declared inventory roster row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RosterRow {
    /// Item identity.
    pub item: ItemId,
    /// Source slot declaration.
    pub source: RosterSource,
    /// Capacity rule.
    pub capacity: RosterCapacity,
}

/// Resolved inventory item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedInventoryItem {
    /// Item identity.
    pub item: ItemId,
    /// Source slot.
    pub source_index: usize,
    /// Capacity rule.
    pub capacity: RosterCapacity,
}

/// Resolve declared item identities through the original API or explicit
/// source slots. `lookup` emulates `Bot_GetItemID`.
pub fn rerelease_inventory_items(
    rows: &[RosterRow],
    inventory_count: usize,
    cells_item: &str,
    cells_index: usize,
    lookup: &dyn Fn(&str) -> Option<i32>,
) -> Result<Vec<ResolvedInventoryItem>, SemanticsError> {
    let mut seen = std::collections::HashSet::new();
    let mut items = Vec::new();
    for row in rows {
        if row.source == RosterSource::Remaining {
            continue;
        }
        let index = match &row.source {
            RosterSource::Index { index } => *index as i32,
            RosterSource::Classname { name } => {
                lookup(name).ok_or_else(|| SemanticsError::RosterMismatch(row.item.clone()))?
            }
            RosterSource::Remaining => continue,
        };
        if index < 0 || index as usize >= inventory_count || !seen.insert(index) {
            return Err(SemanticsError::RosterMismatch(row.item.clone()));
        }
        items.push(ResolvedInventoryItem {
            item: row.item.clone(),
            source_index: index as usize,
            capacity: row.capacity,
        });
    }
    if let Some(remaining) = rows.iter().find(|row| row.source == RosterSource::Remaining) {
        let unnamed: Vec<usize> = (0..inventory_count)
            .filter(|index| !seen.contains(&(*index as i32)))
            .collect();
        if unnamed.len() != 1 {
            return Err(SemanticsError::BadRemaining);
        }
        items.push(ResolvedInventoryItem {
            item: remaining.item.clone(),
            source_index: unnamed[0],
            capacity: remaining.capacity,
        });
    }
    items.sort_by_key(|item| item.source_index);
    let cells_slot = items
        .iter()
        .find(|item| item.item == cells_item)
        .map(|item| item.source_index);
    if items.len() != inventory_count || cells_slot != Some(cells_index) {
        return Err(SemanticsError::RosterStorageMismatch);
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows() -> Vec<RosterRow> {
        vec![
            RosterRow {
                item: "q2:none".to_string(),
                source: RosterSource::Index { index: 0 },
                capacity: RosterCapacity::Fixed { count: 0 },
            },
            RosterRow {
                item: "q2:ammo_cells".to_string(),
                source: RosterSource::Classname {
                    name: "ammo_cells".to_string(),
                },
                capacity: RosterCapacity::Ammo { source_index: 4 },
            },
            RosterRow {
                item: "q2:item_tag_token".to_string(),
                source: RosterSource::Remaining,
                capacity: RosterCapacity::Fixed { count: 100 },
            },
        ]
    }

    #[test]
    fn roster_resolves_names_indexes_and_remainder() {
        let items = rerelease_inventory_items(&rows(), 3, "q2:ammo_cells", 1, &|name| {
            (name == "ammo_cells").then_some(1)
        })
        .expect("roster");
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].source_index, 0);
        assert_eq!(items[1].item, "q2:ammo_cells");
        assert_eq!(items[2].item, "q2:item_tag_token");
        assert_eq!(items[2].source_index, 2);
    }

    #[test]
    fn roster_rejects_mismatches() {
        let duplicate = rerelease_inventory_items(&rows(), 3, "q2:ammo_cells", 1, &|_| Some(0));
        assert_eq!(
            duplicate.unwrap_err(),
            SemanticsError::RosterMismatch("q2:ammo_cells".to_string())
        );
        let unknown = rerelease_inventory_items(&rows(), 3, "q2:ammo_cells", 1, &|_| None);
        assert!(matches!(unknown.unwrap_err(), SemanticsError::RosterMismatch(_)));
        let wrong_cells = rerelease_inventory_items(&rows(), 3, "q2:ammo_cells", 2, &|_| Some(1));
        assert_eq!(wrong_cells.unwrap_err(), SemanticsError::RosterStorageMismatch);
        let short = rerelease_inventory_items(&rows(), 4, "q2:ammo_cells", 1, &|_| Some(1));
        assert_eq!(short.unwrap_err(), SemanticsError::BadRemaining);
    }
}
