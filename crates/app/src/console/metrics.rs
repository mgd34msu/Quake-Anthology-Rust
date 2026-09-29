//! Console grid metrics: scale, line height, and column count.
//!
//! Donor provenance: `src/console/metrics.ts` (`consoleCellWidth`,
//! `consoleMetrics`). Same automatic scale (`height / ratio / 300`,
//! minimum 2), explicit-scale clamp (`1..=4`), and fit rules. The glyph
//! inputs (atlas advance, line height, cap ink) arrive as plain numbers so
//! this module stays independent of the text atlas; per-glyph fitting
//! (`consoleGlyphMetrics`) stays with that display-side port.

/// Computed console grid geometry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConsoleMetrics {
    /// Integer pixel scale.
    pub scale: f64,
    /// Line height in pixels (`8 * scale`).
    pub line_height: f64,
    /// Cell width in pixels.
    pub cell_width: f64,
    /// Text columns that fit the console width.
    pub columns: f64,
}

/// Cell width for a line height, keeping the source glyph aspect ratio.
///
/// `advance` is the `M` advance, `atlas_line_height` the atlas line
/// height, and `cap_height` the cap-ink height when the glyph is text.
#[must_use]
pub fn console_cell_width(advance: f64, atlas_line_height: f64, cap_height: Option<f64>, line_height: f64) -> f64 {
    let normalization = cap_height.map_or(1.0, |cap| 6.0 / cap);
    (advance * line_height / atlas_line_height * normalization).ceil()
}

/// Inputs for [`console_metrics`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConsoleMetricsOptions {
    /// Viewport width in pixels.
    pub width: f64,
    /// Viewport height in pixels.
    pub height: f64,
    /// Device pixel ratio (clamped to at least 1).
    pub pixel_ratio: f64,
    /// Requested scale (positive finite enables explicit mode).
    pub requested_scale: f64,
    /// `M` advance from the text atlas.
    pub advance: f64,
    /// Atlas line height from the text atlas.
    pub atlas_line_height: f64,
    /// Cap-ink height from the text atlas, when the glyph is text.
    pub cap_height: Option<f64>,
}

/// Console grid geometry for a viewport and scale request.
#[must_use]
pub fn console_metrics(options: &ConsoleMetricsOptions) -> ConsoleMetrics {
    let ratio = options.pixel_ratio.max(1.0);
    let automatic = (options.height / ratio / 300.0).floor().max(2.0);
    let explicit = options.requested_scale.is_finite() && options.requested_scale > 0.0;
    let requested = if explicit {
        options.requested_scale.round().clamp(1.0, 4.0)
    } else {
        automatic
    };
    let fit = if explicit {
        (options.width / 40.0).min(options.height / 16.0).floor().max(1.0)
    } else {
        (options.width / 256.0).min(options.height / 96.0).floor().max(1.0)
    };
    let scale = fit.min((requested * ratio).round()).max(1.0);
    let line_height = 8.0 * scale;
    let cell_width = console_cell_width(
        options.advance,
        options.atlas_line_height,
        options.cap_height,
        line_height,
    );
    ConsoleMetrics {
        scale,
        line_height,
        cell_width,
        columns: ((options.width - 16.0 * scale) / cell_width).floor().max(1.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> ConsoleMetricsOptions {
        ConsoleMetricsOptions {
            width: 1280.0,
            height: 720.0,
            pixel_ratio: 1.0,
            requested_scale: f64::NAN,
            advance: 8.0,
            atlas_line_height: 8.0,
            cap_height: Some(6.0),
        }
    }

    #[test]
    fn derives_automatic_scale_and_columns() {
        let metrics = console_metrics(&options());
        assert_eq!(metrics.scale, 2.0);
        assert_eq!(metrics.line_height, 16.0);
        assert_eq!(metrics.cell_width, 16.0);
        assert_eq!(metrics.columns, 78.0);
    }

    #[test]
    fn clamps_explicit_scale_to_the_viewport() {
        let mut explicit = options();
        explicit.requested_scale = 9.0;
        explicit.width = 100.0;
        explicit.height = 40.0;
        let metrics = console_metrics(&explicit);
        assert_eq!(metrics.scale, 2.0);
        assert!(metrics.columns >= 1.0);

        let mut tiny = options();
        tiny.width = 10.0;
        tiny.height = 10.0;
        assert_eq!(console_metrics(&tiny).columns, 1.0);
    }
}
