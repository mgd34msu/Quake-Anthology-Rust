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
        # Every public core primitive has one owner, including examples and
        # aliases. Epoch-mark copies are rejected even after renaming them.
        for name, content, directory in [
            ("stamp-struct", "struct StampSet { marks: Vec<u32> }", "world/src"),
            ("stamp-type-alias", "type StampSet = Renamed;", "render/src"),
            ("stamp-example", "struct StampSet {}", "world/examples"),
            ("name-primitive", "struct NameTable {}", "console/src"),
            ("text-primitive", "struct FixedText {}", "ui/tests"),
            ("geometry-handle", "struct GeometryId { slot:u32, generation:u32 }", "render/src"),
            ("model-pose-policy", "struct ModelRules {}", "app/src"),
            ("collision-store-copy", "struct CollisionStore {}", "session/src"),
            ("trace-scratch-copy", "struct TraceScratch {}", "movement/examples"),
            ("rule-identity-copy", "enum RuleSetId { Quake, QuakeWorld, Quake2, Quake2Rerelease, Quake3 }", "session/src"),
            ("epoch-mark-array", "impl Marks { fn begin(&mut self) { self.generation = self.generation.wrapping_add(1); if self.generation == 0 { self.values.fill(0); self.generation = 1; } } }", "world/src"),
            ("epoch-mark-batches", "impl Batches { fn begin(&mut self) { self.count = self.count.wrapping_add(1); if self.count == 0 { self.count = 1; for row in &mut self.rows { row.seen = 0; row.drawn = 0; } } } }", "render/src"),
            ("epoch-mark-reference", "fn renew(rows: &mut [u64], epoch: &mut u64) { *epoch = epoch.wrapping_add(1); if *epoch == 0 { rows.fill(0); *epoch = 1; } }", "world/examples"),
        ]:
            violation = root / "crates" / directory / "primitive_violation.rs"
            violation.parent.mkdir(parents=True, exist_ok=True)
            violation.write_text(content)
            result = subprocess.run(["python3", str(root / "tools/build.py"), "--check-only"], capture_output=True, text=True)
            output = json.loads(result.stdout)
            passed = result.returncode != 0 and any(v["rule"] == "duplicate-primitive" for v in output["findings"])
            records.append({"rule": name, "rejected_before_cargo": passed})
            (args.evidence / (name + ".log")).write_text(result.stdout + result.stderr)
            if not passed:
                raise RuntimeError("duplicate primitive was admitted: " + name)
            violation.unlink()
        for name, content in [
            ("map-derived-world-rate", "fn run() { let world_rate = match loaded.native_source { Quake2 => TickRate::fixed(100), _ => TickRate::FrameDriven }; }"),
            ("map-derived-link-order", "fn run() { let order = if loaded.native_source == Quake3 { LinkOrder::Head } else { LinkOrder::Tail }; }"),
            ("map-derived-player-preset", "fn run() { let rules = movement_rules.unwrap_or_else(|| input.native_source); }"),
        ]:
            violation = root / "crates/app/src/main.rs"
            original = violation.read_text()
            violation.write_text(content)
            try:
                result = subprocess.run(["python3", str(root / "tools/build.py"), "--check-only"], capture_output=True, text=True)
                output = json.loads(result.stdout)
                passed = result.returncode != 0 and any(v["rule"] == "client-policy-selection" for v in output["findings"])
                records.append({"rule": name, "rejected_before_cargo": passed})
                (args.evidence / (name + ".log")).write_text(result.stdout + result.stderr)
                if not passed:
                    raise RuntimeError("map-derived client policy was admitted: " + name)
            finally:
                violation.write_text(original)
        for name, content in [
            ("movement-derived-trace-role", "impl Parameters { fn load(rules: RuleSetId, player: &PlayerState) -> Self { let (trace_rules, entity_rules) = trace_policy(rules); Self { trace_rules, entity_rules } } }"),
            ("movement-field-trace-role", "impl Parameters { fn load(rules: RuleSetId, player: &PlayerState) -> Self { let (trace_rules, entity_rules) = trace_policy(player.movement_rules); Self { trace_rules, entity_rules } } }"),
            ("inline-movement-trace-selection", "impl Parameters { fn load(rules: RuleSetId, player: &PlayerState) -> Self { let trace_rules = if rules == RuleSetId::Quake3 { TraceRules::ARENA } else { TraceRules::LEGACY }; Self { trace_rules } } }"),
        ]:
            violation = root / "crates/movement/src/physics.rs"
            original = violation.read_text()
            violation.write_text(content)
            try:
                result = subprocess.run(["python3", str(root / "tools/build.py"), "--check-only"], capture_output=True, text=True)
                output = json.loads(result.stdout)
                passed = result.returncode != 0 and any(v["rule"] == "trace-role-selection" for v in output["findings"])
                records.append({"rule": name, "rejected_before_cargo": passed})
                (args.evidence / (name + ".log")).write_text(result.stdout + result.stderr)
                if not passed:
                    raise RuntimeError("movement-derived trace policy was admitted: " + name)
            finally:
                violation.write_text(original)
        for name, content, relative in [
            ("retired-movement-id", "enum MovementRules { Quake, Quake3 }", "crates/movement/src/retired_rules.rs"),
            ("retired-think-id", "enum ThinkTiming { Quake, Quake3 }", "crates/session/src/retired_rules.rs"),
            ("retired-movement-alias", "type MovementRules = RuleSetId;", "crates/movement/examples/retired_rules.rs"),
            ("retired-movement-reexport", "pub use qa_core::primitives::{EntityId as Kept, RuleSetId as MovementRules};", "crates/movement/src/retired_rules.rs"),
            ("retired-think-reexport", "pub use qa_core::primitives::RuleSetId as ThinkTiming;", "crates/session/src/retired_rules.rs"),
            ("retired-console-reexport", "pub use qa_core::primitives::RuleSetId as Source;", "crates/console/src/views.rs"),
            ("renamed-rule-identity", "enum Dialect { Quake, QuakeWorld, Quake2, Quake2Rerelease, Quake3 }", "crates/ui/src/retired_rules.rs"),
            ("retired-console-source", "enum Source { Quake, QuakeWorld, Quake2, Quake2Rerelease, Quake3 }", "crates/console/src/views.rs"),
        ]:
            violation = root / relative
            violation.parent.mkdir(parents=True, exist_ok=True)
            original = violation.read_text() if violation.exists() else None
            violation.write_text(content)
            try:
                result = subprocess.run(["python3", str(root / "tools/build.py"), "--check-only"], capture_output=True, text=True)
                output = json.loads(result.stdout)
                passed = result.returncode != 0 and any(v["rule"] == "rule-identity" for v in output["findings"])
                records.append({"rule": name, "rejected_before_cargo": passed})
                (args.evidence / (name + ".log")).write_text(result.stdout + result.stderr)
                if not passed:
                    raise RuntimeError("retired or copied rule identity was admitted: " + name)
            finally:
                if original is None:
                    violation.unlink()
                else:
                    violation.write_text(original)
        for name, content, relative in [
            ("retired-collision-world", "enum CollisionWorld { Hulls, Brushes }", "crates/world/src/retired_collision.rs"),
            ("variant-trace-scratch", "enum TraceScratch { Hulls, Brushes }", "crates/world/src/collision/store.rs"),
            ("hull-model-registry", "struct Q1Hulls { roots: Box<[HullModel]> }", "crates/world/src/collision/hulls.rs"),
            ("brush-model-registry", "struct Topology { models: Vec<ModelRoot> }", "crates/world/src/collision/tree.rs"),
        ]:
            violation = root / relative
            original = violation.read_text() if violation.exists() else None
            violation.write_text(content)
            try:
                result = subprocess.run(["python3", str(root / "tools/build.py"), "--check-only"], capture_output=True, text=True)
                output = json.loads(result.stdout)
                passed = result.returncode != 0 and any(v["rule"] == "collision-model-storage" for v in output["findings"])
                records.append({"rule": name, "rejected_before_cargo": passed})
                (args.evidence / (name + ".log")).write_text(result.stdout + result.stderr)
                if not passed:
                    raise RuntimeError("retired collision storage was admitted: " + name)
            finally:
                if original is None:
                    violation.unlink()
                else:
                    violation.write_text(original)
        for name, content in [
            ("owned-primitive-import", "use qa_core::stamps::StampSet; fn marks() -> StampSet { StampSet::new(16) }"),
            ("lifetime-generation", "fn retire(value: &mut u32) { *value += 1; }"),
            ("protocol-counter", "fn advance(value: &mut u32) { *value = value.wrapping_add(1); if *value == 0 { *value = 1; } }"),
            ("primitive-associated-type", "impl Iterator for Live { type Item = EntityId; }"),
        ]:
            fixture.write_text(content)
            result = subprocess.run(["python3", str(root / "tools/build.py"), "--check-only"], capture_output=True, text=True)
            passed = result.returncode == 0
            records.append({"rule": name, "allowed_before_cargo": passed})
            (args.evidence / (name + ".log")).write_text(result.stdout + result.stderr)
            if not passed:
                raise RuntimeError("permitted primitive use was rejected: " + name)
        fixture.unlink()
        core_violation = root / "crates/core/src/unsafe_violation.rs"
        for name, content in [
            ("unsafe-core-block", "fn bad() { unsafe { operation(); } }"),
            ("unsafe-core-function", "unsafe fn bad() {}"),
        ]:
            core_violation.write_text(content)
            result = subprocess.run(["python3", str(root / "tools/build.py"), "--check-only"], capture_output=True, text=True)
            output = json.loads(result.stdout)
            passed = result.returncode != 0 and any(v["rule"] == "unsafe-core" for v in output["findings"])
            records.append({"rule": name, "rejected_before_cargo": passed})
            (args.evidence / (name + ".log")).write_text(result.stdout + result.stderr)
            if not passed:
                raise RuntimeError("unsafe core was admitted: " + name)
        core_violation.unlink()
        core_lib = root / "crates/core/src/lib.rs"
        original_core = core_lib.read_text()
        core_lib.write_text(original_core.replace("#![forbid(unsafe_code)]", ""))
        result = subprocess.run(["python3", str(root / "tools/build.py"), "--check-only"], capture_output=True, text=True)
        output = json.loads(result.stdout)
        passed = result.returncode != 0 and any(v["rule"] == "unsafe-core" for v in output["findings"])
        records.append({"rule": "unsafe-core-forbid-removed", "rejected_before_cargo": passed})
        (args.evidence / "unsafe-core-forbid-removed.log").write_text(result.stdout + result.stderr)
        core_lib.write_text(original_core)
        if not passed:
            raise RuntimeError("core unsafe ban could be removed")
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
