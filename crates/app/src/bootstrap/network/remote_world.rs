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
}
