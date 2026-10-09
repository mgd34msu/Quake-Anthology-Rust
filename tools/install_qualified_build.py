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


def qualify(build, profile, evidence, arguments, require_gameplay=True):
    binary = build / "qa-rust"
    metadata = json.loads((build / "build.json").read_text())
    require(metadata.get("source_tree_dirty") is False, "build must come from a committed clean tree")
    require(metadata.get("proof") is False, "development proof candidates cannot be installed")
    compiled = json.loads(subprocess.check_output([str(binary), "--build-info"], text=True))
    require(all(compiled.get(k) == metadata.get(k) for k in ("commit", "source_tree_dirty", "target_cpu", "proof")),
            "build metadata differs from the compiled candidate")
    result = run(binary, profile, evidence, arguments)
    require(result["result"] == "PASS" and result.get("normal_exit"), "copied-profile private launch failed")
    require(result["copied_owner_settings"], "owner profile must contain saved settings")
    require(result["owner_profile_unchanged"], "owner profile changed during qualification")
    require(result["candidate_unchanged"], "candidate changed during qualification")
    require(not result["remaining_owned_pids"], "owned processes remain")
    if require_gameplay:
        require(result["gameplay_reached"], "candidate never reached gameplay; a window-only run does not qualify")
    return metadata, result


PREVIEW_MAPS = (("q1/id1", "e1m1"), ("q2/baseq2", "base1"), ("q3a/baseq3", "q3dm1"))


def qualify_preview(build, profile, evidence, content):
    require(not evidence.exists(), "use a fresh evidence directory")
    evidence.mkdir(parents=True)
    results = []
    metadata = None
    for product, name in PREVIEW_MAPS:
        for renderer in ("gl", "cpu"):
            folder = evidence / (name + "-" + renderer)
            current, result = qualify(build, profile, folder,
                ["--content", str(content / product), "--map", name, "--renderer", renderer,
                 "--frames", "300", "--width", "640", "--height", "400",
                 "--startup-hold-ms", "1500", "--uncapped"], require_gameplay=False)
            frame = next((row for row in result["events"] if row.get("event") == "world_frame_presented"), {})
            normal = next((row for row in result["events"] if row.get("event") == "normal_exit"), {})
            require(frame.get("map") == "maps/" + name + ".bsp" and frame.get("renderer") == renderer,
                    "preview did not render the requested map/backend")
            require(frame.get("client_connected") and frame.get("views", 0) > 0
                    and frame.get("surfaces", 0) > 0 and frame.get("rejected") == 0,
                    "preview did not present a connected world view")
            require(frame.get("profile_consumed") and frame.get("native_input_policy"),
                    "preview did not consume copied saved settings")
            require(normal.get("frames") == 300, "preview did not complete 300 frames")
            require(metadata is None or current == metadata, "candidate metadata changed between runs")
            require(not results or result["candidate_identity"] == results[0]["candidate_identity"],
                    "candidate changed between preview runs")
            metadata = current
            results.append(result)
    summary = {"result": "PASS", "scope": "render_preview", "gameplay_qualified": False,
               "candidate_identity": results[0]["candidate_identity"],
               "runs": [str(evidence / (name + "-" + renderer) / "result.json")
                        for _, name in PREVIEW_MAPS for renderer in ("gl", "cpu")]}
    (evidence / "result.json").write_text(json.dumps(summary, indent=2) + "\n")
    return metadata, summary


def preview_text(metadata):
    return f"""Quake Anthology Rust: limited render preview
Commit: {metadata['commit']}
Build time: {metadata['build_time_seconds']:.2f} seconds
CPU target: {metadata['target_cpu']}; normal build, proof input disabled.

Verified: e1m1, base1 and q3dm1 render on GL and CPU at 640x400,
with copied saved settings, 300 frames and normal quit on each backend.
GL evidence uses Mesa llvmpipe software rendering, not hardware timing proof.

Not implemented/qualified yet: gameplay modules, monsters, weapons, native HUD,
game audio, legacy network play, saves/load, and inline-model collision.
This is not full gameplay qualification or complete movement/visual parity.
The original qa-rust binary and saved profile are preserved.
No comparable gameplay timing qualification is claimed for this preview.

Run from the qfiles directory (close the window to quit, or use a frame limit):
./qa-rust-preview --content q1/id1 --map e1m1 --renderer gl --frames 100000
./qa-rust-preview --content q2/baseq2 --map base1 --renderer cpu --frames 100000
./qa-rust-preview --content q3a/baseq3 --map q3dm1 --renderer gl --frames 100000
Use --renderer cpu or --renderer gl with any of the three examples.
For a short automatic-quit check, use --frames 300.
Later preview installs replace only qa-rust-preview and this file.
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


def install(build, destination, profile, evidence, arguments, timings=None, baseline=None, preview_content=None):
    require(destination.name in ("qa-rust", "qa-rust-preview"), "destination must name qa-rust or qa-rust-preview")
    preview = destination.name == "qa-rust-preview"
    original_profile = {str(p.relative_to(profile)): p.read_bytes() for p in settings(profile)}
    if preview:
        require(not arguments, "preview qualification supplies all six map/backend launches")
        require(timings is None and baseline is None, "gameplay timing reports do not qualify a render preview")
        metadata, qualification = qualify_preview(build, profile, evidence, preview_content or destination.parent)
        performance = {"result": "NOT_QUALIFIED", "scope": "render_preview",
                       "reason": "no comparable measured gameplay workload; preview destination only"}
    else:
        require(preview_content is None, "preview content is only valid for qa-rust-preview")
        metadata, qualification = qualify(build, profile, evidence, arguments)
        require(timings is not None and baseline is not None, "measured gameplay timings and a comparable baseline are required")
        performance = compare(json.loads(timings.read_text()), json.loads(baseline.read_text()),
                              metadata, qualification["candidate_identity"])
    binary = build / "qa-rust"
    destination.parent.mkdir(parents=True, exist_ok=True)
    staged = stage(binary, destination, qualification["candidate_identity"])
    staged_notes = None
    try:
        if preview:
            staged_notes = stage_text(preview_text(metadata), destination.with_suffix(".txt"))
        require(identity(binary) == qualification["candidate_identity"], "qualified candidate changed before install")
        require(original_profile == {str(p.relative_to(profile)): p.read_bytes() for p in settings(profile)}, "profile changed before install")
        os.replace(staged, destination)
        if staged_notes is not None:
            os.replace(staged_notes, destination.with_suffix(".txt"))
        require(equal_files(binary, destination), "installed bytes differ")
        receipt = {"result": "PASS", "commit": metadata["commit"], "build": metadata,
                   "qualification_scope": "render_preview" if preview else "gameplay",
                   "gameplay_qualified": not preview,
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
    parser.add_argument("--preview-content-root", type=Path, help="preview retail root; defaults to destination directory")
    parser.add_argument("--timings", type=Path, help="measured gameplay timing report for this exact candidate")
    parser.add_argument("--baseline", type=Path, help="comparable measured gameplay report")
    parser.add_argument("arguments", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    arguments = args.arguments[1:] if args.arguments[:1] == ["--"] else args.arguments
    try:
        receipt = install(args.build_dir.resolve(strict=True), args.destination.absolute(),
                          args.owner_profile.resolve(strict=True), args.evidence.resolve(), arguments,
                          args.timings, args.baseline, args.preview_content_root)
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as error:
        print("Install refused: " + str(error), file=os.sys.stderr)
        return 1
    print(json.dumps(receipt, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
