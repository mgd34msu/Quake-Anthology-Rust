//! Port of `src/compat/q2/native-combat-call.ts`.
//! Bridges native combat declarations: field layouts, signatures and argument lowering.

use qa_guest::core::contracts::{
    GuestAddress, GuestCallSignature, GuestCallValue, GuestFieldLayout, GuestLayout, GuestStorage,
    GuestValueLayout, NativeAbi, NativeCallAbi,
};

use super::native_primary_reader::{
    NativeModAddress, Reader, native_offset, native_scalar, read_guest_layout,
};
use super::native_primary_weapons::{HostResult, NativeHostError, SyntheticHost};

/// Semantic combat argument slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CombatField {
    /// Damage target entity.
    Target,
    /// Damage inflictor entity.
    Inflictor,
    /// Damage attacker entity.
    Attacker,
    /// Damage direction vector.
    Direction,
    /// Damage point vector.
    Point,
    /// Surface normal vector.
    Normal,
    /// Damage amount.
    Amount,
    /// Knockback impulse.
    Knockback,
    /// Native damage flags.
    Flags,
    /// Means-of-death cause.
    Cause,
    /// Spark feedback flag.
    Sparks,
    /// Pain kick scalar.
    Kick,
}

impl CombatField {
    /// Profile label.
    #[must_use]
    pub const fn label(self) -> &'static str {
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

    /// Decode a profile label.
    #[must_use]
    pub const fn from_label(label: &str) -> Option<Self> {
        match label {
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

/// One declared combat argument.
#[derive(Debug, Clone, PartialEq)]
pub enum CombatArgument {
    /// Semantic slot filled by the caller.
    Field(CombatField),
    /// Fixed default value with explicit bytes.
    Value {
        /// Value layout.
        layout: GuestValueLayout,
        /// Little-endian default bytes.
        bytes: Vec<u8>,
    },
    /// Image-relative default address.
    Address(Option<NativeModAddress>),
}

/// Calling convention of one combat call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CombatConvention {
    /// 32-bit cdecl.
    Cdecl,
    /// 32-bit stdcall.
    Stdcall,
    /// 32-bit fastcall.
    Fastcall,
    /// 32-bit thiscall.
    Thiscall,
    /// 64-bit Microsoft x64.
    MicrosoftX64,
}

impl CombatConvention {
    /// Profile label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Cdecl => "cdecl",
            Self::Stdcall => "stdcall",
            Self::Fastcall => "fastcall",
            Self::Thiscall => "thiscall",
            Self::MicrosoftX64 => "microsoft-x64",
        }
    }

    /// Decode a profile label.
    #[must_use]
    pub const fn from_label(label: &str) -> Option<Self> {
        match label {
            "cdecl" => Some(Self::Cdecl),
            "stdcall" => Some(Self::Stdcall),
            "fastcall" => Some(Self::Fastcall),
            "thiscall" => Some(Self::Thiscall),
            "microsoft-x64" => Some(Self::MicrosoftX64),
            _ => None,
        }
    }

    /// Guest call ABI.
    #[must_use]
    pub const fn call_abi(self) -> NativeCallAbi {
        match self {
            Self::Cdecl => NativeCallAbi::Cdecl,
            Self::Stdcall => NativeCallAbi::Stdcall,
            Self::Fastcall => NativeCallAbi::Fastcall,
            Self::Thiscall => NativeCallAbi::Thiscall,
            Self::MicrosoftX64 => NativeCallAbi::MicrosoftX64,
        }
    }
}

/// Native combat declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeCombatCall {
    /// Calling convention.
    pub convention: CombatConvention,
    /// Declared arguments.
    pub arguments: Vec<CombatArgument>,
}

/// Combat operation selecting the required semantic slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CombatOperation {
    /// Entity damage.
    Damage,
    /// Regular armor absorption.
    RegularArmor,
    /// Power armor absorption.
    PowerArmor,
    /// Pain reaction.
    Pain,
    /// Death reaction.
    Death,
    /// Deferred reaction probe.
    DeferredReaction,
}

/// Rerelease `mod_t` cause layout: id, friendly fire and no-point-loss bytes.
#[must_use]
pub fn native_rerelease_mod_layout() -> GuestLayout {
    GuestLayout::new(
        "q2-rerelease:mod_t",
        3,
        1,
        8,
        vec![
            GuestFieldLayout {
                name: "id".to_string(),
                byte_offset: 0,
                storage: GuestStorage::Uint8,
                count: 1,
            },
            GuestFieldLayout {
                name: "friendly_fire".to_string(),
                byte_offset: 1,
                storage: GuestStorage::Uint8,
                count: 1,
            },
            GuestFieldLayout {
                name: "no_point_loss".to_string(),
                byte_offset: 2,
                storage: GuestStorage::Uint8,
                count: 1,
            },
        ],
    )
}

/// Required semantic slots for an operation at a pointer width.
#[must_use]
pub fn required_fields(operation: CombatOperation, pointer_bytes: usize) -> Vec<CombatField> {
    use CombatField as F;
    let wide = pointer_bytes == 8;
    match operation {
        CombatOperation::Damage => vec![
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
        CombatOperation::RegularArmor => {
            vec![F::Target, F::Point, F::Normal, F::Amount, F::Sparks, F::Flags]
        }
        CombatOperation::PowerArmor => vec![F::Target, F::Point, F::Normal, F::Amount, F::Flags],
        CombatOperation::Pain if wide => vec![F::Target, F::Attacker, F::Kick, F::Amount, F::Cause],
        CombatOperation::Pain => vec![F::Target, F::Attacker, F::Kick, F::Amount],
        CombatOperation::Death if wide => {
            vec![F::Target, F::Inflictor, F::Attacker, F::Amount, F::Point, F::Cause]
        }
        CombatOperation::Death => vec![F::Target, F::Inflictor, F::Attacker, F::Amount, F::Point],
        CombatOperation::DeferredReaction => vec![F::Target],
    }
}

/// Stock declaration: every required slot in order under the ABI convention.
#[must_use]
pub fn stock_native_combat_call(
    operation: CombatOperation,
    abi: Option<NativeAbi>,
) -> NativeCombatCall {
    let pointer_bytes = abi.map_or(4, NativeAbi::pointer_bytes);
    NativeCombatCall {
        convention: if pointer_bytes == 8 {
            CombatConvention::MicrosoftX64
        } else {
            CombatConvention::Cdecl
        },
        arguments: required_fields(operation, pointer_bytes)
            .into_iter()
            .map(CombatArgument::Field)
            .collect(),
    }
}

fn field_layout(field: CombatField, pointer_bytes: usize) -> GuestValueLayout {
    use CombatField as F;
    match field {
        F::Target | F::Inflictor | F::Attacker | F::Direction | F::Point | F::Normal => {
            GuestValueLayout::Scalar(GuestStorage::Pointer)
        }
        F::Amount | F::Knockback | F::Flags | F::Sparks => {
            GuestValueLayout::Scalar(GuestStorage::Int32)
        }
        F::Kick => GuestValueLayout::Scalar(GuestStorage::Float32),
        F::Cause if pointer_bytes == 4 => GuestValueLayout::Scalar(GuestStorage::Int32),
        F::Cause => GuestValueLayout::Aggregate(native_rerelease_mod_layout()),
    }
}

/// Guest signature for a declaration. The convention must match the image ABI.
pub fn native_combat_signature(
    call: &NativeCombatCall,
    operation: CombatOperation,
    abi: NativeAbi,
) -> Result<GuestCallSignature, String> {
    let source = match (abi, call.convention) {
        (NativeAbi::WindowsI386, convention)
            if convention != CombatConvention::MicrosoftX64 =>
        {
            convention.call_abi()
        }
        (NativeAbi::WindowsX86_64, CombatConvention::MicrosoftX64) => NativeCallAbi::MicrosoftX64,
        _ => return Err("native combat convention does not match its source architecture".to_string()),
    };
    let pointer_bytes = source.pointer_bytes();
    let parameters = call
        .arguments
        .iter()
        .map(|argument| match argument {
            CombatArgument::Address(_) => GuestValueLayout::Scalar(GuestStorage::Pointer),
            CombatArgument::Value { layout, .. } => layout.clone(),
            CombatArgument::Field(field) => field_layout(*field, pointer_bytes),
        })
        .collect();
    let result = match operation {
        CombatOperation::RegularArmor | CombatOperation::PowerArmor => {
            Some(GuestValueLayout::Scalar(GuestStorage::Int32))
        }
        _ => None,
    };
    Ok(GuestCallSignature {
        abi: source,
        parameters,
        result,
        variadic: false,
    })
}

/// Marshalled length of one value layout.
#[must_use]
pub const fn value_bytes(layout: &GuestValueLayout, pointer_bytes: usize) -> usize {
    match layout {
        GuestValueLayout::Scalar(storage) => storage.byte_length(pointer_bytes),
        GuestValueLayout::Aggregate(layout) => layout.byte_length,
    }
}

fn validate_layout(layout: &GuestValueLayout, pointer_bytes: usize) -> Result<(), String> {
    match layout {
        GuestValueLayout::Scalar(storage) => {
            if *storage == GuestStorage::Pointer {
                return Err("native combat pointer defaults require an image-relative address".to_string());
            }
            Ok(())
        }
        GuestValueLayout::Aggregate(layout) => {
            if layout.fields.iter().any(|field| field.storage == GuestStorage::Pointer) {
                return Err(
                    "native combat pointer defaults require an image-relative address".to_string(),
                );
            }
            for field in &layout.fields {
                if field.count == 0 {
                    return Err("native combat layout has an empty field".to_string());
                }
                let end = field.byte_offset + field.storage.byte_length(pointer_bytes) * field.count;
                if end > layout.byte_length {
                    return Err("native combat layout exceeds its record".to_string());
                }
            }
            Ok(())
        }
    }
}

/// Validate a declaration: exact required slots, bounded addresses and
/// default bytes matching their layouts.
pub fn validate_native_combat_call(
    call: &NativeCombatCall,
    operation: CombatOperation,
    abi: NativeAbi,
) -> Result<(), String> {
    native_combat_signature(call, operation, abi)?;
    let required = required_fields(operation, abi.pointer_bytes());
    let mut seen = Vec::new();
    for argument in &call.arguments {
        match argument {
            CombatArgument::Field(field) => {
                if !required.contains(field) || seen.contains(field) {
                    return Err("native combat fields must occur exactly once".to_string());
                }
                seen.push(*field);
            }
            CombatArgument::Address(address) => {
                if let Some(address) = address {
                    for value in core::iter::once(address.rva).chain(address.indirections.iter().copied()) {
                        if u64::from(value) > u64::from(u32::MAX) {
                            return Err(
                                "native combat address exceeds its image declaration".to_string()
                            );
                        }
                    }
                }
            }
            CombatArgument::Value { layout, bytes } => {
                validate_layout(layout, abi.pointer_bytes())?;
                if bytes.len() != value_bytes(layout, abi.pointer_bytes()) {
                    return Err(
                        "native combat default bytes do not match their source argument".to_string()
                    );
                }
            }
        }
    }
    if seen.len() != required.len() {
        return Err("native combat declaration omits a required source field".to_string());
    }
    Ok(())
}

/// Read a declaration from a profile subtree and validate it.
pub fn read_native_combat_call(
    reader: &Reader,
    operation: CombatOperation,
    abi: NativeAbi,
) -> NativeCombatCall {
    let convention = reader.field("convention").string();
    let convention = CombatConvention::from_label(&convention)
        .unwrap_or_else(|| reader.fail("unknown combat convention"));
    let arguments = reader.field("arguments").list(|value| {
        match value.field("kind").choice_index(&["field", "value", "address"]) {
            0 => {
                let field = value.field("field").string();
                CombatArgument::Field(
                    CombatField::from_label(&field).unwrap_or_else(|| value.fail("unknown combat field")),
                )
            }
            1 => {
                let layout = value.field("layout");
                let layout = if layout.field("kind").choice_index(&["scalar", "aggregate"]) == 0 {
                    GuestValueLayout::Scalar(native_scalar(&layout.field("storage")).storage())
                } else {
                    GuestValueLayout::Aggregate(read_guest_layout(&layout.field("layout")))
                };
                let bytes = value
                    .field("bytes")
                    .list(|byte| byte.integer(0))
                    .into_iter()
                    .map(|byte| {
                        u8::try_from(byte).unwrap_or_else(|_| value.fail("byte exceeds uint8"))
                    })
                    .collect();
                CombatArgument::Value { layout, bytes }
            }
            _ => CombatArgument::Address(value.field("address").nullable(|address| {
                NativeModAddress {
                    rva: native_offset(&address.field("rva")),
                    indirections: address.field("indirections").list(native_offset),
                }
            })),
        }
    });
    let call = NativeCombatCall {
        convention,
        arguments,
    };
    if let Err(message) = validate_native_combat_call(&call, operation, abi) {
        reader.fail(&message);
    }
    call
}

/// Project the semantic slots out of a declared invocation, in operation order.
/// Only the semantic slots are projected; extra values belong to the exact
/// intercepted invocation.
pub fn read_native_combat_arguments(
    call: &NativeCombatCall,
    operation: CombatOperation,
    values: &[GuestCallValue],
    pointer_bytes: usize,
) -> Result<Vec<GuestCallValue>, String> {
    if values.len() != call.arguments.len() {
        return Err("native combat invocation differs from its declared signature".to_string());
    }
    required_fields(operation, pointer_bytes)
        .into_iter()
        .map(|field| {
            let index = call
                .arguments
                .iter()
                .position(|argument| matches!(argument, CombatArgument::Field(slot) if *slot == field));
            match index.and_then(|index| values.get(index)) {
                Some(value) => Ok(value.clone()),
                None => Err("native combat invocation lacks a declared field".to_string()),
            }
        })
        .collect()
}

/// Find one semantic slot inside a declared invocation.
pub fn combat_argument_for_field<'a>(
    call: &NativeCombatCall,
    values: &'a [GuestCallValue],
    field: CombatField,
) -> Result<&'a GuestCallValue, String> {
    let index = call
        .arguments
        .iter()
        .position(|argument| matches!(argument, CombatArgument::Field(slot) if *slot == field));
    match index.and_then(|index| values.get(index)) {
        Some(value) => Ok(value),
        None => Err("native combat invocation lacks a declared field".to_string()),
    }
}

fn decode_value(
    layout: &GuestValueLayout,
    bytes: &[u8],
    pointer_bytes: usize,
) -> Result<GuestCallValue, NativeHostError> {
    let fail = |what: &str| NativeHostError::Fault(what.to_string());
    match layout {
        GuestValueLayout::Scalar(storage) => {
            if bytes.len() != storage.byte_length(pointer_bytes) {
                return Err(fail("native combat default bytes do not match their layout"));
            }
            let word = |index: usize| bytes[index];
            match storage {
                GuestStorage::Int8 => Ok(GuestCallValue::Int32(i32::from(word(0) as i8))),
                GuestStorage::Uint8 => Ok(GuestCallValue::Uint32(u32::from(word(0)))),
                GuestStorage::Int16 => Ok(GuestCallValue::Int32(i32::from(i16::from_le_bytes([
                    word(0),
                    word(1),
                ])))),
                GuestStorage::Uint16 => Ok(GuestCallValue::Uint32(u32::from(u16::from_le_bytes([
                    word(0),
                    word(1),
                ])))),
                GuestStorage::Int32 => Ok(GuestCallValue::Int32(i32::from_le_bytes([
                    word(0),
                    word(1),
                    word(2),
                    word(3),
                ]))),
                GuestStorage::Uint32 => Ok(GuestCallValue::Uint32(u32::from_le_bytes([
                    word(0),
                    word(1),
                    word(2),
                    word(3),
                ]))),
                GuestStorage::Int64 => {
                    let mut raw = [0u8; 8];
                    raw.copy_from_slice(bytes);
                    Ok(GuestCallValue::Int64(i64::from_le_bytes(raw)))
                }
                GuestStorage::Uint64 => {
                    let mut raw = [0u8; 8];
                    raw.copy_from_slice(bytes);
                    Ok(GuestCallValue::Uint64(u64::from_le_bytes(raw)))
                }
                GuestStorage::Float32 => {
                    let mut raw = [0u8; 4];
                    raw.copy_from_slice(bytes);
                    Ok(GuestCallValue::Float32(f32::from_le_bytes(raw)))
                }
                GuestStorage::Float64 => {
                    let mut raw = [0u8; 8];
                    raw.copy_from_slice(bytes);
                    Ok(GuestCallValue::Float64(f64::from_le_bytes(raw)))
                }
                GuestStorage::Pointer => Err(fail(
                    "native combat pointer defaults require an image-relative address",
                )),
            }
        }
        GuestValueLayout::Aggregate(layout) => {
            if bytes.len() != layout.byte_length {
                return Err(fail("native combat default bytes do not match their layout"));
            }
            Ok(GuestCallValue::Aggregate {
                layout: layout.clone(),
                bytes: bytes.to_vec(),
            })
        }
    }
}

/// Lower semantic values back into a declared invocation. Captured originals
/// win for non-field slots; otherwise value bytes decode and address defaults
/// resolve against `image`.
pub fn lower_native_combat_arguments(
    call: &NativeCombatCall,
    operation: CombatOperation,
    values: &[GuestCallValue],
    host: &mut SyntheticHost,
    image: Option<GuestAddress>,
    original: Option<&[GuestCallValue]>,
) -> HostResult<Vec<GuestCallValue>> {
    let semantic = required_fields(operation, host.core.pointer_bytes());
    if values.len() != semantic.len()
        || original.is_some_and(|captured| captured.len() != call.arguments.len())
    {
        return Err(NativeHostError::Fault(
            "native combat continuation changed its argument extent".to_string(),
        ));
    }
    call.arguments
        .iter()
        .enumerate()
        .map(|(index, argument)| {
            match argument {
                CombatArgument::Field(field) => {
                    let slot = semantic
                        .iter()
                        .position(|slot| slot == field)
                        .expect("validated semantic slot");
                    values.get(slot).cloned().ok_or_else(|| {
                        NativeHostError::Fault("missing native combat field".to_string())
                    })
                }
                _ if let Some(captured) = original => captured.get(index).cloned().ok_or_else(|| {
                    NativeHostError::Fault("missing captured native combat argument".to_string())
                }),
                CombatArgument::Value { layout, bytes } => {
                    decode_value(layout, bytes, host.core.pointer_bytes())
                }
                CombatArgument::Address(address) => {
                    if address.is_some() && image.is_none() {
                        return Err(NativeHostError::Fault(
                            "native combat address default requires its source image base"
                                .to_string(),
                        ));
                    }
                    let mut resolved = match (address, image) {
                        (Some(address), Some(image)) => {
                            Some(host.core.memory.offset(image, i64::from(address.rva))?)
                        }
                        _ => None,
                    };
                    if let Some(address) = address {
                        for displacement in &address.indirections {
                            let current = resolved.ok_or_else(|| {
                                NativeHostError::Fault(
                                    "native combat default address dereferences null".to_string(),
                                )
                            })?;
                            resolved = host.core.memory.read_pointer(
                                host.core.memory.offset(current, i64::from(*displacement))?,
                            )?;
                        }
                    }
                    Ok(GuestCallValue::Pointer(resolved))
                }
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::native_primary_reader::{CLASSIC_DIGEST, parse_json};
    use super::*;

    #[test]
    fn stock_calls_validate_per_operation() {
        for operation in [
            CombatOperation::Damage,
            CombatOperation::RegularArmor,
            CombatOperation::PowerArmor,
            CombatOperation::Pain,
            CombatOperation::Death,
            CombatOperation::DeferredReaction,
        ] {
            let call = stock_native_combat_call(operation, Some(NativeAbi::WindowsI386));
            validate_native_combat_call(&call, operation, NativeAbi::WindowsI386).expect("valid");
            let signature =
                native_combat_signature(&call, operation, NativeAbi::WindowsI386).expect("signature");
            assert_eq!(signature.parameters.len(), call.arguments.len());
            let armored = matches!(
                operation,
                CombatOperation::RegularArmor | CombatOperation::PowerArmor
            );
            assert_eq!(signature.result.is_some(), armored);
        }
        let wide = stock_native_combat_call(CombatOperation::Pain, Some(NativeAbi::WindowsX86_64));
        assert_eq!(wide.convention, CombatConvention::MicrosoftX64);
        assert_eq!(wide.arguments.len(), 5);
    }

    #[test]
    fn reads_mixed_declarations() {
        let value = parse_json(
            r#"{"convention":"cdecl","arguments":[
            {"kind":"field","field":"target"},
            {"kind":"value","layout":{"kind":"scalar","storage":"int32"},"bytes":[4,0,0,0]},
            {"kind":"address","address":{"rva":4096,"indirections":[]}}]}"#,
        )
        .expect("valid json");
        let root = Reader::root(&value);
        let call = read_native_combat_call(&root, CombatOperation::DeferredReaction, NativeAbi::WindowsI386);
        assert_eq!(call.arguments.len(), 3);
        assert!(matches!(call.arguments[0], CombatArgument::Field(CombatField::Target)));
    }

    #[test]
    fn projects_and_lowers_semantic_slots() {
        let mut host = SyntheticHost::synthetic(CLASSIC_DIGEST, 4, 0x2000).expect("host");
        let call = stock_native_combat_call(CombatOperation::DeferredReaction, None);
        let entity = host.core.at(0x100).expect("entity");
        let values = vec![GuestCallValue::Pointer(Some(entity))];
        let semantic =
            read_native_combat_arguments(&call, CombatOperation::DeferredReaction, &values, 4)
                .expect("project");
        assert_eq!(semantic, values);
        assert_eq!(
            combat_argument_for_field(&call, &values, CombatField::Target).expect("slot"),
            &values[0]
        );
        let lowered = lower_native_combat_arguments(
            &call,
            CombatOperation::DeferredReaction,
            &semantic,
            &mut host,
            Some(host.core.image),
            None,
        )
        .expect("lower");
        assert_eq!(lowered, values);
    }

    #[test]
    fn lowers_value_and_address_defaults() {
        let mut host = SyntheticHost::synthetic(CLASSIC_DIGEST, 4, 0x2000).expect("host");
        let image = host.core.image;
        let call = NativeCombatCall {
            convention: CombatConvention::Cdecl,
            arguments: vec![
                CombatArgument::Field(CombatField::Target),
                CombatArgument::Value {
                    layout: GuestValueLayout::Scalar(GuestStorage::Int32),
                    bytes: vec![9, 0, 0, 0],
                },
                CombatArgument::Address(Some(NativeModAddress {
                    rva: 0x400,
                    indirections: vec![],
                })),
            ],
        };
        validate_native_combat_call(&call, CombatOperation::DeferredReaction, NativeAbi::WindowsI386)
            .expect("valid");
        let entity = host.core.at(0x100).expect("entity");
        let lowered = lower_native_combat_arguments(
            &call,
            CombatOperation::DeferredReaction,
            &[GuestCallValue::Pointer(Some(entity))],
            &mut host,
            Some(image),
            None,
        )
        .expect("lower");
        assert_eq!(lowered[1], GuestCallValue::Int32(9));
        assert_eq!(
            lowered[2],
            GuestCallValue::Pointer(Some(host.core.at(0x400).expect("rva")))
        );
    }

    #[test]
    fn rejects_duplicate_or_missing_slots() {
        let duplicate = NativeCombatCall {
            convention: CombatConvention::Cdecl,
            arguments: vec![
                CombatArgument::Field(CombatField::Target),
                CombatArgument::Field(CombatField::Target),
            ],
        };
        assert!(validate_native_combat_call(
            &duplicate,
            CombatOperation::DeferredReaction,
            NativeAbi::WindowsI386
        )
        .is_err());
        let call = stock_native_combat_call(CombatOperation::Damage, None);
        assert!(read_native_combat_arguments(&call, CombatOperation::Damage, &[], 4).is_err());
    }
}
