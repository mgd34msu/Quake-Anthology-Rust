//! Source-memory hooks for collision allocation records.
//!
//! Donor provenance: `/home/buzzkill/Projects/quake-typescript/src/world/collision/q3/allocation.ts`.

/// Zone allocation: raw bytes owned by the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoneAllocation {
    /// Backing bytes.
    pub bytes: Vec<u8>,
}

/// Hunk allocation lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HunkKind {
    /// Permanent allocation.
    Permanent,
    /// Temporary allocation.
    Temporary,
}

/// Hunk allocation: bytes plus their 32-bit source address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HunkAllocation {
    /// Allocation lifetime.
    pub kind: HunkKind,
    /// Source byte offset (address).
    pub byte_offset: u32,
    /// Allocation length in bytes.
    pub byte_length: usize,
    /// Backing bytes.
    pub bytes: Vec<u8>,
}

/// Hunk reservation side preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HunkPreference {
    /// Reserve from the high side, as CM always does.
    High,
}

/// Source-hunk accounting hook behind [`HunkAccountingProfile::SourceHunk`].
pub trait HunkAccounting {
    /// Reserve `bytes` for `source`/`resource` on the requested side.
    fn reserve(&self, source: &str, resource: &str, bytes: usize, preference: HunkPreference) -> HunkAllocation;
}

/// How collision reserves its source memory.
#[derive(Clone)]
pub enum HunkAccountingProfile {
    /// Diagnostic loads allocate locally instead of through a hunk.
    Unaccounted,
    /// Source loads reserve through shared hunk accounting.
    SourceHunk {
        /// Shared accounting hook.
        accounting: std::rc::Rc<dyn HunkAccounting>,
    },
}

impl std::fmt::Debug for HunkAccountingProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unaccounted => write!(f, "Unaccounted"),
            Self::SourceHunk { .. } => write!(f, "SourceHunk(..)"),
        }
    }
}

/// 32-bit release record sizes (`SOURCE_HUNK_RELEASE32`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceHunkRelease32 {
    /// Pointer size.
    pub pointer: usize,
    /// On-disk shader size.
    pub disk_shader: usize,
    /// In-memory collision model size.
    pub collision_model: usize,
    /// In-memory collision node size.
    pub collision_node: usize,
    /// In-memory brush size.
    pub brush: usize,
    /// In-memory leaf size.
    pub leaf: usize,
    /// In-memory area size.
    pub area: usize,
    /// In-memory plane size.
    pub plane: usize,
    /// In-memory brush-side size.
    pub brush_side: usize,
    /// In-memory collision patch header size.
    pub collision_patch: usize,
}

/// Frozen 32-bit release record sizes.
pub const SOURCE_HUNK_RELEASE32: SourceHunkRelease32 = SourceHunkRelease32 {
    pointer: 4,
    disk_shader: 72,
    collision_model: 48,
    collision_node: 12,
    brush: 44,
    leaf: 24,
    area: 8,
    plane: 20,
    brush_side: 12,
    collision_patch: 16,
};

#[cfg(test)]
mod tests {
    use super::*;

    struct FixedAccounting;

    impl HunkAccounting for FixedAccounting {
        fn reserve(&self, _source: &str, _resource: &str, bytes: usize, preference: HunkPreference) -> HunkAllocation {
            assert_eq!(preference, HunkPreference::High);
            HunkAllocation {
                kind: HunkKind::Permanent,
                byte_offset: 0x1000,
                byte_length: bytes,
                bytes: vec![0; bytes],
            }
        }
    }

    #[test]
    fn release_sizes_match_source() {
        assert_eq!(SOURCE_HUNK_RELEASE32.pointer, 4);
        assert_eq!(SOURCE_HUNK_RELEASE32.disk_shader, 72);
        assert_eq!(SOURCE_HUNK_RELEASE32.collision_model, 48);
        assert_eq!(SOURCE_HUNK_RELEASE32.collision_node, 12);
        assert_eq!(SOURCE_HUNK_RELEASE32.brush, 44);
        assert_eq!(SOURCE_HUNK_RELEASE32.leaf, 24);
        assert_eq!(SOURCE_HUNK_RELEASE32.area, 8);
        assert_eq!(SOURCE_HUNK_RELEASE32.plane, 20);
        assert_eq!(SOURCE_HUNK_RELEASE32.brush_side, 12);
        assert_eq!(SOURCE_HUNK_RELEASE32.collision_patch, 16);
    }

    #[test]
    fn source_hunk_profile_reserves_high() {
        let profile = HunkAccountingProfile::SourceHunk {
            accounting: std::rc::Rc::new(FixedAccounting),
        };
        let HunkAccountingProfile::SourceHunk { accounting } = &profile else {
            panic!("expected source-hunk profile");
        };
        let allocation = accounting.reserve("site", "resource", 32, HunkPreference::High);
        assert_eq!(allocation.byte_offset, 0x1000);
        assert_eq!(allocation.bytes.len(), 32);
    }
}
