#!/usr/bin/env python3
"""Check THE-861 scene commands on owned displays; this does not qualify gameplay."""
import argparse
import json
from pathlib import Path
import statistics

from frame_timings import pinned_cores
from private_run import run


EXPECTED_PROBES = {
    "left_multiply": [72, 48, 88],
    "right_multiply": [72, 48, 88],
    "left_clipped": [72, 48, 88],
    "right_clipped": [72, 48, 88],
    "left_textured_poly": [24, 120, 232],
    "right_textured_poly": [24, 120, 232],
    "left_hud_red": [220, 20, 40],
    "right_hud_red": [165, 15, 94],
    "left_hud_overlap": [20, 220, 40],
    "right_hud_overlap": [15, 165, 94],
    "left_clear": [12, 18, 88],
    "right_clear": [12, 18, 88],
}


def event(events, name):
    return next((row for row in events if row.get("event") == name), {})


def timing_summary(rows):
    result = {}
    for index, name in enumerate(("frontend", "backend", "present", "total")):
        samples = sorted(row[index] for row in rows)
        result[name] = {
            "median_ns": statistics.median(samples),
            "p99_ns": samples[593],
        }
    return result


def rgb_matches(actual, expected, tolerance=2):
    return len(actual) >= 3 and all(abs(a - b) <= tolerance for a, b in zip(actual[:3], expected))


def verify(result, folder, backend, renderer):
    events = result.get("events", [])
    ready = event(events, "window_ready")
    timing = event(events, "scene_timings")
    gate = event(events, "allocation_gate")
    fidelity = event(events, "scene_fidelity")
    normal = event(events, "normal_exit")
    probes = {row["name"]: row["rgba"] for row in fidelity.get("probes", [])}
    rows = timing.get("samples_ns", [])
    frame_stats = fidelity.get("frame_stats", {})
    total_stats = fidelity.get("measured_stats", {})
    containment = result.get("private_containment", {})
    expected_stats = {
        "views": 2, "triangles": 8 if renderer == "cpu" else 6,
        "draws_2d": 4, "rejected": 0, "pending_lights": 2,
    }
    checks = {
        "private_run": result.get("result") == "PASS",
        "normal_exit": normal.get("frames") == 600 and normal.get("warmup") == 60,
        "600_measured_frames": len(rows) == 600 and timing.get("frames") == 600 and timing.get("warmup") == 60,
        "four_timing_stages": len(rows) == 600 and all(len(row) == 4 and all(n > 0 for n in row) for row in rows),
        "uncapped_no_debugger": timing.get("vsync") is False and timing.get("debugger") is False,
        "zero_rust_allocations": gate.get("frames") == 600 and gate.get("passed") is True
            and gate.get("allocations") == 0 and gate.get("requested_bytes") == 0
            and gate.get("maximum_allocations") == 0 and gate.get("maximum_requested_bytes") == 0,
        "selected_renderer": ready.get("renderer") == renderer,
        "selected_video_driver": ready.get("video_driver") == ("x11" if backend == "x11" else "wayland"),
        "stable_command_counts": fidelity.get("stats_match") is True and frame_stats == expected_stats
            and total_stats == {name: value * 600 for name, value in expected_stats.items()},
        "shared_packet_shape": all(fidelity.get(name) == value for name, value in
            {"commands": 7, "entities": 2, "polys": 2, "vertices": 6, "lights": 2}.items()),
        "interior_pixel_probes": set(probes) == set(EXPECTED_PROBES)
            and all(rgb_matches(probes[name], expected) for name, expected in EXPECTED_PROBES.items()),
        "reference_rgb": fidelity.get("reference_rgb", {}).get("pixels_over_tolerance", 256000) <= 1280
            and fidelity.get("reference_rgb", {}).get("pixels") == 256000,
        "captured_window": (folder / "window.png").is_file(),
        "exact_framebuffer": (folder / "scene.ppm").is_file()
            and (folder / "scene.ppm").read_bytes().startswith(b"P6\n640 400\n255\n")
            and (folder / "scene.ppm").stat().st_size == len(b"P6\n640 400\n255\n") + 640 * 400 * 3,
        "profile_preserved": result.get("owner_profile_unchanged") is True,
        "candidate_preserved": result.get("candidate_unchanged") is True,
        "owned_cleanup": result.get("remaining_owned_pids") == [],
        "no_gameplay_claim": result.get("gameplay_reached") is False
            and ready.get("gameplay") is False and normal.get("gameplay") is False
            and timing.get("gameplay_qualified") is False,
    }
    if backend == "x11":
        checks["private_display"] = containment.get("wayland_display_unset") is True \
            and containment.get("sdl_video_driver") == "x11" \
            and bool(containment.get("display"))
    else:
        checks["private_display"] = containment.get("display_unset") is True \
            and containment.get("sdl_video_driver") == "wayland" \
            and bool(containment.get("private_wayland_socket")) \
            and isinstance(containment.get("compositor_pid"), int)
    summaries = timing_summary(rows) if checks["four_timing_stages"] else {}
    checks["timing_summary_matches_samples"] = bool(summaries) and summaries == timing.get("stages")
    return {
        "backend": backend, "renderer": renderer,
        "result": "PASS" if all(checks.values()) else "FAIL",
        "checks": checks, "timings": summaries, "allocation_gate": gate,
        "fidelity": fidelity, "renderer_info": ready.get("renderer_info"),
        "error": result.get("error"), "gameplay_qualified": False,
        "scope": "fixed entity/poly/2D command fixture; no world, palette or gameplay proof",
        "triangle_counting": "CPU counts near-clipped output triangles; GL counts submitted triangles",
        "limits": "Rust allocator covers its calling thread. SDL/driver allocations and GPU completion time are not measured.",
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--owner-profile", type=Path, required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--backend", choices=("x11", "sway", "weston"), action="append")
    parser.add_argument("--renderer", choices=("cpu", "gl"), action="append")
    parser.add_argument("--core")
    args = parser.parse_args()
    args.evidence.mkdir(exist_ok=False, parents=True)
    core = args.core or pinned_cores()
    records = []
    for backend in args.backend or ("x11", "sway", "weston"):
        for renderer in args.renderer or ("cpu", "gl"):
            folder = args.evidence / backend / renderer
            result = run(
                args.binary, args.owner_profile, folder,
                ["--renderer", renderer, "--startup-hold-ms", "1500", "--output", "scene.ppm"],
                backend=backend, cores=core, size=(640, 400), timeout=60,
            )
            record = verify(result, folder, backend, renderer)
            records.append(record)
            (folder / "verification.json").write_text(json.dumps(record, indent=2) + "\n")
    comparisons = []
    for backend in args.backend or ("x11", "sway", "weston"):
        pair = {record["renderer"]: record for record in records if record["backend"] == backend}
        if set(pair) != {"cpu", "gl"}:
            continue
        cpu = {row["name"]: row["rgba"] for row in pair["cpu"]["fidelity"].get("probes", [])}
        gl = {row["name"]: row["rgba"] for row in pair["gl"]["fidelity"].get("probes", [])}
        matched = set(cpu) == set(EXPECTED_PROBES) == set(gl) and all(
            rgb_matches(cpu[name], gl[name][:3]) for name in EXPECTED_PROBES
        )
        comparisons.append({"backend": backend, "interior_rgb_matches": matched, "tolerance": 2})
    report = {
        "result": "PASS" if all(row["result"] == "PASS" for row in records)
            and all(row["interior_rgb_matches"] for row in comparisons) else "FAIL",
        "core": core, "records": records, "comparisons": comparisons,
        "gameplay_qualified": False,
        "scope": "THE-861 developer scene fixture; does not qualify installation or renderer targets",
    }
    (args.evidence / "verification.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report))
    return report["result"] != "PASS"


if __name__ == "__main__":
    raise SystemExit(main())
