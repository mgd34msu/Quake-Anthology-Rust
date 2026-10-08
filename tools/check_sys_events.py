#!/usr/bin/env python3
"""Prove normal-candidate system-event ingress on an owned private display."""
import argparse
import json
from pathlib import Path
import socket
import time

from frame_timings import pinned_cores
from private_run import XClient, run


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--owner-profile", type=Path, required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--commands", help="execute config commands before real X-server input")
    parser.add_argument("--console-source", choices=("q1", "qw", "q2", "q2rr", "q3"))
    parser.add_argument("--check-command-time", action="store_true",
                        help="require seeded command time and duration diagnostics")
    parser.add_argument("--check-output-drain", action="store_true")
    parser.add_argument("--expected-output", help="require an echoed marker from the bound command")
    args = parser.parse_args()
    original = XClient.drive
    payloads = (b"first", b"second\0packet", bytes(range(256)))

    def drive(client, window, actions):
        # window_ready precedes startup hold and initial Cbuf execution. Wait
        # for a completed host frame so this hold tests the requested bind,
        # rather than one event delivered to the previous default binding.
        deadline = time.monotonic() + 5
        while True:
            lines = (args.evidence / "runtime.log").read_text().splitlines()
            if any(line.startswith('{"event":"system_event_frame"') for line in lines):
                break
            if time.monotonic() >= deadline:
                raise RuntimeError("candidate did not complete initial config frame")
            time.sleep(0.01)
        # Read the address reported by this copied candidate, never another
        # process. Send actual UDP bytes before the ordinary X-server key hold.
        log = (args.evidence / "runtime.log").read_text()
        events = [json.loads(line) for line in log.splitlines() if line.startswith('{')]
        listen = next(row for row in events if row.get("event") == "udp_listen")
        host, port = listen["address"].rsplit(":", 1)
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sender:
            for data in payloads:
                sender.sendto(data, (host, int(port)))
        original(client, window, actions)

    XClient.drive = drive
    cores = pinned_cores()
    arguments = ["--udp-listen", "127.0.0.1:0", "--frames", "180",
                 "--startup-hold-ms", "1000", "+set", "developer", "1"]
    if args.commands is not None:
        arguments += ["--commands", args.commands]
    if args.console_source is not None:
        arguments += ["--console-source", args.console_source]
    try:
        result = run(args.binary, args.owner_profile, args.evidence,
                     arguments,
                     actions=[{"key": "w", "hold_seconds": 1.25}], cores=cores)
    finally:
        XClient.drive = original
    events = result.get("events", [])
    rows = [row for row in events if row.get("event") == "system_event_frame"]
    exit_event = next((row for row in events if row.get("event") == "normal_exit"), {})
    ready = next((row for row in events if row.get("event") == "window_ready"), {})
    checks = {
        "private_run": result["result"] == "PASS",
        "normal_180_frames": exit_event.get("frames") == 180,
        "real_os_repeat": exit_event.get("key_repeats", 0) > 0,
        "all_frames_drained": len(rows) == 180 and all(row["queue_remaining"] == 0 for row in rows),
        "three_udp_packets_delivered": bool(rows) and rows[-1]["network_packets"] == len(payloads),
        "seat0_forward": any(row["seat0_movement"][0] > 0 for row in rows),
        "sustained_seat0_forward": sum(row["seat0_movement"][0] > 0 for row in rows) >= 30,
        "seat0_returns_neutral": bool(rows) and rows[-1]["seat0_movement"] == [0, 0, 0],
        "seat1_neutral": all(row["seat1_movement"] == [0, 0, 0] for row in rows),
        "no_event_or_packet_rejection": all(row["rejected"] == row["dropped_packets"] == 0 for row in rows),
        "forced_x11": ready.get("video_driver") == "x11" and ready.get("wayland_display_present") is False,
        "profile_and_candidate_preserved": result["owner_profile_unchanged"] and result["candidate_unchanged"],
        "owned_processes_stopped": result["remaining_owned_pids"] == [],
    }
    if args.check_command_time:
        # The R0 shell selects Q3 movement explicitly by default. Its first
        # command must exclude the one-second startup hold; absolute server
        # time remains the platform clock, independently of the duration.
        checks.update({
            "first_command_excludes_startup": bool(rows) and
                1 <= rows[0].get("seat0_duration_ms", 0) <= 50,
            "q3_movement_duration_bounds": len(rows) == 180 and all(
                1 <= row.get("seat0_duration_ms", 0) <= 200 and
                1 <= row.get("seat1_duration_ms", 0) <= 200 for row in rows),
            "absolute_server_time_preserved": bool(rows) and all(
                row.get("command_server_time_ms") == row["time_ns"] // 1_000_000
                for row in rows),
        })
    if args.check_output_drain:
        outputs = [row for row in events if row.get("event") == "output_frame"]
        checks["output_drains_once"] = len(outputs) == 180 and all(
            row["drains"] == 1 and row["remaining"] == row["stale_texts"] == 0 for row in outputs)
    if args.expected_output is not None:
        checks["bound_console_output"] = any(line.strip() == args.expected_output
            for line in (args.evidence / "runtime.log").read_text().splitlines())
    report = {"result": "PASS" if all(checks.values()) else "FAIL",
              "scope": "normal window-shell input and packet boundary; no map or protocol decoding",
              "cores": cores, "checks": checks, "normal_exit": exit_event,
              "udp_packets": len(payloads), "udp_payload_bytes": sum(map(len, payloads)),
              "config_commands": args.commands, "console_source": args.console_source,
              "first_command_duration_ms": rows[0].get("seat0_duration_ms") if rows else None,
              "first_command_server_time_ms": rows[0].get("command_server_time_ms") if rows else None,
              "forward_frames": sum(row["seat0_movement"][0] > 0 for row in rows),
              "gameplay_reached": result["gameplay_reached"]}
    (args.evidence / "verification.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report))
    return report["result"] != "PASS"


if __name__ == "__main__":
    raise SystemExit(main())
