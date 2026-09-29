//! Quake I water transition.
//!
//! Donor provenance: `src/movement/q1/water-transition.ts`.

/// Water transition outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q1WaterTransition {
    /// New water type.
    pub water_type: i32,
    /// New water level.
    pub water_level: i32,
    /// Whether the transition splashes.
    pub splash: bool,
}

/// `SV_CheckWaterTransition` preserves source initialization and empty
/// waterlevel values.
#[must_use]
pub fn q1_water_transition(previous_type: i32, contents: i32) -> Q1WaterTransition {
    if previous_type == 0 {
        return Q1WaterTransition {
            water_type: contents,
            water_level: 1,
            splash: false,
        };
    }
    if contents <= -3 {
        return Q1WaterTransition {
            water_type: contents,
            water_level: 1,
            splash: previous_type == -1,
        };
    }
    Q1WaterTransition {
        water_type: -1,
        water_level: contents,
        splash: previous_type != -1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialization_sets_level_without_splash() {
        assert_eq!(
            q1_water_transition(0, -3),
            Q1WaterTransition {
                water_type: -3,
                water_level: 1,
                splash: false,
            }
        );
    }

    #[test]
    fn entering_water_from_air_splashes() {
        let wet = q1_water_transition(-1, -3);
        assert_eq!((wet.water_type, wet.water_level, wet.splash), (-3, 1, true));
        let stays = q1_water_transition(-3, -4);
        assert_eq!((stays.water_type, stays.water_level, stays.splash), (-4, 1, false));
    }

    #[test]
    fn leaving_water_keeps_contents_as_level() {
        let dry = q1_water_transition(-3, -1);
        assert_eq!((dry.water_type, dry.water_level, dry.splash), (-1, -1, true));
        let idle = q1_water_transition(-1, -1);
        assert_eq!((idle.water_type, idle.water_level, idle.splash), (-1, -1, false));
    }
}
