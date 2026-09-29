//! Q2 rerelease primary world profiles and validation.
//!
//! Donor: `src/compat/q2/rerelease/world-profile.ts` — bridges
//! artifact-qualified source calls, layouts, entries and gameplay metadata.

use std::collections::HashMap;

use qa_guest::core::contracts::{GuestLayout, GuestStorage};
use qa_world::combat::ItemId;
use thiserror::Error;

use super::layouts::{client_layout, edict_layout, private_edict_prefix_layout};

/// Retail artifact digest.
pub const RETAIL_DIGEST: &str = "sha256:045d49c53722d9b922caf14f168dd28a97d4c514a6e443a3140560f8668baccd";

/// World profile failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WorldProfileError {
    /// Profile or read failure with detail.
    #[error("{0}")]
    Invalid(String),
}

/// Source field in a parsed or converted layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceField {
    /// Field name.
    pub name: String,
    /// Byte offset.
    pub byte_offset: usize,
    /// Element storage.
    pub storage: GuestStorage,
    /// Element count.
    pub count: usize,
}

/// Source record layout mirror.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLayout {
    /// Layout identity.
    pub id: String,
    /// Record length.
    pub byte_length: usize,
    /// Alignment.
    pub alignment: usize,
    /// Pointer width.
    pub pointer_bytes: usize,
    /// Fields.
    pub fields: Vec<SourceField>,
}

/// Convert a guest layout into a source layout mirror.
#[must_use]
pub fn source_layout(layout: &GuestLayout) -> SourceLayout {
    SourceLayout {
        id: layout.id.clone(),
        byte_length: layout.byte_length,
        alignment: layout.alignment,
        pointer_bytes: layout.pointer_bytes,
        fields: layout
            .fields
            .iter()
            .map(|field| SourceField {
                name: field.name.clone(),
                byte_offset: field.byte_offset,
                storage: field.storage,
                count: field.count,
            })
            .collect(),
    }
}

fn storage_bytes(storage: GuestStorage) -> usize {
    match storage {
        GuestStorage::Int8 | GuestStorage::Uint8 => 1,
        GuestStorage::Int16 | GuestStorage::Uint16 => 2,
        GuestStorage::Int32 | GuestStorage::Uint32 | GuestStorage::Float32 => 4,
        GuestStorage::Int64 | GuestStorage::Uint64 | GuestStorage::Float64 | GuestStorage::Pointer => 8,
    }
}

/// General-purpose register for region locations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorldRegister {
    /// rax.
    Rax,
    /// rcx.
    Rcx,
    /// rdx.
    Rdx,
    /// rbx.
    Rbx,
    /// rbp.
    Rbp,
    /// rsi.
    Rsi,
    /// rdi.
    Rdi,
    /// r8.
    R8,
    /// r9.
    R9,
    /// r10.
    R10,
    /// r11.
    R11,
    /// r12.
    R12,
    /// r13.
    R13,
    /// r14.
    R14,
    /// r15.
    R15,
}

impl WorldRegister {
    /// Parse a register name.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "rax" => Self::Rax,
            "rcx" => Self::Rcx,
            "rdx" => Self::Rdx,
            "rbx" => Self::Rbx,
            "rbp" => Self::Rbp,
            "rsi" => Self::Rsi,
            "rdi" => Self::Rdi,
            "r8" => Self::R8,
            "r9" => Self::R9,
            "r10" => Self::R10,
            "r11" => Self::R11,
            "r12" => Self::R12,
            "r13" => Self::R13,
            "r14" => Self::R14,
            "r15" => Self::R15,
            _ => return None,
        })
    }
}

/// Scalar storage for region locations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocationStorage {
    /// Pointer.
    Pointer,
    /// Signed 32-bit.
    Int32,
    /// Unsigned 32-bit.
    Uint32,
}

impl LocationStorage {
    /// Parse a storage name.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "pointer" => Self::Pointer,
            "int32" => Self::Int32,
            "uint32" => Self::Uint32,
            _ => return None,
        })
    }

    /// Width in bytes at 8-byte pointers.
    #[must_use]
    pub const fn bytes(self) -> usize {
        match self {
            Self::Pointer => 8,
            Self::Int32 | Self::Uint32 => 4,
        }
    }
}

/// Register or stack location in a native region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorldLocation {
    /// Register lane.
    Register {
        /// Register.
        register: WorldRegister,
        /// Storage.
        storage: LocationStorage,
    },
    /// Stack offset.
    Stack {
        /// Offset.
        offset: usize,
        /// Storage.
        storage: LocationStorage,
    },
}

impl WorldLocation {
    /// Location storage.
    #[must_use]
    pub const fn storage(self) -> LocationStorage {
        match self {
            Self::Register { storage, .. } | Self::Stack { storage, .. } => storage,
        }
    }
}

/// Combat semantic field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CombatField {
    /// Target entity.
    Target,
    /// Inflictor entity.
    Inflictor,
    /// Attacker entity.
    Attacker,
    /// Direction vector.
    Direction,
    /// Point vector.
    Point,
    /// Normal vector.
    Normal,
    /// Amount.
    Amount,
    /// Knockback.
    Knockback,
    /// Damage flags.
    Flags,
    /// Damage cause.
    Cause,
    /// Sparks flag.
    Sparks,
    /// Pain kick.
    Kick,
}

impl CombatField {
    /// Parse a field name.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "target" => Self::Target,
            "inflictor" => Self::Inflictor,
            "attacker" => Self::Attacker,
            "direction" => Self::Direction,
            "point" => Self::Point,
            "normal" => Self::Normal,
            "amount" => Self::Amount,
            "knockback" => Self::Knockback,
            "flags" => Self::Flags,
            "cause" => Self::Cause,
            "sparks" => Self::Sparks,
            "kick" => Self::Kick,
            _ => return None,
        })
    }
}

/// Combat calling convention.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CombatConvention {
    /// cdecl.
    Cdecl,
    /// stdcall.
    Stdcall,
    /// fastcall.
    Fastcall,
    /// thiscall.
    Thiscall,
    /// Microsoft x64.
    MicrosoftX64,
}

impl CombatConvention {
    /// Parse a convention name.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "cdecl" => Self::Cdecl,
            "stdcall" => Self::Stdcall,
            "fastcall" => Self::Fastcall,
            "thiscall" => Self::Thiscall,
            "microsoft-x64" => Self::MicrosoftX64,
            _ => return None,
        })
    }
}

/// Combat operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CombatOperation {
    /// Damage.
    Damage,
    /// Regular armor.
    RegularArmor,
    /// Power armor.
    PowerArmor,
    /// Pain.
    Pain,
    /// Death.
    Death,
    /// Deferred reaction.
    DeferredReaction,
}

/// Required semantic fields per operation at 8-byte pointers.
#[must_use]
pub const fn operation_fields(operation: CombatOperation) -> &'static [CombatField] {
    use CombatField as F;
    match operation {
        CombatOperation::Damage => &[
            F::Target,
            F::Inflictor,
            F::Attacker,
            F::Direction,
            F::Point,
            F::Normal,
            F::Amount,
            F::Knockback,
            F::Flags,
            F::Cause,
        ],
        CombatOperation::RegularArmor => &[F::Target, F::Point, F::Normal, F::Amount, F::Sparks, F::Flags],
        CombatOperation::PowerArmor => &[F::Target, F::Point, F::Normal, F::Amount, F::Flags],
        CombatOperation::Pain => &[F::Target, F::Attacker, F::Kick, F::Amount, F::Cause],
        CombatOperation::Death => &[F::Target, F::Inflictor, F::Attacker, F::Amount, F::Point, F::Cause],
        CombatOperation::DeferredReaction => &[F::Target],
    }
}

/// Default value layout for a combat argument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CombatValueLayout {
    /// Scalar storage.
    Scalar(GuestStorage),
    /// Aggregate record.
    Aggregate(SourceLayout),
}

impl CombatValueLayout {
    /// Byte size of the layout.
    #[must_use]
    pub fn bytes(&self) -> usize {
        match self {
            Self::Scalar(storage) => storage_bytes(*storage),
            Self::Aggregate(layout) => layout.byte_length,
        }
    }

    /// Whether the layout carries pointers.
    #[must_use]
    pub fn has_pointer(&self) -> bool {
        match self {
            Self::Scalar(storage) => *storage == GuestStorage::Pointer,
            Self::Aggregate(layout) => layout.fields.iter().any(|field| field.storage == GuestStorage::Pointer),
        }
    }
}

/// One combat call argument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CombatArgument {
    /// Semantic field.
    Field(CombatField),
    /// Default value bytes.
    Value {
        /// Value layout.
        layout: CombatValueLayout,
        /// Default bytes.
        bytes: Vec<u8>,
    },
    /// Image-relative address default.
    Address {
        /// RVA plus indirections, or null.
        address: Option<ImageAddress>,
    },
}

/// Image-relative address with indirections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageAddress {
    /// Base RVA.
    pub rva: u64,
    /// Indirection offsets.
    pub indirections: Vec<u64>,
}

/// One native combat call declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeCombatCall {
    /// Calling convention.
    pub convention: CombatConvention,
    /// Arguments.
    pub arguments: Vec<CombatArgument>,
}

/// Stock combat call: every semantic field in order.
#[must_use]
pub fn stock_combat_call(operation: CombatOperation) -> NativeCombatCall {
    NativeCombatCall {
        convention: CombatConvention::MicrosoftX64,
        arguments: operation_fields(operation)
            .iter()
            .map(|field| CombatArgument::Field(*field))
            .collect(),
    }
}

/// Validate a combat call against its operation.
pub fn validate_combat_call(call: &NativeCombatCall, operation: CombatOperation) -> Result<(), WorldProfileError> {
    if call.convention != CombatConvention::MicrosoftX64 {
        return Err(WorldProfileError::Invalid(
            "Native combat convention does not match its source architecture".to_string(),
        ));
    }
    let required = operation_fields(operation);
    let mut seen = std::collections::HashSet::new();
    for argument in &call.arguments {
        match argument {
            CombatArgument::Field(field) => {
                if !required.contains(field) || !seen.insert(*field) {
                    return Err(WorldProfileError::Invalid(
                        "Native combat fields must occur exactly once".to_string(),
                    ));
                }
            }
            CombatArgument::Address { address } => {
                if let Some(address) = address {
                    for value in std::iter::once(address.rva).chain(address.indirections.iter().copied()) {
                        if value > 0xffff_ffff {
                            return Err(WorldProfileError::Invalid(
                                "Native combat address exceeds its image declaration".to_string(),
                            ));
                        }
                    }
                }
            }
            CombatArgument::Value { layout, bytes } => {
                if layout.has_pointer() {
                    return Err(WorldProfileError::Invalid(
                        "Native combat pointer defaults require an image-relative address".to_string(),
                    ));
                }
                if bytes.len() != layout.bytes() {
                    return Err(WorldProfileError::Invalid(
                        "Native combat default bytes do not match their source argument".to_string(),
                    ));
                }
            }
        }
    }
    if seen.len() != required.len() {
        return Err(WorldProfileError::Invalid(
            "Native combat declaration omits a required source field".to_string(),
        ));
    }
    Ok(())
}

/// Client profile: artifact authority, layout and counts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldClientProfile {
    /// Artifact digest.
    pub digest: String,
    /// Client record layout.
    pub layout: SourceLayout,
    /// Inventory count.
    pub inventory_count: usize,
    /// Ammunition count.
    pub ammo_count: usize,
}

/// Native entry RVAs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorldEntries {
    /// `G_Spawn`.
    pub spawn: u64,
    /// `G_FreeEdict`.
    pub free: u64,
    /// `T_Damage`.
    pub damage: u64,
    /// Power-armor stage.
    pub power_armor: u64,
    /// Deferred pain processor.
    pub process_pain: u64,
    /// Level time.
    pub time: u64,
    /// Regular-armor region entry.
    pub regular_armor_entry: u64,
    /// Regular-armor region join.
    pub regular_armor_join: u64,
}

/// Regular-armor region locations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegularArmorRegion {
    /// Target location.
    pub target: WorldLocation,
    /// Amount location.
    pub amount: WorldLocation,
    /// Point location.
    pub point: WorldLocation,
    /// Normal location.
    pub normal: WorldLocation,
    /// Flags location.
    pub flags: WorldLocation,
    /// Result location.
    pub result: WorldLocation,
    /// Register repair moves.
    pub repair: Vec<ArmorRepair>,
}

/// One armor register repair move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArmorRepair {
    /// Source location.
    pub source: WorldLocation,
    /// Target location.
    pub target: WorldLocation,
}

/// Monster accumulator offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonsterAccumulator {
    /// Attacker pointer.
    pub attacker: usize,
    /// Inflictor pointer.
    pub inflictor: usize,
    /// Blood counter.
    pub blood: usize,
    /// Knockback counter.
    pub knockback: usize,
    /// Damage point.
    pub point: usize,
    /// `mod_t` bytes.
    pub modem: usize,
    /// Invulnerability deadline.
    pub invincible_time: usize,
}

/// Inventory source declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InventorySource {
    /// Resolve by classname.
    Classname {
        /// Classname.
        name: String,
    },
    /// Explicit index.
    Index {
        /// Index.
        index: usize,
    },
    /// Remaining unnamed slot.
    Remaining,
}

/// Inventory capacity declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InventoryCapacity {
    /// Ammunition slot.
    Ammo {
        /// `max_ammo` index.
        source_index: usize,
    },
    /// Fixed count.
    Fixed {
        /// Count.
        count: i32,
    },
}

/// One inventory roster row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryRow {
    /// Item identity.
    pub item: ItemId,
    /// Source declaration.
    pub source: InventorySource,
    /// Capacity declaration.
    pub capacity: InventoryCapacity,
}

/// Armor metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArmorMetadata {
    /// Armor info table RVA.
    pub table: u64,
    /// Table stride.
    pub stride: usize,
    /// Normal protection offset.
    pub normal: usize,
    /// Energy protection offset.
    pub energy: usize,
    /// Regular armor items.
    pub regular: Vec<ItemId>,
    /// Empty armor item.
    pub empty: ItemId,
    /// Screen item.
    pub screen: ItemId,
    /// Shield item.
    pub shield: ItemId,
    /// Cell item.
    pub cells: ItemId,
    /// Cell inventory index.
    pub cells_index: usize,
}

/// Entity flag bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlagBits {
    /// Godmode bit.
    pub godmode: u64,
    /// Notarget bit.
    pub notarget: u64,
    /// No-knockback bit.
    pub no_knockback: u64,
    /// Power-armor bit.
    pub power_armor: u64,
}

/// Movement body boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MovementBody {
    /// Dimensions entry RVA.
    pub dimensions: u64,
    /// Trace entry RVA.
    pub trace: u64,
    /// Movement global RVA.
    pub movement_global: u64,
}

/// Speed load: continuation RVA plus XMM register.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpeedLoad {
    /// Continuation RVA.
    pub next: u64,
    /// Register index.
    pub register: usize,
}

/// Movement metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovementMetadata {
    /// Body boundary, if admitted.
    pub body: Option<MovementBody>,
    /// Game API RVA.
    pub game_api: u64,
    /// Pmove RVA.
    pub pmove: u64,
    /// Speed loads.
    pub speed_loads: Vec<SpeedLoad>,
}

/// Combat call set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldCalls {
    /// Pain call.
    pub pain: NativeCombatCall,
    /// Death call.
    pub death: NativeCombatCall,
    /// Deferred reaction call.
    pub process_pain: NativeCombatCall,
    /// Damage call.
    pub damage: NativeCombatCall,
    /// Power-armor call.
    pub power_armor: NativeCombatCall,
}

/// Primary world profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RereleasePrimaryWorldProfile {
    /// Combat calls.
    pub calls: WorldCalls,
    /// Artifact digest.
    pub digest: String,
    /// Edict layout.
    pub edict: SourceLayout,
    /// Client profile.
    pub client: WorldClientProfile,
    /// Native entries.
    pub entries: WorldEntries,
    /// Regular-armor region.
    pub regular_armor: RegularArmorRegion,
    /// Monster accumulator.
    pub monster: MonsterAccumulator,
    /// Inventory roster.
    pub inventory: Vec<InventoryRow>,
    /// Armor metadata.
    pub armor: ArmorMetadata,
    /// Flag bits.
    pub flags: FlagBits,
    /// Movement metadata.
    pub movement: MovementMetadata,
}

/// `g_items.cpp` itemlist classnames (the disabled beta disintegrator is
/// not an item).
pub const CLASSNAMES: &[&str] = &[
    "item_armor_body",
    "item_armor_combat",
    "item_armor_jacket",
    "item_armor_shard",
    "item_power_screen",
    "item_power_shield",
    "weapon_grapple",
    "weapon_blaster",
    "weapon_chainfist",
    "weapon_shotgun",
    "weapon_supershotgun",
    "weapon_machinegun",
    "weapon_etf_rifle",
    "weapon_chaingun",
    "ammo_grenades",
    "ammo_trap",
    "ammo_tesla",
    "weapon_grenadelauncher",
    "weapon_proxlauncher",
    "weapon_rocketlauncher",
    "weapon_hyperblaster",
    "weapon_boomer",
    "weapon_plasmabeam",
    "weapon_railgun",
    "weapon_phalanx",
    "weapon_bfg",
    "weapon_disintegrator",
    "ammo_shells",
    "ammo_bullets",
    "ammo_cells",
    "ammo_rockets",
    "ammo_slugs",
    "ammo_magslug",
    "ammo_flechettes",
    "ammo_prox",
    "ammo_nuke",
    "ammo_disruptor",
    "item_quad",
    "item_quadfire",
    "item_invulnerability",
    "item_invisibility",
    "item_silencer",
    "item_breather",
    "item_enviro",
    "item_ancient_head",
    "item_legacy_head",
    "item_adrenaline",
    "item_bandolier",
    "item_pack",
    "item_ir_goggles",
    "item_double",
    "item_sphere_vengeance",
    "item_sphere_hunter",
    "item_sphere_defender",
    "item_doppleganger",
    "key_data_cd",
    "key_power_cube",
    "key_explosive_charges",
    "key_yellow_key",
    "key_power_core",
    "key_pyramid",
    "key_data_spinner",
    "key_pass",
    "key_blue_key",
    "key_red_key",
    "key_green_key",
    "key_commander_head",
    "key_airstrike_target",
    "key_nuke_container",
    "key_nuke",
    "item_health_small",
    "item_health",
    "item_health_large",
    "item_health_mega",
    "item_flag_team1",
    "item_flag_team2",
    "item_tech1",
    "item_tech2",
    "item_tech3",
    "item_tech4",
    "item_flashlight",
    "item_compass",
];

/// Ammunition capacity slot per classname.
#[must_use]
pub fn ammo_slot(name: &str) -> Option<usize> {
    Some(match name {
        "ammo_bullets" => 0,
        "ammo_shells" => 1,
        "ammo_rockets" => 2,
        "ammo_grenades" => 3,
        "ammo_cells" => 4,
        "ammo_slugs" => 5,
        "ammo_magslug" => 6,
        "ammo_trap" => 7,
        "ammo_flechettes" => 8,
        "ammo_tesla" => 9,
        "ammo_disruptor" => 10,
        "ammo_prox" => 11,
        _ => return None,
    })
}

fn retail_client_layout() -> SourceLayout {
    let shared = source_layout(&client_layout());
    let mut fields: Vec<SourceField> = shared
        .fields
        .iter()
        .map(|field| SourceField {
            name: format!("shared.{}", field.name),
            byte_offset: field.byte_offset,
            storage: field.storage,
            count: field.count,
        })
        .collect();
    let sparse = [
        ("resp.score", 0x17c8usize, GuestStorage::Int32, 1usize),
        ("resp.ctf_team", 0x17dc, GuestStorage::Int32, 1),
        ("pers.selected_item", 2672, GuestStorage::Int32, 1),
        ("pers.inventory", 2688, GuestStorage::Int32, 84),
        ("pers.max_ammo", 3024, GuestStorage::Int16, 12),
        ("pers.weapon", 3048, GuestStorage::Pointer, 1),
        ("newweapon", 6296, GuestStorage::Pointer, 1),
        ("v_angle", 6552, GuestStorage::Float32, 3),
        ("invincible_time", 6672, GuestStorage::Int64, 1),
        ("no_weapon_chains", 7016, GuestStorage::Uint8, 1),
    ];
    for (name, byte_offset, storage, count) in sparse {
        fields.push(SourceField {
            name: name.to_string(),
            byte_offset,
            storage,
            count,
        });
    }
    SourceLayout {
        id: "q2-rerelease-retail:observed-client-fields".to_string(),
        byte_length: 7344,
        alignment: 8,
        pointer_bytes: 8,
        fields,
    }
}

fn retail_inventory() -> Vec<InventoryRow> {
    let mut rows = vec![InventoryRow {
        item: "q2:none".to_string(),
        source: InventorySource::Index { index: 0 },
        capacity: InventoryCapacity::Fixed { count: 0 },
    }];
    for name in CLASSNAMES {
        rows.push(InventoryRow {
            item: format!("q2:{name}"),
            source: InventorySource::Classname {
                name: (*name).to_string(),
            },
            capacity: match ammo_slot(name) {
                Some(source_index) => InventoryCapacity::Ammo { source_index },
                None => InventoryCapacity::Fixed { count: 0x7fff_ffff },
            },
        });
    }
    rows.push(InventoryRow {
        item: "q2:item_tag_token".to_string(),
        source: InventorySource::Remaining,
        capacity: InventoryCapacity::Fixed { count: 0x7fff_ffff },
    });
    rows
}

/// Retail primary world profile.
#[must_use]
pub fn retail_world_profile() -> RereleasePrimaryWorldProfile {
    let mut edict = source_layout(&private_edict_prefix_layout());
    edict.byte_length = 3688;
    RereleasePrimaryWorldProfile {
        calls: WorldCalls {
            pain: stock_combat_call(CombatOperation::Pain),
            death: stock_combat_call(CombatOperation::Death),
            process_pain: stock_combat_call(CombatOperation::DeferredReaction),
            damage: stock_combat_call(CombatOperation::Damage),
            power_armor: stock_combat_call(CombatOperation::PowerArmor),
        },
        digest: RETAIL_DIGEST.to_string(),
        edict,
        client: WorldClientProfile {
            digest: RETAIL_DIGEST.to_string(),
            layout: retail_client_layout(),
            inventory_count: 84,
            ammo_count: 12,
        },
        entries: WorldEntries {
            spawn: 0x964b0,
            free: 0x96600,
            damage: 0x5cae0,
            power_armor: 0x5c100,
            process_pain: 0x76e20,
            time: 0x241b28,
            regular_armor_entry: 0x5d022,
            regular_armor_join: 0x5d154,
        },
        regular_armor: RegularArmorRegion {
            target: WorldLocation::Register {
                register: WorldRegister::Rdi,
                storage: LocationStorage::Pointer,
            },
            amount: WorldLocation::Register {
                register: WorldRegister::R14,
                storage: LocationStorage::Int32,
            },
            point: WorldLocation::Register {
                register: WorldRegister::R13,
                storage: LocationStorage::Pointer,
            },
            normal: WorldLocation::Stack {
                offset: 0x108,
                storage: LocationStorage::Pointer,
            },
            flags: WorldLocation::Stack {
                offset: 0x120,
                storage: LocationStorage::Int32,
            },
            result: WorldLocation::Register {
                register: WorldRegister::R12,
                storage: LocationStorage::Int32,
            },
            repair: vec![ArmorRepair {
                source: WorldLocation::Stack {
                    offset: 0xe0,
                    storage: LocationStorage::Uint32,
                },
                target: WorldLocation::Register {
                    register: WorldRegister::Rbx,
                    storage: LocationStorage::Uint32,
                },
            }],
        },
        monster: MonsterAccumulator {
            attacker: 3120,
            inflictor: 3128,
            blood: 3136,
            knockback: 3140,
            point: 3144,
            modem: 3156,
            invincible_time: 0xb88,
        },
        inventory: retail_inventory(),
        armor: ArmorMetadata {
            table: 0x1953a8,
            stride: 192,
            normal: 8,
            energy: 12,
            regular: vec![
                "q2:item_armor_jacket".to_string(),
                "q2:item_armor_combat".to_string(),
                "q2:item_armor_body".to_string(),
            ],
            empty: "q2:item_armor_body".to_string(),
            screen: "q2:item_power_screen".to_string(),
            shield: "q2:item_power_shield".to_string(),
            cells: "q2:ammo_cells".to_string(),
            cells_index: 30,
        },
        flags: FlagBits {
            godmode: 16,
            notarget: 32,
            no_knockback: 2048,
            power_armor: 4096,
        },
        movement: MovementMetadata {
            body: Some(MovementBody {
                dimensions: 0xe9ff0,
                trace: 0xe71a0,
                movement_global: 0x23c9c8,
            }),
            game_api: 0x6bcd0,
            pmove: 0xea560,
            speed_loads: vec![
                SpeedLoad {
                    next: 0xe8293,
                    register: 0,
                },
                SpeedLoad {
                    next: 0xe889c,
                    register: 1,
                },
                SpeedLoad {
                    next: 0xe88ce,
                    register: 0,
                },
                SpeedLoad {
                    next: 0xe8b15,
                    register: 1,
                },
                SpeedLoad {
                    next: 0xe8b1f,
                    register: 1,
                },
                SpeedLoad {
                    next: 0xe9e3e,
                    register: 10,
                },
            ],
        },
    }
}

/// Select the primary world profile for a module digest, if admitted.
#[must_use]
pub fn world_profile_for_digest(digest: &str) -> Option<RereleasePrimaryWorldProfile> {
    if digest == RETAIL_DIGEST {
        Some(retail_world_profile())
    } else {
        None
    }
}

/// Validate a primary world profile against a module digest.
pub fn validate_world_profile(profile: &RereleasePrimaryWorldProfile, digest: &str) -> Result<(), WorldProfileError> {
    let invalid = |message: &str| WorldProfileError::Invalid(message.to_string());
    validate_combat_call(&profile.calls.pain, CombatOperation::Pain)?;
    validate_combat_call(&profile.calls.death, CombatOperation::Death)?;
    validate_combat_call(&profile.calls.process_pain, CombatOperation::DeferredReaction)?;
    validate_combat_call(&profile.calls.damage, CombatOperation::Damage)?;
    validate_combat_call(&profile.calls.power_armor, CombatOperation::PowerArmor)?;
    if profile.digest != digest || profile.client.digest != digest {
        return Err(invalid("Rerelease source world profile belongs to another artifact"));
    }
    let range = |value: u64, min: u64, max: u64| {
        if value < min || value > max {
            return Err(invalid("Source world metadata is outside its declared range"));
        }
        Ok(())
    };
    for value in [
        profile.entries.spawn,
        profile.entries.free,
        profile.entries.damage,
        profile.entries.power_armor,
        profile.entries.process_pain,
        profile.entries.time,
        profile.entries.regular_armor_entry,
        profile.entries.regular_armor_join,
        profile.armor.table,
        profile.movement.game_api,
        profile.movement.pmove,
    ] {
        range(value, 1, 0xffff_ffff)?;
    }
    range(profile.client.inventory_count as u64, 1, 65536)?;
    range(profile.client.ammo_count as u64, 1, 65536)?;
    range(profile.armor.stride as u64, 8, 65536)?;
    range(profile.armor.normal as u64, 0, 65532)?;
    range(profile.armor.energy as u64, 0, 65532)?;
    range(
        profile.armor.cells_index as u64,
        0,
        profile.client.inventory_count as u64 - 1,
    )?;
    for value in [
        profile.flags.godmode,
        profile.flags.notarget,
        profile.flags.no_knockback,
        profile.flags.power_armor,
    ] {
        range(value, 1, u64::MAX)?;
    }
    if let Some(body) = &profile.movement.body {
        for address in [body.dimensions, body.trace, body.movement_global] {
            range(address, 1, 0xffff_ffff)?;
        }
    }
    let mut loads = std::collections::HashSet::new();
    for load in &profile.movement.speed_loads {
        range(load.next, 1, 0xffff_ffff)?;
        range(load.register as u64, 0, 15)?;
        if !loads.insert(load.next) {
            return Err(invalid("Repeated source movement load would apply equipment twice"));
        }
    }
    let armor_locations = [
        profile.regular_armor.target,
        profile.regular_armor.amount,
        profile.regular_armor.point,
        profile.regular_armor.normal,
        profile.regular_armor.flags,
        profile.regular_armor.result,
    ];
    for location in armor_locations.into_iter().chain(
        profile
            .regular_armor
            .repair
            .iter()
            .flat_map(|repair| [repair.source, repair.target]),
    ) {
        if let WorldLocation::Stack { offset, storage } = location {
            range(offset as u64, 0, 1_048_576 - storage.bytes() as u64)?;
        }
    }
    validate_layout_prefix(&profile.edict, &source_layout(&edict_layout()))?;
    validate_layout_prefix(&profile.client.layout, &source_layout(&client_layout()))?;
    let prefix = source_layout(&private_edict_prefix_layout());
    for expected in prefix.fields.iter().filter(|field| !field.name.starts_with("shared.")) {
        let found = profile.edict.fields.iter().find(|field| field.name == expected.name);
        match found {
            Some(found) if found.storage == expected.storage && found.count == expected.count => {}
            _ => {
                return Err(invalid(&format!("Missing typed source edict field {}", expected.name)));
            }
        }
    }
    let require_client = |name: &str, storage: GuestStorage, count: usize| {
        let found = profile.client.layout.fields.iter().find(|field| field.name == name);
        match found {
            Some(found) if found.storage == storage && found.count == count => Ok(()),
            _ => Err(invalid(&format!("Missing typed source client field {name}"))),
        }
    };
    require_client("pers.inventory", GuestStorage::Int32, profile.client.inventory_count)?;
    require_client("pers.max_ammo", GuestStorage::Int16, profile.client.ammo_count)?;
    require_client("v_angle", GuestStorage::Float32, 3)?;
    require_client("invincible_time", GuestStorage::Int64, 1)?;
    for (offset, bytes) in [
        (profile.monster.attacker, 8),
        (profile.monster.inflictor, 8),
        (profile.monster.blood, 4),
        (profile.monster.knockback, 4),
        (profile.monster.point, 12),
        (profile.monster.modem, 3),
        (profile.monster.invincible_time, 8),
    ] {
        if offset + bytes > profile.edict.byte_length {
            return Err(invalid("Source monster accumulator exceeds its edict"));
        }
    }
    for location in [
        profile.regular_armor.target,
        profile.regular_armor.point,
        profile.regular_armor.normal,
    ] {
        if location.storage() != LocationStorage::Pointer {
            return Err(invalid("Source armor geometry requires pointer locations"));
        }
    }
    for location in [
        profile.regular_armor.amount,
        profile.regular_armor.flags,
        profile.regular_armor.result,
    ] {
        if location.storage() != LocationStorage::Int32 {
            return Err(invalid("Source armor values require int32 locations"));
        }
    }
    for repair in &profile.regular_armor.repair {
        if repair.source.storage() != repair.target.storage() {
            return Err(invalid("Source armor repair changes scalar representation"));
        }
    }
    if profile.entries.regular_armor_entry == profile.entries.regular_armor_join {
        return Err(invalid("Source armor region is empty"));
    }
    let items: std::collections::HashSet<&str> = profile.inventory.iter().map(|row| row.item.as_str()).collect();
    if items.len() != profile.inventory.len()
        || profile.inventory.len() != profile.client.inventory_count
        || profile
            .inventory
            .iter()
            .filter(|row| row.source == InventorySource::Remaining)
            .count()
            > 1
    {
        return Err(invalid("Source inventory requires one complete unique roster"));
    }
    for row in &profile.inventory {
        match &row.source {
            InventorySource::Index { index } => {
                range(*index as u64, 0, profile.client.inventory_count as u64 - 1)?;
            }
            InventorySource::Classname { name } => {
                if name.is_empty() || name.contains('\0') {
                    return Err(invalid("Source inventory mapping exceeds its private storage"));
                }
            }
            InventorySource::Remaining => {}
        }
        match row.capacity {
            InventoryCapacity::Ammo { source_index } => {
                range(source_index as u64, 0, profile.client.ammo_count as u64 - 1)?;
            }
            InventoryCapacity::Fixed { count } => {
                range(count as u64, 0, 0x7fff_ffff)?;
            }
        }
    }
    let regular: std::collections::HashSet<&str> = profile.armor.regular.iter().map(String::as_str).collect();
    if regular.len() != profile.armor.regular.len()
        || !profile.armor.regular.contains(&profile.armor.empty)
        || profile.armor.cells_index >= profile.client.inventory_count
        || [
            profile.armor.screen.as_str(),
            profile.armor.shield.as_str(),
            profile.armor.cells.as_str(),
        ]
        .into_iter()
        .chain(regular.iter().copied())
        .any(|item| !items.contains(item))
    {
        return Err(invalid("Source armor metadata has no declared inventory item"));
    }
    if profile.armor.normal + 4 > 65536 || profile.armor.energy + 4 > 65536 || profile.movement.speed_loads.is_empty() {
        return Err(invalid("Invalid source armor or movement metadata"));
    }
    Ok(())
}

fn validate_layout_prefix(layout: &SourceLayout, public: &SourceLayout) -> Result<(), WorldProfileError> {
    let invalid = |message: &str| WorldProfileError::Invalid(message.to_string());
    if layout.pointer_bytes != 8 {
        return Err(invalid("Source world layout requires 8-byte pointers"));
    }
    let mut names = std::collections::HashSet::new();
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for field in &layout.fields {
        let end = field.byte_offset + storage_bytes(field.storage) * field.count;
        if field.count == 0
            || !names.insert(field.name.as_str())
            || ranges
                .iter()
                .any(|(start, stop)| field.byte_offset < *stop && end > *start)
        {
            return Err(invalid("Overlapping or empty native source fields"));
        }
        ranges.push((field.byte_offset, end));
    }
    for field in &public.fields {
        let name = format!("shared.{}", field.name);
        let found = layout.fields.iter().find(|value| value.name == name);
        match found {
            Some(found)
                if found.byte_offset == field.byte_offset
                    && found.storage == field.storage
                    && found.count == field.count => {}
            _ => {
                return Err(invalid("Source world layout changes the public API2023 prefix"));
            }
        }
    }
    Ok(())
}

/// Parsed profile value.
#[derive(Debug, Clone, PartialEq)]
pub enum ProfileValue {
    /// Null.
    Null,
    /// Boolean.
    Bool(bool),
    /// Integer.
    Int(i64),
    /// String.
    Str(String),
    /// List.
    List(Vec<ProfileValue>),
    /// Record.
    Map(HashMap<String, ProfileValue>),
}

/// Reader over a parsed profile value with path context.
#[derive(Debug, Clone)]
pub struct ProfileReader<'a> {
    value: &'a ProfileValue,
    path: String,
}

impl<'a> ProfileReader<'a> {
    /// Create a reader.
    #[must_use]
    pub fn new(value: &'a ProfileValue, path: &str) -> Self {
        Self {
            value,
            path: path.to_string(),
        }
    }

    fn fail(&self, message: &str) -> WorldProfileError {
        WorldProfileError::Invalid(format!("{}: {message}", self.path))
    }

    /// Read a record field.
    pub fn field(&self, name: &str) -> Result<ProfileReader<'a>, WorldProfileError> {
        match self.value {
            ProfileValue::Map(map) => map.get(name).map_or_else(
                || Err(self.fail(&format!("missing field {name}"))),
                |value| {
                    Ok(ProfileReader {
                        value,
                        path: format!("{}.{name}", self.path),
                    })
                },
            ),
            _ => Err(self.fail("expected a record")),
        }
    }

    /// Optional field value.
    #[must_use]
    pub fn has(&self, name: &str) -> bool {
        matches!(self.value, ProfileValue::Map(map) if map.contains_key(name))
    }

    /// Read a string.
    pub fn string(&self) -> Result<String, WorldProfileError> {
        match self.value {
            ProfileValue::Str(value) => Ok(value.clone()),
            _ => Err(self.fail("expected a string")),
        }
    }

    /// Read an integer with a minimum.
    pub fn integer(&self, minimum: i64) -> Result<i64, WorldProfileError> {
        match self.value {
            ProfileValue::Int(value) if *value >= minimum => Ok(*value),
            _ => Err(self.fail("expected an integer in range")),
        }
    }

    /// Read a bounded integer.
    pub fn bounded(&self, min: i64, max: i64) -> Result<i64, WorldProfileError> {
        let value = self.integer(min)?;
        if value > max {
            return Err(self.fail(&format!("Expected integer at most {max}")));
        }
        Ok(value)
    }

    /// Read a list.
    pub fn list<T>(
        &self,
        read: impl Fn(&ProfileReader<'a>) -> Result<T, WorldProfileError>,
    ) -> Result<Vec<T>, WorldProfileError> {
        match self.value {
            ProfileValue::List(values) => values
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    read(&ProfileReader {
                        value,
                        path: format!("{}[{index}]", self.path),
                    })
                })
                .collect(),
            _ => Err(self.fail("expected an array")),
        }
    }

    /// Read a nullable value.
    pub fn nullable<T>(
        &self,
        read: impl Fn(&ProfileReader<'a>) -> Result<T, WorldProfileError>,
    ) -> Result<Option<T>, WorldProfileError> {
        match self.value {
            ProfileValue::Null => Ok(None),
            _ => read(self).map(Some),
        }
    }

    /// Read a choice of strings.
    pub fn choice(&self, choices: &[&str]) -> Result<String, WorldProfileError> {
        let value = self.string()?;
        if choices.contains(&value.as_str()) {
            Ok(value)
        } else {
            Err(self.fail(&format!("expected {}", choices.join(" or "))))
        }
    }

    /// Read a namespaced identity.
    pub fn namespaced(&self) -> Result<String, WorldProfileError> {
        let value = self.string()?;
        match value.find(':') {
            Some(colon) if colon > 0 && colon < value.len() - 1 => Ok(value),
            _ => Err(self.fail("expected a namespaced identity")),
        }
    }
}

fn read_storage(reader: &ProfileReader) -> Result<GuestStorage, WorldProfileError> {
    match reader
        .choice(&[
            "int8", "uint8", "int16", "uint16", "int32", "uint32", "int64", "uint64", "float32", "float64", "pointer",
        ])?
        .as_str()
    {
        "int8" => Ok(GuestStorage::Int8),
        "uint8" => Ok(GuestStorage::Uint8),
        "int16" => Ok(GuestStorage::Int16),
        "uint16" => Ok(GuestStorage::Uint16),
        "int32" => Ok(GuestStorage::Int32),
        "uint32" => Ok(GuestStorage::Uint32),
        "int64" => Ok(GuestStorage::Int64),
        "uint64" => Ok(GuestStorage::Uint64),
        "float32" => Ok(GuestStorage::Float32),
        "float64" => Ok(GuestStorage::Float64),
        _ => Ok(GuestStorage::Pointer),
    }
}

fn read_source_layout(reader: &ProfileReader) -> Result<SourceLayout, WorldProfileError> {
    let pointer_bytes = reader.field("pointerBytes")?.bounded(4, 8)?;
    if pointer_bytes != 4 && pointer_bytes != 8 {
        return Err(WorldProfileError::Invalid(format!("{}: expected 4 or 8", reader.path)));
    }
    Ok(SourceLayout {
        id: reader.field("id")?.namespaced()?,
        byte_length: reader.field("byteLength")?.integer(0)? as usize,
        alignment: reader.field("alignment")?.integer(1)? as usize,
        pointer_bytes: pointer_bytes as usize,
        fields: reader.field("fields")?.list(|field| {
            Ok(SourceField {
                name: field.field("name")?.string()?,
                byte_offset: field.field("byteOffset")?.integer(0)? as usize,
                storage: read_storage(&field.field("storage")?)?,
                count: field.field("count")?.integer(0)? as usize,
            })
        })?,
    })
}

fn read_location(reader: &ProfileReader) -> Result<WorldLocation, WorldProfileError> {
    let kind = reader.field("kind")?.choice(&["register", "stack"])?;
    let storage = LocationStorage::parse(&reader.field("storage")?.choice(&["pointer", "int32", "uint32"])?)
        .ok_or_else(|| WorldProfileError::Invalid(format!("{}: bad storage", reader.path)))?;
    if kind == "register" {
        let register = WorldRegister::parse(&reader.field("register")?.choice(&[
            "rax", "rcx", "rdx", "rbx", "rbp", "rsi", "rdi", "r8", "r9", "r10", "r11", "r12", "r13", "r14", "r15",
        ])?)
        .ok_or_else(|| WorldProfileError::Invalid(format!("{}: bad register", reader.path)))?;
        Ok(WorldLocation::Register { register, storage })
    } else {
        Ok(WorldLocation::Stack {
            offset: reader.field("offset")?.bounded(0, 1_048_576 - storage.bytes() as i64)? as usize,
            storage,
        })
    }
}

fn read_combat_call(reader: &ProfileReader, operation: CombatOperation) -> Result<NativeCombatCall, WorldProfileError> {
    let convention = CombatConvention::parse(&reader.field("convention")?.choice(&[
        "cdecl",
        "stdcall",
        "fastcall",
        "thiscall",
        "microsoft-x64",
    ])?)
    .ok_or_else(|| WorldProfileError::Invalid(format!("{}: bad convention", reader.path)))?;
    let arguments = reader.field("arguments")?.list(|value| {
        let kind = value.field("kind")?.choice(&["field", "value", "address"])?;
        if kind == "field" {
            let field = CombatField::parse(&value.field("field")?.choice(&[
                "target",
                "inflictor",
                "attacker",
                "direction",
                "point",
                "normal",
                "amount",
                "knockback",
                "flags",
                "cause",
                "sparks",
                "kick",
            ])?)
            .ok_or_else(|| WorldProfileError::Invalid(format!("{}: bad field", value.path)))?;
            return Ok(CombatArgument::Field(field));
        }
        if kind == "address" {
            let address = value.field("address")?.nullable(|at| {
                Ok(ImageAddress {
                    rva: at.field("rva")?.bounded(0, 0xffff_ffff)? as u64,
                    indirections: at
                        .field("indirections")?
                        .list(|offset| Ok(offset.bounded(0, 0xffff_ffff)? as u64))?,
                })
            })?;
            return Ok(CombatArgument::Address { address });
        }
        let layout_reader = value.field("layout")?;
        let layout = if layout_reader.field("kind")?.choice(&["scalar", "aggregate"])? == "scalar" {
            CombatValueLayout::Scalar(read_storage(&layout_reader.field("storage")?)?)
        } else {
            CombatValueLayout::Aggregate(read_source_layout(&layout_reader.field("layout")?)?)
        };
        Ok(CombatArgument::Value {
            layout,
            bytes: value.field("bytes")?.list(|byte| Ok(byte.bounded(0, 255)? as u8))?,
        })
    })?;
    let call = NativeCombatCall { convention, arguments };
    validate_combat_call(&call, operation)?;
    Ok(call)
}

/// Read and validate a primary world profile from a parsed value.
pub fn read_world_profile(
    reader: &ProfileReader,
    digest: &str,
) -> Result<RereleasePrimaryWorldProfile, WorldProfileError> {
    let client = reader.field("client")?;
    let entries = reader.field("entries")?;
    let regular = reader.field("regularArmor")?;
    let monster = reader.field("monster")?;
    let armor = reader.field("armor")?;
    let flags = reader.field("flags")?;
    let movement = reader.field("movement")?;
    let calls = reader.field("calls")?;
    let rva = |reader: ProfileReader| reader.bounded(1, 0xffff_ffff).map(|value| value as u64);
    let profile = RereleasePrimaryWorldProfile {
        calls: WorldCalls {
            pain: read_combat_call(&calls.field("pain")?, CombatOperation::Pain)?,
            death: read_combat_call(&calls.field("death")?, CombatOperation::Death)?,
            process_pain: read_combat_call(&calls.field("processPain")?, CombatOperation::DeferredReaction)?,
            damage: read_combat_call(&calls.field("damage")?, CombatOperation::Damage)?,
            power_armor: read_combat_call(&calls.field("powerArmor")?, CombatOperation::PowerArmor)?,
        },
        digest: digest.to_string(),
        edict: read_source_layout(&reader.field("edict")?)?,
        client: WorldClientProfile {
            digest: digest.to_string(),
            layout: read_source_layout(&client.field("layout")?)?,
            inventory_count: client.field("inventoryCount")?.bounded(1, 65536)? as usize,
            ammo_count: client.field("ammoCount")?.bounded(1, 65536)? as usize,
        },
        entries: WorldEntries {
            spawn: rva(entries.field("spawn")?)?,
            free: rva(entries.field("free")?)?,
            damage: rva(entries.field("damage")?)?,
            power_armor: rva(entries.field("powerArmor")?)?,
            process_pain: rva(entries.field("processPain")?)?,
            time: rva(entries.field("time")?)?,
            regular_armor_entry: rva(entries.field("regularArmor")?.field("entry")?)?,
            regular_armor_join: rva(entries.field("regularArmor")?.field("join")?)?,
        },
        regular_armor: RegularArmorRegion {
            target: read_location(&regular.field("target")?)?,
            amount: read_location(&regular.field("amount")?)?,
            point: read_location(&regular.field("point")?)?,
            normal: read_location(&regular.field("normal")?)?,
            flags: read_location(&regular.field("flags")?)?,
            result: read_location(&regular.field("result")?)?,
            repair: regular.field("repair")?.list(|value| {
                Ok(ArmorRepair {
                    source: read_location(&value.field("source")?)?,
                    target: read_location(&value.field("target")?)?,
                })
            })?,
        },
        monster: MonsterAccumulator {
            attacker: monster.field("attacker")?.integer(0)? as usize,
            inflictor: monster.field("inflictor")?.integer(0)? as usize,
            blood: monster.field("blood")?.integer(0)? as usize,
            knockback: monster.field("knockback")?.integer(0)? as usize,
            point: monster.field("point")?.integer(0)? as usize,
            modem: monster.field("mod")?.integer(0)? as usize,
            invincible_time: monster.field("invincibleTime")?.integer(0)? as usize,
        },
        inventory: reader.field("inventory")?.list(|value| {
            let source = value.field("source")?;
            let kind = source.field("kind")?.choice(&["classname", "index", "remaining"])?;
            let capacity = value.field("capacity")?;
            Ok(InventoryRow {
                item: value.field("item")?.namespaced()?,
                source: if kind == "classname" {
                    InventorySource::Classname {
                        name: source.field("name")?.string()?,
                    }
                } else if kind == "index" {
                    InventorySource::Index {
                        index: source.field("index")?.integer(0)? as usize,
                    }
                } else {
                    InventorySource::Remaining
                },
                capacity: if capacity.field("kind")?.choice(&["ammo", "fixed"])? == "ammo" {
                    InventoryCapacity::Ammo {
                        source_index: capacity.field("sourceIndex")?.integer(0)? as usize,
                    }
                } else {
                    InventoryCapacity::Fixed {
                        count: capacity.field("count")?.bounded(0, 0x7fff_ffff)? as i32,
                    }
                },
            })
        })?,
        armor: ArmorMetadata {
            table: rva(armor.field("table")?)?,
            stride: armor.field("stride")?.bounded(8, 65536)? as usize,
            normal: armor.field("normal")?.integer(0)? as usize,
            energy: armor.field("energy")?.integer(0)? as usize,
            regular: armor.field("regular")?.list(|value| value.namespaced())?,
            empty: armor.field("empty")?.namespaced()?,
            screen: armor.field("screen")?.namespaced()?,
            shield: armor.field("shield")?.namespaced()?,
            cells: armor.field("cells")?.namespaced()?,
            cells_index: armor.field("cellsIndex")?.integer(0)? as usize,
        },
        flags: FlagBits {
            godmode: flags.field("godmode")?.integer(1)? as u64,
            notarget: flags.field("notarget")?.integer(1)? as u64,
            no_knockback: flags.field("noKnockback")?.integer(1)? as u64,
            power_armor: flags.field("powerArmor")?.integer(1)? as u64,
        },
        movement: MovementMetadata {
            body: if movement.has("body") {
                let body = movement.field("body")?;
                Some(MovementBody {
                    dimensions: rva(body.field("dimensions")?)?,
                    trace: rva(body.field("trace")?)?,
                    movement_global: rva(body.field("movementGlobal")?)?,
                })
            } else {
                None
            },
            game_api: rva(movement.field("gameApi")?)?,
            pmove: rva(movement.field("pmove")?)?,
            speed_loads: movement.field("speedLoads")?.list(|value| {
                Ok(SpeedLoad {
                    next: rva(value.field("next")?)?,
                    register: value.field("register")?.bounded(0, 15)? as usize,
                })
            })?,
        },
    };
    validate_world_profile(&profile, digest)?;
    Ok(profile)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retail_profile_validates_and_gates_digest() {
        let profile = retail_world_profile();
        assert_eq!(profile.client.inventory_count, 84);
        assert_eq!(profile.inventory.len(), 84);
        validate_world_profile(&profile, RETAIL_DIGEST).expect("retail");
        assert!(world_profile_for_digest(RETAIL_DIGEST).is_some());
        assert!(world_profile_for_digest("sha256:other").is_none());
        assert!(validate_world_profile(&profile, "sha256:other").is_err());
        let mut tampered = profile.clone();
        tampered.entries.spawn = 0;
        assert!(validate_world_profile(&tampered, RETAIL_DIGEST).is_err());
        let mut tampered = profile.clone();
        tampered.movement.speed_loads.clear();
        assert!(validate_world_profile(&tampered, RETAIL_DIGEST).is_err());
        let mut tampered = profile;
        tampered.armor.regular.push("q2:unknown_armor".to_string());
        assert!(validate_world_profile(&tampered, RETAIL_DIGEST).is_err());
    }

    #[test]
    fn combat_calls_validate_fields_and_layouts() {
        let damage = stock_combat_call(CombatOperation::Damage);
        validate_combat_call(&damage, CombatOperation::Damage).expect("damage");
        let mut short = damage.clone();
        short.arguments.pop();
        assert!(validate_combat_call(&short, CombatOperation::Damage).is_err());
        let mut dup = damage.clone();
        dup.arguments[0] = CombatArgument::Field(CombatField::Target);
        dup.arguments[1] = CombatArgument::Field(CombatField::Target);
        assert!(validate_combat_call(&dup, CombatOperation::Damage).is_err());
        let edict = source_layout(&private_edict_prefix_layout());
        assert!(validate_layout_prefix(&edict, &source_layout(&edict_layout())).is_ok());
        let mut broken = edict.clone();
        broken.fields.retain(|field| field.name != "shared.inuse");
        assert!(validate_layout_prefix(&broken, &source_layout(&edict_layout())).is_err());
        let reader = ProfileReader::new(&ProfileValue::Int(5), "root");
        assert_eq!(reader.bounded(1, 10).expect("bounded"), 5);
        assert!(reader.bounded(6, 10).is_err());
        assert_eq!(WorldRegister::parse("r13"), Some(WorldRegister::R13));
        assert_eq!(WorldRegister::parse("rax2"), None);
    }
}
