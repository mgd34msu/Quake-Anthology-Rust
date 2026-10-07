#!/usr/bin/env python3
"""Measure the current window shell; engine renderer qualification waits for R3/R4."""
import argparse
import json
import math
import os
from pathlib import Path
import platform
import statistics
import subprocess

from private_run import identity, run


def pinned_cores():
    available = os.sched_getaffinity(0)
    occupied = set()
    for proc in Path("/proc").iterdir():
        if not proc.name.isdigit():
            continue
        try:
            command = (proc / "cmdline").read_bytes().replace(b"\0", b" ").decode(errors="replace")
            if "quake-anthology" in command or "qfiles/qa-c" in command:
                affinity = os.sched_getaffinity(int(proc.name))
                if len(affinity) < len(available):
                    occupied.update(affinity)
        except (OSError, ValueError):
            pass
    siblings = set(occupied)
    for core in occupied:
        path = Path(f"/sys/devices/system/cpu/cpu{core}/topology/thread_siblings_list")
        if path.exists():
            for part in path.read_text().strip().split(","):
                limits = part.split("-")
                siblings.update(range(int(limits[0]), int(limits[-1]) + 1))
    free = sorted(available - siblings)
    if not free:
        raise ValueError("no core available outside the C agent's pinned runs")
    return str(free[-1])


def summarize(samples):
    values = sorted(samples)
    return {"median_ms": statistics.median(values) / 1e6,
            "p99_ms": values[max(0, math.ceil(len(values) * 0.99) - 1)] / 1e6}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--owner-profile", type=Path, required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    artifact_identity = identity(binary)
    build = json.loads(subprocess.check_output([str(binary), "--build-info"], text=True))
    cores = pinned_cores()
    rows = []
    for width, height in ((1920, 1080), (640, 400), (320, 200)):
        folder = args.evidence / f"{width}x{height}"
        result = run(args.binary, args.owner_profile, folder,
                     ["--frames", "600", "--warmup", "60", "--uncapped", "--frame-timings",
                      "--startup-hold-ms", "2500", "--width", str(width), "--height", str(height)],
                     size=(width, height), cores=cores)
        if result["result"] != "PASS":
            raise RuntimeError("private timing launch failed: " + str(result.get("error")))
        timing = next(v for v in result["events"] if v.get("event") == "frame_timings")
        samples = timing["samples_ns"]
        if len(samples) != 600 or timing["warmup"] != 60 or timing["vsync"]:
            raise ValueError("timing run did not complete the requested uncapped sample")
        stages = {stage: summarize([row[i] for row in samples])
                  for i, stage in enumerate(("input", "present", "total"))}
        rows.append({"resolution": f"{width}x{height}", "scope": timing["scope"],
                     "renderer": "SDL software window shell", "map": None,
                     "hardware_renderer_qualified": False, "warmup": 60, "frames": 600,
                     "debugger": False, "vsync": False, "workload": None,
                     "cores": cores, "stages": stages, "evidence": str(folder)})
    if identity(binary) != artifact_identity:
        raise ValueError("candidate changed during timing runs")
    output = {"scope": "window_shell", "measured": True,
              "qualification": "R0 tooling only; no gameplay renderer performance claim",
              "artifact": str(binary), "artifact_identity": artifact_identity,
              "commit": build["commit"], "target_cpu": build["target_cpu"],
              "machine": dict(platform.uname()._asdict()), "rows": rows}
    (args.evidence / "frame-times.json").write_text(json.dumps(output, indent=2) + "\n")
    print(json.dumps(output, indent=2))


if __name__ == "__main__":
    main()
