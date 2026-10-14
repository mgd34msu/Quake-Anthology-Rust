//! Native text placement and lightstyle arithmetic, independent of geometry.
use qa_core::primitives::{LightStyle, RuleSetId, ThinkTime, Vec3};

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
fn style_value(rules: RuleSetId, value: Option<f32>) -> LightStyle {
    let q1 = matches!(rules, RuleSetId::Quake | RuleSetId::QuakeWorld);
    let (rgb, indexed_scale) = if q1 {
        let scale = value.map_or(256, |value| value as i32);
        ([scale as f32 / 256.0; 3], scale as u16)
    } else {
        let value = value.unwrap_or(1.0);
        ([value; 3], ((value + value + value) * 128.0) as u16)
    };
    LightStyle { rgb, indexed_scale }
}
fn style_letter(rules: RuleSetId, letter: u8) -> f32 {
    let value = i32::from(letter) - i32::from(b'a');
    if matches!(rules, RuleSetId::Quake | RuleSetId::QuakeWorld) {
        (value * 22) as f32
    } else {
        value as f32 / 12.0
    }
}
pub fn lightstyle(rules: RuleSetId, pattern: &[u8], time: ThinkTime) -> Option<LightStyle> {
    if rules == RuleSetId::Quake3 {
        return None;
    }
    let tick = if matches!(rules, RuleSetId::Quake | RuleSetId::QuakeWorld) {
        (time.seconds() * 10.0) as i64
    } else {
        time.milliseconds() / 100
    };
    let value = (!pattern.is_empty()).then(|| {
        style_letter(
            rules,
            pattern[tick.rem_euclid(pattern.len() as i64) as usize],
        )
    });
    Some(style_value(rules, value))
}

/// One native interpolation history per style and client. Retain the target
/// after a transition ends, including repeated letters and unchanged offsets.
#[derive(Clone, Copy, Default)]
pub struct StyleAnimation {
    previous: Option<i64>,
    from: f32,
    to: f32,
    end_ms: i64,
}
impl StyleAnimation {
    pub fn sample(
        &mut self,
        rules: RuleSetId,
        pattern: &[u8],
        time: ThinkTime,
        mode: i32,
    ) -> Option<LightStyle> {
        let current = lightstyle(rules, pattern, time)?;
        if mode <= 0 || pattern.len() < 2 {
            self.previous = None;
            return Some(current);
        }
        if matches!(rules, RuleSetId::Quake | RuleSetId::QuakeWorld) {
            let phase = time.seconds() * 10.0;
            let floor = phase.floor();
            let index = (floor as i64).rem_euclid(pattern.len() as i64) as usize;
            let start = i32::from(pattern[index]) - i32::from(b'a');
            let mut end = i32::from(pattern[(index + 1) % pattern.len()]) - i32::from(b'a');
            if mode < 2 && (end - start).abs() >= 6 {
                end = start;
            }
            let value = f64::from(start * 22) + f64::from((end - start) * 22) * (phase - floor);
            return Some(style_value(rules, Some(value as i32 as f32)));
        }
        let now = time.milliseconds();
        let offset = now / 100;
        if self.previous.is_none_or(|old| offset < old) {
            self.from = current.rgb[0];
            self.to = self.from;
            self.end_ms = now;
        } else if self.previous != Some(offset) {
            self.from = self.to;
            self.to = current.rgb[0];
            self.end_ms = if self.from != self.to {
                now.saturating_add(100)
            } else {
                now
            };
        }
        self.previous = Some(offset);
        let value = if self.end_ms > now {
            let fraction = 1.0 - (self.end_ms - now) as f32 / 100.0;
            qa_core::math::weighted_lerp(self.from, self.to, fraction)
        } else {
            self.to
        };
        Some(style_value(rules, Some(value)))
    }
}
