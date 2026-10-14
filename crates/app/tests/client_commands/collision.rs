use super::*;
use qa_app::{map::collision, model_assets::ModelCatalogBuilder};
use qa_compat::services::ModelLookup;

#[path = "../fixtures/map_collision.rs"]
#[allow(dead_code, unused_imports)]
mod map_fixture;

const ENTRIES: u32 = 9;

fn data() -> [u8; 192] {
    let mut data = [0; 192];
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
    data
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
    put(&mut file, 392 + 8, 8192, 4);
    put(&mut file, 392 + 16, 8192, 4);
    put(&mut file, 208, 12288, 4);
    file.resize(8704, 0);
    pe_rva(&mut file, 0x1140, 0x1800, 4);
    pe_text(&mut file, 0x1800, &[0xc3]);
    let mut vm = Vm::map_image(
        Image::parse(&file, None, LoadRole::Library).unwrap(),
        &(0..ENTRIES)
            .map(|entry| NamedExport {
                name: b"GetGameAPI",
                command: Some(entry),
                parameters: &[NativeScalar::Word; 2],
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
        code.extend([0x48, 0x83, 0xec, 0x28]);
        let (number, pointer) = call(entry);
        if let Some(pointer) = pointer {
            // Native raw NULL is distinct from a QVM wrapped-to-zero base.
            let address = if entry == 8 {
                0
            } else {
                vm.process.base() + 0x2400 + u64::from(pointer)
            };
            code.extend([0x48, 0xba]);
            code.extend(address.to_le_bytes());
        }
        code.extend([0xb9, number as u8, 0, 0, 0, 0x48, 0xb8]);
        code.extend(vm.import_callback().to_le_bytes());
        code.extend([0xff, 0xd0, 0x48, 0x83, 0xc4, 0x28, 0xc3]);
        let end = code.len();
        code[branch..branch + 4].copy_from_slice(&((end - branch - 4) as i32).to_le_bytes());
    }
    code.extend([0x31, 0xc0, 0xc3]);
    assert!(code.len() < 0xc00);
    vm.process.memory_mut().unwrap()[0x1800..0x1800 + code.len()].copy_from_slice(&code);
    vm.process.memory_mut().unwrap()[0x2400..0x2400 + data().len()].copy_from_slice(&data());
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
        .is_err()
    );
}

pub(super) fn selection(native: bool) {
    scenario(native, None);
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
        ] {
            scenario(true, Some(case));
        }
    }
    println!("CLIENT collision load/count/inline selection passed: owned_native={native}");
}

fn scenario(native: bool, rejection: Option<(bool, u32, i32, bool)>) {
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
            map.models.push(map.models[1]);
        }
        resources.push(collision(&map, &mut runtime.geometry).unwrap().0);
    }
    let [q2, q3] = resources.as_slice() else {
        unreachable!()
    };
    let (q2, q3) = (*q2, *q3);
    assert_eq!(runtime.geometry.model_count(q2), Some(2));
    assert_eq!(runtime.geometry.model_count(q3), Some(3));
    // The SERVER world is deliberately not the first selected CLIENT world.
    runtime.collision = Some(qa_app::WorldCollision::new(&runtime.geometry, q3, 0));
    let requests = ids
        .iter()
        .enumerate()
        .map(|(seat, &client)| {
            let mut row = request(&mut runtime, ModuleId(seat as u16 + 1), client, program());
            row.entries = (0..ENTRIES).collect();
            row.frame.callback = CallbackId(0);
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
        if !native {
            rejected(&mut host, seat, 3, 0);
        }
    }
    if let Some((selected, entry, argument, stale)) = rejection {
        assert_eq!(probe(&mut host, 1, 2, 0), 0);
        if selected {
            assert_eq!(probe(&mut host, 0, 1, 0), 0);
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
