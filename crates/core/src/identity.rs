//! Generational identity handles ported from `src/contracts/identity.ts`.
//! Handles carry their issuing session token; only the registry capability
//! ([`IdentityOwner`]) mints them, so a handle from another session never
//! compares equal and cannot be forged by gameplay code.

use std::sync::atomic::{AtomicU64, Ordering};

use thiserror::Error;

static NEXT_SESSION_TOKEN: AtomicU64 = AtomicU64::new(1);

/// Error for invalid identity components.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IdentityError {
    /// A session identity needs a name.
    #[error("A session identity needs a name")]
    EmptyName,
    /// The actor belongs to another session.
    #[error("Actor belongs to another session")]
    ForeignActor,
}

/// Stable implementation name (`namespace:name`), separate from
/// session-owned handles.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProviderId {
    /// Namespace portion.
    pub namespace: String,
    /// Name portion.
    pub name: String,
}

impl ProviderId {
    /// Build an implementation name from its two parts.
    #[must_use]
    pub fn new(namespace: &str, name: &str) -> Self {
        Self {
            namespace: namespace.to_string(),
            name: name.to_string(),
        }
    }
}

/// Session handle. Equality covers the issuing token, not just the name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionId {
    name: String,
    token: u64,
}

impl SessionId {
    /// Session name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Generational actor handle.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ActorId {
    session: u64,
    slot: u32,
    generation: u32,
}

impl ActorId {
    /// Registry slot.
    #[must_use]
    pub fn slot(&self) -> u32 {
        self.slot
    }

    /// Slot generation.
    #[must_use]
    pub fn generation(&self) -> u32 {
        self.generation
    }
}

/// Local-seat handle.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SeatId {
    session: u64,
    index: u32,
}

impl SeatId {
    /// Seat index.
    #[must_use]
    pub fn index(&self) -> u32 {
        self.index
    }
}

/// Connected-client handle.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ClientId {
    session: u64,
    slot: u32,
    generation: u32,
}

impl ClientId {
    /// Client slot.
    #[must_use]
    pub fn slot(&self) -> u32 {
        self.slot
    }

    /// Slot generation.
    #[must_use]
    pub fn generation(&self) -> u32 {
        self.generation
    }
}

/// Actor handle bound to its owning provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedActor {
    id: ActorId,
    owner: ProviderId,
    session: u64,
}

impl OwnedActor {
    /// The bound actor handle.
    #[must_use]
    pub fn id(&self) -> &ActorId {
        &self.id
    }

    /// The owning provider.
    #[must_use]
    pub fn owner(&self) -> &ProviderId {
        &self.owner
    }
}

/// Save-safe actor reference: slot plus generation without the live token.
/// Restoring into another session needs a fresh authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SavedActorId {
    /// Registry slot.
    pub slot: u32,
    /// Slot generation.
    pub generation: u32,
}

impl From<&ActorId> for SavedActorId {
    fn from(id: &ActorId) -> Self {
        Self {
            slot: id.slot,
            generation: id.generation,
        }
    }
}

/// Registry capability that mints handles for one session. Held by the
/// session registry, never passed to gameplay providers.
#[derive(Debug)]
pub struct IdentityOwner {
    session: SessionId,
}

impl IdentityOwner {
    /// Create a fresh authority. Restoring a session into another process
    /// requires a new owner.
    pub fn create(name: &str) -> Result<Self, IdentityError> {
        if name.is_empty() {
            return Err(IdentityError::EmptyName);
        }
        let token = NEXT_SESSION_TOKEN.fetch_add(1, Ordering::Relaxed);
        Ok(Self {
            session: SessionId {
                name: name.to_string(),
                token,
            },
        })
    }

    /// This authority's session handle.
    #[must_use]
    pub fn session(&self) -> &SessionId {
        &self.session
    }

    /// Mint an actor handle.
    #[must_use]
    pub fn actor(&self, slot: u32, generation: u32) -> ActorId {
        ActorId {
            session: self.session.token,
            slot,
            generation,
        }
    }

    /// Bind an actor handle to its owning provider.
    pub fn owned_actor(&self, id: &ActorId, provider: ProviderId) -> Result<OwnedActor, IdentityError> {
        if id.session != self.session.token {
            return Err(IdentityError::ForeignActor);
        }
        Ok(OwnedActor {
            id: id.clone(),
            owner: provider,
            session: self.session.token,
        })
    }

    /// Mint a seat handle.
    #[must_use]
    pub fn seat(&self, index: u32) -> SeatId {
        SeatId {
            session: self.session.token,
            index,
        }
    }

    /// Mint a client handle.
    #[must_use]
    pub fn client(&self, slot: u32, generation: u32) -> ClientId {
        ClientId {
            session: self.session.token,
            slot,
            generation,
        }
    }

    /// Check that an actor handle belongs to this session.
    #[must_use]
    pub fn owns_actor(&self, id: &ActorId) -> bool {
        id.session == self.session.token
    }

    /// Check that a seat handle belongs to this session.
    #[must_use]
    pub fn owns_seat(&self, id: &SeatId) -> bool {
        id.session == self.session.token
    }

    /// Check that a client handle belongs to this session.
    #[must_use]
    pub fn owns_client(&self, id: &ClientId) -> bool {
        id.session == self.session.token
    }

    /// Check that an owned reference belongs to this session.
    #[must_use]
    pub fn owns_owned(&self, id: &OwnedActor) -> bool {
        id.session == self.session.token
    }
}

/// Value equality for a freshly decoded reference within the same registry.
#[must_use]
pub fn same_actor(left: &ActorId, right: &ActorId) -> bool {
    left == right
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sessions_are_isolated() {
        let first = IdentityOwner::create("a").unwrap();
        let second = IdentityOwner::create("a").unwrap();
        assert_ne!(first.session(), second.session());
        let actor = first.actor(3, 1);
        assert!(first.owns_actor(&actor));
        assert!(!second.owns_actor(&actor));
        assert!(second.owned_actor(&actor, ProviderId::new("q3", "game")).is_err());
        let owned = first.owned_actor(&actor, ProviderId::new("q3", "game")).unwrap();
        assert!(first.owns_owned(&owned));
        assert_eq!(SavedActorId::from(&actor), SavedActorId { slot: 3, generation: 1 });
        assert!(same_actor(&actor, &first.actor(3, 1)));
        assert!(!same_actor(&actor, &first.actor(3, 2)));
        assert!(IdentityOwner::create("").is_err());
    }
}
