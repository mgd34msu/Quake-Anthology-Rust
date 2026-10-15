use super::*;

pub(super) fn commands_move_a_native_client_once() {
    use qa_app::{
        WorldCollision,
        client_policy::ClientPolicy,
        map::SpawnAnchor,
        modules::{ApiCheck, Q2Clients},
    };
    use qa_compat::{memory::ModuleMemory, native::q2::Game};
    use qa_core::{
        primitives::{Bounds, MovementMode, PlayerState, UserCmd, Vec3},
        sys_events::SeatId,
    };
    use qa_network::commands::packet::Protocol;
    use qa_world::collision::WorldTrace;
    for (rr, raw_255_ms) in [(false, false), (false, true), (true, false)] {
        let rules = if rr {
            RuleSetId::Quake2Rerelease
        } else {
            RuleSetId::Quake2
        };
        let protocol = if rr {
            Protocol::Quake2Repro1038
        } else {
            Protocol::Quake2_34
        };
        let stride = if rr { 1472 } else { 280 };
        let edict = 0x180003000 + stride;
        let pm = 0x180007000u64;
        let ps = 0x180006000u64;
        let mut file = q2_table_file();
        pe_rva(&mut file, 0x1800, if rr { 2023 } else { 3 }, 4);
        pe_text(&mut file, 0x1700, &[0xc3]);
        pe_text(&mut file, 0x1710, &[0x0f, 0x0b, 0x90, 0x90, 0x90]); // Original movement must never execute.
        pe_text(&mut file, 0x1720, &[0xb8, 1, 0, 0, 0, 0xc3]);
        for index in 0..if rr { 29 } else { 15 } {
            let offset = if !rr || index < 19 {
                8 + index * 8
            } else {
                192 + (index - 19) * 8
            };
            pe_rva(
                &mut file,
                0x1800 + offset,
                if offset == 192 && rr {
                    0x180001710
                } else {
                    0x180001700
                },
                8,
            );
        }
        let init_offset = if rr { 16 } else { 8 };
        pe_rva(&mut file, 0x1800 + init_offset, 0x180001500, 8);
        pe_rva(&mut file, 0x1800 + if rr { 88 } else { 64 }, 0x180001720, 8);
        pe_rva(
            &mut file,
            0x1800 + if rr { 128 } else { 104 },
            0x180001200,
            8,
        );
        let metadata = 0x1800 + if rr { 160 } else { 128 };
        pe_rva(&mut file, metadata, 0x180003000, 8);
        pe_rva(&mut file, metadata + 8, stride, if rr { 8 } else { 4 });
        pe_rva(&mut file, metadata + if rr { 16 } else { 12 }, 2, 4);
        pe_rva(&mut file, metadata + if rr { 20 } else { 16 }, 2, 4);
        let mut image = Image::parse(&file, None, LoadRole::Library).unwrap();
        let mut bytes = std::mem::take(&mut image.bytes).into_vec();
        bytes.resize(0x9000, 0);
        let at = (edict - 0x180000000) as usize + if rr { 120 } else { 88 };
        bytes[at..at + 8].copy_from_slice(&ps.to_le_bytes());
        let mode = if rr {
            2i32
        } else if raw_255_ms {
            0
        } else {
            1
        };
        bytes[0x7000..0x7004].copy_from_slice(&mode.to_le_bytes());
        let gravity = if rr { 32 } else { 18 };
        bytes[0x7000 + gravity..0x7002 + gravity]
            .copy_from_slice(&(if raw_255_ms { 0i16 } else { 800 }).to_le_bytes());
        if raw_255_ms {
            // Actual classic pmove_state_t: origin and velocity are signed
            // eighth units. Stay airborne in the empty world with no gravity.
            bytes[0x7008..0x700a].copy_from_slice(&800i16.to_le_bytes());
            bytes[0x700a..0x700c].copy_from_slice(&800i16.to_le_bytes());
        }
        if rr {
            bytes[0x701c..0x701e].copy_from_slice(&0x4000u16.to_le_bytes());
            bytes[0x701e..0x7020].copy_from_slice(&500u16.to_le_bytes());
            // Free movement must publish fresh native outputs, even when
            // the previous call left nonzero result bytes in this buffer.
            bytes[0x7000 + 3309..0x7000 + 3311].fill(0xff);
            bytes[0x7000 + 3312..0x7000 + 3316].copy_from_slice(&99.0f32.to_le_bytes());
        } else if !raw_255_ms {
            // Classic spectator retains ON_GROUND in pm_flags but publishes
            // a null groundentity. The boundary must preserve both facts.
            bytes[0x7010] = 4;
        }
        let delta_yaw = if rr { 40 } else { 22 };
        let delta = if rr {
            90.0f32.to_bits().to_le_bytes().to_vec()
        } else {
            qa_core::math::angle_to_short(90.0, qa_core::math::AngleShortForm::Factored)
                .to_le_bytes()
                .to_vec()
        };
        bytes[0x7000 + delta_yaw..0x7000 + delta_yaw + delta.len()].copy_from_slice(&delta);
        let state_bytes = if rr { 52 } else { 28 };
        let state = bytes[0x7000..0x7000 + state_bytes].to_vec();
        bytes[0x6000..0x6000 + state_bytes].copy_from_slice(&state);
        let view_yaw = if rr { 56 } else { 32 };
        bytes[0x6000 + view_yaw..0x6004 + view_yaw].copy_from_slice(&90.0f32.to_le_bytes());
        if rr {
            bytes[0x7000 + 3248..0x7000 + 3256].copy_from_slice(&edict.to_le_bytes());
        }
        image.bytes = bytes.into_boxed_slice();
        let mut regions = std::mem::take(&mut image.regions).into_vec();
        regions.push(qa_formats::program::native::Region {
            offset: 0x3000,
            length: 0x6000,
            read: true,
            write: true,
            execute: false,
        });
        image.regions = regions.into_boxed_slice();
        let mut runtime = Runtime::load(2, []).unwrap();
        let bounds = Bounds {
            mins: Vec3([-1000.0; 3]),
            maxs: Vec3([1000.0; 3]),
        };
        let geometry = runtime
            .geometry
            .load_brushes(
                vec![],
                vec![],
                qa_world::collision::surfaces::SurfaceTable::flags(vec![]),
                qa_world::collision::brushes::BrushTree {
                    planes: vec![],
                    nodes: vec![],
                    leaves: vec![qa_world::collision::brushes::CollisionLeaf {
                        stored_contents: None,
                        brushes: qa_core::primitives::IndexSpan { first: 0, count: 0 },
                    }],
                    leaf_brushes: vec![],
                    models: vec![qa_world::collision::brushes::ModelRoot::Leaf(0)],
                },
                vec![bounds],
            )
            .unwrap();
        runtime.collision = Some(WorldCollision::new(&runtime.geometry, geometry, 0));
        let id = runtime
            .connect_local(
                SeatId::FIRST,
                SpawnAnchor {
                    position: Vec3::default(),
                    angles: Vec3([0.0, 90.0, 0.0]),
                    entity: 0,
                    fixture_fallback: false,
                },
                ClientPolicy {
                    input: rules,
                    client: rules,
                    movement: rules,
                    trace: RuleSetId::Quake3,
                },
                protocol,
                0,
            )
            .unwrap();
        let mut game =
            Game::map(image, rules, 25, &runtime.geometry, Duration::from_secs(3)).unwrap();
        let mut init = vec![0x48, 0x83, 0xec, 0x28, 0x48, 0xb9];
        init.extend(0x180001b40u64.to_le_bytes());
        init.extend([0x48, 0xb8]);
        init.extend(
            game.vm
                .process
                .import_pointer(if rr { 11 } else { 9 })
                .unwrap()
                .to_le_bytes(),
        );
        init.extend([0xff, 0xd0, 0x48, 0x83, 0xc4, 0x28, 0xc3]);
        let precache = Export::words(CallbackId(game.entry(b"Init").unwrap()), &[]);
        let configstrings = game
            .vm
            .resource_ranges()
            .unwrap()
            .iter()
            .map(|range| range.range().end)
            .max()
            .unwrap();
        {
            let mut memory = ModuleMemory::borrow(
                game.vm.process.base(),
                game.vm.process.memory_mut().unwrap(),
            )
            .unwrap();
            memory.write(0x180001500, &init).unwrap();
            memory.write(0x180001b40, b"_qa_cue.wav\0").unwrap();
        }
        let api = Q2Clients {
            module: ModuleId(1),
            rules,
            connect: CallbackId(game.entry(b"ClientConnect").unwrap()),
            begin: CallbackId(game.entry(b"ClientBegin").unwrap()),
            disconnect: CallbackId(game.entry(b"ClientDisconnect").unwrap()),
            think: CallbackId(game.entry(b"ClientThink").unwrap()),
            command: CallbackId(game.entry(b"ClientCommand").unwrap()),
            userinfo_changed: CallbackId(game.entry(b"ClientUserinfoChanged").unwrap()),
            userinfo: 0x180004000,
            usercmd: 0x180004c00,
            abi: qa_compat::q2_abi::client_layout(rules).unwrap(),
            social: 0x180004800,
        };
        // This small owned module copies the received usercmd, calls its native
        // Pmove boundary once, then publishes exactly the resulting state.
        let mut code = vec![0x48, 0x83, 0xec, 0x28, 0x48, 0xb8];
        code.extend(0x180006f00u64.to_le_bytes());
        code.extend([0xff, 0x00, 0x49, 0xbb]);
        code.extend((pm + if rr { 52 } else { 28 }).to_le_bytes());
        for offset in (0..api.abi.command_bytes).step_by(4) {
            code.extend([0x8b, 0x42, offset as u8, 0x41, 0x89, 0x43, offset as u8]);
        }
        code.extend([0x48, 0xb9]);
        code.extend(pm.to_le_bytes());
        code.extend([0x48, 0xb8]);
        code.extend(
            if rr {
                0x180001710u64
            } else {
                game.vm.process.import_pointer(21).unwrap()
            }
            .to_le_bytes(),
        );
        code.extend([0xff, 0xd0, 0x49, 0xbb]);
        code.extend(pm.to_le_bytes());
        if rr {
            // Flags clear, while native pm_time survives the early return.
            code.extend([0x66, 0x41, 0x83, 0x7b, 28, 0, 0x74, 2, 0x0f, 0x0b]);
            code.extend([0x66, 0x41, 0x81, 0x7b, 30, 0xf4, 1, 0x74, 2, 0x0f, 0x0b]);
            code.extend([0x66, 0x41, 0x83, 0xbb]);
            code.extend(3309u32.to_le_bytes());
            code.extend([0, 0x74, 2, 0x0f, 0x0b]);
            code.extend([0x41, 0x83, 0xbb]);
            code.extend(3312u32.to_le_bytes());
            code.extend([0, 0x74, 2, 0x0f, 0x0b]);
        } else {
            code.extend([
                0x41,
                0x80,
                0x7b,
                16,
                if raw_255_ms { 0 } else { 4 },
                0x74,
                2,
                0x0f,
                0x0b,
            ]);
            code.extend([0x49, 0x83, 0xbb]);
            code.extend(352u32.to_le_bytes());
            code.extend([0, 0x74, 2, 0x0f, 0x0b]);
            if raw_255_ms {
                // A native msec byte of 255 means 0.255 seconds to gi.Pmove.
                // The child checks the exact native origin, independently of
                // the engine's prepare_command function: 100 * .255 * 8=204.
                code.extend([0x66, 0x41, 0x81, 0x7b, 4, 204, 0, 0x74, 2, 0x0f, 0x0b]);
            }
        }
        code.extend([0x48, 0xb9]);
        code.extend(ps.to_le_bytes());
        for offset in (0..state_bytes).step_by(4) {
            code.extend([0x41, 0x8b, 0x43, offset as u8, 0x89, 0x41, offset as u8]);
        }
        let view = if rr { 3168 } else { 312 };
        let ps_view = if rr { 52 } else { 28 };
        for i in 0..3 {
            code.extend([0x41, 0x8b, 0x83]);
            code.extend((view + i * 4i32).to_le_bytes());
            code.extend([0x89, 0x41, (ps_view + i * 4) as u8]);
        }
        code.extend([0x48, 0xb9]);
        code.extend(edict.to_le_bytes());
        for i in 0..3 {
            if rr {
                code.extend([
                    0x41,
                    0x8b,
                    0x43,
                    (4 + i * 4) as u8,
                    0x89,
                    0x41,
                    (4 + i * 4) as u8,
                ]);
            } else {
                code.extend([
                    0x41,
                    0x0f,
                    0xbf,
                    0x43,
                    (4 + i * 2) as u8,
                    0xf3,
                    0x0f,
                    0x2a,
                    0xc0,
                    0xf3,
                    0x0f,
                    0x59,
                    0x05,
                ]);
                code.extend(
                    ((0x180001af0i64 - (0x180001200 + code.len() as i64 + 4)) as i32).to_le_bytes(),
                );
                code.extend([0xf3, 0x0f, 0x11, 0x41, (4 + i * 4) as u8]);
            }
        }
        // A module may update an unlinked client's body before a link import.
        for (source, target) in [
            (if rr { 3180 } else { 328 }, if rr { 1396 } else { 204 }),
            (if rr { 3192 } else { 340 }, if rr { 1408 } else { 216 }),
        ] {
            for axis in 0..3i32 {
                code.extend([0x41, 0x8b, 0x83]);
                code.extend((source + axis * 4).to_le_bytes());
                code.extend([0x89, 0x81]);
                code.extend((target + axis * 4).to_le_bytes());
            }
        }
        code.extend([0x48, 0x83, 0xc4, 0x28, 0xc3]);
        assert!(code.len() < 0x300);
        {
            let mut memory = ModuleMemory::borrow(
                game.vm.process.base(),
                game.vm.process.memory_mut().unwrap(),
            )
            .unwrap();
            memory.write(0x180001200, &code).unwrap();
            memory
                .write_word(0x180001af0, 0.125f32.to_bits() as i32)
                .unwrap();
            assert_eq!(
                memory.read_u64(0x180001800 + init_offset as u64).unwrap(),
                0x180001500
            );
            assert_eq!(
                memory.read(0x180001500, init.len()).unwrap(),
                init.as_slice()
            );
            assert_eq!(memory.read(0x180001b40, 12).unwrap(), b"_qa_cue.wav\0");
        }
        let imports = game.imports;
        let imports_address = game.imports_address;
        let count = game.vm.export_count();
        let mut request = request(
            &mut runtime,
            ModuleId(1),
            rules,
            TickRate::FrameDriven,
            game.vm,
        );
        if let Program::Native { imports: table, .. } = &mut request.program {
            *table = imports;
        }
        request.entries = (0..count as u32).collect();
        request.initialize = vec![precache];
        request.configstrings = configstrings;
        request.api = Some(ApiCheck::NativeTable {
            export: Export::words(CallbackId(0), &[imports_address]),
        });
        let context = request.context.console;
        let mut host = FrameHost::load_modules(
            Console::new(Context::default()).unwrap(),
            runtime,
            TickRate::FrameDriven,
            vec![request],
        )
        .unwrap();
        let maxclients = host.console.cvars.bind("sv_maxclients", context).unwrap();
        host.console.cvars.force_write(maxclients, "1").unwrap();
        host.initialize_modules(Phase::Server);
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 2);
        let bank = std::sync::Arc::new(
            qa_audio::Bank::load(
                std::num::NonZeroU32::new(44100).unwrap(),
                std::iter::once("sound/_qa_cue.wav")
                    .chain(qa_gameplay::rules::q2::sounds::PATHS.iter().copied())
                    .map(|path| {
                        (
                            path.into(),
                            qa_core::primitives::Pcm {
                                rate: std::num::NonZeroU32::new(44100).unwrap(),
                                channels: qa_core::primitives::PcmChannels::Mono,
                                samples: vec![12000; 22050],
                                loop_start: None,
                                loop_end: None,
                                loop_invalid: false,
                            },
                        )
                    })
                    .collect(),
            )
            .unwrap(),
        );
        host.bind_sounds(&bank).unwrap();
        let other_seat = SeatId::new(1).unwrap();
        let other_angles = Vec3([10.0, 217.0, -5.0]);
        host.runtime.input.set_view_angles(other_seat, other_angles);
        {
            host.register_q2_client_api(api).unwrap();
            host.connect_module_client(api.module, id, b"\\name\\once")
                .unwrap()
        };
        let input_handles =
            qa_console::input_settings::InputHandles::load(&host.console.cvars).unwrap();
        let initial = host.runtime.input.build_frame_with_policy(
            EventTime::default(),
            &[input_handles.policy(&host.console.cvars, qa_core::primitives::RuleSetId::Quake3);
                qa_core::sys_events::SeatId::COUNT],
        );
        assert_eq!(initial[0].view_angles, Vec3::default());
        assert_eq!(initial[1].view_angles, other_angles);
        assert_eq!(
            host.runtime.server.clients[id.0 as usize]
                .player
                .view_angles,
            Vec3([0.0, 90.0, 0.0])
        );
        let mut expected = PlayerState::default();
        expected.movement_rules = rules;
        expected.trace_rules = RuleSetId::Quake3;
        expected.movement.mode = if rr {
            MovementMode::Noclip
        } else if raw_255_ms {
            MovementMode::Walk
        } else {
            MovementMode::Spectator
        };
        expected.movement.tuning.gravity = Some((if raw_255_ms { 0.0 } else { 800.0 }).into());
        expected.movement.grounded = !rr && !raw_255_ms;
        expected.movement.remaining_ms = if rr { 500 } else { 0 };
        expected.movement.delta_angles = Vec3([0.0, 90.0, 0.0]).into();
        let command = UserCmd {
            duration_ms: if raw_255_ms { 255 } else { 16 },
            movement: if raw_255_ms {
                [0.0; 3]
            } else {
                [200.0, 0.0, 0.0]
            },
            units: qa_gameplay::rules::command_units(rules),
            ..Default::default()
        };
        if raw_255_ms {
            expected.body.position = Vec3([25.5, 0.0, 100.0]);
            expected.body.velocity = Vec3([100.0, 0.0, 0.0]);
            expected.view_angles = Vec3([0.0, 90.0, 0.0]);
        }
        let initial_calls = host.module_counts(ModuleId(1)).unwrap().calls;
        for calls in 1..=if raw_255_ms { 1 } else { 2 } {
            let mut scratch = host.runtime.geometry.scratch();
            let mut trace = WorldTrace::new(
                &host.runtime.geometry,
                geometry,
                0,
                &host.runtime.server.entities,
                &host.runtime.server.area,
                &mut scratch,
                qa_core::primitives::CollisionOwner::Lifetime(
                    host.runtime.server.clients[id.0 as usize].entity,
                ),
            );
            if !raw_255_ms {
                qa_movement::pmove(
                    qa_movement::prepare_command(rules, command),
                    &mut expected,
                    &mut trace,
                    Default::default(),
                    qa_movement::no_boundary,
                )
                .unwrap();
            }
            assert!(host.runtime.send_local_command(
                id,
                &command,
                EventTime::from_milliseconds(calls * 16)
            ));
            host.drain(&mut Default::default());
            let client = &host.runtime.server.clients[id.0 as usize];
            assert_eq!(
                client.player.body.position, expected.body.position,
                "{rules:?} command {calls}"
            );
            assert_eq!(client.player.body.velocity, expected.body.velocity);
            assert_eq!(client.player.view_angles, expected.view_angles);
            assert_eq!(client.player.view_angles.0[1], 90.0);
            if !raw_255_ms {
                assert_eq!(client.player.body.mins, expected.body.mins);
                assert_eq!(client.player.body.maxs, expected.body.maxs);
            }
            let world = host.runtime.collision.as_mut().unwrap();
            assert_eq!(
                host.runtime.server.move_pending_clients(
                    &host.runtime.geometry,
                    world.geometry,
                    0,
                    &mut world.scratch,
                    None,
                    None,
                    |_, error| panic!("movement failed: {error:?}"),
                ),
                0
            );
            assert_eq!(
                host.runtime.server.clients[id.0 as usize]
                    .player
                    .body
                    .position,
                expected.body.position
            );
            assert_eq!(
                host.module_counts(ModuleId(1)).unwrap().calls,
                initial_calls + calls
            );
        }
        if raw_255_ms {
            assert_eq!(host.runtime.network.commands, 1);
            host.shutdown_modules();
            continue;
        }
        assert_eq!(host.runtime.network.commands, 2);
        // Both packets share one event drain. Module-owned clients must consume
        // each command once, without leaving another engine movement pending.
        for frame in 3..=4 {
            assert!(host.runtime.send_local_command(
                id,
                &command,
                EventTime::from_milliseconds(frame * 16)
            ));
        }
        host.drain(&mut Default::default());
        assert_eq!(host.runtime.network.commands, 4);
        assert_eq!(
            host.module_counts(ModuleId(1)).unwrap().calls,
            initial_calls + 4
        );
        let position = host.runtime.server.clients[id.0 as usize]
            .player
            .body
            .position;
        let world = host.runtime.collision.as_mut().unwrap();
        assert_eq!(
            host.runtime.server.move_pending_clients(
                &host.runtime.geometry,
                world.geometry,
                0,
                &mut world.scratch,
                None,
                None,
                |_, error| panic!("movement failed: {error:?}"),
            ),
            0
        );
        assert_eq!(
            host.runtime.server.clients[id.0 as usize]
                .player
                .body
                .position,
            position
        );
        super::cues::received_sounds_use_precache_and_the_presentation_ring(
            &mut host,
            id,
            std::sync::Arc::clone(&bank),
        );
        super::cues::muzzle_packets_publish_product_sounds(&mut host, id, bank);
        host.shutdown_modules();
    }
}
