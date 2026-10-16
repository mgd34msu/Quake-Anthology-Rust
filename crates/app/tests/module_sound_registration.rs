#[path = "support/service_program.rs"]
mod service_program;
#[path = "support/sound_registration.rs"]
mod sound_registration;

use qa_app::{
    Runtime,
    host::FrameHost,
    modules::{Export, ModuleRequest, ModuleResult, Phase, Program},
};
use qa_compat::{
    abi::{CallTable, Q3_CLIENT, Q3_UI},
    services::CallContext,
};
use qa_console::{commands::Console, views::Context};
use qa_core::primitives::{CallbackId, ModuleId, RuleSetId, ThinkTime};
use qa_formats::program::qvm::Opcode::*;
use qa_session::timing::TickRate;
use qa_world::{area::LinkOrder, entities::AllocationPolicy};

struct Files(std::path::PathBuf);
impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn request(
    runtime: &mut Runtime,
    module: ModuleId,
    table: &'static CallTable,
    call: i32,
) -> ModuleRequest {
    let anchor = runtime
        .server
        .entities
        .allocate(ThinkTime::Milliseconds(0), module, AllocationPolicy::q3(0))
        .unwrap()
        .id;
    let operations = [
        (Enter, 64),
        (Local, 72),
        (Load4, 0),
        (Const, 1),
        (Eq, 13),
        (Local, 76),
        (Load4, 0),
        (Arg, 8),
        (Const, 1),
        (Arg, 12),
        (Const, -call - 1),
        (Call, 0),
        (Leave, 64),
        // vmMain command one immediately plays an existing native handle.
        (Local, 76),
        (Load4, 0),
        (Arg, 8),
        (Const, 1),
        (Arg, 12),
        (Const, -if call == 34 { 29 } else { 32 } - 1),
        (Call, 0),
        (Leave, 64),
    ];
    let mut data = [0; 256];
    for (offset, path) in [
        (64, b"sound/test.wav\0".as_slice()),
        (96, b"SOUND/TEST.WAV\0".as_slice()),
        (128, b"sound/late.wav\0".as_slice()),
    ] {
        data[offset..offset + path.len()].copy_from_slice(path);
    }
    ModuleRequest {
        q3_image: None,
        context: CallContext {
            module,
            clock: ThinkTime::Milliseconds(0),
            server_frame: 0,
            native_rules: RuleSetId::Quake3,
            trace_rules: RuleSetId::Quake,
            sound_policy: qa_gameplay::rules::sound_policy(RuleSetId::Quake3),
            console: Context::default(),
            allocation: AllocationPolicy::q3(0),
            link_order: LinkOrder::Head,
        },
        timing_rules: RuleSetId::Quake3,
        phase: Phase::Client,
        rate: TickRate::FrameDriven,
        anchor,
        program: Program::Qvm {
            vm: Box::new(service_program::program(&operations, &data)),
            imports: table,
        },
        entries: vec![0, 1],
        frame: Export::words(CallbackId(0), &[64]),
        prepare: Vec::new(),
        initialize: vec![Export::words(CallbackId(0), &[64])],
        api: None,
        shutdown: Vec::new(),
        instruction_budget: 100,
        configstrings: 0,
        files: 0,
        entity_source: None,
    }
}

#[test]
fn client_init_and_late_registration_share_one_bank_and_immediately_play_through_the_mixer() {
    let files = Files(std::env::temp_dir().join(format!(
        "qa-rust-module-sound-registration-{}",
        std::process::id()
    )));
    std::fs::create_dir_all(files.0.join("sound/feedback")).unwrap();
    let wav = sound_registration::wav();
    for path in ["sound/feedback/hit.wav", "sound/test.wav", "sound/late.wav"] {
        std::fs::write(files.0.join(path), &wav).unwrap();
    }
    let mut runtime = Runtime::load(1, []).unwrap();
    runtime.vfs.mount_directory(&files.0, 0).unwrap();
    let requests = [(&Q3_CLIENT, 34), (&Q3_UI, 31)]
        .into_iter()
        .enumerate()
        .map(|(index, (table, call))| {
            request(&mut runtime, ModuleId(index as u16 + 1), table, call)
        })
        .collect();
    let mut host = FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        requests,
    )
    .unwrap();
    qa_app::audio::begin_registration(&mut host, &[], RuleSetId::Quake3).unwrap();
    assert!(host.runtime.sound_assets.is_some());
    assert!(
        host.runtime
            .sound_assets
            .as_ref()
            .unwrap()
            .initial_registration()
    );
    host.initialize_modules(Phase::Server);
    qa_app::audio::load_registered_sounds(&mut host, &[], RuleSetId::Quake3).unwrap();
    host.initialize_modules(Phase::Client);
    let handle = match host
        .module_counts(ModuleId(1))
        .unwrap()
        .last_result
        .unwrap()
    {
        ModuleResult::Qvm(handle) => handle,
        other => panic!("wrong module result: {other:?}"),
    };
    assert!(handle > 0);
    assert_eq!(
        host.module_counts(ModuleId(2)).unwrap().last_result,
        Some(ModuleResult::Qvm(handle))
    );
    qa_app::audio::finish_registration(&mut host).unwrap();
    assert!(
        !host
            .runtime
            .sound_assets
            .as_ref()
            .unwrap()
            .initial_registration()
    );
    let bank = host.runtime.sound_bank().unwrap();
    let sound = bank.find("sound/test.wav").unwrap();
    assert_eq!(bank.registered_sound(handle), Some(sound));
    assert_eq!(
        bank.registered_sound(0),
        bank.find("sound/feedback/hit.wav")
    );
    assert!(bank.find("sound/late.wav").is_none());
    std::fs::remove_file(files.0.join("sound/test.wav")).unwrap();
    let initial_reads = host.runtime.sound_assets.as_ref().unwrap().reads;
    let mut late_handle = None;
    for module in [ModuleId(1), ModuleId(2)] {
        let calls = host.module_counts(module).unwrap().calls;
        host.initialize_modules(Phase::Client);
        assert_eq!(host.module_counts(module).unwrap().calls, calls);
        for path in [96, 128] {
            host.call_module_export(
                module,
                Export::words(CallbackId(0), &[path]),
                ThinkTime::Milliseconds(20),
            )
            .unwrap();
            let Some(ModuleResult::Qvm(result)) = host.module_counts(module).unwrap().last_result
            else {
                panic!("native sound handle");
            };
            if path == 96 {
                assert_eq!(result, handle);
            } else {
                assert!(result > 0);
                assert_ne!(result, handle);
                if let Some(previous) = late_handle {
                    assert_eq!(result, previous);
                }
                late_handle = Some(result);
            }
        }
    }
    assert_eq!(
        host.runtime.sound_assets.as_ref().unwrap().reads,
        initial_reads + 1
    );
    let bank = host.runtime.sound_bank().unwrap();
    assert_eq!(bank.find("sound/test.wav"), Some(sound));
    let late = bank.find("sound/late.wav").unwrap();
    assert_eq!(bank.registered_sound(late_handle.unwrap()), Some(late));
    let mut mixer = qa_audio::Mixer::load(bank.rate, 4, 1);
    host.call_module_export(
        ModuleId(2),
        Export::words(CallbackId(1), &[late_handle.unwrap() as u64]),
        ThinkTime::Milliseconds(21),
    )
    .unwrap();
    let server = &host.runtime.server;
    let mut batch = server.events.batch(server.presentation).unwrap();
    let mut played = 0;
    while let Some(record) = server.events.next(&mut batch) {
        if let qa_core::events::FrameEvent::Sound(event) = record.event {
            assert_eq!(event.sound, late);
            assert_eq!(event.source_ordinal, Some(late_handle.unwrap() as u32));
            assert!(mixer.sound(host.runtime.sound_bank().unwrap(), event));
            played += 1;
        }
    }
    assert_eq!(played, 1);
    let mut pcm = [0; 8];
    assert!(mixer.paint(host.runtime.sound_bank().unwrap(), &mut pcm));
    assert!(pcm.iter().any(|sample| *sample != 0));
}
