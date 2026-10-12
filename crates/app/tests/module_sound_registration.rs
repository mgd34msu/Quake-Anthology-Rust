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
        (Local, 76),
        (Load4, 0),
        (Arg, 8),
        (Const, 1),
        (Arg, 12),
        (Const, -call - 1),
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
        entries: vec![0],
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
fn client_init_shares_one_bank_and_frozen_registration_never_reopens_pcm() {
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
    assert!(host.runtime.sound_loader.is_some());
    assert!(host.runtime.sound_bank.is_none());
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
    let bank = qa_app::audio::finish_registration(&mut host).unwrap();
    assert!(host.runtime.sound_loader.is_none());
    assert!(std::sync::Arc::ptr_eq(
        host.runtime.sound_bank.as_ref().unwrap(),
        &bank
    ));
    let sound = bank.find("sound/test.wav").unwrap();
    assert_eq!(bank.registered_sound(handle), Some(sound));
    assert_eq!(
        bank.registered_sound(0),
        bank.find("sound/feedback/hit.wav")
    );
    assert!(bank.find("sound/late.wav").is_none());
    std::fs::remove_file(files.0.join("sound/test.wav")).unwrap();
    for module in [ModuleId(1), ModuleId(2)] {
        let calls = host.module_counts(module).unwrap().calls;
        host.initialize_modules(Phase::Client);
        assert_eq!(host.module_counts(module).unwrap().calls, calls);
        for (path, expected) in [(96, handle), (128, 0)] {
            host.call_module_export(
                module,
                Export::words(CallbackId(0), &[path]),
                ThinkTime::Milliseconds(20),
            )
            .unwrap();
            assert_eq!(
                host.module_counts(module).unwrap().last_result,
                Some(ModuleResult::Qvm(expected))
            );
        }
    }
    assert!(bank.find("sound/late.wav").is_none());
    assert_eq!(bank.find("sound/test.wav"), Some(sound));
}
