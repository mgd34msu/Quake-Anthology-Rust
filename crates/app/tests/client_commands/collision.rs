use super::*;
use qa_app::{map::collision, model_assets::ModelCatalogBuilder};
use qa_compat::services::ModelLookup;

#[path = "../fixtures/map_collision.rs"]
#[allow(dead_code, unused_imports)]
mod map_fixture;

const ENTRIES: u32 = 60;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const NATIVE_DATA: u64 = 0x4000;

fn data() -> [u8; 1024] {
    let mut data = [0; 1024];
    for (at, text) in [
        (0, b"maps/q2.bsp\0".as_slice()),
        (16, b"maps/q2.bsp\0"),
        (48, b"maps/q3.bsp\0"),
        (80, b"maps/missing.bsp\0"),
        (112, b"maps/render-only.bsp\0"),
        (160, b"*1\0"),
    ] {
        data[at..at + text.len()].copy_from_slice(text);
    }
    for (at, value) in [
        (256, [-1.0f32; 3]),
        (272, [1.0; 3]),
        (288, [-4.0, 0.0, 0.0]),
        (304, [4.0, 0.0, 0.0]),
        (320, [0.0; 3]),
        (336, [10.0, 0.0, 0.0]),
        (352, [f32::NAN; 3]),
        (368, [0.0; 3]),
        (384, [40.0, 0.0, 0.0]),
        (400, [6.0, 0.0, 0.0]),
        (416, [14.0, 0.0, 0.0]),
        (432, [140.0, 0.0, 0.0]),
        (448, [100.0, 0.0, 0.0]),
        (464, [150.0, 0.0, 0.0]),
        (480, [110.0, 0.0, 0.0]),
        (640, [8192.0, 0.0, 0.0]),
        (656, [0.0001, 0.0, 0.0]),
        (672, [-1.0, -2.0, -3.0]),
        (688, [1.0, 2.0, 3.0]),
        (704, [-4.0, 2.5, 0.0]),
        (720, [4.0, 2.5, 0.0]),
        (736, [6.0, 2.5, 0.0]),
        (752, [14.0, 2.5, 0.0]),
        (768, [-1.0, -1.0, -4.0]),
        (784, [1.0, 1.0, 4.0]),
        (800, [-8.0, 0.0, 3.0]),
        (816, [8.0, 0.0, 3.0]),
        (832, [90.0, 0.0, 0.0]),
        (848, [0.0, 0.0, 3.0]),
        (864, [-0.5, -0.5, -1.0]),
        (880, [0.5, 0.5, 1.0]),
        (896, [2.0, 0.0, 3.0]),
        (912, [18.0, 0.0, 3.0]),
        (928, [0.75, 0.0, 0.0]),
        (960, [10.0, 0.0, 8.0]),
        (976, [10.0, 0.0, 0.0]),
    ] {
        for (axis, value) in value.into_iter().enumerate() {
            data[at + axis * 4..at + axis * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
    }
    data[508..572].fill(0xa5);
    data
}

fn query_call(entry: u32) -> Option<(u32, &'static [u32])> {
    Some(match entry {
        11 => (22, &[256, 272]),
        12 => (23, &[320, 255]),
        13 => (23, &[304, 255]),
        14 => (24, &[336, 255, 336, u32::MAX]),
        15 => (25, &[512, 288, 304, 0, 0, 255, 0x02000000]),
        16 => (26, &[512, 400, 416, 0, 0, 255, 0x02000000, 336, u32::MAX]),
        17 => (25, &[512, 288, 304, 256, 0, 255, 0x02000000]),
        18 => (25, &[512, 288, 304, 0, 272, 255, 0x02000000]),
        19 => (25, &[512, 288, 304, 0, 0, 255, 1]),
        20 => (25, &[512, 320, 320, 0, 0, 255, 0x02000000]),
        21 => (25, &[512, 384, 320, 0, 0, 0, 1]),
        22 => (25, &[512, 432, 448, 0, 0, 1, 1]),
        23 => (26, &[512, 464, 480, 0, 0, 1, 1, 336, 368]),
        24 => (23, &[448, 1]),
        25 => (24, &[480, 1, 336, 368]),
        26 => (23, &[u32::MAX, i32::MAX as u32]),
        27 => (25, &[512, u32::MAX, u32::MAX, u32::MAX, u32::MAX, 255, 1]),
        28 => (26, &[512, 640, 656, 0, 0, 255, 1, 336, u32::MAX]),
        29 => (25, &[u32::MAX, 288, 304, 0, 0, 255, 0x02000000]),
        30 => (25, &[288, 288, 304, 0, 0, 255, 0x02000000]),
        31 => (24, &[336, 255, 336, 352]),
        32 => (25, &[512, 288, 304, 0, 0, 254, 0x02000000]),
        33 => (23, &[448, 255]),
        34 => (25, &[512, 432, 448, 0, 0, 255, 1]),
        35 => (26, &[512, 464, 480, 0, 0, 255, 1, 336, u32::MAX]),
        36 => (23, &[65536, 255]),
        37 => (25, &[512, 0, 304, 0, 0, 255, 0x02000000]),
        38 => (83, &[512, 704, 720, 672, 688, 255, 0x02000000]),
        39 => (
            84,
            &[512, 736, 752, 672, 688, 255, 0x02000000, 336, u32::MAX],
        ),
        40 => (25, &[512, 704, 720, 672, 688, 255, 0x02000000]),
        41 => (83, &[512, u32::MAX, u32::MAX, u32::MAX, u32::MAX, 255, 1]),
        42 => (84, &[512, 640, 656, 0, 0, 255, 1, 336, u32::MAX]),
        43 => (83, &[512, 384, 320, 672, 688, 0, 1]),
        44 => (82, &[768, 784]),
        45 => (83, &[512, 800, 816, 0, 0, 254, 0x02000000]),
        46 => (84, &[512, 896, 912, 0, 0, 254, 0x02000000, 336, 832]),
        47 => (84, &[512, 960, 976, 0, 0, 254, 0x02000000, 336, 832]),
        48 => (23, &[848, 254]),
        49 => (24, &[848, 254, 368, 832]),
        50 => (23, &[848, 255]),
        51 => (82, &[u32::MAX, 784]),
        52 => (84, &[512, 896, 912, 0, 0, 254, 0x02000000, 336, u32::MAX]),
        53 => (82, &[864, 880]),
        54 => (25, &[512, 800, 816, 864, 880, 254, 0x02000000]),
        55 => (83, &[512, u32::MAX, u32::MAX, 0, 0, 254, 0x02000000]),
        56 => (84, &[512, 960, 976, 0, 0, 254, 0x02000000, 336, 352]),
        57 => (82, &[0, 784]),
        58 => (23, &[928, 255]),
        59 => (82, &[352, 784]),
        _ => return None,
    })
}
fn pointer_argument(number: u32, index: usize) -> bool {
    match number {
        22 | 82 => true,
        23 => index == 0,
        24 => index != 1,
        25 | 26 | 83 | 84 => index <= 4 || index >= 7,
        _ => false,
    }
}

fn call(entry: u32) -> (u32, Option<u32>) {
    match entry {
        0 => (19, None),
        1 => (18, Some(16)),
        2 => (18, Some(48)),
        3 => (20, None),
        4 => (36, Some(48)),
        5 => (37, Some(160)),
        6 => (18, Some(80)),
        7 => (18, Some(112)),
        8 => (18, Some(65536)),
        _ => unreachable!(),
    }
}

fn qvm_program() -> Program {
    use Opcode::*;
    let mut ops = vec![(Enter, 64)];
    let mut branches = Vec::new();
    for entry in 0..ENTRIES {
        ops.extend([(Local, 72), (Load4, 0), (Const, entry as i32), (Eq, 0)]);
        branches.push(ops.len() - 1);
    }
    ops.extend([(Const, -1), (Leave, 64)]);
    for (entry, branch) in branches.into_iter().enumerate() {
        ops[branch].1 = ops.len() as i32;
        if entry == 9 {
            ops.extend([(Local, 76), (Load4, 0), (Load4, 0), (Leave, 64)]);
            continue;
        }
        if entry == 10 {
            ops.extend([
                (Local, 76),
                (Load4, 0),
                (Local, 80),
                (Load4, 0),
                (Store4, 0),
                (Const, 0),
                (Leave, 64),
            ]);
            continue;
        }
        if let Some((number, words)) = query_call(entry as u32) {
            for (index, &word) in words.iter().enumerate() {
                ops.extend([(Const, word as i32), (Arg, 8 + index as i32 * 4)]);
            }
            ops.extend([(Const, -(number as i32) - 1), (Call, 0), (Leave, 64)]);
            continue;
        }
        let (number, pointer) = call(entry as u32);
        if let Some(pointer) = pointer {
            ops.extend([(Const, pointer as i32), (Arg, 8)]);
        } else if entry == 3 {
            ops.extend([(Local, 76), (Load4, 0), (Arg, 8)]);
        }
        ops.extend([(Const, -(number as i32) - 1), (Call, 0), (Leave, 64)]);
    }
    Program::Qvm {
        vm: Box::new(service_program::program(&ops, &data())),
        imports: &Q3_CLIENT,
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn native_program() -> Program {
    use native_image::*;
    use qa_compat::native::{NamedExport, Vm};
    use qa_formats::program::native::{Image, LoadRole};
    use qa_platform::native::NativeScalar;
    let mut file = pe_export_fixture(64);
    put(&mut file, 392 + 8, 16384, 4);
    put(&mut file, 392 + 16, 16384, 4);
    put(&mut file, 208, 20480, 4);
    file.resize(16896, 0);
    pe_rva(&mut file, 0x1140, 0x1800, 4);
    pe_text(&mut file, 0x1800, &[0xc3]);
    let mut vm = Vm::map_image(
        Image::parse(&file, None, LoadRole::Library).unwrap(),
        &(0..ENTRIES)
            .map(|entry| NamedExport {
                name: b"GetGameAPI",
                command: Some(entry),
                parameters: &[NativeScalar::Word; 3],
                result: NativeScalar::Word,
            })
            .collect::<Vec<_>>(),
        &[],
        Duration::from_secs(3),
    )
    .unwrap();
    let mut code = Vec::new();
    for entry in 0..ENTRIES {
        code.extend([0x83, 0xf9, entry as u8, 0x0f, 0x85]);
        let branch = code.len();
        code.extend([0; 4]);
        if entry == 9 || entry == 10 {
            code.extend([0x48, 0xb8]);
            code.extend((vm.process.base() + NATIVE_DATA).to_le_bytes());
            code.extend(if entry == 9 {
                &[0x8b, 0x04, 0x10, 0xc3][..]
            } else {
                &[0x44, 0x89, 0x04, 0x10, 0x31, 0xc0, 0xc3][..]
            });
        } else {
            code.extend([0x48, 0x83, 0xec, 0x68]);
            let number = if let Some((number, words)) = query_call(entry) {
                for (index, &word) in words.iter().enumerate() {
                    let value = if pointer_argument(number, index) && word != 0 && word != u32::MAX
                    {
                        vm.process.base() + NATIVE_DATA + u64::from(word)
                    } else {
                        u64::from(word)
                    };
                    if value > u64::from(u32::MAX) {
                        match index {
                            0 => code.extend([0x48, 0xba]),
                            1 => code.extend([0x49, 0xb8]),
                            2 => code.extend([0x49, 0xb9]),
                            _ => code.extend([0x48, 0xb8]),
                        }
                        code.extend(value.to_le_bytes());
                    } else {
                        match index {
                            0 => code.push(0xba),
                            1 => code.extend([0x41, 0xb8]),
                            2 => code.extend([0x41, 0xb9]),
                            _ => code.push(0xb8),
                        }
                        code.extend((value as u32).to_le_bytes());
                    }
                    if index >= 3 {
                        code.extend([0x48, 0x89, 0x44, 0x24, 0x20 + (index as u8 - 3) * 8]);
                    }
                }
                number
            } else {
                let (number, pointer) = call(entry);
                if let Some(pointer) = pointer {
                    // Native raw NULL is distinct from a QVM wrapped-to-zero base.
                    let address = if entry == 8 {
                        0
                    } else {
                        vm.process.base() + NATIVE_DATA + u64::from(pointer)
                    };
                    code.extend([0x48, 0xba]);
                    code.extend(address.to_le_bytes());
                }
                number
            };
            code.extend([0xb9, number as u8, 0, 0, 0, 0x48, 0xb8]);
            code.extend(vm.import_callback().to_le_bytes());
            code.extend([0xff, 0xd0, 0x48, 0x83, 0xc4, 0x68, 0xc3]);
        }
        let end = code.len();
        code[branch..branch + 4].copy_from_slice(&((end - branch - 4) as i32).to_le_bytes());
    }
    code.extend([0x31, 0xc0, 0xc3]);
    assert!(
        code.len() < NATIVE_DATA as usize - 0x1800,
        "fixture code bytes={}",
        code.len()
    );
    vm.process.memory_mut().unwrap()[0x1800..0x1800 + code.len()].copy_from_slice(&code);
    vm.process.memory_mut().unwrap()[NATIVE_DATA as usize..NATIVE_DATA as usize + data().len()]
        .copy_from_slice(&data());
    Program::Native {
        vm: Box::new(vm),
        imports: &Q3_CLIENT,
    }
}

fn rejected(host: &mut FrameHost, seat: usize, entry: u32, argument: i32) {
    assert!(
        host.call_module_export(
            ModuleId(seat as u16 + 1),
            Export {
                client: Some(ClientId(seat as u32)),
                ..Export::words(CallbackId(entry), &[argument as u32 as u64])
            },
            ThinkTime::Milliseconds(1),
        )
        .is_err(),
        "expected collision import rejection: entry={entry} native_argument={argument}"
    );
}

pub(super) fn selection(native: bool) {
    scenario(native, None, false);
    scenario(native, None, true);
    if native {
        // A rejected native service ends that owned-child module lifetime.
        for case in [
            (false, 3, 0, false),
            (true, 3, -1, false),
            (true, 3, 2, false),
            (true, 3, i32::MAX, false),
            (true, 6, 0, false),
            (true, 7, 0, false),
            (true, 8, 0, false),
            (true, 0, 0, true),
            (true, 3, 0, true),
            (true, 1, 0, true),
            (true, 29, 0, false),
            (true, 32, 0, false),
            (true, 12, 0, true),
            (true, 37, 0, false),
            (true, 51, 0, false),
            (true, 57, 0, false),
            (true, 52, 0, false),
            (true, 56, 0, false),
            (true, 59, 0, false),
        ] {
            scenario(true, Some(case), false);
        }
    }
    println!("CLIENT collision load/count/inline selection passed: owned_native={native}");
}

fn scenario(native: bool, rejection: Option<(bool, u32, i32, bool)>, inline_last: bool) {
    let program = || {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        if native {
            return native_program();
        }
        qvm_program()
    };
    let (mut runtime, ids) = runtime(2);
    let mut resources = Vec::new();
    for version in [38, 46] {
        let bytes = map_fixture::empty_map_bytes(version);
        let mut map = qa_formats::bsp::Map::parse(&bytes).unwrap();
        map_fixture::two_models(&mut map);
        if version == 46 {
            while map.models.len() < if inline_last { 256 } else { 3 } {
                map.models.push(map.models[1]);
            }
        }
        resources.push(collision(&map, &mut runtime.geometry).unwrap().0);
    }
    let [q2, q3] = resources.as_slice() else {
        unreachable!()
    };
    let (q2, q3) = (*q2, *q3);
    assert_eq!(runtime.geometry.model_count(q2), Some(2));
    assert_eq!(
        runtime.geometry.model_count(q3),
        Some(if inline_last { 256 } else { 3 })
    );
    // The SERVER world is deliberately not the first selected CLIENT world.
    runtime.collision = Some(qa_app::WorldCollision::new(&runtime.geometry, q3, 0));
    let requests = ids
        .iter()
        .enumerate()
        .map(|(seat, &client)| {
            let mut row = request(&mut runtime, ModuleId(seat as u16 + 1), client, program());
            row.entries = (0..ENTRIES).collect();
            row.frame.callback = CallbackId(0);
            row.context.trace_rules = if seat == 0 || inline_last {
                RuleSetId::Quake3
            } else {
                RuleSetId::Quake2
            };
            row
        })
        .collect();
    let mut host = FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        requests,
    )
    .unwrap();
    let mut assets = qa_render::Assets::load().unwrap();
    let a = model_support::world(&mut assets, 0.0);
    let b = model_support::world(&mut assets, 100.0);
    let mut catalog = ModelCatalogBuilder::new(assets, [], [a, b], Default::default()).unwrap();
    for (name, world, geometry) in [("maps/q2.bsp", a, q2), ("maps/q3.bsp", b, q3)] {
        let bounds = host.runtime.geometry.model_bounds(geometry, 0).unwrap();
        catalog
            .bind_loaded_world(name, world, Some(geometry), bounds)
            .unwrap();
    }
    catalog
        .bind_loaded_world("maps/render-only.bsp", a, None, Bounds::default())
        .unwrap();
    host.runtime.model_loader = Some(catalog);
    for seat in 0..2 {
        assert_eq!(probe(&mut host, seat, 0, 0), 0);
        assert_eq!(probe(&mut host, seat, 26, 0), 0); // No nodes: unused invalid point/handle.
        assert_eq!(probe(&mut host, seat, 11, 0), 255); // Temp hull can precede map load.
        assert_eq!(probe(&mut host, seat, 27, 0), 0); // No nodes: no input vector reads.
        assert_eq!(probe(&mut host, seat, 9, 520), 1.0f32.to_bits());
        assert_eq!(probe(&mut host, seat, 9, 524), 0);
        assert_eq!(probe(&mut host, seat, 28, 0), 0); // Wrapper reconstructs end, including rounding.
        assert_eq!(probe(&mut host, seat, 9, 524), 0);
        probe(&mut host, seat, 41, 0); // Capsule direct preserves no-node skipped pointers.
        assert_eq!(probe(&mut host, seat, 9, 524), 0);
        probe(&mut host, seat, 42, 0);
        assert_eq!(probe(&mut host, seat, 9, 524), 0);
        assert_eq!(probe(&mut host, seat, 44, 0), 254);
        probe(&mut host, seat, 55, 0); // Registered254, no nodes skips invalid input vectors.
        assert_eq!(probe(&mut host, seat, 9, 524), 0);
        if !native {
            rejected(&mut host, seat, 3, 0);
        }
    }
    if inline_last {
        assert_eq!(probe(&mut host, 1, 2, 0), 0);
        assert_eq!(probe(&mut host, 1, 0, 0), 256);
        assert_eq!(probe(&mut host, 1, 3, 255), 255);
        assert_eq!(probe(&mut host, 1, 3, 254), 254);
        assert_eq!(probe(&mut host, 1, 44, 0), 254);
        assert_eq!(probe(&mut host, 1, 48, 0), 0); // Real inline254 brush wins over registered temp254.
        probe(&mut host, 1, 45, 0);
        assert_eq!(trace_word(&mut host, 1, 8), 1.0f32.to_bits()); // Trace254 uses actual inline bounds, not registered tall bounds.
        assert_eq!(probe(&mut host, 1, 11, 0), 255);
        assert_eq!(probe(&mut host, 1, 12, 0), 0); // Actual model255 wins over temp at origin.
        assert_eq!(probe(&mut host, 1, 33, 0), 1); // Model255 owns brush at100.
        for (command, end) in [(34, 110.125f32), (35, 120.125)] {
            probe(&mut host, 1, command, 0);
            assert_eq!(trace_word(&mut host, 1, 12), end.to_bits());
            assert_eq!(trace_word(&mut host, 1, 48), 1);
        }
        return;
    }
    if let Some((selected, entry, argument, stale)) = rejection {
        assert_eq!(probe(&mut host, 1, 2, 0), 0);
        if selected {
            assert_eq!(probe(&mut host, 0, 1, 0), 0);
        }
        if [52, 56].contains(&entry) {
            probe(&mut host, 0, 44, 0);
        }
        if stale {
            assert!(host.runtime.geometry.remove(q2));
        }
        rejected(&mut host, 0, entry, argument);
        assert_eq!(
            host.module_state(ModuleId(1)),
            Some(qa_app::modules::State::Failed)
        );
        assert_eq!(probe(&mut host, 1, 0, 0), 3);
        assert_eq!(
            host.module_state(ModuleId(2)),
            Some(qa_app::modules::State::Running)
        );
        return;
    }
    assert_eq!(probe(&mut host, 0, 4, 0), 0); // Renderer B.
    assert_eq!(probe(&mut host, 0, 1, 0), 0); // Collision A.
    assert_eq!(probe(&mut host, 0, 0, 0), 2);
    assert_eq!(probe(&mut host, 1, 2, 0), 0);
    assert_eq!(probe(&mut host, 1, 0, 0), 3);
    query_cases(&mut host, native);
    for (seat, count) in [(0, 2), (1, 3)] {
        assert_eq!(probe(&mut host, seat, 3, 0), 0); // Model zero is valid.
        assert_eq!(probe(&mut host, seat, 3, count - 1), count as u32 - 1);
        if !native {
            rejected(&mut host, seat, 3, -1);
            rejected(&mut host, seat, 3, count as i32);
            rejected(&mut host, seat, 6, 0);
            rejected(&mut host, seat, 7, 0);
        }
        assert_eq!(probe(&mut host, seat, 0, 0), count as u32);
    }
    let render_model = probe(&mut host, 0, 5, 0);
    assert!(render_model > 0);
    if !native {
        assert_eq!(probe(&mut host, 0, 8, 0), 0); // Nonzero base masks to zero.
    }
    let (assets, catalog) = host.runtime.model_loader.take().unwrap().finish();
    assert_eq!(
        catalog.renderer_inline(render_model as i32).unwrap().world,
        b
    );
    assert_eq!(
        catalog.loaded_world(b"maps/q2.bsp").unwrap().collision,
        Some(q2)
    );
    assert_eq!(assets.world(a).unwrap().geometry().models.len(), 2);
    host.runtime.models = Some(std::sync::Arc::new(catalog));
    assert_eq!(probe(&mut host, 0, 2, 0), 0); // Frozen metadata uses same owner.
    assert_eq!(probe(&mut host, 0, 0, 0), 3);
    assert_eq!(probe(&mut host, 0, 1, 0), 0);
    if !native {
        rejected(&mut host, 0, 45, 0); // A world change clears old temp254 registration.
        let saved = [
            probe(&mut host, 0, 9, 0),
            probe(&mut host, 0, 9, 4),
            probe(&mut host, 0, 9, 8),
        ];
        for offset in [0, 4, 8] {
            write(&mut host, 0, offset, 0);
        }
        probe(&mut host, 0, 11, 0);
        assert_eq!(probe(&mut host, 0, 36, 0), 0x02000000); // Nonzero raw base masks to data0.
        rejected(&mut host, 0, 37, 0); // RawNULL must not become that valid vector.
        for (index, value) in saved.into_iter().enumerate() {
            write(&mut host, 0, index as u32 * 4, value);
        }
    }
    if native {
        assert_eq!(probe(&mut host, 0, 0, 0), 2);
        assert_eq!(probe(&mut host, 1, 0, 0), 3);
        return;
    }
    assert!(host.runtime.geometry.remove(q2));
    rejected(&mut host, 0, 0, 0);
    rejected(&mut host, 0, 3, 0);
    assert_eq!(probe(&mut host, 1, 0, 0), 3);
    assert_eq!(probe(&mut host, 0, 2, 0), 0);
    rejected(&mut host, 0, 1, 0); // Catalog reference is generation-stale.
    assert_eq!(probe(&mut host, 0, 0, 0), 3);
    assert_eq!(host.last_unknown_call(ModuleId(1)), None);
    assert_eq!(
        host.runtime.server.clients[0].player.movement_rules,
        RuleSetId::Quake
    );
    assert_eq!(
        host.runtime.server.clients[0].player.trace_rules,
        RuleSetId::QuakeWorld
    );
}

fn write(host: &mut FrameHost, seat: usize, offset: u32, value: u32) {
    assert_eq!(
        probe_args(host, seat, 10, &[u64::from(offset), u64::from(value)]),
        0
    );
}
fn trace_word(host: &mut FrameHost, seat: usize, offset: u32) -> u32 {
    probe(host, seat, 9, u64::from(512 + offset))
}
fn query_cases(host: &mut FrameHost, native: bool) {
    for seat in 0..2 {
        let epsilon = if seat == 0 { 0.125f32 } else { 0.03125 };
        let fraction = (3.0 - epsilon) / 8.0;
        assert_eq!(probe(host, seat, 11, 0), 255);
        assert_eq!(probe(host, seat, 12, 0), 0x02000000);
        assert_eq!(probe(host, seat, 13, 0), 0);
        assert_eq!(probe(host, seat, 14, 0), 0x02000000);
        assert_eq!(probe(host, seat, 31, 0), 0x02000000); // Raw255 never reads NaN angles.
        for command in [38, 39] {
            probe(host, seat, command, 0);
            assert_eq!(trace_word(host, seat, 8), 1.0f32.to_bits());
            assert_eq!(trace_word(host, seat, 48), 0);
        }
        probe(host, seat, 40, 0);
        assert!(f32::from_bits(trace_word(host, seat, 8)) < 1.0); // Box's Y extent differs from capsule radius.
        probe(host, seat, 43, 0);
        assert!(f32::from_bits(trace_word(host, seat, 8)) < 1.0); // Same capsule query clips admitted brush geometry.
        for command in [15, 16] {
            assert_eq!(probe(host, seat, command, 0), 0);
            let mut expected = [0u32; 14];
            expected[2] = fraction.to_bits();
            expected[3] = (if command == 15 {
                -1.0 - epsilon
            } else {
                9.0 - epsilon
            })
            .to_bits();
            expected[6] = (-1.0f32).to_bits();
            expected[9] = 1.0f32.to_bits();
            expected[10] = if seat == 0 { 0x0103 } else { 3 }; // Caller-selected native plane flags.
            expected[12] = 0x02000000;
            for (index, word) in expected.into_iter().enumerate() {
                assert_eq!(
                    trace_word(host, seat, index as u32 * 4),
                    word,
                    "trace command{command} word{index}"
                );
            }
            assert_eq!(probe(host, seat, 9, 508), 0xa5a5a5a5);
            assert_eq!(probe(host, seat, 9, 568), 0xa5a5a5a5);
        }
        for (command, fraction) in [(17, fraction), (18, (2.0 - epsilon) / 8.0)] {
            probe(host, seat, command, 0);
            assert_eq!(trace_word(host, seat, 8), fraction.to_bits());
        }
        probe(host, seat, 19, 0);
        assert_eq!(trace_word(host, seat, 8), 1.0f32.to_bits());
        assert_eq!(trace_word(host, seat, 12), 4.0f32.to_bits());
        assert_eq!(trace_word(host, seat, 48), 0);
        probe(host, seat, 20, 0);
        assert_eq!(trace_word(host, seat, 0), 1);
        assert_eq!(trace_word(host, seat, 4), 1);
        assert_eq!(trace_word(host, seat, 8), 0);
        for (command, end) in [
            (21, 10.0 + epsilon),
            (22, 110.0 + epsilon),
            (23, 120.0 + epsilon),
        ] {
            probe(host, seat, command, 0);
            assert!(f32::from_bits(trace_word(host, seat, 8)) < 1.0);
            assert_eq!(trace_word(host, seat, 12), end.to_bits());
            assert_eq!(trace_word(host, seat, 48), 1);
        }
        assert_eq!(probe(host, seat, 24, 0), 1);
        assert_eq!(probe(host, seat, 25, 0), 1);
        if !native {
            probe(host, seat, 15, 0);
            let before = trace_word(host, seat, 8);
            rejected(host, seat, 29, 0); // Entire output checked before any writes.
            assert_eq!(trace_word(host, seat, 8), before);
            rejected(host, seat, 32, 0); // Unknown254 is not a capsule/box alias.
            assert_eq!(trace_word(host, seat, 8), before);
        }
        // Output aliases start: all query inputs are captured before trace_t writes.
        probe(host, seat, 30, 0);
        assert_eq!(probe(host, seat, 9, 296), fraction.to_bits());
        // Restore every input word overlapped by this aliased56-byte output.
        for at in (288..344).step_by(4) {
            write(
                host,
                seat,
                at,
                u32::from_le_bytes(data()[at as usize..at as usize + 4].try_into().unwrap()),
            );
        }
        capsule_cases(host, seat, native);
    }
    // The same native capsule handle retains separate bounds in each module.
    probe(host, 1, 53, 0);
    assert_eq!(probe(host, 0, 48, 0), 0x02000000);
    assert_eq!(probe(host, 1, 48, 0), 0);
    // Two cgame modules retain independent mutable temp hulls.
    write(host, 1, 256, (-2.0f32).to_bits());
    write(host, 1, 272, 2.0f32.to_bits());
    probe(host, 1, 11, 0);
    for seat in 0..2 {
        write(host, seat, 304, 1.5f32.to_bits());
    }
    assert_eq!(probe(host, 0, 13, 0), 0);
    assert_eq!(probe(host, 1, 13, 0), 0x02000000);
    for seat in 0..2 {
        write(host, seat, 304, 4.0f32.to_bits());
    }
    // Finite inverted target bounds retain native admission semantics.
    write(host, 0, 256, 2.0f32.to_bits());
    assert_eq!(probe(host, 0, 11, 0), 255);
    write(host, 0, 256, (-1.0f32).to_bits());
    probe(host, 0, 11, 0);
}

fn capsule_cases(host: &mut FrameHost, seat: usize, native: bool) {
    assert_eq!(probe(host, seat, 44, 0), 254);
    assert_eq!(probe(host, seat, 48, 0), 0x02000000);
    assert_eq!(probe(host, seat, 49, 0), 0); // Pitch rotates the rounded target, including containment.
    assert_eq!(probe(host, seat, 50, 0), 0); // Capsule registration leaves retained255 unchanged.
    probe(host, seat, 45, 0);
    let fraction = trace_word(host, seat, 8);
    assert!(f32::from_bits(fraction) < 1.0);
    assert_eq!(trace_word(host, seat, 48), 0x02000000);
    assert_eq!(trace_word(host, seat, 52), 0);
    probe(host, seat, 46, 0);
    assert_eq!(trace_word(host, seat, 8), 1.0f32.to_bits()); // Same path clears the pitched target.
    probe(host, seat, 47, 0);
    assert!(f32::from_bits(trace_word(host, seat, 8)) < 1.0);
    assert!(f32::from_bits(trace_word(host, seat, 32)) > 0.99); // Plane restored into world +Z.
    assert_eq!(probe(host, seat, 9, 508), 0xa5a5a5a5);
    assert_eq!(probe(host, seat, 9, 568), 0xa5a5a5a5);
    if !native {
        for entry in [51, 52, 56, 57, 59] {
            rejected(host, seat, entry, 0);
            probe(host, seat, 45, 0);
            assert_eq!(trace_word(host, seat, 8), fraction); // Failed call leaves admitted capsule unchanged.
        }
    }
    probe(host, seat, 54, 0); // Existing box/capsule kernel, without overwriting retained255.
    assert!(f32::from_bits(trace_word(host, seat, 8)) < 1.0);
    assert_eq!(probe(host, seat, 50, 0), 0);
    assert_eq!(probe(host, seat, 58, 0), 0x02000000); // Small box query did not replace the larger retained255.
    assert_eq!(probe(host, seat, 53, 0), 254);
    probe(host, seat, 45, 0);
    assert_eq!(trace_word(host, seat, 8), 1.0f32.to_bits()); // Same handle, newly registered shorter target.
    assert_eq!(probe(host, seat, 44, 0), 254);
    probe(host, seat, 45, 0);
    assert_eq!(trace_word(host, seat, 8), fraction);
}
