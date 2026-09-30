//! Legacy presentation enum translation for the 1.16n/1.17 ABI.
//!
//! Provenance: `src/compat/qvm/legacy-presentation.ts` (public base-game enum
//! translation from the extracted id Software 1.16n/1.17 SDK `bg_public.h`).

use super::client_state::AbiProfile;
use crate::error::GuestError;

/// Legacy event index to modern event number.
const EVENTS: [u8; 65] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30,
    31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 48, 49, 50, 51, 53, 54, 55, 56, 57, 58, 59, 60, 61,
    62, 63, 64, 66, 68,
];

/// Translate an event (plus its `~255` flag bits) between ABIs.
///
/// Forward maps legacy to modern; `reverse` maps modern to legacy.
pub fn qvm_event(value: i32, profile: AbiProfile, reverse: bool) -> Result<i32, GuestError> {
    if profile.is_modern() {
        return Ok(value);
    }
    let event = (value & 255) as usize;
    let flags = value & !255;
    let mapped = if reverse {
        EVENTS.iter().position(|entry| usize::from(*entry) == event)
    } else {
        EVENTS.get(event).map(|entry| usize::from(*entry))
    };
    let mapped = mapped.ok_or_else(|| {
        GuestError::invalid(format!(
            "QVM event {event} is not represented by the selected legacy ABI"
        ))
    })?;
    Ok(mapped as i32 | flags)
}

/// Translate an entity type between ABIs.
pub fn qvm_entity_type(value: i32, profile: AbiProfile, reverse: bool) -> Result<i32, GuestError> {
    if profile.is_modern() {
        return Ok(value);
    }
    let (source_events, target_events) = if reverse { (13, 12) } else { (12, 13) };
    if value >= source_events {
        return Ok(target_events + qvm_event(value - source_events, profile, reverse)?);
    }
    if reverse && value == 12 {
        return Err(GuestError::invalid("Legacy QVM has no team entity type"));
    }
    Ok(value)
}

/// Map persistent player slots from a legacy record to modern slots.
///
/// Source-only reward and accuracy counters stay zeroed in the output.
pub fn qvm_persistent(values: &[i32; 16], _profile: AbiProfile) -> [i32; 16] {
    if _profile.is_modern() {
        return *values;
    }
    let mut result = [0i32; 16];
    result[0] = values[0];
    result[1] = values[1];
    result[2] = values[2];
    result[3] = values[3];
    result[4] = values[4];
    result[6] = values[7];
    result[8] = values[8];
    result[9] = values[9];
    result[10] = values[10];
    result[13] = values[11];
    result
}

/// Map a guest configstring index to its canonical source index.
pub fn qvm_configstring(index: i32, profile: AbiProfile) -> Result<i32, GuestError> {
    if profile.is_modern() {
        return Ok(index);
    }
    if (12..=15).contains(&index) {
        return Ok(index + 8);
    }
    if (16..=26).contains(&index) {
        return Err(GuestError::invalid(format!(
            "Legacy private configstring {index} has no declared modern presentation mapping"
        )));
    }
    Ok(index)
}

/// Validate powerup bits against the legacy ABI (only the low 9 bits map).
pub fn qvm_powerup_bits(bits: i32, profile: AbiProfile) -> Result<i32, GuestError> {
    if !profile.is_modern() && (bits & !0x1ff) != 0 {
        return Err(GuestError::invalid(
            "Legacy ball or private powerup has no modern presentation mapping",
        ));
    }
    Ok(bits)
}

/// Validate powerup slots against the legacy ABI (slots past 8 must be zero).
pub fn qvm_powerups(values: &[i32; 16], profile: AbiProfile) -> Result<[i32; 16], GuestError> {
    if !profile.is_modern() && values[9..].iter().any(|value| *value != 0) {
        return Err(GuestError::invalid(
            "Legacy ball or private powerup has no modern presentation mapping",
        ));
    }
    Ok(*values)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modern_is_identity() {
        assert_eq!(qvm_event(0x1234, AbiProfile::Modern, false).unwrap(), 0x1234);
        assert_eq!(qvm_entity_type(20, AbiProfile::Modern, true).unwrap(), 20);
        assert_eq!(qvm_configstring(15, AbiProfile::Modern).unwrap(), 15);
        assert_eq!(qvm_powerup_bits(0xFFFF, AbiProfile::Modern).unwrap(), 0xFFFF);
        let slots = [7i32; 16];
        assert_eq!(qvm_persistent(&slots, AbiProfile::Modern), slots);
        assert_eq!(qvm_powerups(&slots, AbiProfile::Modern).unwrap(), slots);
    }

    #[test]
    fn legacy_event_round_trip_preserves_flags() {
        let legacy = qvm_event(47 | 0x300, AbiProfile::Legacy, false).unwrap();
        assert_eq!(legacy, 48 | 0x300);
        let back = qvm_event(legacy, AbiProfile::Legacy, true).unwrap();
        assert_eq!(back, 47 | 0x300);
    }

    #[test]
    fn legacy_event_rejects_unmapped() {
        assert!(qvm_event(47, AbiProfile::Legacy, true).is_err());
        assert!(qvm_event(200, AbiProfile::Legacy, false).is_err());
    }

    #[test]
    fn legacy_entity_type_shifts_events() {
        assert_eq!(qvm_entity_type(5, AbiProfile::Legacy, false).unwrap(), 5);
        assert_eq!(qvm_entity_type(12, AbiProfile::Legacy, false).unwrap(), 13);
        assert_eq!(qvm_entity_type(13, AbiProfile::Legacy, true).unwrap(), 12);
        assert!(qvm_entity_type(12, AbiProfile::Legacy, true).is_err());
    }

    #[test]
    fn legacy_persistent_remaps_slots() {
        let values: [i32; 16] = core::array::from_fn(|index| index as i32 + 1);
        let mapped = qvm_persistent(&values, AbiProfile::Legacy);
        assert_eq!([mapped[0], mapped[1], mapped[2], mapped[3], mapped[4]], [1, 2, 3, 4, 5]);
        assert_eq!(mapped[6], 8);
        assert_eq!(mapped[8], 9);
        assert_eq!(mapped[13], 12);
        assert_eq!(mapped[5], 0);
        assert_eq!(mapped[15], 0);
    }

    #[test]
    fn legacy_configstring_maps_and_rejects() {
        assert_eq!(qvm_configstring(11, AbiProfile::Legacy).unwrap(), 11);
        assert_eq!(qvm_configstring(12, AbiProfile::Legacy).unwrap(), 20);
        assert_eq!(qvm_configstring(15, AbiProfile::Legacy).unwrap(), 23);
        assert!(qvm_configstring(16, AbiProfile::Legacy).is_err());
        assert!(qvm_configstring(26, AbiProfile::Legacy).is_err());
        assert_eq!(qvm_configstring(27, AbiProfile::Legacy).unwrap(), 27);
    }

    #[test]
    fn legacy_powerups_reject_private_bits() {
        assert_eq!(qvm_powerup_bits(0x1ff, AbiProfile::Legacy).unwrap(), 0x1ff);
        assert!(qvm_powerup_bits(0x200, AbiProfile::Legacy).is_err());
        let mut slots = [0i32; 16];
        slots[8] = 5;
        assert!(qvm_powerups(&slots, AbiProfile::Legacy).is_ok());
        slots[9] = 1;
        assert!(qvm_powerups(&slots, AbiProfile::Legacy).is_err());
    }
}
