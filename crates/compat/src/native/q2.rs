//! Q2 native API layouts over the one child backend and returned table binder.
use super::{Error, NamedExport, ReturnedTable, TableFunction, Vm, runtime::ImportTrap};
use crate::abi::{CallTable, Q2_CLASSIC, Q2_RERELEASE};
use crate::cvars::NativeCvars;
use crate::entities::{AppearanceLayout, EntityLayout, ExtendedAppearance};
use qa_core::{
    configstrings::ConfigLayout,
    names::NameTable,
    primitives::{NameId, RuleSetId},
};
use qa_formats::program::native::{Encoding, Image};
use qa_platform::native::{NativeAbi, NativeScalar, PAGE_BYTES};
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
    imports: &'static [crate::abi::q2::Import],
    public_imports: usize,
    functions: &'static [Function],
    entity: EntityLayout,
}
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
    imports: &crate::abi::q2::CLASSIC_IMPORTS,
    public_imports: 44,
    functions: CLASSIC_FUNCTIONS,
    entity: EntityLayout {
        source: RuleSetId::Quake2,
        bytes: 280,
        in_use: Some((96, false)),
        linked: None,
        link_count: 100,
        flags: 200,
        snapshot_exclusion_mask: 1,
        player_flag: 0,
        projectile_flag: 0,
        unsupported_flags: 0,
        mins: 204,
        maxs: 216,
        abs_min: 228,
        abs_max: 240,
        size: Some(252),
        solid: (264, false),
        owner: 272,
        owner_number: false,
        origin: 4,
        angles: 16,
        contents: None,
        world_zero: true,
        number: None,
        contents_pose: None,
        area: Some(192),
        area2: Some(196),
        clusters: Some((120, 124, 188)),
        network_solid: 72,
        appearance: AppearanceLayout {
            entity_type: None,
            other_entity: None,
            models: [Some(40), Some(44), Some(48), Some(52)],
            frame: 56,
            skin: Some(60),
            old_origin: 28,
            effects: Some(64),
            effects_wide: false,
            render_flags: 68,
            loop_sound: 76,
            event: (80, false),
            extended: None,
        },
        model_rules: qa_core::primitives::ModelRules {
            rotation: qa_core::primitives::ModelRotation::NegativeEuler,
            link_bounds: qa_core::primitives::RotatedLinkBounds::MaxAbsCube,
        },
    },
};
const RERELEASE: Layout = Layout {
    version: 2023,
    bytes: 272,
    entity_offset: 160,
    wide_stride: true,
    import_header: 16,
    imports: &crate::abi::q2::RERELEASE_IMPORTS,
    public_imports: 70,
    functions: RERELEASE_FUNCTIONS,
    entity: EntityLayout {
        source: RuleSetId::Quake2Rerelease,
        bytes: 1472,
        in_use: Some((1376, true)),
        linked: Some((1377, true)),
        link_count: 1380,
        flags: 1392,
        snapshot_exclusion_mask: 1,
        player_flag: 8,
        projectile_flag: 128,
        unsupported_flags: 0,
        mins: 1396,
        maxs: 1408,
        abs_min: 1420,
        abs_max: 1432,
        size: Some(1444),
        solid: (1456, true),
        owner: 1464,
        owner_number: false,
        origin: 4,
        angles: 16,
        contents: None,
        world_zero: true,
        number: None,
        contents_pose: None,
        area: Some(1384),
        area2: Some(1388),
        clusters: None,
        network_solid: 76,
        appearance: AppearanceLayout {
            entity_type: None,
            other_entity: None,
            models: [Some(40), Some(44), Some(48), Some(52)],
            frame: 56,
            skin: Some(60),
            old_origin: 28,
            effects: Some(64),
            effects_wide: true,
            render_flags: 72,
            loop_sound: 80,
            event: (84, true),
            extended: Some(ExtendedAppearance {
                alpha: 88,
                scale: 92,
                instance_bits: 96,
                loop_volume: 100,
                loop_attenuation: 104,
                owner: 108,
                old_frame: 112,
            }),
        },
        model_rules: CLASSIC.entity.model_rules,
    },
};

pub use crate::entities::Entities;
pub struct Game {
    pub vm: Vm,
    pub imports_address: u64,
    pub imports: &'static CallTable,
    layout: &'static Layout,
    first: u32,
}
fn layout(rules: RuleSetId) -> Option<&'static Layout> {
    match rules {
        RuleSetId::Quake2 => Some(&CLASSIC),
        RuleSetId::Quake2Rerelease => Some(&RERELEASE),
        _ => None,
    }
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
        let layout = layout(rules).ok_or(Error::Export)?;
        let configs = ConfigLayout::for_rules(rules);
        let client = crate::q2_abi::client_layout(rules).ok_or(Error::Export)?;
        if rules == RuleSetId::Quake2Rerelease && (interval_ms == 0 || 1000 % interval_ms != 0) {
            return Err(Error::Export);
        }
        if image.target.bits != 64 {
            return Err(Error::Export);
        }
        let mut names = NameTable::load_reserved(
            std::iter::empty(),
            image.names.len() + layout.public_imports,
            (0..image.names.len())
                .filter_map(|n| image.names.get(NameId(n as u32)))
                .map(<[u8]>::len)
                .sum::<usize>()
                + layout.imports[..layout.public_imports]
                    .iter()
                    .map(|import| import.name.len())
                    .sum::<usize>(),
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
            .take(layout.public_imports)
            .map(|import| names.intern(import.name).map_err(|_| Error::Export))
            .collect::<Result<Vec<_>, _>>()?;
        image.names = names;
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
        let command_offset = PAGE_BYTES
            .checked_add(cvar_bytes)
            .and_then(|offset| offset.checked_add(surface_bytes))
            .ok_or(Error::Export)?;
        let command_bytes =
            crate::command::NativeCommand::byte_length().div_ceil(PAGE_BYTES) * PAGE_BYTES;
        let config_offset = command_offset
            .checked_add(command_bytes)
            .ok_or(Error::Export)?;
        // Native pointers and the common rows use the same CS_SIZE capacities.
        let config_capacities: Vec<usize> = if layout.version == 2023 {
            (0..configs.count)
                .map(|index| {
                    configs
                        .capacity(index)
                        .map(|bytes| bytes + 1)
                        .ok_or(Error::Export)
                })
                .collect::<Result<_, _>>()?
        } else {
            Vec::new()
        };
        let config_bytes = if config_capacities.is_empty() {
            0
        } else {
            crate::configstrings::NativeConfigs::byte_length(&config_capacities)
                .map_err(Error::Service)?
                .div_ceil(PAGE_BYTES)
                * PAGE_BYTES
        };
        let filter_offset = config_offset
            .checked_add(config_bytes)
            .ok_or(Error::Export)?;
        let mut filter =
            (layout.version == 2023).then_some(qa_platform::native::NativeListFilter {
                buffer: 0,
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
        let length = filter_offset
            .checked_add(filter_bytes)
            .ok_or(Error::Export)?;
        // Classic print adapters share synchronous scratch in the load-owned
        // import page, past every pointer slot and before cvar storage.
        let format_offset = PAGE_BYTES / 2;
        if layout.import_header + layout.public_imports * 8 > format_offset {
            return Err(Error::Export);
        }
        let region = image
            .reserve_region(length, PAGE_BYTES, true, true, false)
            .map_err(|_| Error::Export)?;
        let offset = region.offset;
        let address = image.base + offset as u64;
        let command_address = address + command_offset as u64;
        let config_address = address + config_offset as u64;
        if let Some(filter) = &mut filter {
            filter.buffer = address + filter_offset as u64;
        }
        let format_address = address + format_offset as u64;
        let bytes = &mut image.bytes;
        if layout.import_header != 0 {
            bytes[offset..offset + 4].copy_from_slice(&(1000 / interval_ms).to_le_bytes());
            bytes[offset + 4..offset + 8].copy_from_slice(
                &(qa_core::primitives::ThinkTime::Milliseconds(i64::from(interval_ms)).seconds()
                    as f32)
                    .to_le_bytes(),
            );
            bytes[offset + 8..offset + 12].copy_from_slice(&interval_ms.to_le_bytes());
        }
        let abi = if image.target.encoding == Encoding::Pe {
            NativeAbi::Microsoft
        } else {
            NativeAbi::SystemV
        };
        let imports = layout
            .imports
            .iter()
            .enumerate()
            .map(|(ordinal, import)| {
                import
                    .native(ordinal, abi, format_address, filter)
                    .ok_or(Error::Export)
            })
            .collect::<Result<Vec<_>, _>>()?;
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
        vm.native_command =
            Some(crate::command::NativeCommand::load(command_address).map_err(Error::Service)?);
        if !config_capacities.is_empty() {
            vm.native_configs = Some(
                crate::configstrings::NativeConfigs::load(config_address, &config_capacities)
                    .map_err(Error::Service)?,
            );
        }
        vm.resources = Some(configs.resources);
        vm.surfaces = Some(
            crate::surfaces::NativeSurfaces::load(
                address + (PAGE_BYTES + cvar_bytes) as u64,
                geometry,
                layout.version == 2023,
                &mut crate::memory::process_memory(&mut vm.process)?,
            )
            .map_err(Error::Service)?,
        );
        vm.projection = Some(crate::projection::Projection::load_q2(
            crate::entities::EntityProjection::load(
                layout.entity_offset,
                layout.wide_stride,
                layout.entity,
                configs.resources[0],
                qa_world::entities::MAX_ENTITIES,
            ),
            client,
        ));
        vm.pmove = Some(crate::pmove::Pmove::load(
            if layout.version == 2023 {
                &crate::pmove::RERELEASE
            } else {
                &crate::pmove::CLASSIC
            },
            client,
        ));
        let pointers = (0..layout.public_imports)
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
        vm.client_think = layout
            .functions
            .iter()
            .position(|f| f.name == b"ClientThink")
            .map(|n| first + n as u32);
        vm.pmove_redirect = layout
            .functions
            .iter()
            .position(|f| f.name == b"Pmove")
            .map(|n| (first + n as u32, layout.public_imports));
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
    pub fn import_ordinal(&self, name: &[u8]) -> Option<usize> {
        self.layout.imports[..self.layout.public_imports]
            .iter()
            .position(|import| import.name == name)
    }
}
