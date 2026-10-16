#[path = "support/client_asset_frames.rs"]
mod asset_frames;
#[path = "model_assets/support.rs"]
#[allow(dead_code)]
mod files;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/q3_lifecycle_image.rs"]
mod lifecycle;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "../../formats/tests/support/native_image.rs"]
#[allow(dead_code)]
mod native_image;
#[path = "support/sound_registration.rs"]
#[allow(dead_code)]
mod samples;
#[path = "support/service_program.rs"]
#[allow(dead_code)]
mod service_program;

use qa_app::{
    Runtime,
    client_policy::ClientPolicy,
    host::{FrameHost, FrameResult, FrameSource},
    map::SpawnAnchor,
    model_assets::{ModelAssets, ModelRequest},
    modules::{Export, ModuleResult, Q3Spec, State, load_q3},
    renderer::{Renderer, SceneComposer, console::ConsoleAssets, hud::HudView},
};
use qa_audio::{Bank, Listener, Mixer};
use qa_compat::services::ModelLookup;
use qa_console::{commands::Console, views::Context};
use qa_core::{
    loopback::Endpoint,
    primitives::*,
    sys_events::{EventKind, EventTime, Peer, SeatId, SysEvent, SysEventQueue},
};
use qa_formats::program::qvm::Opcode;
use qa_network::{channel::commands::write_gamestate, commands::packet::Protocol};
use qa_render::{Assets, Limits, scene::Command};
use qa_session::timing::TickRate;
use std::{sync::Arc, time::Duration};

const INITIAL: u32 = 0x4000;
const INLINE: u32 = 0x4004;
const CLIENT: u32 = 0x4008;
const SOUND: u32 = 0x400c;
const PATH: u32 = 0x4018;
const GRAPHICS_HANDLE: u32 = 0x4020;
const GRAPHICS_TEXT: u32 = 0x4740;
const REMAP_TEXT: u32 = 0x4780;
const REMAP_OFFSET: u32 = 0x47c0;
const VIDEO_TEXT: u32 = 0x47d0;
const INIT_TEXT: u32 = 0x4200;
const STOP_TEXT: u32 = 0x4220;
const ERROR_TEXT: u32 = 0x4240;
const FAILED_INIT_TEXT: u32 = 0x4280;
const REFDEF: u32 = 0x4300;
const SOUND_PATH: u32 = 0x4500;
const ORIGIN: u32 = 0x4600;
const AXES: u32 = 0x4640;
const UI_INIT_TEXT: u32 = 0x4700;
const UI_STOP_TEXT: u32 = 0x4720;
const COMMAND_OWNED: u32 = 0x4680;
const COMMAND_SHARED: u32 = 0x46a0;
const CLIENT_TEXT: u32 = 0x46c0;
const BSS: u32 = 0x7000;
const GAMESTATE: u32 = 0x8000;
const GLCONFIG: u32 = 0xd000;
const GLCONFIG_ASPECT: u32 = GLCONFIG + 11264 + 12 * 4;

fn data() -> Vec<u8> {
    let mut data = vec![0; 0x5000];
    data[INITIAL as usize..INITIAL as usize + 4].copy_from_slice(&7u32.to_le_bytes());
    for (at, text) in [
        (INIT_TEXT, b"restart init\n\0".as_slice()),
        (STOP_TEXT, b"restart shutdown\n\0".as_slice()),
        (UI_INIT_TEXT, b"renderer ui init\n\0".as_slice()),
        (UI_STOP_TEXT, b"renderer ui shutdown\n\0".as_slice()),
        (GRAPHICS_TEXT, b"graphics/after-engine\0".as_slice()),
        (REMAP_TEXT, b"graphics/replacement\0".as_slice()),
        (REMAP_OFFSET, b"2.5\0".as_slice()),
        (VIDEO_TEXT, b"vid_restart cpu\n\0".as_slice()),
        (
            ERROR_TEXT,
            b"restart did not recreate pristine memory\0".as_slice(),
        ),
        (
            FAILED_INIT_TEXT,
            b"intentional CG_INIT failure\0".as_slice(),
        ),
        (SOUND_PATH, b"sound/restart.wav\0".as_slice()),
        (COMMAND_OWNED, b"restart_owned\0".as_slice()),
        (COMMAND_SHARED, b"restart_shared\0".as_slice()),
        (CLIENT_TEXT, b"-native raw \"quoted\" payload\0".as_slice()),
    ] {
        data[at as usize..at as usize + text.len()].copy_from_slice(text);
    }
    let refdef = &mut data[REFDEF as usize..REFDEF as usize + qa_compat::renderer::Q3_REFDEF_BYTES];
    for (at, value) in [(0, 2i32), (4, 2), (8, 28), (12, 44)] {
        refdef[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    for (at, value) in [
        (16, 90f32),
        (20, 75.),
        (24, 32.),
        (36, -1.),
        (52, -1.),
        (68, 1.),
    ] {
        refdef[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    data[ORIGIN as usize..ORIGIN as usize + 4].copy_from_slice(&32f32.to_le_bytes());
    for (at, value) in [(AXES, -1f32), (AXES + 16, -1.), (AXES + 32, 1.)] {
        data[at as usize..at as usize + 4].copy_from_slice(&value.to_le_bytes());
    }
    data
}

#[derive(Clone, Copy)]
enum ImportArg {
    Pointer(u32),
    Integer(u32),
    Word(u32),
    PointerWord(u32),
}
fn qvm_call(ops: &mut Vec<(Opcode, i32)>, ordinal: i32, args: &[ImportArg]) {
    use Opcode::*;
    for (index, arg) in args.iter().enumerate() {
        match *arg {
            ImportArg::Pointer(value) | ImportArg::Integer(value) => {
                ops.push((Const, value as i32));
            }
            ImportArg::Word(address) | ImportArg::PointerWord(address) => {
                ops.extend([(Const, address as i32), (Load4, 0)]);
            }
        }
        ops.push((Arg, 8 + index as i32 * 4));
    }
    ops.extend([(Const, -ordinal - 1), (Call, 0)]);
}
fn qvm_store(ops: &mut Vec<(Opcode, i32)>, at: u32, value: u32) {
    use Opcode::*;
    ops.extend([(Const, at as i32), (Const, value as i32), (Store4, 0)]);
}
fn qvm_check(ops: &mut Vec<(Opcode, i32)>, at: u32, value: u32, abort: i32) {
    use Opcode::*;
    ops.extend([
        (Const, at as i32),
        (Load4, 0),
        (Const, value as i32),
        (Eq, 0),
    ]);
    let branch = ops.len() - 1;
    qvm_call(ops, abort, &[ImportArg::Pointer(ERROR_TEXT)]);
    ops.push((Pop, 0));
    ops[branch].1 = ops.len() as i32;
}
fn qvm_file() -> Vec<u8> {
    qvm_role(false, false)
}
fn qvm_role(ui: bool, fail_shutdown: bool) -> Vec<u8> {
    qvm_role_with_graphics(ui, fail_shutdown, false)
}
fn qvm_role_with_graphics(ui: bool, fail_shutdown: bool, graphics: bool) -> Vec<u8> {
    use Opcode::*;
    let mut ops = vec![(Enter, 64)];
    let mut branches = Vec::new();
    for command in 0..=10 {
        ops.extend([(Local, 72), (Load4, 0), (Const, command), (Eq, 0)]);
        branches.push(ops.len() - 1);
    }
    ops.extend([(Const, 0), (Leave, 64)]);
    for (command, branch) in branches.into_iter().enumerate() {
        ops[branch].1 = ops.len() as i32;
        if ui {
            match command {
                0 => ops.push((Const, 6)),
                1 => {
                    qvm_check(&mut ops, INITIAL, 7, 0);
                    qvm_check(&mut ops, BSS, 0, 0);
                    if graphics {
                        // Engine font, white, and console own the first three
                        // real registrations before this guest shader.
                        ops.push((Const, GRAPHICS_HANDLE as i32));
                        qvm_call(&mut ops, 20, &[ImportArg::Pointer(GRAPHICS_TEXT)]);
                        ops.push((Store4, 0));
                        qvm_check(&mut ops, GRAPHICS_HANDLE, 4, 0);
                    }
                    qvm_call(&mut ops, 1, &[ImportArg::Pointer(UI_INIT_TEXT)]);
                }
                2 => {
                    if graphics {
                        ops.push((Const, GRAPHICS_HANDLE as i32));
                        qvm_call(&mut ops, 20, &[ImportArg::Pointer(GRAPHICS_TEXT)]);
                        ops.push((Store4, 0));
                        qvm_check(&mut ops, GRAPHICS_HANDLE, 4, 0);
                    }
                    qvm_call(&mut ops, 1, &[ImportArg::Pointer(UI_STOP_TEXT)]);
                    ops.push((Pop, 0));
                    if fail_shutdown {
                        qvm_call(&mut ops, 0, &[ImportArg::Pointer(ERROR_TEXT)]);
                    } else {
                        qvm_store(&mut ops, INITIAL, 0xdead);
                        qvm_store(&mut ops, BSS, 0xbeef);
                        ops.push((Const, 0));
                    }
                }
                3 if graphics => {
                    qvm_call(&mut ops, 43, &[ImportArg::Pointer(GLCONFIG)]);
                    ops.extend([(Pop, 0), (Const, GLCONFIG_ASPECT as i32), (Load4, 0)]);
                }
                5 => {
                    qvm_store(&mut ops, INITIAL, 99);
                    ops.extend([
                        (Const, BSS as i32),
                        (Const, BSS as i32),
                        (Load4, 0),
                        (Const, 1),
                        (Add, 0),
                        (Store4, 0),
                        (Const, 0),
                    ]);
                    if graphics {
                        ops.push((Pop, 0));
                        qvm_call(
                            &mut ops,
                            27,
                            &[
                                ImportArg::Integer(0),
                                ImportArg::Integer(0),
                                ImportArg::Integer(2f32.to_bits()),
                                ImportArg::Integer(2f32.to_bits()),
                                ImportArg::Integer(0),
                                ImportArg::Integer(0),
                                ImportArg::Integer(1f32.to_bits()),
                                ImportArg::Integer(1f32.to_bits()),
                                ImportArg::Word(GRAPHICS_HANDLE),
                            ],
                        );
                    }
                }
                6 if graphics => qvm_call(
                    &mut ops,
                    80,
                    &[
                        ImportArg::Pointer(GRAPHICS_TEXT),
                        ImportArg::Pointer(REMAP_TEXT),
                        ImportArg::Pointer(REMAP_OFFSET),
                    ],
                ),
                7 => ops.extend([(Const, INITIAL as i32), (Load4, 0)]),
                8 if graphics => ops.extend([(Const, GRAPHICS_HANDLE as i32), (Load4, 0)]),
                9 if graphics => {
                    qvm_call(
                        &mut ops,
                        12,
                        &[ImportArg::Integer(0), ImportArg::Pointer(VIDEO_TEXT)],
                    );
                    ops.push((Pop, 0));
                    qvm_store(&mut ops, INITIAL, 0x600d);
                    ops.push((Const, 0x600d));
                }
                10 => ops.extend([(Const, BSS as i32), (Load4, 0)]),
                _ => ops.push((Const, 0)),
            }
            ops.push((Leave, 64));
            continue;
        }
        match command {
            0 => {
                qvm_check(&mut ops, INITIAL, 7, 1);
                qvm_check(&mut ops, BSS, 0, 1);
                if graphics {
                    ops.push((Const, GRAPHICS_HANDLE as i32));
                    qvm_call(&mut ops, 39, &[ImportArg::Pointer(GRAPHICS_TEXT)]);
                    ops.push((Store4, 0));
                    qvm_check(&mut ops, GRAPHICS_HANDLE, 4, 1);
                }
                ops.extend([(Const, CLIENT as i32), (Local, 84), (Load4, 0), (Store4, 0)]);
                ops.extend([(Const, CLIENT as i32), (Load4, 0), (Const, 44), (Ne, 0)]);
                let healthy = ops.len() - 1;
                qvm_call(&mut ops, 1, &[ImportArg::Pointer(FAILED_INIT_TEXT)]);
                ops.push((Pop, 0));
                ops[healthy].1 = ops.len() as i32;
                ops.extend([(Const, CLIENT as i32), (Load4, 0), (Const, 22), (Eq, 0)]);
                let peer = ops.len() - 1;
                qvm_call(&mut ops, 15, &[ImportArg::Pointer(COMMAND_OWNED)]);
                ops.push((Pop, 0));
                ops[peer].1 = ops.len() as i32;
                qvm_call(&mut ops, 15, &[ImportArg::Pointer(COMMAND_SHARED)]);
                ops.push((Pop, 0));
                qvm_call(&mut ops, 50, &[ImportArg::Pointer(GAMESTATE)]);
                ops.push((Pop, 0));
                // CG_INIT selects the exact world path in its accepted native
                // gamestate, using CS_MODELS+1 through the real ABI table.
                ops.extend([
                    (Const, PATH as i32),
                    (Const, GAMESTATE as i32 + 33 * 4),
                    (Load4, 0),
                    (Const, GAMESTATE as i32 + 4096),
                    (Add, 0),
                    (Store4, 0),
                ]);
                for ordinal in [18, 36] {
                    qvm_call(&mut ops, ordinal, &[ImportArg::PointerWord(PATH)]);
                    ops.push((Pop, 0));
                }
                ops.push((Const, INLINE as i32));
                qvm_call(&mut ops, 19, &[]);
                ops.push((Store4, 0));
                ops.push((Const, SOUND as i32));
                qvm_call(
                    &mut ops,
                    34,
                    &[ImportArg::Pointer(SOUND_PATH), ImportArg::Integer(0)],
                );
                ops.push((Store4, 0));
                qvm_call(&mut ops, 0, &[ImportArg::Pointer(INIT_TEXT)]);
                ops.push((Pop, 0));
                qvm_call(
                    &mut ops,
                    33,
                    &[
                        ImportArg::Word(CLIENT),
                        ImportArg::Pointer(ORIGIN),
                        ImportArg::Pointer(AXES),
                        ImportArg::Integer(0),
                    ],
                );
                ops.push((Pop, 0));
                qvm_call(
                    &mut ops,
                    29,
                    &[ImportArg::Word(SOUND), ImportArg::Integer(0)],
                );
                ops.extend([(Pop, 0), (Const, 0)]);
            }
            1 => {
                qvm_call(&mut ops, 0, &[ImportArg::Pointer(STOP_TEXT)]);
                ops.push((Pop, 0));
                qvm_store(&mut ops, INITIAL, 0xdead);
                qvm_store(&mut ops, BSS, 0xbeef);
                qvm_call(
                    &mut ops,
                    29,
                    &[ImportArg::Word(SOUND), ImportArg::Integer(0)],
                );
            }
            3 => {
                ops.extend([
                    (Const, BSS as i32),
                    (Const, BSS as i32),
                    (Load4, 0),
                    (Const, 1),
                    (Add, 0),
                    (Store4, 0),
                ]);
                qvm_store(&mut ops, INITIAL, 99);
                qvm_call(&mut ops, 44, &[ImportArg::Pointer(REFDEF)]);
                ops.push((Pop, 0));
                qvm_call(
                    &mut ops,
                    33,
                    &[
                        ImportArg::Word(CLIENT),
                        ImportArg::Pointer(ORIGIN),
                        ImportArg::Pointer(AXES),
                        ImportArg::Integer(0),
                    ],
                );
            }
            2 if graphics => qvm_call(
                &mut ops,
                79,
                &[
                    ImportArg::Pointer(GRAPHICS_TEXT),
                    ImportArg::Pointer(REMAP_TEXT),
                    ImportArg::Pointer(REMAP_OFFSET),
                ],
            ),
            6 if graphics => ops.extend([(Const, GRAPHICS_HANDLE as i32), (Load4, 0)]),
            4 if graphics => {
                qvm_call(&mut ops, 49, &[ImportArg::Pointer(GLCONFIG)]);
                ops.extend([(Pop, 0), (Const, GLCONFIG_ASPECT as i32), (Load4, 0)]);
            }
            4 | 5 | 6 => {
                let (ordinal, pointer) = match command {
                    4 => (72, COMMAND_OWNED),
                    5 => (16, CLIENT_TEXT),
                    _ => (15, COMMAND_OWNED),
                };
                qvm_call(&mut ops, ordinal, &[ImportArg::Pointer(pointer)]);
            }
            7 => ops.extend([(Const, INITIAL as i32), (Load4, 0)]),
            8 => ops.extend([(Const, INLINE as i32), (Load4, 0)]),
            9 => ops.extend([(Const, CLIENT as i32), (Load4, 0)]),
            2 | 10 => ops.extend([(Const, BSS as i32), (Load4, 0)]),
            _ => ops.push((Const, 0)),
        }
        ops.push((Leave, 64));
    }
    service_program::bytes(&ops, &data())
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn native_file() -> Vec<u8> {
    native_role(false, false)
}
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn native_role(ui: bool, fail_shutdown: bool) -> Vec<u8> {
    native_role_with_graphics(ui, fail_shutdown, false)
}
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn native_role_with_graphics(ui: bool, fail_shutdown: bool, graphics: bool) -> Vec<u8> {
    use native_image::*;
    let mut file = lifecycle::elf_lifecycle_file();
    // Retain the shared fixture's real ELF constructor/finalizer chain, but
    // provide file-backed initialized data and a genuine zero-filled BSS tail.
    elf_segment(&mut file, 64, 1, 1, 0x1180, 0x3180, 0x1e80, 0x20000);
    // elf_segment initializes data rights; preserve this fixture's code rights
    // when extending the same load segment for vmMain and its constructors.
    put(&mut file, 64 + 56 + 4, 7, 4);
    let values = data();
    file[0x2000..0x2800].copy_from_slice(&values[0x4000..0x4800]);
    put(&mut file, 0x1400 + 24 + 8, 0x4800, 8);
    fn rip(code: &mut Vec<u8>, prefix: &[u8], target: u32, base: u32) {
        code.extend(prefix);
        code.extend(
            (i64::from(target) - i64::from(base) - code.len() as i64 - 4).to_le_bytes()[..4].iter(),
        );
    }
    fn call(code: &mut Vec<u8>, ordinal: u32, args: &[ImportArg], base: u32) {
        for (index, arg) in args.iter().enumerate() {
            match *arg {
                ImportArg::Pointer(value) => rip(
                    code,
                    &[
                        [0x48, 0x8d, 0x35],
                        [0x48, 0x8d, 0x15],
                        [0x48, 0x8d, 0x0d],
                        [0x4c, 0x8d, 0x05],
                    ][index],
                    value,
                    base,
                ),
                ImportArg::Integer(value) => {
                    code.extend([[0xbe].as_slice(), &[0xba], &[0xb9], &[0x41, 0xb8]][index]);
                    code.extend(value.to_le_bytes());
                }
                ImportArg::Word(at) => rip(
                    code,
                    [
                        [0x8b, 0x35].as_slice(),
                        &[0x8b, 0x15],
                        &[0x8b, 0x0d],
                        &[0x44, 0x8b, 0x05],
                    ][index],
                    at,
                    base,
                ),
                ImportArg::PointerWord(at) => rip(
                    code,
                    &[
                        [0x48, 0x8b, 0x35],
                        [0x48, 0x8b, 0x15],
                        [0x48, 0x8b, 0x0d],
                        [0x4c, 0x8b, 0x05],
                    ][index],
                    at,
                    base,
                ),
            }
        }
        code.push(0xbf);
        code.extend(ordinal.to_le_bytes());
        rip(code, &[0xff, 0x15], 0x3ac0, base);
    }
    // The new DT_INIT checks initialized and BSS bytes, then tail-calls the
    // unchanged shared constructor which appends the first lifecycle digit.
    let mut constructor = Vec::new();
    for (at, value) in [(INITIAL, 7), (BSS, 0)] {
        rip(&mut constructor, &[0x83, 0x3d], at, 0x4f00);
        // RIP addresses for cmp [rip+disp32],imm8 include that final immediate.
        let displacement = constructor.len() - 4;
        let adjusted = i32::from_le_bytes(constructor[displacement..].try_into().unwrap()) - 1;
        constructor[displacement..].copy_from_slice(&adjusted.to_le_bytes());
        constructor.extend([value, 0x74, 2, 0x0f, 0x0b]);
    }
    rip(&mut constructor, &[0xe9], 0x3700, 0x4f00);
    file[0x2f00..0x2f00 + constructor.len()].copy_from_slice(&constructor);
    for at in (0x1c00..0x1e00).step_by(16) {
        if u64::from_le_bytes(file[at..at + 8].try_into().unwrap()) == 12 {
            put(&mut file, at + 8, 0x4f00, 8);
            break;
        }
    }
    let base = 0x4800;
    let mut code = Vec::new();
    let mut branches = Vec::new();
    for command in [0u8, 1, 3, 4, 5, 6, 7, 8, 9, 2, 10] {
        code.extend([0x83, 0xff, command, 0x0f, 0x84]);
        branches.push((command, code.len()));
        code.extend([0; 4]);
    }
    code.extend([0x31, 0xc0, 0xc3]);
    for (command, branch) in branches {
        let relative = code.len() as i32 - branch as i32 - 4;
        code[branch..branch + 4].copy_from_slice(&relative.to_le_bytes());
        if ui {
            match command {
                0 => code.extend([0xb8, 6, 0, 0, 0, 0xc3]),
                1 => {
                    // The same fresh native child constructor already checks
                    // initialized/BSS storage before dllEntry and this UI_INIT.
                    code.extend([0x48, 0x83, 0xec, 8]);
                    if graphics {
                        call(&mut code, 20, &[ImportArg::Pointer(GRAPHICS_TEXT)], base);
                        rip(&mut code, &[0x89, 0x05], GRAPHICS_HANDLE, base);
                        code.extend([0x83, 0xf8, 4, 0x74, 2, 0x0f, 0x0b]);
                    }
                    call(&mut code, 1, &[ImportArg::Pointer(UI_INIT_TEXT)], base);
                    code.extend([0x48, 0x83, 0xc4, 8, 0x31, 0xc0, 0xc3]);
                }
                2 => {
                    code.extend([0x48, 0x83, 0xec, 8]);
                    if graphics {
                        call(&mut code, 20, &[ImportArg::Pointer(GRAPHICS_TEXT)], base);
                        code.extend([0x83, 0xf8, 4, 0x74, 2, 0x0f, 0x0b]);
                    }
                    call(&mut code, 1, &[ImportArg::Pointer(UI_STOP_TEXT)], base);
                    if fail_shutdown {
                        call(&mut code, 0, &[ImportArg::Pointer(ERROR_TEXT)], base);
                    }
                    code.extend([0x48, 0x83, 0xc4, 8]);
                    // Same exact role-Shutdown/finalizer trap as the cgame.
                    rip(&mut code, &[0xe9], 0x360c, base);
                }
                3 if graphics => {
                    code.extend([0x48, 0x83, 0xec, 8]);
                    call(&mut code, 43, &[ImportArg::Pointer(GLCONFIG)], base);
                    code.extend([0x48, 0x83, 0xc4, 8]);
                    rip(&mut code, &[0x8b, 0x05], GLCONFIG_ASPECT, base);
                    code.push(0xc3);
                }
                5 => {
                    rip(&mut code, &[0xff, 0x05], BSS, base);
                    rip(&mut code, &[0xc7, 0x05], INITIAL, base);
                    let displacement = code.len() - 4;
                    let adjusted = i32::from_le_bytes(code[displacement..].try_into().unwrap()) - 4;
                    code[displacement..].copy_from_slice(&adjusted.to_le_bytes());
                    code.extend(99u32.to_le_bytes());
                    if graphics {
                        // Variadic integer ABI: five register arguments, then
                        // t1/s2/t2 and the actual returned shader on the stack.
                        code.extend([0x48, 0x83, 0xec, 40]);
                        code.extend([0x48, 0xc7, 0x04, 0x24, 0, 0, 0, 0]);
                        for offset in [8, 16] {
                            code.extend([0x48, 0xc7, 0x44, 0x24, offset]);
                            code.extend(1f32.to_bits().to_le_bytes());
                        }
                        rip(&mut code, &[0x8b, 0x05], GRAPHICS_HANDLE, base);
                        code.extend([0x48, 0x89, 0x44, 0x24, 24, 0x45, 0x31, 0xc9]);
                        call(
                            &mut code,
                            27,
                            &[
                                ImportArg::Integer(0),
                                ImportArg::Integer(0),
                                ImportArg::Integer(2f32.to_bits()),
                                ImportArg::Integer(2f32.to_bits()),
                            ],
                            base,
                        );
                        code.extend([0x48, 0x83, 0xc4, 40]);
                    }
                    code.extend([0x31, 0xc0, 0xc3]);
                }
                6 if graphics => {
                    code.extend([0x48, 0x83, 0xec, 8]);
                    call(
                        &mut code,
                        80,
                        &[
                            ImportArg::Pointer(GRAPHICS_TEXT),
                            ImportArg::Pointer(REMAP_TEXT),
                            ImportArg::Pointer(REMAP_OFFSET),
                        ],
                        base,
                    );
                    code.extend([0x48, 0x83, 0xc4, 8, 0xc3]);
                }
                8 if graphics => {
                    rip(&mut code, &[0x8b, 0x05], GRAPHICS_HANDLE, base);
                    code.push(0xc3);
                }
                9 if graphics => {
                    code.extend([0x48, 0x83, 0xec, 8]);
                    call(
                        &mut code,
                        12,
                        &[ImportArg::Integer(0), ImportArg::Pointer(VIDEO_TEXT)],
                        base,
                    );
                    code.extend([0x48, 0x83, 0xc4, 8]);
                    rip(&mut code, &[0xc7, 0x05], INITIAL, base);
                    let displacement = code.len() - 4;
                    let adjusted = i32::from_le_bytes(code[displacement..].try_into().unwrap()) - 4;
                    code[displacement..].copy_from_slice(&adjusted.to_le_bytes());
                    code.extend(0x600du32.to_le_bytes());
                    code.push(0xb8);
                    code.extend(0x600du32.to_le_bytes());
                    code.push(0xc3);
                }
                7 | 10 => {
                    rip(
                        &mut code,
                        &[0x8b, 0x05],
                        if command == 7 { INITIAL } else { BSS },
                        base,
                    );
                    code.push(0xc3);
                }
                _ => code.extend([0x31, 0xc0, 0xc3]),
            }
            continue;
        }
        match command {
            0 => {
                rip(&mut code, &[0x89, 0x0d], CLIENT, base);
                // ctor1/ctor2/ctor3/dllEntry4 must precede every CG_INIT.
                rip(&mut code, &[0x81, 0x3d], 0x3a80, base);
                let displacement = code.len() - 4;
                let adjusted = i32::from_le_bytes(code[displacement..].try_into().unwrap()) - 4;
                code[displacement..].copy_from_slice(&adjusted.to_le_bytes());
                code.extend(1234u32.to_le_bytes());
                code.extend([0x74, 2, 0x0f, 0x0b, 0x48, 0x83, 0xec, 8]);
                if graphics {
                    call(&mut code, 39, &[ImportArg::Pointer(GRAPHICS_TEXT)], base);
                    rip(&mut code, &[0x89, 0x05], GRAPHICS_HANDLE, base);
                    code.extend([0x83, 0xf8, 4, 0x74, 2, 0x0f, 0x0b]);
                    rip(&mut code, &[0x8b, 0x0d], CLIENT, base);
                }
                code.extend([0x83, 0xf9, 44, 0x0f, 0x85]);
                let healthy = code.len();
                code.extend([0; 4]);
                call(&mut code, 1, &[ImportArg::Pointer(FAILED_INIT_TEXT)], base);
                let relative = code.len() as i32 - healthy as i32 - 4;
                code[healthy..healthy + 4].copy_from_slice(&relative.to_le_bytes());
                code.extend([0x83, 0xf9, 22, 0x0f, 0x84]);
                let peer = code.len();
                code.extend([0; 4]);
                call(&mut code, 15, &[ImportArg::Pointer(COMMAND_OWNED)], base);
                let relative = code.len() as i32 - peer as i32 - 4;
                code[peer..peer + 4].copy_from_slice(&relative.to_le_bytes());
                call(&mut code, 15, &[ImportArg::Pointer(COMMAND_SHARED)], base);
                call(&mut code, 50, &[ImportArg::Pointer(GAMESTATE)], base);
                rip(&mut code, &[0x8b, 0x05], GAMESTATE + 33 * 4, base);
                rip(&mut code, &[0x48, 0x8d, 0x15], GAMESTATE + 4096, base);
                code.extend([0x48, 0x01, 0xd0]);
                rip(&mut code, &[0x48, 0x89, 0x05], PATH, base);
                for ordinal in [18, 36] {
                    call(&mut code, ordinal, &[ImportArg::PointerWord(PATH)], base);
                }
                call(&mut code, 19, &[], base);
                rip(&mut code, &[0x89, 0x05], INLINE, base);
                call(
                    &mut code,
                    34,
                    &[ImportArg::Pointer(SOUND_PATH), ImportArg::Integer(0)],
                    base,
                );
                rip(&mut code, &[0x89, 0x05], SOUND, base);
                call(&mut code, 0, &[ImportArg::Pointer(INIT_TEXT)], base);
                // Native listener accepts integer client in RSI, pointers in
                // RDX/RCX, then unused inwater in R8.
                call(
                    &mut code,
                    33,
                    &[
                        ImportArg::Word(CLIENT),
                        ImportArg::Pointer(ORIGIN),
                        ImportArg::Pointer(AXES),
                        ImportArg::Integer(0),
                    ],
                    base,
                );
                call(
                    &mut code,
                    29,
                    &[ImportArg::Word(SOUND), ImportArg::Integer(0)],
                    base,
                );
                code.extend([0x48, 0x83, 0xc4, 8, 0x31, 0xc0, 0xc3]);
            }
            1 => {
                code.extend([0x48, 0x83, 0xec, 8]);
                call(&mut code, 0, &[ImportArg::Pointer(STOP_TEXT)], base);
                call(
                    &mut code,
                    29,
                    &[ImportArg::Word(SOUND), ImportArg::Integer(0)],
                    base,
                );
                code.extend([0x48, 0x83, 0xc4, 8]);
                // Preserve the shared shutdown5/finalizer6/7/8 exact-order trap.
                rip(&mut code, &[0xe9], 0x360c, base);
            }
            3 => {
                rip(&mut code, &[0xff, 0x05], BSS, base);
                rip(&mut code, &[0xc7, 0x05], INITIAL, base);
                let displacement = code.len() - 4;
                let adjusted = i32::from_le_bytes(code[displacement..].try_into().unwrap()) - 4;
                code[displacement..].copy_from_slice(&adjusted.to_le_bytes());
                code.extend(99u32.to_le_bytes());
                code.extend([0x48, 0x83, 0xec, 8]);
                call(&mut code, 44, &[ImportArg::Pointer(REFDEF)], base);
                call(
                    &mut code,
                    33,
                    &[
                        ImportArg::Word(CLIENT),
                        ImportArg::Pointer(ORIGIN),
                        ImportArg::Pointer(AXES),
                        ImportArg::Integer(0),
                    ],
                    base,
                );
                code.extend([0x48, 0x83, 0xc4, 8, 0xc3]);
            }
            2 if graphics => {
                code.extend([0x48, 0x83, 0xec, 8]);
                call(
                    &mut code,
                    79,
                    &[
                        ImportArg::Pointer(GRAPHICS_TEXT),
                        ImportArg::Pointer(REMAP_TEXT),
                        ImportArg::Pointer(REMAP_OFFSET),
                    ],
                    base,
                );
                code.extend([0x48, 0x83, 0xc4, 8, 0xc3]);
            }
            6 if graphics => {
                rip(&mut code, &[0x8b, 0x05], GRAPHICS_HANDLE, base);
                code.push(0xc3);
            }
            4 if graphics => {
                code.extend([0x48, 0x83, 0xec, 8]);
                call(&mut code, 49, &[ImportArg::Pointer(GLCONFIG)], base);
                code.extend([0x48, 0x83, 0xc4, 8]);
                rip(&mut code, &[0x8b, 0x05], GLCONFIG_ASPECT, base);
                code.push(0xc3);
            }
            4 | 5 | 6 => {
                let (ordinal, pointer) = match command {
                    4 => (72, COMMAND_OWNED),
                    5 => (16, CLIENT_TEXT),
                    _ => (15, COMMAND_OWNED),
                };
                code.extend([0x48, 0x83, 0xec, 8]);
                call(&mut code, ordinal, &[ImportArg::Pointer(pointer)], base);
                code.extend([0x48, 0x83, 0xc4, 8, 0xc3]);
            }
            _ => {
                let at = match command {
                    7 => INITIAL,
                    8 => INLINE,
                    9 => CLIENT,
                    _ => BSS,
                };
                rip(&mut code, &[0x8b, 0x05], at, base);
                code.push(0xc3);
            }
        }
    }
    assert!(
        code.len() <= 0x700,
        "native fixture code overlaps constructor"
    );
    file[0x2800..0x2800 + code.len()].copy_from_slice(&code);
    file
}

struct Source {
    time: u64,
    backend: asset_frames::Source,
    mixer: Mixer,
    reject_reset: Option<ClientId>,
    listener_receipts: [Option<(EntityId, ModuleId)>; 2],
    play_receipts: [u32; 2],
    reset_receipts: [u32; 2],
    video_prepares: usize,
    video_publishes: usize,
    video_reject: bool,
    video_publish_reject: bool,
    video_kinds: Vec<Option<qa_app::renderer::Kind>>,
    video_configuration: Arc<qa_core::presentation::RendererConfiguration>,
    video_window: qa_core::presentation::WindowRequest,
    video_pending_window: Option<qa_core::presentation::WindowRequest>,
    views: Vec<(
        qa_render::Viewport,
        qa_render::world::WorldId,
        qa_render::world::WorldPresentationId,
    )>,
}
impl Source {
    fn step(&mut self, host: &mut FrameHost) {
        host.frame(self, true);
    }
}
impl FrameSource for Source {
    fn video_kind(&self) -> Option<qa_app::renderer::Kind> {
        Some(qa_app::renderer::Kind::Cpu)
    }
    fn video_window_request(&self) -> Option<qa_core::presentation::WindowRequest> {
        Some(self.video_window)
    }
    fn prepare_video(
        &mut self,
        policy: qa_app::renderer::VideoPolicy,
        assets: &Assets,
        limits: Limits,
    ) -> Result<qa_app::renderer::PreparedVideo, String> {
        self.video_prepares += 1;
        self.video_kinds.push(policy.kind);
        if self.video_reject || matches!(policy.kind, Some(qa_app::renderer::Kind::Gl)) {
            return Err("headless candidate preparation rejected".into());
        }
        let mut configuration = self.video_configuration.as_ref().clone();
        configuration.display.width = policy.window.width;
        configuration.display.height = policy.window.height;
        configuration.display.window_aspect = policy.window.window_aspect;
        let candidate = qa_app::renderer::PreparedVideo::cpu(
            Arc::new(configuration),
            assets,
            limits,
            policy.cpu_bands,
        )?;
        self.video_pending_window = Some(policy.window);
        Ok(candidate)
    }
    fn publish_video(
        &mut self,
        candidate: qa_app::renderer::PreparedVideo,
    ) -> Result<Arc<qa_core::presentation::RendererConfiguration>, String> {
        if self.video_publish_reject {
            return Err("headless candidate publication rejected".into());
        }
        let mut window = self
            .video_pending_window
            .take()
            .ok_or("headless window request is missing")?;
        let (cpu, configuration) = candidate.into_cpu()?;
        if !configuration.display.fullscreen {
            window.mode = qa_core::presentation::WindowMode::Windowed;
        }
        self.backend.cpu = cpu;
        self.video_window = window;
        self.video_configuration = Arc::clone(&configuration);
        self.video_publishes += 1;
        Ok(configuration)
    }
    fn asset_preparer(&mut self) -> Option<&mut (dyn qa_app::renderer::AssetPreparer + '_)> {
        if self.backend.enabled {
            Some(&mut self.backend)
        } else {
            None
        }
    }
    fn begin_frame(&mut self) -> Duration {
        self.time += 100;
        Duration::from_millis(self.time)
    }
    fn poll_events(&mut self, queue: &mut SysEventQueue) {
        queue
            .push(SysEvent {
                time: EventTime::from_milliseconds(self.time),
                kind: EventKind::Time,
            })
            .unwrap();
    }
    fn wait_time(&mut self, _: Duration) -> Duration {
        panic!("uncapped fixture")
    }
    fn elapsed(&self) -> Duration {
        Duration::ZERO
    }
    fn present(&mut self) {}
    fn listeners(&mut self, listeners: &[Option<Listener>; SeatId::COUNT], volume: f32) {
        self.mixer.listen(listeners, volume);
    }
    fn sound(&mut self, bank: Option<&Bank>, event: SoundEvent) -> bool {
        let accepted = self.mixer.sound(bank.unwrap(), event);
        if accepted
            && event.action == SoundAction::Respatialize
            && let (Some(client), Some(lifetime), Some(module)) =
                (event.client, event.client_lifetime, event.listener_owner)
            && let Some(receipt) = self.listener_receipts.get_mut(client.0 as usize)
        {
            *receipt = Some((lifetime, module));
        }
        if accepted
            && event.action == SoundAction::Play
            && let Some(client) = event.client
            && let Some(receipt) = self.play_receipts.get_mut(client.0 as usize)
        {
            *receipt += 1;
        }
        accepted
    }
    fn reset_presentation(&mut self, client: ClientId, lifetime: EntityId) -> bool {
        if self.reject_reset == Some(client) {
            return false;
        }
        if let Some(count) = self.reset_receipts.get_mut(client.0 as usize) {
            *count += 1;
        }
        self.mixer.reset_client(client, lifetime, false);
        if let Some(receipt) = self.listener_receipts.get_mut(client.0 as usize) {
            *receipt = None;
        }
        if let Some(receipt) = self.play_receipts.get_mut(client.0 as usize) {
            *receipt = 0;
        }
        true
    }
    fn render(
        &mut self,
        scene: Option<&mut SceneComposer>,
        resources: Option<qa_app::model_assets::ModelView<'_>>,
        native: &dyn qa_render::shader::reference::NativeShaders,
        _: &[Option<qa_app::host::ClientView>; SeatId::COUNT],
        _: &[Option<qa_app::client_scene::ClientSceneInput<'_>>; SeatId::COUNT],
        _: &[Option<HudView<'_>>; SeatId::COUNT],
        _: qa_console::presentation::ConsoleView<'_>,
    ) {
        self.views.clear();
        let (Some(scene), Some(resources)) = (scene, resources) else {
            return;
        };
        let Some(mut packet) = scene.finish_frame() else {
            return;
        };
        for command in packet.commands() {
            if let Command::View(view) = command {
                assert_eq!(view.refdef.identity_light, 0.625);
                assert_eq!(
                    view.refdef.perspective_step,
                    qa_render::PerspectiveStep::Sixteen
                );
                for surface in packet.surfaces(view.scene.surfaces) {
                    self.views
                        .push((view.refdef.viewport, surface.world, surface.presentation));
                }
            }
        }
        packet.resolve_shaders(resources.assets, native);
        self.backend.cpu.render(&packet, resources.assets);
        assert!(scene.recycle(packet).is_ok());
    }
}

fn setup(
    module: Vec<u8>,
) -> (
    files::Fixture,
    FrameHost,
    Source,
    [ClientId; 2],
    [EntityId; 2],
) {
    setup_renderers(module, None)
}
fn setup_renderers(
    module: Vec<u8>,
    ui: Option<Vec<u8>>,
) -> (
    files::Fixture,
    FrameHost,
    Source,
    [ClientId; 2],
    [EntityId; 2],
) {
    setup_renderers_with_graphics(module, ui, false)
}
fn setup_renderers_with_graphics(
    module: Vec<u8>,
    ui: Option<Vec<u8>>,
    graphics: bool,
) -> (
    files::Fixture,
    FrameHost,
    Source,
    [ClientId; 2],
    [EntityId; 2],
) {
    setup_renderers_with_images(module, ui, graphics, false)
}
fn setup_renderers_with_images(
    module: Vec<u8>,
    ui: Option<Vec<u8>>,
    graphics: bool,
    image_backed: bool,
) -> (
    files::Fixture,
    FrameHost,
    Source,
    [ClientId; 2],
    [EntityId; 2],
) {
    let has_ui = ui.is_some();
    let mut entries = vec![
        ("vm/restart", module),
        ("maps/a.bsp", files::world_map(8.)),
        ("maps/b.bsp", files::world_map(8.)),
        ("maps/c.bsp", files::world_map(8.)),
        ("sound/restart.wav", samples::wav()),
    ];
    if let Some(ui) = ui {
        entries.push(("vm/ui", ui));
    }
    if graphics {
        let script = if image_backed {
            b"gfx/2d/bigchars { { map textures/restart.tga } }\nwhite { { map $whiteimage } }\nconsole { { map $whiteimage } }\ngraphics/after-engine { { map textures/restart.tga } }\ngraphics/replacement { { map $whiteimage rgbGen const ( 0.25 0.5 0.75 ) } }\n".as_slice()
        } else {
            b"gfx/2d/bigchars { { map $whiteimage } }\nwhite { { map $whiteimage } }\nconsole { { map $whiteimage } }\ngraphics/after-engine { { map $whiteimage } }\ngraphics/replacement { { map $whiteimage rgbGen const ( 0.25 0.5 0.75 ) } }\n".as_slice()
        };
        entries.push(("scripts/engine-graphics.shader", script.to_vec()));
        if image_backed {
            let pixels = (0..64)
                .map(|index| {
                    let x = index % 8;
                    let y = index / 8;
                    [
                        16 + x as u8 * 27,
                        16 + y as u8 * 25,
                        if (x + y) % 3 == 0 { 220 } else { 20 },
                        255,
                    ]
                })
                .collect::<Vec<_>>();
            entries.push(("textures/restart.tga", files::tga(8, 8, &pixels)));
        }
    }
    let mut fixture = files::Fixture::new(&entries);
    let mut runtime = Runtime::load(2, []).unwrap();
    runtime.vfs = std::mem::take(&mut fixture.vfs);
    let ids = std::array::from_fn(|seat| {
        runtime
            .connect_local(
                SeatId::new(seat as u8).unwrap(),
                SpawnAnchor {
                    position: Vec3([32., 0., 0.]),
                    angles: Vec3([0., 180., 0.]),
                    entity: 0,
                    fixture_fallback: false,
                },
                ClientPolicy {
                    input: RuleSetId::QuakeWorld,
                    client: RuleSetId::Quake3,
                    movement: RuleSetId::Quake,
                    trace: RuleSetId::Quake2,
                },
                Protocol::Quake3_68,
                seat as u32,
            )
            .unwrap()
    });
    let mut requests = Vec::new();
    let mut specs = vec![
        Q3Spec::parse("cgame:0:vm/restart").unwrap(),
        Q3Spec::parse("cgame:1:vm/restart").unwrap(),
    ];
    if has_ui {
        specs.extend([
            Q3Spec::parse("ui:0:vm/ui").unwrap(),
            Q3Spec::parse("ui:1:vm/ui").unwrap(),
        ]);
    }
    load_q3(
        &mut runtime,
        &specs,
        &mut requests,
        TickRate::FrameDriven,
        None,
    )
    .unwrap();
    assert!(requests.iter().all(|request| request.q3_image.is_some()));
    assert!(requests.iter().skip(2).all(|request| {
        request.frame.client.is_none()
            && request
                .initialize
                .iter()
                .all(|export| export.client.is_none())
    }));
    let anchors = [requests[0].anchor, requests[1].anchor];
    let mut assets = Assets::load().unwrap();
    let palette = files::palette(&mut assets);
    let (scene, _) = SceneComposer::load(Limits::default(), &mut assets, vec![]).unwrap();
    let profile = ModelRequest::native("", RuleSetId::Quake3, None).profile();
    let mut owner = ModelAssets::new(assets, [profile], [], Default::default()).unwrap();
    owner
        .adopt_palettes(
            qa_app::render_settings::PaletteRegistrations::from_registered([
                Some(palette),
                None,
                None,
                None,
                None,
            ]),
        )
        .unwrap();
    let console = graphics.then(|| {
        ConsoleAssets::load(
            &runtime.vfs,
            &mut owner,
            RuleSetId::Quake3,
            qa_render::material::resources::ImageSettings::native(RuleSetId::Quake3),
            64,
            48,
        )
        .unwrap()
    });
    let mut backend = asset_frames::Source::load(owner.assets());
    runtime.model_assets = Some(owner);
    runtime.scene = Some(scene);
    let mut host = FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        requests,
    )
    .unwrap();
    let bands = host.console.cvars.find("r_cpuBands").unwrap();
    host.console.cvars.set_text(bands, "1").unwrap();
    host.console.cvars.set_latch_active(bands, true);
    let display = qa_core::presentation::DisplayMode {
        width: 64,
        height: 48,
        frequency: 60,
        color_bits: 32,
    };
    host.console
        .cvars
        .set_video_policy(
            Arc::new(qa_core::presentation::VideoCapabilities {
                desktop: display,
                fullscreen: Box::default(),
            }),
            display,
        )
        .unwrap();
    for (name, value) in [
        ("r_mode", "-1"),
        ("r_customwidth", "64"),
        ("r_customheight", "48"),
        ("r_fullscreen", "0"),
    ] {
        let handle = host.console.cvars.find(name).unwrap();
        host.console.cvars.set_text(handle, value).unwrap();
    }
    host.local_clients[0] = Some(ids[0]);
    host.local_clients[1] = Some(ids[1]);
    if let Some(console) = console {
        let mut defaults = [None; 5];
        defaults[RuleSetId::Quake3 as usize] = Some(profile);
        host.configure_native_scenes(&defaults, &[[None; 5]; SeatId::COUNT], 64, 48)
            .unwrap();
        let effects = qa_render::effects::Effects::load(
            Arc::new(qa_core::effects::EffectCatalog::new(
                Box::default(),
                [None; 5],
            )),
            Limits::default().particles / SeatId::COUNT,
            SeatId::COUNT,
            &[None],
        )
        .unwrap();
        let configuration = Arc::new(qa_core::presentation::RendererConfiguration {
            display: qa_core::presentation::DisplayConfiguration {
                width: 64,
                height: 48,
                ..Default::default()
            },
            ..Default::default()
        });
        let huds = std::array::from_fn(|_| None);
        let (assets, staged) = host.stage_engine_graphics(&console, &huds).unwrap();
        let (renderer, scene) = Renderer::load(
            qa_app::renderer::Context::Cpu(configuration),
            64,
            48,
            assets,
            vec![],
            Some(qa_render::cpu::RasterBands::One),
            console,
            huds,
            effects,
        )
        .unwrap();
        staged.commit();
        host.runtime.scene = Some(scene);
        backend = asset_frames::Source::load(host.runtime.model_assets.as_ref().unwrap().assets());
        assert_eq!(renderer.admitted_assets(), backend.cpu.admitted_assets());
    }
    host.publish_model_admission(backend.cpu.admitted_assets().unwrap())
        .unwrap();
    host.bind_client_model_assets(ids.map(|id| (id, profile, None)));
    let mut defaults = [None; 5];
    defaults[RuleSetId::Quake3 as usize] = Some(profile);
    for presentation in &mut host.local_presentations[..2] {
        presentation.identity_light = 0.625;
        presentation.perspective_step = qa_render::PerspectiveStep::Sixteen;
    }
    host.configure_native_scenes(&defaults, &[[None; 5]; SeatId::COUNT], 64, 48)
        .unwrap();
    qa_app::audio::begin_registration(&mut host, &["sound/restart.wav".into()], RuleSetId::Quake3)
        .unwrap();
    qa_app::audio::finish_registration(&mut host).unwrap();
    let rate = host.runtime.sound_bank().unwrap().rate;
    let [width, height] = backend.cpu.dimensions();
    let video_configuration = Arc::new(qa_core::presentation::RendererConfiguration {
        display: qa_core::presentation::DisplayConfiguration {
            width,
            height,
            ..Default::default()
        },
        ..Default::default()
    });
    host.runtime.renderer_configuration = Some(Arc::clone(&video_configuration));
    let source = Source {
        time: 0,
        video_prepares: 0,
        video_publishes: 0,
        video_reject: false,
        video_publish_reject: false,
        video_kinds: Vec::new(),
        video_configuration,
        video_window: qa_core::presentation::WindowRequest::windowed(width, height),
        video_pending_window: None,
        backend,
        mixer: Mixer::load(rate, 16, SeatId::COUNT),
        reject_reset: None,
        listener_receipts: [None; 2],
        play_receipts: [0; 2],
        reset_receipts: [0; 2],
        views: Vec::new(),
    };
    (fixture, host, source, ids, anchors)
}

fn queue_gamestate(
    host: &mut FrameHost,
    client: ClientId,
    map: &str,
    native: u32,
    malformed: bool,
) {
    let info = format!("\\mapname\\{map}");
    let world = format!("maps/{map}.bsp");
    let channel = &mut host
        .runtime
        .network
        .get_mut(client, Endpoint::Server)
        .unwrap()
        .channel;
    let command = channel.command_state().unwrap().queued;
    let mut bytes = [0; 32768];
    let length = channel
        .encode_server_output(&mut bytes, |writer, _| {
            write_gamestate(
                writer,
                command,
                native,
                0,
                [(0, info.as_bytes()), (33, world.as_bytes())],
                std::iter::empty(),
            )
        })
        .unwrap();
    let length = if malformed { length / 2 } else { length };
    let time = EventTime::from_milliseconds(100);
    channel
        .prepare_server_message(&bytes[..length], time)
        .unwrap()
        .unwrap();
    loop {
        let packet = channel.pending_packet().unwrap();
        host.runtime
            .loopback
            .send(Endpoint::Server, client, packet.bytes)
            .unwrap();
        channel.submitted(time).unwrap();
        if !channel.pending_fragments() {
            break;
        }
        channel.prepare_output(time).unwrap().unwrap();
    }
    host.drain(&mut FrameResult::default());
}
fn query(
    host: &mut FrameHost,
    module: ModuleId,
    command: u32,
) -> Result<u32, qa_session::dispatch::CallError> {
    host.call_module_export(
        module,
        Export {
            client: Some(ClientId(u32::from(module.0) - 1)),
            ..Export::words(CallbackId(command), &[])
        },
        ThinkTime::Milliseconds(9000),
    )?;
    Ok(
        match host.module_counts(module).unwrap().last_result.unwrap() {
            ModuleResult::Qvm(value) => value as u32,
            ModuleResult::Native(value) => value as u32,
            _ => unreachable!(),
        },
    )
}
fn queue_snapshot(host: &mut FrameHost, client: ClientId, native: u32, time: i32) {
    use qa_network::{
        projection::{PlayerContext, PlayerProjection},
        snapshots::{Frame, ReceivedFrame},
        states,
    };
    let projection = PlayerProjection::load(Protocol::Quake3_68, &[]);
    let mut player = PlayerState::default();
    player.body.position = Vec3([32., 0., 0.]);
    player.view.fov_x = Some(90.);
    player.movement.command_time_ms = time;
    let mut words = [0; states::PLAYER_WORDS];
    assert!(projection.reduce(
        &player,
        &PlayerContext {
            client_number: Some(native),
            ..Default::default()
        },
        &mut words,
        |_, _| None,
        |_| None,
    ));
    let channel = &mut host
        .runtime
        .network
        .get_mut(client, Endpoint::Server)
        .unwrap()
        .channel;
    let sequence = channel.send_state().sequence;
    let frame = ReceivedFrame::Quake3(Frame {
        sequence,
        time: ThinkTime::Milliseconds(i64::from(time)),
        command: 0,
        flags: 0,
        qw_peers: 0,
        qw_dead: 0,
        areas: &[],
        player: &words,
        entities: &[],
    });
    let mut bytes = [0; 16384];
    let length = channel
        .encode_server_output(&mut bytes, |writer, channel| {
            channel.write_snapshot_candidate(writer, frame, None, 64)
        })
        .unwrap();
    channel.publish_written_snapshot(frame, None).unwrap();
    let now = EventTime::from_milliseconds(time as u64);
    channel
        .prepare_snapshot(sequence, None, &bytes[..length], now)
        .unwrap()
        .unwrap();
    loop {
        assert!(
            channel
                .submit_with(now, |packet| host
                    .runtime
                    .loopback
                    .send(Endpoint::Server, client, packet)
                    .is_ok())
                .unwrap()
                .is_some()
        );
        if !channel.pending_fragments() {
            break;
        }
        channel.prepare_output(now).unwrap().unwrap();
    }
    host.drain(&mut FrameResult::default());
}
fn until_running(host: &mut FrameHost, source: &mut Source) {
    for _ in 0..8 {
        source.step(host);
        if [ModuleId(1), ModuleId(2)]
            .into_iter()
            .all(|module| host.module_state(module) == Some(State::Running))
        {
            return;
        }
    }
    let modules = [ModuleId(1), ModuleId(2)].map(|module| {
        format!(
            "module {}: state {:?}, counts {:?}, unknown {:?}",
            module.0,
            host.module_state(module),
            host.module_counts(module),
            host.last_unknown_call(module),
        )
    });
    let console = host
        .console
        .scrollback
        .chunks(&host.runtime.server.events.texts)
        .map(String::from_utf8_lossy)
        .collect::<String>();
    panic!(
        "bounded queued startup did not initialize both cgames\n{}\n{}\nworlds {:?}, resource rejections {:?}, backend attempts {}\nconsole: {console}",
        modules[0],
        modules[1],
        host.local_worlds,
        host.runtime.client_resources[..2]
            .iter()
            .map(|resources| resources.as_ref().map(|resources| resources.state.rejected))
            .collect::<Vec<_>>(),
        source.backend.attempts,
    );
}
fn marker_count(host: &FrameHost, marker: &[u8]) -> usize {
    host.console
        .scrollback
        .chunks(&host.runtime.server.events.texts)
        .filter(|text| text.windows(marker.len()).any(|part| part == marker))
        .count()
}
fn paint(host: &FrameHost, source: &mut Source) {
    let mut pcm = [0; 16];
    source
        .mixer
        .paint(host.runtime.sound_bank().unwrap(), &mut pcm);
    assert!(pcm.iter().any(|&sample| sample != 0));
}

fn verify(module: Vec<u8>) {
    let (_fixture, mut host, mut source, ids, anchors) = setup(module);
    let lifetimes = ids.map(|id| host.runtime.server.clients[id.0 as usize].entity);
    let roles = ids.map(|id| {
        let row = &host.runtime.server.clients[id.0 as usize];
        (
            row.input_rules,
            row.client_rules,
            row.player.movement_rules,
            row.player.trace_rules,
        )
    });
    queue_gamestate(&mut host, ids[0], "a", 11, false);
    queue_gamestate(&mut host, ids[1], "a", 22, false);
    until_running(&mut host, &mut source);
    assert_eq!(query(&mut host, ModuleId(1), 9), Ok(11));
    assert_eq!(query(&mut host, ModuleId(2), 9), Ok(22));
    assert_eq!(query(&mut host, ModuleId(1), 8), Ok(2));
    // CG_INIT itself checks the pristine initialized/BSS bytes. A normal
    // startup frame may also draw a snapshot published by the local SERVER.
    let draws = query(&mut host, ModuleId(1), 10).unwrap();
    queue_snapshot(&mut host, ids[0], 11, 1000);
    queue_snapshot(&mut host, ids[1], 22, 1000);
    source.step(&mut host);
    assert_eq!(query(&mut host, ModuleId(1), 7), Ok(99));
    assert_eq!(query(&mut host, ModuleId(1), 10), Ok(draws + 1));
    assert_eq!(marker_count(&host, b"restart init"), 2);
    paint(&host, &mut source);
    let old_world = host.local_worlds[0].unwrap();
    assert_eq!(host.local_worlds[1], Some(old_world));
    assert!(
        source
            .views
            .iter()
            .any(|(viewport, world, presentation)| viewport.x == 2
                && (*world, *presentation) == old_world)
    );
    assert!(
        source
            .views
            .iter()
            .any(|(viewport, world, presentation)| viewport.x == 34
                && (*world, *presentation) == old_world)
    );

    let errors = host.runtime.network.command_errors;
    queue_gamestate(&mut host, ids[0], "b", 19, true);
    assert!(host.runtime.network.command_errors > errors);
    assert_eq!(query(&mut host, ModuleId(1), 9), Ok(11));
    source.step(&mut host);
    assert_eq!(host.module_state(ModuleId(1)), Some(State::Running));
    assert_eq!(marker_count(&host, b"restart shutdown"), 0);

    source.reject_reset = Some(ids[0]);
    queue_gamestate(&mut host, ids[0], "b", 19, false);
    assert!(query(&mut host, ModuleId(1), 10).is_err());
    source.step(&mut host);
    assert_eq!(host.module_state(ModuleId(1)), Some(State::Pending));
    let retired_calls = host.module_counts(ModuleId(1)).unwrap().calls;
    let peer_calls = host.module_counts(ModuleId(2)).unwrap().calls;
    for _ in 0..2 {
        source.step(&mut host);
    }
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().calls,
        retired_calls
    );
    assert!(host.module_counts(ModuleId(2)).unwrap().calls > peer_calls);
    assert!(
        source
            .views
            .iter()
            .all(|(viewport, world, presentation)| viewport.x == 34
                && (*world, *presentation) == old_world)
    );
    queue_gamestate(&mut host, ids[0], "c", 33, false);
    source.step(&mut host);
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().calls,
        retired_calls
    );
    source.backend.enabled = false;
    source.reject_reset = None;
    host.runtime.vfs.take_lookup_count();
    let attempts = source.backend.attempts;
    for _ in 0..2 {
        source.step(&mut host);
    }
    assert_eq!(host.module_state(ModuleId(1)), Some(State::Pending));
    assert_eq!(host.runtime.vfs.take_lookup_count(), 0);
    assert_eq!(source.backend.attempts, attempts);
    assert!(
        host.runtime
            .model_assets
            .as_ref()
            .unwrap()
            .catalog()
            .loaded_world(b"maps/b.bsp")
            .is_none()
    );
    assert!(
        host.runtime
            .model_assets
            .as_ref()
            .unwrap()
            .catalog()
            .loaded_world(b"maps/c.bsp")
            .is_none()
    );

    source.backend.enabled = true;
    until_running(&mut host, &mut source);
    assert_eq!(query(&mut host, ModuleId(1), 9), Ok(33));
    assert_eq!(query(&mut host, ModuleId(1), 8), Ok(2));
    let renewed_draws = query(&mut host, ModuleId(1), 10).unwrap();
    queue_snapshot(&mut host, ids[0], 33, 2000);
    source.step(&mut host);
    assert_eq!(query(&mut host, ModuleId(1), 10), Ok(renewed_draws + 1));
    assert_eq!(query(&mut host, ModuleId(1), 7), Ok(99));
    assert_eq!(query(&mut host, ModuleId(2), 9), Ok(22));
    assert_eq!(marker_count(&host, b"restart init"), 3);
    assert_eq!(marker_count(&host, b"restart shutdown"), 1);
    let owner = host.runtime.model_assets.as_ref().unwrap();
    assert!(owner.catalog().loaded_world(b"maps/b.bsp").is_none());
    let loaded = owner.catalog().loaded_world(b"maps/c.bsp").unwrap();
    let current = host.local_worlds[0].unwrap();
    assert_eq!(current.0, loaded.render);
    assert_ne!(current, old_world);
    assert_eq!(host.local_worlds[1], Some(old_world));
    assert!(source.views.iter().any(
        |(viewport, world, presentation)| viewport.x == 2 && (*world, *presentation) == current
    ));
    assert!(
        source
            .views
            .iter()
            .any(|(viewport, world, presentation)| viewport.x == 34
                && (*world, *presentation) == old_world)
    );
    assert_eq!(
        source.backend.cpu.admitted_assets(),
        Some(owner.assets().counts())
    );
    assert_eq!(
        source.listener_receipts[0],
        Some((lifetimes[0], ModuleId(1)))
    );
    assert!(source.play_receipts[0] > 0);
    source.mixer.reset_client(ids[1], lifetimes[1], false);
    paint(&host, &mut source);
    for seat in 0..2 {
        let row = &host.runtime.server.clients[ids[seat].0 as usize];
        assert_eq!(row.entity, lifetimes[seat]);
        assert_eq!(host.local_clients[seat], Some(ids[seat]));
        assert_eq!(
            (
                row.input_rules,
                row.client_rules,
                row.player.movement_rules,
                row.player.trace_rules
            ),
            roles[seat]
        );
        assert!(
            host.runtime
                .server
                .entities
                .resolve(anchors[seat])
                .is_some()
        );
        assert_eq!(
            host.module_counts(ModuleId(seat as u16 + 1)).unwrap().traps,
            0
        );
    }
    // This accepted generation recreates pristine memory, then CG_INIT's
    // known Error import rejects only the invoking cgame.
    queue_gamestate(&mut host, ids[0], "c", 44, false);
    assert!(query(&mut host, ModuleId(1), 9).is_err());
    for _ in 0..8 {
        source.step(&mut host);
        if host.module_state(ModuleId(1)) == Some(State::Failed) {
            break;
        }
    }
    assert_eq!(host.module_state(ModuleId(1)), Some(State::Failed));
    assert_eq!(host.module_state(ModuleId(2)), Some(State::Running));
    assert_eq!(marker_count(&host, b"intentional CG_INIT failure"), 1);
    assert_eq!(marker_count(&host, b"restart init"), 3);
    assert_eq!(marker_count(&host, b"restart shutdown"), 2);
    let failed_calls = host.module_counts(ModuleId(1)).unwrap().calls;
    let failed_traps = host.module_counts(ModuleId(1)).unwrap().traps;
    assert_eq!(failed_traps, 1);
    assert!(host.runtime.server.entities.resolve(anchors[0]).is_none());
    assert!(query(&mut host, ModuleId(1), 9).is_err());
    let peer_calls = host.module_counts(ModuleId(2)).unwrap().calls;
    for _ in 0..3 {
        source.step(&mut host);
    }
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, failed_calls);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, failed_traps);
    assert!(host.module_counts(ModuleId(2)).unwrap().calls > peer_calls);
    assert_eq!(host.module_state(ModuleId(2)), Some(State::Running));
    assert_eq!(
        host.runtime.server.clients[ids[1].0 as usize].entity,
        lifetimes[1]
    );
    assert!(host.runtime.server.entities.resolve(anchors[1]).is_some());
    assert_eq!(query(&mut host, ModuleId(2), 9), Ok(22));
    assert!(!source.views.is_empty());
    assert!(source.views.iter().all(|(viewport, world, presentation)| {
        viewport.x == 34 && (*world, *presentation) == old_world
    }));
    host.shutdown_modules();
    assert_eq!(host.module_state(ModuleId(1)), Some(State::Failed));
    assert_eq!(host.module_state(ModuleId(2)), Some(State::Stopped));
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, failed_traps);
    assert_eq!(host.module_counts(ModuleId(2)).unwrap().traps, 0);
}

fn command_listing(host: &mut FrameHost, source: &mut Source) -> [usize; 2] {
    host.console
        .scrollback
        .clear(&mut host.runtime.server.events.texts);
    host.console
        .append("cmdlist\n", Context::default())
        .unwrap();
    host.execute_commands();
    source.step(host);
    let mut counts = [0; 2];
    for text in host
        .console
        .scrollback
        .chunks(&host.runtime.server.events.texts)
    {
        for (count, name) in counts
            .iter_mut()
            .zip([b"restart_owned".as_slice(), b"restart_shared"])
        {
            *count += usize::from(text == name);
        }
    }
    counts
}
fn verify_commands(module: Vec<u8>) {
    let (_fixture, mut host, mut source, ids, anchors) = setup(module);
    queue_gamestate(&mut host, ids[0], "a", 11, false);
    queue_gamestate(&mut host, ids[1], "a", 22, false);
    until_running(&mut host, &mut source);
    assert_eq!(command_listing(&mut host, &mut source), [1, 1]);
    assert_eq!(query(&mut host, ModuleId(1), 4), Ok(0));
    assert_eq!(command_listing(&mut host, &mut source), [0, 1]);
    assert_eq!(query(&mut host, ModuleId(1), 6), Ok(0));
    let queued = host
        .runtime
        .network
        .get(ids[0], Endpoint::Client)
        .unwrap()
        .channel
        .command_state()
        .unwrap()
        .queued;
    assert_eq!(query(&mut host, ModuleId(1), 5), Ok(0));
    assert_eq!(
        host.runtime
            .network
            .get(ids[0], Endpoint::Client)
            .unwrap()
            .channel
            .command_state()
            .unwrap()
            .queued,
        queued + 1
    );

    source.reject_reset = Some(ids[0]);
    queue_gamestate(&mut host, ids[0], "c", 33, false);
    source.step(&mut host);
    assert_eq!(host.module_state(ModuleId(1)), Some(State::Pending));
    // Native completion names stay registered while the same anchor awaits
    // its replacement image, and peer registrations remain independent.
    assert_eq!(command_listing(&mut host, &mut source), [1, 1]);
    source.reject_reset = None;
    until_running(&mut host, &mut source);
    assert_eq!(command_listing(&mut host, &mut source), [1, 1]);
    assert!(host.shutdown_module(ModuleId(1)));
    assert!(host.runtime.server.entities.resolve(anchors[0]).is_none());
    assert_eq!(command_listing(&mut host, &mut source), [0, 1]);
    assert_eq!(host.module_state(ModuleId(2)), Some(State::Running));
    assert!(host.runtime.server.entities.resolve(anchors[1]).is_some());
    assert!(host.shutdown_module(ModuleId(2)));
    assert_eq!(command_listing(&mut host, &mut source), [0, 0]);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
    assert_eq!(host.module_counts(ModuleId(2)).unwrap().traps, 0);
}

fn ui_query(
    host: &mut FrameHost,
    module: ModuleId,
    command: u32,
) -> Result<u32, qa_session::dispatch::CallError> {
    host.call_module_export(
        module,
        Export::words(CallbackId(command), &[]),
        ThinkTime::Milliseconds(9000),
    )?;
    Ok(
        match host.module_counts(module).unwrap().last_result.unwrap() {
            ModuleResult::Qvm(value) => value as u32,
            ModuleResult::Native(value) => value as u32,
            _ => unreachable!(),
        },
    )
}

fn renderer_markers(host: &FrameHost) -> Vec<u8> {
    host.console
        .scrollback
        .chunks(&host.runtime.server.events.texts)
        .flat_map(|chunk| chunk.split(|&byte| byte == b'\n'))
        .filter_map(|line| match line {
            b"renderer ui init" => Some(b'U'),
            b"renderer ui shutdown" => Some(b'u'),
            b"restart init" => Some(b'C'),
            b"restart shutdown" => Some(b'c'),
            _ => None,
        })
        .collect()
}

fn verify_standalone_ui(ui: Vec<u8>) {
    let mut fixture = files::Fixture::new(&[("vm/ui", ui)]);
    let mut runtime = Runtime::load(2, []).unwrap();
    runtime.vfs = std::mem::take(&mut fixture.vfs);
    let mut requests = Vec::new();
    load_q3(
        &mut runtime,
        &[
            Q3Spec::parse("ui:0:vm/ui").unwrap(),
            Q3Spec::parse("ui:1:vm/ui").unwrap(),
        ],
        &mut requests,
        TickRate::FrameDriven,
        None,
    )
    .unwrap();
    assert!(
        requests
            .iter()
            .all(|request| request.q3_image.is_some() && request.frame.client.is_none())
    );
    let mut host = FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        requests,
    )
    .unwrap();
    assert!(host.runtime.scene.is_none());
    host.initialize_modules(qa_app::modules::Phase::Client);
    for module in [ModuleId(1), ModuleId(2)] {
        assert_eq!(host.module_state(module), Some(State::Running));
        assert_eq!(ui_query(&mut host, module, 7), Ok(7));
    }
    host.client_modules();
    for module in [ModuleId(1), ModuleId(2)] {
        assert_eq!(ui_query(&mut host, module, 7), Ok(99));
        assert_eq!(ui_query(&mut host, module, 10), Ok(1));
        assert_eq!(host.module_counts(module).unwrap().traps, 0);
    }
    let protocol = Protocol::NetQuake15;
    let client = host
        .runtime
        .connect_client(
            SeatId::FIRST,
            ClientPolicy {
                client: RuleSetId::Quake,
                input: RuleSetId::Quake,
                movement: RuleSetId::Quake,
                trace: RuleSetId::Quake,
            },
            qa_app::snapshots::NativePlayerBinding::Awaiting,
            qa_network::ingress::Connection {
                phase: qa_network::ingress::ConnectionPhase::Connecting(
                    qa_network::ingress::NqConnect::new(),
                ),
                route: qa_network::ingress::Route {
                    socket: 27,
                    peer: Peer::Socket("127.0.0.1:26000".parse().unwrap()),
                },
                channel: qa_network::channel::Channel::load(
                    protocol.channel(),
                    Endpoint::Client,
                    8192,
                    16,
                )
                .unwrap(),
                output: None,
                pending_output: None,
                commands: Some(qa_network::commands::connection::Commands::load(
                    protocol, 8192,
                )),
            },
            None,
        )
        .unwrap();
    host.local_clients[0] = Some(client);
    // A reserved pending socket is not a renderer association. Both original
    // private menu generations continue without shutdown/recreation or I/O.
    host.client_modules();
    for module in [ModuleId(1), ModuleId(2)] {
        assert_eq!(ui_query(&mut host, module, 7), Ok(99));
        assert_eq!(ui_query(&mut host, module, 10), Ok(2));
    }
    host.shutdown_modules();
    for module in [ModuleId(1), ModuleId(2)] {
        assert_eq!(host.module_state(module), Some(State::Stopped));
        assert_eq!(host.module_counts(module).unwrap().traps, 0);
    }
}

fn verify_renderer_lifecycle(module: Vec<u8>, ui: Vec<u8>) {
    let (_fixture, mut host, mut source, ids, _) =
        setup_renderers_with_graphics(module, Some(ui), true);
    // Both real cgame requests deliberately precede both UI requests. UI owns
    // no CLIENT service binding and needs no gamestate to initialize menus.
    // The renderer must also tolerate an optional resource owner not yet bound.
    let resources = host.runtime.client_resources[ids[0].0 as usize].take();
    host.initialize_modules_with_driver(qa_app::modules::Phase::Client, &mut source.driver());
    host.runtime.client_resources[ids[0].0 as usize] = resources;
    for module in [ModuleId(1), ModuleId(2)] {
        assert_eq!(host.module_state(module), Some(State::Pending));
    }
    for module in [ModuleId(3), ModuleId(4)] {
        assert_eq!(
            host.module_state(module),
            Some(State::Running),
            "module {}, counts {:?}, console: {}",
            module.0,
            host.module_counts(module),
            host.console
                .scrollback
                .chunks(&host.runtime.server.events.texts)
                .map(String::from_utf8_lossy)
                .collect::<String>(),
        );
        assert_eq!(ui_query(&mut host, module, 7), Ok(7));
        assert_eq!(ui_query(&mut host, module, 8), Ok(4));
    }
    let mut replacement = ModelRequest::native("", RuleSetId::Quake3, None).profile();
    replacement.rules = RuleSetId::Quake2;
    replacement.image_settings =
        qa_render::material::resources::ImageSettings::native(RuleSetId::Quake2);
    let mut defaults = [None; 5];
    defaults[RuleSetId::Quake3 as usize] = Some(replacement);
    assert!(
        host.configure_native_scenes(&defaults, &[[None; 5]; SeatId::COUNT], 64, 48)
            .is_err()
    );
    // Later CM/R init and exact-view assertions still use the original profile;
    // the rejected cold configuration must not retarget any old instance.
    source.step(&mut host);
    assert_eq!(renderer_markers(&host), b"UU");
    let before = renderer_markers(&host).len();
    queue_gamestate(&mut host, ids[0], "a", 11, false);
    queue_gamestate(&mut host, ids[1], "a", 22, false);
    until_running(&mut host, &mut source);
    for module in [ModuleId(3), ModuleId(4)] {
        assert_eq!(host.module_state(module), Some(State::Running));
    }
    assert_eq!(&renderer_markers(&host)[before..], b"uuUUCC");
    for module in [ModuleId(3), ModuleId(4)] {
        assert_eq!(ui_query(&mut host, module, 8), Ok(4));
    }
    let profile = host
        .runtime
        .scene
        .as_ref()
        .unwrap()
        .hud_font(RuleSetId::Quake3)
        .unwrap()
        .profile;
    let owners = ids.map(|client| {
        qa_render::shader::registration::ShaderOwner::Client(
            client,
            host.runtime.server.clients[client.0 as usize].entity,
        )
    });
    let contexts = owners.map(|owner| host.native_shaders().context(owner, profile).unwrap());
    let peer_frames = ui_query(&mut host, ModuleId(4), 10).unwrap();
    let before = renderer_markers(&host).len();
    source.reject_reset = Some(ids[0]);
    queue_gamestate(&mut host, ids[0], "b", 19, false);
    assert!(query(&mut host, ModuleId(1), 9).is_err());
    assert!(ui_query(&mut host, ModuleId(3), 7).is_err());
    assert!(ui_query(&mut host, ModuleId(4), 7).is_ok());
    source.step(&mut host);
    assert_eq!(host.module_state(ModuleId(1)), Some(State::Pending));
    assert_eq!(host.module_state(ModuleId(3)), Some(State::Pending));
    assert_eq!(&renderer_markers(&host)[before..], b"cu");
    assert_eq!(
        host.native_shaders().context(owners[1], profile),
        Some(contexts[1])
    );
    queue_gamestate(&mut host, ids[0], "c", 33, false);
    source.step(&mut host);
    assert_eq!(&renderer_markers(&host)[before..], b"cu");
    assert!(ui_query(&mut host, ModuleId(4), 10).unwrap() > peer_frames);
    source.reject_reset = None;
    until_running(&mut host, &mut source);
    assert_eq!(&renderer_markers(&host)[before..], b"cuUC");
    assert_eq!(query(&mut host, ModuleId(1), 9), Ok(33));
    assert_eq!(host.module_state(ModuleId(3)), Some(State::Running));
    assert_eq!(ui_query(&mut host, ModuleId(3), 8), Ok(4));
    assert_eq!(ui_query(&mut host, ModuleId(4), 8), Ok(4));
    assert_ne!(
        host.native_shaders().context(owners[0], profile),
        Some(contexts[0])
    );
    assert_eq!(
        host.native_shaders().context(owners[1], profile),
        Some(contexts[1])
    );
    for module in [ModuleId(1), ModuleId(2), ModuleId(3), ModuleId(4)] {
        assert_eq!(host.module_counts(module).unwrap().traps, 0);
    }
}

fn verify_ui_shutdown_failure_isolation(module: Vec<u8>, ui: Vec<u8>) {
    let (_fixture, mut host, mut source, ids, _) = setup_renderers(module, Some(ui));
    queue_gamestate(&mut host, ids[0], "a", 11, false);
    queue_gamestate(&mut host, ids[1], "a", 22, false);
    until_running(&mut host, &mut source);
    let peer_frames = ui_query(&mut host, ModuleId(4), 10).unwrap();
    queue_gamestate(&mut host, ids[0], "b", 19, false);
    until_running(&mut host, &mut source);
    assert_eq!(host.module_state(ModuleId(3)), Some(State::Failed));
    assert_eq!(host.module_state(ModuleId(4)), Some(State::Running));
    assert_eq!(query(&mut host, ModuleId(1), 9), Ok(19));
    assert_eq!(query(&mut host, ModuleId(2), 9), Ok(22));
    assert!(ui_query(&mut host, ModuleId(4), 10).unwrap() > peer_frames);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
    assert_eq!(host.module_counts(ModuleId(2)).unwrap().traps, 0);
}

fn verify_ui_association_replacement(module: Vec<u8>, ui: Vec<u8>) {
    let (_fixture, mut host, mut source, ids, _) = setup_renderers(module, Some(ui));
    queue_gamestate(&mut host, ids[0], "a", 11, false);
    queue_gamestate(&mut host, ids[1], "a", 22, false);
    until_running(&mut host, &mut source);
    let old_lifetime = host.runtime.server.clients[ids[0].0 as usize].entity;
    let peer_ui_frames = ui_query(&mut host, ModuleId(4), 10).unwrap();
    assert!(host.shutdown_module_with_driver(ModuleId(1), &mut source.driver()));
    assert!(host.runtime.disconnect_client(ids[0]));
    // Complete actual output retirement without advancing module lifecycle:
    // the old UI still holds A's captured association while the seat is reused.
    for _ in 0..3 {
        host.runtime.publish_audio_retirements();
        host.dispatch_output(&mut source, &mut FrameResult::default());
    }
    let replacement = host
        .runtime
        .connect_local(
            SeatId::FIRST,
            SpawnAnchor {
                position: Vec3([32., 0., 0.]),
                angles: Vec3([0., 180., 0.]),
                entity: 0,
                fixture_fallback: false,
            },
            ClientPolicy {
                client: RuleSetId::Quake3,
                input: RuleSetId::Quake3,
                movement: RuleSetId::Quake3,
                trace: RuleSetId::Quake3,
            },
            Protocol::Quake3_68,
            0,
        )
        .unwrap();
    let new_lifetime = host.runtime.server.clients[replacement.0 as usize].entity;
    assert_ne!(new_lifetime, old_lifetime);
    host.local_clients[0] = Some(replacement);
    assert!(ui_query(&mut host, ModuleId(3), 7).is_err());
    assert!(ui_query(&mut host, ModuleId(4), 7).is_ok());
    let resets = source.reset_receipts;
    let before = renderer_markers(&host).len();
    source.step(&mut host);
    assert_eq!(host.module_state(ModuleId(3)), Some(State::Running));
    assert_eq!(host.module_state(ModuleId(2)), Some(State::Running));
    assert_eq!(host.module_state(ModuleId(4)), Some(State::Running));
    assert_eq!(&renderer_markers(&host)[before..], b"uU");
    assert_eq!(source.reset_receipts, resets);
    assert!(ui_query(&mut host, ModuleId(4), 10).unwrap() > peer_ui_frames);
    assert_eq!(host.module_counts(ModuleId(3)).unwrap().traps, 0);
    assert_eq!(host.module_counts(ModuleId(2)).unwrap().traps, 0);
    assert_eq!(
        host.runtime.server.clients[replacement.0 as usize].entity,
        new_lifetime
    );
}

fn verify_video_restart(module: Vec<u8>, ui: Vec<u8>) {
    use qa_render::shader::{
        context::ShaderNodeId,
        reference::{NativeShaderRef, ShaderRef},
        registration::{ShaderOwner, ShaderResource},
    };
    let (_fixture, mut host, mut source, ids, anchors) =
        setup_renderers_with_graphics(module, Some(ui), true);
    queue_gamestate(&mut host, ids[0], "a", 11, false);
    queue_gamestate(&mut host, ids[1], "a", 22, false);
    until_running(&mut host, &mut source);
    queue_snapshot(&mut host, ids[0], 11, 1000);
    queue_snapshot(&mut host, ids[1], 22, 1000);
    source.step(&mut host);
    assert_eq!(source.backend.cpu.pixels()[0], u32::from_le_bytes([255; 4]));
    assert_eq!(
        source.backend.cpu.pixels()[32],
        u32::from_le_bytes([255; 4])
    );
    let lifetimes = ids.map(|id| host.runtime.server.clients[id.0 as usize].entity);
    let worlds = host.local_worlds;
    let server_geometry = host.runtime.collision.as_ref().map(|world| world.geometry);
    let clients = ids.map(|id| {
        let row = &host.runtime.server.clients[id.0 as usize];
        (
            row.input_rules,
            row.client_rules,
            row.player.movement_rules,
            row.player.trace_rules,
        )
    });
    let native_players = ids.map(|id| {
        host.runtime.snapshots[id.0 as usize]
            .as_ref()
            .unwrap()
            .native_client()
    });
    let connections = ids.map(|id| {
        let row = host.runtime.network.get(id, Endpoint::Client).unwrap();
        (row.phase, row.route, row.output, row.channel.endpoint())
    });
    let font = host
        .runtime
        .scene
        .as_ref()
        .unwrap()
        .hud_font(RuleSetId::Quake3)
        .unwrap();
    let profile = font.profile;
    let font_receipt = font.registration.unwrap();
    let owners =
        std::array::from_fn::<_, 2, _>(|seat| ShaderOwner::Client(ids[seat], lifetimes[seat]));
    let contexts = owners.map(|owner| host.native_shaders().context(owner, profile).unwrap());
    let handles = [
        ui_query(&mut host, ModuleId(3), 8).unwrap(),
        ui_query(&mut host, ModuleId(4), 8).unwrap(),
    ];
    assert_eq!(handles, [4, 4]);
    assert_eq!(query(&mut host, ModuleId(1), 6), Ok(handles[0]));
    assert_eq!(query(&mut host, ModuleId(2), 6), Ok(handles[1]));
    let old_refs = std::array::from_fn::<_, 2, _>(|seat| {
        ShaderRef::Native(NativeShaderRef {
            context: contexts[seat],
            node: ShaderNodeId(handles[seat] - 1),
        })
    });
    let original = old_refs[0].source(host.native_shaders()).unwrap();
    host.call_module_export_with_driver(
        ModuleId(3),
        Export::words(CallbackId(6), &[]),
        ThinkTime::Milliseconds(9000),
        &mut source.driver(),
    )
    .unwrap();
    let resolved = old_refs[0]
        .resolve(
            host.runtime.model_assets.as_ref().unwrap().assets(),
            host.native_shaders(),
        )
        .unwrap();
    assert_ne!(resolved.binding(), original);
    assert_eq!(resolved.time_offset(), 2.5);
    assert_eq!(
        old_refs[1]
            .resolve(
                host.runtime.model_assets.as_ref().unwrap().assets(),
                host.native_shaders()
            )
            .unwrap()
            .binding(),
        original
    );
    source.step(&mut host);
    let remapped_pixel = source.backend.cpu.pixels()[0];
    assert_ne!(remapped_pixel, u32::from_le_bytes([255; 4]));
    assert_ne!(remapped_pixel, 0);
    assert_eq!(
        source.backend.cpu.pixels()[32],
        u32::from_le_bytes([255; 4])
    );
    paint(&host, &mut source);

    let context = Context {
        source: RuleSetId::Quake3,
        seat: SeatId::FIRST,
        ..Context::default()
    };
    let bands = host
        .console
        .cvars
        .bind("r_cpuBands", Context::default())
        .unwrap();
    let game_type = host.console.cvars.bind("g_gametype", context).unwrap();
    let old_game_type = host
        .console
        .cvars
        .read(game_type)
        .unwrap()
        .as_str()
        .to_owned();
    assert_eq!(host.console.cvars.read(bands).unwrap().as_str(), "1");
    assert_eq!(
        source.backend.cpu.raster_config().bands,
        qa_render::cpu::RasterBands::One
    );
    host.console
        .append("r_cpuBands 2\ng_gametype 1\n", context)
        .unwrap();
    host.execute_commands_with_driver(&mut source.driver());
    assert_eq!(host.console.cvars.read(bands).unwrap().as_str(), "1");
    assert_eq!(
        host.console.cvars.latched(bands).unwrap().unwrap().as_str(),
        "2"
    );
    assert_eq!(
        host.console.cvars.read(game_type).unwrap().as_str(),
        old_game_type
    );
    assert_eq!(
        host.console
            .cvars
            .latched(game_type)
            .unwrap()
            .unwrap()
            .as_str(),
        "1"
    );
    let old_markers = renderer_markers(&host);
    let before = old_markers.len();
    let resets = source.reset_receipts;
    let draws = query(&mut host, ModuleId(1), 10).unwrap();
    let prefix = host
        .runtime
        .model_assets
        .as_ref()
        .unwrap()
        .assets()
        .counts();
    source.video_reject = true;
    host.console.append("vid_restart cpu\n", context).unwrap();
    host.execute_commands_with_driver(&mut source.driver());
    host.runtime.vfs.take_lookup_count();
    source.step(&mut host);
    assert_eq!((source.video_prepares, source.video_publishes), (1, 0));
    assert_eq!(host.console.cvars.read(bands).unwrap().as_str(), "1");
    assert_eq!(
        host.console.cvars.latched(bands).unwrap().unwrap().as_str(),
        "2"
    );
    assert_eq!(
        source.backend.cpu.raster_config().bands,
        qa_render::cpu::RasterBands::One
    );
    assert_eq!(renderer_markers(&host), old_markers);
    assert_eq!(source.reset_receipts, resets);
    assert_eq!(
        host.native_shaders().context(owners[0], profile),
        Some(contexts[0])
    );
    assert_eq!(
        host.native_shaders().context(owners[1], profile),
        Some(contexts[1])
    );
    assert_eq!(
        old_refs[0]
            .resolve(
                host.runtime.model_assets.as_ref().unwrap().assets(),
                host.native_shaders()
            )
            .unwrap(),
        resolved
    );
    assert!(query(&mut host, ModuleId(1), 10).unwrap() > draws);
    for module in [ModuleId(1), ModuleId(2), ModuleId(3), ModuleId(4)] {
        assert_eq!(host.module_state(module), Some(State::Running));
    }
    assert_eq!(
        host.runtime
            .model_assets
            .as_ref()
            .unwrap()
            .assets()
            .counts(),
        prefix
    );
    assert_eq!(host.runtime.vfs.take_lookup_count(), 0);
    assert_eq!(source.backend.cpu.pixels()[0], remapped_pixel);

    source.video_reject = false;
    // The reliable native command remains owned by this same channel while
    // the console records a coalesced renderer request.
    assert_eq!(query(&mut host, ModuleId(1), 5), Ok(0));
    let reliable = host
        .runtime
        .network
        .get(ids[0], Endpoint::Client)
        .unwrap()
        .channel
        .command_state();
    let channel = host
        .runtime
        .network
        .get(ids[0], Endpoint::Client)
        .unwrap()
        .channel
        .state();
    host.console
        .append("vid_restart gl\nvid_restart\nvid_restart cpu\n", context)
        .unwrap();
    host.execute_commands_with_driver(&mut source.driver());
    assert_eq!(
        host.runtime
            .network
            .get(ids[0], Endpoint::Client)
            .unwrap()
            .channel
            .command_state(),
        reliable
    );
    assert_eq!(
        host.runtime
            .network
            .get(ids[0], Endpoint::Client)
            .unwrap()
            .channel
            .state(),
        channel
    );
    let prepares = source.video_prepares;
    host.runtime.vfs.take_lookup_count();
    source.step(&mut host);
    until_running(&mut host, &mut source);
    assert_eq!(source.video_prepares, prepares + 1);
    assert_eq!(source.video_publishes, 1);
    assert_eq!(host.console.cvars.read(bands).unwrap().as_str(), "2");
    assert!(host.console.cvars.latched(bands).unwrap().is_none());
    assert_eq!(
        source.backend.cpu.raster_config().bands,
        qa_render::cpu::RasterBands::Two
    );
    assert_eq!(
        host.console.cvars.read(game_type).unwrap().as_str(),
        old_game_type
    );
    assert_eq!(
        host.console
            .cvars
            .latched(game_type)
            .unwrap()
            .unwrap()
            .as_str(),
        "1"
    );
    assert!(matches!(
        source.video_kinds.last(),
        Some(Some(qa_app::renderer::Kind::Cpu))
    ));
    assert_eq!(&renderer_markers(&host)[before..], b"uuccUUCC");
    assert!(Arc::ptr_eq(
        host.runtime.renderer_configuration.as_ref().unwrap(),
        &source.video_configuration
    ));
    assert_eq!(query(&mut host, ModuleId(1), 9), Ok(11));
    assert_eq!(query(&mut host, ModuleId(2), 9), Ok(22));
    assert_eq!(query(&mut host, ModuleId(1), 8), Ok(2));
    assert_eq!(query(&mut host, ModuleId(2), 8), Ok(2));
    assert_eq!(host.local_worlds, worlds);
    assert_eq!(
        host.runtime.collision.as_ref().map(|world| world.geometry),
        server_geometry
    );
    assert_eq!(
        host.runtime
            .model_assets
            .as_ref()
            .unwrap()
            .assets()
            .counts(),
        prefix
    );
    assert_eq!(source.backend.cpu.admitted_assets(), Some(prefix));
    assert_eq!(host.runtime.vfs.take_lookup_count(), 0);
    for seat in 0..2 {
        assert_eq!(host.local_clients[seat], Some(ids[seat]));
        let row = &host.runtime.server.clients[ids[seat].0 as usize];
        assert_eq!(row.entity, lifetimes[seat]);
        assert_eq!(
            (
                row.input_rules,
                row.client_rules,
                row.player.movement_rules,
                row.player.trace_rules
            ),
            clients[seat]
        );
        assert_eq!(
            host.runtime.snapshots[ids[seat].0 as usize]
                .as_ref()
                .unwrap()
                .native_client(),
            native_players[seat]
        );
        let row = host
            .runtime
            .network
            .get(ids[seat], Endpoint::Client)
            .unwrap();
        assert_eq!(
            (row.phase, row.route, row.output, row.channel.endpoint()),
            connections[seat]
        );
        assert!(
            host.runtime
                .server
                .entities
                .resolve(anchors[seat])
                .is_some()
        );
        let renewed = host
            .native_shaders()
            .context(owners[seat], profile)
            .unwrap();
        assert_ne!(renewed, contexts[seat]);
        assert_eq!(old_refs[seat].source(host.native_shaders()), None);
        let handle = ui_query(&mut host, ModuleId(seat as u16 + 3), 8).unwrap();
        assert_eq!(handle, 4);
        assert_eq!(query(&mut host, ModuleId(seat as u16 + 1), 6), Ok(handle));
        let shader = ShaderRef::Native(NativeShaderRef {
            context: renewed,
            node: ShaderNodeId(handle - 1),
        });
        assert_eq!(shader.source(host.native_shaders()), Some(original));
        let selected = shader
            .resolve(
                host.runtime.model_assets.as_ref().unwrap().assets(),
                host.native_shaders(),
            )
            .unwrap();
        assert_eq!((selected.binding(), selected.time_offset()), (original, 0.));
        let nodes = host
            .native_shaders()
            .resource_nodes(renewed, ShaderResource::Graphic(font_receipt.key))
            .unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(
            host.native_shaders()
                .source(NativeShaderRef {
                    context: renewed,
                    node: nodes[0]
                })
                .unwrap()
                .shader,
            font_receipt.shader
        );
    }
    let queued = host
        .runtime
        .network
        .get(ids[0], Endpoint::Client)
        .unwrap()
        .channel
        .command_state()
        .unwrap()
        .queued;
    assert_eq!(query(&mut host, ModuleId(1), 5), Ok(0));
    assert_eq!(
        host.runtime
            .network
            .get(ids[0], Endpoint::Client)
            .unwrap()
            .channel
            .command_state()
            .unwrap()
            .queued,
        queued + 1
    );
    queue_snapshot(&mut host, ids[0], 11, 2000);
    queue_snapshot(&mut host, ids[1], 22, 2000);
    source.step(&mut host);
    assert!(source.backend.cpu.world_stats().spans > 0);
    assert_eq!(source.backend.cpu.pixels()[0], u32::from_le_bytes([255; 4]));
    assert_eq!(
        source.backend.cpu.pixels()[32],
        u32::from_le_bytes([255; 4])
    );
    for seat in 0..2 {
        assert_eq!(
            source.listener_receipts[seat],
            Some((lifetimes[seat], ModuleId(seat as u16 + 1)))
        );
        assert!(source.play_receipts[seat] > 0);
    }
    paint(&host, &mut source);

    let before_invalid = renderer_markers(&host);
    let prepares = source.video_prepares;
    let publishes = source.video_publishes;
    let old_configuration = Arc::clone(host.runtime.renderer_configuration.as_ref().unwrap());
    host.console
        .append("r_cpuBands 3\nvid_restart cpu\n", context)
        .unwrap();
    host.execute_commands_with_driver(&mut source.driver());
    source.step(&mut host);
    assert_eq!(source.video_prepares, prepares);
    assert_eq!(source.video_publishes, publishes);
    assert_eq!(renderer_markers(&host), before_invalid);
    assert!(Arc::ptr_eq(
        host.runtime.renderer_configuration.as_ref().unwrap(),
        &old_configuration
    ));
    assert_eq!(
        source.backend.cpu.raster_config().bands,
        qa_render::cpu::RasterBands::Two
    );
    assert_eq!(host.console.cvars.read(bands).unwrap().as_str(), "2");
    assert_eq!(
        host.console.cvars.latched(bands).unwrap().unwrap().as_str(),
        "3"
    );

    // The benchmark override selects the real replacement's band partition,
    // while the ordinary successful CPU load still activates its shared cvar.
    host.set_cpu_band_override(Some(qa_render::cpu::RasterBands::One));
    host.console
        .append("r_cpuBands 4\nvid_restart cpu\n", context)
        .unwrap();
    host.execute_commands_with_driver(&mut source.driver());
    assert_eq!(host.console.cvars.read(bands).unwrap().as_str(), "2");
    assert_eq!(
        host.console.cvars.latched(bands).unwrap().unwrap().as_str(),
        "4"
    );
    source.step(&mut host);
    until_running(&mut host, &mut source);
    assert_eq!(source.video_prepares, prepares + 1);
    assert_eq!(source.video_publishes, publishes + 1);
    assert_eq!(
        &renderer_markers(&host)[before_invalid.len()..],
        b"uuccUUCC"
    );
    assert_eq!(
        source.backend.cpu.raster_config().bands,
        qa_render::cpu::RasterBands::One
    );
    assert_eq!(host.console.cvars.read(bands).unwrap().as_str(), "4");
    assert!(host.console.cvars.latched(bands).unwrap().is_none());
    assert_eq!(
        host.console.cvars.read(game_type).unwrap().as_str(),
        old_game_type
    );
    assert_eq!(
        host.console
            .cvars
            .latched(game_type)
            .unwrap()
            .unwrap()
            .as_str(),
        "1"
    );
    queue_snapshot(&mut host, ids[0], 11, 2500);
    queue_snapshot(&mut host, ids[1], 22, 2500);
    source.step(&mut host);
    assert!(source.backend.cpu.world_stats().spans > 0);
    assert_eq!(source.backend.cpu.pixels()[0], u32::from_le_bytes([255; 4]));

    // EXEC_NOW records the request while this UI invocation is still active.
    // Its continuation must finish; only later ordinary entries are fenced.
    let before = renderer_markers(&host).len();
    let prepares = source.video_prepares;
    let publishes = source.video_publishes;
    assert_eq!(
        host.call_module_export_with_driver(
            ModuleId(3),
            Export::words(CallbackId(9), &[]),
            ThinkTime::Milliseconds(9000),
            &mut source.driver(),
        ),
        Ok(())
    );
    assert!(matches!(
        host.module_counts(ModuleId(3)).unwrap().last_result,
        Some(ModuleResult::Qvm(0x600d) | ModuleResult::Native(0x600d))
    ));
    assert_eq!(source.video_prepares, prepares);
    assert_eq!(renderer_markers(&host).len(), before);
    assert!(ui_query(&mut host, ModuleId(3), 8).is_err());
    assert!(query(&mut host, ModuleId(1), 9).is_err());
    source.step(&mut host);
    until_running(&mut host, &mut source);
    assert_eq!(&renderer_markers(&host)[before..], b"uuccUUCC");
    assert_eq!(source.video_prepares, prepares + 1);
    assert_eq!(source.video_publishes, publishes + 1);
    assert_eq!(ui_query(&mut host, ModuleId(3), 8), Ok(4));
    assert_eq!(query(&mut host, ModuleId(1), 9), Ok(11));
    for module in [ModuleId(1), ModuleId(2), ModuleId(3), ModuleId(4)] {
        assert_eq!(host.module_counts(module).unwrap().traps, 0);
    }
}

fn verify_video_image_restart(module: Vec<u8>, ui: Vec<u8>) {
    use qa_render::shader::{
        context::ShaderNodeId,
        reference::{NativeShaderRef, ShaderRef},
        registration::ShaderOwner,
    };
    let (_fixture, mut host, mut source, ids, anchors) =
        setup_renderers_with_images(module, Some(ui), true, true);
    queue_gamestate(&mut host, ids[0], "a", 11, false);
    queue_gamestate(&mut host, ids[1], "a", 22, false);
    until_running(&mut host, &mut source);
    queue_snapshot(&mut host, ids[0], 11, 1000);
    queue_snapshot(&mut host, ids[1], 22, 1000);
    source.step(&mut host);
    let font = host
        .runtime
        .scene
        .as_ref()
        .unwrap()
        .hud_font(RuleSetId::Quake3)
        .unwrap();
    let old_profile = font.profile;
    assert_eq!(old_profile.image_settings.picmip, 1);
    let lifetimes = ids.map(|id| host.runtime.server.clients[id.0 as usize].entity);
    let owners =
        std::array::from_fn::<_, 2, _>(|seat| ShaderOwner::Client(ids[seat], lifetimes[seat]));
    let contexts = owners.map(|owner| host.native_shaders().context(owner, old_profile).unwrap());
    let handles = [
        ui_query(&mut host, ModuleId(3), 8).unwrap(),
        ui_query(&mut host, ModuleId(4), 8).unwrap(),
    ];
    assert_eq!(handles, [4, 4]);
    let refs = std::array::from_fn::<_, 2, _>(|seat| {
        ShaderRef::Native(NativeShaderRef {
            context: contexts[seat],
            node: ShaderNodeId(handles[seat] - 1),
        })
    });
    let image_body = |host: &FrameHost, shader: ShaderRef| {
        let binding = shader.source(host.native_shaders()).unwrap();
        let assets = host.runtime.model_assets.as_ref().unwrap().assets();
        let material = assets
            .material(assets.shader_binding(binding).unwrap().material)
            .unwrap();
        let qa_render::assets::StageTexture::Image(image) = material.stages[0].texture else {
            panic!("image-backed guest shader")
        };
        let resource = assets.image(image).unwrap();
        let level = &resource.prepared.as_ref().unwrap().levels[0];
        (
            binding,
            image,
            resource.preparation_revision,
            level.width,
            level.height,
            level.rgba.to_vec(),
        )
    };
    let old_body = image_body(&host, refs[0]);
    assert_eq!((old_body.3, old_body.4), (4, 4));
    assert_eq!(image_body(&host, refs[1]), old_body);
    let prefix = host
        .runtime
        .model_assets
        .as_ref()
        .unwrap()
        .assets()
        .counts();
    let worlds = host.local_worlds;
    let collisions = ids.map(|id| {
        host.runtime.client_resources[id.0 as usize]
            .as_ref()
            .unwrap()
            .state
            .models
            .collision_world()
    });
    let revisions = ids.map(|id| {
        host.runtime.client_resources[id.0 as usize]
            .as_ref()
            .unwrap()
            .state
            .models
            .revision()
    });
    let row_revisions = ids.map(|id| {
        let configs = &host.runtime.client_resources[id.0 as usize]
            .as_ref()
            .unwrap()
            .state
            .configs;
        [
            configs.config(0).unwrap().revision,
            configs.config(33).unwrap().revision,
        ]
    });
    let connections = ids.map(|id| {
        let connection = host.runtime.network.get(id, Endpoint::Client).unwrap();
        (
            connection.route,
            connection.output,
            connection.channel.endpoint(),
        )
    });
    let context = Context {
        source: RuleSetId::Quake3,
        seat: SeatId::FIRST,
        ..Context::default()
    };
    let picmip = host.console.cvars.bind("r_picmip", context).unwrap();
    let width = host.console.cvars.bind("r_customwidth", context).unwrap();
    let height = host.console.cvars.bind("r_customheight", context).unwrap();
    let old_limits = host.runtime.scene.as_ref().unwrap().limits();
    assert_eq!(source.backend.cpu.dimensions(), [64, 48]);
    let explicit_aspect = f32::from_bits(0x3fa1_2345);
    // Both roles captured CLIENT0's half of this 64x48 output. The UI's
    // render-only association grants the same 32x48 viewport, without CLIENT
    // service authority. No explicit aspect overrides these initial extents.
    let old_cgame_aspect = (32.0f32 / 48.0).to_bits();
    let old_ui_aspect = (32.0f32 / 48.0).to_bits();
    assert_eq!(query(&mut host, ModuleId(1), 4), Ok(old_cgame_aspect));
    assert_eq!(ui_query(&mut host, ModuleId(3), 3), Ok(old_ui_aspect));
    let markers = renderer_markers(&host);
    let resets = source.reset_receipts;
    source.video_reject = true;
    host.console
        .append(
            &format!("r_customwidth 96\nr_customheight 80\nr_customaspect {explicit_aspect}\nr_picmip 2\nvid_restart cpu\n"),
            context,
        )
        .unwrap();
    host.execute_commands_with_driver(&mut source.driver());
    assert_eq!(host.console.cvars.read(picmip).unwrap().as_str(), "1");
    assert_eq!(
        host.console
            .cvars
            .latched(picmip)
            .unwrap()
            .unwrap()
            .as_str(),
        "2"
    );
    source.step(&mut host);
    assert_eq!((source.video_prepares, source.video_publishes), (1, 0));
    assert_eq!(source.backend.cpu.dimensions(), [64, 48]);
    assert_eq!(query(&mut host, ModuleId(1), 4), Ok(old_cgame_aspect));
    assert_eq!(ui_query(&mut host, ModuleId(3), 3), Ok(old_ui_aspect));
    assert_eq!(host.console.cvars.read(width).unwrap().as_str(), "64");
    assert_eq!(host.console.cvars.read(height).unwrap().as_str(), "48");
    assert_eq!(
        host.console.cvars.latched(width).unwrap().unwrap().as_str(),
        "96"
    );
    assert_eq!(
        host.console
            .cvars
            .latched(height)
            .unwrap()
            .unwrap()
            .as_str(),
        "80"
    );
    assert_eq!(host.runtime.scene.as_ref().unwrap().limits(), old_limits);
    assert_eq!(renderer_markers(&host), markers);
    assert_eq!(source.reset_receipts, resets);
    assert_eq!(host.console.cvars.read(picmip).unwrap().as_str(), "1");
    assert_eq!(
        host.console
            .cvars
            .latched(picmip)
            .unwrap()
            .unwrap()
            .as_str(),
        "2"
    );
    assert_eq!(
        host.runtime
            .scene
            .as_ref()
            .unwrap()
            .hud_font(RuleSetId::Quake3)
            .unwrap()
            .profile,
        old_profile
    );
    assert_eq!(host.local_worlds, worlds);
    assert_eq!(
        host.runtime
            .model_assets
            .as_ref()
            .unwrap()
            .assets()
            .counts(),
        prefix
    );
    for seat in 0..2 {
        assert_eq!(
            host.native_shaders().context(owners[seat], old_profile),
            Some(contexts[seat])
        );
        assert_eq!(image_body(&host, refs[seat]), old_body);
    }

    source.video_reject = false;
    source.video_publish_reject = true;
    host.console.append("vid_restart cpu\n", context).unwrap();
    host.execute_commands_with_driver(&mut source.driver());
    source.step(&mut host);
    until_running(&mut host, &mut source);
    assert_eq!((source.video_prepares, source.video_publishes), (2, 0));
    assert_eq!(source.backend.cpu.dimensions(), [64, 48]);
    assert_eq!(query(&mut host, ModuleId(1), 4), Ok(old_cgame_aspect));
    assert_eq!(ui_query(&mut host, ModuleId(3), 3), Ok(old_ui_aspect));
    assert_eq!(host.console.cvars.read(width).unwrap().as_str(), "64");
    assert_eq!(host.console.cvars.read(height).unwrap().as_str(), "48");
    assert_eq!(
        host.console.cvars.latched(width).unwrap().unwrap().as_str(),
        "96"
    );
    assert_eq!(
        host.console
            .cvars
            .latched(height)
            .unwrap()
            .unwrap()
            .as_str(),
        "80"
    );
    assert_eq!(host.runtime.scene.as_ref().unwrap().limits(), old_limits);
    assert_eq!(&renderer_markers(&host)[markers.len()..], b"uuccUUCC");
    assert_eq!(host.console.cvars.read(picmip).unwrap().as_str(), "1");
    assert_eq!(
        host.console
            .cvars
            .latched(picmip)
            .unwrap()
            .unwrap()
            .as_str(),
        "2"
    );
    assert_eq!(
        host.runtime
            .scene
            .as_ref()
            .unwrap()
            .hud_font(RuleSetId::Quake3)
            .unwrap()
            .profile,
        old_profile
    );
    assert_eq!(host.local_worlds, worlds);
    assert_eq!(
        host.runtime
            .model_assets
            .as_ref()
            .unwrap()
            .assets()
            .counts(),
        prefix
    );
    assert_eq!(source.backend.cpu.admitted_assets(), Some(prefix));
    for seat in 0..2 {
        let context = host
            .native_shaders()
            .context(owners[seat], old_profile)
            .unwrap();
        assert_eq!(
            ui_query(&mut host, ModuleId(seat as u16 + 3), 8),
            Ok(handles[seat])
        );
        let shader = ShaderRef::Native(NativeShaderRef {
            context,
            node: ShaderNodeId(handles[seat] - 1),
        });
        assert_eq!(image_body(&host, shader), old_body);
        let resources = &host.runtime.client_resources[ids[seat].0 as usize]
            .as_ref()
            .unwrap()
            .state;
        assert_eq!(resources.models.collision_world(), collisions[seat]);
        assert_eq!(resources.models.revision(), revisions[seat]);
        assert_eq!(
            [
                resources.configs.config(0).unwrap().revision,
                resources.configs.config(33).unwrap().revision
            ],
            row_revisions[seat]
        );
    }
    queue_snapshot(&mut host, ids[0], 11, 1500);
    queue_snapshot(&mut host, ids[1], 22, 1500);
    source.step(&mut host);
    assert!(source.backend.cpu.world_stats().spans > 0);
    assert_ne!(source.backend.cpu.pixels()[0], 0);
    assert_eq!(
        source.backend.cpu.pixels()[0],
        source.backend.cpu.pixels()[32]
    );

    assert!(source.views.iter().any(|(view, _, _)| *view
        == qa_render::Viewport {
            x: 2,
            y: 2,
            width: 28,
            height: 44
        }));
    assert!(source.views.iter().any(|(view, _, _)| *view
        == qa_render::Viewport {
            x: 34,
            y: 2,
            width: 28,
            height: 44
        }));
    source.video_publish_reject = false;
    let retry_markers = renderer_markers(&host).len();
    host.console.append("vid_restart cpu\n", context).unwrap();
    host.execute_commands_with_driver(&mut source.driver());
    source.step(&mut host);
    until_running(&mut host, &mut source);
    assert_eq!((source.video_prepares, source.video_publishes), (3, 1));
    assert_eq!(source.backend.cpu.dimensions(), [96, 80]);
    assert_eq!(
        source
            .video_configuration
            .display
            .window_aspect
            .unwrap()
            .to_bits(),
        explicit_aspect.to_bits()
    );
    assert_eq!(
        source.video_window.window_aspect.unwrap().to_bits(),
        explicit_aspect.to_bits()
    );
    assert_eq!(
        query(&mut host, ModuleId(1), 4),
        Ok(explicit_aspect.to_bits())
    );
    assert_eq!(
        ui_query(&mut host, ModuleId(3), 3),
        Ok(explicit_aspect.to_bits())
    );
    assert_eq!(
        [
            source.video_configuration.display.width,
            source.video_configuration.display.height
        ],
        [96, 80]
    );
    assert_eq!(host.console.cvars.read(width).unwrap().as_str(), "96");
    assert_eq!(host.console.cvars.read(height).unwrap().as_str(), "80");
    assert!(host.console.cvars.latched(width).unwrap().is_none());
    assert!(host.console.cvars.latched(height).unwrap().is_none());
    let limits = host.runtime.scene.as_ref().unwrap().limits();
    assert!(limits.commands > old_limits.commands);
    assert!(limits.vertices > old_limits.vertices);
    assert_eq!(&renderer_markers(&host)[retry_markers..], b"uuccUUCC");
    assert_eq!(host.console.cvars.read(picmip).unwrap().as_str(), "2");
    assert!(host.console.cvars.latched(picmip).unwrap().is_none());
    let font = host
        .runtime
        .scene
        .as_ref()
        .unwrap()
        .hud_font(RuleSetId::Quake3)
        .unwrap();
    let profile = font.profile;
    let mut settings = old_profile.image_settings;
    settings.picmip = 2;
    assert_eq!(profile.image_settings, settings);
    assert_eq!(
        (
            profile.source_rules,
            profile.rules,
            profile.source_palette,
            profile.palette
        ),
        (
            old_profile.source_rules,
            old_profile.rules,
            old_profile.source_palette,
            old_profile.palette
        )
    );
    let mut replacement_body = None;
    for seat in 0..2 {
        let module = ModuleId(seat as u16 + 1);
        let ui = ModuleId(seat as u16 + 3);
        assert_eq!(ui_query(&mut host, ui, 8), Ok(handles[seat]));
        assert_eq!(query(&mut host, module, 6), Ok(handles[seat]));
        let renewed = host
            .native_shaders()
            .context(owners[seat], profile)
            .unwrap();
        assert_ne!(renewed, contexts[seat]);
        assert_eq!(refs[seat].source(host.native_shaders()), None);
        let shader = ShaderRef::Native(NativeShaderRef {
            context: renewed,
            node: ShaderNodeId(handles[seat] - 1),
        });
        let body = image_body(&host, shader);
        assert_ne!(body.0, old_body.0);
        assert_ne!(body.1, old_body.1);
        assert_eq!((body.3, body.4), (2, 2));
        if let Some(expected) = &replacement_body {
            assert_eq!(&body, expected);
        } else {
            replacement_body = Some(body);
        }
        let old_world = worlds[seat].unwrap();
        let world = host.local_worlds[seat].unwrap();
        assert_eq!(world.0, old_world.0);
        assert_ne!(world.1, old_world.1);
        let registered = host
            .runtime
            .model_assets
            .as_ref()
            .unwrap()
            .assets()
            .world_presentation_registration(world.1)
            .unwrap();
        assert_eq!(registered.profile.image_settings, settings);
        let resources = &host.runtime.client_resources[ids[seat].0 as usize]
            .as_ref()
            .unwrap()
            .state;
        assert_eq!(resources.models.collision_world(), collisions[seat]);
        assert_eq!(resources.models.revision(), revisions[seat]);
        assert_eq!(
            [
                resources.configs.config(0).unwrap().revision,
                resources.configs.config(33).unwrap().revision
            ],
            row_revisions[seat]
        );
        assert_eq!(
            host.runtime.server.clients[ids[seat].0 as usize].entity,
            lifetimes[seat]
        );
        assert!(
            host.runtime
                .server
                .entities
                .resolve(anchors[seat])
                .is_some()
        );
        let connection = host
            .runtime
            .network
            .get(ids[seat], Endpoint::Client)
            .unwrap();
        assert_eq!(
            (
                connection.route,
                connection.output,
                connection.channel.endpoint()
            ),
            connections[seat]
        );
    }
    let assets = host.runtime.model_assets.as_ref().unwrap().assets();
    assert!(assets.counts().contains(prefix));
    let old_material = assets
        .material(assets.shader_binding(old_body.0).unwrap().material)
        .unwrap();
    assert_eq!(
        old_material.stages[0].texture,
        qa_render::assets::StageTexture::Image(old_body.1)
    );
    for old_world in worlds.into_iter().flatten() {
        assert_eq!(
            assets
                .world_presentation_registration(old_world.1)
                .unwrap()
                .profile
                .image_settings,
            old_profile.image_settings
        );
    }
    let old_image = assets.image(old_body.1).unwrap();
    assert_eq!(old_image.preparation_revision, old_body.2);
    let old_level = &old_image.prepared.as_ref().unwrap().levels[0];
    assert_eq!(
        (old_level.width, old_level.height),
        (old_body.3, old_body.4)
    );
    assert_eq!(old_level.rgba.as_ref(), old_body.5.as_slice());
    assert_eq!(source.backend.cpu.admitted_assets(), Some(assets.counts()));
    queue_snapshot(&mut host, ids[0], 11, 2000);
    queue_snapshot(&mut host, ids[1], 22, 2000);
    source.step(&mut host);
    assert!(source.backend.cpu.world_stats().spans > 0);
    assert_ne!(source.backend.cpu.pixels()[0], 0);
    assert_eq!(
        source.backend.cpu.pixels()[0],
        source.backend.cpu.pixels()[48]
    );
    assert!(source.views.iter().any(|(view, _, _)| *view
        == qa_render::Viewport {
            x: 2,
            y: 2,
            width: 28,
            height: 44
        }));
    assert!(source.views.iter().any(|(view, _, _)| *view
        == qa_render::Viewport {
            x: 50,
            y: 2,
            width: 28,
            height: 44
        }));
    paint(&host, &mut source);
    for module in [ModuleId(1), ModuleId(2), ModuleId(3), ModuleId(4)] {
        assert_eq!(host.module_counts(module).unwrap().traps, 0);
    }
}

fn main() {
    if let Some(status) = qa_platform::native::native_child_bootstrap() {
        std::process::exit(status);
    }
    verify_video_image_restart(
        qvm_role_with_graphics(false, false, true),
        qvm_role_with_graphics(true, false, true),
    );
    verify_video_restart(
        qvm_role_with_graphics(false, false, true),
        qvm_role_with_graphics(true, false, true),
    );
    verify_standalone_ui(qvm_role(true, false));
    verify_renderer_lifecycle(qvm_file(), qvm_role_with_graphics(true, false, true));
    verify_ui_shutdown_failure_isolation(qvm_file(), qvm_role(true, true));
    verify_ui_association_replacement(qvm_file(), qvm_role(true, false));
    println!("actual load_q3 QVM shared UI/cgame renderer lifecycle passed");
    verify_commands(qvm_file());
    println!("actual load_q3 QVM client command imports passed");
    verify(qvm_file());
    println!("actual load_q3 QVM same-lifetime restart checks passed");
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        verify_video_image_restart(
            native_role_with_graphics(false, false, true),
            native_role_with_graphics(true, false, true),
        );
        verify_video_restart(
            native_role_with_graphics(false, false, true),
            native_role_with_graphics(true, false, true),
        );
        verify_standalone_ui(native_role(true, false));
        verify_renderer_lifecycle(native_file(), native_role_with_graphics(true, false, true));
        verify_ui_shutdown_failure_isolation(native_file(), native_role(true, true));
        verify_ui_association_replacement(native_file(), native_role(true, false));
        println!("actual load_q3 owned-native shared UI/cgame renderer lifecycle passed");
        verify_commands(native_file());
        println!("actual load_q3 owned-native client command imports passed");
        verify(native_file());
        println!("actual load_q3 owned-native same-lifetime restart checks passed");
    }
}
