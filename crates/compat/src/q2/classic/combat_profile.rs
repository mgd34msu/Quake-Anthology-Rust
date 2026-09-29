//! Donor: `src/compat/q2/classic/combat-profile.ts` — stock combat call
//! declarations and the verified original-artifact profile.
//!
//! Bridges native damage/armor/pain/death callsites to the shared combat
//! rules: each `ClassicNativeCombatCall` declares which semantic fields a
//! call carries, and `ClassicCombatProfile` pins every field offset, entry
//! point, and inventory index the bindings touch.

use qa_guest::abi::values::{decode_value, validate_value_layout, value_bytes};
use qa_guest::core::contracts::{
    ContentDigest, GuestAddress, GuestCallSignature, GuestCallValue, GuestStorage, GuestValueLayout,
    NativeCallAbi,
};
use qa_guest::core::memory::SparseGuestMemory;

use super::layout::{classic_signature, q2_int, q2_pointer, ClassicQ2Error, ClassicResult};

/// Semantic combat field carried by one native argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClassicNativeCombatField {
    /// Damage target.
    Target,
    /// Inflicting entity.
    Inflictor,
    /// Attacking entity.
    Attacker,
    /// Damage direction vector.
    Direction,
    /// Impact point vector.
    Point,
    /// Impact normal vector.
    Normal,
    /// Damage amount.
    Amount,
    /// Knockback.
    Knockback,
    /// Damage flags.
    Flags,
    /// Means-of-death cause.
    Cause,
    /// Armor spark count.
    Sparks,
    /// Pain kick.
    Kick,
}

impl ClassicNativeCombatField {
    /// Save-file field name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Target => "target",
            Self::Inflictor => "inflictor",
            Self::Attacker => "attacker",
            Self::Direction => "direction",
            Self::Point => "point",
            Self::Normal => "normal",
            Self::Amount => "amount",
            Self::Knockback => "knockback",
            Self::Flags => "flags",
            Self::Cause => "cause",
            Self::Sparks => "sparks",
            Self::Kick => "kick",
        }
    }

    /// Parse a save-file field name.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "target" => Some(Self::Target),
            "inflictor" => Some(Self::Inflictor),
            "attacker" => Some(Self::Attacker),
            "direction" => Some(Self::Direction),
            "point" => Some(Self::Point),
            "normal" => Some(Self::Normal),
            "amount" => Some(Self::Amount),
            "knockback" => Some(Self::Knockback),
            "flags" => Some(Self::Flags),
            "cause" => Some(Self::Cause),
            "sparks" => Some(Self::Sparks),
            "kick" => Some(Self::Kick),
            _ => None,
        }
    }
}

/// Image-relative default address with pointer indirections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CombatAddressDefault {
    /// Image-relative offset.
    pub rva: u32,
    /// Pointer indirections applied in order.
    pub indirections: Vec<u32>,
}

/// One declared native combat argument.
#[derive(Debug, Clone, PartialEq)]
pub enum ClassicNativeCombatArgument {
    /// Semantic field projected from the invocation.
    Field(ClassicNativeCombatField),
    /// Fixed default value with exact layout bytes.
    Value {
        /// Value layout.
        layout: GuestValueLayout,
        /// Little-endian default bytes.
        bytes: Vec<u8>,
    },
    /// Image-relative default address (null stays null).
    Address {
        /// Default address, if any.
        target: Option<CombatAddressDefault>,
    },
}

/// Declared native combat call: convention plus arguments.
#[derive(Debug, Clone, PartialEq)]
pub struct ClassicNativeCombatCall {
    /// Calling convention.
    pub convention: NativeCallAbi,
    /// Declared arguments.
    pub arguments: Vec<ClassicNativeCombatArgument>,
}

/// Combat operation selecting the required semantic fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassicCombatOperation {
    /// `T_Damage`.
    Damage,
    /// Regular armor stage.
    RegularArmor,
    /// Power armor stage.
    PowerArmor,
    /// Pain reaction.
    Pain,
    /// Death reaction.
    Death,
}

impl ClassicCombatOperation {
    /// Required semantic fields for 32-bit classic calls.
    #[must_use]
    pub fn fields(self) -> &'static [ClassicNativeCombatField] {
        use ClassicNativeCombatField as Field;
        match self {
            Self::Damage => &[
                Field::Target,
                Field::Inflictor,
                Field::Attacker,
                Field::Direction,
                Field::Point,
                Field::Normal,
                Field::Amount,
                Field::Knockback,
                Field::Flags,
                Field::Cause,
            ],
            Self::RegularArmor => &[
                Field::Target,
                Field::Point,
                Field::Normal,
                Field::Amount,
                Field::Sparks,
                Field::Flags,
            ],
            Self::PowerArmor => &[Field::Target, Field::Point, Field::Normal, Field::Amount, Field::Flags],
            Self::Pain => &[Field::Target, Field::Attacker, Field::Kick, Field::Amount],
            Self::Death => &[Field::Target, Field::Inflictor, Field::Attacker, Field::Amount, Field::Point],
        }
    }

    /// Whether the operation returns the saved-damage integer.
    #[must_use]
    pub fn returns_saved(self) -> bool {
        matches!(self, Self::RegularArmor | Self::PowerArmor)
    }
}

/// Stock cdecl declaration carrying every semantic field in order.
#[must_use]
pub fn stock_native_combat_call(operation: ClassicCombatOperation) -> ClassicNativeCombatCall {
    ClassicNativeCombatCall {
        convention: NativeCallAbi::Cdecl,
        arguments: operation
            .fields()
            .iter()
            .map(|field| ClassicNativeCombatArgument::Field(*field))
            .collect(),
    }
}

fn argument_layout(argument: &ClassicNativeCombatArgument) -> GuestValueLayout {
    match argument {
        ClassicNativeCombatArgument::Address { .. } => q2_pointer(),
        ClassicNativeCombatArgument::Value { layout, .. } => layout.clone(),
        ClassicNativeCombatArgument::Field(field) => match field {
            ClassicNativeCombatField::Target
            | ClassicNativeCombatField::Inflictor
            | ClassicNativeCombatField::Attacker
            | ClassicNativeCombatField::Direction
            | ClassicNativeCombatField::Point
            | ClassicNativeCombatField::Normal => q2_pointer(),
            ClassicNativeCombatField::Kick => GuestValueLayout::Scalar(GuestStorage::Float32),
            ClassicNativeCombatField::Amount
            | ClassicNativeCombatField::Knockback
            | ClassicNativeCombatField::Flags
            | ClassicNativeCombatField::Sparks
            | ClassicNativeCombatField::Cause => q2_int(),
        },
    }
}

/// Call signature for a declared combat call on the classic ABI.
pub fn native_combat_signature(
    call: &ClassicNativeCombatCall,
    operation: ClassicCombatOperation,
) -> ClassicResult<GuestCallSignature> {
    let abi = match call.convention {
        NativeCallAbi::Cdecl | NativeCallAbi::Stdcall | NativeCallAbi::Thiscall | NativeCallAbi::Fastcall => {
            call.convention
        }
        NativeCallAbi::MicrosoftX64 | NativeCallAbi::SystemVI386 | NativeCallAbi::SystemVX86_64 => {
            return Err(ClassicQ2Error::invalid(
                "Native combat convention does not match its source architecture",
            ));
        }
    };
    Ok(GuestCallSignature {
        abi,
        parameters: call.arguments.iter().map(argument_layout).collect(),
        result: operation.returns_saved().then(q2_int),
        variadic: false,
    })
}

/// Validate a declared combat call against its operation.
pub fn validate_native_combat_call(
    call: &ClassicNativeCombatCall,
    operation: ClassicCombatOperation,
) -> ClassicResult<()> {
    native_combat_signature(call, operation)?;
    let required = operation.fields();
    let mut seen = Vec::with_capacity(required.len());
    for argument in &call.arguments {
        match argument {
            ClassicNativeCombatArgument::Field(field) => {
                if !required.contains(field) || seen.contains(field) {
                    return Err(ClassicQ2Error::invalid(
                        "Native combat fields must occur exactly once",
                    ));
                }
                seen.push(*field);
            }
            ClassicNativeCombatArgument::Address { .. } => {}
            ClassicNativeCombatArgument::Value { layout, bytes } => {
                validate_value_layout(layout, 4)?;
                let has_pointer = match layout {
                    GuestValueLayout::Scalar(storage) => *storage == GuestStorage::Pointer,
                    GuestValueLayout::Aggregate(record) => record
                        .fields
                        .iter()
                        .any(|field| field.storage == GuestStorage::Pointer),
                };
                if has_pointer {
                    return Err(ClassicQ2Error::invalid(
                        "Native combat pointer defaults require an image-relative address",
                    ));
                }
                if bytes.len() != value_bytes(layout, 4) {
                    return Err(ClassicQ2Error::invalid(
                        "Native combat default bytes do not match their source argument",
                    ));
                }
            }
        }
    }
    if seen.len() != required.len() {
        return Err(ClassicQ2Error::invalid(
            "Native combat declaration omits a required source field",
        ));
    }
    Ok(())
}

/// Project an invocation to its semantic fields in canonical order.
pub fn read_native_combat_arguments(
    call: &ClassicNativeCombatCall,
    operation: ClassicCombatOperation,
    values: &[GuestCallValue],
) -> ClassicResult<Vec<GuestCallValue>> {
    if values.len() != call.arguments.len() {
        return Err(ClassicQ2Error::invalid(
            "Native combat invocation differs from its declared signature",
        ));
    }
    let mut result = Vec::with_capacity(operation.fields().len());
    for field in operation.fields() {
        let index = call
            .arguments
            .iter()
            .position(|argument| matches!(argument, ClassicNativeCombatArgument::Field(other) if other == field))
            .ok_or_else(|| ClassicQ2Error::invalid("Native combat invocation lacks a declared field"))?;
        result.push(values[index].clone());
    }
    Ok(result)
}

/// Lower semantic values back to a declared call, filling defaults.
pub fn lower_native_combat_arguments(
    call: &ClassicNativeCombatCall,
    operation: ClassicCombatOperation,
    values: &[GuestCallValue],
    memory: &mut SparseGuestMemory,
    image: Option<GuestAddress>,
    original: Option<&[GuestCallValue]>,
) -> ClassicResult<Vec<GuestCallValue>> {
    let semantic = operation.fields();
    if values.len() != semantic.len()
        || original.map_or(false, |captured| captured.len() != call.arguments.len())
    {
        return Err(ClassicQ2Error::invalid(
            "Native combat continuation changed its argument extent",
        ));
    }
    let mut result = Vec::with_capacity(call.arguments.len());
    for (index, argument) in call.arguments.iter().enumerate() {
        match argument {
            ClassicNativeCombatArgument::Field(field) => {
                let position = semantic
                    .iter()
                    .position(|other| other == field)
                    .ok_or_else(|| ClassicQ2Error::invalid("Missing native combat field"))?;
                result.push(values[position].clone());
            }
            _ if original.is_some() => {
                result.push(original.unwrap()[index].clone());
            }
            ClassicNativeCombatArgument::Value { layout, bytes } => {
                result.push(decode_value(layout, bytes, memory)?);
            }
            ClassicNativeCombatArgument::Address { target } => {
                if target.is_some() && image.is_none() {
                    return Err(ClassicQ2Error::invalid(
                        "Native combat address default requires its source image base",
                    ));
                }
                let mut address = match (target, image) {
                    (Some(default), Some(image)) => Some(memory.offset(image, i64::from(default.rva))?),
                    _ => None,
                };
                if let Some(default) = target {
                    for offset in &default.indirections {
                        let Some(current) = address else {
                            return Err(ClassicQ2Error::invalid(
                                "Native combat default address dereferences null",
                            ));
                        };
                        address = memory.read_pointer(memory.offset(current, i64::from(*offset)?)?)?;
                    }
                }
                result.push(GuestCallValue::Pointer(address));
            }
        }
    }
    Ok(result)
}

/// Classic game variant selecting the cause roster.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassicGame {
    /// Base Quake II.
    Base,
    /// The Reckoning.
    Xatrix,
    /// Ground Zero.
    Rogue,
    /// Capture the flag.
    Ctf,
}

impl ClassicGame {
    /// Save-file game name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Base => "base",
            Self::Xatrix => "xatrix",
            Self::Rogue => "rogue",
            Self::Ctf => "ctf",
        }
    }

    /// Parse a save-file game name.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "base" => Some(Self::Base),
            "xatrix" => Some(Self::Xatrix),
            "rogue" => Some(Self::Rogue),
            "ctf" => Some(Self::Ctf),
            _ => None,
        }
    }
}

/// Entity field offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CombatEntityFields {
    /// Health word.
    pub health: usize,
    /// Damageable word.
    pub damageable: usize,
    /// Entity flags word.
    pub flags: usize,
    /// Mass word.
    pub mass: usize,
    /// Velocity vector.
    pub velocity: usize,
    /// Pain callback pointer.
    pub pain: usize,
    /// Death callback pointer.
    pub die: usize,
}

/// Client field offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CombatClientFields {
    /// Inventory array.
    pub inventory: usize,
    /// Inventory slot count.
    pub inventory_count: usize,
    /// Maximum grenades word.
    pub max_grenades: usize,
    /// Invulnerability frame word.
    pub invincible_frame: usize,
    /// Userinfo string.
    pub userinfo: usize,
    /// Userinfo bound.
    pub userinfo_bytes: usize,
    /// View angles vector.
    pub view_angles: usize,
}

/// Combat entry points as image-relative offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CombatEntries {
    /// Damage entry.
    pub damage: u32,
    /// Power armor entry.
    pub power_armor: u32,
    /// Regular armor entry.
    pub regular_armor: u32,
    /// Spawn entry.
    pub spawn: u32,
    /// Free entry.
    pub free: u32,
}

/// Combat globals as image-relative offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CombatGlobals {
    /// Level frame counter.
    pub level_frame: u32,
    /// Item list base.
    pub item_list: u32,
    /// Item record stride.
    pub item_bytes: usize,
}

/// Combat inventory indexes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CombatItems {
    /// Jacket armor index.
    pub jacket: usize,
    /// Combat armor index.
    pub combat: usize,
    /// Body armor index.
    pub body: usize,
    /// Power screen index.
    pub screen: usize,
    /// Power shield index.
    pub shield: usize,
    /// Cells index.
    pub cells: usize,
    /// Grenades index.
    pub grenades: usize,
}

/// Item record field offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CombatItemFields {
    /// Classname pointer.
    pub class_name: usize,
    /// Armor info pointer.
    pub armor_info: usize,
}

/// Armor info record field offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CombatArmorInfo {
    /// Normal protection fraction.
    pub normal_protection: usize,
    /// Energy protection fraction.
    pub energy_protection: usize,
}

/// Regular armor priorities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CombatArmor {
    /// Regular indexes in pickup priority order.
    pub regular: Vec<usize>,
    /// Empty-tier index.
    pub empty: usize,
}

/// Entity flag masks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CombatFlags {
    /// Invulnerability bit.
    pub invulnerable: u32,
    /// Notarget bit.
    pub notarget: u32,
    /// No-knockback bit.
    pub no_knockback: u32,
    /// Power armor active bit.
    pub power_armor: u32,
}

/// Team game masks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CombatTeams {
    /// Same-model teams bit.
    pub model: u32,
    /// Same-skin teams bit.
    pub skin: u32,
}

/// Combat calls per operation.
#[derive(Debug, Clone, PartialEq)]
pub struct CombatCalls {
    /// Pain call.
    pub pain: ClassicNativeCombatCall,
    /// Death call.
    pub death: ClassicNativeCombatCall,
    /// Damage call.
    pub damage: ClassicNativeCombatCall,
    /// Regular armor call.
    pub regular_armor: ClassicNativeCombatCall,
    /// Power armor call.
    pub power_armor: ClassicNativeCombatCall,
}

/// Verified combat profile for one original artifact.
#[derive(Debug, Clone, PartialEq)]
pub struct ClassicCombatProfile {
    /// Combat calls.
    pub calls: CombatCalls,
    /// Original artifact digest.
    pub digest: ContentDigest,
    /// Game variant.
    pub game: ClassicGame,
    /// Source edict stride.
    pub entity_bytes: usize,
    /// Entity fields.
    pub fields: CombatEntityFields,
    /// Client fields.
    pub client: CombatClientFields,
    /// Entries.
    pub entries: CombatEntries,
    /// Globals.
    pub globals: CombatGlobals,
    /// Item indexes.
    pub items: CombatItems,
    /// Item fields.
    pub item_fields: CombatItemFields,
    /// Armor info fields.
    pub armor_info: CombatArmorInfo,
    /// Armor priorities.
    pub armor: CombatArmor,
    /// Flag masks.
    pub flags: CombatFlags,
    /// Team masks.
    pub teams: CombatTeams,
}

/// Original Xatrix DLL digest value.
pub const XATRIX_DIGEST_VALUE: &str = "8187df3fd5b4d435d8227434d3351aad2b47e546236403e52adcd4d275810c45";

/// Original Xatrix DLL profile: `g_local.h` i686 layouts with each used
/// entry and store verified against the PE.
#[must_use]
pub fn xatrix_combat_profile() -> ClassicCombatProfile {
    ClassicCombatProfile {
        calls: CombatCalls {
            pain: stock_native_combat_call(ClassicCombatOperation::Pain),
            death: stock_native_combat_call(ClassicCombatOperation::Death),
            damage: stock_native_combat_call(ClassicCombatOperation::Damage),
            regular_armor: stock_native_combat_call(ClassicCombatOperation::RegularArmor),
            power_armor: stock_native_combat_call(ClassicCombatOperation::PowerArmor),
        },
        digest: ContentDigest::new("sha256", XATRIX_DIGEST_VALUE),
        game: ClassicGame::Xatrix,
        entity_bytes: 896,
        fields: CombatEntityFields {
            health: 480,
            damageable: 512,
            flags: 264,
            mass: 400,
            velocity: 376,
            pain: 452,
            die: 456,
        },
        client: CombatClientFields {
            inventory: 740,
            inventory_count: 256,
            max_grenades: 1776,
            invincible_frame: 3728,
            userinfo: 188,
            userinfo_bytes: 512,
            view_angles: 3652,
        },
        entries: CombatEntries {
            damage: 0x5050,
            power_armor: 0x5580,
            regular_armor: 0x5760,
            spawn: 0x19090,
            free: 0x19140,
        },
        globals: CombatGlobals { level_frame: 0x76800, item_list: 0x4b828, item_bytes: 76 },
        items: CombatItems { jacket: 3, combat: 2, body: 1, screen: 5, shield: 6, cells: 23, grenades: 12 },
        item_fields: CombatItemFields { class_name: 0, armor_info: 64 },
        armor_info: CombatArmorInfo { normal_protection: 8, energy_protection: 12 },
        armor: CombatArmor { regular: vec![3, 2, 1], empty: 1 },
        flags: CombatFlags { invulnerable: 16, notarget: 32, no_knockback: 2048, power_armor: 4096 },
        teams: CombatTeams { model: 64, skin: 128 },
    }
}

fn scalar(value: u64, length: u64) -> ClassicResult<()> {
    if value + length > 0x1_0000_0000 {
        return Err(ClassicQ2Error::invalid(
            "Classic combat source field exceeds its address range",
        ));
    }
    Ok(())
}

/// Validate a combat profile against the source address ranges.
pub fn validate_classic_combat_profile(profile: &ClassicCombatProfile) -> ClassicResult<()> {
    validate_native_combat_call(&profile.calls.pain, ClassicCombatOperation::Pain)?;
    validate_native_combat_call(&profile.calls.death, ClassicCombatOperation::Death)?;
    validate_native_combat_call(&profile.calls.damage, ClassicCombatOperation::Damage)?;
    validate_native_combat_call(&profile.calls.regular_armor, ClassicCombatOperation::RegularArmor)?;
    validate_native_combat_call(&profile.calls.power_armor, ClassicCombatOperation::PowerArmor)?;
    scalar(profile.entity_bytes as u64, 0)?;
    scalar(profile.globals.item_bytes as u64, 0)?;
    if profile.entity_bytes < 260 || profile.globals.item_bytes < 4 || profile.client.inventory_count < 1 {
        return Err(ClassicQ2Error::invalid("Classic combat source record sizes are invalid"));
    }
    let fields = [
        ("health", profile.fields.health, 4),
        ("damageable", profile.fields.damageable, 4),
        ("flags", profile.fields.flags, 4),
        ("mass", profile.fields.mass, 4),
        ("velocity", profile.fields.velocity, 12),
        ("pain", profile.fields.pain, 4),
        ("die", profile.fields.die, 4),
    ];
    for (name, offset, length) in fields {
        scalar(offset as u64, 4)?;
        if offset % 4 != 0 || offset + length > profile.entity_bytes {
            return Err(ClassicQ2Error::invalid(format!(
                "Classic combat field {name} is outside its source edict"
            )));
        }
    }
    for (name, offset) in [
        ("inventory", profile.client.inventory),
        ("maxGrenades", profile.client.max_grenades),
        ("invincibleFrame", profile.client.invincible_frame),
        ("userinfo", profile.client.userinfo),
        ("viewAngles", profile.client.view_angles),
    ] {
        scalar(offset as u64, 4)?;
        if name != "userinfo" && offset % 4 != 0 {
            return Err(ClassicQ2Error::invalid("Classic combat client field is unaligned"));
        }
    }
    if profile.client.userinfo_bytes < 1 {
        return Err(ClassicQ2Error::invalid("Classic combat userinfo requires a bounded source string"));
    }
    scalar(profile.client.userinfo as u64, profile.client.userinfo_bytes as u64)?;
    scalar(profile.client.inventory as u64, profile.client.inventory_count as u64 * 4)?;
    for value in [
        profile.entries.damage,
        profile.entries.power_armor,
        profile.entries.regular_armor,
        profile.entries.spawn,
        profile.entries.free,
        profile.globals.level_frame,
        profile.globals.item_list,
    ] {
        scalar(u64::from(value), 4)?;
    }
    for offset in [profile.item_fields.class_name, profile.item_fields.armor_info] {
        scalar(offset as u64, 4)?;
        if offset % 4 != 0 || offset + 4 > profile.globals.item_bytes {
            return Err(ClassicQ2Error::invalid(
                "Classic armor item field exceeds its original item record",
            ));
        }
    }
    for offset in [profile.armor_info.normal_protection, profile.armor_info.energy_protection] {
        scalar(offset as u64, 4)?;
        if offset % 4 != 0 {
            return Err(ClassicQ2Error::invalid("Classic armor information is unaligned"));
        }
    }
    if profile.armor_info.normal_protection == profile.armor_info.energy_protection
        || profile.item_fields.class_name == profile.item_fields.armor_info
    {
        return Err(ClassicQ2Error::invalid("Classic armor fields overlap"));
    }
    let mut distinct = profile.armor.regular.clone();
    distinct.sort_unstable();
    distinct.dedup();
    if profile.armor.regular.is_empty()
        || distinct.len() != profile.armor.regular.len()
        || !profile.armor.regular.contains(&profile.armor.empty)
    {
        return Err(ClassicQ2Error::invalid(
            "Classic regular armor requires distinct source priorities and an admitted empty tier",
        ));
    }
    for index in [
        profile.items.jacket,
        profile.items.combat,
        profile.items.body,
        profile.items.screen,
        profile.items.shield,
        profile.items.cells,
        profile.items.grenades,
    ]
    .into_iter()
    .chain(profile.armor.regular.iter().copied())
    {
        if index < 1 || index >= profile.client.inventory_count {
            return Err(ClassicQ2Error::invalid(
                "Classic combat inventory index exceeds its declared source storage",
            ));
        }
    }
    Ok(())
}

/// Select the combat profile for an artifact digest, if admitted.
#[must_use]
pub fn classic_combat_profile(digest: &ContentDigest) -> Option<ClassicCombatProfile> {
    if digest.algorithm == "sha256" && digest.value == XATRIX_DIGEST_VALUE {
        Some(xatrix_combat_profile())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::ProviderId;
    use qa_guest::core::contracts::{GuestAllocationOptions, ModuleIdentity};

    fn test_memory() -> SparseGuestMemory {
        SparseGuestMemory::new(
            ModuleIdentity::new(
                ProviderId::new("q2", "combat-profile-test"),
                "combat-profile",
                ContentDigest::new("sha256", "0"),
                "test",
            ),
            4,
            0x10000,
        )
        .unwrap()
    }

    #[test]
    fn xatrix_profile_validates_and_matches_its_digest() {
        let profile = xatrix_combat_profile();
        validate_classic_combat_profile(&profile).unwrap();
        assert_eq!(profile.game, ClassicGame::Xatrix);
        assert_eq!(profile.calls.damage.arguments.len(), 10);
        assert_eq!(profile.calls.regular_armor.arguments.len(), 6);
        assert_eq!(profile.calls.power_armor.arguments.len(), 5);
        let found = classic_combat_profile(&ContentDigest::new("sha256", XATRIX_DIGEST_VALUE)).unwrap();
        assert_eq!(found.digest, profile.digest);
        assert!(classic_combat_profile(&ContentDigest::new("sha256", "00")).is_none());
        let damage = native_combat_signature(&profile.calls.damage, ClassicCombatOperation::Damage).unwrap();
        assert_eq!(damage.result, None);
        let armor = native_combat_signature(&profile.calls.regular_armor, ClassicCombatOperation::RegularArmor).unwrap();
        assert_eq!(armor.result, Some(q2_int()));
    }

    #[test]
    fn combat_arguments_project_and_lower_with_defaults() {
        let mut memory = test_memory();
        let image = memory.allocate(&GuestAllocationOptions::bytes(0x400)).unwrap();
        let target = memory.allocate(&GuestAllocationOptions::bytes(4)).unwrap();
        memory.write_pointer(memory.offset(image, 0x100).unwrap(), Some(target)).unwrap();
        let mut call = stock_native_combat_call(ClassicCombatOperation::PowerArmor);
        call.arguments.push(ClassicNativeCombatArgument::Value {
            layout: q2_int(),
            bytes: 9i32.to_le_bytes().to_vec(),
        });
        call.arguments.push(ClassicNativeCombatArgument::Address {
            target: Some(CombatAddressDefault { rva: 0x100, indirections: vec![0] }),
        });
        validate_native_combat_call(&call, ClassicCombatOperation::PowerArmor).unwrap();
        let semantic: Vec<GuestCallValue> = vec![
            GuestCallValue::Pointer(Some(target)),
            GuestCallValue::Pointer(None),
            GuestCallValue::Pointer(None),
            GuestCallValue::Int32(25),
            GuestCallValue::Int32(0),
        ];
        let lowered = lower_native_combat_arguments(
            &call,
            ClassicCombatOperation::PowerArmor,
            &semantic,
            &mut memory,
            Some(image),
            None,
        )
        .unwrap();
        assert_eq!(lowered.len(), 7);
        assert_eq!(lowered[5], GuestCallValue::Int32(9));
        assert_eq!(lowered[6], GuestCallValue::Pointer(Some(target)));
        let projected = read_native_combat_arguments(&call, ClassicCombatOperation::PowerArmor, &lowered).unwrap();
        assert_eq!(projected, semantic);
    }

    #[test]
    fn validation_rejects_bad_strides_overlaps_and_armor() {
        let mut profile = xatrix_combat_profile();
        profile.entity_bytes = 259;
        assert!(validate_classic_combat_profile(&profile).is_err());
        profile = xatrix_combat_profile();
        profile.armor_info.energy_protection = profile.armor_info.normal_protection;
        assert!(validate_classic_combat_profile(&profile).is_err());
        profile = xatrix_combat_profile();
        profile.armor.empty = 9;
        assert!(validate_classic_combat_profile(&profile).is_err());
        profile = xatrix_combat_profile();
        profile.calls.damage.arguments.pop();
        assert!(validate_classic_combat_call(&profile.calls.damage, ClassicCombatOperation::Damage).is_err());
        assert!(validate_classic_combat_profile(&profile).is_err());
    }
}
