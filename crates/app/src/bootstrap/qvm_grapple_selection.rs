//! Exact-executable QVM grapple selection for Quake III mods.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/qvm-grapple-selection.ts`
//! (`QvmGrappleSelection`, `PreparedQvmGrapple`, `applicationQvmGrappleSelection`,
//! `prepareApplicationQvmGrapple`). Digest matching, artifact resolution, and profile
//! parsing come from the ported grapple profiles, [`resolve_qvm_artifact`], and
//! [`q3_grapple_profile`]; mounts arrive through the [`QvmGrappleCatalog`] seam. Two
//! documented folds: the selection keeps the guest [`QvmGrappleProfile`] instead of the
//! contract definition (no profile-to-definition converter exists yet — lifting into
//! `contract::GrappleSelection` belongs to the equipment-selection owner), and the
//! prepared value retains the executable read rather than the mount plan. Resolution
//! parses into the interpreter image while the profile matcher grounds against the
//! game-data image, so the resolved image is converted between the two already-ported
//! shapes below (both opcode enums share donor order, asserted in tests).

use qa_content::contract::ContentId;
use qa_content::q3::equipment::grapple_profiles::q3_grapple_profile;
use qa_content::q3::equipment::lrctf_grapple_profile::LRCTF_GRAPPLE_DIGEST;
use qa_content::q3::equipment::threewave_grapple_profile::THREEWAVE_GRAPPLE_DIGEST;
use qa_core::identity::ProviderId;
use qa_guest::core::contracts::{ContentDigest, ModuleIdentity as ResolveModuleIdentity};
use qa_guest::error::GuestError;
use qa_guest::qvm::artifacts::{resolve_qvm_artifact, ResolvedQvmArtifact};
use qa_guest::qvm::game_data::{
    ModuleIdentity as ProfileModuleIdentity, QvmArtifact, QvmImage, QvmOpcode as ProfileOpcode, QvmRole as ProfileRole,
};
use qa_guest::qvm::grapple_profile::{qvm_grapple_profile_declaration, QvmGrappleProfile};
use qa_guest::qvm::image::{QvmImage as ParsedImage, QvmOpcode as ParsedOpcode};
use qa_guest::qvm::syscalls::{QvmAbiProfile, QvmRole};
use thiserror::Error;

/// One opened `vm/qagame.qvm` (donor mount read).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmGrappleExecutable {
    /// Requested mount path.
    pub requested_path: String,
    /// Mount digest metadata.
    pub digest: String,
    /// Executable bytes.
    pub bytes: Vec<u8>,
}

/// Content family and executable reads (donor catalog plus mount plans).
pub trait QvmGrappleCatalog {
    /// Whether content belongs to the `q3` family.
    fn is_q3_family(&self, content: &ContentId) -> bool;
    /// Open `vm/qagame.qvm`, or [`None`] when absent.
    fn open_qagame(&mut self, content: &ContentId) -> Option<QvmGrappleExecutable>;
}

/// Enabled offhand classic `q3-qvm` grapple selection (donor `QvmGrappleSelection`).
///
/// The donor's constant fields (enabled kind, `q3-qvm` mechanic, classic edition, offhand
/// binding) are inherent in this type; `provider` is the `q3:grapple/...` module id.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmGrappleSelection {
    /// Selecting provider (module id).
    pub provider: String,
    /// Selected content.
    pub content: ContentId,
    /// Parsed grapple profile.
    pub profile: QvmGrappleProfile,
}

/// Prepared selection with its resolved artifact (donor `PreparedQvmGrapple`).
#[derive(Debug)]
pub struct PreparedQvmGrapple {
    /// Prepared selection.
    pub selection: QvmGrappleSelection,
    /// Resolved module artifact.
    pub artifact: ResolvedQvmArtifact,
    /// Retained executable read.
    pub executable: QvmGrappleExecutable,
}

/// Failure to prepare a grapple selection, with donor messages.
#[derive(Debug, Error)]
pub enum QvmGrappleError {
    /// The installed executable or declaration changed under the selection.
    #[error("Selected grapple differs from its installed source executable or declaration")]
    Changed,
    /// Guest failure.
    #[error(transparent)]
    Guest(#[from] GuestError),
}

/// Map a parsed opcode to its game-data twin (identical donor order).
fn convert_opcode(opcode: ParsedOpcode) -> ProfileOpcode {
    match opcode {
        ParsedOpcode::OpUndef => ProfileOpcode::OpUndef,
        ParsedOpcode::OpIgnore => ProfileOpcode::OpIgnore,
        ParsedOpcode::OpBreak => ProfileOpcode::OpBreak,
        ParsedOpcode::OpEnter => ProfileOpcode::OpEnter,
        ParsedOpcode::OpLeave => ProfileOpcode::OpLeave,
        ParsedOpcode::OpCall => ProfileOpcode::OpCall,
        ParsedOpcode::OpPush => ProfileOpcode::OpPush,
        ParsedOpcode::OpPop => ProfileOpcode::OpPop,
        ParsedOpcode::OpConst => ProfileOpcode::OpConst,
        ParsedOpcode::OpLocal => ProfileOpcode::OpLocal,
        ParsedOpcode::OpJump => ProfileOpcode::OpJump,
        ParsedOpcode::OpEq => ProfileOpcode::OpEq,
        ParsedOpcode::OpNe => ProfileOpcode::OpNe,
        ParsedOpcode::OpLti => ProfileOpcode::OpLti,
        ParsedOpcode::OpLei => ProfileOpcode::OpLei,
        ParsedOpcode::OpGti => ProfileOpcode::OpGti,
        ParsedOpcode::OpGei => ProfileOpcode::OpGei,
        ParsedOpcode::OpLtu => ProfileOpcode::OpLtu,
        ParsedOpcode::OpLeu => ProfileOpcode::OpLeu,
        ParsedOpcode::OpGtu => ProfileOpcode::OpGtu,
        ParsedOpcode::OpGeu => ProfileOpcode::OpGeu,
        ParsedOpcode::OpEqf => ProfileOpcode::OpEqf,
        ParsedOpcode::OpNef => ProfileOpcode::OpNef,
        ParsedOpcode::OpLtf => ProfileOpcode::OpLtf,
        ParsedOpcode::OpLef => ProfileOpcode::OpLef,
        ParsedOpcode::OpGtf => ProfileOpcode::OpGtf,
        ParsedOpcode::OpGef => ProfileOpcode::OpGef,
        ParsedOpcode::OpLoad1 => ProfileOpcode::OpLoad1,
        ParsedOpcode::OpLoad2 => ProfileOpcode::OpLoad2,
        ParsedOpcode::OpLoad4 => ProfileOpcode::OpLoad4,
        ParsedOpcode::OpStore1 => ProfileOpcode::OpStore1,
        ParsedOpcode::OpStore2 => ProfileOpcode::OpStore2,
        ParsedOpcode::OpStore4 => ProfileOpcode::OpStore4,
        ParsedOpcode::OpArg => ProfileOpcode::OpArg,
        ParsedOpcode::OpBlockCopy => ProfileOpcode::OpBlockCopy,
        ParsedOpcode::OpSex8 => ProfileOpcode::OpSex8,
        ParsedOpcode::OpSex16 => ProfileOpcode::OpSex16,
        ParsedOpcode::OpNegi => ProfileOpcode::OpNegi,
        ParsedOpcode::OpAdd => ProfileOpcode::OpAdd,
        ParsedOpcode::OpSub => ProfileOpcode::OpSub,
        ParsedOpcode::OpDivi => ProfileOpcode::OpDivi,
        ParsedOpcode::OpDivu => ProfileOpcode::OpDivu,
        ParsedOpcode::OpModi => ProfileOpcode::OpModi,
        ParsedOpcode::OpModu => ProfileOpcode::OpModu,
        ParsedOpcode::OpMuli => ProfileOpcode::OpMuli,
        ParsedOpcode::OpMulu => ProfileOpcode::OpMulu,
        ParsedOpcode::OpBand => ProfileOpcode::OpBand,
        ParsedOpcode::OpBor => ProfileOpcode::OpBor,
        ParsedOpcode::OpBxor => ProfileOpcode::OpBxor,
        ParsedOpcode::OpBcom => ProfileOpcode::OpBcom,
        ParsedOpcode::OpLsh => ProfileOpcode::OpLsh,
        ParsedOpcode::OpRshi => ProfileOpcode::OpRshi,
        ParsedOpcode::OpRshu => ProfileOpcode::OpRshu,
        ParsedOpcode::OpNegf => ProfileOpcode::OpNegf,
        ParsedOpcode::OpAddf => ProfileOpcode::OpAddf,
        ParsedOpcode::OpSubf => ProfileOpcode::OpSubf,
        ParsedOpcode::OpDivf => ProfileOpcode::OpDivf,
        ParsedOpcode::OpMulf => ProfileOpcode::OpMulf,
        ParsedOpcode::OpCvif => ProfileOpcode::OpCvif,
        ParsedOpcode::OpCvfi => ProfileOpcode::OpCvfi,
    }
}

/// Convert a parsed image into the game-data shape the profile matcher grounds.
fn convert_image(image: &ParsedImage) -> QvmImage {
    QvmImage {
        source: image.source.clone(),
        instructions: image
            .instructions
            .iter()
            .map(|instruction| qa_guest::qvm::game_data::QvmInstruction {
                opcode: convert_opcode(instruction.opcode),
                operand: match instruction.operand {
                    qa_guest::qvm::image::QvmOperand::Word(word) => word,
                    qa_guest::qvm::image::QvmOperand::Byte(byte) => i32::from(byte),
                    qa_guest::qvm::image::QvmOperand::None => 0,
                },
                operand_width: instruction.operand_width() as u8,
                byte_offset: instruction.byte_offset,
            })
            .collect(),
        code_offset: image.code_offset,
        code_length: image.code_length,
        data_length: image.data_length,
        literal_length: image.literal_length,
        bss_length: image.bss_length,
        allocated_data_length: image.allocated_data_length,
        initialized_data: image.initialized_data.clone(),
        data_mask: image.data_mask.max(0) as usize,
    }
}

/// Select the grapple for mounted content (donor `fromMounts`).
fn from_mounts(
    content: &ContentId,
    executable: &QvmGrappleExecutable,
) -> Result<Option<(QvmGrappleSelection, ResolvedQvmArtifact)>, GuestError> {
    if executable.digest != LRCTF_GRAPPLE_DIGEST && executable.digest != THREEWAVE_GRAPPLE_DIGEST {
        return Ok(None);
    }
    let Some((algorithm, value)) = executable.digest.split_once(':') else {
        return Ok(None);
    };
    let module_id = format!("q3:grapple/{content}");
    let module = ResolveModuleIdentity::new(
        ProviderId::new("q3", &format!("grapple/{content}")),
        &executable.requested_path,
        ContentDigest::new(algorithm, value),
        &executable.digest,
    );
    let artifact = resolve_qvm_artifact(
        &module,
        QvmRole::Qagame,
        &executable.bytes,
        Vec::new(),
        QvmAbiProfile::Modern,
    )?;
    let ResolvedQvmArtifact::Bytecode { image, .. } = &artifact else {
        return Ok(None);
    };
    let profile_artifact = QvmArtifact {
        module: ProfileModuleIdentity {
            id: module_id.clone(),
            artifact_path: executable.requested_path.clone(),
            digest: executable.digest.clone(),
            revision: executable.digest.clone(),
        },
        role: ProfileRole::Qagame,
        abi_profile: None,
        image: convert_image(image),
    };
    let Some(profile) = q3_grapple_profile(&profile_artifact)? else {
        return Ok(None);
    };
    Ok(Some((
        QvmGrappleSelection {
            provider: module_id,
            content: content.clone(),
            profile,
        },
        artifact,
    )))
}

/// Select the installed grapple for content (donor `applicationQvmGrappleSelection`).
pub fn application_qvm_grapple_selection(
    catalog: &mut impl QvmGrappleCatalog,
    content: &ContentId,
) -> Result<Option<QvmGrappleSelection>, GuestError> {
    if !catalog.is_q3_family(content) {
        return Ok(None);
    }
    let Some(executable) = catalog.open_qagame(content) else {
        return Ok(None);
    };
    Ok(from_mounts(content, &executable)?.map(|(selection, _)| selection))
}

/// Prepare a selection against its installed executable (donor `prepareApplicationQvmGrapple`).
pub fn prepare_application_qvm_grapple(
    catalog: &mut impl QvmGrappleCatalog,
    selection: &QvmGrappleSelection,
) -> Result<PreparedQvmGrapple, QvmGrappleError> {
    let Some(executable) = catalog.open_qagame(&selection.content) else {
        return Err(QvmGrappleError::Changed);
    };
    let Some((mounted, artifact)) = from_mounts(&selection.content, &executable)? else {
        return Err(QvmGrappleError::Changed);
    };
    if mounted.provider != selection.provider
        || qvm_grapple_profile_declaration(&mounted.profile) != qvm_grapple_profile_declaration(&selection.profile)
    {
        return Err(QvmGrappleError::Changed);
    }
    Ok(PreparedQvmGrapple {
        selection: selection.clone(),
        artifact,
        executable,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Stub {
        q3: bool,
        executable: Option<QvmGrappleExecutable>,
    }

    impl QvmGrappleCatalog for Stub {
        fn is_q3_family(&self, _content: &ContentId) -> bool {
            self.q3
        }
        fn open_qagame(&mut self, _content: &ContentId) -> Option<QvmGrappleExecutable> {
            self.executable.clone()
        }
    }

    fn content() -> ContentId {
        ContentId("q3:classic:baseq3:1".to_string())
    }

    #[test]
    fn non_q3_and_missing_executables_select_nothing() {
        let mut catalog = Stub {
            q3: false,
            executable: None,
        };
        assert!(application_qvm_grapple_selection(&mut catalog, &content())
            .unwrap()
            .is_none());
        let mut catalog = Stub {
            q3: true,
            executable: None,
        };
        assert!(application_qvm_grapple_selection(&mut catalog, &content())
            .unwrap()
            .is_none());
    }

    #[test]
    fn unknown_digest_selects_nothing() {
        let mut catalog = Stub {
            q3: true,
            executable: Some(QvmGrappleExecutable {
                requested_path: "vm/qagame.qvm".to_string(),
                digest: "sha256:0000000000000000000000000000000000000000000000000000000000000000".to_string(),
                bytes: vec![0; 64],
            }),
        };
        assert!(application_qvm_grapple_selection(&mut catalog, &content())
            .unwrap()
            .is_none());
    }

    #[test]
    fn bytes_must_match_their_digest() {
        let mut catalog = Stub {
            q3: true,
            executable: Some(QvmGrappleExecutable {
                requested_path: "vm/qagame.qvm".to_string(),
                digest: LRCTF_GRAPPLE_DIGEST.to_string(),
                bytes: vec![0; 64],
            }),
        };
        assert!(application_qvm_grapple_selection(&mut catalog, &content()).is_err());
    }

    #[test]
    fn opcode_tables_share_donor_order() {
        for discriminant in 0..60u8 {
            let parsed = qa_guest::qvm::image::QvmOpcode::from_u8(discriminant).unwrap();
            assert_eq!(convert_opcode(parsed) as u8, discriminant);
        }
    }

    #[test]
    fn prepare_rejects_changed_installs() {
        let mut catalog = Stub {
            q3: true,
            executable: None,
        };
        use qa_guest::qvm::game_data::{QvmInstruction, QvmOpcode};

        // Synthetic image satisfying declaration grounding: geometry covers the
        // entity records and globals, with function entries at the callbacks.
        let mut instructions = vec![QvmInstruction::single(QvmOpcode::OpIgnore, 0); 184_570];
        for entry in [
            178_797, 179_014, 183_076, 183_171, 151_164, 183_577, 5_362, 15_874, 184_569, 140_358, 169_306, 22_369,
        ] {
            instructions[entry] = QvmInstruction::single(QvmOpcode::OpEnter, entry);
        }
        let coordinates = QvmArtifact {
            module: ProfileModuleIdentity {
                id: "q3:grapple/q3:classic:baseq3:1".to_string(),
                artifact_path: "vm/qagame.qvm".to_string(),
                digest: LRCTF_GRAPPLE_DIGEST.to_string(),
                revision: LRCTF_GRAPPLE_DIGEST.to_string(),
            },
            role: ProfileRole::Qagame,
            abi_profile: None,
            image: QvmImage {
                data_length: 1_100_000,
                allocated_data_length: 2_097_152,
                instructions,
                ..QvmImage::default()
            },
        };
        let profile = qa_content::q3::equipment::lrctf_grapple_profile::lrctf_grapple_profile(&coordinates)
            .unwrap()
            .unwrap();
        let selection = QvmGrappleSelection {
            provider: "q3:grapple/q3:classic:baseq3:1".to_string(),
            content: content(),
            profile,
        };
        let error = prepare_application_qvm_grapple(&mut catalog, &selection).unwrap_err();
        assert!(matches!(error, QvmGrappleError::Changed));
    }
}
