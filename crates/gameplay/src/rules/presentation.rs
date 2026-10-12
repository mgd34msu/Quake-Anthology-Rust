//! Native text placement and lightstyle arithmetic, independent of geometry.
use qa_core::primitives::{RuleSetId, ThinkTime};

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
