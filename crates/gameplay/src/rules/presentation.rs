//! Native text placement and lightstyle arithmetic, independent of geometry.
use qa_core::primitives::{RuleSetId, ThinkTime, Vec3};

#[derive(Clone, Copy)]
enum PunchRules {
    None,
    Current,
    Interpolated,
}

/// Native view construction is selected from the accepted presentation source,
/// independently of movement, trace and the loaded world.
#[derive(Clone, Copy)]
pub struct ViewRules {
    punch: PunchRules,
    weapon_follows_view: bool,
    kick_setting: bool,
}

const VIEW: [ViewRules; 5] = [
    // WinQuake view.c builds the gun before adding the clientdata punch vector.
    ViewRules {
        punch: PunchRules::Current,
        weapon_follows_view: false,
        kick_setting: false,
    },
    // QW's separate smallkick/bigkick messages are not playerinfo punch fields.
    ViewRules {
        punch: PunchRules::None,
        weapon_follows_view: false,
        kick_setting: false,
    },
    // Q2 CL_CalcViewValues adds interpolated kick before CL_AddViewWeapon.
    ViewRules {
        punch: PunchRules::Interpolated,
        weapon_follows_view: true,
        kick_setting: false,
    },
    // q2repro CL_SetupFirstPersonView retains the native cl_kickangles switch.
    ViewRules {
        punch: PunchRules::Interpolated,
        weapon_follows_view: true,
        kick_setting: true,
    },
    // Q3's cgame-local weapon kick is not a transmitted playerState punch.
    ViewRules {
        punch: PunchRules::None,
        weapon_follows_view: true,
        kick_setting: false,
    },
];

pub fn view_rules(source: RuleSetId) -> ViewRules {
    VIEW[source as usize]
}

impl ViewRules {
    pub fn interpolate_punch(self) -> bool {
        matches!(self.punch, PunchRules::Interpolated)
    }
    pub fn angles(
        self,
        base: Vec3,
        current: Vec3,
        previous: Option<Vec3>,
        fraction: f32,
        enabled: bool,
    ) -> Vec3 {
        if self.kick_setting && !enabled {
            return base;
        }
        let punch = match self.punch {
            PunchRules::None => return base,
            PunchRules::Current => current,
            PunchRules::Interpolated => previous.map_or(current, |previous| {
                Vec3(std::array::from_fn(|axis| {
                    qa_core::math::lerp_angle(previous.0[axis], current.0[axis], fraction)
                }))
            }),
        };
        base + punch
    }
    pub fn weapon_angles(self, accepted: Vec3, presented: Vec3) -> Vec3 {
        if self.weapon_follows_view {
            presented
        } else {
            accepted
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ClockRules {
    /// Native Q2 uses remaining milliseconds times its precomputed 0.01.
    /// Other native clocks use the interval reciprocal, in a different order.
    pub remaining_ms_scale: Option<f32>,
}
pub fn clock_rules(rules: RuleSetId) -> ClockRules {
    ClockRules {
        remaining_ms_scale: (rules == RuleSetId::Quake2).then_some(0.01),
    }
}

#[derive(Clone, Copy)]
pub struct TextRules {
    pub colors: bool,
    pub glyph_height: u32,
    pub center: bool,
    pub layouts: bool,
    pub layout_stat: Option<usize>,
}
const TEXT: [TextRules; 5] = [
    TextRules {
        colors: false,
        glyph_height: 8,
        center: true,
        layouts: false,
        layout_stat: None,
    },
    TextRules {
        colors: false,
        glyph_height: 8,
        center: true,
        layouts: false,
        layout_stat: None,
    },
    TextRules {
        colors: false,
        glyph_height: 8,
        center: true,
        layouts: true,
        layout_stat: Some(13),
    },
    TextRules {
        colors: false,
        glyph_height: 8,
        center: true,
        layouts: true,
        layout_stat: Some(13),
    },
    TextRules {
        colors: true,
        glyph_height: 16,
        center: false,
        layouts: false,
        layout_stat: None,
    },
];
pub fn text_rules(rules: RuleSetId) -> TextRules {
    TEXT[rules as usize]
}

#[derive(Clone, Copy)]
pub struct HudNumberRules {
    pub client_clock: bool,
    pub minimum_armor: i32,
}
pub fn hud_number_rules(source: RuleSetId) -> HudNumberRules {
    match source {
        RuleSetId::Quake2Rerelease => HudNumberRules {
            client_clock: true,
            minimum_armor: 0,
        },
        _ => HudNumberRules {
            client_clock: false,
            minimum_armor: 1,
        },
    }
}
impl HudNumberRules {
    pub fn flash(self, server_frame: u32, time_ms: u64) -> bool {
        if self.client_clock {
            time_ms % 1000 < 500
        } else {
            (server_frame >> 2) & 1 != 0
        }
    }
}
pub fn center_y(height: u32, lines: usize) -> i32 {
    if lines <= 4 {
        (f64::from(height) * 0.35) as i32
    } else {
        48
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StyleSample {
    pub rgb: [f32; 3],
    pub indexed_scale: u16,
    /// Rerelease interpolation remains a separate presentation requirement.
    pub interpolation_unimplemented: bool,
}
pub fn lightstyle(rules: RuleSetId, pattern: &[u8], time: ThinkTime) -> Option<StyleSample> {
    let q1 = matches!(rules, RuleSetId::Quake | RuleSetId::QuakeWorld);
    if rules == RuleSetId::Quake3 {
        return None;
    }
    let value = if pattern.is_empty() {
        None
    } else {
        let tick = if q1 {
            (time.seconds() * 10.0) as i64
        } else {
            time.milliseconds() / 100
        };
        Some(i32::from(pattern[tick.rem_euclid(pattern.len() as i64) as usize]) - i32::from(b'a'))
    };
    let (rgb, indexed_scale) = if q1 {
        let scale = value.map_or(256, |value| value * 22);
        ([scale as f32 / 256.0; 3], scale as u16)
    } else {
        let value = value.map_or(1.0, |value| value as f32 / 12.0);
        ([value; 3], ((value + value + value) * 128.0) as u16)
    };
    Some(StyleSample {
        rgb,
        indexed_scale,
        interpolation_unimplemented: rules == RuleSetId::Quake2Rerelease && pattern.len() > 1,
    })
}
