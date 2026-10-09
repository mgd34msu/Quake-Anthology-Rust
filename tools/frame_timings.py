#!/usr/bin/env python3
"""Measure private host frames or fixed retail CPU draws; neither proves gameplay."""
import argparse
import json
import math
import os
from pathlib import Path
import platform
import statistics
import subprocess

from private_run import identity, run


def pinned_cores(count=1):
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
    physical = {}
    for cpu in free:
        topology = Path(f"/sys/devices/system/cpu/cpu{cpu}/topology")
        try:
            key = (int((topology / "physical_package_id").read_text()),
                   int((topology / "core_id").read_text()))
        except (OSError, ValueError):
            key = (0, cpu)
        physical.setdefault(key, cpu)
    return ",".join(str(cpu) for cpu in sorted(physical.values())[-count:])


def summarize(samples):
    values = sorted(samples)
    return {"median_ms": statistics.median(values) / 1e6,
            "p99_ms": values[max(0, math.ceil(len(values) * 0.99) - 1)] / 1e6}



def renderer_label(renderer, driver):
    if renderer == "cpu":
        return "CPU edge/span"
    name = (driver or {}).get("renderer", "unknown")
    software = any(token in name.lower() for token in ("llvmpipe", "softpipe", "swrast"))
    return "OpenGL (Mesa software)" if software else "OpenGL"


def measure_retail(args, binary, build, artifact_identity, cores):
    benchmark = args.cpu_retail_binary.resolve(strict=True)
    benchmark_identity = identity(benchmark)
    folder = args.evidence.resolve() / ("retail-cpu-" + args.cpu_bands)
    rgba, depth = args.evidence.resolve() / "pixels.rgba", args.evidence.resolve() / "depth.f32"
    arguments = ["--content", str(args.content.resolve()), "--map", args.map,
                 "--output", str(rgba), "--depth-output", str(depth),
                 "--startup-hold-ms", "1500"]
    if args.cpu_bands != "auto":
        arguments += ["--cpu-bands", args.cpu_bands]
    result = run(benchmark, args.owner_profile, folder, arguments, cores=cores, timeout=90)
    if result["result"] != "PASS":
        raise RuntimeError("private retail draw failed: " + str(result.get("error")))
    timing = next(v for v in result["events"] if v.get("event") == "retail_draw_timings")
    allocation = next(v for v in result["events"] if v.get("event") == "allocation_gate")
    if (len(timing["samples_ns"]) != 600 or timing["warmup"] != 60 or timing["debug_build"]
            or timing["diagnostic_instrumentation"] or not allocation["passed"]
            or allocation["frames"] != 600
            or any(allocation[key] for key in ("allocations", "reallocations", "requested_bytes"))):
        raise ValueError("retail draw timing/allocation protocol failed")
    if identity(binary) != artifact_identity or identity(benchmark) != benchmark_identity:
        raise ValueError("candidate changed during timing run")
    output = {"scope": "retail_cpu_draw", "measured": True,
              "qualification": "Fixed scene; not gameplay or installation qualification",
              "commit": build["commit"], "target_cpu": build["target_cpu"],
              "artifact_identity": artifact_identity, "benchmark_identity": benchmark_identity,
              "machine": dict(platform.uname()._asdict()), "rows": [{
                  "map": args.map, "renderer": "CPU edge/span", "resolution": "640x400",
                  "requested_bands": args.cpu_bands, "bands": timing["bands"],
                  "workers": timing["workers"], "cores": cores, "frames": 600, "warmup": 60,
                  "debugger": False, "vsync": False,
                  "stages": {"draw": summarize(timing["samples_ns"])},
                  "allocation": allocation, "pixels": str(rgba), "depth": str(depth),
                  "evidence": str(folder), "workload": next(v for v in result["events"]
                      if v.get("event") == "retail_draw_workload")}]}
    (args.evidence / "frame-times.json").write_text(json.dumps(output, indent=2) + "\n")
    print(json.dumps(output, indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--owner-profile", type=Path, required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--content", type=Path)
    parser.add_argument("--map")
    parser.add_argument("--renderer", choices=("cpu", "gl"), default="cpu")
    parser.add_argument("--cpu-bands", choices=("auto", "1", "2", "4", "8"), default="auto")
    parser.add_argument("--cpu-retail-binary", type=Path,
                        help="fixed-scene CPU example; --binary supplies engine build metadata")
    parser.add_argument("--cores", help="explicit common affinity mask for comparable rows")
    args = parser.parse_args()
    if bool(args.content) != bool(args.map):
        parser.error("--content and --map must be supplied together")
    if args.cpu_retail_binary and (not args.map or args.renderer != "cpu"):
        parser.error("fixed retail CPU measurement requires --content, --map and --renderer cpu")
    if args.renderer != "cpu" and args.cpu_bands != "auto":
        parser.error("--cpu-bands applies to CPU rendering")
    binary = args.binary.resolve(strict=True)
    artifact_identity = identity(binary)
    build = json.loads(subprocess.check_output([str(binary), "--build-info"], text=True))
    cores = args.cores or pinned_cores(8 if args.map and args.renderer == "cpu" else 1)
    if args.cpu_retail_binary:
        measure_retail(args, binary, build, artifact_identity, cores)
        return
    rows = []
    for width, height in ((1920, 1080), (640, 400), (320, 200)):
        folder = args.evidence / f"{width}x{height}"
        arguments = ["--frames", "600", "--warmup", "60", "--uncapped", "--frame-timings",
                     "--startup-hold-ms", "2500", "--width", str(width), "--height", str(height)]
        if args.map:
            arguments += ["--content", str(args.content.resolve()), "--map", args.map,
                          "--renderer", args.renderer]
            if args.cpu_bands != "auto":
                arguments += ["--cpu-bands", args.cpu_bands]
        result = run(args.binary, args.owner_profile, folder, arguments,
                     size=(width, height), cores=cores, timeout=90)
        if result["result"] != "PASS":
            raise RuntimeError("private timing launch failed: " + str(result.get("error")))
        timing = next(v for v in result["events"] if v.get("event") == "frame_timings")
        samples = timing["samples_ns"]
        if len(samples) != 600 or timing["warmup"] != 60 or timing["vsync"]:
            raise ValueError("timing run did not complete the requested uncapped sample")
        stages = {stage: summarize([row[i] for row in samples])
                  for i, stage in enumerate(("input", "present", "total"))}
        if args.map:
            stages.update({stage: summarize([row[i] for row in timing["stage_samples_ns"]])
                           for i, stage in enumerate(timing["stage_columns"])})
        driver = next((v for v in result["events"] if v.get("event") == "gl_context"), None)
        label = renderer_label(args.renderer, driver) if args.map else "SDL software window shell"
        rows.append({"resolution": f"{width}x{height}", "scope": timing["scope"],
                     "renderer": label, "map": args.map, "driver": driver,
                     "requested_bands": args.cpu_bands,
                     "hardware_renderer_qualified": False, "warmup": 60, "frames": 600,
                     "debugger": False, "vsync": False, "workload": None,
                     "cores": cores, "stages": stages, "evidence": str(folder)})
    if identity(binary) != artifact_identity:
        raise ValueError("candidate changed during timing runs")
    output = {"scope": "retail_map_host" if args.map else "window_shell", "measured": True,
              "qualification": "Developer rendering measurement; no gameplay or installation qualification",
              "artifact": str(binary), "artifact_identity": artifact_identity,
              "commit": build["commit"], "target_cpu": build["target_cpu"],
              "machine": dict(platform.uname()._asdict()), "rows": rows}
    (args.evidence / "frame-times.json").write_text(json.dumps(output, indent=2) + "\n")
    print(json.dumps(output, indent=2))


if __name__ == "__main__":
    main()
