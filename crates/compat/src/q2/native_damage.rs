//! Port of `src/compat/q2/native-damage.ts`.
//! Bridges native damage flags and means-of-death causes into world combat values.

use std::fmt::{Display, Formatter};

use qa_world::combat::{ArmorDamageFlags, DamageCause, Delivery, attack_damage_flags};

/// Native cause edition selecting the obituary roster.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CauseEdition {
    /// Classic module roster.
    Classic,
    /// Rerelease module roster.
    Rerelease,
}

/// Stored native cause captured from a Q2 attack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredNativeCause {
    /// Roster edition.
    pub edition: CauseEdition,
    /// Classic game tag (`base`, `xatrix`, ...); `None` for rerelease.
    pub game: Option<String>,
    /// Means of death id.
    pub means_of_death: u32,
}

/// Cause profile of the native module receiving the damage call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2NativeCauseProfile {
    /// Roster edition.
    pub edition: CauseEdition,
    /// Classic game tag.
    pub game: Option<String>,
}

/// Damage request entering a Q2 native call.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeDamageRequest {
    /// Canonical world cause.
    pub cause: DamageCause,
    /// Canonical means of death id.
    pub means_of_death: u32,
    /// Captured native cause, when the attack is already Q2.
    pub native: Option<StoredNativeCause>,
    /// Damage delivery.
    pub delivery: Delivery,
}

/// Control-flow error: the damage target retired during armor protection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovedNativeDamage {
    /// Means of death of the interrupted request.
    pub means_of_death: u32,
    /// Delivery of the interrupted request.
    pub delivery: Delivery,
}

impl RemovedNativeDamage {
    /// Capture the interrupted request identity.
    #[must_use]
    pub fn for_request(request: &NativeDamageRequest) -> Self {
        Self {
            means_of_death: request.means_of_death,
            delivery: request.delivery,
        }
    }
}

impl Display for RemovedNativeDamage {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "native damage target was removed during armor protection"
        )
    }
}

impl std::error::Error for RemovedNativeDamage {}

/// Flags at the original armor callsite, after the source has applied its
/// damage gates.
#[must_use]
pub fn q2_native_armor_flags(flags: i32) -> ArmorDamageFlags {
    ArmorDamageFlags {
        stage: None,
        no_armor: flags & 2 != 0,
        no_power_armor: flags & 0x100 != 0,
        no_regular_armor: flags & 0x80 != 0,
        energy: flags & 4 != 0,
        regular_protection_scale: 1.0,
    }
}

/// Canonical means of death for a stored native cause.
#[must_use]
pub const fn canonical_cause_from_native(stored: &StoredNativeCause) -> u32 {
    stored.means_of_death
}

/// Stored native cause for a canonical means of death under a roster.
#[must_use]
pub fn native_cause_from_canonical(profile: &Q2NativeCauseProfile, means_of_death: u32) -> StoredNativeCause {
    StoredNativeCause {
        edition: profile.edition,
        game: profile.game.clone(),
        means_of_death,
    }
}

/// Lowered native damage call arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeDamageArguments {
    /// Native damage flags word.
    pub damage_flags: i32,
    /// Native cause passed to the module.
    pub native: StoredNativeCause,
}

/// Lower a damage request into a Q2 native call. Foreign attacks keep their
/// provenance; only the arguments entering Q2 are lowered. `MOD_UNKNOWN` is
/// the authored fallback for attacks absent from the module roster.
#[must_use]
pub fn q2_native_damage_arguments(
    request: &NativeDamageRequest,
    profile: &Q2NativeCauseProfile,
) -> NativeDamageArguments {
    if let DamageCause::Q2 { damage_flags } = &request.cause {
        let captured = request.native.as_ref();
        let same_source = captured.is_some_and(|stored| {
            stored.edition == profile.edition
                && (profile.edition != CauseEdition::Classic || stored.game == profile.game)
        });
        let native = match captured {
            Some(stored)
                if same_source && canonical_cause_from_native(stored) == request.means_of_death =>
            {
                stored.clone()
            }
            _ => native_cause_from_canonical(profile, request.means_of_death),
        };
        return NativeDamageArguments {
            damage_flags: *damage_flags,
            native,
        };
    }
    let flags = attack_damage_flags(&request.cause);
    let mut damage_flags = match request.delivery {
        Delivery::Radius => 1,
        Delivery::Direct => 0,
    };
    if flags.armor.no_armor {
        damage_flags |= 2;
    }
    if flags.armor.energy {
        damage_flags |= 4;
    }
    if flags.no_knockback {
        damage_flags |= 8;
    }
    if flags.no_protection {
        damage_flags |= 32;
    }
    NativeDamageArguments {
        damage_flags,
        native: native_cause_from_canonical(profile, 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> Q2NativeCauseProfile {
        Q2NativeCauseProfile {
            edition: CauseEdition::Classic,
            game: Some("xatrix".to_string()),
        }
    }

    #[test]
    fn decodes_armor_callsite_flags() {
        let flags = q2_native_armor_flags(2 | 4 | 0x80 | 0x100);
        assert!(flags.no_armor);
        assert!(flags.energy);
        assert!(flags.no_regular_armor);
        assert!(flags.no_power_armor);
        let clear = q2_native_armor_flags(0);
        assert!(!clear.no_armor && !clear.energy && !clear.no_regular_armor && !clear.no_power_armor);
    }

    #[test]
    fn keeps_same_source_causes() {
        let stored = StoredNativeCause {
            edition: CauseEdition::Classic,
            game: Some("xatrix".to_string()),
            means_of_death: 7,
        };
        let request = NativeDamageRequest {
            cause: DamageCause::Q2 { damage_flags: 9 },
            means_of_death: 7,
            native: Some(stored.clone()),
            delivery: Delivery::Direct,
        };
        let lowered = q2_native_damage_arguments(&request, &profile());
        assert_eq!(lowered.damage_flags, 9);
        assert_eq!(lowered.native, stored);
    }

    #[test]
    fn remaps_cross_source_causes() {
        let request = NativeDamageRequest {
            cause: DamageCause::Q2 { damage_flags: 3 },
            means_of_death: 11,
            native: Some(StoredNativeCause {
                edition: CauseEdition::Rerelease,
                game: None,
                means_of_death: 11,
            }),
            delivery: Delivery::Direct,
        };
        let lowered = q2_native_damage_arguments(&request, &profile());
        assert_eq!(lowered.damage_flags, 3);
        assert_eq!(lowered.native.edition, CauseEdition::Classic);
        assert_eq!(lowered.native.game.as_deref(), Some("xatrix"));
    }

    #[test]
    fn lowers_foreign_attacks_with_mod_unknown_fallback() {
        let request = NativeDamageRequest {
            cause: DamageCause::Q3 { damage_flags: 2 | 4 },
            means_of_death: 5,
            native: None,
            delivery: Delivery::Radius,
        };
        let lowered = q2_native_damage_arguments(&request, &profile());
        assert_eq!(lowered.damage_flags & 1, 1);
        assert_eq!(lowered.damage_flags & 2, 2);
        assert_eq!(lowered.damage_flags & 8, 8);
        assert_eq!(lowered.native.means_of_death, 0);
        let removed = RemovedNativeDamage::for_request(&request);
        assert_eq!(removed.means_of_death, 5);
        assert_eq!(
            removed.to_string(),
            "native damage target was removed during armor protection"
        );
    }
}
