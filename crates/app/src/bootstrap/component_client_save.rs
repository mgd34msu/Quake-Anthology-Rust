//! Component-client save checkpoint.
//!
//! Donor provenance: `src/app/bootstrap/component-client-save.ts`
//! (`readComponentClients`, `saveComponentClients`,
//! `requireComponentClientPresentation`).
//!
//! Sync port with no behavioral changes. Provider records, ownership, and the
//! checkpoint codec are reused from [`qa_world::save`]. The donor operates on
//! the full `SaveImage`; the current [`qa_world::session::SaveImage`] port
//! carries no provider rows or recipe, so this module defines the minimal
//! [`ComponentClientImage`] view (provider rows plus the source provider id
//! the donor reads from `image.recipe.map.entities.provider`).

use qa_world::save::ownership::{
    save_provider_contract, validate_save_provider_owner, ProviderCheckpoint,
};
use qa_world::save::value::{
    decode_checkpoint_value, encode_checkpoint_value, SaveJson, SaveReader,
};
use qa_world::WorldError;
use thiserror::Error;

/// Checkpoint schema for saved component clients.
pub const COMPONENT_CLIENT_SCHEMA: &str = "app:component-clients";

/// Failure of a component-client checkpoint operation.
#[derive(Debug, Error)]
pub enum ComponentClientSaveError {
    /// More than one checkpoint row matched the schema.
    #[error("Duplicate component client checkpoint")]
    Duplicate,
    /// A checkpoint row is already attached.
    #[error("Component client checkpoint is already attached")]
    AlreadyAttached,
    /// Saved seats need a graphical destination.
    #[error("Saved component viewing clients require a graphical destination")]
    GraphicalDestination,
    /// Save codec or ownership failure.
    #[error(transparent)]
    World(#[from] WorldError),
}

/// Minimal save-image view: provider rows plus the owning source provider.
#[derive(Debug, Clone, PartialEq)]
pub struct ComponentClientImage {
    /// Provider checkpoint rows.
    pub providers: Vec<ProviderCheckpoint>,
    /// Source provider id (donor `image.recipe.map.entities.provider`).
    pub source_provider: String,
}

/// Read the component-client checkpoint, or [`None`] when absent.
pub fn read_component_clients(
    image: &ComponentClientImage,
) -> Result<Option<SaveJson>, ComponentClientSaveError> {
    let rows: Vec<&ProviderCheckpoint> = image
        .providers
        .iter()
        .filter(|row| row.schema == COMPONENT_CLIENT_SCHEMA)
        .collect();
    if rows.is_empty() {
        return Ok(None);
    }
    if rows.len() != 1 {
        return Err(ComponentClientSaveError::Duplicate);
    }
    let row = rows[0];
    validate_save_provider_owner(row, &image.source_provider)?;
    let value = decode_checkpoint_value(&row.bytes)?;
    SaveReader::at(&value, COMPONENT_CLIENT_SCHEMA)
        .field("version")
        .literal_i64(1)?;
    Ok(Some(value))
}

/// Attach a component-client checkpoint, failing when one already exists.
pub fn save_component_clients(
    image: &ComponentClientImage,
    value: &SaveJson,
) -> Result<ComponentClientImage, ComponentClientSaveError> {
    if image
        .providers
        .iter()
        .any(|row| row.schema == COMPONENT_CLIENT_SCHEMA)
    {
        return Err(ComponentClientSaveError::AlreadyAttached);
    }
    let contract = save_provider_contract(COMPONENT_CLIENT_SCHEMA, &image.source_provider)?;
    let mut providers = image.providers.clone();
    providers.push(ProviderCheckpoint {
        provider: contract.provider,
        schema: contract.schema,
        version: contract.version,
        bytes: encode_checkpoint_value(value),
    });
    Ok(ComponentClientImage {
        providers,
        source_provider: image.source_provider.clone(),
    })
}

/// Reject restores whose saved seats need a graphical destination.
pub fn require_component_client_presentation(
    image: &ComponentClientImage,
) -> Result<(), ComponentClientSaveError> {
    let Some(value) = read_component_clients(image)? else {
        return Ok(());
    };
    let seats = SaveReader::at(&value, COMPONENT_CLIENT_SCHEMA)
        .field("seats")
        .list(|_| Ok::<(), ComponentClientSaveError>(()))?;
    if seats.is_empty() {
        return Ok(());
    }
    Err(ComponentClientSaveError::GraphicalDestination)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_world::save::value::{arr, int, obj};

    fn image() -> ComponentClientImage {
        ComponentClientImage {
            providers: Vec::new(),
            source_provider: "q1:netquake".to_string(),
        }
    }

    fn value() -> SaveJson {
        obj(vec![
            ("version", int(1)),
            ("seats", arr(Vec::new())),
        ])
    }

    #[test]
    fn missing_checkpoint_reads_none() {
        assert_eq!(read_component_clients(&image()).unwrap(), None);
        require_component_client_presentation(&image()).unwrap();
    }

    #[test]
    fn save_then_read_roundtrip() {
        let saved = save_component_clients(&image(), &value()).unwrap();
        assert_eq!(saved.providers.len(), 1);
        assert_eq!(saved.providers[0].schema, COMPONENT_CLIENT_SCHEMA);
        assert_eq!(saved.providers[0].provider, COMPONENT_CLIENT_SCHEMA);
        assert_eq!(read_component_clients(&saved).unwrap(), Some(value()));
    }

    #[test]
    fn duplicate_rows_and_double_attach_fail() {
        let saved = save_component_clients(&image(), &value()).unwrap();
        assert!(matches!(
            save_component_clients(&saved, &value()).unwrap_err(),
            ComponentClientSaveError::AlreadyAttached
        ));
        let mut providers = saved.providers.clone();
        providers.push(saved.providers[0].clone());
        let duplicated = ComponentClientImage {
            providers,
            source_provider: saved.source_provider.clone(),
        };
        assert!(matches!(
            read_component_clients(&duplicated).unwrap_err(),
            ComponentClientSaveError::Duplicate
        ));
    }

    #[test]
    fn wrong_owner_or_version_fails() {
        let mut saved = save_component_clients(&image(), &value()).unwrap();
        saved.providers[0].provider = "q1:netquake".to_string();
        assert!(read_component_clients(&saved).is_err());
        let mut saved = save_component_clients(&image(), &value()).unwrap();
        saved.providers[0].version = 2;
        assert!(read_component_clients(&saved).is_err());
    }

    #[test]
    fn nonempty_seats_require_graphical_destination() {
        let seated = obj(vec![("version", int(1)), ("seats", arr(vec![int(1)]))]);
        let saved = save_component_clients(&image(), &seated).unwrap();
        assert!(matches!(
            require_component_client_presentation(&saved).unwrap_err(),
            ComponentClientSaveError::GraphicalDestination
        ));
        let empty = save_component_clients(&image(), &value()).unwrap();
        require_component_client_presentation(&empty).unwrap();
    }
}
