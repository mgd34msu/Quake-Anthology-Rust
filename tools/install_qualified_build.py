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
from timing_guard import compare


def require(condition, message):
    if not condition:
        raise ValueError(message)


def qualify(build, profile, evidence, arguments, require_gameplay=True, audio_driver="disk"):
    binary = build / "qa-rust"
    metadata = json.loads((build / "build.json").read_text())
    require(metadata.get("source_tree_dirty") is False, "build must come from a committed clean tree")
    require(metadata.get("proof") is False, "development proof candidates cannot be installed")
    compiled = json.loads(subprocess.check_output([str(binary), "--build-info"], text=True))
    require(all(compiled.get(k) == metadata.get(k) for k in ("commit", "source_tree_dirty", "target_cpu", "proof")),
            "build metadata differs from the compiled candidate")
    result = run(binary, profile, evidence, arguments, audio_driver=audio_driver)
    require(result["result"] == "PASS" and result.get("normal_exit"), "copied-profile private launch failed")
    require(result["copied_owner_settings"], "owner profile must contain saved settings")
    require(result["owner_profile_unchanged"], "owner profile changed during qualification")
    require(result["candidate_unchanged"], "candidate changed during qualification")
    require(not result["remaining_owned_pids"], "owned processes remain")
    if require_gameplay:
        require(result["gameplay_reached"], "candidate never reached gameplay; a window-only run does not qualify")
    return metadata, result


SMOKE_MAPS = (("q1/id1", "start"), ("q2/baseq2", "base1"), ("q3a/baseq3", "q3dm1"))


def qualify_smoke(build, profile, evidence, content):
    require(not evidence.exists(), "use a fresh evidence directory")
    evidence.mkdir(parents=True)
    results = []
    metadata = None
    for product, name in SMOKE_MAPS:
        for renderer in ("gl", "cpu"):
            folder = evidence / (name + "-" + renderer)
            current, result = qualify(build, profile, folder,
                ["--content", str(content / product), "--map", name, "--renderer", renderer,
                 "--frames", "300", "--width", "640", "--height", "400",
                 "--startup-hold-ms", "1500", "--uncapped"], require_gameplay=False, audio_driver="dummy")
            frame = next((row for row in result["events"] if row.get("event") == "world_frame_presented"), {})
            normal = next((row for row in result["events"] if row.get("event") == "normal_exit"), {})
            require(frame.get("map") == "maps/" + name + ".bsp" and frame.get("renderer") == renderer,
                    "smoke run did not render the requested map/backend")
            require(frame.get("client_connected") and frame.get("views", 0) > 0
                    and frame.get("surfaces", 0) > 0 and frame.get("rejected") == 0,
                    "smoke run did not present a connected world view")
            require(frame.get("profile_consumed") and frame.get("native_input_policy"),
                    "smoke run did not consume copied saved settings")
            require(normal.get("frames") == 300, "smoke run did not complete 300 frames")
            require(metadata is None or current == metadata, "candidate metadata changed between runs")
            require(not results or result["candidate_identity"] == results[0]["candidate_identity"],
                    "candidate changed between smoke runs")
            metadata = current
            results.append(result)
    summary = {"result": "PASS", "scope": "render_smoke", "gameplay_qualified": False,
               "candidate_identity": results[0]["candidate_identity"],
               "runs": [str(evidence / (name + "-" + renderer) / "result.json")
                        for _, name in SMOKE_MAPS for renderer in ("gl", "cpu")]}
    (evidence / "result.json").write_text(json.dumps(summary, indent=2) + "\n")
    return metadata, summary


def smoke_text(metadata):
    return f"""Quake Anthology Rust: current development build
Commit: {metadata['commit']}
Built at UTC: {metadata['built_at_utc']}
Build time: {metadata['build_time_seconds']:.2f} seconds
CPU target: {metadata['target_cpu']}; normal build, proof input disabled.

Verified: Q1 start, Q2 base1 and Q3 q3dm1 load and render on GL and CPU
at 640x400, with copied saved settings, 300 frames and exit 0 for each.
Runs use owned Xvfb, forced X11, dummy audio and private HOME.
GL driver identity is in the per-run logs; these runs do not qualify GL speed.

Known gaps: native hosts and live entity-memory integration, delta channel and
legacy network interoperability, stock HUD drawing. Gameplay module lifecycle,
monsters, weapons, game audio and save/load remain unfinished or unqualified.
QuakeC/QVM reader/interpreter component checks do not prove retail gameplay.
This owner-authorized smoke install is not gameplay or timing qualification.
The owner's original saved profile is preserved.

Run from the qfiles directory:
./qa-rust --content q1/id1 --map start --renderer gl --frames 100000
./qa-rust --content q2/baseq2 --map base1 --renderer cpu --frames 100000
./qa-rust --content q3a/baseq3 --map q3dm1 --renderer gl --frames 100000
Use --renderer cpu or --renderer gl with any example.
Close the window to quit, or use --frames 300 for automatic exit.
"""


def stage_text(text, target):
    descriptor, name = tempfile.mkstemp(prefix=target.name + ".", suffix=".tmp", dir=target.parent)
    temporary = Path(name)
    try:
        with os.fdopen(descriptor, "w") as output:
            output.write(text)
            output.flush()
            os.fchmod(output.fileno(), 0o644)
            os.fsync(output.fileno())
        return temporary
    except BaseException:
        temporary.unlink(missing_ok=True)
        raise


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


def install(build, destination, profile, evidence, arguments, timings=None, baseline=None, smoke_content=None, owner_smoke=False):
    require(destination.name == "qa-rust", "destination must name qa-rust")
    original_profile = {str(p.relative_to(profile)): p.read_bytes() for p in settings(profile)}
    if owner_smoke:
        require(not arguments, "smoke qualification supplies all six map/backend launches")
        require(timings is None and baseline is None, "gameplay timing reports do not qualify a smoke install")
        metadata, qualification = qualify_smoke(build, profile, evidence, smoke_content or destination.parent)
        performance = {"result": "NOT_QUALIFIED", "scope": "render_smoke",
                       "reason": "owner-authorized smoke install; no comparable measured gameplay workload"}
    else:
        require(smoke_content is None, "smoke content requires --owner-smoke")
        metadata, qualification = qualify(build, profile, evidence, arguments)
        require(timings is not None and baseline is not None, "measured gameplay timings and a comparable baseline are required")
        performance = compare(json.loads(timings.read_text()), json.loads(baseline.read_text()),
                              metadata, qualification["candidate_identity"])
    binary = build / "qa-rust"
    destination.parent.mkdir(parents=True, exist_ok=True)
    staged = stage(binary, destination, qualification["candidate_identity"])
    staged_notes = None
    try:
        if owner_smoke:
            staged_notes = stage_text(smoke_text(metadata), destination.with_suffix(".txt"))
        require(identity(binary) == qualification["candidate_identity"], "qualified candidate changed before install")
        require(original_profile == {str(p.relative_to(profile)): p.read_bytes() for p in settings(profile)}, "profile changed before install")
        os.replace(staged, destination)
        if staged_notes is not None:
            os.replace(staged_notes, destination.with_suffix(".txt"))
        require(equal_files(binary, destination), "installed bytes differ")
        receipt = {"result": "PASS", "commit": metadata["commit"], "build": metadata,
                   "qualification_scope": "render_smoke" if owner_smoke else "gameplay",
                   "gameplay_qualified": not owner_smoke,
                   "build_time_seconds": metadata["build_time_seconds"], "installed_at_utc": time_utc(),
                   "qualification_evidence": str(evidence / "result.json"),
                   "performance": performance, "timing_evidence": str(timings), "timing_baseline": str(baseline),
                   "destination": str(destination), "byte_equal": True, "installed_identity": identity(destination)}
        (evidence / "install-receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
        return receipt
    finally:
        staged.unlink(missing_ok=True)
        if staged_notes is not None:
            staged_notes.unlink(missing_ok=True)


def time_utc():
    import time
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--build-dir", type=Path, required=True)
    parser.add_argument("--destination", type=Path, required=True)
    parser.add_argument("--owner-profile", type=Path, required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--owner-smoke", action="store_true", help="owner-authorized development install after six private map/backend smoke runs; no gameplay/timing qualification")
    parser.add_argument("--smoke-content-root", type=Path, help="smoke retail root; defaults to destination directory")
    parser.add_argument("--timings", type=Path, help="measured gameplay timing report for this exact candidate")
    parser.add_argument("--baseline", type=Path, help="comparable measured gameplay report")
    parser.add_argument("arguments", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    arguments = args.arguments[1:] if args.arguments[:1] == ["--"] else args.arguments
    try:
        receipt = install(args.build_dir.resolve(strict=True), args.destination.absolute(),
                          args.owner_profile.resolve(strict=True), args.evidence.resolve(), arguments,
                          args.timings, args.baseline, args.smoke_content_root, args.owner_smoke)
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as error:
        print("Install refused: " + str(error), file=os.sys.stderr)
        return 1
    print(json.dumps(receipt, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
