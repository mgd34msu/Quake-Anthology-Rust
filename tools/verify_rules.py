#!/usr/bin/env python3
"""Plant isolated rule violations and prove the build stops before compilation."""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
CASES = {
    "shared-mutable-pool": "struct Bad { v: Rc<RefCell<i32>> }",
    "numeric-emulation": "type Bad = NumericOps;",
    "string-keyed-state": "struct Bad { v: HashMap<String, i32> }",
    "crypto-hash": "fn bad() { Sha256::new(); }",
    "content-fingerprint": "fn bad() { DefaultHasher::new(); }",
    "retired-identifiers": "fn donor_step() {}",
    "family-gate": "fn bad() { match kind { GameFamily::Quake => () } }",
    "panic-or-unwrap": "fn bad() { value.unwrap(); }",
    "duplicate-primitive": "struct PlayerState {}",
    "duplicate-md4": "struct Md4 {}",
    "platform-event-source": "fn bad() { std::time::Instant::now(); SystemTime::now(); SDL_PollEvent(&mut event); }",
    "platform-network-source": "fn bad() { UdpSocket::bind(address); }",
    "duplicate-event-storage": "struct SoundRing { values: [Option<SoundEvent>; 16] }",
    "diagnostics-path": 'fn bad() { eprintln!("event"); }',
    "temporary-diagnostics": "// TEMP-DIAG\nfn bad() {}",
    "test-share": "fn live() {}\n#[cfg(test)]\nmod tests { " + "fn scenario() {} " * 50 + "}",
}

PRODUCTION = "\n".join(f"fn live_{index}(value: u32) -> u32 {{ value + {index} }}" for index in range(20))
BOUNDARY_TEST = "#[cfg(test)]\nfn test_only() {}"


def production_chars(length):
    base = "const P:u8=0;"
    return "const P" + "_" * (length - len(base)) + ":u8=0;"


NESTED_TEST = "#[cfg(test)] mod oracle { #[cfg(test)] fn nested() {} }"
TEST_SHARE_ALLOWED = {
    "test-share-enum-variant": "enum Choice { Live, #[cfg(test)] Oracle(u32, [u8; 2]), Last }\n" + PRODUCTION,
    "test-share-struct-variant": "enum Choice { Live, #[cfg(test)] Oracle { value: u32, bytes: [u8; 2] }, Last }\n" + PRODUCTION,
    "test-share-struct-field": "struct Fields { live: u32, #[cfg(test)] oracle: Result<Vec<[u8; 2]>, (u8, u16)>, last: u32 }\n" + PRODUCTION,
    "test-share-tuple-field": "struct Fields(u32, #[cfg(test)] Vec<u32>, u16);\n" + PRODUCTION,
    "test-share-initializer-field": "struct Fields { live: u32, #[cfg(test)] oracle: Vec<[u32; 2]> } fn make() -> Fields { Fields { live: 0, #[cfg(test)] oracle: vec![[0, 1], [2, 3]], } }\n" + PRODUCTION,
    "test-share-match-arms": "fn select(value: u8) -> u8 { match value { #[cfg(test)] 7 => helper::<u32, Vec<u8>>(), #[cfg(test)] 8 => return 1, #[cfg(test)] 9 => { if value == 9 { 2 } else { 3 } } _ => 0 } }\n" + PRODUCTION,
    "test-share-optional-attributes": "#[allow(dead_code)] #[cfg(test)] #[derive(Clone)] struct Oracle { value: u32 }\n" + PRODUCTION,
    "test-share-early-function": '#[cfg(test)] fn oracle() { let brackets = "{[,,;]}"; let _ = brackets; }\n' + PRODUCTION,
    "test-share-early-module": "#[cfg(test)] mod oracle { fn nested() {} }\n" + PRODUCTION,
    "test-share-external-path": '#[cfg(test)] #[path = "outside.rs"] mod oracle; fn live() {}',
    "test-share-external-module": "#[cfg(test)] mod oracle; fn live() {}",
    "test-share-nested-once": production_chars(len(NESTED_TEST) * 2) + NESTED_TEST,
    "test-share-exact-40-percent": production_chars(len(BOUNDARY_TEST) * 3 // 2) + BOUNDARY_TEST,
    "test-share-below-40-percent": production_chars(len(BOUNDARY_TEST) * 3 // 2 + 1) + BOUNDARY_TEST,
}
TEST_SHARE_REJECTED = {
    "test-share-above-40-percent": production_chars(len(BOUNDARY_TEST) * 3 // 2 - 1) + BOUNDARY_TEST,
    "test-share-large-function": "#[cfg(test)] fn oracle() { " + "let value = 1; " * 50 + "}\nfn live() {}",
    "test-share-multiple-items": "fn live() {}\n" + "\n".join(f"#[cfg(test)] fn oracle_{index}() {{}}" for index in range(20)),
    "test-share-small-first-marker": "enum Choice { Live, #[cfg(test)] Oracle }\nfn live() {}\n#[cfg(test)] mod oracle { " + "fn nested() {} " * 50 + "}",
    "test-share-generic-arm": "fn choose(value: u8) -> u8 { match value { #[cfg(test)] 0 => helper::<" + ",".join(f"{{{index}}}" for index in range(30)) + ">(), _ => 0 } }",
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence", type=Path, required=True)
    args = parser.parse_args()
    args.evidence.mkdir(parents=True, exist_ok=False)
    records = []
    with tempfile.TemporaryDirectory(prefix="qa-rust-rule-fixture-") as temp:
        root = Path(temp)
        for source in [*ROOT.glob("crates/*/**/*.rs"), ROOT / "tools/build.py", ROOT / "tools/check_rules.py", ROOT / "tools/rules-allowlist.json", ROOT / "tools/gen_cvars.py", ROOT / "tools/cvar_catalog.py", *ROOT.glob("data/unified-cvars.*"), ROOT / "data/unified-cvars-policy-issues.json"]:
            target = root / source.relative_to(ROOT)
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, target)
        fixture = root / "crates/world/src/violation.rs"
        fixture.write_text('// mirror seam donor in comments are not identifiers\nfn fine() { let _ = "Sha256 panic!"; }\nimpl Iterator for Query { type Item = EntityId; }')
        baseline = subprocess.run(["python3", str(root / "tools/build.py"), "--check-only"], capture_output=True, text=True)
        if baseline.returncode:
            raise RuntimeError("non-code words caused a false positive: " + baseline.stdout)
        for name, content in TEST_SHARE_ALLOWED.items():
            fixture.write_text(content)
            result = subprocess.run(["python3", str(root / "tools/build.py"), "--check-only"], capture_output=True, text=True)
            passed = result.returncode == 0
            records.append({"rule": name, "allowed_before_cargo": passed})
            (args.evidence / (name + ".log")).write_text(result.stdout + result.stderr)
            if not passed:
                raise RuntimeError("bounded test item caused a false positive: " + name)
        for name, content in TEST_SHARE_REJECTED.items():
            fixture.write_text(content)
            result = subprocess.run(["python3", str(root / "tools/build.py"), "--check-only"], capture_output=True, text=True)
            output = json.loads(result.stdout)
            passed = result.returncode != 0 and any(v["rule"] == "test-share" for v in output["findings"])
            records.append({"rule": name, "rejected_before_cargo": passed})
            (args.evidence / (name + ".log")).write_text(result.stdout + result.stderr)
            if not passed:
                raise RuntimeError("embedded test share was not enforced: " + name)
        for rule, content in CASES.items():
            fixture.write_text(content)
            result = subprocess.run(["python3", str(root / "tools/build.py"), "--check-only"], capture_output=True, text=True)
            output = json.loads(result.stdout)
            passed = result.returncode != 0 and any(v["rule"] == rule for v in output["findings"])
            records.append({"rule": rule, "rejected_before_cargo": passed})
            (args.evidence / (rule + ".log")).write_text(result.stdout + result.stderr)
            if not passed:
                raise RuntimeError("rule was not enforced: " + rule)
        fixture.unlink()
        # Reject every source independently, including developer examples and
        # imports/foreign declarations, while permitting the platform boundary.
        for name, text, directory in [
            ("clock-instant", "fn bad() { Instant::now(); }", "world/src"),
            ("clock-system-time", "fn bad() { SystemTime::now(); }", "world/src"),
            ("clock-example", "fn bad() { std::time::Instant::now(); }", "world/examples"),
            ("clock-import-alias", "use std::time::Instant as Clock; fn bad() { Clock::now(); }", "world/src"),
            ("sdl-time", "fn bad() { SDL_GetTicks(); }", "world/src"),
            ("sdl-input-import", "use external::SDL_PollEvent as poll;", "world/src"),
            ("sdl-foreign", 'unsafe extern "C" { fn SDL_PollEvent(event: *mut u8) -> i32; }', "world/src"),
            ("clock-unix-epoch", "use std::time::UNIX_EPOCH as epoch;", "world/src"),
            ("clock-posix", "fn bad() { libc::clock_gettime(0, &mut value); }", "world/src"),
            ("thread-spawn", "fn bad() { std::thread::spawn(work); }", "world/src"),
            ("thread-sleep", "fn bad() { std::thread::sleep(duration); }", "world/src"),
            ("thread-import-alias", "use std::thread::{spawn as start}; fn bad() { start(work); }", "world/src"),
            ("thread-module-alias", "use std::thread as jobs; fn bad() { jobs::sleep(duration); }", "world/examples"),
            ("stdin-call", "fn bad() { std::io::stdin(); }", "world/src"),
            ("stdin-import-alias", "use std::io::stdin as read; fn bad() { read(); }", "world/src"),
            ("stdin-group-alias", "use std::io::{stdin as read};", "world/src"),
            ("sdl-crate-alias", "use sdl2 as video; fn bad() { video::init(); }", "world/src"),
            ("sdl3-crate-alias", "use sdl3 as video; fn bad() { video::init(); }", "world/src"),
            ("sdl3-clock-example", "fn bad() { SDL_GetTicksNS(); }", "world/examples"),
            ("sdl-foreign-alias", '#[link_name = "SDL_PollEvent"] unsafe extern "C" fn poll();', "world/src"),
            ("sdl-library", '#[link(name = "SDL2")] unsafe extern "C" { fn poll(); }', "world/src"),
        ]:
            violation = root / "crates" / directory / "source_violation.rs"
            violation.parent.mkdir(parents=True, exist_ok=True)
            violation.write_text(text)
            result = subprocess.run(["python3", str(root / "tools/build.py"), "--check-only"], capture_output=True, text=True)
            output = json.loads(result.stdout)
            passed = result.returncode != 0 and any(v["rule"] == "platform-event-source" for v in output["findings"])
            records.append({"rule": name, "rejected_before_cargo": passed})
            (args.evidence / (name + ".log")).write_text(result.stdout + result.stderr)
            if not passed:
                raise RuntimeError("source boundary was not enforced: " + name)
            violation.unlink()
        for name, text in [
            ("output-deque", "struct Pending { events: VecDeque<FrameEvent> }"),
            ("packet-ring", "struct PacketRing { slots: [u8; 16] }"),
            ("output-renamed-queue", "struct PendingQueue { slots: [Option<EffectEvent>; 16] }"),
            ("output-type-alias", "type Prints = VecDeque<PrintEvent>;"),
            ("sound-vector", "struct Pending { values: Vec<SoundEvent> }"),
            ("effect-array", "struct Pending { values: [Option<EffectEvent>; 16] }"),
            ("print-boxed-array", "struct Pending { values: Box<[PrintEvent; 16]> }"),
            ("renamed-payload", "use qa_core::primitives::SoundEvent as S; struct Pending { values: Vec<S> }"),
        ]:
            fixture.write_text(text)
            result = subprocess.run(["python3", str(root / "tools/build.py"), "--check-only"], capture_output=True, text=True)
            output = json.loads(result.stdout)
            passed = result.returncode != 0 and any(v["rule"] == "duplicate-event-storage" for v in output["findings"])
            records.append({"rule": name, "rejected_before_cargo": passed})
            (args.evidence / (name + ".log")).write_text(result.stdout + result.stderr)
            if not passed:
                raise RuntimeError("duplicate storage was admitted: " + name)
            fixture.unlink()
        # Source ownership permits these declarations only at the platform
        # boundary. Core alone may define the shared event storage.
        platform_fixture = root / "crates/platform/src/source_boundary.rs"
        platform_fixture.write_text('use std::thread::{spawn, sleep}; use std::io::stdin; use std::time::UNIX_EPOCH; use sdl2 as video; #[link_name = "SDL_PollEvent"] unsafe extern "C" fn poll();')
        core_fixture = root / "crates/core/src/storage_boundary.rs"
        core_fixture.write_text("struct SoundRing { slots: [Option<SoundEvent>; 16] }")
        result = subprocess.run(["python3", str(root / "tools/build.py"), "--check-only"], capture_output=True, text=True)
        if result.returncode:
            raise RuntimeError("permitted ownership boundaries were rejected: " + result.stdout)
        (args.evidence / "permitted-boundaries.log").write_text(result.stdout + result.stderr)
        platform_fixture.unlink()
        core_fixture.unlink()
        generated = root / "crates/console/src/cvars_generated.rs"
        generated.write_text(generated.read_text() + "\n// stale catalog fixture\n")
        result = subprocess.run(["python3", str(root / "tools/build.py"), "--check-only"], capture_output=True, text=True)
        passed = result.returncode != 0 and "stale generated file:" in result.stderr
        records.append({"rule": "stale-cvar-catalog", "rejected_before_cargo": passed})
        (args.evidence / "stale-cvar-catalog.log").write_text(result.stdout + result.stderr)
        if not passed:
            raise RuntimeError("stale generated cvars were admitted")
        result = subprocess.run(["python3", str(ROOT / "tools/build.py"), "--check-only"], capture_output=True, text=True)
        if result.returncode:
            raise RuntimeError("main does not pass: " + result.stdout)
        (args.evidence / "main.log").write_text(result.stdout + result.stderr)
    report = {"result": "PASS", "cases": records, "main_passes": True}
    (args.evidence / "result.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report))


if __name__ == "__main__":
    main()
