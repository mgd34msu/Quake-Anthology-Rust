# Frame times

R0 measurements from the software SDL window shell, build 3636f0c6, baseline
CPU target, core 23, Xvfb with Openbox. Each run used 60 warm-up frames followed
by 600 measured frames, uncapped, without a debugger or vsync. Times include
input polling, clear and presentation. They exclude startup and deliberate pacing.

| Resolution | Median ms | p99 ms | Scope |
| --- | ---: | ---: | --- |
| 1920×1080 | 13.437199 | 15.424342 | Software window shell |
| 640×400 | 1.744748 | 1.835870 | Software window shell |
| 320×200 | 0.516311 | 0.614083 | Software window shell |

These measurements verify the timing tool. They are not game renderer results,
and qualify none of the GL/CPU targets. R3/R4 must supply map workloads, hardware
GL timing on the spare GPU, and CPU renderer timing with matching game state.

```sh
timeout 300 python3 tools/frame_timings.py --binary target/candidate/qa-rust \
  --owner-profile "$PROFILE" --evidence "$EVIDENCE"
```

The tool selects a free pinned core after checking the C agent's pinned processes
and CPU sibling topology, and records per-stage median and nearest-rank p99 in
frame-times.json inside the evidence directory. Artifact provenance comes from
the compiled build information and device/inode/size/mtime, without fingerprints.

The installer accepts only gameplay timing reports with sim, scene, draw,
present, audio and total stages. Candidate and baseline must share the machine,
CPU target, pinned cores and workloads (roles, settings, seed, simulation steps,
final state and events). Any stage median or p99 regression over 10% refuses
installation. R0 window-shell reports are ineligible.
