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

## R2 system event drain

THE-859's release example `system_events`, baseline CPU on core 23, measured
600 frames after 60 warm-up frames on 2026-10-07, without a debugger. Each
frame drained five events: a held/repeating key, controller axis, character,
real loopback UDP datagram and a time marker. It dispatched devices to
separate seats, passed packet bytes to the network boundary and built the
four usercmds. The sender, SDL and rendering were outside this workload.

| Scope | Median ns | p99 ns | Allocations/reallocations per measured frame |
| --- | ---: | ---: | ---: |
| Headless queue, input and UDP receive/dispatch | 1,500 | 1,540 | 0 |

The allocator positive control detected one allocation, one reallocation and
320 requested bytes. The workload delivered 660 datagrams and 660 characters,
with zero rejected events or dropped packets. This is event-service timing,
not map/gameplay or renderer qualification, and has no comparable regression
baseline yet. Reproduce with the commands in [system events](system-events.md).

## R2 host frame

THE-884's `host_frame` release example runs the actual app host function on
baseline CPU, pinned core 23, without a debugger. It warms 60 frames and
measures 600. This record is for commit 591b6f32, before THE-885's additional
local snapshot workload. Each frame receives one real UDP datagram and a held/repeating
key, drains before/after server work, runs the empty console and builds four
seat commands. The scheduler drives world/Q2/rerelease/Q3 counter callbacks at
50/100/25/50 ms on actual platform event time. A 16 ms pause and packet send
are outside timing/counting; SDL, presentation, game modules and map work are
absent. This is a new workload, not a regression comparison with THE-859.

| Scope | Median ns | p99 ns | Allocations/reallocations per measured frame |
| --- | ---: | ---: | ---: |
| Headless Com_Frame, two drains and mixed-rate counters | 3,850 | 8,191 | 0 |

The run delivered 660 packets, 659 repeats and 212/106/424/212 native-rate
ticks, with no hot cvar lookups. Its allocator positive control detected an
allocation and reallocation. Evidence is `THE-884-probe.json` and
`THE-884-probe-build.log` in the R2 evidence directory. This does not qualify
gameplay, combined-mode modules or either renderer.

```sh
timeout 300 cargo build --release -p qa-platform --example host_frame \
  --features allocation-tracking
timeout 300 taskset -c "$CORE" target/release/examples/host_frame
```

THE-885 extends that probe with same-frame local snapshot packets from each
native-rate counter callback. `--local` uses an in-process client packet in
place of UDP and opens no UDP socket. It measured 1,640 ns median and 2,140 ns
p99 on core 23 over 600 frames after 60 warm-up frames. All measured frames
had zero Rust-thread allocations/reallocations/requested bytes and zero hot
cvar lookups. Both directions drained without overwrite. The run delivered
1,399 packets, 659 key repeats and 211/105/423/211 world/provider ticks. Client
ring insertion is counted but precedes the timer; provider sends and both
drains are timed. This is another workload baseline, not a speedup comparison.
Evidence: `THE-885-local-probe.json` and `THE-885-probe-build.log`.

```sh
timeout 300 taskset -c "$CORE" target/release/examples/host_frame --local
```
