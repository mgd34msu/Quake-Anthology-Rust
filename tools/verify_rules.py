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
    "diagnostics-path": 'fn bad() { eprintln!("event"); }',
    "temporary-diagnostics": "// TEMP-DIAG\nfn bad() {}",
    "test-share": "fn live() {}\n#[cfg(test)]\nmod tests { " + "fn scenario() {} " * 50 + "}",
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence", type=Path, required=True)
    args = parser.parse_args()
    args.evidence.mkdir(parents=True, exist_ok=False)
    records = []
    with tempfile.TemporaryDirectory(prefix="qa-rust-rule-fixture-") as temp:
        root = Path(temp)
        for source in [*ROOT.glob("crates/*/src/**/*.rs"), ROOT / "tools/build.py", ROOT / "tools/check_rules.py", ROOT / "tools/rules-allowlist.json"]:
            target = root / source.relative_to(ROOT)
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, target)
        fixture = root / "crates/world/src/violation.rs"
        fixture.write_text('// mirror seam donor in comments are not identifiers\nfn fine() { let _ = "Sha256 panic!"; }\nimpl Iterator for Query { type Item = EntityId; }')
        baseline = subprocess.run(["python3", str(root / "tools/build.py"), "--check-only"], capture_output=True, text=True)
        if baseline.returncode:
            raise RuntimeError("non-code words caused a false positive: " + baseline.stdout)
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
        result = subprocess.run(["python3", str(ROOT / "tools/build.py"), "--check-only"], capture_output=True, text=True)
        if result.returncode:
            raise RuntimeError("main does not pass: " + result.stdout)
        (args.evidence / "main.log").write_text(result.stdout + result.stderr)
    report = {"result": "PASS", "cases": records, "main_passes": True}
    (args.evidence / "result.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report))


if __name__ == "__main__":
    main()
