//! Real parent termination, without running a game or relying on Rust Drop.
use super::{BASE, NativeAbi, NativeError, NativeScalar, child};
use crate::native::implementation::bind_parent_death;
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::process::ExitStatusExt,
    process::{Child, Command, Stdio},
    time::Duration,
};

const TEST: &str = "native::tests::parent_death_tests::";
const SIGKILL: i32 = 9;
const SIGCONT: i32 = 18;
const SIGSTOP: i32 = 19;

unsafe extern "C" {
    fn prctl(operation: i32, ...) -> i32;
    fn kill(pid: i32, signal: i32) -> i32;
    fn waitpid(pid: i32, status: *mut i32, flags: i32) -> i32;
}

fn helper(test: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args(["--exact", &format!("{TEST}{test}"), "--nocapture"]);
    command
}

struct Owned {
    parent: Child,
    native: Option<u32>,
}
impl Drop for Owned {
    fn drop(&mut self) {
        // These are the two PIDs created and recorded by this fixture only.
        let _ = self.parent.kill();
        let _ = self.parent.wait();
        if let Some(pid) = self.native.take() {
            unsafe {
                kill(pid as i32, SIGKILL);
                waitpid(pid as i32, &mut 0, 0);
            }
        }
    }
}

fn state(pid: u32) -> Option<char> {
    std::fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("State:\t"))?
        .chars()
        .next()
}

#[test]
fn native_children_die_when_the_engine_parent_is_killed() {
    // Isolate subreaping from the ordinary test process and parallel owners.
    if std::env::var_os("QA_PARENT_DEATH_COLLECTOR").is_none() {
        assert!(
            helper("native_children_die_when_the_engine_parent_is_killed")
                .env("QA_PARENT_DEATH_COLLECTOR", "1")
                .status()
                .unwrap()
                .success()
        );
        return;
    }
    // SAFETY: this dedicated fixture owns every descendant; subreaping lets it
    // collect the exact native PID after killing its engine parent, avoiding
    // both init-reaping races and zombie residue.
    assert_eq!(unsafe { prctl(36, 1, 0, 0, 0) }, 0);
    for mode in ["parked", "spinning", "startup-race"] {
        let parent = helper("engine_parent")
            .env("QA_PARENT_DEATH_OWNER", mode)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut owned = Owned {
            parent,
            native: None,
        };
        let mut reader = BufReader::new(owned.parent.stdout.take().unwrap());
        let mut line = String::new();
        loop {
            line.clear();
            assert_ne!(reader.read_line(&mut line).unwrap(), 0, "{mode}");
            if let Some(pid) = line.trim().strip_prefix("QA_NATIVE_PID:") {
                owned.native = Some(pid.parse().unwrap());
                break;
            }
        }
        let pid = owned.native.unwrap();
        let expected = match mode {
            "spinning" => 'R',
            "startup-race" => 'T',
            _ => 'D',
        };
        let started = crate::Stopwatch::start();
        while if mode == "parked" {
            !matches!(state(pid), Some('S' | 'D'))
        } else {
            state(pid) != Some(expected)
        } {
            assert!(started.elapsed() < Duration::from_secs(2), "{mode}");
            crate::pause(Duration::from_millis(1));
        }
        owned.parent.kill().unwrap();
        assert_eq!(owned.parent.wait().unwrap().signal(), Some(SIGKILL));
        if mode == "startup-race" {
            // This child deliberately stopped immediately before the binding.
            // Its now-dead known parent must be rejected when startup resumes.
            assert_eq!(unsafe { kill(pid as i32, SIGCONT) }, 0);
        }
        let started = crate::Stopwatch::start();
        let status = loop {
            let mut status = 0;
            // SAFETY: subreaping transferred this recorded descendant to us.
            let result = unsafe { waitpid(pid as i32, &mut status, 1) };
            if result == pid as i32 {
                break std::process::ExitStatus::from_raw(status);
            }
            assert_eq!(result, 0, "{mode}: {}", std::io::Error::last_os_error());
            assert!(started.elapsed() < Duration::from_secs(2), "{mode}");
            crate::pause(Duration::from_millis(1));
        };
        owned.native = None;
        if mode == "startup-race" {
            // Parent death can deliver SIGKILL while the binding is being
            // installed, before getppid reports the mismatch to the fixture.
            assert!(
                status.success() || status.signal() == Some(SIGKILL),
                "{status:?}"
            );
        } else {
            assert_eq!(status.signal(), Some(SIGKILL), "{mode}");
        }
        assert_eq!(state(pid), None, "{mode}: descendant was not reaped");
    }
}

#[test]
fn engine_parent() {
    let Some(mode) = std::env::var_os("QA_PARENT_DEATH_OWNER") else {
        return;
    };
    if mode == "startup-race" {
        let mut process = helper("child_before_parent_binding")
            .env("QA_PARENT_DEATH_EXPECTED", std::process::id().to_string())
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        println!("QA_NATIVE_PID:{}", process.id());
        std::io::stdout().flush().unwrap();
        process.wait().unwrap();
        panic!("the fixture must terminate its engine parent");
    }
    let mut process = child(&[0xeb, 0xfe], Duration::from_secs(5)).unwrap();
    println!("QA_NATIVE_PID:{}", process.pid());
    std::io::stdout().flush().unwrap();
    if mode == "spinning" {
        let entry = process
            .bind(BASE, NativeAbi::SystemV, &[], NativeScalar::Void)
            .unwrap();
        let _ = process.invoke(entry, [0; 13], |_, _, _| Err(NativeError::Callback));
        panic!("the fixture must terminate its engine parent");
    }
    loop {
        crate::pause(Duration::from_secs(1));
    }
}

#[test]
fn child_before_parent_binding() {
    let Some(parent) = std::env::var_os("QA_PARENT_DEATH_EXPECTED") else {
        return;
    };
    // SAFETY: stop only this owned fixture PID to hold the pre-binding race.
    assert_eq!(unsafe { kill(std::process::id() as i32, SIGSTOP) }, 0);
    let parent = parent.to_str().unwrap().parse().unwrap();
    assert_eq!(
        bind_parent_death(parent).unwrap_err().raw_os_error(),
        Some(3)
    );
    std::process::exit(0);
}
