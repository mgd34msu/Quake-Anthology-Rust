#[path = "support/quakec_program.rs"]
#[allow(dead_code)]
mod quakec_program;

use qa_compat::quakec::{Builtins, CallPhase, Layout, Trap, Vm};
use qa_core::{
    names::NameMatch,
    primitives::{CallbackId, ModuleId, NativeEntity, RuleSetId, ThinkTime},
};
use qa_formats::{
    entities::{EntityLump, EntitySyntax},
    program::quakec::{Image, Opcode},
};
use qa_session::dispatch::{FunctionBinding, FunctionTable};
use qa_world::entities::{AllocationPolicy, EntityTable};

const FIELDS: &[(u16, u16, &[u8])] = &[
    (1, 0, b"classname"),
    (2, 1, b"spawnflags"),
    (1, 2, b"model"),
    (2, 3, b"modelindex"),
    (2, 4, b"solid"),
    (2, 5, b"movetype"),
    (3, 6, b"angles"),
    (2, 9, b"angle"),
    (2, 10, b"light_lev"),
    (2, 11, b"light"),
    (2, 12, b"value"),
    (2, 13, b"Value"),
    (4, 14, b"target_entity"),
    (5, 15, b"target_field"),
    (6, 16, b"target_function"),
    (2, 17, b"nextthink"),
    (6, 18, b"think"),
    (0, 19, b"worldspawn"),
    (0, 20, b"ignored_void"),
    (7, 21, b"ignored_pointer"),
    (2, 22, b"frame"),
    (3, 23, b"mins"),
];
const GLOBALS: &[(u16, u16, &[u8])] = &[
    (4, 34, b"self"),
    (2, 35, b"time"),
    (4, 36, b"other"),
    (1, 37, b"mapname"),
    (2, 38, b"serverflags"),
    (2, 39, b"deathmatch"),
];

fn load(mutate: impl FnOnce(&mut Image)) -> Vm {
    let bytes = quakec_program::program_with_definitions(
        &[(Opcode::Return, 1, 0, 0)],
        &[0, 1, 1],
        GLOBALS,
        FIELDS,
        32,
        64,
    );
    let mut image = Image::parse(&bytes, Some(5927)).unwrap();
    image.functions[1].name = image.names.find(b"worldspawn").unwrap();
    image.functions[2].name = image.names.find(b"target_function").unwrap();
    mutate(&mut image);
    Vm::load(
        image,
        Layout {
            entities: 64,
            header_bytes: 16,
            extra_string_bytes: 2048,
            state_step: 0.1,
            random_seed: 1,
        },
    )
    .unwrap()
}
fn vm() -> Vm {
    load(|_| {})
}
fn read(vm: &Vm, entity: u32, name: &[u8], width: usize) -> Vec<u32> {
    let address = vm
        .field_address(entity, vm.image.field(name).unwrap() as u32, width)
        .unwrap();
    (0..width)
        .map(|index| vm.entities.read_word(address + index as u64 * 4).unwrap() as u32)
        .collect()
}
fn write(vm: &mut Vm, entity: u32, name: &[u8], words: &[u32]) {
    let address = vm
        .field_address(entity, vm.image.field(name).unwrap() as u32, words.len())
        .unwrap();
    for (index, &word) in words.iter().enumerate() {
        vm.entities
            .write_word(address + index as u64 * 4, word as i32)
            .unwrap();
    }
}
fn parse(bytes: &[u8], syntax: EntitySyntax) -> EntityLump<'_> {
    EntityLump::parse_with_match(bytes, syntax, NameMatch::Exact).unwrap()
}
fn seed(vm: &mut Vm, rules: RuleSetId) -> EntityTable {
    let mut entities = EntityTable::new(64, 4).unwrap();
    let anchor = entities
        .allocate(
            ThinkTime::Seconds(1.0),
            ModuleId(3),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    entities.columns.native_entity[anchor.slot as usize] = Some(NativeEntity {
        module: ModuleId(3),
        slot: 0,
    });
    vm.seed_entities(&entities, ModuleId(3), rules, 1).unwrap();
    entities
}
fn bind(vm: &mut Vm, entries: &[u32]) {
    let table = FunctionTable::<qa_session::clients::Server>::load([(
        ModuleId(3),
        RuleSetId::Quake,
        entries
            .iter()
            .map(|&entry| FunctionBinding {
                entry,
                call: |_, _, _, _| true,
            })
            .collect(),
    )])
    .unwrap();
    vm.bind_thinks(table.module_binding(ModuleId(3)), entries);
}

#[test]
fn exact_fields_do_not_inherit_map_lexical_syntax_or_folded_names() {
    for syntax in [
        EntitySyntax::Quake,
        EntitySyntax::Quake2,
        EntitySyntax::Quake3,
    ] {
        let mut vm = vm();
        let lump = parse(br#"{ "value" "1" "Value" "2" "VALUE" "3" }"#, syntax);
        let fields = vm.bind_map_fields(&lump.names, RuleSetId::Quake).unwrap();
        let mut warnings = Vec::new();
        vm.admit_map_record(&fields, 0, &lump.fields, |key| warnings.push(key.to_vec()))
            .unwrap();
        assert_eq!(read(&vm, 0, b"value", 1), [1.0f32.to_bits()]);
        assert_eq!(read(&vm, 0, b"Value", 1), [2.0f32.to_bits()]);
        assert_eq!(warnings, [b"VALUE".to_vec()]);
    }
}

#[test]
fn aliases_precede_nq_space_trim_while_qw_does_not_trim() {
    let lump = parse(
        br#"{ "angle" "45" "angle " "8" "light" "12" "light " "17" "unknown " "0" }"#,
        EntitySyntax::Quake2,
    );
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut vm = vm();
        let fields = vm.bind_map_fields(&lump.names, rules).unwrap();
        let mut warnings = Vec::new();
        vm.admit_map_record(&fields, 0, &lump.fields, |key| warnings.push(key.to_vec()))
            .unwrap();
        assert_eq!(
            read(&vm, 0, b"angles", 3),
            [0.0f32, 45.0, 0.0].map(f32::to_bits)
        );
        assert_eq!(read(&vm, 0, b"light_lev", 1), [12.0f32.to_bits()]);
        if rules == RuleSetId::Quake {
            assert_eq!(read(&vm, 0, b"angle", 1), [8.0f32.to_bits()]);
            assert_eq!(read(&vm, 0, b"light", 1), [17.0f32.to_bits()]);
            assert_eq!(warnings, [b"unknown".to_vec()]);
        } else {
            assert_eq!(read(&vm, 0, b"angle", 1), [0]);
            assert_eq!(read(&vm, 0, b"light", 1), [0]);
            assert_eq!(
                warnings,
                [b"angle ".to_vec(), b"light ".to_vec(), b"unknown ".to_vec()]
            );
        }
    }
}

#[test]
fn vectors_keep_literal_spaces_and_angle_alias_uses_the_same_decoder() {
    for (text, expected) in [
        (&b"1  3"[..], [1.0f32, 0.0, 3.0]),
        (&b"1\t2 3 4"[..], [1.0, 3.0, 4.0]),
        (&b"1"[..], [1.0, 0.0, 0.0]),
        (&b"-0 2 3 4"[..], [-0.0, 2.0, 3.0]),
    ] {
        let source = [b"{ \"angles\" \"".as_slice(), text, b"\" }"].concat();
        let lump = parse(&source, EntitySyntax::Quake);
        let mut vm = vm();
        let fields = vm.bind_map_fields(&lump.names, RuleSetId::Quake).unwrap();
        vm.admit_map_record(&fields, 0, &lump.fields, |_| {})
            .unwrap();
        assert_eq!(read(&vm, 0, b"angles", 3), expected.map(f32::to_bits));
    }
    let lump = parse(br#"{ "angle" " 90" }"#, EntitySyntax::Quake);
    let mut vm = vm();
    let fields = vm.bind_map_fields(&lump.names, RuleSetId::Quake).unwrap();
    vm.admit_map_record(&fields, 0, &lump.fields, |_| {})
        .unwrap();
    assert_eq!(
        read(&vm, 0, b"angles", 3),
        [0.0f32, 0.0, 90.0].map(f32::to_bits)
    );
}

#[test]
fn typed_values_preserve_raw_bytes_native_offsets_and_current_field_global() {
    let source = b"{ \"value\" \"-0junk\xff\" \"model\" \"raw\xff\\nend\\q\" \"target_entity\" \"7suffix\" \"target_field\" \"value\" \"target_function\" \"worldspawn\" }";
    let lump = parse(source, EntitySyntax::Quake3);
    let mut vm = vm();
    let entities = seed(&mut vm, RuleSetId::Quake);
    let fields = vm.bind_map_fields(&lump.names, RuleSetId::Quake).unwrap();
    for word in [0xfedc_ba98, 0x0123_4567] {
        vm.image.globals[12] = word;
        vm.admit_map_record(&fields, 0, &lump.fields, |_| {})
            .unwrap();
        assert_eq!(read(&vm, 0, b"value", 1), [(-0.0f32).to_bits()]);
        assert_eq!(read(&vm, 0, b"target_field", 1), [word]);
        assert_eq!(read(&vm, 0, b"target_function", 1), [1]);
        assert_eq!(read(&vm, 0, b"target_entity", 1), [(vm.stride * 7) as u32]);
        let string = read(&vm, 0, b"model", 1)[0];
        assert_eq!(vm.string(string as i32).unwrap(), b"raw\xff\nend\\");
    }
    assert_eq!(
        vm.resolve_entity(&entities, ModuleId(3), (vm.stride * 7) as u32)
            .unwrap(),
        None
    );
}

#[test]
fn known_field_or_function_values_fail_before_later_spawn_inhibition() {
    for (name, value, trap) in [
        ("target_field", "VALUE", Trap::Memory),
        ("target_function", "Worldspawn", Trap::Function),
        ("target_entity", "-1", Trap::Memory),
        ("target_entity", "64", Trap::Memory),
    ] {
        let source = format!("{{ \"spawnflags\" \"2048\" \"{name}\" \"{value}\" }}");
        let lump = parse(source.as_bytes(), EntitySyntax::Quake2);
        let mut vm = vm();
        let fields = vm.bind_map_fields(&lump.names, RuleSetId::Quake).unwrap();
        assert_eq!(
            vm.admit_map_record(&fields, 0, &lump.fields, |_| {}),
            Err(trap)
        );
    }
    let mut vm = load(|image| {
        image
            .fielddefs
            .iter_mut()
            .find(|def| image.names.get(def.name) == Some(b"value".as_slice()))
            .unwrap()
            .offset = 80;
        image.entityfields = 96;
    });
    let lump = parse(br#"{ "target_field" "value" }"#, EntitySyntax::Quake);
    let fields = vm.bind_map_fields(&lump.names, RuleSetId::Quake).unwrap();
    assert_eq!(
        vm.admit_map_record(&fields, 0, &lump.fields, |_| {}),
        Err(Trap::Memory)
    );
}

#[test]
fn utility_and_unknown_pairs_count_as_initialized_and_void_pointer_fields_do_nothing() {
    let mut vm = vm();
    write(&mut vm, 0, b"ignored_void", &[71]);
    write(&mut vm, 0, b"ignored_pointer", &[81]);
    let lump = parse(
        br#"{ "_editor" "x" "unknown" "y" "ignored_void" "z" "ignored_pointer" "q" }"#,
        EntitySyntax::Quake,
    );
    let fields = vm.bind_map_fields(&lump.names, RuleSetId::Quake).unwrap();
    let mut warnings = Vec::new();
    let admitted = vm
        .admit_map_record(&fields, 0, &lump.fields, |key| warnings.push(key.to_vec()))
        .unwrap();
    assert!(admitted.has_pairs);
    assert_eq!(admitted.classname, None);
    assert_eq!(warnings, [b"unknown".to_vec()]);
    assert_eq!(read(&vm, 0, b"ignored_void", 1), [71]);
    assert_eq!(read(&vm, 0, b"ignored_pointer", 1), [81]);
    assert!(
        !vm.admit_map_record(&fields, 0, &[], |_| {})
            .unwrap()
            .has_pairs
    );
}

struct Reject;
impl Builtins for Reject {
    fn entity_access(&self) -> qa_compat::quakec::EntityAccess<'_> {
        qa_compat::quakec::EntityAccess::native_only()
    }
    fn call(&mut self, _: &mut Vm, _: u32, _: usize) -> Result<(), Trap> {
        Err(Trap::Builtin)
    }
}

#[test]
fn classname_empty_unknown_and_cached_callback_mapping_remain_distinct() {
    let mut vm = vm();
    bind(&mut vm, &[2, 0, 1]);
    for (source, expected) in [
        (br#"{ "value" "1" }"#.as_slice(), None),
        (
            br#"{ "classname" "worldspawn" }"#.as_slice(),
            Some(Some(CallbackId(2))),
        ),
        (br#"{ "classname" "Worldspawn" }"#.as_slice(), Some(None)),
        (
            br#"{ "classname" "" }"#.as_slice(),
            Some(Some(CallbackId(1))),
        ),
    ] {
        // Ordinary rows start clear through ED_Alloc. This fixture uses a
        // fresh backing row without pretending it has a common lifetime.
        write(&mut vm, 0, b"classname", &[0]);
        let lump = parse(source, EntitySyntax::Quake2);
        let fields = vm.bind_map_fields(&lump.names, RuleSetId::Quake).unwrap();
        let admitted = vm
            .admit_map_record(&fields, 0, &lump.fields, |_| {})
            .unwrap();
        assert_eq!(
            admitted
                .classname
                .map(|name| vm.map_spawn_callback(name).unwrap()),
            expected
        );
        if source == br#"{ "classname" "" }"# {
            assert_ne!(admitted.classname.unwrap(), 0);
            assert_eq!(vm.call(&mut Reject, 0, 10, false), Err(Trap::Function));
        }
    }
    bind(&mut vm, &[2]);
    let name = vm.insert_string(b"worldspawn").unwrap() as u32;
    assert_eq!(vm.map_spawn_callback(name), Err(Trap::Function));
}

#[test]
fn spawn_updates_only_self_while_export_and_think_keep_their_native_globals() {
    let mut vm = vm();
    vm.prepare_call(Some(0), Some(0), ThinkTime::Seconds(1.0), CallPhase::Export)
        .unwrap();
    let entity = vm.stride as u32 * 7;
    write(&mut vm, entity, b"nextthink", &[2.0f32.to_bits()]);
    vm.image.globals[35] = 7.25f32.to_bits();
    vm.image.globals[36] = 99;
    vm.prepare_call(
        Some(entity),
        Some(0),
        ThinkTime::Seconds(8.0),
        CallPhase::Spawn,
    )
    .unwrap();
    assert_eq!(vm.image.globals[34], entity);
    assert_eq!(vm.image.globals[35], 7.25f32.to_bits());
    assert_eq!(vm.image.globals[36], 99);
    assert_eq!(read(&vm, entity, b"nextthink", 1), [2.0f32.to_bits()]);
    bind(&mut vm, &[1]);
    vm.prepare_call(
        Some(entity),
        Some(0),
        ThinkTime::Seconds(8.0),
        CallPhase::Think,
    )
    .unwrap();
    assert_eq!(vm.image.globals[35], 8.0f32.to_bits());
    assert_eq!(vm.image.globals[36], 0);
    assert_eq!(read(&vm, entity, b"nextthink", 1), [0]);
}

#[test]
fn world_seed_keeps_other_native_fields_and_parser_does_not_clear_it() {
    let mut vm = vm();
    let entities = seed(&mut vm, RuleSetId::Quake);
    write(&mut vm, 0, b"frame", &[99.0f32.to_bits()]);
    write(&mut vm, 0, b"mins", &[1.0f32, 2.0, 3.0].map(f32::to_bits));
    vm.seed_map_world(0, b"maps/start.bsp").unwrap();
    assert_eq!(read(&vm, 0, b"modelindex", 1), [1.0f32.to_bits()]);
    assert_eq!(read(&vm, 0, b"solid", 1), [4.0f32.to_bits()]);
    assert_eq!(read(&vm, 0, b"movetype", 1), [7.0f32.to_bits()]);
    assert_eq!(
        vm.string(read(&vm, 0, b"model", 1)[0] as i32).unwrap(),
        b"maps/start.bsp"
    );
    let lump = parse(br#"{ "classname" "worldspawn" }"#, EntitySyntax::Quake);
    let fields = vm.bind_map_fields(&lump.names, RuleSetId::Quake).unwrap();
    vm.admit_map_record(&fields, 0, &lump.fields, |_| {})
        .unwrap();
    assert_eq!(read(&vm, 0, b"frame", 1), [99.0f32.to_bits()]);
    assert_eq!(
        read(&vm, 0, b"mins", 3),
        [1.0f32, 2.0, 3.0].map(f32::to_bits)
    );
    let anchor = vm
        .resolve_entity(&entities, ModuleId(3), 0)
        .unwrap()
        .unwrap();
    assert_ne!(anchor.slot, 0);
    assert_eq!(
        vm.seed_map_world(vm.stride as u32, b"maps/start.bsp"),
        Err(Trap::Memory)
    );
}

#[test]
fn native_initial_globals_are_typed_and_optional_numeric_globals_do_not_grow_the_program() {
    let mut vm = vm();
    vm.initialize_map(ThinkTime::Seconds(1.0), b"start\xff", 5.0)
        .unwrap();
    assert_eq!(vm.image.globals[35], 1.0f32.to_bits());
    assert_eq!(
        vm.string(vm.image.globals[37] as i32).unwrap(),
        b"start\xff"
    );
    assert_eq!(vm.image.globals[38], 5.0f32.to_bits());
    vm.set_time(ThinkTime::Seconds(1.03125)).unwrap();
    assert_eq!(vm.image.globals[35], 1.03125f32.to_bits());
    let count = vm.image.globaldefs.len();
    vm.set_map_number(b"coop", 1.0).unwrap();
    assert_eq!(vm.image.globaldefs.len(), count);
    vm.set_map_number(b"deathmatch", 1.0).unwrap();
    assert_eq!(vm.image.globals[39], 1.0f32.to_bits());
    assert_eq!(vm.set_map_number(b"mapname", 1.0), Err(Trap::Memory));
    for name in [b"time".as_slice(), b"mapname", b"serverflags"] {
        let mut vm = load(|image| {
            let id = image.names.find(name).unwrap();
            image
                .globaldefs
                .iter_mut()
                .find(|def| def.name == id)
                .unwrap()
                .value_type = 4;
        });
        let before = vm.image.globals.to_vec();
        assert_eq!(
            vm.initialize_map(ThinkTime::Seconds(1.0), b"start", 1.0),
            Err(Trap::Memory)
        );
        assert_eq!(&vm.image.globals[..], before);
    }
}
