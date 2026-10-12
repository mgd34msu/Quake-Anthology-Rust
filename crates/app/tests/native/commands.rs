use super::*;

const BASE: u64 = 0x180000000;
const IMPORTS: usize = 0x1b20;
const COMMAND_STATE: usize = 0x1b30;
const THINK_COUNT: usize = 0x1b34;
const COMMAND: &[u8] = b"quit \"a b\" \x80raw";

pub(super) fn packet_commands_keep_native_order_and_borrowed_arguments() {
    for rerelease in [false, true] {
        packet_commands_for_abi(rerelease);
    }
}

fn packet_commands_for_abi(rerelease: bool) {
    use qa_app::{
        client_policy::ClientPolicy,
        map::SpawnAnchor,
        modules::{ApiCheck, Q2Clients},
    };
    use qa_compat::native::q2::Game;
    use qa_core::{loopback::Endpoint, primitives::Vec3, sys_events::SeatId};
    use qa_network::commands::packet::{Move, Protocol, ZERO_Q2, ZERO_Q2_RR};

    let rules = if rerelease {
        RuleSetId::Quake2Rerelease
    } else {
        RuleSetId::Quake2
    };
    let protocol = if rerelease {
        Protocol::Quake2Repro1038
    } else {
        Protocol::Quake2_34
    };
    let mut runtime = Runtime::load(1, []).unwrap();
    let client = runtime
        .connect_local(
            SeatId::ALL[0],
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
    if rerelease {
        for endpoint in [Endpoint::Client, Endpoint::Server] {
            runtime
                .network
                .get_mut(client, endpoint)
                .unwrap()
                .channel
                .configure_q2_repro(50, Default::default())
                .unwrap();
        }
    }

    let stride = if rerelease { 1472 } else { 280 };
    let edict = BASE + 0x3000 + stride as u64;
    let header = if rerelease { 16 } else { 0 };
    let argc = if rerelease { 42 } else { 39 };
    let argv = argc + 1;
    let args = argc + 2;
    let health = BASE + 0x6000 + if rerelease { 168 } else { 122 };
    let mut file = q2_table_file();
    pe_rva(&mut file, 0x1800, if rerelease { 2023 } else { 3 }, 4);
    pe_text(&mut file, 0x1700, &[0xc3]);
    pe_text(&mut file, 0x1710, &[0xc3, 0x90, 0x90, 0x90, 0x90]);
    pe_text(&mut file, 0x1720, &[0xb8, 1, 0, 0, 0, 0xc3]);
    let mut api = vec![0x48, 0x89, 0x0d];
    api.extend((IMPORTS as i32 - 0x1087).to_le_bytes());
    api.extend([0x48, 0x8d, 0x05]);
    api.extend((0x1800i32 - 0x108e).to_le_bytes());
    api.push(0xc3);
    pe_text(&mut file, 0x1080, &api);

    // Each byte is checked in the owned child, through real argc/argv/args
    // imports. Quotes survive in args; argv removes them without recoding 0x80.
    let mut command = vec![0x48, 0x83, 0xec, 0x28];
    expect_edict(&mut command, edict);
    import(&mut command, 0x1200, header, argc);
    expect_eax(&mut command, 3);
    for (index, bytes) in [
        (0, &b"quit\0"[..]),
        (1, &b"a b\0"[..]),
        (2, &b"\x80raw\0"[..]),
    ] {
        command.push(0xb9);
        command.extend((index as u32).to_le_bytes());
        import(&mut command, 0x1200, header, argv);
        expect_string(&mut command, bytes);
    }
    command.push(0xb9);
    command.extend(999u32.to_le_bytes());
    import(&mut command, 0x1200, header, argv);
    expect_string(&mut command, b"\0");
    import(&mut command, 0x1200, header, args);
    expect_string(&mut command, b"\"a b\" \x80raw\0");
    address(&mut command, BASE + COMMAND_STATE as u64);
    command.extend([0x83, 0x00, 11, 0x48, 0x83, 0xc4, 0x28, 0xc3]);
    assert!(command.len() <= 0x200);
    pe_text(&mut file, 0x1200, &command);

    // The next callback must not see a previous command's borrowed arguments.
    // Health samples the command state at the exact native ClientThink position.
    let mut think = vec![0x48, 0x83, 0xec, 0x28];
    expect_edict(&mut think, edict);
    import(&mut think, 0x1400, header, argc);
    expect_eax(&mut think, 0);
    think.extend([0x31, 0xc9]);
    import(&mut think, 0x1400, header, argv);
    expect_string(&mut think, b"\0");
    import(&mut think, 0x1400, header, args);
    expect_string(&mut think, b"\0");
    address(&mut think, BASE + COMMAND_STATE as u64);
    think.extend([0x8b, 0x10, 0x83, 0xfa, 0, 0x7d, 2, 0x0f, 0x0b]);
    address(&mut think, health);
    think.extend([0x66, 0x89, 0x10]);
    address(&mut think, BASE + THINK_COUNT as u64);
    think.extend([0x83, 0x00, 1, 0x48, 0x83, 0xc4, 0x28, 0xc3]);
    assert!(think.len() <= 0x100);
    pe_text(&mut file, 0x1400, &think);

    let mut userinfo = Vec::new();
    expect_edict(&mut userinfo, edict);
    userinfo.extend([0x48, 0x89, 0xd0]);
    expect_string(&mut userinfo, b"\\name\\\x80native\0");
    address(&mut userinfo, BASE + COMMAND_STATE as u64);
    userinfo.extend([0x83, 0x00, 100, 0xc3]);
    assert!(userinfo.len() <= 0x100);
    pe_text(&mut file, 0x1500, &userinfo);

    // Shutdown proves the native disconnect callback actually ran. A malformed
    // tail command or tail ClientThink traps if ingress fails to stop at it.
    let mut disconnect = Vec::new();
    expect_edict(&mut disconnect, edict);
    address(&mut disconnect, BASE + COMMAND_STATE as u64);
    disconnect.extend([0x81, 0x38]);
    disconnect.extend(122u32.to_le_bytes());
    disconnect.extend([0x74, 2, 0x0f, 0x0b, 0xc7, 0x00]);
    disconnect.extend((-100i32).to_le_bytes());
    disconnect.push(0xc3);
    pe_text(&mut file, 0x1600, &disconnect);
    let mut shutdown = Vec::new();
    address(&mut shutdown, BASE + COMMAND_STATE as u64);
    shutdown.extend([0x81, 0x38]);
    shutdown.extend((-100i32).to_le_bytes());
    shutdown.extend([0x74, 2, 0x0f, 0x0b]);
    address(&mut shutdown, BASE + THINK_COUNT as u64);
    shutdown.extend([0x83, 0x38, 2, 0x74, 2, 0x0f, 0x0b, 0xc3]);
    pe_text(&mut file, 0x1740, &shutdown);

    for index in 0..if rerelease { 29 } else { 15 } {
        let offset = if !rerelease || index < 19 {
            8 + index * 8
        } else {
            192 + (index - 19) * 8
        };
        pe_rva(
            &mut file,
            0x1800 + offset,
            BASE + if rerelease && offset == 192 {
                0x1710
            } else {
                0x1700
            },
            8,
        );
    }
    let (
        shutdown_offset,
        connect_offset,
        userinfo_offset,
        disconnect_offset,
        command_offset,
        think_offset,
    ) = if rerelease {
        (24, 88, 104, 112, 120, 128)
    } else {
        (16, 64, 80, 88, 96, 104)
    };
    for (offset, code) in [
        (shutdown_offset, 0x1740),
        (connect_offset, 0x1720),
        (userinfo_offset, 0x1500),
        (disconnect_offset, 0x1600),
        (command_offset, 0x1200),
        (think_offset, 0x1400),
    ] {
        pe_rva(&mut file, 0x1800 + offset, BASE + code, 8);
    }
    let metadata = if rerelease { 160 } else { 128 };
    pe_rva(&mut file, 0x1800 + metadata, BASE + 0x3000, 8);
    pe_rva(
        &mut file,
        0x1800 + metadata + 8,
        stride as u64,
        if rerelease { 8 } else { 4 },
    );
    let count_offset = metadata + if rerelease { 16 } else { 12 };
    pe_rva(&mut file, 0x1800 + count_offset, 2, 4);
    pe_rva(&mut file, 0x1800 + count_offset + 4, 2, 4);
    let mut image = Image::parse(&file, None, LoadRole::Library).unwrap();
    let mut bytes = std::mem::take(&mut image.bytes).into_vec();
    bytes.resize(0x8000, 0);
    let pointer = 0x3000 + stride + if rerelease { 120 } else { 88 };
    bytes[pointer..pointer + 8].copy_from_slice(&(BASE + 0x6000).to_le_bytes());
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
        rules,
        if rerelease { 25 } else { 100 },
        &runtime.geometry,
        Duration::from_secs(3),
    )
    .unwrap();
    let module = ModuleId(1);
    let api = Q2Clients {
        module,
        rules,
        connect: CallbackId(game.entry(b"ClientConnect").unwrap()),
        begin: CallbackId(game.entry(b"ClientBegin").unwrap()),
        disconnect: CallbackId(game.entry(b"ClientDisconnect").unwrap()),
        think: CallbackId(game.entry(b"ClientThink").unwrap()),
        command: CallbackId(game.entry(b"ClientCommand").unwrap()),
        userinfo_changed: CallbackId(game.entry(b"ClientUserinfoChanged").unwrap()),
        abi: qa_compat::q2_abi::client_layout(rules).unwrap(),
        userinfo: BASE + 0x5000,
        social: BASE + 0x5800,
        usercmd: BASE + 0x5c00,
    };
    let entries = (0..game.vm.export_count() as u32).collect();
    let imports = game.imports;
    let imports_address = game.imports_address;
    let frame = CallbackId(game.entry(b"RunFrame").unwrap());
    let shutdown = CallbackId(game.entry(b"Shutdown").unwrap());
    let mut module_request = request(
        &mut runtime,
        module,
        rules,
        TickRate::fixed(if rerelease { 25 } else { 100 }).unwrap(),
        game.vm,
    );
    if let Program::Native { imports: table, .. } = &mut module_request.program {
        *table = imports;
    }
    module_request.entries = entries;
    module_request.frame = Export::words(frame, &[0]);
    module_request.api = Some(ApiCheck::NativeTable {
        export: Export::words(CallbackId(0), &[imports_address]),
    });
    module_request.shutdown.push(Export::words(shutdown, &[]));
    let mut host = FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![module_request],
    )
    .unwrap();
    host.initialize_modules(Phase::Server);
    let maximum = host
        .console
        .cvars
        .bind(
            "sv_maxclients",
            Context {
                source: rules,
                ..Context::default()
            },
        )
        .unwrap();
    host.console.cvars.force_write(maximum, "1").unwrap();
    host.connect_q2_client(&api, client, b"\\name\\initial")
        .unwrap();
    host.local_clients[0] = Some(client);
    host.runtime.loopback.clear_client(client);
    let movement = if rerelease {
        let mut commands = [ZERO_Q2_RR; 3];
        commands[2].msec = 10;
        Move::Quake2Repro {
            last_frame: -1,
            commands,
        }
    } else {
        let mut commands = [ZERO_Q2; 3];
        commands[2].msec = 10;
        Move::Quake2 {
            last_frame: -1,
            commands,
        }
    };
    let mut prefix = Vec::new();
    string(&mut prefix, 4, COMMAND);
    let mut suffix = Vec::new();
    string(&mut suffix, 4, COMMAND);
    submit(&mut host, client, &prefix, &movement, &suffix);
    assert!(
        !host.runtime.quit,
        "packet strings must not reach local commands"
    );
    assert_eq!(
        host.runtime.server.clients[client.0 as usize].player.health, 11,
        "ClientCommand must precede ClientThink; the suffix must follow it ({rules:?})"
    );
    let mut prefix = Vec::new();
    string(&mut prefix, 3, b"\\name\\\x80native");
    submit(&mut host, client, &prefix, &movement, &[]);
    assert_eq!(
        host.runtime.server.clients[client.0 as usize].player.health, 122,
        "userinfo callback must precede the next ClientThink ({rules:?})"
    );
    assert_eq!(host.module_counts(module).unwrap().traps, 0);
    let mut prefix = Vec::new();
    string(&mut prefix, 4, b"disconnect");
    string(&mut prefix, 4, b"set bad");
    submit(&mut host, client, &prefix, &movement, &[]);
    assert!(
        host.runtime.server.clients[client.0 as usize]
            .connection
            .is_none()
    );
    assert!(host.runtime.network.get(client, Endpoint::Client).is_none());
    assert!(host.runtime.network.get(client, Endpoint::Server).is_none());
    assert!(host.local_clients[0].is_none());
    assert_eq!(host.module_counts(module).unwrap().traps, 0);
    host.shutdown_modules();
    assert_eq!(host.module_counts(module).unwrap().traps, 0);
}

fn address(code: &mut Vec<u8>, pointer: u64) {
    code.extend([0x48, 0xb8]);
    code.extend(pointer.to_le_bytes());
}
fn expect_edict(code: &mut Vec<u8>, edict: u64) {
    address(code, edict);
    code.extend([0x48, 0x39, 0xc1, 0x74, 2, 0x0f, 0x0b]);
}
fn expect_eax(code: &mut Vec<u8>, expected: u8) {
    code.extend([0x83, 0xf8, expected, 0x74, 2, 0x0f, 0x0b]);
}
fn expect_string(code: &mut Vec<u8>, expected: &[u8]) {
    code.extend([0x48, 0x85, 0xc0, 0x75, 2, 0x0f, 0x0b]);
    for (offset, &byte) in expected.iter().enumerate() {
        code.extend([0x80, 0x78, offset as u8, byte, 0x74, 2, 0x0f, 0x0b]);
    }
}
fn import(code: &mut Vec<u8>, rva: usize, header: usize, ordinal: usize) {
    code.extend([0x48, 0x8b, 0x05]);
    let next = rva + code.len() + 4;
    code.extend((IMPORTS as i32 - next as i32).to_le_bytes());
    code.extend([0x48, 0x8b, 0x80]);
    code.extend(((header + ordinal * 8) as i32).to_le_bytes());
    code.extend([0xff, 0xd0]);
}
fn string(payload: &mut Vec<u8>, opcode: u8, text: &[u8]) {
    payload.push(opcode);
    payload.extend(text);
    payload.push(0);
}
fn submit(
    host: &mut FrameHost,
    client: qa_core::primitives::ClientId,
    prefix: &[u8],
    movement: &qa_network::commands::packet::Move,
    suffix: &[u8],
) {
    use qa_core::loopback::Endpoint;
    use qa_network::commands::packet::{self, Key};
    let connection = host
        .runtime
        .network
        .get_mut(client, Endpoint::Client)
        .unwrap();
    let sequence = connection.channel.send_state().sequence;
    let mut bytes = [0; 1400];
    bytes[..prefix.len()].copy_from_slice(prefix);
    let length = packet::write(
        &mut bytes[prefix.len()..],
        movement,
        sequence,
        Key::default(),
    )
    .unwrap();
    let end = prefix.len() + length;
    bytes[end..end + suffix.len()].copy_from_slice(suffix);
    connection
        .channel
        .prepare_move(&bytes[..end + suffix.len()], host.time, None)
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
}
