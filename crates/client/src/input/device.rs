//! MIDI and source-device cvar registration.
//!
//! Donor provenance: `src/input/device-settings.ts`
//! (`registerMidiSettings`, `registerSourceInputSettings`).

use qa_core::cvar::{flags, q2_flags, CvarError, CvarRegistry};

const MIDI_DEFAULTS: [(&str, &str); 5] = [
    ("in_midi", "0"),
    ("in_midiport", "1"),
    ("in_midichannel", "1"),
    ("in_mididevice", "0"),
    ("in_midiseat", "1"),
];

const JOYSTICK_DEFAULTS: [(&str, &str, u32); 9] = [
    ("in_mouse", "1", flags::ARCHIVE),
    ("in_dgamouse", "1", flags::ARCHIVE),
    ("in_subframe", "1", flags::ARCHIVE),
    ("in_nograb", "0", flags::NONE),
    ("in_joystick", "0", flags::ARCHIVE | flags::LATCH),
    ("in_debugjoystick", "0", flags::TEMPORARY),
    ("joy_threshold", "0.15", flags::ARCHIVE),
    ("in_joyBallScale", "0.02", flags::ARCHIVE),
    ("in_joystickSeat", "1", flags::ARCHIVE),
];

fn joystick_profile_default() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else {
        "linux"
    }
}

/// Every cvar name owned by MIDI and source-device settings.
#[must_use]
pub fn input_device_cvar_names() -> Vec<String> {
    MIDI_DEFAULTS
        .iter()
        .map(|(name, _)| (*name).to_string())
        .chain(JOYSTICK_DEFAULTS.iter().map(|(name, _, _)| (*name).to_string()))
        .chain(std::iter::once("in_joystickProfile".to_string()))
        .collect()
}

fn register_missing(cvars: &mut CvarRegistry, name: &str, value: &str, flags: u32) -> Result<(), CvarError> {
    if cvars.get(name).is_none() || cvars.is_console_created(name) {
        cvars.register(name, value, flags)?;
    }
    Ok(())
}

/// Register the `in_midi*` settings.
pub fn register_midi_settings(cvars: &mut CvarRegistry) -> Result<(), CvarError> {
    for (name, value) in MIDI_DEFAULTS {
        register_missing(cvars, name, value, flags::ARCHIVE)?;
    }
    Ok(())
}

/// Register source mouse/joystick settings with per-dialect flags.
pub fn register_source_input_settings(cvars: &mut CvarRegistry) -> Result<(), CvarError> {
    let dialect = cvars.dialect();
    let mut entries: Vec<(&str, &str, u32)> = JOYSTICK_DEFAULTS.to_vec();
    entries.push((
        "in_joystickProfile",
        joystick_profile_default(),
        flags::ARCHIVE | flags::LATCH,
    ));
    for (name, value, entry_flags) in entries {
        let mapped = if dialect.is_q2() {
            (if entry_flags & flags::ARCHIVE != 0 {
                q2_flags::ARCHIVE
            } else {
                0
            }) | (if entry_flags & flags::LATCH != 0 {
                q2_flags::LATCH
            } else {
                0
            })
        } else if dialect == qa_core::cmd::Dialect::Q3 {
            entry_flags
        } else {
            entry_flags & flags::ARCHIVE
        };
        register_missing(cvars, name, value, mapped)?;
    }
    Ok(())
}

/// Register MIDI and source-device settings together.
pub fn register_input_device_cvars(cvars: &mut CvarRegistry) -> Result<(), CvarError> {
    register_midi_settings(cvars)?;
    register_source_input_settings(cvars)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::cmd::Dialect;

    #[test]
    fn registers_defaults_once() {
        let mut cvars = CvarRegistry::new(Dialect::Q3);
        register_input_device_cvars(&mut cvars).unwrap();
        assert_eq!(cvars.get("in_midi").unwrap().value, "0");
        assert_eq!(cvars.get("in_midichannel").unwrap().value, "1");
        assert_eq!(cvars.get("joy_threshold").unwrap().value, "0.15");
        assert_eq!(
            cvars.get("in_joystickProfile").unwrap().value,
            joystick_profile_default()
        );
        assert_eq!(cvars.get("in_mouse").unwrap().flags & flags::ARCHIVE, flags::ARCHIVE);
        cvars.set("joy_threshold", "0.5", false).unwrap();
        register_input_device_cvars(&mut cvars).unwrap();
        assert_eq!(cvars.get("joy_threshold").unwrap().value, "0.5");
        assert!(input_device_cvar_names().contains(&"in_joystickSeat".to_string()));
    }

    #[test]
    fn maps_flags_per_dialect() {
        let mut q2 = CvarRegistry::new(Dialect::Q2Classic);
        register_source_input_settings(&mut q2).unwrap();
        assert_eq!(
            q2.get("in_joystick").unwrap().flags,
            q2_flags::ARCHIVE | q2_flags::LATCH
        );
        assert_eq!(q2.get("in_debugjoystick").unwrap().flags, q2_flags::NONE);
        let mut q1 = CvarRegistry::new(Dialect::Q1Netquake);
        register_source_input_settings(&mut q1).unwrap();
        assert_eq!(q1.get("in_joystick").unwrap().flags, flags::ARCHIVE);
        assert_eq!(q1.get("in_nograb").unwrap().flags, flags::NONE);
    }
}
