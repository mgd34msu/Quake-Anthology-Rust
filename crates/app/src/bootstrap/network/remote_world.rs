//! Remote world content holder.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/network/remote-world.ts`
//! (`RemoteWorldContent`). Server admission supplies the first world;
//! connection setup only needs mounts. The scene queries build lazily from
//! the loaded content and reset whenever content is replaced; collision
//! settings rebind onto the live queries when present.
//!
//! `LoadedApplicationContent`, the scene queries, and the Q3 collision
//! settings live outside this shard, so the holder is generic over content
//! (`C`), scene (`S`), and collision settings (`K`). Callers pass a scene
//! builder; the only behavior this module owns is the lazy-build/reset
//! lifecycle and the register-before-bind collision order.

use std::fmt::Debug;

use qa_content::contract::InventoryEntry;
use qa_core::identity::ProviderId;
use qa_core::math::Vec3;
use thiserror::Error;

/// Remote world content failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RemoteWorldError {
    /// No world has been supplied yet.
    #[error("Remote server has not supplied a world")]
    NoWorld,
}

/// Collision settings hook (donor `CollisionMapSettings::registerMap`).
pub trait RemoteCollisionSettings {
    /// Register the settings against their collision map.
    fn register_map(&mut self);
}

/// Scene builder for [`RemoteWorldContent`].
pub type SceneBuilder<C, S, K> = Box<dyn Fn(&C, Option<&K>) -> S>;

/// Lazily-built remote world content (donor `RemoteWorldContent`).
pub struct RemoteWorldContent<C, S, K> {
    loaded: Option<C>,
    scene: Option<S>,
    collision: Option<K>,
    build: SceneBuilder<C, S, K>,
}

impl<C, S, K> RemoteWorldContent<C, S, K> {
    /// Wrap optional initial content with a scene builder.
    pub fn new(loaded: Option<C>, build: impl Fn(&C, Option<&K>) -> S + 'static) -> Self {
        Self {
            loaded,
            scene: None,
            collision: None,
            build: Box::new(build),
        }
    }

    /// Whether content has been supplied.
    #[must_use]
    pub fn has_content(&self) -> bool {
        self.loaded.is_some()
    }

    /// Bind collision settings, registering them before rebinding the live scene.
    pub fn bind_collision_settings(&mut self, mut settings: K)
    where
        K: RemoteCollisionSettings,
    {
        settings.register_map();
        self.collision = Some(settings);
        if let (Some(loaded), Some(collision)) = (self.loaded.as_ref(), self.collision.as_ref()) {
            self.scene = Some((self.build)(loaded, Some(collision)));
        }
    }

    /// Borrow the loaded content, failing before server admission.
    pub fn content(&self) -> Result<&C, RemoteWorldError> {
        self.loaded.as_ref().ok_or(RemoteWorldError::NoWorld)
    }

    /// Replace the loaded content, retiring the cached scene queries.
    pub fn set_content(&mut self, content: C) {
        self.loaded = Some(content);
        self.scene = None;
    }

    /// Drop the cached scene queries without replacing content (donor
    /// same-content assignment, which nulls the queries).
    pub fn reset_scene(&mut self) {
        self.scene = None;
    }

    /// Borrow the scene queries, building them lazily from the content.
    pub fn scene(&mut self) -> Result<&S, RemoteWorldError> {
        if self.scene.is_none() {
            let loaded = self.loaded.as_ref().ok_or(RemoteWorldError::NoWorld)?;
            let scene = (self.build)(loaded, self.collision.as_ref());
            self.scene = Some(scene);
        }
        self.scene.as_ref().ok_or(RemoteWorldError::NoWorld)
    }

    /// Mutably borrow the scene queries, building them lazily from the content.
    pub fn scene_mut(&mut self) -> Result<&mut S, RemoteWorldError> {
        if self.scene.is_none() {
            let loaded = self.loaded.as_ref().ok_or(RemoteWorldError::NoWorld)?;
            let scene = (self.build)(loaded, self.collision.as_ref());
            self.scene = Some(scene);
        }
        self.scene.as_mut().ok_or(RemoteWorldError::NoWorld)
    }
}

impl<C: Debug, S: Debug, K: Debug> Debug for RemoteWorldContent<C, S, K> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteWorldContent")
            .field("loaded", &self.loaded)
            .field("scene", &self.scene)
            .field("collision", &self.collision)
            .finish()
    }
}

/// Shared origin vector (verbatim copies lived in every remote presentation).
pub(crate) const ZERO: Vec3 = Vec3 { x: 0.0, y: 0.0, z: 0.0 };

/// Split a `namespace:name` provider string (shared remote helper).
pub(crate) fn provider_id(provider: &str) -> ProviderId {
    let (namespace, name) = provider.split_once(':').unwrap_or(("", ""));
    ProviderId::new(namespace, name)
}

/// Convert an `[f64; 3]` triple into a scene vector (shared remote helper).
pub(crate) fn vec3(value: [f64; 3]) -> Vec3 {
    Vec3 {
        x: value[0] as f32,
        y: value[1] as f32,
        z: value[2] as f32,
    }
}

/// Linearly interpolate scene vectors (shared remote helper).
pub(crate) fn lerp_vec(a: Vec3, b: Vec3, fraction: f64) -> Vec3 {
    let fraction = fraction as f32;
    Vec3 {
        x: a.x + (b.x - a.x) * fraction,
        y: a.y + (b.y - a.y) * fraction,
        z: a.z + (b.z - a.z) * fraction,
    }
}

/// Interpolate angles across the 360-degree wrap (shared remote helper).
pub(crate) fn lerp_angles(a: Vec3, b: Vec3, fraction: f64) -> Vec3 {
    let fraction = fraction as f32;
    let axis = |a: f32, b: f32| a + (((b - a + 540.0) % 360.0) - 180.0) * fraction;
    Vec3 {
        x: axis(a.x, b.x),
        y: axis(a.y, b.y),
        z: axis(a.z, b.z),
    }
}

/// Map a HUD inventory entry onto the snapshot entry shape (shared remote helper).
pub(crate) fn snapshot_entry(entry: &InventoryEntry) -> qa_world::inventory::InventoryEntry {
    use qa_content::contract::{InventoryCountPolicy, SourceCounterArithmetic};
    use qa_world::inventory::{CountArithmetic, CountPolicy};
    qa_world::inventory::InventoryEntry {
        item: entry.item.clone(),
        count: entry.count,
        capacity: entry.capacity,
        count_policy: entry.count_policy.map(|policy| match policy {
            InventoryCountPolicy::Stack => CountPolicy::Stack,
            InventoryCountPolicy::SourceCounter(arithmetic) => CountPolicy::SourceCounter(match arithmetic {
                SourceCounterArithmetic::Binary32 => CountArithmetic::Binary32,
                SourceCounterArithmetic::Binary64 => CountArithmetic::Binary64,
                SourceCounterArithmetic::Int32 => CountArithmetic::Int32,
            }),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestCollision {
        registered: bool,
    }

    impl RemoteCollisionSettings for TestCollision {
        fn register_map(&mut self) {
            self.registered = true;
        }
    }

    #[test]
    fn scene_before_admission_fails() {
        let mut world: RemoteWorldContent<String, String, TestCollision> =
            RemoteWorldContent::new(None, |content, _| format!("scene:{content}"));
        assert_eq!(world.content().unwrap_err(), RemoteWorldError::NoWorld);
        assert_eq!(world.scene().unwrap_err(), RemoteWorldError::NoWorld);
    }

    #[test]
    fn scene_builds_lazily_and_caches() {
        let mut world = RemoteWorldContent::new(
            Some("map1".to_string()),
            |content: &String, _: Option<&TestCollision>| format!("scene:{content}"),
        );
        assert!(world.has_content());
        assert_eq!(world.scene().unwrap(), "scene:map1");
        assert_eq!(world.scene().unwrap(), "scene:map1");
    }

    #[test]
    fn reset_scene_rebuilds_queries() {
        use std::cell::Cell;
        use std::rc::Rc;
        let builds = Rc::new(Cell::new(0));
        let counter = builds.clone();
        let mut world = RemoteWorldContent::new(
            Some("map1".to_string()),
            move |content: &String, _: Option<&TestCollision>| {
                counter.set(counter.get() + 1);
                format!("scene:{content}")
            },
        );
        assert_eq!(world.scene().unwrap(), "scene:map1");
        assert_eq!(builds.get(), 1);
        world.reset_scene();
        assert_eq!(world.scene().unwrap(), "scene:map1");
        assert_eq!(builds.get(), 2);
    }

    #[test]
    fn set_content_retires_scene() {
        let mut world = RemoteWorldContent::new(
            Some("map1".to_string()),
            |content: &String, _: Option<&TestCollision>| format!("scene:{content}"),
        );
        assert_eq!(world.scene().unwrap(), "scene:map1");
        world.set_content("map2".to_string());
        assert_eq!(world.content().unwrap(), "map2");
        assert_eq!(world.scene().unwrap(), "scene:map2");
    }

    #[test]
    fn collision_binding_registers_and_rebuilds_scene() {
        let mut world = RemoteWorldContent::new(
            Some("map1".to_string()),
            |content: &String, collision: Option<&TestCollision>| {
                format!(
                    "scene:{content}:{}",
                    collision.map(|settings| settings.registered).unwrap_or(false)
                )
            },
        );
        assert_eq!(world.scene().unwrap(), "scene:map1:false");
        world.bind_collision_settings(TestCollision { registered: false });
        assert_eq!(world.scene().unwrap(), "scene:map1:true");
    }

    #[test]
    fn shared_zero_is_origin() {
        assert_eq!(ZERO, Vec3 { x: 0.0, y: 0.0, z: 0.0 });
    }

    #[test]
    fn shared_provider_id_splits_namespace() {
        let id = provider_id("q1:base");
        assert_eq!((id.namespace.as_str(), id.name.as_str()), ("q1", "base"));
        let empty = provider_id("bare");
        assert_eq!((empty.namespace.as_str(), empty.name.as_str()), ("", ""));
    }

    #[test]
    fn shared_vec3_narrows_triples() {
        assert_eq!(
            vec3([1.5, -2.5, 3.25]),
            Vec3 {
                x: 1.5,
                y: -2.5,
                z: 3.25
            }
        );
    }

    #[test]
    fn shared_lerp_helpers_match_family_math() {
        let from = Vec3 { x: 0.0, y: 0.0, z: 0.0 };
        let to = Vec3 {
            x: 10.0,
            y: 20.0,
            z: 30.0,
        };
        assert_eq!(
            lerp_vec(from, to, 0.5),
            Vec3 {
                x: 5.0,
                y: 10.0,
                z: 15.0
            }
        );
        assert_eq!(
            lerp_angles(
                Vec3 {
                    x: 350.0,
                    y: 0.0,
                    z: 0.0
                },
                Vec3 {
                    x: 10.0,
                    y: 0.0,
                    z: 0.0
                },
                0.5
            ),
            Vec3 {
                x: 360.0,
                y: 0.0,
                z: 0.0
            }
        );
    }

    #[test]
    fn shared_snapshot_entry_maps_policies() {
        use qa_content::contract::{InventoryCountPolicy, SourceCounterArithmetic};
        use qa_world::inventory::{CountArithmetic, CountPolicy};
        let plain = snapshot_entry(&InventoryEntry {
            item: "q1:ammo/shells".to_string(),
            count: 8.0,
            capacity: 100.0,
            count_policy: None,
        });
        assert_eq!(plain.item, "q1:ammo/shells");
        assert_eq!(plain.count, 8.0);
        assert_eq!(plain.count_policy, None);
        let stacked = snapshot_entry(&InventoryEntry {
            item: "item".to_string(),
            count: 1.0,
            capacity: 1.0,
            count_policy: Some(InventoryCountPolicy::Stack),
        });
        assert_eq!(stacked.count_policy, Some(CountPolicy::Stack));
        let counter = snapshot_entry(&InventoryEntry {
            item: "item".to_string(),
            count: 1.0,
            capacity: 1.0,
            count_policy: Some(InventoryCountPolicy::SourceCounter(SourceCounterArithmetic::Binary64)),
        });
        assert_eq!(
            counter.count_policy,
            Some(CountPolicy::SourceCounter(CountArithmetic::Binary64))
        );
    }
}
