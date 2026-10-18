use super::*;
use qa_app::{
    model_assets::{ModelAssets, ModelRequest},
    renderer::{AssetPreparer, SceneComposer},
};
use qa_compat::services::ModelLookup;
use qa_render::{Assets, cpu::CpuBackend};

const MODEL: u32 = 80;
const SKIN: u32 = 84;
const SHADER: u32 = 88;
const CACHED: u32 = 92;
const TAG_RESULT: u32 = 96;
const AFTER_KEY: u32 = 100;
const NESTED: u32 = 104;
const TRIGGER: u32 = 108;
const ORIENTATION: u32 = 112;
const MINS: u32 = 164;
const MAXS: u32 = 176;
const FRAMES: u32 = 192;
const DRAWS: u32 = 196;
const VIEW: u32 = 208;
const MODEL_NAME: u32 = 592;
const SKIN_NAME: u32 = 624;
const SHADER_NAME: u32 = 656;
const TAG_NAME: u32 = 688;
const ENTITY: u32 = 720;
const COMMAND: u32 = 880;

fn registrations(ui: bool) -> Vec<Op> {
    use Value::*;
    vec![
        call(
            if ui { 18 } else { 37 },
            &[Pointer(MODEL_NAME)],
            Some(MODEL),
        ),
        call(
            if ui { 56 } else { 47 },
            &[Word(MODEL), Pointer(MINS), Pointer(MAXS)],
            None,
        ),
        call(
            if ui { 29 } else { 48 },
            &[
                Pointer(ORIENTATION),
                Word(MODEL),
                Constant(0),
                Constant(1),
                Constant(0.25f32.to_bits()),
                Pointer(TAG_NAME),
            ],
            Some(TAG_RESULT),
        ),
    ]
}

fn skin_and_shader(ui: bool) -> Vec<Op> {
    vec![
        call(
            if ui { 19 } else { 38 },
            &[Value::Pointer(SKIN_NAME)],
            Some(SKIN),
        ),
        call(
            if ui { 20 } else { 39 },
            &[Value::Pointer(SHADER_NAME)],
            Some(SHADER),
        ),
    ]
}

struct Source {
    input: super::Source,
    backend: CpuBackend,
    attempts: usize,
    reject_next: bool,
    views: u32,
    models: u32,
    rejected: u32,
}
impl AssetPreparer for Source {
    fn prepare_assets(
        &mut self,
        assets: &Assets,
        scene: Option<qa_render::Limits>,
    ) -> Result<(), String> {
        self.attempts += 1;
        if self.reject_next {
            self.reject_next = false;
            return Err("fixture rejects the first native registration".to_owned());
        }
        self.backend
            .prepare_assets(assets, scene)
            .map_err(str::to_owned)
    }
}
impl FrameSource for Source {
    fn asset_preparer(&mut self) -> Option<&mut (dyn AssetPreparer + '_)> {
        Some(self)
    }
    fn begin_frame(&mut self) -> Duration {
        self.input.begin_frame()
    }
    fn poll_events(&mut self, queue: &mut SysEventQueue) {
        self.input.poll_events(queue);
    }
    fn wait_time(&mut self, span: Duration) -> Duration {
        self.input.wait_time(span)
    }
    fn elapsed(&self) -> Duration {
        self.input.elapsed()
    }
    fn present(&mut self) {}
    fn render(
        &mut self,
        composer: Option<&mut SceneComposer>,
        resources: Option<qa_app::model_assets::ModelView<'_>>,
        native: &dyn qa_render::shader::reference::NativeShaders,
        _: &[Option<qa_app::host::ClientView>; SeatId::COUNT],
        _: &[Option<qa_app::client_scene::ClientSceneInput<'_>>; SeatId::COUNT],
        _: &[Option<qa_app::renderer::hud::HudView<'_>>; SeatId::COUNT],
        _: qa_console::presentation::ConsoleView<'_>,
    ) {
        let composer = composer.unwrap();
        let resources = resources.unwrap();
        let mut packet = composer.finish_frame().unwrap();
        for command in packet.commands() {
            if let qa_render::scene::Command::View(view) = command {
                self.models += view.scene.entities.count;
            }
        }
        packet.resolve_shaders(resources.assets, native);
        let stats = self.backend.render(&packet, resources.assets);
        self.views += stats.views;
        self.rejected += stats.rejected;
        assert!(composer.recycle(packet).is_ok());
    }
}

fn seed(host: &mut FrameHost, ui: bool) {
    let mut bytes = [0u8; 1024];
    for (at, text) in [
        (MODEL_NAME, b"models/test.md3\0".as_slice()),
        (SKIN_NAME, b"models/test.skin\0"),
        (SHADER_NAME, b"textures/glow\0"),
        (TAG_NAME, b"tag_weapon\0"),
        (COMMAND, b"late_assets\n\0"),
    ] {
        bytes[at as usize..at as usize + text.len()].copy_from_slice(text);
    }
    for (at, value) in [(8, if ui { 64u32 } else { 32 }), (12, 64), (76, 1)] {
        bytes[VIEW as usize + at..VIEW as usize + at + 4].copy_from_slice(&value.to_le_bytes());
    }
    for (at, value) in [(16, 90f32), (20, 90.), (36, 1.), (52, 1.), (68, 1.)] {
        bytes[VIEW as usize + at..VIEW as usize + at + 4]
            .copy_from_slice(&value.to_bits().to_le_bytes());
    }
    for (at, value) in [(28, 1f32), (44, 1.), (60, 1.), (68, 32.), (84, 32.)] {
        bytes[ENTITY as usize + at..ENTITY as usize + at + 4]
            .copy_from_slice(&value.to_bits().to_le_bytes());
    }
    bytes[ENTITY as usize + 116..ENTITY as usize + 120].fill(255);
    // The native helper treats offset zero as a null pointer; it is already
    // initialized and is not part of this fixture's output records.
    for (index, bytes) in bytes.chunks_exact(4).enumerate().skip(1) {
        write(host, 0, ui, index as u32 * 4, word(bytes));
    }
}

pub(super) fn run(native: bool) {
    use Value::*;
    let mut files = model_support::Fixture::surfaces();
    let mut host = fixture_config(
        native,
        |seat, ui, bodies| {
            if seat == 0 {
                bodies[30] = vec![
                    call(
                        if ui { 18 } else { 37 },
                        &[Pointer(MODEL_NAME)],
                        Some(MODEL),
                    ),
                    Op::Return(Word(MODEL)),
                ];
                if ui {
                    let mut key = registrations(true);
                    key.extend([
                        call(12, &[Constant(0), Pointer(COMMAND)], None),
                        call(18, &[Pointer(MODEL_NAME)], Some(CACHED)),
                        Op::Increment(AFTER_KEY),
                        Op::Return(Constant(0)),
                    ]);
                    bodies[3] = key;
                    let mut console = skin_and_shader(true);
                    console.extend([Op::Increment(NESTED), Op::Return(Constant(1))]);
                    bodies[8] = console;
                } else {
                    let mut draw = vec![Op::Increment(DRAWS)];
                    draw.extend(registrations(false));
                    draw.extend(skin_and_shader(false));
                    draw.extend([
                        call(40, &[], None),
                        Op::Write(ENTITY + 8, Word(MODEL)),
                        Op::Write(ENTITY + 108, Word(SKIN)),
                        call(41, &[Pointer(ENTITY)], None),
                        call(44, &[Pointer(VIEW)], None),
                    ]);
                    bodies[0] = vec![
                        Op::Increment(FRAMES),
                        Op::IfEqual(Word(TRIGGER), Constant(1), draw),
                        Op::Return(Constant(0)),
                    ];
                }
            }
            vec![]
        },
        |_| {},
        Some(std::mem::take(&mut files.vfs)),
    );
    let profile = ModelRequest::native("", RuleSetId::Quake3, None).profile();
    let mut owner = ModelAssets::new(
        &host.runtime.vfs,
        Assets::load().unwrap(),
        [profile],
        [],
        Default::default(),
    )
    .unwrap();
    host.runtime.scene = Some(
        SceneComposer::load(qa_render::Limits::default(), owner.assets_mut(), vec![])
            .unwrap()
            .0,
    );
    let backend = CpuBackend::load_with_limits(
        64,
        64,
        owner.assets(),
        qa_render::cpu::CpuLimits {
            cache_bytes: 4096,
            ..Default::default()
        },
    )
    .unwrap();
    let initial = backend.admitted_assets().unwrap();
    host.runtime.model_assets = Some(owner);
    host.bind_client_model_assets([]);
    host.publish_model_admission(initial).unwrap();
    host.configure_native_scenes(
        &[None, None, None, None, Some(profile)],
        &[[None, None, None, None, Some(profile)]; SeatId::COUNT],
        64,
        64,
    )
    .unwrap();
    seed(&mut host, true);
    seed(&mut host, false);
    // The same public export without a backend remains cache-only.
    assert_eq!(invoke(&mut host, 0, true, 30, &[]), 0);
    let baseline_provider = host.module_counts(ModuleId(5)).unwrap().calls;
    let mut source = Source {
        input: super::Source::default(),
        backend,
        attempts: 0,
        reject_next: true,
        views: 0,
        models: 0,
        rejected: 0,
    };
    catch(&mut host, 0, true, key_catchers::UI);
    for repeat in [false, true] {
        source.input.time += 16;
        source
            .input
            .pending
            .extend([vec![key(DeviceId::Keyboard(10), 4, true, repeat)], vec![]]);
        let polls = source.input.polls;
        let attempts = source.attempts;
        let frame = host.frame(&mut source, true);
        assert_eq!(frame.drains, 2);
        assert_eq!(source.input.polls - polls, 2);
        assert!(!host.runtime.quit);
        assert_eq!(read(&mut host, 0, true, AFTER_KEY), 1 + u32::from(repeat));
        assert_eq!(read(&mut host, 0, true, NESTED), 1 + u32::from(repeat));
        assert!(read(&mut host, 0, true, SKIN) > 0);
        assert!(read(&mut host, 0, true, SHADER) > 0);
        if !repeat {
            assert_eq!(read(&mut host, 0, true, MODEL), 0);
            assert!(read(&mut host, 0, true, CACHED) > 0);
            assert!(source.attempts > attempts);
            // The failed model can be retried after other assets were admitted.
            write(&mut host, 0, false, TRIGGER, 1);
            // A bound cgame waits for a native snapshot. Submit through the
            // production channel; the next physical drain accepts its packet.
            assert!(host.runtime.send_snapshot(
                ClientId(0),
                EventTime::from_milliseconds(source.input.time),
                None,
                &mut source,
                |_, _| None,
                |_, _, _, _| None,
            ));
        } else {
            assert_eq!(source.attempts, attempts);
            let handle = read(&mut host, 0, true, MODEL);
            assert!(handle > 0);
            assert_eq!(read(&mut host, 0, true, CACHED), handle);
            let catalog = host.runtime.model_assets.as_ref().unwrap().catalog();
            let bounds = catalog.model_bounds(handle as i32).unwrap();
            let tag = catalog
                .lerp_tag(handle as i32, 0, 1, 0.25, b"tag_weapon")
                .unwrap();
            for (at, values) in [
                (MINS, bounds.mins.0),
                (MAXS, bounds.maxs.0),
                (ORIENTATION, tag.origin.0),
            ] {
                for (axis, value) in values.into_iter().enumerate() {
                    assert_eq!(
                        read(&mut host, 0, true, at + axis as u32 * 4),
                        value.to_bits()
                    );
                }
            }
            let trigger = read(&mut host, 0, false, TRIGGER);
            let frames = read(&mut host, 0, false, FRAMES);
            let draws = read(&mut host, 0, false, DRAWS);
            let client_model = read(&mut host, 0, false, MODEL);
            assert!(
                source.models > 0,
                "cgame calls={} result={:?} trigger={} sample={:?} packet={:?} views={} rejected={} frames={frames} draws={draws} model={client_model}",
                host.module_counts(module(0, false)).unwrap().calls,
                host.module_counts(module(0, false)).unwrap().last_result,
                trigger,
                host.runtime.snapshots[0].as_ref().unwrap().interpolation(),
                host.runtime
                    .network
                    .get(ClientId(0), Endpoint::Client)
                    .unwrap()
                    .channel
                    .current_snapshot()
                    .map(|frame| (frame.sequence(), frame.time())),
                source.views,
                source.rejected,
            );
            assert!(source.views > 0);
            assert_eq!(source.rejected, 0);
        }
    }
    let attempts = source.attempts;
    assert!(invoke(&mut host, 0, true, 30, &[]) > 0);
    assert_eq!(source.attempts, attempts);
    assert!(host.module_counts(ModuleId(5)).unwrap().calls > baseline_provider);
    for module in 1..=5 {
        assert_eq!(host.module_counts(ModuleId(module)).unwrap().traps, 0);
    }
    println!(
        "late registration through queued input, nested console and CLIENT scene: {}",
        if native { "owned child" } else { "QVM" }
    );
}
