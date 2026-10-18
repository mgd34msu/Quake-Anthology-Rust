use super::*;
use qa_app::modules::{Q2Spec, Q3Spec, QuakeCSpec, State, load_q2, load_q3, load_quakec};
use qa_content::vfs::Vfs;
use std::path::PathBuf;

struct Files(PathBuf);
impl Files {
    fn new(label: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("qa-module-sources-{}-{label}", std::process::id()));
        let _ = qa_platform::files::remove_dir_all(&root);
        qa_platform::files::create_dir_all(&root).unwrap();
        Self(root)
    }
    fn mount(
        &self,
        vfs: &mut Vfs,
        label: &str,
        marker: i32,
        text: &[u8],
    ) -> qa_content::vfs::SearchView {
        let path = self.0.join(label);
        qa_platform::files::create_dir_all(path.join("vm")).unwrap();
        qa_platform::files::write(path.join("vm/ui.qvm"), ui_program(marker)).unwrap();
        qa_platform::files::write(path.join("source.txt"), text).unwrap();
        let mount = vfs.mount_directory(&path, marker).unwrap();
        vfs.search_view([mount]).unwrap()
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = qa_platform::files::remove_dir_all(&self.0);
    }
}

fn ui_program(marker: i32) -> Vec<u8> {
    use qa_formats::program::qvm::Opcode::*;
    let mut ops = vec![(Enter, 64), (Local, 72), (Load4, 0), (Const, 0), (Eq, 0)];
    // UI vmMain imports FS_FOpenFile/FS_Read/FS_FClose and Print through the
    // session CallTable, including during initialization and shutdown.
    ops.extend([
        (Const, 16),
        (Arg, 8),
        (Const, 64),
        (Arg, 12),
        (Const, 0),
        (Arg, 16),
        (Const, -14),
        (Call, 0),
        (Pop, 0),
        (Const, 80),
        (Arg, 8),
        (Const, 3),
        (Arg, 12),
        (Const, 64),
        (Load4, 0),
        (Arg, 16),
        (Const, -15),
        (Call, 0),
        (Pop, 0),
        (Const, 64),
        (Load4, 0),
        (Arg, 8),
        (Const, -17),
        (Call, 0),
        (Pop, 0),
        (Const, 80),
        (Arg, 8),
        (Const, -2),
        (Call, 0),
        (Pop, 0),
        (Const, marker),
        (Leave, 64),
    ]);
    ops[4].1 = ops.len() as i32;
    ops.extend([(Const, 4), (Leave, 64)]);
    let mut data = [0; 128];
    data[16..27].copy_from_slice(b"source.txt\0");
    service_program::bytes(&ops, &data)
}

fn prints(host: &FrameHost) -> Vec<Vec<u8>> {
    let events = &host.runtime.server.events;
    let mut batch = events.batch(host.runtime.server.presentation).unwrap();
    let mut out = Vec::new();
    while let Some(record) = events.next(&mut batch) {
        if let FrameEvent::Print(print) = record.event {
            out.push(events.texts.get(print.text).unwrap().to_vec());
        }
    }
    out
}

#[test]
fn module_program_and_all_lifecycle_file_calls_retain_the_same_sources() {
    let files = Files::new("lifecycle");
    let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
    let low = files.mount(&mut runtime.vfs, "low", 111, b"low");
    let high = files.mount(&mut runtime.vfs, "high", 222, b"top");
    let mut specs = Vec::new();
    for search in [Some(low.clone()), Some(high), None] {
        let mut spec = Q3Spec::parse("ui:vm/ui.qvm").unwrap();
        spec.search = search;
        specs.push(spec);
    }
    let mut requests = Vec::new();
    load_q3(
        &mut runtime,
        &specs,
        &mut requests,
        TickRate::FrameDriven,
        None,
    )
    .unwrap();
    assert!(requests[0].search.as_ref().unwrap().same_selection(&low));
    let mut host = FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        requests,
    )
    .unwrap();
    host.initialize_modules(Phase::Client);
    for (module, marker) in [(ModuleId(1), 111), (ModuleId(2), 222), (ModuleId(3), 222)] {
        assert_eq!(host.module_state(module), Some(State::Running));
        assert_eq!(
            host.module_counts(module).unwrap().last_result,
            Some(ModuleResult::Qvm(marker))
        );
    }
    assert_eq!(
        prints(&host),
        [b"low".to_vec(), b"top".to_vec(), b"top".to_vec()]
    );
    files.mount(&mut host.runtime.vfs, "later", 333, b"new");
    for (module, marker) in [(ModuleId(1), 111), (ModuleId(2), 222), (ModuleId(3), 222)] {
        host.call_module_export(
            module,
            Export::clocked(CallbackId(5)),
            ThinkTime::Milliseconds(50),
        )
        .unwrap();
        assert_eq!(
            host.module_counts(module).unwrap().last_result,
            Some(ModuleResult::Qvm(marker))
        );
    }
    host.shutdown_modules();
    assert_eq!(
        prints(&host),
        [
            b"low", b"top", b"top", b"low", b"top", b"new", b"low", b"top", b"new"
        ]
        .map(Vec::from)
    );
}

#[test]
fn foreign_module_sources_are_rejected_before_program_loading_or_guest_callbacks() {
    let files = Files::new("foreign");
    let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
    let mut other = Vfs::default();
    let foreign = files.mount(&mut other, "foreign", 123, b"bad");
    let before = runtime.server.entities.len();
    let mut q3 = Q3Spec::parse("ui:vm/ui.qvm").unwrap();
    q3.search = Some(foreign.clone());
    assert!(
        load_q3(
            &mut runtime,
            &[q3],
            &mut Vec::new(),
            TickRate::FrameDriven,
            None
        )
        .is_err()
    );
    let mut qc = QuakeCSpec::parse("q1:progs.dat").unwrap();
    qc.search = Some(foreign.clone());
    assert!(load_quakec(&mut runtime, &[qc], None).is_err());
    let mut q2 = Q2Spec::parse("q2:game.so").unwrap();
    q2.search = Some(foreign.clone());
    assert!(load_q2(&mut runtime, &[q2], &mut Vec::new(), "base1", 0).is_err());
    assert_eq!(runtime.server.entities.len(), before);

    files.mount(&mut runtime.vfs, "local", 222, b"top");
    let mut requests = Vec::new();
    load_q3(
        &mut runtime,
        &[Q3Spec::parse("ui:vm/ui.qvm").unwrap()],
        &mut requests,
        TickRate::FrameDriven,
        None,
    )
    .unwrap();
    requests[0].search = Some(foreign);
    let error = FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        requests,
    )
    .err()
    .unwrap();
    assert!(error.contains("module filesystem binding"), "{error}");
}
