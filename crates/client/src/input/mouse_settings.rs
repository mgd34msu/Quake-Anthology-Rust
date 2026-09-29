//! Mouse tuning cvar storage.
//!
//! Donor provenance: `src/input/mouse-settings.ts` (`MouseSettings`).
//! The seat registry owns the cvar values; [`super::MouseInput`]
//! owns the live tuning.

use qa_core::cvar::{flags, CvarError, CvarRegistry};

use super::{default_mouse_tuning, MouseTuning};

/// Register mouse and view-centering cvars.
pub fn register_mouse_settings(cvars: &mut CvarRegistry) -> Result<(), CvarError> {
    for (name, value) in [("v_centerspeed", "500"), ("v_centermove", "0.15")] {
        if cvars.get(name).is_none() || cvars.is_console_created(name) {
            cvars.register(name, value, flags::NONE)?;
        }
    }
    let defaults = default_mouse_tuning();
    for (name, value) in [
        ("sensitivity", defaults.sensitivity),
        ("cl_mouseAccel", defaults.acceleration),
        ("m_filter", f64::from(u8::from(defaults.filter))),
        ("m_yaw", defaults.yaw),
        ("m_pitch", defaults.pitch),
        ("m_side", defaults.side),
        ("m_forward", defaults.forward),
        ("lookspring", 0.0),
        ("lookstrafe", 0.0),
        ("freelook", f64::from(u8::from(defaults.free_look))),
    ] {
        if cvars.get(name).is_none() || cvars.is_console_created(name) {
            cvars.register(name, &cvar_number(value), flags::ARCHIVE)?;
        }
    }
    Ok(())
}

fn cvar_number(value: f64) -> String {
    if value == 0.0 && value.is_sign_negative() {
        "-0".to_string()
    } else {
        format!("{value}")
    }
}

/// Read tuning from cvars.
#[must_use]
pub fn read_mouse_tuning(cvars: &CvarRegistry) -> MouseTuning {
    let pitch = f64::from(cvars.variable_value("m_pitch"));
    let filter = if cvars.dialect() == qa_core::cmd::Dialect::Q3 {
        cvars.get("m_filter").map_or(0, |snapshot| snapshot.integer_value) != 0
    } else {
        cvars.variable_value("m_filter") != 0.0
    };
    MouseTuning {
        sensitivity: f64::from(cvars.variable_value("sensitivity")),
        acceleration: f64::from(cvars.variable_value("cl_mouseAccel")),
        filter,
        yaw: f64::from(cvars.variable_value("m_yaw")),
        pitch: pitch.abs(),
        side: f64::from(cvars.variable_value("m_side")),
        forward: f64::from(cvars.variable_value("m_forward")),
        free_look: cvars.variable_value("freelook") != 0.0,
        look_spring: cvars.variable_value("lookspring") != 0.0,
        look_strafe: cvars.variable_value("lookstrafe") != 0.0,
        invert_pitch: pitch < 0.0 || pitch == 0.0 && pitch.is_sign_negative(),
    }
}

/// Write tuning to cvars.
pub fn write_mouse_tuning(cvars: &mut CvarRegistry, value: &MouseTuning) -> Result<(), CvarError> {
    cvars.set("sensitivity", &cvar_number(value.sensitivity), false)?;
    cvars.set("cl_mouseAccel", &cvar_number(value.acceleration), false)?;
    cvars.set("m_filter", if value.filter { "1" } else { "0" }, false)?;
    cvars.set("m_yaw", &cvar_number(value.yaw), false)?;
    let pitch = value.pitch * if value.invert_pitch { -1.0 } else { 1.0 };
    cvars.set("m_pitch", &cvar_number(pitch), false)?;
    cvars.set("m_side", &cvar_number(value.side), false)?;
    cvars.set("m_forward", &cvar_number(value.forward), false)?;
    cvars.set("lookspring", if value.look_spring { "1" } else { "0" }, false)?;
    cvars.set("lookstrafe", if value.look_strafe { "1" } else { "0" }, false)?;
    cvars.set("freelook", if value.free_look { "1" } else { "0" }, false)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::cmd::Dialect;

    #[test]
    fn round_trips_tuning() {
        // Cvar numerics are binary32; tuning round-trips within float
        // precision while flags and signs stay exact.
        let close = |tuning: &MouseTuning, expected: &MouseTuning| {
            for (got, want) in [
                (tuning.sensitivity, expected.sensitivity),
                (tuning.acceleration, expected.acceleration),
                (tuning.yaw, expected.yaw),
                (tuning.pitch, expected.pitch),
                (tuning.side, expected.side),
                (tuning.forward, expected.forward),
            ] {
                assert!((got - want).abs() < 1e-6, "{got} vs {want}");
            }
            assert_eq!(tuning.filter, expected.filter);
            assert_eq!(tuning.free_look, expected.free_look);
            assert_eq!(tuning.look_spring, expected.look_spring);
            assert_eq!(tuning.look_strafe, expected.look_strafe);
            assert_eq!(tuning.invert_pitch, expected.invert_pitch);
        };
        let mut cvars = CvarRegistry::new(Dialect::Q3);
        register_mouse_settings(&mut cvars).unwrap();
        close(&read_mouse_tuning(&cvars), &default_mouse_tuning());
        let mut tuning = default_mouse_tuning();
        tuning.sensitivity = 4.5;
        tuning.filter = true;
        tuning.invert_pitch = true;
        tuning.look_spring = true;
        tuning.free_look = false;
        write_mouse_tuning(&mut cvars, &tuning).unwrap();
        assert_eq!(cvars.get("m_pitch").unwrap().value, "-0.022");
        close(&read_mouse_tuning(&cvars), &tuning);
        let mut cvars = CvarRegistry::new(Dialect::Q1Netquake);
        register_mouse_settings(&mut cvars).unwrap();
        cvars.set("m_filter", "2", false).unwrap();
        assert!(read_mouse_tuning(&cvars).filter);
    }
}
