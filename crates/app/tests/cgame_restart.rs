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
    renderer::{SceneComposer, hud::HudView},
};
use qa_audio::{Bank, Listener, Mixer};
use qa_compat::services::ModelLookup;
use qa_console::{commands::Console, views::Context};
use qa_core::{
    loopback::Endpoint,
    primitives::*,
    sys_events::{EventKind, EventTime, SeatId, SysEvent, SysEventQueue},
};
use qa_formats::program::qvm::Opcode;
use qa_network::{channel::commands::write_gamestate, commands::packet::Protocol};
use qa_render::{Assets, Limits, scene::Command};
use qa_session::timing::TickRate;
use std::time::Duration;

const INITIAL: u32 = 0x4000;
const INLINE: u32 = 0x4004;
const CLIENT: u32 = 0x4008;
const SOUND: u32 = 0x400c;
const PATH: u32 = 0x4018;
const INIT_TEXT: u32 = 0x4200;
const STOP_TEXT: u32 = 0x4220;
const ERROR_TEXT: u32 = 0x4240;
const FAILED_INIT_TEXT: u32 = 0x4280;
const REFDEF: u32 = 0x4300;
const SOUND_PATH: u32 = 0x4500;
const ORIGIN: u32 = 0x4600;
const AXES: u32 = 0x4640;
const COMMAND_OWNED: u32 = 0x4680;
const COMMAND_SHARED: u32 = 0x46a0;
const CLIENT_TEXT: u32 = 0x46c0;
const BSS: u32 = 0x7000;
const GAMESTATE: u32 = 0x8000;

fn data() -> Vec<u8> {
    let mut data = vec![0; 0x5000];
    data[INITIAL as usize..INITIAL as usize + 4].copy_from_slice(&7u32.to_le_bytes());
    for (at, text) in [
        (INIT_TEXT, b"restart init\n\0".as_slice()),
        (STOP_TEXT, b"restart shutdown\n\0".as_slice()),
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
fn qvm_check(ops: &mut Vec<(Opcode, i32)>, at: u32, value: u32) {
    use Opcode::*;
    ops.extend([
        (Const, at as i32),
        (Load4, 0),
        (Const, value as i32),
        (Eq, 0),
    ]);
    let branch = ops.len() - 1;
    qvm_call(ops, 1, &[ImportArg::Pointer(ERROR_TEXT)]);
    ops.push((Pop, 0));
    ops[branch].1 = ops.len() as i32;
}
fn qvm_file() -> Vec<u8> {
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
        match command {
            0 => {
                qvm_check(&mut ops, INITIAL, 7);
                qvm_check(&mut ops, BSS, 0);
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
        _: &[Option<qa_app::host::ClientView>; SeatId::COUNT],
        _: &[Option<qa_app::client_scene::ClientSceneInput<'_>>; SeatId::COUNT],
        _: &[Option<HudView<'_>>; SeatId::COUNT],
        _: qa_console::presentation::ConsoleView<'_>,
    ) {
        self.views.clear();
        let (Some(scene), Some(resources)) = (scene, resources) else {
            return;
        };
        let Some(packet) = scene.finish_frame() else {
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
    let mut fixture = files::Fixture::new(&[
        ("vm/restart", module),
        ("maps/a.bsp", files::world_map(8.)),
        ("maps/b.bsp", files::world_map(8.)),
        ("maps/c.bsp", files::world_map(8.)),
        ("sound/restart.wav", samples::wav()),
    ]);
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
    load_q3(
        &mut runtime,
        &[
            Q3Spec::parse("cgame:0:vm/restart").unwrap(),
            Q3Spec::parse("cgame:1:vm/restart").unwrap(),
        ],
        &mut requests,
        TickRate::FrameDriven,
        None,
    )
    .unwrap();
    assert!(requests.iter().all(|request| request.q3_image.is_some()));
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
    let backend = asset_frames::Source::load(owner.assets());
    runtime.model_assets = Some(owner);
    runtime.scene = Some(scene);
    let mut host = FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        requests,
    )
    .unwrap();
    host.local_clients[0] = Some(ids[0]);
    host.local_clients[1] = Some(ids[1]);
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
    let source = Source {
        time: 0,
        backend,
        mixer: Mixer::load(rate, 16, SeatId::COUNT),
        reject_reset: None,
        listener_receipts: [None; 2],
        play_receipts: [0; 2],
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

fn main() {
    if let Some(status) = qa_platform::native::native_child_bootstrap() {
        std::process::exit(status);
    }
    verify_commands(qvm_file());
    println!("actual load_q3 QVM client command imports passed");
    verify(qvm_file());
    println!("actual load_q3 QVM same-lifetime restart checks passed");
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        verify_commands(native_file());
        println!("actual load_q3 owned-native client command imports passed");
        verify(native_file());
        println!("actual load_q3 owned-native same-lifetime restart checks passed");
    }
}
