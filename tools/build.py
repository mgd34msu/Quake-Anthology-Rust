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
    args = parser.parse_args()
    env = os.environ.copy()
    env["CARGO_TARGET_DIR"] = str(ROOT / "target")
    env["RUSTFLAGS"] = "" if args.cpu == "baseline" else "-C target-cpu=native"
    started = time.monotonic()
    subprocess.run(["cargo", "build", "--release", "--workspace"], cwd=ROOT, env=env, check=True)
    candidate = ROOT / "target/candidate"
    candidate.mkdir(exist_ok=True)
    binary = candidate / "qa-rust"
    binary.write_bytes((ROOT / "target/release/qa-rust").read_bytes())
    binary.chmod(0o755)
    symbols = candidate / "qa-rust.debug"
    subprocess.run(["objcopy", "--only-keep-debug", str(binary), str(symbols)], check=True)
    subprocess.run(["strip", "--strip-debug", str(binary)], check=True)
    subprocess.run(["objcopy", "--add-gnu-debuglink=" + symbols.name, binary.name], cwd=candidate, check=True)
    record = {"commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
              "build_time_seconds": time.monotonic() - started, "target_cpu": args.cpu,
              "built_at_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
              "candidate": str(binary), "debug_symbols": str(symbols), "gameplay": False}
    (candidate / "build.json").write_text(json.dumps(record, indent=2) + "\n")
    print(json.dumps(record))


if __name__ == "__main__":
    main()
