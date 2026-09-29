//! Source area-stop order from `AAS_PredictClientMovement`, ported from
//! `src/bots/navigation/aas-prediction-stop.ts`, checked before the final
//! ground/liquid tests.

use qa_core::math::Vec3;

use crate::aas::{aas_trace_areas, AasAsset};
use crate::behavior::BotMovementStop;
use crate::error::BotsError;

/// Classify a predicted segment endpoint against area stop events.
pub fn aas_prediction_stop(
    asset: &AasAsset,
    start: Vec3,
    end: Vec3,
    frame: i32,
    events: i32,
    stop_area: i32,
) -> Result<Option<BotMovementStop>, BotsError> {
    if (events & (512 | 128 | 256 | 4096)) == 0 {
        return Ok(None);
    }
    for crossing in aas_trace_areas(asset, start, end, 20)? {
        let contents = asset
            .settings
            .get(crossing.area as usize)
            .map_or(0, |setting| setting.contents);
        if (events & 512) != 0 && crossing.area == stop_area {
            return Ok(Some(BotMovementStop {
                events: 512,
                origin: crossing.point,
                area: Some(crossing.area),
            }));
        }
        if (events & 128) != 0 && frame != 0 && (contents & 128) != 0 {
            return Ok(Some(BotMovementStop {
                events: 128,
                origin: crossing.point,
                area: Some(crossing.area),
            }));
        }
        if (events & 256) != 0 && (contents & 64) != 0 {
            return Ok(Some(BotMovementStop {
                events: 256,
                origin: crossing.point,
                area: Some(crossing.area),
            }));
        }
        if (events & 4096) != 0 && (contents & 8) != 0 {
            return Ok(Some(BotMovementStop {
                events: 4096,
                origin: crossing.point,
                area: Some(crossing.area),
            }));
        }
    }
    Ok(None)
}
