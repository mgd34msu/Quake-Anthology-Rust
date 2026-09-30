//! Test-only helpers for the Q3 content tree.

use std::path::Path;

use qa_guest::qvm::game_data::{AbiProfile, ModuleIdentity, QvmArtifact, QvmImage, QvmInstruction, QvmOpcode, QvmRole};

use crate::contract::{ContentId, ContentMount, LooseMount, MountPlanId, ResolvedMountPlan};
use crate::mounts::{open_mount_plan, MountedContent, OpenMountOptions};

/// Mount a loose directory as test content (mirrors the mods seam helper).
pub(crate) fn mount_loose_dir(root: &Path, content: &ContentId) -> MountedContent {
    use crate::contract::{create_mount_id, create_mount_identity};
    let mount = LooseMount {
        identity: create_mount_identity(create_mount_id("seam", "q3").unwrap(), content.clone(), 0).unwrap(),
        root_path: root.to_string_lossy().into_owned(),
    };
    open_mount_plan(
        &ResolvedMountPlan {
            id: MountPlanId("mount-plan:seam:q3".to_string()),
            mounts: vec![ContentMount::Loose(mount.clone())],
            default_order: vec![mount.identity.id.clone()],
            prefix_orders: Vec::new(),
        },
        OpenMountOptions::default(),
    )
    .unwrap()
}

/// Build a fixture QVM artifact: modern ABI, `data_length` bytes of source
/// data, and `OpEnter` at each listed callback entry.
pub(crate) fn fixture_artifact(digest: &str, role: QvmRole, data_length: usize, entries: &[usize]) -> QvmArtifact {
    let width = entries.iter().copied().max().unwrap_or(0) + 1;
    let mut instructions = vec![QvmInstruction::single(QvmOpcode::OpIgnore, 0); width];
    for entry in entries {
        instructions[*entry] = QvmInstruction::single(QvmOpcode::OpEnter, *entry);
    }
    QvmArtifact {
        module: ModuleIdentity {
            digest: digest.to_string(),
            artifact_path: "vm/qagame.qvm".to_string(),
            ..Default::default()
        },
        role,
        abi_profile: Some(AbiProfile::Modern),
        image: QvmImage {
            source: "<test>".to_string(),
            instructions,
            data_length,
            allocated_data_length: data_length,
            ..Default::default()
        },
    }
}
