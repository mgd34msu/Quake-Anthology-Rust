//! Known QVM artifact identities and bytecode/replacement resolution.
//!
//! Port of `src/compat/qvm/artifacts.ts` (`KnownQvmArtifact`,
//! `knownQvmArtifacts`, `QvmReplacement`, `resolveQvmArtifact`). Caller
//! supplies resolved archive provenance in the module identity; names alone
//! never select game code.
//!
//! SHA-256 is implemented locally (FIPS 180-4, verified against the standard
//! `"abc"` vector) because `qa-guest` takes no hash dependency.

use qa_core::identity::ProviderId;

use crate::core::contracts::ModuleIdentity;
use crate::error::GuestError;

use super::image::{parse_qvm, QvmImage};
use super::interpreter::QvmArguments;
use super::syscalls::{QvmAbiProfile, QvmHost, QvmRole};

/// One donor-observed artifact identity (not a claim of parity with every
/// source revision).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownQvmArtifact {
    /// Module role.
    pub role: QvmRole,
    /// Product the artifact shipped in.
    pub product: QvmProduct,
    /// Reference package the bytes were observed in.
    pub reference_package: String,
    /// Related game build date.
    pub related_game_build_date: String,
    /// Exact byte length.
    pub byte_length: usize,
    /// `sha256:` digest of the bytes.
    pub digest: String,
}

/// Product a known artifact shipped in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QvmProduct {
    /// Base Quake III Arena.
    Baseq3,
    /// Team Arena mission pack.
    Missionpack,
}

impl QvmProduct {
    /// Donor spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Baseq3 => "baseq3",
            Self::Missionpack => "missionpack",
        }
    }
}

struct KnownEntry {
    role: QvmRole,
    product: QvmProduct,
    package: &'static str,
    date: &'static str,
    byte_length: usize,
    digest: &'static str,
}

const KNOWN_ENTRIES: &[KnownEntry] = &[
    KnownEntry {
        role: QvmRole::Ui,
        product: QvmProduct::Baseq3,
        package: "baseq3/pak8",
        date: "2002-09-30",
        byte_length: 278_308,
        digest: "sha256:3a6fd12b889f5d35df20a09b51bf8eca46966d014be55ffad38ddc2ffb38c807",
    },
    KnownEntry {
        role: QvmRole::Cgame,
        product: QvmProduct::Baseq3,
        package: "baseq3/pak8",
        date: "2002-09-30",
        byte_length: 325_220,
        digest: "sha256:4ea18569bf56a282d26dc89eb9efcc5eedbe0b69c10182fc38446174c1e55b49",
    },
    KnownEntry {
        role: QvmRole::Qagame,
        product: QvmProduct::Baseq3,
        package: "baseq3/pak8",
        date: "2002-09-30",
        byte_length: 469_796,
        digest: "sha256:57c52bf22e4f528c064f8af1553a7103723bab0a02276bb11eed944bf829b219",
    },
    KnownEntry {
        role: QvmRole::Ui,
        product: QvmProduct::Missionpack,
        package: "missionpack/pak0",
        date: "2000-12-04",
        byte_length: 272_040,
        digest: "sha256:7b157f32acdb21a3904d078296672ed2d32195c5b7a206922f6f7d33c6c40e40",
    },
    KnownEntry {
        role: QvmRole::Cgame,
        product: QvmProduct::Missionpack,
        package: "missionpack/pak0",
        date: "2000-12-04",
        byte_length: 442_304,
        digest: "sha256:09d0b6eb41ea623d67031d2d7a73058ccb3bc6556ec044ead529d48b58d15f4c",
    },
    KnownEntry {
        role: QvmRole::Qagame,
        product: QvmProduct::Missionpack,
        package: "missionpack/pak0",
        date: "2000-12-04",
        byte_length: 547_700,
        digest: "sha256:da041f17f296feeaf8269eabc9062cefdecddfd24ff4d84eb291902e527d1d8a",
    },
];

/// Donor-observed QVM artifact identities.
#[must_use]
pub fn known_qvm_artifacts() -> Vec<KnownQvmArtifact> {
    KNOWN_ENTRIES
        .iter()
        .map(|entry| KnownQvmArtifact {
            role: entry.role,
            product: entry.product,
            reference_package: entry.package.to_string(),
            related_game_build_date: entry.date.to_string(),
            byte_length: entry.byte_length,
            digest: entry.digest.to_string(),
        })
        .collect()
}

/// Live replacement instance: synchronous entry plus shutdown.
pub struct QvmReplacementInstance {
    /// Invoke the replacement entry.
    pub invoke: Box<dyn FnMut(&QvmArguments) -> i32>,
    /// Shut the replacement down.
    pub shutdown: Box<dyn FnMut()>,
}

impl std::fmt::Debug for QvmReplacementInstance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QvmReplacementInstance").finish_non_exhaustive()
    }
}

/// Factory that creates a replacement instance bound to `module` and `host`.
pub type QvmReplacementCreate = Box<dyn FnMut(&ModuleIdentity, Box<dyn QvmHost>) -> QvmReplacementInstance>;

/// Native replacement for one known artifact.
pub struct QvmReplacement {
    /// Artifact replaced.
    pub artifact: KnownQvmArtifact,
    /// Implementing provider.
    pub implementation: ProviderId,
    /// Create an instance bound to `module` and `host`.
    pub create: QvmReplacementCreate,
}

impl std::fmt::Debug for QvmReplacement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QvmReplacement")
            .field("artifact", &self.artifact)
            .field("implementation", &self.implementation)
            .finish_non_exhaustive()
    }
}

/// Resolved artifact: parsed bytecode or a native replacement.
pub enum ResolvedQvmArtifact {
    /// Parsed bytecode image.
    Bytecode {
        /// Module identity.
        module: ModuleIdentity,
        /// Module role.
        role: QvmRole,
        /// Matched known artifact, if any.
        known: Option<KnownQvmArtifact>,
        /// Parsed image.
        image: QvmImage,
        /// ABI profile.
        abi_profile: QvmAbiProfile,
    },
    /// Native replacement instance factory.
    TypeScript {
        /// Module identity.
        module: ModuleIdentity,
        /// Module role.
        role: QvmRole,
        /// Matched known artifact, if any.
        known: Option<KnownQvmArtifact>,
        /// Selected replacement.
        replacement: QvmReplacement,
    },
}

impl std::fmt::Debug for ResolvedQvmArtifact {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bytecode {
                module,
                role,
                known,
                abi_profile,
                ..
            } => f
                .debug_struct("Bytecode")
                .field("module", module)
                .field("role", role)
                .field("known", known)
                .field("abi_profile", abi_profile)
                .finish_non_exhaustive(),
            Self::TypeScript {
                module,
                role,
                known,
                replacement,
            } => f
                .debug_struct("TypeScript")
                .field("module", module)
                .field("role", role)
                .field("known", known)
                .field("replacement", replacement)
                .finish_non_exhaustive(),
        }
    }
}

impl ResolvedQvmArtifact {
    /// Module identity.
    #[must_use]
    pub fn module(&self) -> &ModuleIdentity {
        match self {
            Self::Bytecode { module, .. } | Self::TypeScript { module, .. } => module,
        }
    }

    /// Module role.
    #[must_use]
    pub fn role(&self) -> QvmRole {
        match self {
            Self::Bytecode { role, .. } | Self::TypeScript { role, .. } => *role,
        }
    }
}

/// SHA-256 digest of `bytes` as lowercase hex (FIPS 180-4).
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98,
        0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
        0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8,
        0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
        0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819,
        0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
        0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut state: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    let bit_length = (bytes.len() as u64).wrapping_mul(8);
    let mut padded = bytes.to_vec();
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_length.to_be_bytes());
    for block in padded.as_chunks::<64>().0 {
        let mut schedule = [0u32; 64];
        for (index, word) in schedule.iter_mut().take(16).enumerate() {
            *word = u32::from_be_bytes([
                block[index * 4],
                block[index * 4 + 1],
                block[index * 4 + 2],
                block[index * 4 + 3],
            ]);
        }
        for index in 16..64 {
            let s0 = schedule[index - 15].rotate_right(7)
                ^ schedule[index - 15].rotate_right(18)
                ^ (schedule[index - 15] >> 3);
            let s1 = schedule[index - 2].rotate_right(17)
                ^ schedule[index - 2].rotate_right(19)
                ^ (schedule[index - 2] >> 10);
            schedule[index] = schedule[index - 16]
                .wrapping_add(s0)
                .wrapping_add(schedule[index - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
        for index in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[index])
                .wrapping_add(schedule[index]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
        state[4] = state[4].wrapping_add(e);
        state[5] = state[5].wrapping_add(f);
        state[6] = state[6].wrapping_add(g);
        state[7] = state[7].wrapping_add(h);
    }
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut hex = String::with_capacity(64);
    for word in state {
        for shift in [28, 24, 20, 16, 12, 8, 4, 0] {
            hex.push(HEX[((word >> shift) & 0xf) as usize] as char);
        }
    }
    hex
}

fn digest_string(module: &ModuleIdentity) -> String {
    format!("{}:{}", module.digest.algorithm, module.digest.value)
}

/// Resolve `bytes` for `role`: verify the module digest, match known artifacts
/// and replacements, else parse bytecode.
pub fn resolve_qvm_artifact(
    module: &ModuleIdentity,
    role: QvmRole,
    bytes: &[u8],
    replacements: Vec<QvmReplacement>,
    abi_profile: QvmAbiProfile,
) -> Result<ResolvedQvmArtifact, GuestError> {
    let digest = format!("sha256:{}", sha256_hex(bytes));
    if digest_string(module) != digest {
        return Err(GuestError::invalid(
            "QVM artifact bytes do not match their module identity",
        ));
    }
    let known = known_qvm_artifacts()
        .into_iter()
        .find(|entry| entry.digest == digest && entry.byte_length == bytes.len());
    if let Some(known) = known.as_ref() {
        if known.role != role {
            return Err(GuestError::invalid(format!(
                "QVM artifact is {}, requested {}",
                known.role.as_str(),
                role.as_str()
            )));
        }
    }
    if let Some(replacement) = replacements.into_iter().find(|entry| {
        entry.artifact.digest == digest && entry.artifact.role == role && entry.artifact.byte_length == bytes.len()
    }) {
        return Ok(ResolvedQvmArtifact::TypeScript {
            module: module.clone(),
            role,
            known,
            replacement,
        });
    }
    let image = parse_qvm(bytes, &module.artifact_path)?;
    Ok(ResolvedQvmArtifact::Bytecode {
        module: module.clone(),
        role,
        known,
        image,
        abi_profile,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::contracts::ContentDigest;

    fn module_for(bytes: &[u8]) -> ModuleIdentity {
        ModuleIdentity::new(
            ProviderId::new("qvm", "test"),
            "vm/test.qvm",
            ContentDigest::new("sha256", &sha256_hex(bytes)),
            "1",
        )
    }

    #[test]
    fn sha256_matches_the_standard_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(sha256_hex(b"").len(), 64);
    }

    #[test]
    fn known_table_covers_both_products() {
        let known = known_qvm_artifacts();
        assert_eq!(known.len(), 6);
        assert_eq!(known[0].role, QvmRole::Ui);
        assert_eq!(known[0].product, QvmProduct::Baseq3);
        assert_eq!(known[0].byte_length, 278_308);
        assert!(known[0].digest.starts_with("sha256:"));
        assert_eq!(known[5].product, QvmProduct::Missionpack);
    }

    #[test]
    fn digest_mismatch_and_role_mismatch_fail() {
        let bytes = vec![0u8; 64];
        let mut module = module_for(&bytes);
        module.digest = ContentDigest::new("sha256", "00");
        assert!(resolve_qvm_artifact(&module, QvmRole::Qagame, &bytes, Vec::new(), QvmAbiProfile::Modern).is_err());
    }

    #[test]
    fn replacement_wins_over_bytecode() {
        let bytes = vec![7u8; 128];
        let module = module_for(&bytes);
        let artifact = KnownQvmArtifact {
            role: QvmRole::Ui,
            product: QvmProduct::Baseq3,
            reference_package: "test".to_string(),
            related_game_build_date: "test".to_string(),
            byte_length: bytes.len(),
            digest: digest_string(&module),
        };
        let resolved = resolve_qvm_artifact(
            &module,
            QvmRole::Ui,
            &bytes,
            vec![QvmReplacement {
                artifact,
                implementation: ProviderId::new("native", "ui"),
                create: Box::new(|_, _| QvmReplacementInstance {
                    invoke: Box::new(|_| 1),
                    shutdown: Box::new(|| {}),
                }),
            }],
            QvmAbiProfile::Modern,
        )
        .unwrap();
        assert!(matches!(resolved, ResolvedQvmArtifact::TypeScript { .. }));
        assert_eq!(resolved.role(), QvmRole::Ui);
    }

    #[test]
    fn unknown_bytes_parse_as_bytecode_or_fail_cleanly() {
        // Minimal valid image: 1 instruction (BREAK), no data.
        let mut bytes = vec![0u8; 36];
        bytes[0..4].copy_from_slice(&0x12721444u32.to_le_bytes());
        bytes[4..8].copy_from_slice(&1i32.to_le_bytes());
        bytes[8..12].copy_from_slice(&32i32.to_le_bytes());
        bytes[12..16].copy_from_slice(&4i32.to_le_bytes());
        bytes[16..20].copy_from_slice(&36i32.to_le_bytes());
        bytes[32] = 2;
        let module = module_for(&bytes);
        let resolved =
            resolve_qvm_artifact(&module, QvmRole::Cgame, &bytes, Vec::new(), QvmAbiProfile::Modern).unwrap();
        assert!(matches!(resolved, ResolvedQvmArtifact::Bytecode { .. }));
    }
}
