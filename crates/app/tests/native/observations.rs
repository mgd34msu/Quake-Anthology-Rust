use super::*;

pub(super) fn snapshot_frames_follow_native_provider_lifetimes() {
    for protocol in [
        qa_network::commands::packet::Protocol::Quake2Repro1038,
        qa_network::commands::packet::Protocol::Quake2_34,
    ] {
        snapshot_frames_for_protocol(protocol);
    }
}
fn snapshot_frames_for_protocol(protocol: qa_network::commands::packet::Protocol) {
    use qa_app::{
        client_policy::ClientPolicy,
        map::SpawnAnchor,
        modules::{ApiCheck, Q2Clients},
    };
    use qa_compat::native::q2::Game;
    use qa_core::{loopback::Endpoint, primitives::Vec3, sys_events::SeatId};
    use qa_network::commands::packet::{self, Key, Move, Protocol, ZERO_Q2, ZERO_Q2_RR};
    let (old_sequence, latest_sequence, stale_sequence) = if protocol == Protocol::Quake2Repro1038 {
        (46, 48, 50)
    } else {
        (23, 24, 25)
    };
    let movement = |sequence| {
        if protocol == Protocol::Quake2Repro1038 {
            Move::Quake2Repro {
                last_frame: sequence,
                commands: [ZERO_Q2_RR; 3],
            }
        } else {
            Move::Quake2 {
                last_frame: sequence,
                commands: [ZERO_Q2; 3],
            }
        }
    };
    let mut runtime = Runtime::load(2, []).unwrap();
    let mut requests = Vec::new();
    let mut clients = Vec::new();
    let mut lifetimes = Vec::new();
    let mut apis = Vec::new();
    for (index, period) in [25, 100].into_iter().enumerate() {
        let id = runtime
            .connect_local(
                SeatId::ALL[index],
                SpawnAnchor {
                    position: Vec3::default(),
                    angles: Vec3::default(),
                    entity: 0,
                    fixture_fallback: false,
                },
                ClientPolicy {
                    client: RuleSetId::Quake3,
                    movement: RuleSetId::Quake,
                    trace: RuleSetId::Quake3,
                },
                protocol,
                0,
            )
            .unwrap();
        for endpoint in [Endpoint::Client, Endpoint::Server]
            .into_iter()
            .filter(|_| protocol == Protocol::Quake2Repro1038)
        {
            runtime
                .network
                .get_mut(id, endpoint)
                .unwrap()
                .channel
                .configure_q2_repro(50, Default::default())
                .unwrap();
        }
        clients.push(id);
        let module = ModuleId(index as u16 + 1);
        let mut file = q2_table_file();
        pe_rva(&mut file, 0x1800, 2023, 4);
        pe_text(&mut file, 0x1700, &[0xc3]);
        pe_text(&mut file, 0x1710, &[0xc3, 0x90, 0x90, 0x90, 0x90]);
        pe_text(&mut file, 0x1720, &[0xb8, 1, 0, 0, 0, 0xc3]);
        // Retain the real import table; ClientThink projects the historical
        // command frame into health and current gi.ServerFrame into armor.
        let mut api_code = vec![0x48, 0x89, 0x0d];
        api_code.extend((0x1b20i32 - 0x1087).to_le_bytes());
        api_code.extend([0x48, 0x8d, 0x05]);
        api_code.extend((0x1800i32 - 0x108e).to_le_bytes());
        api_code.push(0xc3);
        pe_text(&mut file, 0x1080, &api_code);
        let mut think = vec![0x48, 0x83, 0xec, 0x28, 0x8b, 0x42, 24, 0x48, 0xb9];
        think.extend(0x1800060a8u64.to_le_bytes());
        think.extend([0x66, 0x89, 0x01, 0x48, 0x8b, 0x05]);
        let next = 0x1200 + think.len() + 4;
        think.extend((0x1b20i32 - next as i32).to_le_bytes());
        think.extend([0x48, 0x8b, 0x80]);
        think.extend((16i32 + 65 * 8).to_le_bytes());
        think.extend([0xff, 0xd0, 0x48, 0xb9]);
        think.extend(0x1800060b0u64.to_le_bytes());
        think.extend([0x66, 0x89, 0x01, 0x48, 0x83, 0xc4, 0x28, 0xc3]);
        pe_text(&mut file, 0x1200, &think);
        for index in 0..29 {
            let offset = if index < 19 {
                8 + index * 8
            } else {
                192 + (index - 19) * 8
            };
            pe_rva(
                &mut file,
                0x1800 + offset,
                if offset == 192 {
                    0x180001710
                } else {
                    0x180001700
                },
                8,
            );
        }
        pe_rva(&mut file, 0x1800 + 88, 0x180001720, 8);
        pe_rva(&mut file, 0x1800 + 128, 0x180001200, 8);
        pe_rva(&mut file, 0x1800 + 160, 0x180003000, 8);
        pe_rva(&mut file, 0x1800 + 168, 1472, 8);
        pe_rva(&mut file, 0x1800 + 176, 2, 4);
        pe_rva(&mut file, 0x1800 + 180, 2, 4);
        let mut image = Image::parse(&file, None, LoadRole::Library).unwrap();
        let mut bytes = std::mem::take(&mut image.bytes).into_vec();
        bytes.resize(0x8000, 0);
        let client_pointer = 0x3000 + 1472 + 120;
        bytes[client_pointer..client_pointer + 8].copy_from_slice(&0x180006000u64.to_le_bytes());
        image.bytes = bytes.into_boxed_slice();
        let mut regions = std::mem::take(&mut image.regions).into_vec();
        regions.push(qa_formats::program::native::Region {
            offset: 0x3000,
            length: 0x5000,
            read: true,
            write: true,
            execute: false,
        });
        image.regions = regions.into_boxed_slice();
        let game = Game::map(
            image,
            RuleSetId::Quake2Rerelease,
            period,
            &runtime.geometry,
            Duration::from_secs(3),
        )
        .unwrap();
        let api = Q2Clients {
            module,
            rules: RuleSetId::Quake2Rerelease,
            connect: CallbackId(game.entry(b"ClientConnect").unwrap()),
            begin: CallbackId(game.entry(b"ClientBegin").unwrap()),
            disconnect: CallbackId(game.entry(b"ClientDisconnect").unwrap()),
            think: CallbackId(game.entry(b"ClientThink").unwrap()),
            command: CallbackId(game.entry(b"ClientCommand").unwrap()),
            userinfo_changed: CallbackId(game.entry(b"ClientUserinfoChanged").unwrap()),
            abi: &qa_compat::q2_abi::RERELEASE,
            userinfo: 0x180005000,
            social: 0x180005800,
            usercmd: 0x180005c00,
        };
        let entries = (0..game.vm.export_count() as u32).collect();
        let imports = game.imports;
        let imports_address = game.imports_address;
        let frame = CallbackId(game.entry(b"RunFrame").unwrap());
        let mut request = request(
            &mut runtime,
            module,
            RuleSetId::Quake2Rerelease,
            TickRate::fixed(period).unwrap(),
            game.vm,
        );
        lifetimes.push(request.anchor);
        if let Program::Native { imports: table, .. } = &mut request.program {
            *table = imports;
        }
        request.entries = entries;
        request.frame = Export::words(frame, &[0]);
        request.api = Some(ApiCheck::NativeTable {
            export: Export::words(CallbackId(0), &[imports_address]),
        });
        requests.push(request);
        apis.push(api);
    }
    let mut host = FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        requests,
    )
    .unwrap();
    host.initialize_modules(Phase::Server);
    let maximum = host
        .console
        .cvars
        .bind(
            "sv_maxclients",
            Context {
                source: RuleSetId::Quake2Rerelease,
                ..Context::default()
            },
        )
        .unwrap();
    host.console.cvars.force_write(maximum, "2").unwrap();
    for (&client, api) in clients.iter().zip(&apis) {
        {
            host.register_q2_client_api(*api).unwrap();
            host.connect_module_client(api.module, client, b"\\name\\observer")
        }
        .unwrap();
    }
    host.local_clients[0] = Some(clients[0]);
    host.local_clients[1] = Some(clients[1]);
    let mut source = Source {
        time: EventTime::from_milliseconds(2200),
        polls: 0,
    };
    for time in [2200, 2250, 2300, 2400] {
        source.time = EventTime::from_milliseconds(time);
        host.frame(&mut source, true);
    }
    for (index, &client) in clients.iter().enumerate() {
        let connection = host.runtime.network.get(client, Endpoint::Server).unwrap();
        let observed = connection
            .channel
            .snapshot_observation(old_sequence as u32)
            .unwrap();
        assert_eq!(observed.module_lifetime, lifetimes[index]);
        assert_eq!(observed.native_frame, [4, 1][index]);
        assert_eq!(observed.time, EventTime::from_milliseconds(2300));
        assert_eq!(
            connection
                .channel
                .snapshot_observation(latest_sequence)
                .unwrap()
                .native_frame,
            [8, 2][index]
        );
    }
    // Drop automatic CLIENT commands so this admission is exactly the delayed
    // explicit packet ACK, not a fresh latest-snapshot command.
    for &client in &clients {
        host.runtime.loopback.clear_client(client);
    }
    for (index, &client) in clients.iter().enumerate() {
        let movement = movement(old_sequence);
        let connection = host
            .runtime
            .network
            .get_mut(client, Endpoint::Client)
            .unwrap();
        let sequence = connection.channel.send_state().sequence;
        let mut bytes = [0; 1400];
        let length = packet::write(&mut bytes, &movement, sequence, Key::default()).unwrap();
        connection
            .channel
            .prepare_move(&bytes[..length], host.time, Some(old_sequence as u32))
            .unwrap()
            .unwrap();
        connection
            .channel
            .submit_with(host.time, |bytes| {
                host.runtime
                    .loopback
                    .send(Endpoint::Client, client, bytes)
                    .is_ok()
            })
            .unwrap()
            .unwrap();
        host.drain(&mut Default::default());
        assert_eq!(
            host.runtime.server.clients[client.0 as usize].player.health,
            [4, 1][index]
        );
        assert_eq!(
            host.runtime.server.clients[client.0 as usize].player.armor,
            [8, 2][index]
        );
    }
    // A retained snapshot stamped with another lifetime cannot borrow the
    // frame number of a replacement module, even when its numeric id matches.
    let client = clients[0];
    let mut stale = host
        .runtime
        .network
        .get(client, Endpoint::Server)
        .unwrap()
        .channel
        .snapshot_observation(latest_sequence)
        .unwrap();
    stale.module_lifetime.generation += 1;
    assert!(host.runtime.send_snapshot(
        client,
        EventTime::from_milliseconds(2500),
        Some(stale),
        &mut source,
        |_, _| None
    ));
    host.runtime.loopback.clear_client(client);
    let connection = host
        .runtime
        .network
        .get_mut(client, Endpoint::Client)
        .unwrap();
    let sequence = connection.channel.send_state().sequence;
    let mut bytes = [0; 1400];
    let length = packet::write(
        &mut bytes,
        &movement(stale_sequence),
        sequence,
        Key::default(),
    )
    .unwrap();
    connection
        .channel
        .prepare_move(&bytes[..length], host.time, Some(stale_sequence as u32))
        .unwrap()
        .unwrap();
    connection
        .channel
        .submit_with(host.time, |bytes| {
            host.runtime
                .loopback
                .send(Endpoint::Client, client, bytes)
                .is_ok()
        })
        .unwrap()
        .unwrap();
    host.drain(&mut Default::default());
    assert_eq!(
        host.runtime.server.clients[client.0 as usize].player.health,
        0
    );
    assert_eq!(
        host.runtime.server.clients[client.0 as usize].player.armor,
        8
    );
    assert_eq!(host.module_counts(apis[0].module).unwrap().traps, 0);
    assert_eq!(host.module_counts(apis[1].module).unwrap().traps, 0);
    host.shutdown_modules();
}
