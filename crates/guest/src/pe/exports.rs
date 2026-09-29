//! PE export resolution across forwarder chains.
//!
//! Donor: `src/guest/pe/exports.ts`. Library policy and dependency loading
//! belong to the caller; forwarders never load native code.

use std::collections::HashSet;

use crate::core::contracts::{GuestAddress, GuestSymbolName};
use crate::error::GuestError;
use crate::pe::format::{pe_error, PeStage};
use crate::pe::image::PeImage;

/// Resolved export: defining image plus address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeResolvedExport {
    /// Defining image.
    pub image: PeImage,
    /// Export address.
    pub address: GuestAddress,
}

/// Library lookup for forwarder chains.
pub type PeLibraryLookup<'a> = &'a dyn Fn(&str, &PeImage) -> Option<PeImage>;

/// Resolve an export, following forwarder chains up to 128 entries.
pub fn resolve_pe_export(
    image: &PeImage,
    symbol: &GuestSymbolName,
    lookup: PeLibraryLookup,
) -> Result<PeResolvedExport, GuestError> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut current = image.clone();
    let mut name = symbol.clone();
    for _ in 0..128 {
        let key = match &name {
            GuestSymbolName::Ordinal(ordinal) => format!("ordinal:{ordinal}"),
            GuestSymbolName::Name { name, version } => {
                format!("name:{name}:{}", version.as_deref().unwrap_or(""))
            }
        };
        // Identity is (module artifact, load base, symbol): mappings are
        // disjoint, so distinct loads never collide.
        let visit = format!(
            "{}:{}:{:x}:{key}",
            current.image.module.artifact_path,
            current.image.module.digest.value,
            current.image.base.offset
        );
        if !seen.insert(visit) {
            return Err(pe_error(PeStage::Exports, "cyclic export forwarder"));
        }
        let requested = name.clone();
        let exported = current.image.exports.iter().find(|entry| match (&entry.symbol, &requested) {
            (GuestSymbolName::Ordinal(left), GuestSymbolName::Ordinal(right)) => left == right,
            (
                GuestSymbolName::Name { name: left, version: left_version },
                GuestSymbolName::Name { name: right, version: right_version },
            ) => left == right && left_version == right_version,
            _ => false,
        }).cloned();
        let Some(exported) = exported else {
            return Err(pe_error(
                PeStage::Exports,
                format!("unresolved {:?} {key}", current.image.module.id),
            ));
        };
        match &exported.target {
            crate::core::contracts::GuestExportTarget::Address(address) => {
                return Ok(PeResolvedExport {
                    image: current,
                    address: *address,
                });
            }
            crate::core::contracts::GuestExportTarget::Forward { library, symbol } => {
                let Some(dependency) = lookup(library, &current) else {
                    return Err(pe_error(
                        PeStage::Exports,
                        format!("unresolved forwarded library {library}"),
                    ));
                };
                if dependency.image.base.space != image.image.base.space
                    || dependency.image.abi.pointer_bytes() != image.image.abi.pointer_bytes()
                {
                    return Err(pe_error(
                        PeStage::Exports,
                        "forwarded export belongs to another guest address space or ABI width",
                    ));
                }
                current = dependency;
                name = symbol.clone();
            }
        }
    }
    Err(pe_error(
        PeStage::Exports,
        "export forwarder chain exceeds 128 entries",
    ))
}
