#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use qa_app::{
    Runtime,
    host::FrameHost,
    modules::{Export, ModuleResult, Phase, Program, Q3Spec, State, load_q3},
};
use qa_compat::{
    memory::{ModuleMemory, Width},
    native::{Error, ReturnedTable, TableFunction},
};
use qa_console::{commands::Console, views::Context};
use qa_core::primitives::{
    CallbackId, ClientId, ModuleId, NativeEntity, NumericValue, RuleSetId, ThinkTime, Vec3,
};
use qa_formats::program::native::{Image, LoadRole};
use qa_platform::native::{NativeAbi, NativeAction, NativeRequest, NativeScalar};
use qa_session::timing::TickRate;
use std::path::PathBuf;

#[path = "support/visibility_world.rs"]
mod visibility_world;

struct Files(PathBuf);
impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn symbol(image: &Image, name: &[u8]) -> u64 {
    image
        .symbols
        .iter()
        .find(|s| s.defined && s.name.and_then(|id| image.names.get(id)) == Some(name))
        .unwrap()
        .address
}

#[test]
#[ignore = "requires compiled original Q3 syscall fixture and the verified sibling qa-native-i386 helper"]
fn original_elf32_q3_exports_use_the_shared_session_and_call_table() {
    let fixture = PathBuf::from(
        std::env::var_os("QA_NATIVE_Q3_ELF32_FIXTURE")
            .expect("explicit compiled qagame32.so fixture"),
    );
    original_native_q3_exports_use_the_shared_session_and_call_table(&fixture, 32);
}

pub(crate) fn original_native_q3_exports_use_the_shared_session_and_call_table(
    fixture: &std::path::Path,
    bits: u8,
) {
    let bytes = std::fs::read(&fixture).unwrap();
    let image = Image::parse(&bytes, Some(0x2000_0000), LoadRole::Library).unwrap();
    assert_eq!(image.target.bits, u32::from(bits));
    let entry = symbol(&image, b"dllEntry");
    let passfloat = symbol(&image, b"qa_fixture_passfloat");
    let layout = symbol(&image, b"qa_q3_layout");
    let files = Files(std::env::temp_dir().join(format!("qa-elf32-q3-{}", std::process::id())));
    std::fs::create_dir(&files.0).unwrap();
    std::fs::write(files.0.join("qagame32.so"), &bytes).unwrap();
    std::fs::write(files.0.join("fixture.dat"), b"0123456789").unwrap();
    let mut runtime = Runtime::load(2, std::iter::empty()).unwrap();
    assert_eq!(
        runtime
            .server
            .connect(qa_session::clients::Connection::Remote, ModuleId(2), None),
        Some(ClientId(0))
    );
    let id = runtime
        .server
        .connect(
            qa_session::clients::Connection::Local,
            ModuleId(1),
            Some(NativeEntity {
                module: ModuleId(1),
                slot: 0,
            }),
        )
        .unwrap();
    assert_eq!(id, ClientId(1));
    let player = &mut runtime.server.clients[id.0 as usize].player;
    player.movement_rules = RuleSetId::QuakeWorld;
    player.trace_rules = RuleSetId::Quake2;
    player.body.position = Vec3([123.25, -99.5, 40.0]);
    player.body.velocity = Vec3([-22.5, 66.25, 3.0]);
    player.view_angles = Vec3([10.5, 270.25, -8.0]);
    let pose = (
        [
            player.body.position,
            player.body.velocity,
            player.body.mins,
            player.body.maxs,
        ],
        player.view_angles,
        player.movement.tuning,
    );
    runtime.vfs.mount_directory(&files.0, 0).unwrap();
    runtime
        .geometry
        .load_brushes(
            vec![],
            vec![],
            Vec::new(),
            qa_world::collision::surfaces::SurfaceTable::flags(vec![]),
            qa_world::collision::brushes::BrushTree::direct(0).unwrap(),
            vec![qa_core::primitives::Bounds {
                mins: Vec3([-10.0; 3]),
                maxs: Vec3([10.0; 3]),
            }],
        )
        .unwrap();
    runtime.collision = Some(visibility_world::collision(
        &runtime.geometry,
        [Some(0), Some(1)],
        qa_world::visibility::PvsRows::load(vec![Some(0), Some(1)], vec![3, 3]).unwrap(),
    ));
    let mut requests = Vec::new();
    load_q3(
        &mut runtime,
        &[Q3Spec::parse("game:qagame32.so").unwrap()],
        &mut requests,
        TickRate::fixed(50).unwrap(),
        None,
    )
    .unwrap();
    let Program::Native { vm, .. } = &mut requests[0].program else {
        panic!("native program")
    };
    if bits == 32 {
        assert!(matches!(
            vm.declare_table(ReturnedTable {
                version: 3,
                bytes: 16,
                functions: vec![TableFunction {
                    offset: 8,
                    parameters: &[],
                    result: NativeScalar::Void,
                }]
                .into_boxed_slice(),
            }),
            Err(Error::Process(
                qa_platform::native::NativeError::Unsupported
            ))
        ));
    }
    let base = vm.process.base();
    {
        let memory = ModuleMemory::borrow(base, vm.process.memory_mut().unwrap()).unwrap();
        let expected = [
            u32::from(bits / 8),
            u32::from(bits / 8),
            4,
            4,
            272,
            0,
            4,
            8,
            12,
            16,
            56,
            468,
            208,
            24,
            516,
        ];
        for (index, value) in expected.into_iter().enumerate() {
            assert_eq!(
                memory
                    .read_field(layout + (index * 4) as u64, Width::Word)
                    .unwrap(),
                value
            );
        }
        for (index, value) in [(36, 184), (37, 248), (38, 4), (39, 16), (40, 16)] {
            assert_eq!(memory.read_word(layout + index * 4).unwrap(), value);
        }
        for (index, value) in [0, 4, 8, 12, 24, 44, 48, 52, 23, 24, 32, 33]
            .into_iter()
            .enumerate()
        {
            assert_eq!(
                memory.read_word(layout + (41 + index as u64) * 4).unwrap(),
                value
            );
        }
    }
    if bits == 32 {
        let memory = ModuleMemory::borrow(base, vm.process.memory_mut().unwrap()).unwrap();
        assert_eq!(memory.read_word(layout + 15 * 4).unwrap(), 808);
        assert_eq!(memory.read_word(layout + 16 * 4).unwrap(), 776);
    }
    // Observe the actual original PASSFLOAT syscall words through the same gate.
    // Session execution below uses the production Q3 CallTable for services.
    let entry = vm
        .process
        .bind(
            entry,
            NativeAbi::SystemV,
            &[NativeScalar::Word],
            NativeScalar::Void,
        )
        .unwrap();
    let mut arguments = [0; 13];
    arguments[0] = vm.import_callback();
    vm.process
        .invoke::<()>(entry, arguments, |_, _, _| {
            panic!("dllEntry must only store the callback")
        })
        .unwrap();
    let probe = vm
        .process
        .bind(passfloat, NativeAbi::SystemV, &[], NativeScalar::I32)
        .unwrap();
    let mut calls = 0;
    let result = vm
        .process
        .invoke::<()>(probe, [0; 13], |request, _, _| {
            let NativeRequest::Import(call) = request else {
                panic!("scalar fixture unexpectedly resumed a callback")
            };
            calls += 1;
            assert_eq!(call.number, 205);
            assert_eq!(call.arguments[0] as u32, (-1.25_f32).to_bits());
            Ok(NativeAction::Return(0x31415926))
        })
        .unwrap();
    assert_eq!(calls, 1);
    assert_eq!(result, 0x31415926);

    let mut host = FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        requests,
    )
    .unwrap();
    host.initialize_modules(Phase::Server);
    assert_eq!(host.module_state(ModuleId(1)), Some(State::Running));
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
    let check_values = |host: &FrameHost, revision: i32| {
        let values = host.runtime.catalog.hud.values.native(RuleSetId::Quake3);
        let client = &host.runtime.server.clients[id.0 as usize];
        let player = &client.player;
        for (ordinal, binding) in values.stats.iter().enumerate() {
            let value = match ordinal {
                0 => 71 + revision,
                3 => 29 + revision,
                14 => i32::MIN,
                15 => i32::MAX,
                _ => -123 - ordinal as i32 - revision,
            };
            assert_eq!(
                player.values.get(binding.id),
                Some(NumericValue::integer(value))
            );
        }
        for (ordinal, binding) in values.persistent.iter().enumerate() {
            let value = if ordinal == 0 {
                -5 + revision
            } else {
                90000 + ordinal as i32 + revision
            };
            assert_eq!(
                player.values.get(binding.id),
                Some(NumericValue::integer(value))
            );
        }
        assert_eq!(
            (player.health, player.armor, player.score),
            (71 + revision, 29 + revision, -5 + revision)
        );
        assert_eq!(
            (
                [
                    player.body.position,
                    player.body.velocity,
                    player.body.mins,
                    player.body.maxs
                ],
                player.view_angles,
                player.movement.tuning
            ),
            pose
        );
        assert_eq!(
            (player.movement_rules, player.trace_rules),
            (RuleSetId::QuakeWorld, RuleSetId::Quake2)
        );
        assert_eq!(
            player.weapon_state,
            Some(qa_core::primitives::WeaponState {
                source: RuleSetId::Quake3,
                namespace: qa_core::primitives::ResourceNamespace::Module(ModuleId(1)),
                readiness: qa_core::primitives::WeaponReadiness::RemainingMilliseconds(-99),
            })
        );
        assert!(!client.module_movement);
        assert_eq!(host.runtime.server.clients[0].player.health, 0);
    };
    check_values(&host, 0);
    let entity = |host: &FrameHost, number| {
        host.runtime
            .server
            .entities
            .active()
            .find(|id| {
                host.runtime.server.entities.columns.native_entity[id.slot as usize]
                    == Some(qa_core::primitives::NativeEntity {
                        module: ModuleId(1),
                        slot: number,
                    })
            })
            .unwrap()
    };
    let zero = entity(&host, 0);
    let first = entity(&host, 1);
    assert_ne!(zero, first);
    assert_eq!(
        host.runtime.server.entities.columns.appearance[first.slot as usize].frame,
        11
    );
    let old_bounds = host.runtime.server.area.bounds(first).unwrap();
    assert_eq!(
        host.runtime.server.entities.columns.point_contents_pose[first.slot as usize],
        Some(qa_core::primitives::EntityPose {
            position: qa_core::primitives::Vec3([1.0, 0.0, 0.0]),
            angles: qa_core::primitives::Vec3([0.0, 45.0, 0.0])
        })
    );
    for (index, time) in [100, 137, 1000].into_iter().enumerate() {
        host.call_module_export(
            ModuleId(1),
            Export::clocked(CallbackId(8)),
            ThinkTime::Milliseconds(time),
        )
        .unwrap();
        let counts = host.module_counts(ModuleId(1)).unwrap();
        // Guest validates exact cvars, negative seek, file tail, startup vectors,
        // PASSFLOAT results and constructor order before returning this count.
        assert_eq!(
            counts.last_result,
            Some(ModuleResult::Native(0x13570001 + index as u64))
        );
        assert_eq!(counts.traps, 0);
        assert_eq!(counts.rejected, 0);
        check_values(&host, index as i32 + 1);
        match index {
            0 => {
                assert!(host.runtime.server.entities.resolve(zero).is_some());
                assert!(host.runtime.server.area.bounds(zero).is_none());
                assert_eq!(
                    host.runtime.server.entities.columns.appearance[zero.slot as usize].models,
                    [None; 4]
                );
                assert_eq!(
                    host.runtime.server.entities.columns.position[first.slot as usize],
                    qa_core::primitives::Vec3([3.0, 0.0, 0.0])
                );
                assert_eq!(host.runtime.server.area.bounds(first), Some(old_bounds));
                assert_eq!(
                    host.runtime.server.entities.columns.appearance[first.slot as usize].frame,
                    12
                );
            }
            1 => {
                assert!(host.runtime.server.entities.resolve(first).is_none());
                assert_ne!(entity(&host, 1), first);
                assert_eq!(
                    host.runtime.server.entities.columns.appearance[entity(&host, 1).slot as usize]
                        .frame,
                    13
                );
                assert!(host.runtime.server.entities.active().all(|id| {
                    host.runtime.server.entities.columns.native_entity[id.slot as usize]
                        != Some(qa_core::primitives::NativeEntity {
                            module: ModuleId(1),
                            slot: 64,
                        })
                }));
            }
            _ => {
                assert_eq!(entity(&host, 0), zero);
                assert_eq!(
                    host.runtime.server.entities.columns.position[zero.slot as usize],
                    qa_core::primitives::Vec3([-3.0, 0.0, 0.0])
                );
                assert!(host.runtime.server.area.bounds(zero).is_some());
            }
        }
    }
    let calls = host.module_counts(ModuleId(1)).unwrap().calls;
    host.shutdown_modules();
    let counts = host.module_counts(ModuleId(1)).unwrap();
    assert_eq!(host.module_state(ModuleId(1)), Some(State::Stopped));
    assert_eq!(counts.calls, calls + 5); // GAME_SHUTDOWN, three array entries, DT_FINI.
    assert_eq!(counts.traps, 0); // finalizer bodies verify reverse order.
    assert_eq!(counts.rejected, 0);
    let calls = counts.calls;
    host.shutdown_modules();
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, calls);
}

#[test]
#[ignore = "requires the same compiled ELF32 original Q3 fixture"]
fn elf32_binding_rejects_x64_and_unverified_hardening_versions() {
    use qa_compat::native::{NamedExport, Vm};
    use qa_core::{names::NameTable, primitives::NameId};
    use std::time::Duration;
    let fixture = PathBuf::from(
        std::env::var_os("QA_NATIVE_Q3_ELF32_FIXTURE")
            .expect("explicit compiled qagame32.so fixture"),
    );
    let bytes = std::fs::read(&fixture).unwrap();
    for version in [b"GLIBC_2.2.5".as_slice(), b"GLIBC_2.99"] {
        let mut image = Image::parse(&bytes, Some(0x2000_0000), LoadRole::Library).unwrap();
        image.names = NameTable::load_reserved(
            (0..image.names.len()).map(|i| image.names.get(NameId(i as u32)).unwrap()),
            1,
            version.len(),
        )
        .unwrap();
        let version = image.names.intern(version).unwrap();
        let index = image
            .symbols
            .iter()
            .position(|s| s.name.and_then(|id| image.names.get(id)) == Some(b"strlen"))
            .unwrap();
        image.symbols[index].version.as_mut().unwrap().name = version;
        let error = match Vm::map_image(
            image,
            &[NamedExport {
                name: b"vmMain",
                command: Some(0),
                parameters: &[NativeScalar::Word; 13],
                result: NativeScalar::Word,
            }],
            &[],
            Duration::from_secs(3),
        ) {
            Err(Error::Binding(error)) => error,
            _ => panic!("foreign width/version must reject before launch"),
        };
        assert!(error.contains("strlen@GLIBC_2."));
    }
    let bytes = std::fs::read(fixture.with_file_name("qagame32-host-hardening.so")).unwrap();
    let image = Image::parse(&bytes, Some(0x2000_0000), LoadRole::Library).unwrap();
    let error = match Vm::map_image(
        image,
        &[NamedExport {
            name: b"vmMain",
            command: Some(0),
            parameters: &[NativeScalar::Word; 13],
            result: NativeScalar::Word,
        }],
        &[],
        Duration::from_secs(3),
    ) {
        Err(Error::Binding(error)) => error,
        _ => panic!("unverified hardening import must reject before launch"),
    };
    assert!(error.contains("__stack_chk_fail@GLIBC_2.4"));
}
