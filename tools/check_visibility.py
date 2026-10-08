#!/usr/bin/env python3
"""Pin a headless THE-862 retail visibility check; no renderer or gameplay qualification."""
import argparse
import json
import os
from pathlib import Path
import shutil
import statistics
import subprocess

from frame_timings import pinned_cores
from private_run import equal_files, identity


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--q1-archive", type=Path, required=True)
    parser.add_argument("--q2-archive", type=Path, required=True)
    parser.add_argument("--q3-archive", type=Path, required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--core")
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    evidence = args.evidence.resolve()
    if "qfiles" in evidence.parts:
        raise ValueError("evidence must be outside qfiles")
    evidence.mkdir(exist_ok=False, parents=True, mode=0o700)
    candidate = evidence / "visibility"
    shutil.copy2(binary, candidate)
    if not equal_files(binary, candidate):
        raise ValueError("developer binary copy differs")
    original = identity(binary)
    core = args.core or pinned_cores()
    pairs = [
        (args.q1_archive.resolve(strict=True), "maps/e1m1.bsp"),
        (args.q2_archive.resolve(strict=True), "maps/base1.bsp"),
        (args.q3_archive.resolve(strict=True), "maps/q3dm1.bsp"),
    ]
    archive_metadata = {str(path): identity(path) for path, _ in pairs}
    argv = ["timeout", "300", "taskset", "-c", core, str(candidate)]
    for archive, name in pairs:
        argv.extend(["--archive", str(archive), "--map", name])
    output = evidence / "visibility.json"
    argv.extend(["--output", str(output)])
    # No window or SDL subsystem is initialized. Remove desktop endpoints and
    # select dummy drivers so an accidental initialization cannot reach them.
    env = {
        "PATH": os.environ["PATH"], "LANG": "C.UTF-8",
        "HOME": str(evidence), "XDG_CONFIG_HOME": str(evidence / "config"),
        "XDG_DATA_HOME": str(evidence / "data"), "XDG_CACHE_HOME": str(evidence / "cache"),
        "SDL_VIDEODRIVER": "dummy", "SDL_VIDEO_DRIVER": "dummy",
        "SDL_AUDIODRIVER": "dummy", "SDL_AUDIO_DRIVER": "dummy",
    }
    with (evidence / "runtime.log").open("w") as log:
        process = subprocess.run(argv, cwd=evidence, env=env, stdout=log, stderr=subprocess.STDOUT, timeout=305)
    report = json.loads(output.read_text()) if output.exists() else {}
    samples = report.get("samples_ns", [])
    maps = report.get("maps", [])
    checks = {
        "normal_exit": process.returncode == 0,
        "600_measured_frames": report.get("frames") == 600 and report.get("warmup") == 60 and len(samples) == 600,
        "pinned_batch_workload": report.get("queries_per_map_per_frame") == 32 and report.get("measured_queries") == 57600,
        "zero_rust_allocations": all(report.get(field) == 0 for field in
            ("allocations", "requested_bytes", "maximum_allocations", "maximum_requested_bytes")),
        "timed_state_matches": report.get("state_match") is True,
        "all_three_retail_maps": len(maps) == 3
            and {(map.get("family"), map.get("map")) for map in maps} ==
                {(1, "maps/e1m1.bsp"), (2, "maps/base1.bsp"), (3, "maps/q3dm1.bsp")},
        "raw_pvs_bits_match": len(maps) == 3 and all(
            map.get("pvs_rows_matched") is True and map.get("pvs_rows", 0) > 0
            and map.get("pvs_bits_compared") == map["pvs_rows"] * map["row_bytes"] * 8
            and map.get("missing_pvs_all_visible") is True for map in maps),
        "leaf_and_face_membership_match": len(maps) == 3 and all(
            map.get("leaf_selectors_matched") is True and map.get("point_leaves_matched") is True
            and map.get("face_membership_matched") is True and map.get("seeded_points") == 10000
            and map.get("spawn_origins", 0) > 0
            and map.get("point_cases") == map["seeded_points"] + map["spawn_origins"]
            and map.get("secondary_row_cases", 0) > 0
            and map.get("membership_queries") == map["point_cases"] * 2
            and map.get("membership_surface_checks") == map["membership_queries"] * map["surfaces"]
            and map.get("measured_queries") == 19200 for map in maps),
        "binary_preserved": identity(binary) == original and equal_files(binary, candidate),
        "archive_metadata_preserved": all(identity(path) == archive_metadata[str(path)] for path, _ in pairs),
        "headless_scope": report.get("window_opened") is False and report.get("gameplay_qualified") is False
            and "DISPLAY" not in env and "WAYLAND_DISPLAY" not in env,
    }
    if len(samples) == 600:
        ordered = sorted(samples)
        checks["timing_summary_matches"] = report.get("median_ns") == statistics.median(samples) \
            and report.get("p99_ns") == ordered[593] and all(sample > 0 for sample in samples)
    else:
        checks["timing_summary_matches"] = False
    verification = {
        "result": "PASS" if all(checks.values()) else "FAIL",
        "checks": checks, "core": core, "argv": argv,
        "returncode": process.returncode, "median_ns": report.get("median_ns"),
        "p99_ns": report.get("p99_ns"), "maps": maps,
        "gameplay_qualified": False,
        "scope": "retail BSP load/PVS/leaf/face queries only; no render or install qualification",
        "evidence": str(output),
        "limits": "Archives were opened read-only; metadata preservation is checked without hashes. Native reference is Rust over parsed BSP data, not an extracted C executable.",
    }
    (evidence / "verification.json").write_text(json.dumps(verification, indent=2) + "\n")
    print(json.dumps(verification))
    return verification["result"] != "PASS"


if __name__ == "__main__":
    raise SystemExit(main())
