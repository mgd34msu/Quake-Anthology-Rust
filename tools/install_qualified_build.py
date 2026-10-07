#!/usr/bin/env python3
"""Qualify the exact Rust candidate with copied owner settings before installation."""
import argparse
import json
import os
from pathlib import Path
import stat
import subprocess
import tempfile

from private_run import equal_files, identity, run, settings


def require(condition, message):
    if not condition:
        raise ValueError(message)


def qualify(build, profile, evidence, arguments):
    binary = build / "qa-rust"
    metadata = json.loads((build / "build.json").read_text())
    require(metadata.get("source_tree_dirty") is False, "build must come from a committed clean tree")
    compiled = json.loads(subprocess.check_output([str(binary), "--build-info"], text=True))
    require(all(compiled.get(k) == metadata.get(k) for k in ("commit", "source_tree_dirty", "target_cpu")),
            "build metadata differs from the compiled candidate")
    result = run(binary, profile, evidence, arguments)
    require(result["result"] == "PASS" and result.get("normal_exit"), "copied-profile private launch failed")
    require(result["copied_owner_settings"], "owner profile must contain saved settings")
    require(result["owner_profile_unchanged"], "owner profile changed during qualification")
    require(result["candidate_unchanged"], "candidate changed during qualification")
    require(not result["remaining_owned_pids"], "owned processes remain")
    require(result["gameplay_reached"], "candidate never reached gameplay; a window-only run does not qualify")
    return metadata, result


def stage(source, target, expected):
    descriptor, name = tempfile.mkstemp(prefix=target.name + ".", suffix=".tmp", dir=target.parent)
    temporary = Path(name)
    try:
        with os.fdopen(descriptor, "wb") as output, source.open("rb") as original:
            info = os.fstat(original.fileno())
            require(identity(source) == expected, "qualified candidate changed before copying")
            while chunk := original.read(1024 * 1024):
                output.write(chunk)
            output.flush()
            os.fchmod(output.fileno(), stat.S_IMODE(info.st_mode))
            os.fsync(output.fileno())
        require(equal_files(source, temporary) and identity(source) == expected, "candidate changed during copying")
        return temporary
    except BaseException:
        temporary.unlink(missing_ok=True)
        raise


def install(build, destination, profile, evidence, arguments):
    require(destination.name == "qa-rust", "destination must name qa-rust")
    original_profile = {str(p.relative_to(profile)): p.read_bytes() for p in settings(profile)}
    metadata, qualification = qualify(build, profile, evidence, arguments)
    binary = build / "qa-rust"
    destination.parent.mkdir(parents=True, exist_ok=True)
    staged = stage(binary, destination, qualification["candidate_identity"])
    try:
        require(identity(binary) == qualification["candidate_identity"], "qualified candidate changed before install")
        require(original_profile == {str(p.relative_to(profile)): p.read_bytes() for p in settings(profile)}, "profile changed before install")
        os.replace(staged, destination)
        require(equal_files(binary, destination), "installed bytes differ")
        receipt = {"result": "PASS", "commit": metadata["commit"], "build": metadata,
                   "build_time_seconds": metadata["build_time_seconds"], "installed_at_utc": time_utc(),
                   "qualification_evidence": str(evidence / "result.json"),
                   "destination": str(destination), "byte_equal": True, "installed_identity": identity(destination)}
        (evidence / "install-receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
        return receipt
    finally:
        staged.unlink(missing_ok=True)


def time_utc():
    import time
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--build-dir", type=Path, required=True)
    parser.add_argument("--destination", type=Path, required=True)
    parser.add_argument("--owner-profile", type=Path, required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("arguments", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    arguments = args.arguments[1:] if args.arguments[:1] == ["--"] else args.arguments
    try:
        receipt = install(args.build_dir.resolve(strict=True), args.destination.absolute(),
                          args.owner_profile.resolve(strict=True), args.evidence.resolve(), arguments)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print("Install refused: " + str(error), file=os.sys.stderr)
        return 1
    print(json.dumps(receipt, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
