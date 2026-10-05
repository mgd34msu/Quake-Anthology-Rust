//! Shared shadow-source setting.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/render-settings.ts`
//! (`registerRenderSettings`). The merged [`CvarRegistry`](qa_core::cvar::CvarRegistry) has no
//! binding or documentation surface, so the validator and documentation are ported as explicit
//! module items with the donor's exact texts; registration installs the variable.

use qa_core::cmd::Dialect;
use qa_core::cvar::{flags, CvarError, CvarRegistry};

use super::shared_setting_cvars::validate_finite_number;

/// `r_shadows` documentation summary.
pub const R_SHADOWS_DOC_SUMMARY: &str =
    "Quake model shadows projected onto the sampled floor. Zero disables; nonzero enables.";
/// `r_shadows` usage line.
pub const R_SHADOWS_DOC_USAGE: &str = "r_shadows [0|1]";
/// `r_shadows` examples.
pub const R_SHADOWS_DOC_EXAMPLES: [&str; 2] = ["r_shadows 1", "r_shadows 0"];

/// Validate an `r_shadows` value (`None` accepts; donor text otherwise).
#[must_use]
pub fn validate_r_shadows(text: &str, dialect: Dialect) -> Option<String> {
    if validate_finite_number(text, dialect).is_none() {
        None
    } else {
        Some("Shadows require a finite number".to_string())
    }
}

/// Register the shared shadow-source cvar.
pub fn register_render_settings(cvars: &mut CvarRegistry) -> Result<(), CvarError> {
    cvars.register("r_shadows", "0", flags::ARCHIVE)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use qa_core::cmd::Dialect;

    use super::*;

    #[test]
    fn accepts_finite_numbers() {
        // The dialect `atof` consumes a numeric prefix, so trailing junk
        // parses (value 1 for "1x", -16 for "-0x10", 0 for "0x").
        for text in [
            "0", "1", "0.5", "-2", " 1 ", "0x10", "+0x10", "0b101", "0o17", "1e3", "1x", "-0x10", "0x", "1.2.3",
        ] {
            for dialect in [Dialect::Q1Netquake, Dialect::Q2Classic, Dialect::Q3] {
                assert_eq!(validate_r_shadows(text, dialect), None, "{text}");
            }
        }
    }

    #[test]
    fn rejects_non_finite_text() {
        for text in ["", "   ", "abc", "NaN", "Infinity", "-Infinity", ".", "+", "-"] {
            for dialect in [Dialect::Q1Netquake, Dialect::Q2Classic, Dialect::Q3] {
                assert_eq!(
                    validate_r_shadows(text, dialect),
                    Some("Shadows require a finite number".to_string()),
                    "{text}"
                );
            }
        }
    }

    #[test]
    fn registers_shadow_cvar_archived() {
        let mut cvars = CvarRegistry::new(Dialect::Q3);
        register_render_settings(&mut cvars).unwrap();
        let snapshot = cvars.get("r_shadows").expect("registered");
        assert_eq!(snapshot.value, "0");
        assert_eq!(snapshot.reset_value, "0");
        assert_eq!(snapshot.flags & flags::ARCHIVE, flags::ARCHIVE);
    }

    #[test]
    fn documents_shadow_cvar() {
        assert!(R_SHADOWS_DOC_SUMMARY.contains("Zero disables"));
        assert_eq!(R_SHADOWS_DOC_USAGE, "r_shadows [0|1]");
        assert_eq!(R_SHADOWS_DOC_EXAMPLES, ["r_shadows 1", "r_shadows 0"]);
    }
}
