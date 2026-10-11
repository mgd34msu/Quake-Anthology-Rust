//! Q2 native API layouts over the one child backend and returned table binder.
use super::{Error, NamedExport, ReturnedTable, TableFunction, Vm, runtime::ImportTrap};
use crate::abi::{CallTable, Q2_CLASSIC, Q2_RERELEASE};
use crate::cvars::NativeCvars;
use crate::entities::EntityLayout;
use crate::services::ResourceRange;
use qa_core::{
    names::NameTable,
    primitives::{NameId, RuleSetId},
};
use qa_formats::program::native::{Encoding, Image, Region};
use qa_platform::native::{NativeAbi, NativeImport, NativeScalar, PAGE_BYTES};
use std::time::Duration;

struct Function {
    name: &'static [u8],
    offset: usize,
    parameters: &'static [NativeScalar],
    result: NativeScalar,
}
macro_rules! functions {
    ($(($name:literal, $offset:literal, $result:ident, [$($parameter:ident),*])),* $(,)?) => {
        &[ $(Function { name: $name, offset: $offset, parameters: &[$(NativeScalar::$parameter),*], result: NativeScalar::$result }),* ]
    };
}
struct Layout {
    version: i32,
    bytes: usize,
    entity_offset: usize,
    wide_stride: bool,
    import_header: usize,
    imports: &'static [&'static [u8]],
    functions: &'static [Function],
    resources: [ResourceRange; 3],
    entity: EntityLayout,
}
const CLASSIC_IMPORTS: &[&[u8]] = &[
    b"bprintf",
    b"dprintf",
    b"cprintf",
    b"centerprintf",
    b"sound",
    b"positioned_sound",
    b"configstring",
    b"error",
    b"modelindex",
    b"soundindex",
    b"imageindex",
    b"setmodel",
    b"trace",
    b"pointcontents",
    b"inPVS",
    b"inPHS",
    b"SetAreaPortalState",
    b"AreasConnected",
    b"linkentity",
    b"unlinkentity",
    b"BoxEdicts",
    b"Pmove",
    b"multicast",
    b"unicast",
    b"WriteChar",
    b"WriteByte",
    b"WriteShort",
    b"WriteLong",
    b"WriteFloat",
    b"WriteString",
    b"WritePosition",
    b"WriteDir",
    b"WriteAngle",
    b"TagMalloc",
    b"TagFree",
    b"FreeTags",
    b"cvar",
    b"cvar_set",
    b"cvar_forceset",
    b"argc",
    b"argv",
    b"args",
    b"AddCommandString",
    b"DebugGraph",
];
const CLASSIC_FUNCTIONS: &[Function] = functions!(
    (b"Init", 8, Void, []),
    (b"Shutdown", 16, Void, []),
    (b"SpawnEntities", 24, Void, [Word, Word, Word]),
    (b"WriteGame", 32, Void, [Word, I32]),
    (b"ReadGame", 40, Void, [Word]),
    (b"WriteLevel", 48, Void, [Word]),
    (b"ReadLevel", 56, Void, [Word]),
    (b"ClientConnect", 64, I32, [Word, Word]),
    (b"ClientBegin", 72, Void, [Word]),
    (b"ClientUserinfoChanged", 80, Void, [Word, Word]),
    (b"ClientDisconnect", 88, Void, [Word]),
    (b"ClientCommand", 96, Void, [Word]),
    (b"ClientThink", 104, Void, [Word, Word]),
    (b"RunFrame", 112, Void, []),
    (b"ServerCommand", 120, Void, []),
);
const RERELEASE_IMPORTS: &[&[u8]] = &[
    b"Broadcast_Print",
    b"Com_Print",
    b"Client_Print",
    b"Center_Print",
    b"sound",
    b"positioned_sound",
    b"local_sound",
    b"configstring",
    b"get_configstring",
    b"Com_Error",
    b"modelindex",
    b"soundindex",
    b"imageindex",
    b"setmodel",
    b"trace",
    b"clip",
    b"pointcontents",
    b"inPVS",
    b"inPHS",
    b"SetAreaPortalState",
    b"AreasConnected",
    b"linkentity",
    b"unlinkentity",
    b"BoxEdicts",
    b"multicast",
    b"unicast",
    b"WriteChar",
    b"WriteByte",
    b"WriteShort",
    b"WriteLong",
    b"WriteFloat",
    b"WriteString",
    b"WritePosition",
    b"WriteDir",
    b"WriteAngle",
    b"WriteEntity",
    b"TagMalloc",
    b"TagFree",
    b"FreeTags",
    b"cvar",
    b"cvar_set",
    b"cvar_forceset",
    b"argc",
    b"argv",
    b"args",
    b"AddCommandString",
    b"DebugGraph",
    b"GetExtension",
    b"Bot_RegisterEdict",
    b"Bot_UnRegisterEdict",
    b"Bot_MoveToPoint",
    b"Bot_FollowActor",
    b"GetPathToGoal",
    b"Loc_Print",
    b"Draw_Line",
    b"Draw_Point",
    b"Draw_Circle",
    b"Draw_Bounds",
    b"Draw_Sphere",
    b"Draw_OrientedWorldText",
    b"Draw_StaticWorldText",
    b"Draw_Cylinder",
    b"Draw_Ray",
    b"Draw_Arrow",
    b"ReportMatchDetails_Multicast",
    b"ServerFrame",
    b"SendToClipBoard",
    b"Info_ValueForKey",
    b"Info_RemoveKey",
    b"Info_SetValueForKey",
];
const RERELEASE_FUNCTIONS: &[Function] = functions!(
    (b"PreInit", 8, Void, []),
    (b"Init", 16, Void, []),
    (b"Shutdown", 24, Void, []),
    (b"SpawnEntities", 32, Void, [Word, Word, Word]),
    (b"WriteGameJson", 40, Word, [U8, Word]),
    (b"ReadGameJson", 48, Void, [Word]),
    (b"WriteLevelJson", 56, Word, [U8, Word]),
    (b"ReadLevelJson", 64, Void, [Word]),
    (b"CanSave", 72, U8, []),
    (
        b"ClientChooseSlot",
        80,
        Word,
        [Word, Word, U8, Word, Word, U8]
    ),
    (b"ClientConnect", 88, U8, [Word, Word, Word, U8]),
    (b"ClientBegin", 96, Void, [Word]),
    (b"ClientUserinfoChanged", 104, Void, [Word, Word]),
    (b"ClientDisconnect", 112, Void, [Word]),
    (b"ClientCommand", 120, Void, [Word]),
    (b"ClientThink", 128, Void, [Word, Word]),
    (b"RunFrame", 136, Void, [U8]),
    (b"PrepFrame", 144, Void, []),
    (b"ServerCommand", 152, Void, []),
    (b"Pmove", 192, Void, [Word]),
    (b"GetExtension", 200, Word, [Word]),
    (b"Bot_SetWeapon", 208, Void, [Word, I32, U8]),
    (b"Bot_TriggerEdict", 216, Void, [Word, Word]),
    (b"Bot_UseItem", 224, Void, [Word, I32]),
    (b"Bot_GetItemID", 232, I32, [Word]),
    (b"Edict_ForceLookAtPoint", 240, Void, [Word, Word]),
    (b"Bot_PickedUpItem", 248, U8, [Word, Word]),
    (b"Entity_IsVisibleToPlayer", 256, U8, [Word, Word]),
    (b"GetShadowLightData", 264, Word, [I32]),
);

const CLASSIC: Layout = Layout {
    version: 3,
    bytes: 152,
    entity_offset: 128,
    wide_stride: false,
    import_header: 0,
    imports: CLASSIC_IMPORTS,
    functions: CLASSIC_FUNCTIONS,
    entity: EntityLayout {
        bytes: 280,
        in_use: (96, false),
        linked: None,
        link_count: 100,
        flags: 200,
        player_flag: 0,
        projectile_flag: 0,
        mins: 204,
        maxs: 216,
        abs_min: 228,
        abs_max: 240,
        size: 252,
        solid: (264, false),
        owner: 272,
        area: 192,
        area2: 196,
        clusters: Some((120, 124, 188)),
        network_solid: 72,
        model_rules: qa_core::primitives::ModelRules {
            rotation: qa_core::primitives::ModelRotation::NegativeEuler,
            link_bounds: qa_core::primitives::RotatedLinkBounds::MaxAbsCube,
        },
    },
    resources: [
        ResourceRange {
            first: 32,
            count: 256,
        },
        ResourceRange {
            first: 288,
            count: 256,
        },
        ResourceRange {
            first: 544,
            count: 256,
        },
    ],
};
const RERELEASE: Layout = Layout {
    version: 2023,
    bytes: 272,
    entity_offset: 160,
    wide_stride: true,
    import_header: 16,
    imports: RERELEASE_IMPORTS,
    functions: RERELEASE_FUNCTIONS,
    entity: EntityLayout {
        bytes: 1472,
        in_use: (1376, true),
        linked: Some(1377),
        link_count: 1380,
        flags: 1392,
        player_flag: 8,
        projectile_flag: 128,
        mins: 1396,
        maxs: 1408,
        abs_min: 1420,
        abs_max: 1432,
        size: 1444,
        solid: (1456, true),
        owner: 1464,
        area: 1384,
        area2: 1388,
        clusters: None,
        network_solid: 76,
        model_rules: CLASSIC.entity.model_rules,
    },
    resources: [
        ResourceRange {
            first: 62,
            count: 8192,
        },
        ResourceRange {
            first: 8254,
            count: 2048,
        },
        ResourceRange {
            first: 10302,
            count: 512,
        },
    ],
};

pub use crate::entities::Entities;
pub struct Game {
    pub vm: Vm,
    pub imports_address: u64,
    pub imports: &'static CallTable,
    layout: &'static Layout,
    first: u32,
}
impl Game {
    /// Bind the API shape at load. Unsupported engine services remain named traps; reaching one rejects only
    /// that call through the existing missing-import path.
    pub fn map(
        mut image: Image,
        rules: RuleSetId,
        interval_ms: u32,
        geometry: &qa_world::collision::CollisionStore,
        timeout: Duration,
    ) -> Result<Self, Error> {
        let layout = match rules {
            RuleSetId::Quake2 => &CLASSIC,
            RuleSetId::Quake2Rerelease if interval_ms != 0 && 1000 % interval_ms == 0 => &RERELEASE,
            _ => return Err(Error::Export),
        };
        if image.target.bits != 64 {
            return Err(Error::Export);
        }
        let mut names = NameTable::load_reserved(
            std::iter::empty(),
            image.names.len() + layout.imports.len(),
            (0..image.names.len())
                .filter_map(|n| image.names.get(NameId(n as u32)))
                .map(<[u8]>::len)
                .sum::<usize>()
                + layout.imports.iter().map(|n| n.len()).sum::<usize>(),
        )
        .map_err(|_| Error::Export)?;
        // Re-intern in numeric order to preserve every existing exact NameId.
        for n in 0..image.names.len() {
            let id = NameId(n as u32);
            if names
                .intern(image.names.get(id).ok_or(Error::Export)?)
                .map_err(|_| Error::Export)?
                != id
            {
                return Err(Error::Export);
            }
        }
        let labels = layout
            .imports
            .iter()
            .map(|name| names.intern(name).map_err(|_| Error::Export))
            .collect::<Result<Vec<_>, _>>()?;
        image.names = names;
        let offset = image
            .bytes
            .len()
            .div_ceil(PAGE_BYTES)
            .checked_mul(PAGE_BYTES)
            .ok_or(Error::Export)?;
        let cvar_capacity = 1024;
        let cvar_bytes = NativeCvars::byte_length(cvar_capacity)
            .ok_or(Error::Export)?
            .div_ceil(PAGE_BYTES)
            * PAGE_BYTES;
        let surface_bytes =
            crate::surfaces::NativeSurfaces::byte_length(geometry, layout.version == 2023)
                .ok_or(Error::Export)?
                .div_ceil(PAGE_BYTES)
                .checked_mul(PAGE_BYTES)
                .ok_or(Error::Export)?;
        let filter_offset = offset
            .checked_add(PAGE_BYTES + cvar_bytes + surface_bytes)
            .ok_or(Error::Export)?;
        let filter_address = image
            .base
            .checked_add(filter_offset as u64)
            .ok_or(Error::Export)?;
        let filter = (layout.version == 2023).then_some(qa_platform::native::NativeListFilter {
            buffer: filter_address,
            capacity: qa_world::entities::MAX_ENTITIES as u32,
            depth: 8,
            list: 2,
            limit: 3,
            callback: 5,
            data: 6,
            keep: 0,
            end: 64,
        });
        let filter_bytes = filter
            .map_or(Some(0), |filter| filter.byte_length())
            .ok_or(Error::Export)?;
        let end = filter_offset
            .checked_add(filter_bytes)
            .filter(|&n| n <= 512 * 1024 * 1024)
            .ok_or(Error::Export)?;
        let address = image.base.checked_add(offset as u64).ok_or(Error::Export)?;
        image.base.checked_add(end as u64).ok_or(Error::Export)?;
        let mut bytes = std::mem::take(&mut image.bytes).into_vec();
        bytes.resize(end, 0);
        if layout.import_header != 0 {
            bytes[offset..offset + 4].copy_from_slice(&(1000 / interval_ms).to_le_bytes());
            bytes[offset + 4..offset + 8].copy_from_slice(
                &(qa_core::primitives::ThinkTime::Milliseconds(i64::from(interval_ms)).seconds()
                    as f32)
                    .to_le_bytes(),
            );
            bytes[offset + 8..offset + 12].copy_from_slice(&interval_ms.to_le_bytes());
        }
        image.bytes = bytes.into_boxed_slice();
        let mut regions = std::mem::take(&mut image.regions).into_vec();
        regions.push(Region {
            offset,
            length: PAGE_BYTES + cvar_bytes + surface_bytes + filter_bytes,
            read: true,
            write: true,
            execute: false,
        });
        image.regions = regions.into_boxed_slice();
        let abi = if image.target.encoding == Encoding::Pe {
            NativeAbi::Microsoft
        } else {
            NativeAbi::SystemV
        };
        let imports = labels
            .iter()
            .enumerate()
            .map(|(ordinal, _)| {
                let cvar = if layout.version == 2023 { 39 } else { 36 };
                let local = if ordinal == cvar - 3 {
                    Some(if layout.version == 2023 {
                        qa_platform::native::runtime::TAG_MALLOC64
                    } else {
                        qa_platform::native::runtime::TAG_MALLOC32
                    })
                } else if ordinal == cvar - 2 {
                    Some(qa_platform::native::runtime::TAG_FREE)
                } else if ordinal == cvar - 1 {
                    Some(qa_platform::native::runtime::FREE_TAGS)
                } else {
                    None
                };
                if let Some(number) = local {
                    let function =
                        qa_platform::native::runtime::function(number).ok_or(Error::Export)?;
                    return Ok(NativeImport {
                        filter: None,
                        trap: false,
                        number,
                        abi,
                        parameters: function.parameters,
                        result: function.result,
                    });
                }
                let signature = if ordinal == cvar {
                    Some((
                        &[NativeScalar::Word, NativeScalar::Word, NativeScalar::U32][..],
                        NativeScalar::Word,
                    ))
                } else if ordinal == cvar + 1 || ordinal == cvar + 2 {
                    Some((
                        &[NativeScalar::Word, NativeScalar::Word][..],
                        NativeScalar::Word,
                    ))
                } else if (if layout.version == 2023 {
                    21..23
                } else {
                    18..20
                })
                .contains(&ordinal)
                    || (layout.version == 2023 && matches!(ordinal, 48 | 49))
                {
                    Some((&[NativeScalar::Word][..], NativeScalar::Void))
                } else if (if layout.version == 2023 {
                    10..13
                } else {
                    8..11
                })
                .contains(&ordinal)
                {
                    Some((&[NativeScalar::Word][..], NativeScalar::I32))
                } else if ordinal == if layout.version == 2023 { 13 } else { 11 } {
                    Some((
                        &[NativeScalar::Word, NativeScalar::Word][..],
                        NativeScalar::Void,
                    ))
                } else if ordinal == if layout.version == 2023 { 14 } else { 12 } {
                    Some((
                        &[
                            NativeScalar::Word,
                            NativeScalar::Word,
                            NativeScalar::Word,
                            NativeScalar::Word,
                            NativeScalar::Word,
                            NativeScalar::Word,
                            NativeScalar::U32,
                        ][..],
                        NativeScalar::Word,
                    ))
                } else if ordinal == if layout.version == 2023 { 23 } else { 20 } {
                    Some((
                        if layout.version == 2023 {
                            &[
                                NativeScalar::Word,
                                NativeScalar::Word,
                                NativeScalar::Word,
                                NativeScalar::Word,
                                NativeScalar::I32,
                                NativeScalar::Word,
                                NativeScalar::Word,
                            ][..]
                        } else {
                            &[
                                NativeScalar::Word,
                                NativeScalar::Word,
                                NativeScalar::Word,
                                NativeScalar::I32,
                                NativeScalar::I32,
                            ][..]
                        },
                        if layout.version == 2023 {
                            NativeScalar::Word
                        } else {
                            NativeScalar::I32
                        },
                    ))
                } else if ordinal == if layout.version == 2023 { 7 } else { 6 } {
                    Some((
                        &[NativeScalar::I32, NativeScalar::Word][..],
                        NativeScalar::Void,
                    ))
                } else if layout.version == 2023 && matches!(ordinal, 1 | 9) {
                    Some((&[NativeScalar::Word][..], NativeScalar::Void))
                } else {
                    None
                };
                Ok(NativeImport {
                    filter: if ordinal == 23 { filter } else { None },
                    trap: signature.is_none(),
                    number: ordinal as u32,
                    abi,
                    parameters: signature.map_or(&[], |s| s.0),
                    result: signature.map_or(NativeScalar::Void, |s| s.1),
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let mut vm = Vm::map_image(
            image,
            &[NamedExport {
                name: b"GetGameAPI",
                command: None,
                parameters: &[NativeScalar::Word],
                result: NativeScalar::Word,
            }],
            &imports,
            timeout,
        )?;
        vm.cvars = Some(NativeCvars::load(
            address + PAGE_BYTES as u64,
            cvar_capacity,
            layout.version == 2023,
        ));
        vm.resources = Some(layout.resources);
        vm.surfaces = Some(
            crate::surfaces::NativeSurfaces::load(
                address + (PAGE_BYTES + cvar_bytes) as u64,
                geometry,
                layout.version == 2023,
                &mut crate::memory::ModuleMemory::borrow(
                    vm.process.base(),
                    vm.process.memory_mut().map_err(Error::Process)?,
                )
                .map_err(|_| Error::Service(crate::services::CallError::Memory))?,
            )
            .map_err(Error::Service)?,
        );
        vm.entities = Some(crate::entities::EntityProjection::load(
            layout.entity_offset,
            layout.wide_stride,
            layout.entity,
            layout.resources[0],
        ));
        let pointers = (0..imports.len())
            .map(|n| vm.process.import_pointer(n).ok_or(Error::Export))
            .collect::<Result<Vec<_>, _>>()?;
        let memory = vm.process.memory_mut().map_err(Error::Process)?;
        for (ordinal, pointer) in pointers.into_iter().enumerate() {
            let at = offset + layout.import_header + ordinal * 8;
            memory[at..at + 8].copy_from_slice(&pointer.to_le_bytes());
        }
        let mut traps = std::mem::take(&mut vm.traps).into_vec();
        traps.extend(
            labels
                .into_iter()
                .enumerate()
                .filter(|(ordinal, _)| imports[*ordinal].trap)
                .map(|(ordinal, name)| ImportTrap {
                    ordinal,
                    name: Some(name),
                    provider: None,
                    version: None,
                    symbol_ordinal: None,
                    calls: 0,
                }),
        );
        vm.traps = traps.into_boxed_slice();
        let first = vm.declare_table(ReturnedTable {
            version: layout.version,
            bytes: layout.bytes,
            functions: layout
                .functions
                .iter()
                .map(|f| TableFunction {
                    offset: f.offset,
                    parameters: f.parameters,
                    result: f.result,
                })
                .collect(),
        })?;
        Ok(Self {
            vm,
            imports_address: address,
            imports: if layout.version == 2023 {
                &Q2_RERELEASE
            } else {
                &Q2_CLASSIC
            },
            layout,
            first,
        })
    }
    /// Resolve native names only at load; session dispatch caches the number.
    pub fn entry(&self, name: &[u8]) -> Option<u32> {
        self.layout
            .functions
            .iter()
            .position(|f| f.name == name)
            .map(|n| self.first + n as u32)
    }
    pub fn entities(&mut self) -> Result<Entities, Error> {
        self.vm.entities()
    }
}
