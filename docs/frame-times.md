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

## R1 Q1 hull traces

On 2026-10-07, release example `hull_trace` at `bd36feb` used retail e1m1
geometry and the same 10,000 segments for each native hull. Seed `0x5155414B`
selects alternating whole-map and short local segments. Each hull used 600
warm-up traces, then 60 repetitions of the segments (600,000 timed calls),
pinned to core 23, baseline CPU, without a debugger. Times are ns per trace,
including the two clock reads; p99 uses nearest rank. Rust fraction, endpoint,
plane and flags matched functions extracted from original `world.c` for all
30,000 cases. The counting allocator recorded zero trace allocations.

| Hull | Rust median ns | Rust p99 ns | C-port kernel median ns | C-port kernel p99 ns |
| --- | ---: | ---: | ---: | ---: |
| Point | 150 | 1,010 | 250 | 1,500 |
| Player | 150 | 720 | 170 | 880 |
| Large | 140 | 610 | 160 | 800 |

Muse's re-audit reported **229 µs per trace (229,000 ns)**. That historical
workload was not rerun, so these numbers do not establish a controlled speedup.
All three Rust medians meet THE-636's under-2-µs target for this workload.

The C column compiles the actual `src/world/collision/q1.c` native kernel and
its production arena/helpers with `-O3 -ffp-contract=off`, on the same segments
and core. It excludes full engine dispatch and uses the kernel's null blocking
policy. Its source was captured from the dirty C tree based on `18b27fa`; the
exact source snapshot is retained with the probe evidence. The C port uses
float epsilon/backoff literals, whereas Rust preserves original `world.c`
rounding. These are kernel measurements with different scopes and arithmetic,
not an exact cross-port speedup or installed-engine performance comparison.
Initial Rust probe build: 5.377 s; incremental committed-tree build: 0.017 s;
C probe build: 1.063 s. No gameplay or renderer qualification follows from them.

The evidence folder `THE-636-per-hull` contains `verification.json`, the original
C reference, segments, expected results, build/run logs and `c-port-source`.

```sh
timeout 300 python3 tools/check_hull_trace.py --pak "$Q1_PAK" \
  --qsrc "$QSRC" --c-port "$C_ENGINE" --output "$EVIDENCE"
```
