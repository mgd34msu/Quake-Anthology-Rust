//! QuakeC weapon-behavior (trajectory adapter) capability profile.
//!
//! Ported from `src/compat/qc/weapon-behavior-profile.ts`. The program-view
//! mirrors (`QcProgramView`, `QcValueType`) live in `super::mod_provider`,
//! absorbed from `src/compat/qc/program.ts`.
//!
//! Note on `src/contracts/native-weapon-behavior.ts`: the donor profile only
//! references `QcProgram`/`QcValueType`, so no native weapon-behavior types
//! are absorbed here; the QVM-side worker covers that contract.

use super::mod_provider::{QcProgramView, QcValueType};

/// Required entity fields with their types.
#[must_use]
pub fn qc_weapon_behavior_fields() -> Vec<(&'static str, QcValueType)> {
    vec![
        ("origin", QcValueType::Vector),
        ("velocity", QcValueType::Vector),
        ("angles", QcValueType::Vector),
        ("mins", QcValueType::Vector),
        ("maxs", QcValueType::Vector),
        ("v_angle", QcValueType::Vector),
        ("health", QcValueType::Float),
        ("solid", QcValueType::Float),
        ("nextthink", QcValueType::Float),
        ("classname", QcValueType::String),
        ("netname", QcValueType::String),
        ("think", QcValueType::Function),
    ]
}

/// Required globals with their types.
#[must_use]
pub fn qc_weapon_behavior_globals() -> Vec<(&'static str, QcValueType)> {
    vec![("self", QcValueType::Entity), ("other", QcValueType::Entity)]
}

/// Optional entity fields with their required-when-declared types.
#[must_use]
pub fn qc_weapon_behavior_optional_fields() -> Vec<(&'static str, QcValueType)> {
    vec![
        ("model", QcValueType::String),
        ("modelindex", QcValueType::Float),
        ("chain", QcValueType::Entity),
    ]
}

/// Optional globals with their required-when-declared types.
#[must_use]
pub fn qc_weapon_behavior_optional_globals() -> Vec<(&'static str, QcValueType)> {
    vec![
        ("time", QcValueType::Float),
        ("trace_endpos", QcValueType::Vector),
        ("trace_plane_normal", QcValueType::Vector),
        ("trace_ent", QcValueType::Entity),
        ("deathmatch", QcValueType::Float),
        ("coop", QcValueType::Float),
        ("trace_fraction", QcValueType::Float),
        ("trace_allsolid", QcValueType::Float),
        ("trace_startsolid", QcValueType::Float),
        ("trace_inwater", QcValueType::Float),
        ("trace_inopen", QcValueType::Float),
        ("trace_plane_dist", QcValueType::Float),
    ]
}

/// Capability error when the program cannot host the trajectory adapter.
#[must_use]
pub fn qc_weapon_behavior_capability_error(program: &dyn QcProgramView) -> Option<String> {
    for (name, expected) in qc_weapon_behavior_fields() {
        if program.field_type(name) != Some(expected) {
            return Some(format!(
                "QuakeC trajectory adapter requires entity field {name} of type {expected:?}"
            ));
        }
    }
    for (name, expected) in qc_weapon_behavior_globals() {
        if program.global_type(name) != Some(expected) {
            return Some(format!(
                "QuakeC trajectory adapter requires global {name} of type {expected:?}"
            ));
        }
    }
    for (name, expected) in qc_weapon_behavior_optional_fields() {
        if program.field_type(name).is_some_and(|found| found != expected) {
            return Some(format!(
                "QuakeC trajectory adapter requires declared entity field {name} to have type {expected:?}"
            ));
        }
    }
    for (name, expected) in qc_weapon_behavior_optional_globals() {
        if program.global_type(name).is_some_and(|found| found != expected) {
            return Some(format!(
                "QuakeC trajectory adapter requires declared global {name} to have type {expected:?}"
            ));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use super::super::mod_provider::{QcApiKind, QcFunctionView};

    struct FakeProgram {
        fields: HashMap<String, QcValueType>,
        globals: HashMap<String, QcValueType>,
    }

    impl QcProgramView for FakeProgram {
        fn digest(&self) -> &str {
            "abc"
        }

        fn api_kind(&self) -> QcApiKind {
            QcApiKind::Q1Netquake
        }

        fn field_type(&self, name: &str) -> Option<QcValueType> {
            self.fields.get(name).copied()
        }

        fn global_type(&self, name: &str) -> Option<QcValueType> {
            self.globals.get(name).copied()
        }

        fn function_named(&self, _name: &str) -> Option<QcFunctionView> {
            None
        }

        fn function_at(&self, _index: i32) -> Option<QcFunctionView> {
            None
        }

        fn functions(&self) -> Vec<QcFunctionView> {
            Vec::new()
        }
    }

    fn capable() -> FakeProgram {
        FakeProgram {
            fields: qc_weapon_behavior_fields()
                .into_iter()
                .map(|(name, kind)| (name.to_string(), kind))
                .collect(),
            globals: qc_weapon_behavior_globals()
                .into_iter()
                .map(|(name, kind)| (name.to_string(), kind))
                .collect(),
        }
    }

    #[test]
    fn capable_program_has_no_error() {
        assert_eq!(qc_weapon_behavior_capability_error(&capable()), None);
    }

    #[test]
    fn missing_field_reports_capability() {
        let mut program = capable();
        program.fields.remove("health");
        let error = qc_weapon_behavior_capability_error(&program).unwrap();
        assert!(error.contains("health"), "{error}");
    }

    #[test]
    fn mistyped_optional_global_reports_capability() {
        let mut program = capable();
        program.globals.insert("time".to_string(), QcValueType::String);
        let error = qc_weapon_behavior_capability_error(&program).unwrap();
        assert!(error.contains("time"), "{error}");
    }

    #[test]
    fn profile_lists_cover_donor_surface() {
        assert_eq!(qc_weapon_behavior_fields().len(), 12);
        assert_eq!(qc_weapon_behavior_globals().len(), 2);
        assert_eq!(qc_weapon_behavior_optional_fields().len(), 3);
        assert_eq!(qc_weapon_behavior_optional_globals().len(), 12);
    }
}
