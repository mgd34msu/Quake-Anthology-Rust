//! Startup summary layout.
//!
//! Donor: `src/app/bootstrap/startup-summary.ts` (`startupSummaryLayout`).
//! Pure layout math; infallible like the donor.

use thiserror::Error;

/// Startup summary failure.
///
/// The donor never throws; this enum exists for module convention.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[allow(dead_code)]
pub enum StartupSummaryError {
    /// Unreachable (donor is infallible).
    #[error("unreachable startup summary failure")]
    Unreachable,
}

/// Layout bounds (`bounds`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SummaryBounds {
    /// Origin.
    pub x: f64,
    /// Origin.
    pub y: f64,
    /// Width.
    pub width: f64,
    /// Height.
    pub height: f64,
}

/// Computed summary layout.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StartupSummaryLayout {
    /// Origin.
    pub x: f64,
    /// Origin.
    pub y: f64,
    /// Width.
    pub width: f64,
    /// Row height.
    pub row_height: f64,
    /// Label scale.
    pub label_scale: f64,
    /// Value scale.
    pub value_scale: f64,
    /// Value offset.
    pub value_offset: f64,
}

/// Text layout uses eight logical units per scale; keep both lines and
/// their shadow inside each row.
#[must_use]
pub fn startup_summary_layout(count: f64, bounds: &SummaryBounds) -> StartupSummaryLayout {
    let row_height = 30.0f64.min(bounds.height / 1.0f64.max(count));
    let density = 1.0f64.min((row_height - 2.0) / (8.0 * (1.35 + 2.1) + 1.0));
    let label_scale = 1.35 * density;
    let value_scale = 2.1 * density;
    StartupSummaryLayout {
        x: bounds.x,
        y: bounds.y,
        width: bounds.width,
        row_height,
        label_scale,
        value_scale,
        value_offset: 8.0 * label_scale + density,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds() -> SummaryBounds {
        SummaryBounds { x: 4.0, y: 8.0, width: 320.0, height: 120.0 }
    }

    #[test]
    fn layout_happy_path() {
        let layout = startup_summary_layout(4.0, &bounds());
        assert_eq!(layout.x, 4.0);
        assert_eq!(layout.y, 8.0);
        assert_eq!(layout.width, 320.0);
        assert_eq!(layout.row_height, 30.0);
        let density = (30.0f64 - 2.0) / (8.0 * (1.35 + 2.1) + 1.0);
        assert!((layout.label_scale - 1.35 * density).abs() < 1e-12);
        assert!((layout.value_scale - 2.1 * density).abs() < 1e-12);
        assert!((layout.value_offset - (8.0 * layout.label_scale + density)).abs() < 1e-12);
    }

    #[test]
    fn layout_zero_count_clamps_to_one_row() {
        let layout = startup_summary_layout(0.0, &bounds());
        assert_eq!(layout.row_height, 30.0f64.min(120.0));
    }

    #[test]
    fn layout_zero_height_collapses_density() {
        let flat = SummaryBounds { height: 0.0, ..bounds() };
        let layout = startup_summary_layout(4.0, &flat);
        assert_eq!(layout.row_height, 0.0);
        assert!(layout.label_scale < 0.0);
    }
}
