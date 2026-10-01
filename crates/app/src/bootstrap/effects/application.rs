//! Transient-effect math ported from
//! `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/effects.ts`.
//!
//! Ports the donor's dependency-free effect logic: the Quake beam model
//! table, the Quake II player muzzle-flash profile table, explosion
//! animation math, the bonus-flash decay, and the shadow-light distance
//! fade, plus the shared effect color constants.
//!
//! The donor `ApplicationEffects` orchestrator (plus `ApplicationEffectFrame`
//! and `UnhandledApplicationEffect`) is NOT ported: it is driven by
//! `SimulationPresentationEvent` (`simulation/types.ts`, unported) and
//! `ApplicationAssets` (`assets.ts`, unported), and fans out to unported
//! siblings (`effects/q2-view.ts`, `effects/q3.ts`, `SceneModelRenderer`,
//! render scene submissions). The `SourceEffectSound` re-export belongs to
//! out-of-scope `effects/q3.ts`.

use qa_content::q1::foundation::types::Q1BeamStyle;
use qa_core::math::Vec3;

/// Zero vector shared by effect origins and directions.
pub const EFFECT_ZERO: Vec3 = Vec3 { x: 0.0, y: 0.0, z: 0.0 };
/// White effect color.
pub const EFFECT_WHITE: Vec3 = Vec3 { x: 1.0, y: 1.0, z: 1.0 };
/// Orange explosion color.
pub const EFFECT_ORANGE: Vec3 = Vec3 { x: 1.0, y: 0.5, z: 0.5 };

/// Quake beam model for a beam style (donor `q1BeamModels`).
#[must_use]
pub fn q1_beam_model(style: Q1BeamStyle) -> &'static str {
    match style {
        Q1BeamStyle::Lightning1 => "progs/bolt.mdl",
        Q1BeamStyle::Lightning2 => "progs/bolt2.mdl",
        Q1BeamStyle::Lightning3 => "progs/bolt3.mdl",
        Q1BeamStyle::Grapple => "progs/beam.mdl",
    }
}

/// Quake II player muzzle-flash light profile (donor `muzzle`).
///
/// The donor adds a `random & 31` radius jitter at push time; callers own
/// that jitter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MuzzleFlashProfile {
    /// Light color.
    pub color: Vec3,
    /// Base light radius before jitter.
    pub radius: f64,
    /// Light duration in seconds.
    pub duration: f64,
}

/// Resolve a Quake II player muzzle-flash profile (donor `muzzle`).
///
/// Returns `None` for flashes with no source definition (15, 21-29, 40+).
#[must_use]
pub fn muzzle_flash_profile(flash: u32, silenced: bool) -> Option<MuzzleFlashProfile> {
    if !((flash <= 20 && flash != 15) || (30..=39).contains(&flash)) {
        return None;
    }
    let color = match flash {
        6 => Vec3 { x: 0.5, y: 0.5, z: 1.0 },
        7 => Vec3 { x: 1.0, y: 0.5, z: 0.2 },
        8 | 4 | 31 => Vec3 { x: 1.0, y: 0.5, z: 0.0 },
        3 => Vec3 {
            x: 1.0,
            y: 0.25,
            z: 0.0,
        },
        12 | 19 | 34 | 9 => Vec3 { x: 0.0, y: 1.0, z: 0.0 },
        10 | 36 => Vec3 { x: 1.0, y: 0.0, z: 0.0 },
        35 => Vec3 {
            x: -1.0,
            y: -1.0,
            z: -1.0,
        },
        17 | 38 => Vec3 { x: 0.0, y: 0.0, z: 1.0 },
        39 => Vec3 { x: 0.0, y: 1.0, z: 1.0 },
        16 | 18 | 20 => Vec3 { x: 1.0, y: 0.5, z: 0.5 },
        30 => Vec3 { x: 0.9, y: 0.7, z: 0.0 },
        _ => Vec3 { x: 1.0, y: 1.0, z: 0.0 },
    };
    let radius = match flash {
        4 => 225.0,
        5 => 250.0,
        3 => 200.0,
        _ if silenced => 100.0,
        _ => 200.0,
    };
    let duration = if flash == 33 || (36..=39).contains(&flash) {
        0.1
    } else if (9..=11).contains(&flash) {
        0.001
    } else if flash == 4 || flash == 5 {
        0.0001
    } else {
        0.0
    };
    Some(MuzzleFlashProfile {
        color,
        radius,
        duration,
    })
}

/// Explosion animation frame and fractional progress (donor
/// `explosionModel` frame math: 100ms per frame).
#[must_use]
pub fn explosion_frame(start_seconds: f64, now_seconds: f64) -> (u32, f64) {
    let fraction = (now_seconds.mul_add(1000.0, 0.0).round() - start_seconds.mul_add(1000.0, 0.0).round()) / 100.0;
    (fraction.floor().max(0.0) as u32, fraction)
}

/// Whether an explosion with `frames` total frames is still live (donor
/// `prepare` explosion filter).
#[must_use]
pub fn explosion_live(frame: u32, frames: u32) -> bool {
    i64::from(frame) < i64::from(frames) - 1
}

/// Explosion presentation kind (donor `Explosion["kind"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExplosionKind {
    /// Polygon explosion with fading alpha and stepped skin.
    Poly,
    /// Miscellaneous fading model.
    Misc,
    /// Constant full-bright flash.
    Flash,
}

/// Explosion model alpha (donor `explosionModel` alpha math).
#[must_use]
pub fn explosion_alpha(kind: ExplosionKind, fraction: f64, frames: u32, frame: u32) -> f64 {
    match kind {
        ExplosionKind::Flash => 1.0,
        ExplosionKind::Misc => {
            if frames < 2 {
                1.0
            } else {
                1.0 - fraction / f64::from(frames - 1)
            }
        }
        ExplosionKind::Poly => (16 - frame.min(16)) as f64 / 16.0,
    }
}

/// Explosion model skin (donor `explosionModel` skin math).
#[must_use]
pub fn explosion_skin(kind: ExplosionKind, frame: u32, skin: u32) -> u32 {
    match kind {
        ExplosionKind::Poly => {
            if frame < 10 {
                frame >> 1
            } else if frame < 13 {
                5
            } else {
                6
            }
        }
        ExplosionKind::Misc | ExplosionKind::Flash => skin,
    }
}

/// WinQuake bonus-flash blend percent (donor `sharedPlayerView`): 50
/// percent, decaying at 100 per second.
#[must_use]
pub fn bonus_flash_percent(until_seconds: f64, now_seconds: f64) -> f64 {
    ((until_seconds - now_seconds) * 100.0).clamp(0.0, 50.0)
}

/// Shadow-light distance fade (donor `shadowSceneLights` fade math).
#[must_use]
pub fn shadow_light_fade(fade_start: f64, fade_end: f64, distance: f64) -> f64 {
    if fade_start <= 1.0 && fade_end <= 1.0 || fade_start > fade_end {
        return 1.0;
    }
    let fraction = (distance / fade_end).clamp(0.0, 1.0);
    let start = fade_start / fade_end;
    if start <= 0.0 {
        fraction
    } else if start < 1.0 {
        let value = ((fraction - start) / (1.0 - start)).clamp(0.0, 1.0);
        1.0 - value * value * (3.0 - 2.0 * value)
    } else if fraction < 1.0 {
        1.0
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beam_models_match_donor_table() {
        assert_eq!(q1_beam_model(Q1BeamStyle::Lightning1), "progs/bolt.mdl");
        assert_eq!(q1_beam_model(Q1BeamStyle::Lightning2), "progs/bolt2.mdl");
        assert_eq!(q1_beam_model(Q1BeamStyle::Lightning3), "progs/bolt3.mdl");
        assert_eq!(q1_beam_model(Q1BeamStyle::Grapple), "progs/beam.mdl");
    }

    #[test]
    fn undefined_flashes_have_no_profile() {
        assert_eq!(muzzle_flash_profile(15, false), None);
        assert_eq!(muzzle_flash_profile(25, false), None);
        assert_eq!(muzzle_flash_profile(40, false), None);
    }

    #[test]
    fn muzzle_profiles_match_donor_tables() {
        let rail = muzzle_flash_profile(7, false).expect("railgun flash");
        assert_eq!(rail.color, Vec3 { x: 1.0, y: 0.5, z: 0.2 });
        assert_eq!((rail.radius, rail.duration), (200.0, 0.0));
        let hyper = muzzle_flash_profile(4, false).expect("hyperblaster flash");
        assert_eq!((hyper.radius, hyper.duration), (225.0, 0.0001));
        let silenced = muzzle_flash_profile(0, true).expect("silenced flash");
        assert_eq!(silenced.radius, 100.0);
        let login = muzzle_flash_profile(9, false).expect("login flash");
        assert_eq!(login.duration, 0.001);
    }

    #[test]
    fn explosion_frames_advance_every_100ms() {
        let (frame, fraction) = explosion_frame(1.0, 1.25);
        assert_eq!((frame, fraction), (2, 2.5));
        assert!(explosion_live(2, 15));
        assert!(!explosion_live(14, 15));
        assert_eq!(explosion_alpha(ExplosionKind::Flash, 3.0, 4, 3), 1.0);
        assert_eq!(explosion_alpha(ExplosionKind::Misc, 1.0, 4, 1), 1.0 - 1.0 / 3.0);
        assert_eq!(explosion_alpha(ExplosionKind::Poly, 0.0, 15, 0), 1.0);
        assert_eq!(explosion_skin(ExplosionKind::Poly, 4, 0), 2);
        assert_eq!(explosion_skin(ExplosionKind::Poly, 11, 0), 5);
        assert_eq!(explosion_skin(ExplosionKind::Poly, 14, 0), 6);
        assert_eq!(explosion_skin(ExplosionKind::Misc, 3, 2), 2);
    }

    #[test]
    fn bonus_flash_decays_to_zero() {
        assert_eq!(bonus_flash_percent(1.5, 1.0), 50.0);
        assert_eq!(bonus_flash_percent(1.5, 1.25), 25.0);
        assert_eq!(bonus_flash_percent(1.5, 2.0), 0.0);
    }

    #[test]
    fn shadow_fade_matches_donor_branches() {
        assert_eq!(shadow_light_fade(0.0, 0.0, 100.0), 1.0);
        assert_eq!(shadow_light_fade(0.0, 500.0, 250.0), 0.5);
        assert_eq!(shadow_light_fade(500.0, 500.0, 250.0), 1.0);
        assert_eq!(shadow_light_fade(500.0, 500.0, 600.0), 0.0);
        let smooth = shadow_light_fade(250.0, 500.0, 375.0);
        assert!((0.0..=1.0).contains(&smooth));
    }
}
