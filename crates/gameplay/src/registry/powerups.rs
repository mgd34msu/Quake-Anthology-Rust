use super::{ItemDef, RegistryError};
use qa_core::primitives::{ItemId, PowerupId, RuleSetId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PowerupDef {
    pub id: PowerupId,
    pub source: RuleSetId,
    /// Original Q3 PW_* ordinal, not an item-list index.
    pub native: u16,
    pub item: ItemId,
    /// Active invulnerability does not describe whether its holdable is owned.
    pub owns_item: bool,
}

/// bg_public.h PW_* and bg_misc.c bg_itemlist, including Team Arena.
/// Invulnerability's active state uses the existing holdable item identity.
const Q3_ITEMS: [(u16, u16); 14] = [
    (1, 28),  // QUAD
    (2, 29),  // BATTLESUIT
    (3, 30),  // HASTE
    (4, 31),  // INVIS
    (5, 32),  // REGEN
    (6, 33),  // FLIGHT
    (7, 34),  // REDFLAG
    (8, 35),  // BLUEFLAG
    (9, 46),  // NEUTRALFLAG
    (10, 42), // SCOUT
    (11, 43), // GUARD
    (12, 44), // DOUBLER
    (13, 45), // AMMOREGEN
    (14, 38), // INVULNERABILITY
];

pub(super) fn load(items: &[ItemDef]) -> Result<Box<[PowerupDef]>, RegistryError> {
    Q3_ITEMS
        .into_iter()
        .map(|(native, item)| {
            let item = items
                .iter()
                .find(|entry| entry.source == RuleSetId::Quake3 && entry.native == item)
                .ok_or(RegistryError::Powerup)?;
            Ok(PowerupDef {
                id: PowerupId(u16::try_from(item.id.0).map_err(|_| RegistryError::Powerup)?),
                source: RuleSetId::Quake3,
                native,
                item: item.id,
                owns_item: native != 14,
            })
        })
        .collect()
}
