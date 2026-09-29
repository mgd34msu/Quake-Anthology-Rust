//! Legacy reliable-command translation for client configstrings.
//!
//! Provenance: `src/compat/qvm/legacy-client-abi.ts` (reliable source
//! commands carry canonical configstring indices into the client ABI).

use super::client_state::AbiProfile;
use crate::error::GuestError;

/// Translate a reliable server-command argv into the selected client ABI.
///
/// Only `cs` commands change under the legacy profile: indices 20-23 shift
/// down by 8, while indices 12-19 and 24-26 have no legacy mapping.
pub fn legacy_client_command(argv: &[String], profile: AbiProfile) -> Result<Vec<String>, GuestError> {
    if profile.is_modern() || argv.first().is_none_or(|head| head != "cs") {
        return Ok(argv.to_vec());
    }
    let index: i32 = argv
        .get(1)
        .and_then(|text| text.parse().ok())
        .ok_or_else(|| GuestError::invalid("Invalid configstring command index"))?;
    if (20..=23).contains(&index) {
        let mut translated = vec!["cs".to_string(), (index - 8).to_string()];
        translated.extend_from_slice(&argv[2..]);
        return Ok(translated);
    }
    if (12..=26).contains(&index) {
        return Err(GuestError::invalid(format!("Configstring {index} has no legacy client mapping")));
    }
    Ok(argv.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn modern_is_identity() {
        let command = argv(&["cs", "20", "x"]);
        assert_eq!(legacy_client_command(&command, AbiProfile::Modern).unwrap(), command);
    }

    #[test]
    fn non_cs_is_identity() {
        let command = argv(&["print", "20"]);
        assert_eq!(legacy_client_command(&command, AbiProfile::Legacy).unwrap(), command);
        let empty: Vec<String> = Vec::new();
        assert_eq!(legacy_client_command(&empty, AbiProfile::Legacy).unwrap(), empty);
    }

    #[test]
    fn legacy_shifts_server_indices() {
        assert_eq!(
            legacy_client_command(&argv(&["cs", "20", "v"]), AbiProfile::Legacy).unwrap(),
            argv(&["cs", "12", "v"])
        );
        assert_eq!(
            legacy_client_command(&argv(&["cs", "23"]), AbiProfile::Legacy).unwrap(),
            argv(&["cs", "15"])
        );
        assert_eq!(
            legacy_client_command(&argv(&["cs", "5", "v"]), AbiProfile::Legacy).unwrap(),
            argv(&["cs", "5", "v"])
        );
    }

    #[test]
    fn legacy_rejects_unmapped_and_invalid() {
        assert!(legacy_client_command(&argv(&["cs", "12"]), AbiProfile::Legacy).is_err());
        assert!(legacy_client_command(&argv(&["cs", "19", "v"]), AbiProfile::Legacy).is_err());
        assert!(legacy_client_command(&argv(&["cs", "24"]), AbiProfile::Legacy).is_err());
        assert!(legacy_client_command(&argv(&["cs", "nope"]), AbiProfile::Legacy).is_err());
        assert!(legacy_client_command(&argv(&["cs"]), AbiProfile::Legacy).is_err());
    }
}
