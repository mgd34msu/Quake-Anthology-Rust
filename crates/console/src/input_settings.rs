//! Cold-bound input settings from the one cvar registry.
use crate::cvars::Cvars;
use qa_core::primitives::{CvarHandle, RuleSetId};
use qa_input::{InputPolicy, settings_rules};

const INPUT_NAMES: [&str; 21] = [
    "cl_forwardspeed",
    "cl_backspeed",
    "cl_sidespeed",
    "cl_upspeed",
    "cl_yawspeed",
    "cl_pitchspeed",
    "cl_anglespeedkey",
    "cl_movespeedkey",
    "cl_run",
    "sensitivity",
    "cl_mouseAccel",
    "m_yaw",
    "m_pitch",
    "m_side",
    "m_forward",
    "m_filter",
    "cl_freelook",
    "lookstrafe",
    "lookspring",
    "v_centerspeed",
    "v_centermove",
];

#[derive(Clone, Copy)]
pub struct InputHandles([CvarHandle; INPUT_NAMES.len()]);
impl InputHandles {
    pub fn load(vars: &Cvars) -> Result<Self, String> {
        let mut handles = [CvarHandle(0); INPUT_NAMES.len()];
        for (index, name) in INPUT_NAMES.iter().enumerate() {
            handles[index] = vars
                .find(name)
                .ok_or_else(|| format!("missing input cvar {name}"))?;
        }
        Ok(Self(handles))
    }
    pub fn policy(&self, vars: &Cvars, rules: RuleSetId) -> InputPolicy {
        let semantics = settings_rules(rules);
        let value = |index: usize| vars.finite_value_in(self.0[index], rules);
        let integer = |index: usize| vars.integer_in(self.0[index], rules) != 0;
        let speed = [value(0), value(2), value(3)];
        let always_run = if semantics.explicit_run {
            vars.explicit_number(self.0[8], rules)
                .filter(|number| number.is_finite())
                .is_some_and(|number| number != 0.0)
        } else if semantics.integer_run {
            integer(8)
        } else {
            value(8) != 0.0
        };
        InputPolicy {
            rules,
            speed,
            back_speed: if semantics.back_follows_forward {
                vars.explicit_number(self.0[1], rules)
                    .filter(|number| number.is_finite())
                    .unwrap_or(speed[0])
            } else {
                value(1)
            },
            angle_speed: [value(4), value(5)],
            angle_multiplier: value(6),
            move_multiplier: value(7),
            always_run,
            sensitivity: value(9),
            acceleration: value(10),
            mouse_scale: [value(11), value(12)],
            mouse_side: value(13),
            mouse_forward: value(14),
            filter: if semantics.integer_mouse_flags {
                integer(15)
            } else {
                value(15) != 0.0
            },
            freelook: if semantics.integer_mouse_flags {
                integer(16)
            } else {
                value(16) != 0.0
            },
            look_strafe: value(17) != 0.0,
            delta_pitch: 0.0,
            lookspring: value(18) != 0.0,
            center_speed: value(19),
            center_delay: value(20),
        }
    }
}
