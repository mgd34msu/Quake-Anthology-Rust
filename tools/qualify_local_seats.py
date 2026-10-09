#!/usr/bin/env python3
"""Check normal retail split-seat walks on owned private displays; no gameplay install."""
import argparse
import json
from pathlib import Path

from frame_timings import pinned_cores, renderer_label, summarize
from private_run import run


def qualify(binary, profile, content, evidence, cores):
    reports = []
    for product, name, native, foreign in (
        ("q1/id1", "e1m1", "q1", "q3"),
        ("q2/baseq2", "base1", "q2", "q3"),
        ("q3a/baseq3", "q3dm1", "q3", "q1"),
    ):
        for renderer in ("cpu", "gl"):
            directory = evidence / f"{name}-{renderer}"
            arguments = ["--content", str(content / product), "--map", name,
                         "--renderer", renderer, "--seat-policy", f"1:{native}:{foreign}:{native}",
                         "--mouse-seat", "0:1", "--frames", "600", "--warmup", "60",
                         "--startup-hold-ms", "1000", "--frame-timings",
                         "+set", "developer", "1", "+set", "com_maxfps", "120",
                         "--commands", "bind W +forward; bind MOUSE1 +forward; fov 90; cg_fov 90"]
            result = run(binary, profile, directory, arguments, timeout=120, cores=cores,
                         input_after_first_frame=True, actions=[
                             {"key": "w", "hold_seconds": 1.1, "wait_seconds": 0.2},
                             {"button": 1, "hold_seconds": 1.1, "wait_seconds": 0.2},
                             {"key": "w", "button": 1, "hold_seconds": 1.1, "wait_seconds": 0.2},
                         ])
            events = result.get("events", [])
            frames = [row for row in events if row.get("event") == "system_event_frame"]
            walks = [row for row in events if row.get("event") == "walk_frame"]
            starts = [row for row in events if row.get("event") == "map_loaded"]
            exit_row = next((row for row in events if row.get("event") == "normal_exit"), {})
            allocations = next((row for row in events if row.get("event") == "allocation_gate"), {})
            presented = next((row for row in events if row.get("event") == "world_frame_presented"), {})
            timings = next((row for row in events if row.get("event") == "frame_timings"), {})
            driver = next((row for row in events if row.get("event") == "gl_context"), None)
            render_frames = [row for row in events if row.get("event") == "render_frame"]
            per_seat = [[row for row in walks if row.get("seat") == seat] for seat in (0, 1)]
            phases = {label: sum(bool(row["seat0_movement"][0]) == first
                                 and bool(row["seat1_movement"][0]) == second for row in frames)
                      for label, first, second in (("keyboard_only", True, False),
                                                   ("mouse_only", False, True),
                                                   ("both", True, True))}
            checks = {
                "private_normal_exit": result["result"] == "PASS" and exit_row.get("frames") == 600,
                "real_x_key_repeat": exit_row.get("key_repeats", 0) > 0,
                "two_distinct_authored_spawns": len(starts) == 2
                    and starts[0]["spawn_entity"] != starts[1]["spawn_entity"]
                    and starts[0]["position"] != starts[1]["position"],
                "two_presented_views": presented.get("views") == 2 and len(render_frames) == 600
                    and all(row["views"] == 2 and row["presented"] and row["rejected"] == 0 for row in render_frames),
                "independent_device_phases": all(count >= 5 for count in phases.values()),
                "released_devices_neutral": bool(frames)
                    and frames[-1]["seat0_movement"] == frames[-1]["seat1_movement"] == [0, 0, 0],
                "one_queue_two_intakes": len(frames) == 660 and all(
                    row["drains"] == 2 and row["queue_remaining"] == row["rejected"] == row["dropped_packets"] == 0
                    for row in frames),
                "native_and_foreign_move_on_shared_state": all(len(rows) == 660 for rows in per_seat)
                    and all(len({tuple(row["position"]) for row in rows}) > 10 for rows in per_seat)
                    and all(row["movement"] == (native, foreign)[seat] and row["trace_rules"] == native
                            for seat, rows in enumerate(per_seat) for row in rows),
                "all_instrumented_rust_threads_zero_heap": allocations.get("passed") is True
                    and allocations.get("frames") == 600
                    and all(allocations.get(key) == 0 for key in ("allocations", "reallocations", "requested_bytes", "failed_frames")),
                "profile_candidate_preserved": result.get("owner_profile_unchanged") and result.get("candidate_unchanged"),
                "copied_owner_profile_consumed": presented.get("profile_consumed") is True,
                "owned_pids_stopped": result["remaining_owned_pids"] == [],
                "gameplay_gate_preserved": not result["gameplay_reached"]
                    and presented.get("gameplay") is False,
            }
            report = {"map": name, "renderer": renderer, "result": "PASS" if all(checks.values()) else "FAIL",
                      "checks": checks, "input_phase_frames": phases, "normal_exit": exit_row,
                      "allocation_gate": allocations, "evidence": str(directory),
                      "frame_times": summarize(timings["samples_ns"]) if timings.get("samples_ns") else None,
                      "gl_driver": driver,
                      "renderer_label": renderer_label(renderer, driver),
                      "error": result.get("error")}
            reports.append(report)
            (directory / "walk-checks.json").write_text(json.dumps(report, indent=2) + "\n")
            print(json.dumps(report), flush=True)
            if report["result"] != "PASS":
                break
        if reports[-1]["result"] != "PASS":
            break
    summary = {"result": "PASS" if len(reports) == 6 and all(r["result"] == "PASS" for r in reports) else "FAIL",
               "scope": "normal retail renderer/queue/usercmd/SERVER/prediction split-seat integration; no native gameplay, audio, wire or install qualification",
               "cores": cores, "runs": reports}
    (evidence / "qualification.json").write_text(json.dumps(summary, indent=2) + "\n")
    return summary


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--owner-profile", type=Path, required=True)
    parser.add_argument("--content", type=Path, required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    args = parser.parse_args()
    if args.evidence.exists():
        parser.error("use a fresh evidence directory")
    args.evidence.mkdir(parents=True, mode=0o700)
    report = qualify(args.binary.resolve(strict=True), args.owner_profile,
                     args.content.resolve(strict=True), args.evidence.resolve(), pinned_cores(8))
    return 0 if report["result"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
