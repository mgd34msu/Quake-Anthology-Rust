use super::*;
use qa_core::primitives::{
    EntityAppearance, ModelRef, NativeAppearanceTail, ResourceNamespace, Vec3,
};

const BASE: u64 = 0x180000000;
const PHASE: u64 = BASE + 0x1b60;

pub(super) fn native_appearance_uses_common_lifetimes_and_player_state() {
    for rerelease in [false, true] {
        appearance_for_abi(rerelease);
    }
}

fn appearance_for_abi(rerelease: bool) {
    use qa_app::{
        client_policy::ClientPolicy,
        map::SpawnAnchor,
        modules::{ApiCheck, Q2Clients},
    };
    use qa_compat::native::q2::Game;
    use qa_core::{primitives::NativeEntity, sys_events::SeatId};
    use qa_network::commands::packet::Protocol;
    use qa_world::area::{LinkFlags, LinkIntent};

    let rules = if rerelease {
        RuleSetId::Quake2Rerelease
    } else {
        RuleSetId::Quake2
    };
    let module = ModuleId(1);
    let stride = if rerelease { 1472 } else { 280 };
    let in_use = if rerelease { 1376 } else { 96 };
    let object = BASE + 0x3000 + 2 * stride as u64;
    let native_client = BASE + 0x3000 + stride as u64;
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
            Protocol::Quake2_34,
            0,
        )
        .unwrap();
    let mut file = q2_table_file();
    pe_rva(&mut file, 0x1800, if rerelease { 2023 } else { 3 }, 4);
    pe_text(&mut file, 0x1700, &[0xc3]);
    pe_text(&mut file, 0x1710, &[0xc3, 0x90, 0x90, 0x90, 0x90]);
    pe_text(&mut file, 0x1720, &[0xb8, 1, 0, 0, 0, 0xc3]);
    let mut begin = Vec::new();
    address(&mut begin, 0xb8, native_client + in_use as u64);
    if rerelease {
        begin.extend([0xc6, 0x00, 1]);
    } else {
        begin.extend([0xc7, 0x00, 1, 0, 0, 0]);
    }
    begin.push(0xc3);
    pe_text(&mut file, 0x1740, &begin);

    // Native RunFrame makes successive edits without LinkEntity. Each source
    // row is immutable fixture data, copied by the real child into its edict.
    let mut frame_code = vec![0x56, 0x57];
    address(&mut frame_code, 0xb8, PHASE);
    frame_code.extend([0x8b, 0x10, 0x83, 0x00, 1]);
    let mut jumps = Vec::new();
    for phase in 0..4 {
        frame_code.extend([0x83, 0xfa, phase, 0x0f, 0x84]);
        jumps.push(frame_code.len());
        frame_code.extend([0; 4]);
    }
    frame_code.extend([0x0f, 0x0b]);
    for (phase, jump) in jumps.into_iter().enumerate() {
        let relative = frame_code.len() as i32 - (jump + 4) as i32;
        frame_code[jump..jump + 4].copy_from_slice(&relative.to_le_bytes());
        if phase == 2 {
            address(&mut frame_code, 0xb8, object + in_use as u64);
            if rerelease {
                frame_code.extend([0xc6, 0x00, 0]);
            } else {
                frame_code.extend([0xc7, 0x00, 0, 0, 0, 0]);
            }
        } else {
            copy(
                &mut frame_code,
                object,
                BASE + [0x7000, 0x7800, 0, 0x8000][phase],
                stride,
            );
            copy(
                &mut frame_code,
                BASE + 0x6000,
                BASE + if phase == 0 { 0x8800 } else { 0x8c00 },
                if rerelease { 296 } else { 184 },
            );
        }
        frame_code.extend([0x5f, 0x5e, 0xc3]);
    }
    assert!(frame_code.len() <= 0x500);
    pe_text(&mut file, 0x1200, &frame_code);
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
    for (offset, code) in if rerelease {
        [(88, 0x1720), (96, 0x1740), (136, 0x1200)]
    } else {
        [(64, 0x1720), (72, 0x1740), (112, 0x1200)]
    } {
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
    let count = metadata + if rerelease { 16 } else { 12 };
    pe_rva(&mut file, 0x1800 + count, 3, 4);
    pe_rva(&mut file, 0x1800 + count + 4, 3, 4);
    let mut image = Image::parse(&file, None, LoadRole::Library).unwrap();
    let mut bytes = std::mem::take(&mut image.bytes).into_vec();
    bytes.resize(0xa000, 0);
    let client_pointer = 0x3000 + stride + if rerelease { 120 } else { 88 };
    put(&mut bytes, client_pointer, BASE + 0x6000, 8);
    let first = appearance(rules, module, 0);
    let second = appearance(rules, module, 1);
    let replacement = appearance(rules, module, 3);
    for (at, appearance, position) in [
        (0x7000, first, Vec3([11., -12., 13.])),
        (0x7800, second, Vec3([101., 102., 103.])),
        (0x8000, replacement, Vec3([-21., -22., -23.])),
    ] {
        write_edict(&mut bytes[at..at + stride], rerelease, appearance, position);
    }
    write_player(&mut bytes[0x8800..0x8a00], rerelease, 99, 77);
    write_player(&mut bytes[0x8c00..0x8e00], rerelease, 0, -15);
    image.bytes = bytes.into_boxed_slice();
    let mut regions = std::mem::take(&mut image.regions).into_vec();
    regions.push(qa_formats::program::native::Region {
        offset: 0x3000,
        length: 0x7000,
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
    let frame = CallbackId(game.entry(b"RunFrame").unwrap());
    let entries = (0..game.vm.export_count() as u32).collect();
    let imports = game.imports;
    let imports_address = game.imports_address;
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
    host.connect_q2_client(&api, client, b"\\name\\appearance")
        .unwrap();
    let native = NativeEntity { module, slot: 2 };
    run_frame(&mut host, module, frame);
    let old = find_native(&host, native);
    assert!(host.runtime.server.area.bounds(old).is_none());
    assert_appearance(
        host.runtime.server.entities.columns.appearance[old.slot as usize],
        first,
    );
    assert_eq!(
        host.runtime.server.entities.columns.position[old.slot as usize],
        Vec3([11., -12., 13.])
    );
    let player = &host.runtime.server.clients[client.0 as usize].player;
    assert_eq!(
        player.view_weapon.model,
        ModelRef::native(ResourceNamespace::Module(module), 99)
    );
    assert_eq!(player.view_weapon.frame, 77);
    assert_eq!(player.view_weapon.offset, Vec3([4., 5., -6.]));
    assert_eq!(player.view_weapon.angles, Vec3([-7., 8., 9.]));
    assert_eq!(player.view_weapon.skin, if rerelease { -7 } else { 0 });
    assert_eq!(player.view_weapon.rate, if rerelease { 60 } else { 0 });
    assert_eq!(player.view_angles, Vec3([10., 20., 30.]));
    assert_eq!(
        player.view_offset,
        Vec3([1., 2., if rerelease { 23. } else { 3. }])
    );
    assert_eq!(player.punch_angles, Vec3([-1., -2., -3.]));
    // Simulate an already-linked shared body. Importing a new presentation pose
    // must keep its prior explicit-link bounds and insertion counter unchanged.
    assert!(host.runtime.server.area.link(
        &host.runtime.server.entities,
        old,
        LinkFlags::LINKED,
        qa_gameplay::rules::link_order(rules),
        LinkIntent::Explicit
    ));
    let bounds = host.runtime.server.area.bounds(old).unwrap();
    let relinks = host.runtime.server.area.relinks;
    run_frame(&mut host, module, frame);
    assert_eq!(find_native(&host, native), old);
    assert_appearance(
        host.runtime.server.entities.columns.appearance[old.slot as usize],
        second,
    );
    assert_eq!(
        host.runtime.server.entities.columns.position[old.slot as usize],
        Vec3([101., 102., 103.])
    );
    assert_eq!(host.runtime.server.area.bounds(old), Some(bounds));
    assert_eq!(host.runtime.server.area.relinks, relinks);
    assert!(
        host.runtime.server.clients[client.0 as usize]
            .player
            .view_weapon
            .model
            .is_none()
    );
    assert_eq!(
        host.runtime.server.clients[client.0 as usize]
            .player
            .view_weapon
            .frame,
        -15
    );
    run_frame(&mut host, module, frame);
    assert!(host.runtime.server.entities.resolve(old).is_none());
    assert!(host.runtime.server.area.bounds(old).is_none());
    run_frame(&mut host, module, frame);
    let new = find_native(&host, native);
    assert_ne!(new, old);
    assert!(host.runtime.server.entities.resolve(old).is_none());
    assert!(host.runtime.server.area.bounds(new).is_none());
    assert_appearance(
        host.runtime.server.entities.columns.appearance[new.slot as usize],
        replacement,
    );
    assert_eq!(host.module_counts(module).unwrap().traps, 0);
    host.shutdown_modules();
    assert_eq!(host.module_counts(module).unwrap().traps, 0);
}

fn appearance(source: RuleSetId, module: ModuleId, phase: u32) -> EntityAppearance {
    let rr = source == RuleSetId::Quake2Rerelease;
    EntityAppearance {
        source,
        models: [
            ModelRef::native(ResourceNamespace::Module(module), 7 + phase),
            None,
            ModelRef::native(ResourceNamespace::Module(module), 9),
            ModelRef::native(ResourceNamespace::Module(module), 255),
        ],
        frame: 1024 + phase as i32,
        skin: -456 - phase as i32,
        old_origin: Vec3([-0.0, 45., -67.]),
        alpha: if rr {
            if phase == 0 { 0. } else { 0.3 }
        } else {
            1.
        },
        scale: if rr {
            if phase == 0 { 0. } else { 0.75 }
        } else {
            1.
        },
        native: NativeAppearanceTail {
            colormap: 0,
            effects: if rr {
                0x8123_4567_9234_5678
            } else {
                0x9234_5678
            },
            render_flags: 0xfedc_ba98,
            event: if rr { 234 } else { 0x1234_abcd },
            loop_sound: 0x9234_5678,
            solid: 0xa234_5678,
            instance_bits: if rr { 197 } else { 0 },
            loop_volume: if rr { f32::from_bits(0x7fc0_1234) } else { 0. },
            loop_attenuation: if rr { -1. } else { 0. },
            owner: if rr { -7 } else { 0 },
            old_frame: if rr { 6543 } else { 0 },
        },
    }
}
fn write_edict(bytes: &mut [u8], rr: bool, value: EntityAppearance, position: Vec3) {
    vector(bytes, 4, position);
    vector(bytes, 16, Vec3([21., -22., 23.]));
    vector(bytes, 28, value.old_origin);
    for (index, model) in value.models.into_iter().enumerate() {
        put(
            bytes,
            40 + index * 4,
            model.map_or(0, |m| u64::from(m.ordinal)),
            4,
        );
    }
    put(bytes, 56, value.frame as u32 as u64, 4);
    put(bytes, 60, value.skin as u32 as u64, 4);
    put(bytes, 64, value.native.effects, if rr { 8 } else { 4 });
    let render = if rr { 72 } else { 68 };
    put(bytes, render, value.native.render_flags.into(), 4);
    put(bytes, render + 4, value.native.solid.into(), 4);
    put(bytes, render + 8, value.native.loop_sound.into(), 4);
    put(
        bytes,
        render + 12,
        value.native.event.into(),
        if rr { 1 } else { 4 },
    );
    if rr {
        for (offset, word) in [
            (88, value.alpha.to_bits()),
            (92, value.scale.to_bits()),
            (100, value.native.loop_volume.to_bits()),
            (104, value.native.loop_attenuation.to_bits()),
            (108, value.native.owner as u32),
            (112, value.native.old_frame as u32),
        ] {
            put(bytes, offset, word.into(), 4);
        }
        bytes[96] = value.native.instance_bits;
    }
    put(bytes, if rr { 1376 } else { 96 }, 1, if rr { 1 } else { 4 });
    vector(bytes, if rr { 1396 } else { 204 }, Vec3([-4., -5., -6.]));
    vector(bytes, if rr { 1408 } else { 216 }, Vec3([7., 8., 9.]));
}
fn write_player(bytes: &mut [u8], rr: bool, model: u32, frame: i32) {
    let view = if rr { 52 } else { 28 };
    vector(bytes, view, Vec3([10., 20., 30.]));
    vector(bytes, view + 12, Vec3([1., 2., 3.]));
    vector(bytes, view + 24, Vec3([-1., -2., -3.]));
    vector(bytes, view + 36, Vec3([-7., 8., 9.]));
    vector(bytes, view + 48, Vec3([4., 5., -6.]));
    put(bytes, view + 60, model.into(), 4);
    put(
        bytes,
        view + if rr { 68 } else { 64 },
        frame as u32 as u64,
        4,
    );
    if rr {
        bytes[48] = 20;
        put(bytes, 116, (-7i32) as u32 as u64, 4);
        put(bytes, 124, 60, 4);
    }
}
fn vector(bytes: &mut [u8], at: usize, vector: Vec3) {
    for (index, value) in vector.0.into_iter().enumerate() {
        put(bytes, at + index * 4, value.to_bits().into(), 4);
    }
}
fn address(code: &mut Vec<u8>, opcode: u8, value: u64) {
    code.extend([0x48, opcode]);
    code.extend(value.to_le_bytes());
}
fn copy(code: &mut Vec<u8>, destination: u64, source: u64, length: usize) {
    address(code, 0xbf, destination);
    address(code, 0xbe, source);
    code.push(0xb9);
    code.extend((length as u32).to_le_bytes());
    code.extend([0xf3, 0xa4]);
}
fn run_frame(host: &mut FrameHost, module: ModuleId, frame: CallbackId) {
    host.call_module_export(
        module,
        Export::words(frame, &[0]),
        ThinkTime::Milliseconds(25),
    )
    .unwrap();
}
fn find_native(
    host: &FrameHost,
    native: qa_core::primitives::NativeEntity,
) -> qa_core::primitives::EntityId {
    host.runtime
        .server
        .entities
        .active()
        .find(|id| {
            host.runtime.server.entities.columns.native_entity[id.slot as usize] == Some(native)
        })
        .unwrap()
}
fn assert_appearance(actual: EntityAppearance, expected: EntityAppearance) {
    assert_eq!(
        (actual.source, actual.models, actual.frame, actual.skin),
        (
            expected.source,
            expected.models,
            expected.frame,
            expected.skin
        )
    );
    assert_eq!(
        actual.old_origin.0.map(f32::to_bits),
        expected.old_origin.0.map(f32::to_bits)
    );
    assert_eq!(actual.alpha.to_bits(), expected.alpha.to_bits());
    assert_eq!(actual.scale.to_bits(), expected.scale.to_bits());
    assert_eq!(
        (
            actual.native.effects,
            actual.native.render_flags,
            actual.native.event,
            actual.native.loop_sound,
            actual.native.solid,
            actual.native.instance_bits,
            actual.native.owner,
            actual.native.old_frame
        ),
        (
            expected.native.effects,
            expected.native.render_flags,
            expected.native.event,
            expected.native.loop_sound,
            expected.native.solid,
            expected.native.instance_bits,
            expected.native.owner,
            expected.native.old_frame
        )
    );
    assert_eq!(
        actual.native.loop_volume.to_bits(),
        expected.native.loop_volume.to_bits()
    );
    assert_eq!(
        actual.native.loop_attenuation.to_bits(),
        expected.native.loop_attenuation.to_bits()
    );
}
