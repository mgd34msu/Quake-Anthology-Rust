//! Display output gamma table (Q3 convention).
//!
//! Donor provenance: `src/render/output-gamma.ts` (`outputGammaTable`) with
//! the Q3 row of `buildGammaTable` from `src/formats/images/palette.ts`.
//! Values above one brighten output: each entry maps `i` through
//! `255 * (i / 255)^(1 / gamma)`, rounded half up.

use super::error::RenderError;

/// Lowest accepted display gamma.
pub const MIN_OUTPUT_GAMMA: f32 = 0.5;
/// Highest accepted display gamma.
pub const MAX_OUTPUT_GAMMA: f32 = 3.0;

/// Build the 256-entry display gamma table, or `None` when `gamma` is
/// exactly `1.0` (identity needs no table).
///
/// # Errors
///
/// Returns [`RenderError::Backend`] when `gamma` is not finite or falls
/// outside `0.5..=3`.
pub fn output_gamma_table(gamma: f32) -> Result<Option<[u8; 256]>, RenderError> {
    if !gamma.is_finite() || gamma < MIN_OUTPUT_GAMMA || gamma > MAX_OUTPUT_GAMMA {
        return Err(RenderError::Backend(format!(
            "Output gamma must be between {MIN_OUTPUT_GAMMA} and {MAX_OUTPUT_GAMMA}"
        )));
    }
    if gamma == 1.0 {
        return Ok(None);
    }
    let inverse = 1.0 / gamma;
    let mut table = [0u8; 256];
    for (index, entry) in table.iter_mut().enumerate() {
        let normalized = f32::from(index as u8) / 255.0;
        let value = 255.0 * normalized.powf(inverse) + 0.5;
        *entry = value.floor().clamp(0.0, 255.0) as u8;
    }
    Ok(Some(table))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_gamma_needs_no_table() {
        assert_eq!(output_gamma_table(1.0), Ok(None));
    }

    #[test]
    fn endpoints_are_fixed() {
        for gamma in [0.5, 0.8, 1.5, 2.0, 3.0] {
            let table = output_gamma_table(gamma).unwrap().unwrap();
            assert_eq!(table[0], 0, "gamma {gamma}");
            assert_eq!(table[255], 255, "gamma {gamma}");
        }
    }

    #[test]
    fn above_one_brightens_midtones() {
        let table = output_gamma_table(2.0).unwrap().unwrap();
        assert!(table[64] > 64, "table[64]={}", table[64]);
        assert!(table[128] > 128, "table[128]={}", table[128]);
        let expected = (255.0 * (128.0f32 / 255.0).powf(0.5) + 0.5).floor() as u8;
        assert_eq!(table[128], expected);
    }

    #[test]
    fn below_one_darkens_midtones() {
        let table = output_gamma_table(0.5).unwrap().unwrap();
        assert!(table[64] < 64, "table[64]={}", table[64]);
        assert!(table[128] < 128, "table[128]={}", table[128]);
    }

    #[test]
    fn tables_are_monotonic() {
        for gamma in [0.5, 2.2, 3.0] {
            let table = output_gamma_table(gamma).unwrap().unwrap();
            for window in table.windows(2) {
                assert!(window[0] <= window[1], "gamma {gamma}");
            }
        }
    }

    #[test]
    fn out_of_range_gamma_is_rejected() {
        for gamma in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.0, 0.49, 3.01, 10.0] {
            assert!(
                matches!(output_gamma_table(gamma), Err(RenderError::Backend(_))),
                "gamma {gamma}"
            );
        }
        assert!(output_gamma_table(0.5).is_ok());
        assert!(output_gamma_table(3.0).is_ok());
    }
}
