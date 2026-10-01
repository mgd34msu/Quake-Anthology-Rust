//! Quake I/II model dispatcher: magic-based dispatch over the MDL, MD2,
//! SPR, and SP2 decoders plus decoded-resource pairing.
//!
//! Donor provenance: `/home/buzzkill/Projects/quake-typescript/src/formats/q12-model/index.ts`.

use qa_core::binary::{BinaryError, BinaryReader};

use crate::contract::ResolvedResourceReference;
use crate::md2::Md2Model;
use crate::mdl::MdlModel;
use crate::spr::{Sp2Model, SprModel};

pub use crate::md2::{
    build_md2_geometry, decode_md2_commands, interpolate_alias_frames, parse_md2, sample_timed_frame, Md2Command,
    Md2CommandVertex,
};
pub use crate::mdl::parse_mdl;
pub use crate::normals::ALIAS_NORMALS;
pub use crate::spr::{parse_sp2, parse_spr};

/// Any Quake I/II alias or sprite model (`Q12Model`).
#[derive(Debug, Clone, PartialEq)]
pub enum Q12Model {
    /// Quake I alias model.
    Mdl(MdlModel),
    /// Quake II alias model.
    Md2(Md2Model),
    /// Quake I sprite.
    Spr(SprModel),
    /// Quake II sprite.
    Sp2(Sp2Model),
}

/// Decoded model paired with its resolved source (`Q12ModelResource`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q12ModelResource {
    /// Resolved source reference.
    pub source: ResolvedResourceReference,
    /// Decoded model.
    pub model: Q12Model,
}

/// Parse a model by four-byte magic: `IDPO` (MDL), `IDP2` (MD2), `IDSP`
/// (SPR), `IDS2` (SP2) (`parseQ12Model`).
pub fn parse_q12_model(data: &[u8], source: &str) -> Result<Q12Model, BinaryError> {
    let magic = BinaryReader::new(data, source).fixed_byte_string(4)?;
    match magic.as_str() {
        "IDPO" => Ok(Q12Model::Mdl(parse_mdl(data, source)?)),
        "IDP2" => Ok(Q12Model::Md2(parse_md2(data, source)?)),
        "IDSP" => Ok(Q12Model::Spr(parse_spr(data, source)?)),
        "IDS2" => Ok(Q12Model::Sp2(parse_sp2(data, source)?)),
        _ => Err(BinaryError::custom(
            source,
            0,
            format!("Unknown Q1/Q2 model magic {magic:?}"),
        )),
    }
}

/// Pair resolved bytes with their decoded model (`decodeQ12ModelResource`).
/// The resolved byte length must equal the delivered bytes.
pub fn decode_q12_model_resource(
    source: ResolvedResourceReference,
    data: &[u8],
) -> Result<Q12ModelResource, BinaryError> {
    if source.byte_length != data.len() as u64 {
        return Err(BinaryError::custom(
            &source.requested_path,
            0,
            "Resource byte length differs from resolved content",
        ));
    }
    let model = parse_q12_model(data, &source.requested_path)?;
    Ok(Q12ModelResource { source, model })
}

#[cfg(test)]
mod tests {
    use qa_core::binary::BinaryWriter;

    use crate::contract::{
        ContentId, LooseMount, MountId, MountIdentity, MountPlanId, ResourceId, ResourceProvenance, ResourceResolution,
    };

    use super::*;

    fn mdl_bytes() -> Vec<u8> {
        let mut writer = BinaryWriter::new(256);
        writer.bytes(b"IDPO").unwrap();
        writer.i32(6).unwrap();
        for value in [0.5f32, 0.5, 0.5] {
            writer.f32(value).unwrap();
        }
        for value in [1.0f32, 2.0, 3.0] {
            writer.f32(value).unwrap();
        }
        writer.f32(10.0).unwrap();
        for value in [0.0f32, 0.0, 0.0] {
            writer.f32(value).unwrap();
        }
        writer.i32(1).unwrap();
        writer.i32(2).unwrap();
        writer.i32(2).unwrap();
        writer.i32(1).unwrap();
        writer.i32(1).unwrap();
        writer.i32(1).unwrap();
        writer.i32(0).unwrap();
        writer.i32(0).unwrap();
        writer.f32(1.0).unwrap();
        writer.i32(0).unwrap();
        writer.bytes(&[7u8; 4]).unwrap();
        writer.i32(0).unwrap();
        writer.i32(8).unwrap();
        writer.i32(12).unwrap();
        writer.i32(1).unwrap();
        writer.i32(0).unwrap();
        writer.i32(0).unwrap();
        writer.i32(0).unwrap();
        writer.i32(0).unwrap();
        writer.bytes(&[0, 0, 0, 0, 4, 4, 4, 0]).unwrap();
        let mut name = [0u8; 16];
        name[..5].copy_from_slice(b"frame");
        writer.bytes(&name).unwrap();
        writer.bytes(&[2, 2, 2, 5]).unwrap();
        writer.finish()
    }

    fn reference(path: &str, byte_length: u64) -> ResolvedResourceReference {
        ResolvedResourceReference {
            id: ResourceId("resource:test:model".to_string()),
            requested_path: path.to_string(),
            provenance: ResourceProvenance::Loose {
                mount: LooseMount {
                    identity: MountIdentity {
                        id: MountId("mount:test:loose".to_string()),
                        content: ContentId("content:test:base:0".to_string()),
                        generation: 0,
                    },
                    root_path: "/tmp".to_string(),
                },
                member_path: path.to_string(),
            },
            digest: create_test_digest(),
            byte_length,
            resolution: ResourceResolution::DefaultOrder {
                plan: MountPlanId("mount-plan:test:0".to_string()),
                rank: 0,
            },
        }
    }

    fn create_test_digest() -> crate::contract::ContentDigest {
        crate::contract::create_content_digest(&"ab".repeat(32)).unwrap()
    }

    #[test]
    fn q12_dispatches_each_magic() {
        let data = mdl_bytes();
        let model = parse_q12_model(&data, "<model>").unwrap();
        assert!(matches!(model, Q12Model::Mdl(parsed) if parsed.scale == [0.5, 0.5, 0.5]));
        for magic in ["IDP2", "IDSP", "IDS2"] {
            let error = parse_q12_model(magic.as_bytes(), "<model>").unwrap_err();
            assert!(
                !error.to_string().contains("Unknown Q1/Q2 model magic"),
                "magic {magic} must route to its decoder: {error}"
            );
        }
    }

    #[test]
    fn q12_rejects_unknown_magic() {
        let error = parse_q12_model(b"XXXX", "<model>").unwrap_err();
        assert_eq!(error.to_string(), "<model>:0: Unknown Q1/Q2 model magic \"XXXX\"");
    }

    #[test]
    fn q12_resource_checks_byte_length() {
        let data = mdl_bytes();
        let error = decode_q12_model_resource(reference("models/test.mdl", data.len() as u64 + 1), &data).unwrap_err();
        assert_eq!(
            error.to_string(),
            "models/test.mdl:0: Resource byte length differs from resolved content"
        );
        let resource = decode_q12_model_resource(reference("models/test.mdl", data.len() as u64), &data).unwrap();
        assert_eq!(resource.source.requested_path, "models/test.mdl");
        assert!(matches!(resource.model, Q12Model::Mdl(_)));
    }
}
