//! Points-only armor grants (`src/content/q1/quakec/armor-points.ts`).
//!
//! Donor provenance: `src/content/q1/quakec/armor-points.ts`
//! (`qcEmptyArmor`).

use crate::contract::{ModQcEmptyArmor, ModQcEmptyArmorItem, ModSourceCall, RegularArmorState};

use super::id1_program::{id1_program_snapshot, Id1Attribution};
use super::qc_view::QcProgramView;
use super::{fround, QcError};

/// Points-only armor grant (donor `qcEmptyArmor` closure).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QcEmptyArmorGrant {
    /// Armor item.
    item: ModQcEmptyArmorItem,
    /// Absorption.
    absorption: f64,
}

impl QcEmptyArmorGrant {
    /// Grant armor with `points` (donor closure body).
    #[must_use]
    pub fn grant(&self, points: f64) -> RegularArmorState {
        RegularArmorState::Q1 {
            points,
            absorption: self.absorption,
            item: match self.item {
                ModQcEmptyArmorItem::Armor1 => "q1:item_armor1".to_string(),
                ModQcEmptyArmorItem::Armor2 => "q1:item_armor2".to_string(),
                ModQcEmptyArmorItem::ArmorInv => "q1:item_armorInv".to_string(),
            },
        }
    }
}

/// Original id1 armor_touch red tier; other artifacts declare their
/// points-only grant explicitly (donor `qcEmptyArmor`).
///
/// The declared item is already restricted to the three authored items
/// by [`ModQcEmptyArmorItem`].
pub fn qc_empty_armor(
    program: &QcProgramView,
    declared: Option<&ModQcEmptyArmor>,
    declared_damage: Option<&ModSourceCall>,
) -> Result<Option<QcEmptyArmorGrant>, QcError> {
    let pinned_default = id1_program_snapshot(program, declared_damage)?.attribution == Id1Attribution::Pinned;
    let source: Option<(ModQcEmptyArmorItem, f64)> = match declared {
        Some(declared) => Some((declared.item, declared.absorption)),
        None if pinned_default => Some((ModQcEmptyArmorItem::ArmorInv, 0.8)),
        None => None,
    };
    let Some((item, absorption)) = source else {
        return Ok(None);
    };
    if !fround(absorption).is_finite() || absorption < 0.0 {
        return Err(QcError::program(
            "QC points-only armor requires an authored item and finite nonnegative absorption",
            program.source,
        ));
    }
    Ok(Some(QcEmptyArmorGrant {
        item,
        absorption: fround(absorption),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grant_builds_q1_armor_state() {
        let grant = QcEmptyArmorGrant {
            item: ModQcEmptyArmorItem::Armor2,
            absorption: 0.6,
        };
        let state = grant.grant(150.0);
        assert_eq!(
            state,
            RegularArmorState::Q1 {
                points: 150.0,
                absorption: 0.6,
                item: String::from("q1:item_armor2"),
            }
        );
    }
}
