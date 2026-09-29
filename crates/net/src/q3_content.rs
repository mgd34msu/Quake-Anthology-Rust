//! Quake III pak content references.
//!
//! Donor provenance: `Q3MountedPak`, `registerQ3Pak`, and
//! `Q3ContentReferences` in `src/network/q3/content.ts`. Pure checksums
//! reuse [`q3_archive_checksums`](crate::q3_net::q3_archive_checksums) and
//! pak tracking reuses [`PakReferences`](crate::q3_pak_references::PakReferences).
//!
//! The content-crate types (`ArchiveMount`, `ArchiveHandle`,
//! `PureMountPolicy`) are not ported yet: this module defines minimal
//! local mirrors ([`Q3ArchiveMount`], [`Q3ResolvedReference`],
//! [`Q3PureMountPolicy`]) capturing exactly what the donor needs. SEAM:
//! replace these mirrors with the content crate's types when they land;
//! only this module's constructors and accessors should change.

use std::collections::HashMap;

use thiserror::Error;

use crate::common::session::ContentDigest;
use crate::q3_net::{q3_archive_checksums, Q3ArchiveHandle, Q3NetError};
use crate::q3_pak_references::{reorder_pure_paks, PakCatalogEntry, PakReferences, PureSearchPath, Q3PakError};

/// Content failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Q3ContentError {
    /// Underlying pak-reference failure.
    #[error("{0}")]
    Pak(#[from] Q3PakError),
    /// Underlying Q3 netcode failure.
    #[error("{0}")]
    Net(#[from] Q3NetError),
    /// Duplicate mounted pak identity.
    #[error("Duplicate Q3 mounted pak identity")]
    DuplicateMount,
    /// Opened resource does not belong to this catalog.
    #[error("Opened source resource does not belong to this Q3 pak catalog")]
    ForeignProvenance,
    /// No installed archives match the server pure list.
    #[error("No installed Q3 archives match the server pure list")]
    NoMatchingArchives,
}

/// Minimal mount identity mirror (donor `MountIdentity`: id + generation).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Q3MountIdentity {
    /// Mount id (`mount:namespace:name`).
    pub id: String,
    /// Mount generation.
    pub generation: u64,
}

/// Minimal archive mount mirror (donor `ArchiveMount`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3ArchiveMount {
    /// Mount identity.
    pub identity: Q3MountIdentity,
    /// Archive path.
    pub archive_path: String,
    /// Archive content digest.
    pub archive_digest: ContentDigest,
}

/// Minimal resource provenance mirror (donor `ResourceProvenance`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Q3ResourceProvenance {
    /// Loose file.
    Loose,
    /// Archive member with its mount at open time.
    Archive {
        /// Mount the resource was opened from.
        mount: Q3ArchiveMount,
    },
}

/// Minimal resolved reference mirror (donor `ResolvedResourceReference`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3ResolvedReference {
    /// Requested path.
    pub requested_path: String,
    /// Resource provenance.
    pub provenance: Q3ResourceProvenance,
}

/// Minimal pure mount policy mirror (donor `PureMountPolicy`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3PureMountPolicy {
    /// Accepted archive digests.
    pub archives: Vec<ContentDigest>,
}

/// Mounted pak (`Q3MountedPak`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3MountedPak {
    /// Mount.
    pub mount: Q3ArchiveMount,
    /// Catalog entry.
    pub pack: PakCatalogEntry,
}

/// Register a pak (`registerQ3Pak`).
pub fn register_q3_pak(
    mount: Q3ArchiveMount,
    archive: &Q3ArchiveHandle,
    game: &str,
    basename: &str,
    checksum_feed: i32,
) -> Result<Q3MountedPak, Q3ContentError> {
    let (checksum, pure_checksum) = q3_archive_checksums(archive, checksum_feed)?;
    let pack = PakCatalogEntry {
        game: game.to_owned(),
        basename: basename.to_owned(),
        archive_path: mount.archive_path.clone(),
        checksum,
        pure_checksum,
    };
    Ok(Q3MountedPak { mount, pack })
}

/// Content reference tracker (`Q3ContentReferences`).
pub struct Q3ContentReferences {
    references: PakReferences,
    by_mount: HashMap<String, Q3MountedPak>,
    packs: Vec<Q3MountedPak>,
}

impl Q3ContentReferences {
    /// Build a tracker over mounted packs.
    pub fn new(
        packs: Vec<Q3MountedPak>,
        checksum_feed: u32,
        random: impl FnMut() -> f64 + 'static,
    ) -> Result<Self, Q3ContentError> {
        let mut by_mount = HashMap::new();
        for pak in &packs {
            if by_mount.contains_key(&pak.mount.identity.id) {
                return Err(Q3ContentError::DuplicateMount);
            }
            by_mount.insert(pak.mount.identity.id.clone(), pak.clone());
        }
        let references = PakReferences::new(
            packs.iter().map(|pak| pak.pack.clone()).collect(),
            checksum_feed,
            random,
        )?;
        Ok(Self {
            references,
            by_mount,
            packs,
        })
    }

    /// Borrow the reference tracker.
    pub fn references(&self) -> &PakReferences {
        &self.references
    }

    /// Mutably borrow the reference tracker.
    pub fn references_mut(&mut self) -> &mut PakReferences {
        &mut self.references
    }

    /// Record a resolved open (`opened`).
    pub fn opened(&mut self, reference: &Q3ResolvedReference) -> Result<(), Q3ContentError> {
        match &reference.provenance {
            Q3ResourceProvenance::Loose => {
                self.references.record_loose_open(&reference.requested_path)?;
                Ok(())
            }
            Q3ResourceProvenance::Archive { mount } => {
                let Some(pak) = self.by_mount.get(&mount.identity.id) else {
                    return Err(Q3ContentError::ForeignProvenance);
                };
                if pak.mount.archive_digest != mount.archive_digest
                    || pak.mount.identity.generation != mount.identity.generation
                {
                    return Err(Q3ContentError::ForeignProvenance);
                }
                let pack = pak.pack.clone();
                self.references.record_packed_open(&pack, &reference.requested_path)?;
                Ok(())
            }
        }
    }

    /// Map server checksums to mounted archive digests (`pureMountPolicy`).
    pub fn pure_mount_policy(&self, server_checksums: &[i32]) -> Result<Q3PureMountPolicy, Q3ContentError> {
        if server_checksums.is_empty() {
            return Ok(Q3PureMountPolicy { archives: Vec::new() });
        }
        let paths: Vec<PureSearchPath<&Q3MountedPak>> = self
            .packs
            .iter()
            .map(|value| PureSearchPath::Pak {
                value,
                checksum: value.pack.checksum,
            })
            .collect();
        let accepted: HashMap<u32, ()> = server_checksums.iter().map(|value| (*value as u32, ())).collect();
        let mut archives = Vec::new();
        for path in reorder_pure_paks(&paths, server_checksums) {
            if let PureSearchPath::Pak { value, checksum } = path {
                if accepted.contains_key(&checksum) {
                    archives.push(value.mount.archive_digest.clone());
                }
            }
        }
        if archives.is_empty() {
            return Err(Q3ContentError::NoMatchingArchives);
        }
        Ok(Q3PureMountPolicy { archives })
    }

    /// Build the referenced pure command (`referencedPureCommand`).
    pub fn referenced_pure_command(&self, server_id: i32) -> String {
        format!("cp {server_id} {}", self.references.referenced_pak_pure_checksums())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q3_net::{Q3ArchiveEntry, Q3ArchiveHandle};

    fn digest(byte: u8) -> ContentDigest {
        ContentDigest::new(&format!("{byte:02x}").repeat(32)).unwrap()
    }

    fn mount(id: &str, generation: u64, digest: ContentDigest) -> Q3ArchiveMount {
        Q3ArchiveMount {
            identity: Q3MountIdentity {
                id: id.to_owned(),
                generation,
            },
            archive_path: format!("baseq3/{id}.pk3"),
            archive_digest: digest,
        }
    }

    fn archive() -> Q3ArchiveHandle {
        Q3ArchiveHandle {
            pak_format: false,
            entries: vec![Q3ArchiveEntry {
                byte_length: 10,
                crc32: 0x1234_5678,
                pak_entry: false,
            }],
        }
    }

    fn pak(id: &str, generation: u64, byte: u8) -> Q3MountedPak {
        register_q3_pak(
            mount(id, generation, digest(byte)),
            &archive(),
            "baseq3",
            &format!("{id}.pk3"),
            1,
        )
        .unwrap()
    }

    #[test]
    fn registration_carries_checksums_and_mount() {
        let pak = pak("pak0", 3, 0xAB);
        assert_eq!(pak.pack.game, "baseq3");
        assert_eq!(pak.pack.basename, "pak0.pk3");
        assert_eq!(pak.pack.archive_path, "baseq3/pak0.pk3");
        let (checksum, pure) = q3_archive_checksums(&archive(), 1).unwrap();
        assert_eq!(pak.pack.checksum, checksum);
        assert_eq!(pak.pack.pure_checksum, pure);
        assert_eq!(pak.mount.identity.generation, 3);
        let mut demo = archive();
        demo.pak_format = true;
        assert!(register_q3_pak(mount("demo", 0, digest(1)), &demo, "baseq3", "demo.pak", 1).is_err());
    }

    #[test]
    fn duplicate_mounts_fail() {
        let packs = vec![pak("pak0", 0, 1), pak("pak0", 1, 2)];
        assert_eq!(
            Q3ContentReferences::new(packs, 0, || 0.0).err(),
            Some(Q3ContentError::DuplicateMount)
        );
    }

    #[test]
    fn opens_route_by_provenance() {
        let mut content = Q3ContentReferences::new(vec![pak("pak0", 0, 1)], 0, || 1.0).unwrap();
        content
            .opened(&Q3ResolvedReference {
                requested_path: "autoexec.cfg".to_owned(),
                provenance: Q3ResourceProvenance::Loose,
            })
            .unwrap();
        // Allowed loose paths leave the fake checksum at zero.
        assert_eq!(content.referenced_pure_command(7).split(' ').count(), 4);
        content
            .opened(&Q3ResolvedReference {
                requested_path: "vm/qagame.qvm".to_owned(),
                provenance: Q3ResourceProvenance::Archive {
                    mount: mount("pak0", 0, digest(1)),
                },
            })
            .unwrap();
        assert!(!content.references().game_pure_checksum().is_empty());
        assert_eq!(
            content.opened(&Q3ResolvedReference {
                requested_path: "vm/cgame.qvm".to_owned(),
                provenance: Q3ResourceProvenance::Archive {
                    mount: mount("pak9", 0, digest(9)),
                },
            }),
            Err(Q3ContentError::ForeignProvenance)
        );
        assert_eq!(
            content.opened(&Q3ResolvedReference {
                requested_path: "vm/cgame.qvm".to_owned(),
                provenance: Q3ResourceProvenance::Archive {
                    mount: mount("pak0", 1, digest(1)),
                },
            }),
            Err(Q3ContentError::ForeignProvenance)
        );
    }

    #[test]
    fn pure_policy_maps_server_order() {
        let content = Q3ContentReferences::new(vec![pak("pak0", 0, 1), pak("pak1", 0, 2)], 0, || 0.0).unwrap();
        let policy = content.pure_mount_policy(&[]).unwrap();
        assert!(policy.archives.is_empty());
        // Both test archives share checksums; the policy still resolves.
        let checksum = content.packs[0].pack.checksum as i32;
        let policy = content.pure_mount_policy(&[checksum]).unwrap();
        assert_eq!(policy.archives.len(), 2);
        assert_eq!(policy.archives[0], digest(1));
        assert!(content.pure_mount_policy(&[12345679]).is_err());
    }
}
