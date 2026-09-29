//! Donor: `src/compat/q2/classic/layout.ts` — API 3 ABI selector, record
//! layouts, and the game import/export tables.
//!
//! Bridges the 32-bit Q2 game ABI (cdecl calls, 4-byte pointers, 4-byte
//! alignment) to `qa-guest` layouts and call signatures. Also carries the
//! shared classic error type used by every other module in this directory.

use qa_guest::core::contracts::{
    GuestCallSignature, GuestFieldLayout, GuestLayout, GuestStorage, GuestValueLayout, NativeCallAbi,
};
use qa_guest::GuestError;
use qa_world::WorldError;
use thiserror::Error;

/// Errors for the classic Q2 compatibility shims.
#[derive(Debug, Error)]
pub enum ClassicQ2Error {
    /// A validation or protocol failure with a donor-shaped message.
    #[error("classic q2: {0}")]
    Invalid(String),
    /// A damage target was released while source armor ran.
    #[error("classic q2: native damage target was removed during armor protection")]
    TargetRemoved,
    /// Guest memory or ABI fault.
    #[error(transparent)]
    Guest(#[from] GuestError),
    /// World table fault.
    #[error(transparent)]
    World(#[from] WorldError),
}

impl ClassicQ2Error {
    /// Build a validation failure.
    #[must_use]
    pub fn invalid(detail: impl Into<String>) -> Self {
        Self::Invalid(detail.into())
    }
}

/// Result for the classic Q2 compatibility shims.
pub type ClassicResult<T> = Result<T, ClassicQ2Error>;

/// Classic Q2 ABI: 32-bit Windows cdecl (`pe32`, 4-byte pointers).
pub const CLASSIC_Q2_ABI: NativeCallAbi = NativeCallAbi::Cdecl;
/// Seconds per server frame.
pub const CLASSIC_Q2_FRAME_SECONDS: f64 = 0.1;
/// Public edict prefix length.
pub const CLASSIC_Q2_EDICT_BYTES: usize = 260;
/// Player state record length.
pub const CLASSIC_Q2_PLAYER_STATE_BYTES: usize = 184;
/// Client prefix length visible to the game module.
pub const CLASSIC_Q2_CLIENT_PREFIX_BYTES: usize = 188;
/// `usercmd_t` record length.
pub const CLASSIC_Q2_USERCMD_BYTES: usize = 16;
/// `pmove_t` record length.
pub const CLASSIC_Q2_PMOVE_BYTES: usize = 240;
/// Game export table length.
pub const CLASSIC_Q2_EXPORT_BYTES: usize = 80;
/// Game import table length (44 pointers).
pub const CLASSIC_Q2_IMPORT_BYTES: usize = 176;

fn field(name: &str, byte_offset: usize, storage: GuestStorage, count: usize) -> GuestFieldLayout {
    GuestFieldLayout {
        name: name.to_string(),
        byte_offset,
        storage,
        count,
    }
}

/// `q2-classic:usercmd` layout.
#[must_use]
pub fn classic_usercmd_layout() -> GuestLayout {
    GuestLayout::new(
        "q2-classic:usercmd",
        CLASSIC_Q2_USERCMD_BYTES,
        2,
        4,
        vec![
            field("msec", 0, GuestStorage::Uint8, 1),
            field("buttons", 1, GuestStorage::Uint8, 1),
            field("angles", 2, GuestStorage::Int16, 3),
            field("move", 8, GuestStorage::Int16, 3),
            field("impulse", 14, GuestStorage::Uint8, 1),
            field("lightlevel", 15, GuestStorage::Uint8, 1),
        ],
    )
}

/// `q2-classic:player-state` layout.
#[must_use]
pub fn classic_player_state_layout() -> GuestLayout {
    GuestLayout::new(
        "q2-classic:player-state",
        CLASSIC_Q2_PLAYER_STATE_BYTES,
        4,
        4,
        vec![
            field("pmove.pm_type", 0, GuestStorage::Int32, 1),
            field("pmove.origin", 4, GuestStorage::Int16, 3),
            field("pmove.velocity", 10, GuestStorage::Int16, 3),
            field("pmove.pm_flags", 16, GuestStorage::Uint8, 1),
            field("pmove.pm_time", 17, GuestStorage::Uint8, 1),
            field("pmove.gravity", 18, GuestStorage::Int16, 1),
            field("pmove.delta_angles", 20, GuestStorage::Int16, 3),
            field("viewangles", 28, GuestStorage::Float32, 3),
            field("viewoffset", 40, GuestStorage::Float32, 3),
            field("kick_angles", 52, GuestStorage::Float32, 3),
            field("gunangles", 64, GuestStorage::Float32, 3),
            field("gunoffset", 76, GuestStorage::Float32, 3),
            field("gunindex", 88, GuestStorage::Int32, 1),
            field("gunframe", 92, GuestStorage::Int32, 1),
            field("blend", 96, GuestStorage::Float32, 4),
            field("fov", 112, GuestStorage::Float32, 1),
            field("rdflags", 116, GuestStorage::Int32, 1),
            field("stats", 120, GuestStorage::Int16, 32),
        ],
    )
}

/// `q2-classic:edict-prefix` layout.
#[must_use]
pub fn classic_edict_layout() -> GuestLayout {
    GuestLayout::new(
        "q2-classic:edict-prefix",
        CLASSIC_Q2_EDICT_BYTES,
        4,
        4,
        vec![
            field("s.number", 0, GuestStorage::Int32, 1),
            field("s.origin", 4, GuestStorage::Float32, 3),
            field("s.angles", 16, GuestStorage::Float32, 3),
            field("s.old_origin", 28, GuestStorage::Float32, 3),
            field("s.modelindex", 40, GuestStorage::Int32, 4),
            field("s.frame", 56, GuestStorage::Int32, 1),
            field("s.skinnum", 60, GuestStorage::Int32, 1),
            field("s.effects", 64, GuestStorage::Uint32, 1),
            field("s.renderfx", 68, GuestStorage::Int32, 1),
            field("s.solid", 72, GuestStorage::Int32, 1),
            field("s.sound", 76, GuestStorage::Int32, 1),
            field("s.event", 80, GuestStorage::Int32, 1),
            field("client", 84, GuestStorage::Pointer, 1),
            field("inuse", 88, GuestStorage::Int32, 1),
            field("linkcount", 92, GuestStorage::Int32, 1),
            field("area", 96, GuestStorage::Pointer, 2),
            field("num_clusters", 104, GuestStorage::Int32, 1),
            field("clusternums", 108, GuestStorage::Int32, 16),
            field("headnode", 172, GuestStorage::Int32, 1),
            field("areanum", 176, GuestStorage::Int32, 2),
            field("svflags", 184, GuestStorage::Int32, 1),
            field("mins", 188, GuestStorage::Float32, 3),
            field("maxs", 200, GuestStorage::Float32, 3),
            field("absmin", 212, GuestStorage::Float32, 3),
            field("absmax", 224, GuestStorage::Float32, 3),
            field("size", 236, GuestStorage::Float32, 3),
            field("solid", 248, GuestStorage::Int32, 1),
            field("clipmask", 252, GuestStorage::Int32, 1),
            field("owner", 256, GuestStorage::Pointer, 1),
        ],
    )
}

/// `q2-classic:trace` layout.
#[must_use]
pub fn classic_trace_layout() -> GuestLayout {
    GuestLayout::new(
        "q2-classic:trace",
        56,
        4,
        4,
        vec![
            field("allsolid", 0, GuestStorage::Int32, 1),
            field("startsolid", 4, GuestStorage::Int32, 1),
            field("fraction", 8, GuestStorage::Float32, 1),
            field("endpos", 12, GuestStorage::Float32, 3),
            field("plane.normal", 24, GuestStorage::Float32, 3),
            field("plane.dist", 36, GuestStorage::Float32, 1),
            field("plane.type", 40, GuestStorage::Uint8, 1),
            field("plane.signbits", 41, GuestStorage::Uint8, 1),
            field("plane.pad", 42, GuestStorage::Uint8, 2),
            field("surface", 44, GuestStorage::Pointer, 1),
            field("contents", 48, GuestStorage::Int32, 1),
            field("ent", 52, GuestStorage::Pointer, 1),
        ],
    )
}

/// 32-bit integer value layout.
#[must_use]
pub fn q2_int() -> GuestValueLayout {
    GuestValueLayout::Scalar(GuestStorage::Int32)
}

/// Guest pointer value layout.
#[must_use]
pub fn q2_pointer() -> GuestValueLayout {
    GuestValueLayout::Scalar(GuestStorage::Pointer)
}

/// 32-bit float value layout.
#[must_use]
pub fn q2_float() -> GuestValueLayout {
    GuestValueLayout::Scalar(GuestStorage::Float32)
}

/// 64-bit float value layout.
#[must_use]
pub fn q2_double() -> GuestValueLayout {
    GuestValueLayout::Scalar(GuestStorage::Float64)
}

/// Trace aggregate value layout.
#[must_use]
pub fn q2_trace() -> GuestValueLayout {
    GuestValueLayout::Aggregate(classic_trace_layout())
}

/// Build a classic call signature (`None` result means `void`).
#[must_use]
pub fn classic_signature(
    parameters: Vec<GuestValueLayout>,
    result: Option<GuestValueLayout>,
    variadic: bool,
) -> GuestCallSignature {
    GuestCallSignature {
        abi: CLASSIC_Q2_ABI,
        parameters,
        result,
        variadic,
    }
}

/// One engine import: name plus signature. Order matches the donor table.
#[derive(Debug, Clone, PartialEq)]
pub struct ClassicQ2Import {
    /// Import name.
    pub name: &'static str,
    /// Call signature.
    pub signature: GuestCallSignature,
}

fn import(name: &'static str, signature: GuestCallSignature) -> ClassicQ2Import {
    ClassicQ2Import { name, signature }
}

/// The 44-entry `game_import_t` table in donor order.
#[must_use]
pub fn classic_q2_imports() -> Vec<ClassicQ2Import> {
    vec![
        import("bprintf", classic_signature(vec![q2_int(), q2_pointer()], None, true)),
        import("dprintf", classic_signature(vec![q2_pointer()], None, true)),
        import(
            "cprintf",
            classic_signature(vec![q2_pointer(), q2_int(), q2_pointer()], None, true),
        ),
        import(
            "centerprintf",
            classic_signature(vec![q2_pointer(), q2_pointer()], None, true),
        ),
        import(
            "sound",
            classic_signature(
                vec![q2_pointer(), q2_int(), q2_int(), q2_float(), q2_float(), q2_float()],
                None,
                false,
            ),
        ),
        import(
            "positioned_sound",
            classic_signature(
                vec![
                    q2_pointer(),
                    q2_pointer(),
                    q2_int(),
                    q2_int(),
                    q2_float(),
                    q2_float(),
                    q2_float(),
                ],
                None,
                false,
            ),
        ),
        import(
            "configstring",
            classic_signature(vec![q2_int(), q2_pointer()], None, false),
        ),
        import("error", classic_signature(vec![q2_pointer()], None, true)),
        import(
            "modelindex",
            classic_signature(vec![q2_pointer()], Some(q2_int()), false),
        ),
        import(
            "soundindex",
            classic_signature(vec![q2_pointer()], Some(q2_int()), false),
        ),
        import(
            "imageindex",
            classic_signature(vec![q2_pointer()], Some(q2_int()), false),
        ),
        import(
            "setmodel",
            classic_signature(vec![q2_pointer(), q2_pointer()], None, false),
        ),
        import(
            "trace",
            classic_signature(
                vec![
                    q2_pointer(),
                    q2_pointer(),
                    q2_pointer(),
                    q2_pointer(),
                    q2_pointer(),
                    q2_int(),
                ],
                Some(q2_trace()),
                false,
            ),
        ),
        import(
            "pointcontents",
            classic_signature(vec![q2_pointer()], Some(q2_int()), false),
        ),
        import(
            "inPVS",
            classic_signature(vec![q2_pointer(), q2_pointer()], Some(q2_int()), false),
        ),
        import(
            "inPHS",
            classic_signature(vec![q2_pointer(), q2_pointer()], Some(q2_int()), false),
        ),
        import(
            "SetAreaPortalState",
            classic_signature(vec![q2_int(), q2_int()], None, false),
        ),
        import(
            "AreasConnected",
            classic_signature(vec![q2_int(), q2_int()], Some(q2_int()), false),
        ),
        import("linkentity", classic_signature(vec![q2_pointer()], None, false)),
        import("unlinkentity", classic_signature(vec![q2_pointer()], None, false)),
        import(
            "BoxEdicts",
            classic_signature(
                vec![q2_pointer(), q2_pointer(), q2_pointer(), q2_int(), q2_int()],
                Some(q2_int()),
                false,
            ),
        ),
        import("Pmove", classic_signature(vec![q2_pointer()], None, false)),
        import(
            "multicast",
            classic_signature(vec![q2_pointer(), q2_int()], None, false),
        ),
        import("unicast", classic_signature(vec![q2_pointer(), q2_int()], None, false)),
        import("WriteChar", classic_signature(vec![q2_int()], None, false)),
        import("WriteByte", classic_signature(vec![q2_int()], None, false)),
        import("WriteShort", classic_signature(vec![q2_int()], None, false)),
        import("WriteLong", classic_signature(vec![q2_int()], None, false)),
        import("WriteFloat", classic_signature(vec![q2_float()], None, false)),
        import("WriteString", classic_signature(vec![q2_pointer()], None, false)),
        import("WritePosition", classic_signature(vec![q2_pointer()], None, false)),
        import("WriteDir", classic_signature(vec![q2_pointer()], None, false)),
        import("WriteAngle", classic_signature(vec![q2_float()], None, false)),
        import(
            "TagMalloc",
            classic_signature(vec![q2_int(), q2_int()], Some(q2_pointer()), false),
        ),
        import("TagFree", classic_signature(vec![q2_pointer()], None, false)),
        import("FreeTags", classic_signature(vec![q2_int()], None, false)),
        import(
            "cvar",
            classic_signature(vec![q2_pointer(), q2_pointer(), q2_int()], Some(q2_pointer()), false),
        ),
        import(
            "cvar_set",
            classic_signature(vec![q2_pointer(), q2_pointer()], Some(q2_pointer()), false),
        ),
        import(
            "cvar_forceset",
            classic_signature(vec![q2_pointer(), q2_pointer()], Some(q2_pointer()), false),
        ),
        import("argc", classic_signature(vec![], Some(q2_int()), false)),
        import("argv", classic_signature(vec![q2_int()], Some(q2_pointer()), false)),
        import("args", classic_signature(vec![], Some(q2_pointer()), false)),
        import("AddCommandString", classic_signature(vec![q2_pointer()], None, false)),
        import("DebugGraph", classic_signature(vec![q2_float(), q2_int()], None, false)),
    ]
}

/// One game export: name, table offset, and signature.
#[derive(Debug, Clone, PartialEq)]
pub struct ClassicQ2Export {
    /// Export name.
    pub name: &'static str,
    /// Byte offset in the export table.
    pub offset: u32,
    /// Call signature.
    pub signature: GuestCallSignature,
}

fn export(name: &'static str, offset: u32, signature: GuestCallSignature) -> ClassicQ2Export {
    ClassicQ2Export {
        name,
        offset,
        signature,
    }
}

/// The 15-entry game export table in donor order.
#[must_use]
pub fn classic_q2_exports() -> Vec<ClassicQ2Export> {
    vec![
        export("Init", 4, classic_signature(vec![], None, false)),
        export("Shutdown", 8, classic_signature(vec![], None, false)),
        export(
            "SpawnEntities",
            12,
            classic_signature(vec![q2_pointer(), q2_pointer(), q2_pointer()], None, false),
        ),
        export(
            "WriteGame",
            16,
            classic_signature(vec![q2_pointer(), q2_int()], None, false),
        ),
        export("ReadGame", 20, classic_signature(vec![q2_pointer()], None, false)),
        export("WriteLevel", 24, classic_signature(vec![q2_pointer()], None, false)),
        export("ReadLevel", 28, classic_signature(vec![q2_pointer()], None, false)),
        export(
            "ClientConnect",
            32,
            classic_signature(vec![q2_pointer(), q2_pointer()], Some(q2_int()), false),
        ),
        export("ClientBegin", 36, classic_signature(vec![q2_pointer()], None, false)),
        export(
            "ClientUserinfoChanged",
            40,
            classic_signature(vec![q2_pointer(), q2_pointer()], None, false),
        ),
        export(
            "ClientDisconnect",
            44,
            classic_signature(vec![q2_pointer()], None, false),
        ),
        export("ClientCommand", 48, classic_signature(vec![q2_pointer()], None, false)),
        export(
            "ClientThink",
            52,
            classic_signature(vec![q2_pointer(), q2_pointer()], None, false),
        ),
        export("RunFrame", 56, classic_signature(vec![], None, false)),
        export("ServerCommand", 60, classic_signature(vec![], None, false)),
    ]
}

/// Look up one game export by name.
#[must_use]
pub fn classic_q2_export(name: &str) -> Option<ClassicQ2Export> {
    classic_q2_exports().into_iter().find(|entry| entry.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_guest::abi::values::{decode_value, encode_value};

    #[test]
    fn import_table_has_44_entries_in_donor_order() {
        let imports = classic_q2_imports();
        assert_eq!(imports.len(), 44);
        assert_eq!(imports.len() * 4, CLASSIC_Q2_IMPORT_BYTES);
        let names: Vec<&str> = imports.iter().map(|entry| entry.name).collect();
        assert_eq!(names[0], "bprintf");
        assert_eq!(names[12], "trace");
        assert_eq!(names[21], "Pmove");
        assert_eq!(names[43], "DebugGraph");
        assert!(imports.iter().all(|entry| entry.signature.abi == CLASSIC_Q2_ABI));
        assert!(
            imports
                .iter()
                .find(|entry| entry.name == "bprintf")
                .unwrap()
                .signature
                .variadic
        );
        assert!(
            !imports
                .iter()
                .find(|entry| entry.name == "trace")
                .unwrap()
                .signature
                .variadic
        );
    }

    #[test]
    fn export_table_offsets_match_donor() {
        assert_eq!(classic_q2_export("Init").unwrap().offset, 4);
        assert_eq!(classic_q2_export("ClientThink").unwrap().offset, 52);
        assert_eq!(classic_q2_export("ServerCommand").unwrap().offset, 60);
        assert_eq!(classic_q2_exports().len(), 15);
        assert!(classic_q2_export("Missing").is_none());
        let connect = classic_q2_export("ClientConnect").unwrap();
        assert_eq!(connect.signature.parameters.len(), 2);
        assert_eq!(connect.signature.result, Some(q2_int()));
        assert_eq!(classic_q2_export("RunFrame").unwrap().signature.result, None);
    }

    #[test]
    fn record_layouts_carry_donor_byte_lengths() {
        assert_eq!(classic_usercmd_layout().byte_length, CLASSIC_Q2_USERCMD_BYTES);
        assert_eq!(classic_player_state_layout().byte_length, CLASSIC_Q2_PLAYER_STATE_BYTES);
        assert_eq!(classic_edict_layout().byte_length, CLASSIC_Q2_EDICT_BYTES);
        assert_eq!(classic_trace_layout().byte_length, 56);
        assert_eq!(classic_edict_layout().fields.len(), 29);
        let trace = q2_trace();
        assert!(matches!(trace, GuestValueLayout::Aggregate(_)));
    }

    #[test]
    fn value_layouts_round_trip_through_abi_codecs() {
        let memory = qa_guest::core::memory::SparseGuestMemory::new(
            qa_guest::core::contracts::ModuleIdentity::new(
                qa_core::identity::ProviderId::new("q2", "layout-test"),
                "layout",
                qa_guest::core::contracts::ContentDigest::new("sha256", "0"),
                "test",
            ),
            4,
            0x10000,
        )
        .unwrap();
        let encoded = encode_value(
            &q2_int(),
            &qa_guest::core::contracts::GuestCallValue::Int32(-7),
            &memory,
        )
        .unwrap();
        assert_eq!(encoded, (-7i32).to_le_bytes());
        let decoded = decode_value(&q2_int(), &encoded, &memory).unwrap();
        assert_eq!(decoded, qa_guest::core::contracts::GuestCallValue::Int32(-7));
        let error = ClassicQ2Error::invalid("probe");
        assert_eq!(format!("{error}"), "classic q2: probe");
    }
}
