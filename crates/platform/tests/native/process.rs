use super::{
    NativeAbi, NativeError, NativeImage, NativeRegion,
    implementation::{NativeProcess, child_main},
};
use std::{os::unix::process::ExitStatusExt, process::Command, time::Duration};

const BASE: u64 = 0x2000_0000;
const REGIONS: [NativeRegion; 2] = [
    NativeRegion {
        offset: 0,
        length: 4096,
        permissions: 5,
    },
    NativeRegion {
        offset: 4096,
        length: 4096,
        permissions: 3,
    },
];

#[test]
fn child_entry() {
    if std::env::var_os("QA_NATIVE_TEST_CHILD").is_some() {
        std::process::exit(if child_main().is_ok() { 0 } else { 125 });
    }
}

fn child(code: &[u8], timeout: Duration) -> Result<NativeProcess, NativeError> {
    let mut bytes = vec![0; 8192];
    bytes[..code.len()].copy_from_slice(code);
    let mut command = Command::new(std::env::current_exe()?);
    command
        .args(["--exact", "native::tests::child_entry", "--nocapture"])
        .env("QA_NATIVE_TEST_CHILD", "1");
    NativeProcess::launch(
        command,
        NativeImage {
            base: BASE,
            pointer_bytes: 8,
            bytes: &bytes,
            regions: &REGIONS,
            timeout,
        },
    )
}

fn standard(code: &[u8]) -> NativeProcess {
    match child(code, Duration::from_secs(3)) {
        Ok(child) => child,
        Err(error) => panic!("native child failed: {error:?}"),
    }
}

#[test]
fn native_system_v_and_microsoft_calls_publish_the_same_owned_memory() {
    // mov rax,[rdi]; add rax,rsi; mov [rdi],rax; ret
    let mut system_v = standard(&[0x48, 0x8b, 0x07, 0x48, 0x01, 0xf0, 0x48, 0x89, 0x07, 0xc3]);
    assert_ne!(system_v.pid(), std::process::id());
    for task in std::fs::read_dir(format!("/proc/{}/task", system_v.pid())).expect("owned tasks") {
        let status = std::fs::read_to_string(task.expect("owned task").path().join("status"))
            .expect("kernel status");
        assert!(status.lines().any(|line| line.starts_with("State:\tT")));
        assert!(status.lines().any(|line| line == "Seccomp:\t2"));
    }
    system_v.memory_mut().expect("parked")[4096..4104].copy_from_slice(&11u64.to_le_bytes());
    let mut arguments = [0; 13];
    arguments[0] = BASE + 4096;
    arguments[1] = 7;
    for wanted in [18u64, 25, 32] {
        assert_eq!(
            system_v
                .invoke(BASE, NativeAbi::SystemV, arguments, |_, _, _| Err(
                    NativeError::Callback
                ))
                .expect("native return"),
            wanted
        );
        assert_eq!(
            &system_v.memory().expect("parked")[4096..4104],
            &wanted.to_le_bytes()
        );
    }
    // mov rax,[rcx]; add rax,rdx; mov [rcx],rax; ret
    let mut microsoft = standard(&[0x48, 0x8b, 0x01, 0x48, 0x01, 0xd0, 0x48, 0x89, 0x01, 0xc3]);
    assert_eq!(
        microsoft
            .invoke(BASE, NativeAbi::Microsoft, arguments, |_, _, _| Err(
                NativeError::Callback
            ))
            .expect("native return"),
        7
    );
    assert_eq!(
        &microsoft.memory().expect("parked")[4096..4104],
        &7u64.to_le_bytes()
    );
}

#[test]
fn native_import_stops_before_engine_access_and_resumes_with_the_reply() {
    // sub rsp,8; mov rax,rdi; mov edi,37; mov rsi,rdx; call rax; add rsp,8; ret
    let mut process = standard(&[
        0x48, 0x83, 0xec, 0x08, 0x48, 0x89, 0xf8, 0xbf, 37, 0, 0, 0, 0x48, 0x89, 0xd6, 0xff, 0xd0,
        0x48, 0x83, 0xc4, 0x08, 0xc3,
    ]);
    let mut arguments = [0; 13];
    arguments[0] = process.callback(NativeAbi::SystemV);
    arguments[2] = BASE + 4096;
    let mut calls = 0;
    let result = process
        .invoke(BASE, NativeAbi::SystemV, arguments, |call, base, memory| {
            calls += 1;
            assert_eq!(call.number, 37);
            assert_eq!(call.arguments[0], base + 4096);
            memory[4096..4104].copy_from_slice(&123u64.to_le_bytes());
            Ok(91)
        })
        .expect("import return");
    assert_eq!(result, 91);
    assert_eq!(calls, 1);
    assert_eq!(
        &process.memory().expect("parked")[4096..4104],
        &123u64.to_le_bytes()
    );
}

#[test]
fn child_fault_and_timeout_leave_other_native_owners_running() {
    let mut healthy = standard(&[0x48, 0x89, 0xf8, 0xc3]); // mov rax,rdi; ret
    let mut fault = standard(&[0x48, 0x31, 0xc0, 0x48, 0x8b, 0x00, 0xc3]); // null read
    let result = fault.invoke(BASE, NativeAbi::SystemV, [0; 13], |_, _, _| {
        Err(NativeError::Callback)
    });
    assert!(
        matches!(result, Err(NativeError::Exited(status)) if status.signal() == Some(11)),
        "{result:?}"
    );
    assert_eq!(fault.pid(), 0);
    let mut timeout = child(&[0xeb, 0xfe], Duration::from_millis(200)).expect("timeout child");
    assert!(matches!(
        timeout.invoke(BASE, NativeAbi::SystemV, [0; 13], |_, _, _| Err(
            NativeError::Callback
        )),
        Err(NativeError::Timeout)
    ));
    assert_eq!(timeout.pid(), 0);
    let mut arguments = [0; 13];
    arguments[0] = 54;
    assert_eq!(
        healthy
            .invoke(BASE, NativeAbi::SystemV, arguments, |_, _, _| Err(
                NativeError::Callback
            ))
            .expect("healthy still runs"),
        54
    );
}

#[test]
fn native_code_cannot_spawn_an_uncontrolled_writer_or_change_page_rights() {
    // syscall clone with all-zero arguments: must fail with EPERM before a child exists.
    let mut process = standard(&[
        0xb8, 56, 0, 0, 0, 0x48, 0x31, 0xff, 0x48, 0x31, 0xf6, 0x48, 0x31, 0xd2, 0x0f, 0x05, 0xc3,
    ]);
    assert_eq!(
        process
            .invoke(BASE, NativeAbi::SystemV, [0; 13], |_, _, _| Err(
                NativeError::Callback
            ))
            .expect("denied clone"),
        u64::MAX
    );
    // mprotect(address,4096,7) is denied even for the child's shared map.
    process.memory_mut().expect("parked")[..8]
        .copy_from_slice(&[0xb8, 10, 0, 0, 0, 0x0f, 0x05, 0xc3]);
    let mut arguments = [0; 13];
    arguments[0] = BASE;
    arguments[1] = 4096;
    arguments[2] = 7;
    assert_eq!(
        process
            .invoke(BASE, NativeAbi::SystemV, arguments, |_, _, _| Err(
                NativeError::Callback
            ))
            .expect("denied rights change"),
        u64::MAX
    );
}

#[test]
fn nonexecutable_entry_and_callback_rejection_are_local() {
    let mut process = standard(&[0xc3]);
    assert!(matches!(
        process.invoke(BASE + 4096, NativeAbi::SystemV, [0; 13], |_, _, _| Err(
            NativeError::Callback
        )),
        Err(NativeError::Extent)
    ));
    assert_ne!(process.pid(), 0);
    assert!(matches!(
        NativeProcess::load(NativeImage {
            base: BASE,
            pointer_bytes: 4,
            bytes: &[],
            regions: &[],
            timeout: Duration::from_secs(1)
        }),
        Err(NativeError::Unsupported)
    ));
    let pid = process.pid();
    drop(process);
    assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
}
