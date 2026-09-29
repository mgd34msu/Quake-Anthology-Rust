//! Save provider ownership ported from `src/persistence/provider-ownership.ts`.
//!
//! Host-owned schemas have fixed owners; every other supported schema
//! belongs to the selected source. `world:simulation` is version 11, all
//! other source schemas are version 1.

use super::value::{arr, obj, str, SaveJson, SaveReader};
use crate::WorldError;

/// Source-private state checkpoint (donor `ProviderCheckpoint`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCheckpoint {
    /// Owning provider (`namespace:name`).
    pub provider: String,
    /// Schema (`namespace:name`).
    pub schema: String,
    /// Schema version.
    pub version: i64,
    /// Encoded payload.
    pub bytes: Vec<u8>,
}

/// Expected owner/version for a checkpoint record (without payload).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderContract {
    /// Owning provider.
    pub provider: String,
    /// Schema.
    pub schema: String,
    /// Schema version.
    pub version: i64,
}

/// Fixed campaign-unit checkpoint contract.
pub fn campaign_unit_contract() -> ProviderContract {
    ProviderContract {
        provider: "session:campaign-unit".to_string(),
        schema: "session:campaign-unit".to_string(),
        version: 1,
    }
}

/// Resolve the expected owner for a schema under a source provider.
pub fn save_provider_contract(schema: &str, source: &str) -> Result<ProviderContract, WorldError> {
    match schema {
        "world:source-slots" => Ok(ProviderContract {
            provider: "world:actors".to_string(),
            schema: schema.to_string(),
            version: 1,
        }),
        "world:source-items" | "world:primary-protection" => Ok(ProviderContract {
            provider: "world:gameplay".to_string(),
            schema: schema.to_string(),
            version: 1,
        }),
        "session:campaign-unit" => Ok(campaign_unit_contract()),
        "app:component-clients" => Ok(ProviderContract {
            provider: "app:component-clients".to_string(),
            schema: schema.to_string(),
            version: 1,
        }),
        _ if schema.starts_with("session:") => {
            Err(WorldError::BadSave(format!("Unsupported session checkpoint {schema}")))
        }
        _ => Ok(ProviderContract {
            provider: source.to_string(),
            schema: schema.to_string(),
            version: if schema == "world:simulation" { 11 } else { 1 },
        }),
    }
}

/// Validate a record against its expected owner and version.
pub fn validate_save_provider_owner(record: &ProviderCheckpoint, source: &str) -> Result<(), WorldError> {
    let expected = save_provider_contract(&record.schema, source)?;
    if record.provider != expected.provider {
        return Err(WorldError::BadSave(format!(
            "Saved provider {} has a different owner",
            record.schema
        )));
    }
    if record.version != expected.version {
        return Err(WorldError::BadSave(format!(
            "Unsupported saved provider version {}",
            record.schema
        )));
    }
    Ok(())
}

/// Read a provider checkpoint record.
pub fn read_provider_checkpoint(reader: SaveReader) -> Result<ProviderCheckpoint, WorldError> {
    use super::value::namespaced;
    Ok(ProviderCheckpoint {
        provider: namespaced(reader.field("provider"))?,
        schema: namespaced(reader.field("schema"))?,
        version: reader.field("version").integer(0)?,
        bytes: reader.field("bytes").bytes()?,
    })
}

/// Write a provider checkpoint record.
#[must_use]
pub fn write_provider_checkpoint(record: &ProviderCheckpoint) -> SaveJson {
    obj(vec![
        ("provider", str(&record.provider)),
        ("schema", str(&record.schema)),
        ("version", super::value::int(record.version)),
        ("bytes", SaveJson::Bytes(record.bytes.clone())),
    ])
}

/// Filter records to one schema (helper for provider-local restores).
#[must_use]
pub fn records_for_schema<'a>(records: &'a [ProviderCheckpoint], schema: &str) -> Vec<&'a ProviderCheckpoint> {
    records.iter().filter(|record| record.schema == schema).collect()
}

/// Encode a provider-record list.
#[must_use]
pub fn write_provider_list(records: &[ProviderCheckpoint]) -> SaveJson {
    arr(records.iter().map(write_provider_checkpoint).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::save::value::{decode_checkpoint_value, encode_checkpoint_value};

    #[test]
    fn contracts_match_donor_owners() {
        assert_eq!(
            save_provider_contract("world:source-slots", "q3:game").unwrap(),
            ProviderContract {
                provider: "world:actors".to_string(),
                schema: "world:source-slots".to_string(),
                version: 1,
            }
        );
        assert_eq!(
            save_provider_contract("world:primary-protection", "q3:game")
                .unwrap()
                .provider,
            "world:gameplay"
        );
        assert_eq!(
            save_provider_contract("session:campaign-unit", "q3:game").unwrap(),
            campaign_unit_contract()
        );
        assert_eq!(
            save_provider_contract("world:simulation", "q3:game").unwrap().version,
            11
        );
        assert_eq!(save_provider_contract("q3:custom", "q3:game").unwrap().version, 1);
        assert!(save_provider_contract("session:unknown", "q3:game").is_err());
    }

    #[test]
    fn owner_validation_rejects_mismatches() {
        let record = ProviderCheckpoint {
            provider: "world:gameplay".to_string(),
            schema: "world:source-items".to_string(),
            version: 1,
            bytes: vec![1, 2],
        };
        assert!(validate_save_provider_owner(&record, "q3:game").is_ok());
        let bad_owner = ProviderCheckpoint {
            provider: "q3:game".to_string(),
            ..record.clone()
        };
        assert!(validate_save_provider_owner(&bad_owner, "q3:game").is_err());
        let bad_version = ProviderCheckpoint {
            version: 2,
            ..record.clone()
        };
        assert!(validate_save_provider_owner(&bad_version, "q3:game").is_err());
    }

    #[test]
    fn records_round_trip_through_checkpoints() {
        let record = ProviderCheckpoint {
            provider: "q2:game".to_string(),
            schema: "q2:classic-native-original".to_string(),
            version: 1,
            bytes: vec![9, 8, 7],
        };
        let json = write_provider_checkpoint(&record);
        let back = decode_checkpoint_value(&encode_checkpoint_value(&json)).unwrap();
        assert_eq!(read_provider_checkpoint(SaveReader::at(&back, "p")).unwrap(), record);
    }
}
