//! Rogue `p_view.c` and `g_sphere.c` presentation state.
//!
//! Port of donor `src/app/bootstrap/effects/q2-view.ts`
//! (Copyright (C) Id Software, Inc. GPL-2.0-or-later).
//!
//! Infrared flicker, nuke-blind blending, and defender-sphere camera
//! overrides. The donor excludes `tracker-pain` from `receive` through the
//! parameter type; here it arrives as an enum variant and is ignored.

use std::collections::{HashMap, HashSet};

use qa_client::view::{perspective_projection, SceneCamera};
use qa_content::q2::missionpacks::types::Q2MissionPackPlayerEffect;
use qa_core::identity::ActorId;
use qa_core::math::{angles_to_axis, vec4, Vec3, Vec4};

/// Sphere-camera override captured from a `sphere-camera` effect.
#[derive(Debug, Clone, PartialEq)]
struct SphereCameraState {
    /// Sphere whose pose overrides the camera.
    sphere: ActorId,
    /// Fallback origin when the sphere pose is unavailable.
    origin: Vec3,
    /// Override view angles.
    angles: Vec3,
}

/// Per-player infrared/nuke/sphere state.
#[derive(Debug, Clone, PartialEq)]
struct PlayerEffects {
    /// Infrared expiry in seconds.
    ir_until: f64,
    /// Nuke-blind expiry in seconds.
    nuke_until: f64,
    /// Active sphere-camera override.
    sphere: Option<SphereCameraState>,
}

/// Player view after effect presentation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2EffectPlayerView {
    /// Camera with any sphere override applied.
    pub camera: SceneCamera,
    /// Whether infrared rendering is active.
    pub infrared: bool,
    /// Damage-blend color override.
    pub blend: Option<Vec4>,
}

/// Rogue player-effect presentation (`Q2EffectViews`).
#[derive(Debug, Clone, Default)]
pub struct Q2EffectViews {
    players: HashMap<ActorId, PlayerEffects>,
}

impl Q2EffectViews {
    /// Empty presentation state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Receive an infrared, nuke-blind, or sphere-camera effect.
    /// `tracker-pain` is not presented here and is ignored.
    pub fn receive(&mut self, event: &Q2MissionPackPlayerEffect) {
        let actor = match event {
            Q2MissionPackPlayerEffect::TrackerPain { .. } => return,
            Q2MissionPackPlayerEffect::NukeBlind { actor, .. }
            | Q2MissionPackPlayerEffect::Ir { actor, .. }
            | Q2MissionPackPlayerEffect::SphereCamera { actor, .. } => actor,
        };
        let state = self.players.entry(actor.clone()).or_insert(PlayerEffects {
            ir_until: 0.0,
            nuke_until: 0.0,
            sphere: None,
        });
        match event {
            Q2MissionPackPlayerEffect::Ir { until, .. } => state.ir_until = *until,
            Q2MissionPackPlayerEffect::NukeBlind { until, .. } => state.nuke_until = *until,
            Q2MissionPackPlayerEffect::SphereCamera {
                sphere, origin, angles, ..
            } => {
                state.sphere = sphere.as_ref().map(|sphere| SphereCameraState {
                    sphere: sphere.clone(),
                    origin: *origin,
                    angles: *angles,
                });
            }
            Q2MissionPackPlayerEffect::TrackerPain { .. } => {}
        }
    }

    /// Present one player's camera for the current frame.
    ///
    /// `pose` resolves live sphere origins. The 140-degree sphere lens only
    /// patches the projection's focal entries; a degenerate viewport keeps
    /// the incoming projection (the donor range-checks the same inputs).
    pub fn frame(
        &self,
        actor: &ActorId,
        camera: &SceneCamera,
        seconds: f64,
        pose: &dyn Fn(&ActorId) -> Option<Vec3>,
    ) -> Q2EffectPlayerView {
        let Some(state) = self.players.get(actor) else {
            return Q2EffectPlayerView {
                camera: *camera,
                infrared: false,
                blend: None,
            };
        };
        let frames = (state.ir_until * 10.0).round() as i64 - (seconds * 10.0).round() as i64;
        let infrared = frames > 0 && (frames > 30 || frames & 4 != 0);
        let nuke_alpha = ((state.nuke_until - seconds) / 2.0).clamp(0.0, 1.0);
        let alpha = nuke_alpha + (1.0 - nuke_alpha) * if infrared { 0.2 } else { 0.0 };
        let fraction = if alpha > 0.0 { nuke_alpha / alpha } else { 0.0 };
        let blend = if alpha > 0.0 {
            Some(vec4(1.0, fraction as f32, fraction as f32, alpha as f32))
        } else {
            None
        };
        let mut selected = *camera;
        if let Some(sphere) = &state.sphere {
            let viewport = &camera.viewport;
            let vertical = ((f64::from(viewport.height) / f64::from(viewport.width))
                * (140.0 * std::f64::consts::PI / 360.0).tan())
            .atan()
                * 360.0
                / std::f64::consts::PI;
            if let Ok(lens) = perspective_projection(140.0, vertical as f32, 16384.0, 4.0) {
                let mut projection = camera.projection;
                projection[0] = lens[0];
                projection[5] = lens[5];
                selected.projection = projection;
            }
            selected.origin = pose(&sphere.sphere).unwrap_or(sphere.origin);
            selected.axis = angles_to_axis(sphere.angles);
        }
        Q2EffectPlayerView {
            camera: selected,
            infrared,
            blend,
        }
    }

    /// Drop players (and sphere overrides) outside the live actor set.
    pub fn retain(&mut self, actors: &HashSet<ActorId>) {
        self.players.retain(|actor, state| {
            if !actors.contains(actor) {
                return false;
            }
            if let Some(sphere) = &state.sphere {
                if !actors.contains(&sphere.sphere) {
                    state.sphere = None;
                }
            }
            true
        });
    }

    /// Clear every player effect.
    pub fn clear(&mut self) {
        self.players.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_client::view::{CameraClip, Rect};
    use qa_core::identity::IdentityOwner;
    use qa_core::math::{vec3, Axis};

    fn owner() -> IdentityOwner {
        IdentityOwner::create("q2-view").expect("owner")
    }

    fn camera() -> SceneCamera {
        SceneCamera {
            origin: vec3(1.0, 2.0, 3.0),
            axis: [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)],
            projection: [0.0; 16],
            viewport: Rect {
                x: 0,
                y: 0,
                width: 640,
                height: 480,
            },
            clip: CameraClip::None,
        }
    }

    fn axis_axes() -> Axis {
        angles_to_axis(vec3(0.0, 90.0, 0.0))
    }

    #[test]
    fn unknown_player_passes_camera_through() {
        let owner = owner();
        let actor = owner.actor(1, 0);
        let views = Q2EffectViews::new();
        let view = views.frame(&actor, &camera(), 12.0, &|_| None);
        assert_eq!(view.camera, camera());
        assert!(!view.infrared && view.blend.is_none());
    }

    #[test]
    fn tracker_pain_is_ignored() {
        let owner = owner();
        let actor = owner.actor(1, 0);
        let mut views = Q2EffectViews::new();
        views.receive(&Q2MissionPackPlayerEffect::TrackerPain {
            actor: actor.clone(),
            until: 99.0,
        });
        let view = views.frame(&actor, &camera(), 1.0, &|_| None);
        assert!(!view.infrared && view.blend.is_none());
    }

    #[test]
    fn infrared_flickers_after_thirty_frames() {
        let owner = owner();
        let actor = owner.actor(1, 0);
        let mut views = Q2EffectViews::new();
        views.receive(&Q2MissionPackPlayerEffect::Ir {
            actor: actor.clone(),
            until: 10.0,
        });
        // 31 frames remain: steady on with a 0.2 blend.
        let view = views.frame(&actor, &camera(), 6.9, &|_| None);
        assert!(view.infrared);
        assert_eq!(view.blend, Some(vec4(1.0, 0.0, 0.0, 0.2)));
        // 20 frames remain with bit 2 set: on.
        let view = views.frame(&actor, &camera(), 8.0, &|_| None);
        assert!(view.infrared);
        // 24 frames remain with bit 2 clear: off.
        let view = views.frame(&actor, &camera(), 7.6, &|_| None);
        assert!(!view.infrared && view.blend.is_none());
        // Expired: off.
        let view = views.frame(&actor, &camera(), 10.0, &|_| None);
        assert!(!view.infrared && view.blend.is_none());
    }

    #[test]
    fn nuke_blend_ramps_down_over_two_seconds() {
        let owner = owner();
        let actor = owner.actor(1, 0);
        let mut views = Q2EffectViews::new();
        views.receive(&Q2MissionPackPlayerEffect::NukeBlind {
            actor: actor.clone(),
            until: 10.0,
        });
        let view = views.frame(&actor, &camera(), 9.0, &|_| None);
        assert!(!view.infrared);
        assert_eq!(view.blend, Some(vec4(1.0, 1.0, 1.0, 0.5)));
        let view = views.frame(&actor, &camera(), 12.0, &|_| None);
        assert!(view.blend.is_none());
    }

    #[test]
    fn infrared_and_nuke_mix_through_alpha() {
        let owner = owner();
        let actor = owner.actor(1, 0);
        let mut views = Q2EffectViews::new();
        views.receive(&Q2MissionPackPlayerEffect::Ir {
            actor: actor.clone(),
            until: 100.0,
        });
        views.receive(&Q2MissionPackPlayerEffect::NukeBlind {
            actor: actor.clone(),
            until: 11.0,
        });
        // Nuke alpha 0.5 plus infrared 0.2 over the remainder: 0.6 total.
        let view = views.frame(&actor, &camera(), 10.0, &|_| None);
        assert!(view.infrared);
        let blend = view.blend.expect("blend");
        assert!((blend.w - 0.6).abs() < 1e-6);
        assert!((blend.y - (0.5 / 0.6)).abs() < 1e-6);
    }

    #[test]
    fn sphere_camera_overrides_origin_axis_and_lens() {
        let owner = owner();
        let actor = owner.actor(1, 0);
        let sphere = owner.actor(7, 0);
        let mut views = Q2EffectViews::new();
        views.receive(&Q2MissionPackPlayerEffect::SphereCamera {
            actor: actor.clone(),
            sphere: Some(sphere.clone()),
            origin: vec3(9.0, 9.0, 9.0),
            angles: vec3(0.0, 90.0, 0.0),
        });
        let live = vec3(4.0, 5.0, 6.0);
        let view = views.frame(&actor, &camera(), 1.0, &|id| {
            assert_eq!(id, &sphere);
            Some(live)
        });
        assert_eq!(view.camera.origin, live);
        assert_eq!(view.camera.axis, axis_axes());
        let lens = perspective_projection(140.0, 128.226, 16384.0, 4.0).expect("lens");
        assert!((view.camera.projection[0] - lens[0]).abs() < 1e-3);
        assert!((view.camera.projection[5] - lens[5]).abs() < 1e-3);
        // Missing pose falls back to the effect origin.
        let view = views.frame(&actor, &camera(), 1.0, &|_| None);
        assert_eq!(view.camera.origin, vec3(9.0, 9.0, 9.0));
    }

    #[test]
    fn null_sphere_clears_override() {
        let owner = owner();
        let actor = owner.actor(1, 0);
        let mut views = Q2EffectViews::new();
        views.receive(&Q2MissionPackPlayerEffect::SphereCamera {
            actor: actor.clone(),
            sphere: Some(owner.actor(7, 0)),
            origin: vec3(9.0, 9.0, 9.0),
            angles: vec3(0.0, 90.0, 0.0),
        });
        views.receive(&Q2MissionPackPlayerEffect::SphereCamera {
            actor: actor.clone(),
            sphere: None,
            origin: vec3(0.0, 0.0, 0.0),
            angles: vec3(0.0, 0.0, 0.0),
        });
        let view = views.frame(&actor, &camera(), 1.0, &|_| None);
        assert_eq!(view.camera, camera());
    }

    #[test]
    fn retain_drops_gone_players_and_spheres() {
        let owner = owner();
        let actor = owner.actor(1, 0);
        let gone = owner.actor(2, 0);
        let sphere = owner.actor(7, 0);
        let mut views = Q2EffectViews::new();
        views.receive(&Q2MissionPackPlayerEffect::Ir {
            actor: actor.clone(),
            until: 100.0,
        });
        views.receive(&Q2MissionPackPlayerEffect::Ir {
            actor: gone.clone(),
            until: 100.0,
        });
        views.receive(&Q2MissionPackPlayerEffect::SphereCamera {
            actor: actor.clone(),
            sphere: Some(sphere.clone()),
            origin: vec3(0.0, 0.0, 0.0),
            angles: vec3(0.0, 0.0, 0.0),
        });
        // Gone player drops; live player keeps infrared but loses the sphere.
        views.retain(&HashSet::from([actor.clone()]));
        let view = views.frame(&gone, &camera(), 1.0, &|_| None);
        assert!(!view.infrared);
        let view = views.frame(&actor, &camera(), 1.0, &|_| Some(vec3(1.0, 1.0, 1.0)));
        assert!(view.infrared);
        assert_eq!(view.camera.origin, vec3(1.0, 2.0, 3.0));
        views.clear();
        let view = views.frame(&actor, &camera(), 1.0, &|_| None);
        assert!(!view.infrared);
    }
}
