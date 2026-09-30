//! QuakeWorld multicast reception test (`SV_Multicast` visibility).
//!
//! Ported from donor `src/compat/qc/message-routing.ts`
//! (`receivesQuakeWorldMessage`).
//!
//! Local mirrors: [`QwVisibilityScene`] mirrors the
//! `pointLeaf`/`leafCluster`/`clusterVisible` subset of
//! `Q1ClientVisibilityScene`. Destinations reuse
//! `super::presentation_host`.

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use super::presentation_host::{QcMessageDestination, VisibilityScope};

/// Audible multicast radius (1024 units).
pub const PHS_RADIUS: f32 = 1024.0;

/// Leaf-visibility scene behind multicast reception.
pub trait QwVisibilityScene {
    /// Leaf containing a point.
    fn point_leaf(&self, point: Vec3) -> u32;
    /// Cluster containing a leaf.
    fn leaf_cluster(&self, leaf: u32) -> i32;
    /// Whether two clusters connect under a scope.
    fn cluster_visible(&self, from: i32, to: i32, scope: VisibilityScope) -> bool;
}

/// Whether `actor` at `origin` receives a message sent to `destination`.
/// QW multicast uses the client origin and includes nearby PHS listeners.
pub fn receives_quake_world_message<S: QwVisibilityScene>(
    actor: &ActorId,
    destination: &QcMessageDestination,
    origin: Vec3,
    scene: &S,
) -> bool {
    match destination {
        QcMessageDestination::Client { actor: recipient } => actor == recipient,
        QcMessageDestination::Signon => false,
        QcMessageDestination::Broadcast { .. } => true,
        QcMessageDestination::Multicast {
            origin: source,
            visibility,
            reliable: _,
        } => {
            if *visibility == VisibilityScope::All {
                return true;
            }
            let dx = origin.x - source.x;
            let dy = origin.y - source.y;
            let dz = origin.z - source.z;
            if *visibility == VisibilityScope::Phs && dx * dx + dy * dy + dz * dz <= PHS_RADIUS * PHS_RADIUS {
                return true;
            }
            let from = scene.leaf_cluster(scene.point_leaf(*source));
            let to = scene.leaf_cluster(scene.point_leaf(origin));
            scene.cluster_visible(from, to, *visibility)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;
    use qa_core::math::vec3;

    struct FakeScene {
        visible: bool,
    }

    impl QwVisibilityScene for FakeScene {
        fn point_leaf(&self, point: Vec3) -> u32 {
            point.x as u32
        }

        fn leaf_cluster(&self, leaf: u32) -> i32 {
            leaf as i32
        }

        fn cluster_visible(&self, _from: i32, _to: i32, _scope: VisibilityScope) -> bool {
            self.visible
        }
    }

    #[test]
    fn direct_and_broadcast_destinations() {
        let owner = IdentityOwner::create("message-routing").unwrap();
        let (one, two) = (owner.actor(1, 1), owner.actor(2, 1));
        let scene = FakeScene { visible: false };
        let origin = vec3(0.0, 0.0, 0.0);
        assert!(receives_quake_world_message(
            &one,
            &QcMessageDestination::Client { actor: one.clone() },
            origin,
            &scene
        ));
        assert!(!receives_quake_world_message(
            &two,
            &QcMessageDestination::Client { actor: one.clone() },
            origin,
            &scene
        ));
        assert!(!receives_quake_world_message(
            &one,
            &QcMessageDestination::Signon,
            origin,
            &scene
        ));
        assert!(receives_quake_world_message(
            &one,
            &QcMessageDestination::Broadcast { reliable: false },
            origin,
            &scene
        ));
        assert!(receives_quake_world_message(
            &one,
            &QcMessageDestination::Multicast {
                origin,
                visibility: VisibilityScope::All,
                reliable: false
            },
            origin,
            &scene
        ));
    }

    #[test]
    fn phs_includes_nearby_listeners() {
        let owner = IdentityOwner::create("message-routing").unwrap();
        let actor = owner.actor(1, 1);
        let scene = FakeScene { visible: false };
        let near = QcMessageDestination::Multicast {
            origin: vec3(0.0, 0.0, 0.0),
            visibility: VisibilityScope::Phs,
            reliable: false,
        };
        assert!(receives_quake_world_message(
            &actor,
            &near,
            vec3(100.0, 0.0, 0.0),
            &scene
        ));
        assert!(!receives_quake_world_message(
            &actor,
            &near,
            vec3(2000.0, 0.0, 0.0),
            &scene
        ));
    }

    #[test]
    fn pvs_defers_to_cluster_visibility() {
        let owner = IdentityOwner::create("message-routing").unwrap();
        let actor = owner.actor(1, 1);
        let dest = QcMessageDestination::Multicast {
            origin: vec3(0.0, 0.0, 0.0),
            visibility: VisibilityScope::Pvs,
            reliable: true,
        };
        // Far away so the PHS shortcut cannot apply even to PHS scopes; PVS
        // always consults the scene.
        assert!(receives_quake_world_message(
            &actor,
            &dest,
            vec3(5000.0, 0.0, 0.0),
            &FakeScene { visible: true }
        ));
        assert!(!receives_quake_world_message(
            &actor,
            &dest,
            vec3(5000.0, 0.0, 0.0),
            &FakeScene { visible: false }
        ));
    }
}
