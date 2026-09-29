//! Q2 rerelease primary protection publication during pickup grants.
//!
//! Donor: `src/compat/q2/rerelease/pickup-protection.ts` — bridges original
//! primary protection stores into held damage cursors while a grant commits.

use qa_world::combat::{ArmorState, PoweredProtection, RegularArmor};
use thiserror::Error;

/// Protection failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProtectionError {
    /// Native protection operation has no live recipient.
    #[error("Native protection operation has no live recipient")]
    NoRecipient,
    /// Native primary protection ownership changed during its operation.
    #[error("Native primary protection ownership changed during its operation")]
    OwnershipChanged,
    /// Native protection operation lost its combat binding.
    #[error("Native protection operation lost its combat binding")]
    NoBinding,
}

/// Protection channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtectionChannel {
    /// Regular armor.
    Regular,
    /// Powered protection.
    Powered,
}

/// Combat engine surface used by the protection scope.
pub trait ProtectionEngine {
    /// Current protection owner for an actor channel, if held elsewhere.
    fn protection_owner(&self, actor: u32, channel: ProtectionChannel) -> Option<String>;
    /// Read an actor's armor.
    fn read_armor(&self, actor: u32) -> Option<ArmorState>;
    /// Publish regular armor to held cursors.
    fn set_regular_armor(&mut self, actor: u32, armor: RegularArmor);
    /// Publish powered protection to held cursors.
    fn set_powered_protection(&mut self, actor: u32, powered: PoweredProtection);
    /// Whether the actor's damage is currently observed.
    fn observing_damage(&self, actor: u32) -> bool;
}

/// Publish original primary protection stores while a selected grant or
/// debit is held. `note_write` checkpoints emulate the source write
/// observers: callers invoke `publish` after committing source bytes.
pub struct ProtectionScope<'e, E: ProtectionEngine> {
    engine: &'e mut E,
    /// Recipient actor.
    pub recipient: u32,
    /// Channel.
    pub channel: ProtectionChannel,
}

impl<'e, E: ProtectionEngine> ProtectionScope<'e, E> {
    /// Open a scope over a live recipient.
    pub fn open(
        engine: &'e mut E,
        recipient: Option<u32>,
        channel: ProtectionChannel,
    ) -> Result<Self, ProtectionError> {
        let Some(recipient) = recipient else {
            return Err(ProtectionError::NoRecipient);
        };
        Ok(Self {
            engine,
            recipient,
            channel,
        })
    }

    /// Re-check ownership after a step.
    pub fn checkpoint(&self) -> Result<(), ProtectionError> {
        if self
            .engine
            .protection_owner(self.recipient, self.channel)
            .is_some()
        {
            return Err(ProtectionError::OwnershipChanged);
        }
        Ok(())
    }

    /// Publish committed source bytes to held damage cursors.
    pub fn publish(&mut self, committed: bool) -> Result<(), ProtectionError> {
        self.checkpoint()?;
        if !committed || self.engine.observing_damage(self.recipient) {
            return Ok(());
        }
        let state = self
            .engine
            .read_armor(self.recipient)
            .ok_or(ProtectionError::NoBinding)?;
        match self.channel {
            ProtectionChannel::Powered => self
                .engine
                .set_powered_protection(self.recipient, state.powered),
            ProtectionChannel::Regular => self
                .engine
                .set_regular_armor(self.recipient, state.regular),
        }
        Ok(())
    }

    /// Run a grant operation with publication around each consume step.
    pub fn run<T>(
        mut self,
        committed: &dyn Fn() -> bool,
        operation: impl FnOnce(Publisher<'_, 'e, E>) -> Result<T, ProtectionError>,
    ) -> Result<T, ProtectionError> {
        let owned = self
            .engine
            .protection_owner(self.recipient, self.channel)
            .is_some();
        if !owned {
            self.checkpoint()?;
        }
        let result = operation(Publisher {
            scope: &mut self,
            committed,
            passthrough: owned,
        })?;
        if !owned {
            self.checkpoint()?;
        }
        Ok(result)
    }
}

/// Grant-operation handle: `consume` checkpoints and publishes around
/// each source-mutating step, or passes through when held elsewhere.
pub struct Publisher<'s, 'e, E: ProtectionEngine> {
    scope: &'s mut ProtectionScope<'e, E>,
    committed: &'s dyn Fn() -> bool,
    passthrough: bool,
}

impl<E: ProtectionEngine> Publisher<'_, '_, E> {
    /// Run one source-mutating step with publication.
    pub fn consume(&mut self, execute: impl FnOnce()) -> Result<(), ProtectionError> {
        if self.passthrough {
            execute();
            return Ok(());
        }
        self.scope.checkpoint()?;
        execute();
        self.scope.publish((self.committed)())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct FakeEngine {
        owners: HashMap<(u32, ProtectionChannel), String>,
        armor: HashMap<u32, ArmorState>,
        published_regular: Vec<(u32, RegularArmor)>,
        published_powered: Vec<(u32, PoweredProtection)>,
        observing: Vec<u32>,
    }

    impl FakeEngine {
        fn new() -> Self {
            Self {
                owners: HashMap::new(),
                armor: HashMap::new(),
                published_regular: Vec::new(),
                published_powered: Vec::new(),
                observing: Vec::new(),
            }
        }
    }

    impl ProtectionEngine for FakeEngine {
        fn protection_owner(&self, actor: u32, channel: ProtectionChannel) -> Option<String> {
            self.owners.get(&(actor, channel)).cloned()
        }
        fn read_armor(&self, actor: u32) -> Option<ArmorState> {
            self.armor.get(&actor).cloned()
        }
        fn set_regular_armor(&mut self, actor: u32, armor: RegularArmor) {
            self.published_regular.push((actor, armor));
        }
        fn set_powered_protection(&mut self, actor: u32, powered: PoweredProtection) {
            self.published_powered.push((actor, powered));
        }
        fn observing_damage(&self, actor: u32) -> bool {
            self.observing.contains(&actor)
        }
    }

    fn armor() -> ArmorState {
        ArmorState {
            regular: RegularArmor::Q2 {
                points: 50.0,
                normal_protection: 0.6,
                energy_protection: 0.6,
                item: "q2:item_armor_combat".to_string(),
            },
            powered: PoweredProtection::None,
        }
    }

    #[test]
    fn grant_publishes_regular_armor() {
        let mut engine = FakeEngine::new();
        engine.armor.insert(3, armor());
        let scope = ProtectionScope::open(&mut engine, Some(3), ProtectionChannel::Regular)
            .expect("scope");
        let committed = || true;
        let result = scope
            .run(&committed, |mut publish| {
                publish.consume(|| {})?;
                Ok(7)
            })
            .expect("run");
        assert_eq!(result, 7);
        assert_eq!(engine.published_regular.len(), 1);
        assert_eq!(engine.published_regular[0].0, 3);
    }

    #[test]
    fn held_owned_or_removed_recipients_skip_or_fail() {
        let mut engine = FakeEngine::new();
        engine.armor.insert(4, armor());
        engine
            .owners
            .insert((4, ProtectionChannel::Powered), "other".to_string());
        let scope = ProtectionScope::open(&mut engine, Some(4), ProtectionChannel::Powered)
            .expect("scope");
        let committed = || true;
        scope
            .run(&committed, |mut publish| {
                publish.consume(|| {})?;
                Ok(())
            })
            .expect("run");
        assert!(engine.published_powered.is_empty());
        let mut engine = FakeEngine::new();
        assert_eq!(
            ProtectionScope::open(&mut engine, None, ProtectionChannel::Regular)
                .unwrap_err(),
            ProtectionError::NoRecipient
        );
        let mut engine = FakeEngine::new();
        engine.observing.push(5);
        engine.armor.insert(5, armor());
        let mut scope =
            ProtectionScope::open(&mut engine, Some(5), ProtectionChannel::Regular).expect("scope");
        scope.publish(true).expect("publish");
        assert!(engine.published_regular.is_empty());
        let mut engine = FakeEngine::new();
        let mut scope =
            ProtectionScope::open(&mut engine, Some(6), ProtectionChannel::Regular).expect("scope");
        assert_eq!(
            scope.publish(true).unwrap_err(),
            ProtectionError::NoBinding
        );
    }
}
