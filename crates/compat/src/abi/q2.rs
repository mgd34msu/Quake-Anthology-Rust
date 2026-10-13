//! Native Q2 import metadata for the child binder and the shared CallTable.
use super::*;
use NativeScalar::{Float, I32, U8, U32, Void, Word};
use qa_platform::native::{NativeAbi, NativeFormat, NativeImport, NativeListFilter, NativeScalar};

#[derive(Clone, Copy)]
enum Destination {
    Engine(Option<CallEntry>),
    Runtime(u32),
    Trap,
}
#[derive(Clone, Copy)]
struct Format {
    capacity: u16,
    argument: u8,
}
#[derive(Clone, Copy)]
pub(crate) struct Import {
    pub(crate) name: &'static [u8],
    parameters: &'static [NativeScalar],
    result: NativeScalar,
    destination: Destination,
    format: Option<Format>,
    filter: bool,
}
impl Import {
    const fn formatted(mut self, capacity: u16, argument: u8) -> Self {
        self.format = Some(Format { capacity, argument });
        self
    }
    const fn filtered(mut self) -> Self {
        self.filter = true;
        self
    }
    pub(crate) fn native(
        self,
        ordinal: usize,
        abi: NativeAbi,
        format_buffer: u64,
        filter: Option<NativeListFilter>,
    ) -> Option<NativeImport<'static>> {
        let (number, parameters, result) = match self.destination {
            Destination::Runtime(number) => {
                let function = qa_platform::native::runtime::function(number)?;
                (number, function.parameters, function.result)
            }
            _ => (ordinal as u32, self.parameters, self.result),
        };
        Some(NativeImport {
            trap: matches!(self.destination, Destination::Trap),
            number,
            abi,
            parameters,
            result,
            format: self.format.map(|format| NativeFormat {
                buffer: format_buffer,
                capacity: format.capacity,
                format: format.argument,
            }),
            filter: if self.filter { filter } else { None },
        })
    }
}
const fn engine(
    name: &'static [u8],
    parameters: &'static [NativeScalar],
    result: NativeScalar,
    handler: Option<CallEntry>,
) -> Import {
    Import {
        name,
        parameters,
        result,
        destination: Destination::Engine(handler),
        format: None,
        filter: false,
    }
}
const fn runtime(name: &'static [u8], number: u32) -> Import {
    Import {
        destination: Destination::Runtime(number),
        ..trap(name)
    }
}
const fn trap(name: &'static [u8]) -> Import {
    Import {
        name,
        parameters: &[],
        result: Void,
        destination: Destination::Trap,
        format: None,
        filter: false,
    }
}
#[derive(Clone, Copy)]
struct Slot {
    ordinal: usize,
    import: Import,
}
const fn pair(classic: usize, old: Import, rerelease: usize, new: Import) -> [Option<Slot>; 2] {
    [
        Some(Slot {
            ordinal: classic,
            import: old,
        }),
        Some(Slot {
            ordinal: rerelease,
            import: new,
        }),
    ]
}
const fn same(classic: usize, rerelease: usize, import: Import) -> [Option<Slot>; 2] {
    pair(classic, import, rerelease, import)
}
const fn only(rerelease: usize, import: Import) -> [Option<Slot>; 2] {
    [
        None,
        Some(Slot {
            ordinal: rerelease,
            import,
        }),
    ]
}

// Each native import is declared once with its two ABI cells. The rerelease's
// Pmove cell is a private child redirection, not a game_import_t pointer slot.
const ROWS: &[[Option<Slot>; 2]] = &[
    pair(
        0,
        engine(
            b"bprintf",
            &[I32, Word],
            Void,
            scalar!(native_print::<false, false>),
        )
        .formatted(2048, 1),
        0,
        engine(
            b"Broadcast_Print",
            &[I32, Word],
            Void,
            scalar!(native_print::<false, false>),
        ),
    ),
    pair(
        1,
        engine(b"dprintf", &[Word], Void, scalar!(print)).formatted(1024, 0),
        1,
        engine(b"Com_Print", &[Word], Void, scalar!(print)),
    ),
    pair(
        2,
        engine(
            b"cprintf",
            &[Word, I32, Word],
            Void,
            scalar!(native_print::<true, false>),
        )
        .formatted(1024, 2),
        2,
        engine(
            b"Client_Print",
            &[Word, I32, Word],
            Void,
            scalar!(native_print::<true, false>),
        ),
    ),
    pair(
        3,
        engine(
            b"centerprintf",
            &[Word, Word],
            Void,
            scalar!(native_print::<true, true>),
        )
        .formatted(1024, 1),
        3,
        engine(
            b"Center_Print",
            &[Word, Word],
            Void,
            scalar!(native_print::<true, true>),
        ),
    ),
    pair(
        4,
        engine(
            b"sound",
            &[Word, I32, I32, Float, Float, Float],
            Void,
            scalar!(native_sound::<false, false, false>),
        ),
        4,
        engine(
            b"sound",
            &[Word, U8, I32, Float, Float, Float],
            Void,
            scalar!(native_sound::<false, false, true>),
        ),
    ),
    pair(
        5,
        engine(
            b"positioned_sound",
            &[Word, Word, I32, I32, Float, Float, Float],
            Void,
            scalar!(native_sound::<true, false, false>),
        ),
        5,
        engine(
            b"positioned_sound",
            &[Word, Word, U8, I32, Float, Float, Float],
            Void,
            scalar!(native_sound::<true, false, true>),
        ),
    ),
    only(
        6,
        engine(
            b"local_sound",
            &[Word, Word, Word, U8, I32, Float, Float, Float, U32],
            Void,
            scalar!(native_sound::<true, true, true>),
        ),
    ),
    same(
        6,
        7,
        engine(b"configstring", &[I32, Word], Void, scalar!(config_set)),
    ),
    only(
        8,
        engine(
            b"get_configstring",
            &[I32],
            Word,
            scalar!(native_config_get),
        ),
    ),
    pair(
        7,
        engine(b"error", &[Word], Void, scalar!(abort)).formatted(1024, 0),
        9,
        engine(b"Com_Error", &[Word], Void, scalar!(abort)),
    ),
    same(
        8,
        10,
        engine(b"modelindex", &[Word], I32, scalar!(resource_index::<0>)),
    ),
    same(
        9,
        11,
        engine(b"soundindex", &[Word], I32, scalar!(resource_index::<1>)),
    ),
    same(
        10,
        12,
        engine(b"imageindex", &[Word], I32, scalar!(resource_index::<2>)),
    ),
    same(
        11,
        13,
        engine(b"setmodel", &[Word, Word], Void, scalar!(native_set_model)),
    ),
    pair(
        12,
        engine(
            b"trace",
            &[Word, Word, Word, Word, Word, Word, U32],
            Word,
            scalar!(native_trace::<false, false>),
        ),
        14,
        engine(
            b"trace",
            &[Word, Word, Word, Word, Word, Word, U32],
            Word,
            scalar!(native_trace::<true, false>),
        ),
    ),
    only(
        15,
        engine(
            b"clip",
            &[Word, Word, Word, Word, Word, Word, U32],
            Word,
            scalar!(native_trace::<true, true>),
        ),
    ),
    pair(
        13,
        engine(
            b"pointcontents",
            &[Word],
            I32,
            scalar!(native_point_contents),
        ),
        16,
        engine(
            b"pointcontents",
            &[Word],
            U32,
            scalar!(native_point_contents),
        ),
    ),
    pair(
        14,
        engine(
            b"inPVS",
            &[Word, Word],
            I32,
            scalar!(native_visibility::<false, false>),
        ),
        17,
        engine(
            b"inPVS",
            &[Word, Word, U8],
            U8,
            scalar!(native_visibility::<true, false>),
        ),
    ),
    pair(
        15,
        engine(
            b"inPHS",
            &[Word, Word],
            I32,
            scalar!(native_visibility::<false, true>),
        ),
        18,
        engine(
            b"inPHS",
            &[Word, Word, U8],
            U8,
            scalar!(native_visibility::<true, true>),
        ),
    ),
    pair(
        16,
        engine(
            b"SetAreaPortalState",
            &[I32, I32],
            Void,
            scalar!(native_portal_set::<false>),
        ),
        19,
        engine(
            b"SetAreaPortalState",
            &[I32, U8],
            Void,
            scalar!(native_portal_set::<true>),
        ),
    ),
    pair(
        17,
        engine(
            b"AreasConnected",
            &[I32, I32],
            I32,
            scalar!(native_areas_connected::<false>),
        ),
        20,
        engine(
            b"AreasConnected",
            &[I32, I32],
            U8,
            scalar!(native_areas_connected::<true>),
        ),
    ),
    same(
        18,
        21,
        engine(b"linkentity", &[Word], Void, scalar!(native_link)),
    ),
    same(
        19,
        22,
        engine(b"unlinkentity", &[Word], Void, scalar!(native_unlink)),
    ),
    pair(
        20,
        engine(
            b"BoxEdicts",
            &[Word, Word, Word, I32, I32],
            I32,
            scalar!(native_area_query::<false>),
        ),
        23,
        engine(
            b"BoxEdicts",
            &[Word, Word, Word, Word, I32, Word, Word],
            Word,
            scalar!(native_area_query::<true>),
        )
        .filtered(),
    ),
    same(
        21,
        70,
        engine(b"Pmove", &[Word], Void, scalar!(native_pmove)),
    ),
    pair(
        22,
        engine(
            b"multicast",
            &[Word, I32],
            Void,
            scalar!(native_multicast::<false>),
        ),
        24,
        engine(
            b"multicast",
            &[Word, I32, U8],
            Void,
            scalar!(native_multicast::<true>),
        ),
    ),
    pair(
        23,
        engine(
            b"unicast",
            &[Word, I32],
            Void,
            scalar!(native_unicast::<false>),
        ),
        25,
        engine(
            b"unicast",
            &[Word, U8, U32],
            Void,
            scalar!(native_unicast::<true>),
        ),
    ),
    same(
        24,
        26,
        engine(
            b"WriteChar",
            &[I32],
            Void,
            scalar!(native_message_bits::<8>),
        ),
    ),
    same(
        25,
        27,
        engine(
            b"WriteByte",
            &[I32],
            Void,
            scalar!(native_message_bits::<8>),
        ),
    ),
    same(
        26,
        28,
        engine(
            b"WriteShort",
            &[I32],
            Void,
            scalar!(native_message_bits::<16>),
        ),
    ),
    same(
        27,
        29,
        engine(
            b"WriteLong",
            &[I32],
            Void,
            scalar!(native_message_bits::<32>),
        ),
    ),
    same(
        28,
        30,
        engine(
            b"WriteFloat",
            &[Float],
            Void,
            scalar!(native_message_bits::<32>),
        ),
    ),
    same(
        29,
        31,
        engine(
            b"WriteString",
            &[Word],
            Void,
            scalar!(native_message_string),
        ),
    ),
    pair(
        30,
        engine(
            b"WritePosition",
            &[Word],
            Void,
            scalar!(native_message_position::<false>),
        ),
        32,
        engine(
            b"WritePosition",
            &[Word],
            Void,
            scalar!(native_message_position::<true>),
        ),
    ),
    same(
        31,
        33,
        engine(
            b"WriteDir",
            &[Word],
            Void,
            scalar!(native_message_direction),
        ),
    ),
    same(
        32,
        34,
        engine(b"WriteAngle", &[Float], Void, scalar!(native_message_angle)),
    ),
    only(
        35,
        engine(
            b"WriteEntity",
            &[Word],
            Void,
            scalar!(native_message_entity),
        ),
    ),
    pair(
        33,
        runtime(b"TagMalloc", qa_platform::native::runtime::TAG_MALLOC32),
        36,
        runtime(b"TagMalloc", qa_platform::native::runtime::TAG_MALLOC64),
    ),
    same(
        34,
        37,
        runtime(b"TagFree", qa_platform::native::runtime::TAG_FREE),
    ),
    same(
        35,
        38,
        runtime(b"FreeTags", qa_platform::native::runtime::FREE_TAGS),
    ),
    same(
        36,
        39,
        engine(b"cvar", &[Word, Word, U32], Word, scalar!(q2_cvar)),
    ),
    same(
        37,
        40,
        engine(
            b"cvar_set",
            &[Word, Word],
            Word,
            scalar!(q2_cvar_set::<false>),
        ),
    ),
    same(
        38,
        41,
        engine(
            b"cvar_forceset",
            &[Word, Word],
            Word,
            scalar!(q2_cvar_set::<true>),
        ),
    ),
    same(39, 42, engine(b"argc", &[], I32, scalar!(argc))),
    same(40, 43, engine(b"argv", &[I32], Word, scalar!(native_argv))),
    same(41, 44, engine(b"args", &[], Word, scalar!(native_args))),
    same(
        42,
        45,
        engine(b"AddCommandString", &[Word], Void, scalar!(command_append)),
    ),
    same(43, 46, trap(b"DebugGraph")),
    only(47, trap(b"GetExtension")),
    only(
        48,
        engine(
            b"Bot_RegisterEdict",
            &[Word],
            Void,
            scalar!(native_register_observer),
        ),
    ),
    only(
        49,
        engine(
            b"Bot_UnRegisterEdict",
            &[Word],
            Void,
            scalar!(native_forget_observer),
        ),
    ),
    only(50, trap(b"Bot_MoveToPoint")),
    only(51, trap(b"Bot_FollowActor")),
    only(52, trap(b"GetPathToGoal")),
    only(
        53,
        engine(
            b"Loc_Print",
            &[Word, I32, Word, Word, Word],
            Void,
            scalar!(native_localized_print),
        ),
    ),
    only(54, trap(b"Draw_Line")),
    only(55, trap(b"Draw_Point")),
    only(56, trap(b"Draw_Circle")),
    only(57, trap(b"Draw_Bounds")),
    only(58, trap(b"Draw_Sphere")),
    only(59, trap(b"Draw_OrientedWorldText")),
    only(60, trap(b"Draw_StaticWorldText")),
    only(61, trap(b"Draw_Cylinder")),
    only(62, trap(b"Draw_Ray")),
    only(63, trap(b"Draw_Arrow")),
    only(64, trap(b"ReportMatchDetails_Multicast")),
    only(
        65,
        engine(b"ServerFrame", &[], U32, scalar!(native_server_frame)),
    ),
    only(66, trap(b"SendToClipBoard")),
    only(
        67,
        engine(
            b"Info_ValueForKey",
            &[Word, Word, Word, Word],
            Word,
            scalar!(native_info_value),
        ),
    ),
    only(
        68,
        engine(
            b"Info_RemoveKey",
            &[Word, Word],
            U8,
            scalar!(native_info_remove),
        ),
    ),
    only(
        69,
        engine(
            b"Info_SetValueForKey",
            &[Word, Word, Word],
            U8,
            scalar!(native_info_set),
        ),
    ),
];

const fn project<const N: usize>(rerelease: bool) -> [Import; N] {
    let mut imports = [trap(b""); N];
    let mut row = 0;
    while row < ROWS.len() {
        if let Some(slot) = ROWS[row][rerelease as usize] {
            imports[slot.ordinal] = slot.import;
        }
        row += 1;
    }
    imports
}
pub(crate) const CLASSIC_IMPORTS: [Import; 44] = project(false);
pub(crate) const RERELEASE_IMPORTS: [Import; 71] = project(true);

const fn call_table(imports: &[Import]) -> CallTable {
    let mut table = CallTable {
        client_view: false,
        entries: [None; 256],
    };
    let mut ordinal = 0;
    while ordinal < imports.len() {
        if let Destination::Engine(handler) = imports[ordinal].destination {
            table.entries[ordinal] = handler;
        }
        ordinal += 1;
    }
    table
}
pub const CLASSIC: CallTable = call_table(&CLASSIC_IMPORTS);
pub const RERELEASE: CallTable = call_table(&RERELEASE_IMPORTS);
