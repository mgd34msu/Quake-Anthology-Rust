//! Shared navigation entity observers; path data enters in milestone order.
#![forbid(unsafe_code)]
use qa_core::primitives::EntityId;

/// Membership belongs to a common lifetime, never a native pointer or ordinal.
/// Consumers resolve these handles through the one entity table before use.
pub struct Observers {
    rows: Box<[Option<EntityId>]>,
}
impl Observers {
    pub fn load(entity_capacity: usize) -> Self {
        Self {
            rows: vec![None; entity_capacity].into_boxed_slice(),
        }
    }
    pub fn register(&mut self, entity: EntityId) -> bool {
        let Some(row) = self.rows.get_mut(entity.slot as usize) else {
            return false;
        };
        *row = Some(entity);
        true
    }
    pub fn unregister(&mut self, entity: EntityId) -> bool {
        let Some(row) = self.rows.get_mut(entity.slot as usize) else {
            return false;
        };
        if *row != Some(entity) {
            return false;
        }
        *row = None;
        true
    }
    pub fn contains(&self, entity: EntityId) -> bool {
        self.rows
            .get(entity.slot as usize)
            .is_some_and(|row| *row == Some(entity))
    }
    pub fn iter(&self) -> impl Iterator<Item = EntityId> + '_ {
        self.rows.iter().copied().flatten()
    }
}
