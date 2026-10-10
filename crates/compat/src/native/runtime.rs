//! Cold library/name/version binding to the shared C function entries.
use super::elf::{self, Bindings, Definition, Pass};
use crate::memory::ModuleMemory;
use qa_core::{names::compare_folded, primitives::NameId};
use qa_formats::program::native::{Encoding, Image};
use qa_platform::native::{
    NativeAbi, NativeImport, NativeProcess,
    runtime::{FUNCTIONS, Function, RuntimeConfig},
};

fn library(image: &Image, id: NameId, function: Option<&Function>) -> bool {
    let Some(name) = image.names.get(id) else {
        return false;
    };
    match image.target.encoding {
        Encoding::Elf => function.map_or_else(
            || [b"libc.so.6".as_slice(), b"libm.so.6"].contains(&name),
            |function| name == function.provider,
        ),
        Encoding::Pe => {
            if function.is_some_and(|function| function.provider.ends_with(b".dll")) {
                return function
                    .is_some_and(|function| compare_folded(name, function.provider).is_eq());
            }
            [b"msvcrt.dll".as_slice(), b"ucrtbase.dll"]
                .iter()
                .any(|expected| compare_folded(name, expected).is_eq())
                || [
                    (
                        b"api-ms-win-crt-string-l1-1-0.dll".as_slice(),
                        b"libc.so.6".as_slice(),
                    ),
                    (b"api-ms-win-crt-memory-l1-1-0.dll", b"libc.so.6"),
                    (b"api-ms-win-crt-heap-l1-1-0.dll", b"libc.so.6"),
                    (b"api-ms-win-crt-math-l1-1-0.dll", b"libm.so.6"),
                ]
                .iter()
                .any(|(expected, provider)| {
                    compare_folded(name, expected).is_eq()
                        && function.is_none_or(|function| function.provider == *provider)
                })
        }
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
fn function(image: &Image, name: Option<NameId>) -> Option<&'static Function> {
    let name = name.and_then(|id| image.names.get(id));
    FUNCTIONS
        .iter()
        .find(|function| Some(function.name) == name)
}
pub(super) struct BoundRuntime {
    pub imports: Vec<NativeImport<'static>>,
    pub config: Option<RuntimeConfig>,
    pub startup: Option<u64>,
}
pub(super) fn bind(image: &mut Image, prefix: usize) -> Result<BoundRuntime, String> {
    let needs_heap = match image.target.encoding {
        Encoding::Pe => image
            .imports
            .iter()
            .any(|import| function(image, import.name).is_some_and(|function| function.heap)),
        Encoding::Elf => image.relocations.iter().any(|relocation| {
            relocation.kind != 0
                && relocation
                    .symbol
                    .and_then(|index| image.symbols.get(index))
                    .filter(|symbol| !symbol.defined)
                    .and_then(|symbol| function(image, symbol.name))
                    .is_some_and(|function| function.heap)
        }),
    };
    let lifecycle = image.target.encoding == Encoding::Elf
        && image
            .dynamic
            .iter()
            .any(|&(tag, value)| value != 0 && matches!(tag, 12 | 13 | 25 | 26));
    let (config, startup) = if needs_heap || lifecycle {
        const BYTES: usize = 32 * 1024 * 1024;
        let page = qa_platform::native::PAGE_BYTES;
        let offset = image.bytes.len().div_ceil(page) * page;
        let bytes = page + if needs_heap { BYTES } else { 0 };
        let end = offset
            .checked_add(bytes)
            .filter(|&n| n <= 512 * 1024 * 1024)
            .ok_or("native runtime extent")?;
        let base = image
            .base
            .checked_add(offset as u64)
            .ok_or("native runtime address")?;
        base.checked_add(bytes as u64)
            .ok_or("native runtime extent")?;
        let config = RuntimeConfig {
            base,
            heap_bytes: if needs_heap { BYTES } else { 0 },
        };
        let mut storage = std::mem::take(&mut image.bytes).into_vec();
        storage.resize(end, 0);
        image.bytes = storage.into_boxed_slice();
        let mut regions = std::mem::take(&mut image.regions).into_vec();
        regions.push(qa_formats::program::native::Region {
            offset,
            length: bytes,
            read: true,
            write: true,
            execute: false,
        });
        image.regions = regions.into_boxed_slice();
        // argc is zero; argv and envp point to separate owned null terminators.
        (Some(config), lifecycle.then_some(base))
    } else {
        (None, None)
    };
    for &id in &image.needed {
        if !library(image, id, None) {
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
        let function =
            function(image, name).ok_or_else(|| import_error(image, name, provider, version))?;
        let supported_version = version.is_none_or(|id| {
            image
                .names
                .get(id)
                .is_some_and(|version| function.versions.contains(&version))
        });
        if provider.is_some_and(|id| !library(image, id, Some(function))) || !supported_version {
            return Err(import_error(image, name, provider, version));
        }
        if let Some(offset) = function.data_offset() {
            return config
                .map(|config| config.base + offset as u64)
                .ok_or_else(|| "native runtime data storage".into());
        }
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
    if image.target.encoding == Encoding::Pe
        && imports.iter().any(|i| {
            qa_platform::native::runtime::function(i.number).is_some_and(Function::windows_object)
        })
    {
        for function in FUNCTIONS.iter().filter(|f| f.windows_object()) {
            if !imports.iter().any(|i| i.number == function.number) {
                imports.push(NativeImport {
                    number: function.number,
                    abi,
                    parameters: function.parameters,
                    result: function.result,
                });
            }
        }
        let config = config.ok_or("native MSVC storage")?;
        let at = usize::try_from(config.base - image.base).map_err(|_| "native MSVC storage")?;
        let image_bytes = image.bytes.len();
        config
            .prepare_msvc(
                &mut image.bytes[at..at + qa_platform::native::PAGE_BYTES],
                |number| {
                    let ordinal = imports
                        .iter()
                        .position(|i| i.number == number)
                        .ok_or(qa_platform::native::NativeError::Extent)?;
                    NativeProcess::import_address(image.base, image_bytes, prefix + ordinal)
                },
            )
            .map_err(|e| format!("native MSVC tables: {e:?}"))?;
    }
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
    Ok(BoundRuntime {
        imports,
        config,
        startup,
    })
}
