//! Guest save/restore: checkpoints of game-module state. The engine owns
//! actor lifetime and field storage; modules own no hidden state outside
//! the field table and the event log, so a guest save is the field image
//! plus the module roster that must match on restore (donor
//! `src/contracts/execution.ts` guest checkpoints).

use qa_core::identity::ActorId;

use crate::error::GuestError;
use crate::fields::{FieldCheckpoint, FieldTable};

/// Guest save schema version.
pub const GUEST_SCHEMA_VERSION: u32 = 1;

/// Saved guest image.
#[derive(Debug, Clone, PartialEq)]
pub struct GuestSave {
    /// Schema version.
    pub schema_version: u32,
    /// Registered module names in dispatch order.
    pub modules: Vec<String>,
    /// Entity fields.
    pub fields: Vec<FieldCheckpoint>,
}

/// Checkpoint guest state.
#[must_use]
pub fn checkpoint_guest(modules: &[String], fields: &FieldTable) -> GuestSave {
    GuestSave {
        schema_version: GUEST_SCHEMA_VERSION,
        modules: modules.to_vec(),
        fields: fields.checkpoint(),
    }
}

/// Restore guest fields after validating the schema and module roster.
pub fn restore_guest(
    live: &dyn Fn(&qa_core::identity::SavedActorId) -> Option<ActorId>,
    expected_modules: &[String],
    save: &GuestSave,
    fields: &mut FieldTable,
) -> Result<(), GuestError> {
    if save.schema_version != GUEST_SCHEMA_VERSION {
        return Err(GuestError::BadSave("Unsupported guest save schema".to_string()));
    }
    if save.modules != expected_modules {
        return Err(GuestError::BadSave(
            "Guest module roster changed across save/restore".to_string(),
        ));
    }
    fields.restore(live, &save.fields)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;

    use crate::fields::{FieldLayout, FieldValue};

    #[test]
    fn guest_save_validates_schema_and_roster() {
        let owner = IdentityOwner::create("test").unwrap();
        let actor = owner.actor(1, 1);
        let mut fields = FieldTable::new();
        fields.allocate(&actor, &FieldLayout::qc_entity()).unwrap();
        fields.set(&actor, "frame", FieldValue::Float(3.0)).unwrap();
        let modules = vec!["test:game".to_string()];
        let save = checkpoint_guest(&modules, &fields);
        assert_eq!(save.schema_version, GUEST_SCHEMA_VERSION);
        let mut restored = FieldTable::new();
        restore_guest(&|_| Some(actor.clone()), &modules, &save, &mut restored).unwrap();
        assert_eq!(restored.get(&actor, "frame").unwrap(), &FieldValue::Float(3.0));
        assert!(restore_guest(
            &|_| Some(actor.clone()),
            &["test:other".to_string()],
            &save,
            &mut restored
        )
        .is_err());
        let mut bad_schema = save.clone();
        bad_schema.schema_version = 99;
        assert!(restore_guest(&|_| Some(actor.clone()), &modules, &bad_schema, &mut restored).is_err());
    }
}
