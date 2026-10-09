#!/usr/bin/env python3
"""Build a release candidate and split debug symbols before shipping."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cpu", choices=("baseline", "native"), default="baseline")
    parser.add_argument("--check-only", action="store_true")
    parser.add_argument("--proof", action="store_true", help="separate development candidate with scripted SDL input")
    parser.add_argument("--allocation-tracking", action="store_true", help="instrument the normal candidate and platform workers")
    args = parser.parse_args()
    subprocess.run(["python3", str(ROOT / "tools/check_rules.py"), "--root", str(ROOT)], check=True)
    subprocess.run(["python3", str(ROOT / "tools/gen_cvars.py"), "--root", str(ROOT), "--check"], check=True)
    if args.check_only:
        return
    env = os.environ.copy()
    env["CARGO_TARGET_DIR"] = str(ROOT / "target")
    env["RUSTFLAGS"] = "" if args.cpu == "baseline" else "-C target-cpu=native"
    commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    dirty = bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT, text=True))
    env["QA_BUILD_COMMIT"] = commit
    env["QA_BUILD_DIRTY"] = str(dirty).lower()
    env["QA_TARGET_CPU"] = args.cpu
    started = time.monotonic()
    command = ["cargo", "build", "--release", "--workspace"]
    if args.proof:
        command += ["--features", "qa-app/proof"]
    elif args.allocation_tracking:
        command += ["--features", "qa-app/allocation-tracking"]
    subprocess.run(command, cwd=ROOT, env=env, check=True)
    candidate = ROOT / ("target/proof-candidate" if args.proof else "target/candidate")
    candidate.mkdir(exist_ok=True)
    binary = candidate / "qa-rust"
    binary.write_bytes((ROOT / "target/release/qa-rust").read_bytes())
    binary.chmod(0o755)
    symbols = candidate / "qa-rust.debug"
    subprocess.run(["objcopy", "--only-keep-debug", str(binary), str(symbols)], check=True)
    subprocess.run(["strip", "--strip-debug", str(binary)], check=True)
    subprocess.run(["objcopy", "--add-gnu-debuglink=" + symbols.name, binary.name], cwd=candidate, check=True)
    record = {"commit": commit, "source_tree_dirty": dirty, "proof": args.proof,
              "allocation_tracking": args.allocation_tracking or args.proof,
              "build_time_seconds": time.monotonic() - started, "target_cpu": args.cpu,
              "built_at_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
              "candidate": str(binary), "debug_symbols": str(symbols)}
    (candidate / "build.json").write_text(json.dumps(record, indent=2) + "\n")
    print(json.dumps(record))


if __name__ == "__main__":
    main()
