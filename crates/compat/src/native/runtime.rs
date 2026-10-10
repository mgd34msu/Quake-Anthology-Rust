//! Cold library/name/version binding to the shared C function entries.
use super::elf::{self, Bindings, Definition, Pass};
use crate::{abi::runtime::FUNCTIONS, memory::ModuleMemory};
use qa_core::{names::compare_folded, primitives::NameId};
use qa_formats::program::native::{Encoding, Image};
use qa_platform::native::{NativeAbi, NativeImport, NativeProcess};

fn library(image: &Image, id: NameId) -> bool {
    let Some(name) = image.names.get(id) else {
        return false;
    };
    match image.target.encoding {
        Encoding::Elf => name == b"libc.so.6",
        Encoding::Pe => [
            b"msvcrt.dll".as_slice(),
            b"ucrtbase.dll",
            b"api-ms-win-crt-string-l1-1-0.dll",
            b"api-ms-win-crt-memory-l1-1-0.dll",
        ]
        .iter()
        .any(|expected| compare_folded(name, expected).is_eq()),
    }
}
fn import_error(
    image: &Image,
    name: Option<NameId>,
    provider: Option<NameId>,
    version: Option<NameId>,
) -> String {
    let text = |id| String::from_utf8_lossy(image.names.get(id).unwrap_or_default());
    format!(
        "native import not bound: {}{}{}",
        provider
            .map(|id| format!("{}:", text(id)))
            .unwrap_or_default(),
        name.map(text).unwrap_or_else(|| "<ordinal>".into()),
        version
            .map(|id| format!("@{}", text(id)))
            .unwrap_or_default()
    )
}
pub(super) fn bind(image: &mut Image, prefix: usize) -> Result<Vec<NativeImport<'static>>, String> {
    for &id in &image.needed {
        if !library(image, id) {
            return Err(format!(
                "native provider not bound: {}",
                String::from_utf8_lossy(image.names.get(id).unwrap_or_default())
            ));
        }
    }
    let abi = match image.target.encoding {
        Encoding::Elf => NativeAbi::SystemV,
        Encoding::Pe => NativeAbi::Microsoft,
    };
    let mut imports: Vec<NativeImport<'static>> = Vec::new();
    let mut symbols = vec![None; image.symbols.len()];
    let mut slots = Vec::new();
    let mut resolve = |name: Option<NameId>,
                       provider: Option<NameId>,
                       version: Option<NameId>|
     -> Result<u64, String> {
        let raw_name = name.and_then(|id| image.names.get(id));
        let function = FUNCTIONS.iter().find(|f| Some(f.name) == raw_name);
        let supported_version = version.is_none_or(|id| {
            let version = image.names.get(id);
            version == Some(b"GLIBC_2.2.5".as_slice())
                || (raw_name == Some(b"memcpy".as_slice())
                    && version == Some(b"GLIBC_2.14".as_slice()))
        });
        if provider.is_some_and(|id| !library(image, id)) || !supported_version {
            return Err(import_error(image, name, provider, version));
        }
        let function = function.ok_or_else(|| import_error(image, name, provider, version))?;
        let ordinal = match imports
            .iter()
            .position(|entry| entry.number == function.number)
        {
            Some(ordinal) => ordinal,
            None => {
                let ordinal = imports.len();
                imports.push(NativeImport {
                    number: function.number,
                    abi,
                    parameters: function.parameters,
                    result: function.result,
                });
                ordinal
            }
        };
        NativeProcess::import_address(image.base, image.bytes.len(), prefix + ordinal)
            .map_err(|e| format!("native import reservation: {e:?}"))
    };
    match image.target.encoding {
        Encoding::Elf => {
            for relocation in &image.relocations {
                let Some(index) = relocation.symbol else {
                    continue;
                };
                let symbol = image
                    .symbols
                    .get(index)
                    .ok_or("native relocation symbol out of range")?;
                if symbol.defined || symbols[index].is_some() || relocation.kind == 0 {
                    continue;
                }
                let provider = symbol.version.and_then(|v| v.library);
                let version = symbol.version.map(|v| v.name);
                match resolve(symbol.name, provider, version) {
                    Ok(address) => symbols[index] = Some(Definition::Address { address, bytes: 0 }),
                    Err(_) if symbol.weak => {}
                    Err(e) => return Err(e),
                }
            }
        }
        Encoding::Pe => {
            for import in &image.imports {
                if import.delayed {
                    return Err(format!(
                        "native delayed {}",
                        import_error(image, import.name, import.library, None)
                    ));
                }
                slots.push((import.slot, resolve(import.name, import.library, None)?));
            }
        }
    }
    // Drop the cold resolver's borrow before patching the one image.
    drop(resolve);
    if image.target.encoding == Encoding::Elf {
        let indirect = vec![None; image.relocations.len()];
        let bindings = Bindings {
            symbols: &symbols,
            local_tls: None,
            indirect: &indirect,
        };
        elf::bind(image, &bindings, Pass::Regular)
            .and_then(|()| elf::bind(image, &bindings, Pass::Indirect))
            .map_err(|e| format!("native binding: {e:?}"))?;
    } else {
        let mut memory = ModuleMemory::borrow(image.base, &mut image.bytes)
            .map_err(|_| "native import memory")?;
        for (slot, address) in slots {
            memory
                .write(slot, &address.to_le_bytes())
                .map_err(|_| "native import slot out of range")?;
        }
    }
    Ok(imports)
}
