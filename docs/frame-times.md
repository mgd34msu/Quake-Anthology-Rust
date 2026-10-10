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

THE-625's caller-selected trace rules were rechecked on 2026-10-08 with the
same retail geometry, seed, segments and native hulls, baseline release build
and core 23. All 30,000 original-C comparisons matched and trace allocations
remained zero. The point/player/large median times were 160/160/140 ns;
p99 was 1,060/740/620 ns. The release probe build took 6.124 s. Evidence is
`caller-selected-trace-rules/hulls/verification.json`; it records the dirty
source based on `de70bdff`. The measured world sources match staged tree
`2b350561b71574d6299c106ea29d702975cbc116`. This recheck did not remeasure
the C port or Muse and does not qualify installed gameplay or arbitrary
foreign-player shapes in Q1's fixed compiled hulls.

## THE-2875 owned scratch and allocation counter

The unused raw arena was removed on 2026-10-08; no production caller depended
on it. Core now forbids unsafe code. The release `frame_allocations` example
uses plain load-owned arrays and a reserved vector, pinned to core 23, baseline
CPU, without a debugger. After 60 warm-up frames, 600 measured frames reused
1,024 points, 4,096 bytes and 256 scratch values. Median and nearest-rank p99
were both 150 ns. The positive control detected one allocation, one
reallocation and 320 requested bytes; every measured frame and all totals
recorded zero allocation, reallocation and requested bytes.

This times the developer scratch/counter workload, with no gameplay, workers
or native heap measurement. It establishes no engine speedup or regression
comparison. Release build: 15.33 seconds. Evidence:
`THE-2875-safe-core/owned-scratch.json`. Reproduction commands are in
[frame allocations](frame-allocations.md).

## R1 brush-tree traversal

THE-625/THE-1862's 2026-10-08 release probe preserves BSP nodes, ordered leaf
brush references and inline-model membership in one immutable representation.
Caller-owned traversal frames, brush stamps and stationary-leaf storage allocate
at load. Q2 and Q3 clipping rules run over the same ten synthetic trees.
The original C functions matched all 13 declared raw result words for 15,045
trace/point pairs per rule: 10,000 seeded pairs plus 5,045 focused cases.
Identical-row controls passed; one-bit fraction and point-content mutations
were rejected. The Q3 fixture uses libc `memset` only to initialize trace state;
the extracted collision function bodies remain unchanged.

The baseline-CPU release timing example ran on core 23 without a debugger,
with 60 warm-up batches and 600 measured batches of 64 pairs each. A pair
includes a trace, point contents, all-word comparison and result accounting.
All 38,400 measured pairs per rule matched the native rows. Both runs recorded
zero calling-thread Rust allocations, reallocations and requested bytes; their
positive controls each detected one allocation. No workers were created.

| Caller rules | Median batch ns | p99 batch ns | Median batch ns / 64 |
| --- | ---: | ---: | ---: |
| Q2 | 19,230 | 158,800 | 300.46875 |
| Q3 | 7,225 | 109,730 | 112.890625 |

These are batch timings, not individual-trace latency distributions. They
exclude retail BSP loading, linked entities, patches, capsules, transforms,
rendering and installed gameplay. There is no comparable previous-tree or
C-port timing baseline, so no speedup or regression result follows. The two
release examples built together in 15.86 seconds. Evidence is
`brush-tree-20261008/comparison/report.json`, `q2-timing.json` and `q3-timing.json`.

```sh
timeout 300 cargo build --release -p qa-world --example brush_tree \
  -p qa-platform --example brush_tree_timing --features qa-platform/allocation-tracking
timeout 300 python3 tools/check_brush_tree.py --qsrc "$QSRC" \
  --rust-binary target/release/examples/brush_tree --output "$EVIDENCE/comparison"
timeout 300 taskset -c "$CORE" target/release/examples/brush_tree_timing q2 \
  "$EVIDENCE/comparison/tree-fixture.bin" "$EVIDENCE/comparison/q2-native.bin" \
  "$EVIDENCE/q2-timing.json"
timeout 300 taskset -c "$CORE" target/release/examples/brush_tree_timing q3 \
  "$EVIDENCE/comparison/tree-fixture.bin" "$EVIDENCE/comparison/q3-native.bin" \
  "$EVIDENCE/q3-timing.json"
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

## THE-887: matched real-console workload

Core 23, release baseline CPU build, no debugger, 60 warm-up and 600 measured
frames. Each frame appends the same alias/echo, vstr, five cvar assignments and
compressed-PK3 exec sequence in all five source contexts to one Console;
Com_Frame also receives a ConsoleLine, a held-key repeat and local packets.
The platform fixture generates times 16 ms apart so both builds execute the
same 210/105/421/210 world/Q2/Q2RR/Q3 counter ticks, 1,396 packets and 659
repeats. A 16 ms pause is outside counting/timing. Appending commands, both
command drains, provider counters, decoder reset/read, prints and client-frame
conversion are inside measurement. These are counters, not gameplay modules.

| Build/workload | Median ns | p99 ns | Maximum allocations/reallocations | Maximum requested bytes |
| --- | ---: | ---: | ---: | ---: |
| Archived 96878df0 with the same probe | 18,183,639.5 | 19,078,973 | 318 | 397,774 |
| THE-887 working tree | 182,714 | 213,895 | 0 | 0 |

Final cvar values and an empty command buffer are checked in both runs.
All 67,320 bytes of command output match directly; provider/packet/input counts
and parameters match. The baseline probe disables only its zero-allocation
rejection to report the measured violations. No content fingerprint is used.
The first fixed-text layout measured 53 ms median; separating text from hot
records and refreshing only dependent projections replaced that slow layout.
That exploratory run had wall-clock provider rates and is not a matched ratio.

Evidence: THE-887-console-comparison.json, THE-887-before-probe.log,
THE-887-console-probe.log and THE-887-probe-build.log in the R2 evidence folder.
This proves the named calling-Rust-thread workload, excluding SDL, map loading,
physics, rendering, native allocations and other threads. Per-game and combined
map acceptance, qualified installation and renderer timing remain outstanding.

```sh
cargo build --release -p qa-platform --example host_frame --features allocation-tracking
taskset -c "$CORE" target/release/examples/host_frame --local --console --content "$SCRIPT_PRODUCT"
```


## THE-888: config binds and button dispatch

Release baseline CPU, core 23, no debugger, 60 warm-up and 600 measured frames.
The unchanged THE-887 console workload measured 182,344 ns median and 202,015 ns
p99, with zero Rust-thread allocations/reallocations/requested bytes. Its command
output, final values, 210/105/421/210 counter ticks, 1,396 packets and 659 repeats
match the historical THE-887 measurement directly. Relative to that recorded
182,714/213,895 ns baseline, the ratios are 0.998 median and 0.944 p99. This is a
historical comparison, not a simultaneous baseline/candidate pair.

`--binds` adds ten config bind/query/unbind commands in each source view, cached
jump and alias-attack edges, and a second seat's controller movement to the same
workload. It measured 228,400 ns median and 247,705 ns p99, again with zero counted
allocations/reallocations/requested bytes. Both seat commands are checked every
frame against the prescribed holds. This new workload has no functional Muse
baseline and is not a speedup claim. Fixed fixture events are generated by this
development example; no input recording, journal or playback is involved.

Evidence: THE-888-console-comparison.json, THE-888-console-probe.log and
THE-888-binds-probe.log in the R2 evidence directory. Counts cover the calling
Rust thread and real Console/Com_Frame/local-ring operations, excluding SDL,
actual game modules, physics, scenes, renderers and other-thread/native heaps.

```sh
cargo build --release -p qa-platform --example host_frame --features allocation-tracking
timeout 300 taskset -c "$CORE" target/release/examples/host_frame --local --binds --content "$SCRIPT_PRODUCT"
```

## THE-889: server-side bot commands and startup duration

Release baseline CPU, core 23, no debugger, 60 warm-up and 600 measured frames.
The unchanged bind/console/local-ring workload measured 230,605 ns median and
256,006 ns p99. Its console output and final values, 210/105/421/210 counter
ticks, 1,396 packets and 659 repeats match THE-888's historical workload.
The historical median/p99 were 228,400/247,705 ns; this is not a fresh paired
before/after baseline.

Adding 64 connected bots with fixed per-client intents and mixed movement
policies measured 227,915 ns median and 271,966 ns p99. The 50 ms world ticks
built 12,288 bot commands during the measured frames; client frames only built
human commands. Both modes counted zero Rust-thread allocations/reallocations
and requested bytes. The added-workload numbers are absolute measurements,
not a speedup claim. Fidelity checks cover each bot's movement, buttons, impulse,
weapon, light level and server tick time after every frame.

Evidence: THE-889-probe-results.json, THE-889-binds-probe.log,
THE-889-bots-probe.log and THE-889-probe-build.log in the R2 evidence directory.
The workload still excludes navigation/AI, physics, modules, maps, scenes,
renderers, SDL and other-thread/native heaps. This is not gameplay qualification.

```sh
cargo build --release -p qa-platform --example host_frame --features allocation-tracking
timeout 300 taskset -c "$CORE" target/release/examples/host_frame --local --binds --bots --content "$SCRIPT_PRODUCT"
```

## THE-890: one output drain

A fresh sequential comparison rebuilt THE-889 commit 7d76e71b and measured its
unchanged bind/console/local workload against THE-890 on core 23. Both used
release baseline CPU, no debugger, 60 warm-up and 600 measured frames.

| Workload | Median ns | p99 ns | Counted allocations/reallocations |
| --- | ---: | ---: | ---: |
| Fresh THE-889 baseline | 216,384 | 300,156 | 0 |
| THE-890 matched workload | 218,099.5 | 306,966 | 0 |
| THE-890 plus 64 bots and output consumers | 227,374.5 | 324,126 | 0 |

Console output, final values, 1,396 packets, 659 repeats and 210/105/421/210
counter ticks match directly. Candidate/baseline ratios are 1.008 median and
1.023 p99. The added workload builds 12,288 bot commands in measured server
ticks and dispatches three sound/effect/print events each frame. Consumers
received 1,980 sounds and 1,980 effects including warm-up. Each frame drains
output once and leaves the ring empty. Requested Rust-thread bytes were zero.

Evidence: THE-890-paired.json, THE-890-paired-baseline.log,
THE-890-paired-current.log and THE-890-paired-outputs.log in the R2 evidence
directory. Earlier overlapping/checker and historical measurements are retained
but not used for this comparison. The source archive used the existing target
directory; its release artifacts were cleared afterward.

This is headless dispatch and consumer fidelity, not live mixing, particles,
map gameplay, renderer performance or installation qualification. Native and
other-thread heaps are outside the allocation count.

## THE-886: bounded stdin polling

A release-mode ABBA comparison on core 23, 60 warm-up and 600 measured frames
per run, held the local bind/console fixture and output destination constant.
The median of the two baseline medians was 204,259.5 ns; with idle stdin polling
it was 205,259.25 ns (+0.49%). The corresponding p99 medians were 294,521 and
291,576 ns. Both variants retained the same packets, repeats and tick counts,
with zero Rust allocations. An earlier single pair using captured pipe output
showed +12.3%; it is retained as evidence and is not used to qualify performance.
This is a headless shell fixture, not gameplay or renderer qualification.

Evidence: THE-886-timing.json and THE-886-timing-repeat.json in the R2 evidence
root. Private pipe and owned canonical PTY runs each completed 600 frames with
zero measured Rust allocations, including line dispatch and idle polling.

## THE-901: SDL3 transport

The private SDL3 signed-16 PCM transport probe on core 23 used 60 warm-up and
600 measured writes of 480 stereo sample frames at 48 kHz. It measured a 770 ns
median and 880 ns p99 for write/queued-byte queries, with zero Rust allocations
and 5,760 peak queued bytes. Bounded mixahead waiting is outside that stage.
The disk capture contained 655,360 samples, 623,040 non-silent, peak 2,048;
it is a generated 400 Hz signal, not a game cue. SDL's native device thread
and allocator are outside the Rust counter. This is not mixer qualification.

The display verifier records pinned shell input/present/total times on Xvfb,
sway and Weston. Those capped totals include the draining frame wait and are
not renderer timings or an installer baseline. Each backend separately checks
60 warm-up and 600 measured frames, time/input/output and zero Rust allocations.

Evidence: THE-901-audio.log, THE-901-audio-summary.json and the THE-901-three
verification directories in the R2 evidence root.

## THE-891: shared movement primitive

Core 23, release mode, 60 warm-up and 600 measured frames: 64 mixed-rule clients
with usercmd/bot construction, authoritative movement and current-command
prediction on analytic stairs/walls. Median 24,760 ns, p99 36,550 ns; 38,400
server movement steps, matching authoritative/predicted states and zero measured
Rust allocations or requested bytes. No prior movement workload supplies a
comparable baseline. This does not measure retail gameplay, renderer work,
network latency correction or native/other-thread allocations.

The original Q2/Q3 movement fixture comparison covers 1,152 states per game.
Q2 coordinates/velocities, flags and timers matched exactly. Q3 flags/timers
matched exactly; maximum float-component error was 0.0000112 units, with 544
components differing in float bits. That result is not a Q3 bit-match claim.
Evidence: THE-891-native-final/result.json and THE-891-movement-final.log in the R2
evidence root. Retail-map and combined-mode qualification remain pending.

A final-source ABBA host comparison against 55dc2e89, on core 23 with the same
local bind/console fixture and regular-file output, measured 205,043.25 ns for
the median of baseline medians and 204,170.25 ns for the candidate (-0.43%).
Both sides retained identical fixture results and zero Rust allocations. This
host workload has no loaded geometry; it verifies the existing host path, not
movement throughput. Evidence: THE-891-host-final-timing.json. The earlier
mid-step comparison (+0.37%) is retained separately.

## THE-861 shared scene command fixture, 2026-10-08

Release build, core 23, 640×400, 60 warm-up and 600 measured uncapped frames
per backend. Two views share a static mesh with texture/lightmap passes and
a near-clipped dynamic polygon; four ordered HUD draws exercise before-HUD
and final color-blend phases. Frontend, backend and present were measured
separately through platform counters. No build, checker or debugger ran during
the timing pass. GL was Mesa 26.2.2 llvmpipe, GL 4.6 core, rather than a spare-GPU
measurement. Its backend times include reused-buffer fence waits; they do not
measure complete GPU execution separately from presentation.

| Private display / consumer | Frontend median / p99 ns | Backend median / p99 ns | Present median / p99 ns | Total median / p99 ns |
|---|---:|---:|---:|---:|
| Xvfb CPU | 120 / 150 | 13,353,280 / 13,395,930 | 1,816,612 / 2,164,182 | 15,192,961 / 15,528,131 |
| Xvfb GL | 130 / 160 | 97,820.5 / 3,321,762 | 3,743,288 / 3,833,603 | 3,863,327.5 / 4,049,193 |
| sway CPU | 140 / 160 | 13,369,699.5 / 13,416,790 | 1,758,966.5 / 1,777,912 | 15,133,756 / 15,181,831 |
| sway GL | 120 / 150 | 68,555 / 1,693,591 | 3,764,597 / 3,787,603 | 3,837,898 / 3,862,343 |
| Weston CPU | 150 / 180 | 13,387,750 / 13,422,869 | 1,764,631.5 / 1,850,121 | 15,163,476 / 15,261,301 |
| Weston GL | 130 / 160 | 68,495 / 1,697,841 | 3,776,643 / 3,832,212 | 3,853,157 / 3,911,883 |

All six runs passed fixed packet counts, interior pixel probes, owned-display
containment, profile/candidate preservation and normal exit. All 256,000 final
RGB pixels matched the CPU reference exactly, including both blend phases.
Each run measured zero calling-thread Rust allocations or requested bytes;
SDL, compositor and driver allocations are outside that counter. Evidence:
`THE-861-scene-b/verification.json`, per-run logs, captures and `scene.ppm`.

These are entity/polygon/2D fixture results. The CPU triangle path is above the
CPU target; native world spans and surface caching remain THE-862. Retail maps,
native indexed palettes, dynamic lights, original images, mixed movement and
hardware renderer timing remain unqualified. This fixture cannot authorize an
installation or serve as a gameplay regression baseline.

## THE-862 retail visibility queries, 2026-10-08

Portable release example, core 23, 60 warm-up and 600 measured batches.
Each batch runs 32 preloaded queries per map on e1m1, base1 and q3dm1 (96 total),
rotating fixed seeded points and primary/secondary PVS unions. The measured
batch median is 11,302,168.5 ns and p99 is 12,077,568 ns. The calling-thread
Rust counter recorded zero allocations and requested bytes. Builds, checkers
and debuggers were absent during the timing pass.

All normalized PVS rows matched raw retail bits: 1,322,496 bits on e1m1,
2,093,808 on base1 and 889,248 on q3dm1. Ten thousand seeded points per map,
plus 9/18/7 spawn origins, matched independent point-in-leaf and primary/union
face-membership checks. All three maps exercised unions that expanded the
visible set. Areas, frustum rejection and shared DAG parents have separate
behavior fixtures. Evidence: `THE-862-visibility-b/verification.json` and
`visibility.json`; release example build 11.47 s.

This measures visibility queries over parsed retail records, with many seeded
points in solids and their all-visible fallback. It is not a gameplay frame,
renderer target, native-image comparison or installer baseline. The independent
reference is Rust over the original records; an extracted C executable was not
used. At that checkpoint, the app had not yet submitted a loaded retail world.

## THE-861/862 partial retail renderer, 2026-10-08

Commit `7ec5706bfef9ffdf1b3dcf0d2339131c087750ec`, portable release build,
core 23, owned Xvfb, 640×400, uncapped static spawn camera. Each map/backend
ran 60 warm-up and 600 measured frames. No build, checker or debugger ran
concurrently. A separate allocation-tracking executable measured the calling
Rust thread; it is not the normal shipping candidate. The normal candidate
built in 27.7702 s and the instrumented developer executable in 24.5061 s.
Both were built from the same clean commit with proof input disabled.

| Map / consumer | Scene median / p99 ns | Draw median / p99 ns | Present median / p99 ns |
|---|---:|---:|---:|
| e1m1 CPU | 27,915 / 32,970 | 2,345,397 / 2,388,382 | 1,781,511 / 1,853,791 |
| e1m1 GL | 39,720 / 45,810 | 5,603,409 / 7,154,446 | 5,612,589 / 5,761,094 |
| base1 CPU | 81,610 / 89,500 | 5,019,713.5 / 5,066,674 | 1,825,886.5 / 1,923,911 |
| base1 GL | 101,545 / 115,190 | 8,692,366.5 / 12,330,019 | 4,349,168.5 / 4,479,253 |
| q3dm1 CPU | 169,330.5 / 195,650 | 36,566,606 / 38,111,987 | 1,828,856.5 / 2,110,172 |
| q3dm1 GL | 190,475 / 213,240 | 18,288,543 / 22,053,186 | 5,325,653.5 / 8,090,686 |

All six measured passes recorded zero Rust allocations, reallocations and
requested bytes across 600 frames each. Each copied candidate/profile was
preserved, the process exited normally and the owned display was cleaned up.
SDL, compositor and driver allocations are outside this counter. Draw/present
times do not isolate GPU completion, and these runs did not record GL driver
identity. The base1 and q3dm1 CPU draw times exceed the 4 ms target.

The renderer is incomplete: q3dm1 CPU initially rejected 11 sky submissions;
its cloud sky and Q2 indexed warp/translucency still need coverage. A separate
120-frame repeated-key walk with the normal candidate recorded CPU rejection
in 19 base1 frames and all 120 q3dm1 frames, while GL rejected none. Captures
show retail world geometry but no module entities or HUD. Q2 GL brightness
also needs original image preparation. Static-pass rejection was logged only
on its initial frame, so that pass does not prove complete coverage.

Evidence: `static-tracking-report.json`, the six
`tracking-static-{e1m1,base1,q3dm1}-{cpu,gl}-a` directories and the normal
`retail-*` directories under `r3-20261008`. Their logs, display captures and
result files describe the bounded workload. These timings are not a matched
native-fidelity comparison, gameplay regression baseline or installation
qualification. Foreign movement, wall/step routes, native image parity and
combined views remain unqualified; the app still reports gameplay=false.


## THE-862 native images and CPU cloud sky, 2026-10-08

Commit `872a865b65a5fce1ccda47cbe23fa79f1b88cee1`, portable release build,
core 23, owned Xvfb, 640×400, uncapped static spawn camera. Each map/backend
ran 60 warm-up and 600 measured frames. No build, checker or debugger ran
concurrently. The normal candidate built in 28.8814 s. A separate developer
executable with calling-thread allocation tracking built in 26.2720 s from
the same clean commit. Both disabled proof input. The tracked executable
supplied the measurements below; it is not the shipping candidate.

| Map / consumer | Simulation median / p99 ns | Client median / p99 ns | Scene median / p99 ns | Draw median / p99 ns | Present median / p99 ns |
|---|---:|---:|---:|---:|---:|
| e1m1 CPU | 2,050 / 4,750 | 2,240 / 4,200 | 33,010 / 49,630 | 2,442,377 / 2,618,942 | 1,835,801.5 / 2,036,022 |
| e1m1 Mesa software GL | 2,330 / 4,540 | 2,640 / 4,240 | 41,975 / 52,780 | 5,507,429 / 6,900,165 | 5,776,044 / 5,943,065 |
| base1 CPU | 100,050 / 108,990 | 87,110 / 94,250 | 81,255 / 89,230 | 5,076,149 / 5,113,704 | 1,822,141.5 / 1,899,731 |
| base1 Mesa software GL | 100,210 / 110,600 | 86,740 / 95,130 | 96,600 / 109,000 | 8,500,266 / 12,057,568 | 4,462,483 / 4,844,884 |
| q3dm1 CPU | 31,705 / 40,900 | 22,120 / 27,900 | 175,480 / 205,631 | 44,368,553 / 45,978,173 | 1,855,287 / 2,311,122 |
| q3dm1 Mesa software GL | 31,730 / 36,070 | 22,040 / 28,230 | 187,610 / 201,140 | 18,311,098.5 / 21,877,306 | 5,414,184 / 8,241,826 |

All six aggregate allocation gates counted zero Rust allocations,
reallocations and requested bytes, with no failing measured frames.
Each run preserved its copied candidate/profile, exited normally and cleaned
up its owned display. SDL, compositor and driver allocations are outside this
counter. All GL runs recorded llvmpipe LLVM 22.1.8, GL 4.6 core, Mesa
26.2.2-arch1.1. Draw/present times do not isolate GPU completion. The base1
and q3dm1 CPU draw medians still exceed the 4 ms target.

A separate normal-candidate walk on each map/backend accepted real key
repeats and stdin commands. It measured 120 frames after 60 warm-up frames.
All q3dm1 CPU render records reported zero rejected submissions and its
capture showed the cloud sky. Base1 CPU rejected submissions in 22 measured
frames, at most two per frame; the other five cases rejected none. Those
counts come from the walking runs with developer diagnostics enabled.
The static allocation runs disabled those diagnostics and recorded coverage
only at startup, so they do not prove zero rejections across all 600 frames.

The earlier `7ec5706b` timing workload lacked the CPU cloud sky and used
different native image preparation. Its output is not fidelity-equivalent,
so the two sets do not establish a regression percentage. Q3 CPU prepared
RGB levels, mip selection and precombined surface caching remain open.
Q2 indexed warp/translucency, native bitmap parity, module entities/HUD,
foreign movement routes and combined views also remain unqualified.
The app reports gameplay=false. These runs cannot qualify installation or
supply a gameplay regression baseline.

Evidence under `r3-20261008`: `native-tracking-report.json`, the six
`native-tracking-{e1m1,base1,q3dm1}-{cpu,gl}-b` directories,
`native-image-retail-report.json` and the six normal
`native-image-{e1m1,base1,q3dm1}-{cpu,gl}-b` directories. Their runtime logs,
result files, embedded stage samples and window captures record these results.


## THE-862 q3dm1 CPU sampling profile, 2026-10-08

The copied normal `872a865b` candidate ran q3dm1 CPU at 640×400 on owned
Xvfb, core 23, with a fresh copied owner profile. Perf attached after the first
world frame and sampled `cpu-clock:u` at 999 Hz with DWARF call stacks during
600 measured frames after 60 warm-up frames. The run exited normally,
preserved candidate/profile and cleaned its owned PIDs. Perf recorded
30,762 samples, zero lost samples. No build, checker or debugger ran alongside
it. This separate profiling run cannot supply qualification timings.

| Symbol | Process self samples |
|---|---:|
| Repeat linear texture sampler | 21.17% |
| Clamp linear texture sampler | 12.18% |
| World span consumption | 18.23% |
| Rust floating remainder | 7.28% |
| Other floating remainder | 0.78% |
| Floating round | 5.88% |
| World clipping | 5.18% |
| Opaque world setup | 3.68% |
| Edge polygon submission | 1.40% |
| Stage texture modifiers | 1.04% |

These are self samples across the process, including presentation and native
threads. They are neither draw-only percentages nor per-phase elapsed times.
Source inspection shows RGB world spans currently bypass the cache and sample
stages per pixel. The existing cache storage extension has not yet changed
that path. Curves, sky, material classes and cache hit rate need runtime
counters to accompany these samples. Static texture-times-lightmap caching,
prepared RGB mip sampling, deterministic banded raster work and pinned
before/after results remain required before THE-862 review. No speedup is
claimed from this profile.

Evidence: `q3dm1-cpu-profile-before-a/{profile.json,perf-self.json,perf-flat.txt,
perf.data,runtime.log,result.json}` under `r3-20261008`.

The clean portable normal `2368a9b0` candidate built in 30.5749 s with proof
input disabled. Its separate uncapped CPU run used owned Xvfb, core 23,
640×400, 60 warm-up frames and 600 measured host frames. Draw median/p99 were
45.530763 / 48.765556 ms. No build, checker or debugger ran alongside it.
It exited normally, preserved candidate/profile and cleaned owned PIDs.
This partial-renderer baseline has no allocation instrumentation and does
not qualify gameplay or the 4 ms target.

| Final rendered frame category | Pixel writes |
|---|---:|
| All categories | 509,805 |
| Sky | 81,730 |
| Generic material stages | 428,075 |
| Indexed cache | 0 |
| Curves, subset of generic stages | 55,424 |
| Multiple stages, subset of generic stages | 425,337 |

The final frame contained 3,764 polygons, including 1,487 patch polygons,
and reported zero rejects. This final-frame counter does not prove zero
rejects in every frame. Cumulative cache hits, fills, evictions and rejects
were all zero over 661 rendered frames, including startup and warm-up.
Curve and multiple-stage categories overlap. The counters count actual
writes, including material passes, rather than unique screen pixels.

Evidence: `q3dm1-cpu-workload-before-a/{workload.json,runtime.log,result.json}`
under `r3-20261008`.


## THE-862 serial prepared RGB checkpoint, 2026-10-08

Clean portable commit `e9e6211d249cc38954e98d5fac94fa96a4a517dd`
built the normal candidate in 32.7986 s and a separate allocation-tracked
developer candidate in 27.9215 s. Both disabled proof input. Each CPU run used
owned Xvfb, core 23, 640×400, a fresh copied owner profile, 60 warm-up frames
and 600 measured uncapped frames. No build, checker, profiler or debugger ran
concurrently. All six runs exited normally, preserved candidate/profile and
cleaned their recorded owned PIDs.

| Map / candidate | Simulation median / p99 ns | Client median / p99 ns | Scene median / p99 ns | Draw median / p99 ns | Present median / p99 ns |
|---|---:|---:|---:|---:|---:|
| e1m1 / normal | 1,520 / 2,060 | 1,910 / 2,650 | 27,845 / 35,610 | 2,420,417 / 2,467,182 | 1,802,231 / 1,925,761 |
| base1 / normal | 100,035 / 109,750 | 88,315 / 99,310 | 85,540 / 105,210 | 5,239,064 / 5,399,344 | 1,811,486.5 / 2,165,002 |
| q3dm1 / normal | 32,840 / 45,050 | 22,950 / 35,160 | 215,990 / 278,340 | 52,107,273 / 57,219,412 | 2,070,131.5 / 2,773,122 |
| e1m1 / tracking | 1,510 / 2,280 | 1,880 / 2,540 | 28,470 / 37,420 | 2,425,332 / 2,501,312 | 1,802,676 / 1,873,562 |
| base1 / tracking | 101,020.5 / 115,360 | 88,320 / 96,940 | 86,125.5 / 147,010 | 5,231,689 / 5,859,534 | 1,814,011.5 / 2,219,572 |
| q3dm1 / tracking | 33,435 / 41,230 | 22,185 / 28,590 | 208,925 / 233,650 | 52,196,412.5 / 53,734,119 | 2,074,537 / 2,498,792 |

All three tracked runs recorded 600 measured frames, zero failing frames,
zero allocations, zero reallocations and zero requested bytes on the calling
Rust thread. Rendering workers are not wired in this checkpoint. SDL and
native heap work are outside that counter; the normal candidate has no
allocation instrumentation.

The conservative RGB cache applies only to exact nearest-base, constant-light
pairs. q3dm1 used none of these recipes: hits, fills, evictions and rejects
were zero over 661 rendered frames including startup and warm-up. Its final
normal frame contained 3,764 polygons, including 1,487 patch polygons, and
509,811 pixel writes. Sky wrote 81,730 pixels; generic stages wrote 428,081.
Curves contributed 55,424 writes and multiple stages 425,343, overlapping
subsets of the generic count. Final-frame rejects were zero; diagnostics
were disabled, so this does not prove zero rejects throughout the run.

The normal draw medians are 2.4204 ms on e1m1, 5.2391 ms on base1 and
52.1073 ms on q3dm1. Base1 and q3dm1 exceed the 4 ms target. Prepared RGB
mip sampling changes the prior image workload, and live shader time can
change animated pixel/span counts between runs. These measurements establish
a new serial checkpoint; they do not establish a fidelity-matched regression
percentage or speedup. Broad filtered static caching, prepare-once parallel
raster and native image/gameplay qualification remain open. No installation.

Evidence under `r3-20261008`: `serial-rgb-report.json` and
`serial-rgb-{normal,tracking}-{e1m1,base1,q3dm1}-cpu-a` directories, each
with runtime log, private-run result, workload counters and window capture.

## THE-862 static lit surface cache checkpoint, 2026-10-08

The developer `cpu_retail` example renders one immutable retail scene packet
at native spawn with the copied owner's presentation/FOV and shader time zero.
It times only `CpuBackend::render`, with presentation and reporting outside the
sample. Both engine snapshots use the identical committed benchmark source
from `df974fad`; neither is an installation candidate or gameplay proof.

The baseline is `e9e6211d249cc38954e98d5fac94fa96a4a517dd`; the static-cache
snapshot is `2c64ec1c6471b38c71ae44bfe86a3ee92e33d0d5`. Each archive was built
from a cleared Cargo release cache, with portable release settings and allocation
tracking. Clearing that cache matters when moving between archived revisions:
an intermediate `before-b` run reused newer renderer dependencies despite the
older source archive. Those rows are discarded. Rebuilt `before-c` images match
all three earlier `before-a` raw images byte for byte.

All six accepted runs used owned Xvfb, forced X11/private audio, core 23,
640×400, 60 warm-up draws and 600 measured draws. They exited normally, preserved
the copied candidate and owner's original profile, and cleaned recorded owned
PIDs. No build, checker, debugger or profiler ran alongside timing. All reported
zero calling-thread Rust allocations/reallocations/requested bytes, stable
backend statistics and zero rejected draws. Workers are not wired in this
checkpoint; this counter excludes native SDL/driver heap activity.

| Map | Baseline draw median / p99 ns | Static-cache draw median / p99 ns |
|---|---:|---:|
| e1m1 | 2,427,977 / 2,472,542 | 2,436,262 / 2,462,422 |
| base1 | 5,284,889 / 5,332,544 | 5,172,409 / 5,289,224 |
| q3dm1 | 51,089,057 / 52,304,838 | 23,560,732 / 25,819,408 |

q3dm1 now fills native base-mip texels multiplied by bilinear lightmap mip zero
into the shared rover. Its measured 600-draw cache deltas are 11,863,200 hits,
zero fills, zero evictions and zero rejects. Across startup, warm-up and measured
661 draws, it recorded 13,067,848 hits, 1,444 fills, zero evictions and zero
rejects. The old baseline recorded zero for all four cache counters.

The final cached frame wrote 211,003 pixels through 19,772 RGBA spans, including
16,881 minified spans. Generic non-sky stages wrote 4,115 pixels through 2,647
stage calls; sky wrote 83,712 pixels through 17,608 stage calls. Total writes
are 298,830; the old separate-stage baseline wrote 509,833. Both consume the same
3,770 clipped polygons, including 1,496 patch polygons, and the same immutable
scene metadata. These are writes, not unique screen pixels.

The e1m1 and base1 indexed images remain byte-identical. q3dm1 changes 193,967 of
256,000 pixels, with a maximum channel difference of 75. Its new software-cache
lattice reads original mip texels and rounds the native collapsed product once;
the former independently filtered stage image is not an oracle for that
convention. Independent cache-fill fixtures cover native mip/ROI/color arithmetic.
Retail native visual acceptance remains open, so these rows do not establish a
fidelity-matched speedup or qualify the timing guard.

The 23.56 ms q3dm1 and 5.17 ms base1 medians still exceed 4 ms. Shared geometry
preparation and fixed raster bands on platform workers are next. Actual animated
stage handling, complete static-material coverage, live RGB style/dlight inputs,
foreign movement, combined views and gameplay qualification remain open. No
installation or Slack notice.

Evidence under `r3-20261008`: `fixed-retail-before-c-report.json`,
`fixed-retail-after-c-report.json`, `fixed-retail-static-cache-comparison.json`,
`fixed-retail-baseline-rebuild-validation.json`, and the corresponding
`fixed-retail-{before,after}-c-{e1m1,base1,q3dm1}-cpu` directories containing
runtime logs, private-run results, raw RGBA and window captures.

## THE-862 shared serial preparation checkpoint, 2026-10-08

Commit `e56c2a0761b50ac8c70e8536dfe0392c1a50135f` prepares world geometry,
shader attributes and clipping once per view, then consumes that frozen result
through one full-height raster band. Its portable release developer example
built from a cleared release dependency cache in 32.3779 s. The immutable scene
packet and raw RGBA match the static-cache checkpoint byte for byte on all three
maps. The benchmark source also remains byte-identical.

Owned private Xvfb runs used core 23, 640×400, shader time zero, 60 warm-up draws
and 600 measured draws. All exited normally, preserved the owner profile and
copied candidate, and cleaned recorded owned PIDs. No build, checker, debugger
or profiler ran alongside timing.

| Map | Draw median / p99 ns | Measured cache hits / fills / evictions / rejects |
|---|---:|---:|
| e1m1 | 2,212,476.5 / 2,239,911 | 2,227,200 / 0 / 0 / 0 |
| base1 | 4,858,093 / 4,996,074 | 3,318,000 / 0 / 0 / 0 |
| q3dm1 | 21,729,916 / 24,177,567 | 11,863,200 / 0 / 0 / 0 |

Each run recorded zero calling-thread Rust allocations, reallocations and
requested bytes, stable backend counters and zero rejects. Cache counts match
the preceding static-cache checkpoint. The polygon counter now counts prepared
boundaries once, before row-window scanning, rather than accepted scanner
polygons; its different value does not indicate a different scene packet.
Workers are not dispatched in this checkpoint. Native heap work, gameplay and
installation remain outside its qualification. Base1 and q3dm1 still exceed
4 ms.

A separate process profile of the preceding static-cache binary recorded
16,084 samples with zero lost samples. It identifies cached spans, generic and
sky material stages, clipping and memory copies as remaining draw costs.
Inspection of that release binary found a 4,568-byte Camera copy followed by a
4,560-byte Refdef copy on every opaque span. Commit `4e0ed10c` changes those
parameters to borrowed immutable references; its performance is measured
separately. The profile's timings are discarded and its libc samples are not
all attributed to those copies.

Evidence under `r3-20261008`: `fixed-retail-prepare-serial-a-report.json`,
`fixed-retail-prepare-serial-comparison.json`, corresponding
`fixed-retail-prepare-serial-a-{e1m1,base1,q3dm1}-cpu` directories, and
`profile-static-cached-q3dm1-cpu`.

Commit `4e0ed10ca2770390f783145d26e8ec02ce6b4762` built the borrowed-view
portable release example in 33.5617 s from a cleared release cache. Under the
same private core-23 workload, its 60/600 draw measurements are:

| Map | Borrowed-view draw median / p99 ns |
|---|---:|
| e1m1 | 2,067,036.5 / 2,225,911 |
| base1 | 4,600,083 / 4,759,513 |
| q3dm1 | 19,517,524 / 22,332,287 |

All three raw images, immutable workload fields and backend/cache counters match
`e56c2a07` exactly. Per-run private profile/evidence paths are excluded from the
workload comparison. The measured calling-thread allocation counters remain
zero, with normal exits and recorded owned-PID cleanup. This remains a serial
draw benchmark, and base1/q3dm1 remain over the target. Evidence:
`fixed-retail-borrow-serial-a-report.json`,
`fixed-retail-borrow-serial-comparison.json` and corresponding private-run folders.

## THE-862 platform raster band checkpoint, 2026-10-08

Commit `dc9cd60c643245f51e98e79b0704c27a77329dc6` built the portable release
allocation-tracked retail example in 33.2382 s from a cleared release cache.
Both platform and app tracking features were enabled. One immutable time-zero
scene packet is rendered with 1, 2, 4 or 8 row bands through the shared app
platform dispatcher. The one-band path executes inline; the others use a
persistent platform worker pool with one worker per band.

The AMD Ryzen 9 5900X private runs used 640×400, 60 warm-up and 600 measured
draws, owned Xvfb/forced X11/private audio, and fresh copied owner profiles.
The process affinity masks were `23`, `22,23`, `20-23` and `16-23`, respectively.
Those logical CPUs select distinct physical cores; workers inherit the process
mask and are not individually pinned. No build, checker, debugger or profiler
ran alongside these measurements. All twelve runs exited normally, preserved
candidate/profile and cleaned their recorded owned PIDs.

| Map | Bands | Draw median / p99 ns |
|---|---:|---:|
| e1m1 | 1 | 2,114,262 / 2,262,122 |
| e1m1 | 2 | 2,129,601 / 2,345,562 |
| e1m1 | 4 | 916,310.5 / 1,456,641 |
| e1m1 | 8 | 858,430 / 1,178,940 |
| base1 | 1 | 4,607,858.5 / 4,677,813 |
| base1 | 2 | 4,219,168 / 4,894,114 |
| base1 | 4 | 2,645,092 / 2,846,872 |
| base1 | 8 | 1,708,531.5 / 2,236,182 |
| q3dm1 | 1 | 18,940,294 / 21,370,516 |
| q3dm1 | 2 | 16,785,257.5 / 21,695,906 |
| q3dm1 | 4 | 13,495,139.5 / 15,960,462 |
| q3dm1 | 8 | 10,475,298 / 14,566,900 |

All four band counts produce byte-identical RGBA and depth buffers for each map,
with identical immutable workload fields. RGBA also matches the borrowed-view
serial checkpoint. Each run records zero measured allocations, reallocations
and requested bytes across the calling Rust thread and every dispatched worker.
Counts are collected after every batch, including rejected batches, and consumed
once per frame. Positive-control fixtures cover multiple batches, caller counts,
startup/reset, the terminal batch and an error batch. Native SDL/driver heap,
scene submission and presentation are outside the direct-draw allocation gate.

The total cache arena remains 33,554,432 bytes at every count; band shares are
32, 16, 8 and 4 MiB. Other load-owned scanners, metadata and preparation scratch
are additional memory, not included in that arena figure. q3dm1's largest
mandatory surface reservation is 2,454,928 bytes. Its cache measurements are:

| Bands | Measured hits / fills / evictions / rejects | Startup + warm-up + measured lifetime hits / fills / evictions / rejects |
|---|---:|---:|
| 1 | 11,863,200 / 0 / 0 / 0 | 13,067,848 / 1,444 / 0 / 0 |
| 2 | 11,863,200 / 0 / 0 / 0 | 13,067,770 / 1,522 / 0 / 0 |
| 4 | 11,863,200 / 0 / 0 / 0 | 13,067,684 / 1,608 / 0 / 0 |
| 8 | 11,863,200 / 0 / 0 / 0 | 13,067,484 / 1,808 / 0 / 0 |

Per-band records sum to the reported cache totals. q3dm1 remains over 4 ms at
every count. Live lightstyle/dlight rebuilding, complete static cutout/overlay
coverage, native visual acceptance, moving-camera gameplay and installation
also remain open. No installation or Slack notice.

A separate eight-band process profile records 16,734 samples, zero lost samples,
and normal private cleanup. Its timings are discarded. The largest self-sample
categories are the span callback 25.53%, bilinear sampler 12.78%, edge insertion
8.81%, row-range raster setup 8.20%, shared view preparation 4.30% and clipping
3.36%. These are aggregate process CPU samples, not nested wall-clock timings
or an attribution of all cost to one material category.

Evidence under `r3-20261008`: `fixed-bands-platform-a-report.json`,
`fixed-bands-platform-a-comparison.json`, the twelve
`fixed-bands-platform-a-{1,2,4,8}-{e1m1,base1,q3dm1}-cpu` directories with raw
pixels/depth, per-band records and private results, and
`profile-platform-bands8-q3dm1-cpu`.

## THE-862 diagnostic dispatch wall times, 2026-10-08

Portable allocation-tracked commit `083ef9d0d220ccd4aad5c3a6d63362b332c55be1`
built from a cleared release cache in 33.9510 s. Separate private q3dm1 runs
at 640×400 enabled the developer example's optional nested platform clocks.
These are diagnostic measurements, not performance qualification or CPU times.
They use the same immutable packet, 60 warm-up/600 measured draws and the
one-band core-23/eight-band cores-16-23 process affinity masks. Raw RGBA matches
the noninstrumented-stage-timer checkpoint; caller/worker allocations remain
zero, and normal private exit/profile preservation/owned cleanup passed.

| Bands | First opaque dispatch wall median / p99 ns | Subsequent dispatch wall sum median / p99 ns | Approximate serial/other wall residual median / p99 ns |
|---|---:|---:|---:|
| 1 | 8,211,746 / 8,834,266 | 6,867,380 / 7,032,175 | 4,172,463 / 5,095,964 |
| 8 | 3,203,502 / 6,378,764 | 2,823,747 / 5,097,254 | 5,105,149 / 5,874,784 |

Both packets dispatch 14 batches per draw. Waits include the completion barrier
and allocation-count collection. The residual subtracts each frame's sum of
nested waits from its direct total; it includes instrumentation and other serial
work. Summaries of separate distributions are not additive. Every frame's
summed waits were within its direct total. These numbers identify preparation
and worker consumption as separate remaining costs, not an exact attribution
of that residual to one function.

Evidence under `r3-20261008`: `diagnostic-bands-083ef9d0-report.json` and
`diagnostic-bands-083ef9d0-{1,8}-q3dm1-cpu`.

## THE-862 cached span and band-index checkpoint, 2026-10-08

Commits `314fdd2`, `205693d` and `f9ff6bc0` add ordered native row indices,
cold Product color/source-coordinate preparation and baseline SSE2 cached RGBA
span consumption. The common edge scanner, sampling lattice and depth/rank
semantics remain the comparison contract.

The paired portable release developer examples use engine `dc9cd60c` before
and `f9ff6bc0` after, with the same explicit `f9ff6bc0` benchmark source.
Their release dependency caches were cleared separately; builds took 34.5131 s
and 34.6078 s. Both app and platform allocation tracking are enabled; proof
input is disabled. These are developer draw candidates, not installed binaries.

The workload is the same fixed time-zero 640×400 retail scene. Each of the
24 private runs used a fresh owner-profile copy, owned Xvfb/forced X11/private
audio, 60 warm-up and 600 measured draws. Affinity masks are `23`, `22,23`,
`20-23` and `16-23` for 1/2/4/8 bands; workers inherit those masks. No build,
checker, debugger or profiler ran during the paired measurements. Private exit,
candidate/profile preservation and recorded-PID cleanup passed for every run.

| Map | Bands | Before median / p99 ms | After median / p99 ms | Median change |
|---|---:|---:|---:|---:|
| e1m1 | 1 | 2.1026 / 2.1321 | 2.0312 / 2.0547 | -3.40% |
| base1 | 1 | 4.6417 / 4.8098 | 4.5602 / 4.5923 | -1.76% |
| q3dm1 | 1 | 19.3386 / 20.9672 | 15.7264 / 16.4951 | -18.68% |
| e1m1 | 2 | 2.1359 / 2.2358 | 2.0479 / 2.1350 | -4.12% |
| base1 | 2 | 4.2346 / 4.8362 | 4.2325 / 4.7453 | -0.05% |
| q3dm1 | 2 | 15.8899 / 19.9986 | 13.7973 / 16.3213 | -13.17% |
| e1m1 | 4 | 0.9164 / 1.4509 | 1.2446 / 1.7895 | +35.81% |
| base1 | 4 | 2.6223 / 2.8716 | 2.5479 / 2.7882 | -2.84% |
| q3dm1 | 4 | 13.5284 / 15.5483 | 12.2644 / 13.6680 | -9.34% |
| e1m1 | 8 | 1.0788 / 1.3306 | 1.0103 / 1.7342 | -6.35% |
| base1 | 8 | 1.7426 / 2.8983 | 1.6407 / 2.1675 | -5.85% |
| q3dm1 | 8 | 10.6704 / 14.7839 | 8.9466 / 12.4617 | -16.15% |

Raw RGBA and inverse-depth bits are identical before/after and across all band
counts within each map. Immutable workload fields and all reported backend and
raster/cache counters match. Every measured calling-thread and worker allocation,
reallocation and requested-byte count is zero. SDL/native heap, scene submission
and presentation remain outside the direct-draw allocation gate.

q3dm1 records 11,863,200 measured cache hits, zero fills, zero evictions and
zero rejects at every count. Lifetime hits/fills are unchanged from the preceding
band checkpoint, including 13,067,484 hits and 1,808 fills at eight bands.
The one cache arena remains 33,554,432 bytes, divided among bands. Load-owned
index payloads range from 22,064 to 176,512 bytes for e1m1, 31,620 to 252,960
for base1 and 73,860 to 590,880 for q3dm1; these are separate from the rover
budget. Cold material fields, source coordinates and other scratch are also
additional memory.

q3dm1 improves from 10.6704 to 8.9466 ms at eight bands and remains above 4 ms.
The four-band e1m1 row regresses by 35.81%; it is retained and requires a matched
recheck rather than qualification. Live lighting, complete static stage coverage,
native-image acceptance, movement/gameplay and qualified installation remain open.

A separate process profile of `f9ff6bc0` captured 13,009 samples with zero lost
samples and normal private cleanup. Its timings are discarded. Self samples
include span consumption 22.15%, bilinear sampling 15.82%, raster setup 10.85%,
edge insertion 4.58%, shared view preparation 3.95%, clipping 3.91%, surface-layout
calculation 3.61% and mip selection 2.84%. These aggregate CPU categories do not
identify a material or provide wall-time attribution.

Evidence under `r3-20261008`: `fixed-bands-paired-before-a-report.json`,
`fixed-bands-paired-after-a-report.json`, `fixed-bands-paired-after-a-comparison.json`,
the 24 corresponding private-run directories with raw pixels/depth and all-thread
counts, and `profile-bands8-f9ff6bc0-q3dm1-cpu`.

## THE-862 active clip-plane checkpoint, 2026-10-08

Commit `3deeaa32` classifies each current clip plane using the original distance
arithmetic and fixed scratch. Accepted planes retain their vertex order without
copying; crossings retain the original graph and interpolation. Frozen old-loop
fixtures include earlier removal of an overflowing source with a finite polygon
surviving. Checker, workspace tests and tracked app compilation passed.

The portable release draw example built in 33.6783 s from a cleared cache.
The benchmark source is byte-identical to the preceding candidate. Twelve new
private runs use the same fixed scenes, resolution, profiles, 60/600 frame counts
and 1/2/4/8 affinity masks. No build, checker, debugger or profiler overlapped.

| Map | Bands | Before median / p99 ms | After median / p99 ms |
|---|---:|---:|---:|
| e1m1 | 1 | 2.0312 / 2.0547 | 2.0155 / 2.0387 |
| base1 | 1 | 4.5602 / 4.5923 | 4.5322 / 4.6707 |
| q3dm1 | 1 | 15.7264 / 16.4951 | 15.6306 / 16.6776 |
| e1m1 | 2 | 2.0479 / 2.1350 | 1.2477 / 2.1427 |
| base1 | 2 | 4.2325 / 4.7453 | 4.1706 / 4.7655 |
| q3dm1 | 2 | 13.7973 / 16.3213 | 13.5976 / 16.1838 |
| e1m1 | 4 | 1.2446 / 1.7895 | 0.8979 / 1.3886 |
| base1 | 4 | 2.5479 / 2.7882 | 2.5858 / 2.7589 |
| q3dm1 | 4 | 12.2644 / 13.6680 | 11.9819 / 13.4244 |
| e1m1 | 8 | 1.0103 / 1.7342 | 1.0132 / 1.8311 |
| base1 | 8 | 1.6407 / 2.1675 | 1.5656 / 1.9530 |
| q3dm1 | 8 | 8.9466 / 12.4617 | 8.2173 / 11.0006 |

All private exits and profile/candidate preservation/owned cleanup checks passed.
RGBA/depth bits, immutable workload and all aggregate/per-band counters match
the preceding candidate and across band counts. Measured caller plus worker
allocation/reallocation/requested-byte counts are zero. q3dm1 measured cache
activity remains 11,863,200 hits and zero fills/evictions/rejects.

q3dm1 at eight bands is 8.2173 ms median and 11.0006 ms p99, above the 4 ms
target. The four-band e1m1 row returns below the earlier baseline on this run;
the two-band row also varies strongly. Those changes need a repeated matched
check before attributing a large gain or qualification. This fixed-view developer
workload does not prove gameplay, native visual acceptance or installation.

Evidence under `r3-20261008`: `fixed-bands-clip-after-a-report.json`,
`fixed-bands-clip-after-a-comparison.json` and its twelve private-run directories.


## THE-862 cold mip layouts and ordered dispatch checkpoint, 2026-10-08

`6b6405bb` retains exact mip minima, dimensions and payload lengths in the
load-owned catalog. `40fc370c` batches consecutive prepared world draws within
each raster band, preserving draw rank, cache operations and barriers at
external draws and command boundaries. Both snapshots passed the rule checker,
workspace tests and tracked app compilation. Release draw builds used cleared
caches and byte-identical benchmark sources; builds took 33.0748 and 33.4530 s.

The following two sets of twelve private runs use the same fixed time-zero
retail scenes at 640×400, fresh copied profiles, 60 warm-up and 600 measured
draws, and the preceding 1/2/4/8 process affinity masks. No build, checker,
debugger or profiler overlapped timing.

| Map | Bands | Mip layouts median / p99 ms | Batched dispatch median / p99 ms | Median change |
|---|---:|---:|---:|---:|
| e1m1 | 1 | 1.9571 / 2.0127 | 1.9601 / 2.0007 | +0.15% |
| base1 | 1 | 4.4820 / 4.5935 | 4.5187 / 4.6057 | +0.82% |
| q3dm1 | 1 | 15.2329 / 17.4926 | 15.2469 / 16.7221 | +0.09% |
| e1m1 | 2 | 1.3450 / 2.0808 | 1.9561 / 2.1156 | +45.43% |
| base1 | 2 | 4.1379 / 4.6235 | 4.1272 / 4.6982 | -0.26% |
| q3dm1 | 2 | 13.2422 / 15.7149 | 15.2008 / 16.0418 | +14.79% |
| e1m1 | 4 | 0.8679 / 1.2245 | 0.8711 / 1.2276 | +0.36% |
| base1 | 4 | 2.5289 / 2.7168 | 2.4894 / 2.7155 | -1.56% |
| q3dm1 | 4 | 11.9990 / 13.0695 | 10.4994 / 12.5114 | -12.50% |
| e1m1 | 8 | 0.8216 / 1.1370 | 0.9943 / 1.6378 | +21.01% |
| base1 | 8 | 1.6455 / 2.6664 | 1.6344 / 2.1168 | -0.68% |
| q3dm1 | 8 | 8.5708 / 12.8273 | 7.3555 / 9.2227 | -14.18% |

Comparisons against the clip checkpoint and between these two sets passed:
raw RGBA and inverse-depth bytes, immutable workload, and all aggregate and
per-band raster/cache counters match. Every private exit, candidate/profile
preservation and owned-PID cleanup check passed. Measured caller plus worker
allocations, reallocations and requested bytes are zero. Native heap, scene
submission, presentation, gameplay and installation are outside this draw gate.

q3dm1 records 11,863,200 measured cache hits and zero fills, evictions or
rejects at every band count. At eight bands its cumulative counters are
13,067,484 hits, 1,808 fills and zero evictions/rejects. The total cache arena
is still 33,554,432 bytes. Shared mip metadata occupies 2,248,440 bytes for
e1m1, 3,394,960 for base1 and 4,573,400 for q3dm1, separate from that arena.

The latest q3dm1 eight-band median is 7.3555 ms and p99 is 9.2227 ms. The
4 ms target remains unmet. Mip metadata alone showed no consistent improvement;
batching regressed q3dm1 at two bands and e1m1 at two/eight bands. These
results are retained without qualification; repeated matched checks remain
necessary before attributing gains or accepting regressions. The earlier
`f9ff6bc0` sampling run included startup/loading and does not isolate the hot
frame window.

The owner's 14:15 priority update pauses CPU speed work here. Common R1
primitives and the event system precede further caching and unification work.
Live lighting, complete static-stage coverage and native visual acceptance
remain open.

Evidence under `r3-20261008`: `fixed-bands-mip-layout-after-a-report.json`,
`fixed-bands-mip-layout-after-a-comparison.json`,
`fixed-bands-ordered-dispatch-after-a-report.json`,
`fixed-bands-ordered-dispatch-after-a-comparison.json` and their 24 private
run directories.

## THE-625 shared linked-box trace checkpoint, 2026-10-08

The shared scene query reads linked collision columns directly from the entity
SoA and the existing area index. Caller data selects Q1/Q2/Q3 filtering, body
clipping and hit merging, independently of the map. SERVER excludes its moving
client, then commits and relinks that body before the next client. Current-command
prediction uses the same service. Q1 hull geometry is immutable; each caller
loads and reuses its own traversal scratch.

`tools/check_linked_merge.py` compares unchanged linked-hit statement blocks
from Q1 `SV_ClipToLinks` and Q2/Q3 `SV_ClipMoveToEntities`. All 396 rows per rule
matched every declared raw output word, including the solid flags and retained
contact identity. Each rule rejected a one-bit output mutation. The blocks
allocate zero Rust calling-thread bytes. This proves those statements, not
complete native `SV_Trace`, native ABI encoding or linked BSP/capsule geometry.

The unchanged convex-brush workload also passed all 10,070 rows per Q2/Q3
rule with zero Rust calling-thread allocations. The Q1 stack-box workload
completed 50,000 traces with zero allocations. Source review and focused
fixtures cover the distinct Q2 point-contents maximum face, Q2/Q3 pass semantics,
immediate world hits, and transformed-box endpoint/centering operation order.
Those fixtures do not replace native retail entity-trace comparisons.

Pinned primitive measurement uses baseline CPU code on core 23, without a
debugger. Sixty warm-up frames precede 600 measured frames. The new analytic
workload has 64 local/remote/bot clients with all five movement choices, 321
loaded brushes, shared linked-body queries, and authoritative/prediction state
comparisons in 64 rooms arranged 8 by 8.

| Workload | Median / p99 ns | Measured SERVER steps | Maximum Rust allocations / requested bytes |
|---|---:|---:|---:|
| `linked_scene_64_native_range_rooms` | 1,757,736 / 2,345,232 | 38,400 | 0 / 0 |

Authoritative and prediction position, velocity and movement state match on
every frame. No workers or SDL/driver operations enter this headless workload;
it proves the calling Rust thread only. Its initial linear room layout placed
Q2 clients outside the native signed eighth-unit coordinate range and failed
the state check after those origins wrapped into other rooms. The corrected
layout keeps native widths and physics unchanged. This workload differs from
the earlier analytic movement fixture, so it supplies no matched speedup or
regression claim. The measured release example rebuild took 6.74 s.

The immutable-hull rerun retained the same 10,000 seeded e1m1 segments per
native hull and matched all 30,000 original-C outputs. It allocated zero bytes
after load. Release build took 5.451 s, on core 23, with 600 warm-up traces and
600,000 timed calls per hull.

| Hull | Median / p99 ns per trace |
|---|---:|
| Point | 160 / 1,040 |
| Player | 160 / 730 |
| Large | 150 / 630 |

Muse's historical 229 microseconds is still not a matched baseline. No C-port
timing was taken. Evidence under `caller-linked-trace`: `merge/result.json`,
`brush/result.json`, `movement/verification.json` and `hulls/verification.json`.
These are developer examples, not a shipped or installed gameplay candidate.
Placed BSP bodies, capsules/patches, complete leaf-content and result adaptation,
retail module execution and the three-map installed acceptance remain open.

### THE-884: two physical intake points

The host collects SDL, stdin and UDP before SERVER and again before CLIENT.
Frame startup samples platform time without collection. The cap waits only on
platform time before the first intake. Its deadline uses integer-millisecond
timestamps and the native zero startup baseline. Both command phases and
same-frame loopback delivery remain in place. The owner ruling replaces the
previous cap-wait polling behavior.

The existing private SDL3 check now requires exactly two drains, rather than
accepting any count above two. The allocation-instrumented release binary and
host example built together in 29.29 seconds. Four host fixtures cover phase
order, cap aliases, startup/fractional deadlines and bot timing.

Core 23, no debugger, 60 warm-up frames followed by 600 measured frames:

| Workload | Median | p99 | Measured Rust allocations |
| --- | ---: | ---: | ---: |
| Headless host, UDP, repeated keys, native-rate counter providers and local packets | 5.80 us | 8.79 us | 0 |

This row measures the current host workload and has no matched before/after
baseline. It measures the calling thread; no worker is dispatched. It does not
measure SDL/driver allocation or renderer performance.

The copied candidate passed the owned Xvfb, headless sway and Weston runs.
Each ran 60 warm-up frames and 600 measured frames, with exactly two host
drains, one output drain, empty queues, stdin ingress and zero measured Rust
allocations. Xvfb delivered 26 real key repeats and sway delivered 17; held
movement and release checks passed. Weston exercised ConsoleLine input only.
All runs selected the private display driver, quit normally, preserved the
original profile and candidate, and left no owned processes running. The
allocation scope contains one calling thread and zero workers; native heap
allocation is unmeasured.

Evidence folder `two-physical-intakes-20261008`: `host.json` and
`private/verification.json`, with per-backend logs and reports. These are
window-shell and host checks, not installed three-map or gameplay acceptance.
No installation was performed.

### THE-709: native think scheduling checkpoint

The common dispatcher now has independent deadline/function columns and one
per-entity entry. Each owning module supplies its own native clock and loaded
scheduling rule. Q1 calls once with its float clamp; QW repeats due reschedules;
Q2 retains float times promoted for the double 0.001 tolerance; rerelease keeps
exact signed int64 milliseconds; Q3 compares integer times through float while
retaining its integer callback context. Function handles are u32, matching the
native function-index domain instead of imposing a 16-bit engine limit.

The original-source comparison runs complete unchanged SV_RunThink/G_RunThink
functions, with minimal type/callback fixtures. It also extracts the rerelease
gtime_t definition and both millisecond literals unchanged. Each row compares
100 raw u64 values for three entities, independent module clocks and explicit
caller order. This includes callback timestamps, deadline clearing, callback
persistence, finite reschedules, function changes and removal without slot reuse.

| Scheduling rule | Native/Rust rows matched | Dispatch allocation/reallocation count |
| --- | ---: | ---: |
| Q1 | 2,965 / 2,965 | 0 |
| QuakeWorld | 2,965 / 2,965 | 0 |
| Q2 classic | 2,690 / 2,690 | 0 |
| Q2 rerelease | 2,290 / 2,290 | 0 |
| Q3 / Team Arena | 2,480 / 2,480 | 0 |

Every rule passed the allocator positive control and an identical-record
comparison; a one-bit timestamp mutation was rejected by the same comparator.
The separate 400-entity mixed-rule loop made 4,000,000 callbacks over 10,000
iterations with zero counted Rust allocations after loading. No wall-time or
performance improvement is claimed for these checks.

Evidence folder `native-thinks-20261008`: `comparison/report.json`, original
source spans, fixture/raw-output files and compiler logs; separate
`dispatch-allocations.json`. The allocation scope is the calling thread after
cold setup. Native null-callback fatal errors are caught by the reference
fixture and compared with the required scoped Rust rejection. Free teardown
is normalized. Caller-order fixtures are not full native frame traversal.
Module VM execution, touch/use ABI completion, physics-phase integration,
live monsters and installed acceptance remain open; no install was performed.

## THE-656 client arena checkpoint (2026-10-08)

The common server now sizes one client array at load. Internal client handles
are u32, with native capacity and field widths applied at module/protocol
boundaries. Native entity identities are explicit connection data; the common
world/client reservation does not assign Q3 or Q2 entity numbers.

`client_state` release example build: 14.38 s. The headless probe loaded 512
local/remote/bot clients, exercised client 511 disconnect/reconnect and command
building for 60 warm-up plus 600 measured iterations, and counted zero Rust
calling-thread allocation/reallocation calls and requested bytes. The positive
allocation control counted one call. Evidence: `native-client-state-20261008`
`allocation.json`. Native Q2/RR capacity 256 and internal IDs above 255 are
covered by client-state checks; HUD routing covers IDs 64, 255 and 511 with a
duplicate local-seat binding.

This is state/allocation evidence, without elapsed-time performance or worker
measurements. Native protocol encoding, guest module binding, installed
independent-seat gameplay and the three-map gate remain open. No install.

## THE-617 exact-name checkpoint (2026-10-08)

One name arena now preserves exact case and raw bytes. A load-built numeric
equivalence table serves folded callers. The shared target index selects the
caller's matching policy, preserves source slot order and distinguishes absent
fields from explicit empty names. The item registry's classname lookup uses
exact IDs.

Release target/lifecycle example build: 7.94 s. Unchanged Q1/QW PF_Find and
Q2/Q3 G_Find fixtures matched all 12,712 rows (Q1 3056, QW 3056, Q2 3280,
Q3 3320), including ordered slots, empty/NULL fields, raw non-UTF8 names,
inactive entities and starting positions. Each rule counted zero Rust
allocation/reallocation calls after cold load; positive allocation controls
counted one, identical comparator controls passed and one-bit slot-result
mutations were rejected. Evidence: `native-target-names-20261008`
`comparison/report.json`. This is behavior/allocation evidence, not a timing.

The fixture uses equal native/common slot numbers and minimal field adapters,
without VM string/entity ABI execution. It compiles Q1's unchanged default
branch; Q2 uses the unchanged POSIX comparator in C locale, without Windows
coverage. Q1/QW NULL-query fatal behavior is caught and compared with scoped
boundary rejection; Q2 NULL queries and embedded NUL strings are excluded.
Dynamic index refresh/lifecycle allocation proof is recorded separately.
The lifecycle probe's final rebuild took 5.81 s. It performed 2,560,000 entity
allocations over 10,000 cycles, with changed target indexes and area links,
counting zero Rust allocation/reallocation calls after cold load and one in
its positive control (`entity-allocations.json`). No worker was created.
Native module namespace mapping, retail trigger/door execution and installed
combined gameplay remain open. No install.

## THE-2875 liveness and named-change checkpoint (2026-10-08)

Entity liveness now has one bitset, including reserved and generation-retired
slots. Ascending think dispatch re-reads that bitset after callbacks. One target
index consumes coalesced named changes and updates its sorted rows individually;
unnamed allocation/release does not rebuild or sort the index.

The matched baseline is `70dd962b`. Both archived source trees built the same
`entity_tables` release probe with baseline CPU code and allocation tracking.
Core 23 ran ABBA at each capacity, without a debugger, with 60 warm-up and 600
measured frames per run. Each frame scheduled 64 sparse Q2-rule callbacks,
allocated/released one unnamed entity, refreshed exact/folded target lookup,
and checked callback order, generation handles and raw timestamp bits.

Each table cell lists the two runs' median/p99 in ns, in execution order within
that revision. Total includes unnamed churn, scheduling, dispatch and fidelity
checks; dispatch includes its timer reads.

| Capacity | Before dispatch | After dispatch | Before total | After total |
| ---: | --- | --- | --- | --- |
| 128 | 520/570; 520/900 | 560/560; 550/600 | 2,060/2,120; 2,080/3,300 | 750/810; 750/800 |
| 1,024 | 850/1,230; 910/960 | 560/600; 560/940 | 3,230/5,390; 3,280/3,350 | 760/810; 790/1,320 |
| 8,192 | 3,600/5,910; 3,560/3,600 | 570/590; 570/630 | 12,940/22,020; 12,810/17,780 | 800/850; 800/850 |

The mean of the two dispatch medians increased 6.7% at capacity 128 and fell
36.4%/84.1% at 1,024/8,192. Each run checked 38,400 ordered callbacks with zero
fidelity mismatches and zero calling-thread Rust allocation/reallocation calls
and requested bytes; the positive control counted one allocation. Baseline
unnamed refreshes were 1,200 per run; the new index counted zero. The bit walk
still examines empty words; this is not a claim of capacity-independent cost.

Fresh original-C comparisons matched all 12,712 target rows, 13,390 per-entity
think rows and 27,367 lifetime rows. Target/think comparator mutation controls
passed.
The separate lifecycle workload performed 2,560,000 allocations and 2,580,000
area relinks with zero hot Rust allocation calls and one positive-control call.
Release probe builds took 18.06 s before and 17.69 s after; the latter tree's
target, think and lifetime builds took 7.34 s, 5.50 s and 6.71 s respectively.

Evidence: `THE-2875-entities/timings.json`, `proof-build.json`,
`targets/report.json`, `thinks/report.json`, `lifetimes/verification.json` and
`allocations.log`. A cached baseline that wrongly showed new-index behavior
was rejected under `invalid-artifact-reuse`; refreshed archive source timestamps
and old/new behavior guards prevent that reuse in the recorded comparison.
These developer fixtures do not execute native module ABIs, retail rocket/nail
spawning or installed think/touch ordering. No workers or foreign heap were
measured. THE-2875's e1m1/q3dm1 live acceptance remains open; no install.

```sh
timeout 300 cargo build --release -p qa-platform --example entity_tables \
  --features allocation-tracking
timeout 300 taskset -c "$CORE" target/release/examples/entity_tables 8192
```


## THE-892 shared membership marks (2026-10-08)

One core `StampSet` now supplies collision brush visits, per-view visibility,
GL sky membership and surface-cache batch pins. The former four epoch-reset
implementations were deleted. Forced rollover, independent sets, repeated
visibility queries and cache metadata recycling have behavioral checks.

Initial headless comparisons used parent `1da98fe0` and the staged StampSet
slice. Both had the same pre-THE-2882 retail Q2 admission defect; these rows
cover valid synthetic geometry and visibility, not Q2 app admission. Release
builds used baseline CPU, core 23, no debugger, 60 warm-up batches and 600
measured batches in ABBA order. Each trace batch contains 64 trace/point pairs.
Each visibility batch contains 96 queries over e1m1, base1 and q3dm1.

| Headless workload | Before medians (ns/batch) | After medians (ns/batch) | Before p99 (ns/batch) | After p99 (ns/batch) |
| --- | --- | --- | --- | --- |
| Q2 brush trace + point | 18,875 / 18,840.5 | 19,090 / 19,025 | 158,520 / 155,120 | 156,710 / 156,451 |
| Q3 brush trace + point | 6,930 / 6,955 | 6,965 / 6,985 | 91,160 / 88,750 | 90,110 / 90,100 |
| Three-map visibility | 11,333,463.5 / 11,321,159 | 11,173,108 / 11,103,038 | 12,125,539 / 12,087,738 | 11,923,739 / 11,867,779 |

Each native rule matches all 15,045 original-C rows, with identical contacts
and enclosed results across timing runs. Each run measures 38,400 trace/point
pairs without Rust allocation/reallocation/requested bytes; the positive
control reports one allocation. Visibility checks raw PVS data, point/leaf
selection and face order through a separate Rust oracle over decoded native
data. Each run covers 57,600 measured queries with zero calling-thread Rust
allocations. It does not execute an extracted C visibility implementation.

The first visibility measurement overlapped developer compilation and its p99
is excluded. The displayed visibility rows repeat the complete ABBA workload
after compilation and checks finished. Evidence: developer cache
`THE-892-shared-stamps/headless-comparison.json`,
`visibility-timings-final.json`, and corresponding raw reports. The checker
sweep accepted 18 permitted fixtures and rejected 62 prohibited fixtures before
Cargo, including renamed epoch implementations and duplicate primitive types;
`rule-fixtures-final/result.json` records all 80 cases. These pattern checks
do not prove arbitrary Rust name resolution.

Private CPU/GL comparisons use a separate evidence root,
`THE-892-shared-stamps-q2-fixed`, with the signed axial fix from `c23d0182` in
both candidates. Results follow.
No headless row qualifies gameplay, installation, native heap use or the
renderer performance targets.


Both private comparison candidates include `c23d0182`; the baseline was built
from that commit in 40.89 s and the StampSet engine source from staged tree
`7af1284b` in 39.07 s. Both use allocation tracking without proof input. Critical
core/world/render sources were freshly compiled rather than reused from a
different archive. Final documentation does not change those compiled sources.

The fixed-camera CPU draw probe uses 640x400, one band, core 23, vsync off,
60 warm-up and 600 measured frames. The four runs per map use ABBA order.

| Static CPU draw | Before medians (ms) | After medians (ms) | Before p99 (ms) | After p99 (ms) |
| --- | --- | --- | --- | --- |
| e1m1 | 1.963 / 1.977 | 2.041 / 2.024 | 2.012 / 2.036 | 2.145 / 2.126 |
| base1 | 4.535 / 4.517 | 4.585 / 4.555 | 5.467 / 5.186 | 5.402 / 5.166 |
| q3dm1 | 16.365 / 16.116 | 16.176 / 16.335 | 17.915 / 17.325 | 18.830 / 18.627 |

Compared by the mean of each pair of run medians, changes are +3.16%, +0.97%
and +0.09%; corresponding p99 changes are +5.53%, -0.80% and +6.29%. Every
run has byte-identical raw RGBA/depth, workload and cache statistics, with
zero measured calling-thread Rust allocations/reallocations/requested bytes.
This is a static draw probe, not the full frame or the native gameplay gate.
Base1 and q3dm1 exceed the owner's 4 ms target even in this bounded workload;
q3dm1 speed work remains paused.

Six private GL host runs use 60 warm-up and 600 measured frames. Before/after
initial fixed-time screenshots match exactly on e1m1/base1/q3dm1, as do scene
metadata and driver identity. The all-instrumented Rust-thread allocation gate
passes with zero measured heap work in every run. Native SDL/driver allocation
is outside that counter. GL driver: llvmpipe (LLVM 22.1.8, 256 bits), OpenGL
4.6 Core, Mesa 26.2.2-arch1.1: **software GL**. Later animation uses host time,
so these runs do not supply a matched GL performance comparison.

All 18 private runs preserve the original profile and copied candidate, quit
normally and leave no owned PIDs. CPU probes locate their copied profile but
do not prove the app's settings semantics; GL host runs consume copied saved
settings. Exact output comparisons prove this refactor's bounded fidelity,
not complete original-game visuals, gameplay, installation or performance.
Evidence: `THE-892-shared-stamps-q2-fixed/private-comparison.json` and each
run's `result.json`, `runtime.log`, pixel/depth files and window capture.


## THE-2868 / THE-2883 shared collision models (2026-10-08)

The geometry owner is one `CollisionStore` with a flat model table and typed
resource generation handles. Every query selects its resource and native model
ordinal explicitly. Hull/brush kernels no longer retain duplicate model tables;
all application, session and developer callers use the store. Linked models
use entity-role rotation/link rules independently of the query's trace rules.
An optional published point-contents pose preserves Q3's distinct `s.origin` /
`s.angles` versus `r.currentOrigin` / `r.currentAngles`; ordinary bodies use
their physical pose. These internal values do not alter any wire layout.

The baseline is `08ad0eb4`; the staged engine tree is `075b85ed`.
Fresh baseline-CPU release builds took 37.04 s and 41.64 s respectively,
including the allocation-tracked app and developer probes. Proof input is
absent. The final documentation does not change those compiled Rust sources.
Exact staged workspace/all-target tests and the allocation-tracked app check
pass. Six retail tests cover signed plane admission, every native model bound,
nonzero e1m1/base1/q3dm7 model contacts, and resource removal/compaction/reload.
Q3 q3dm1 has one model; q3dm7 supplies the nonzero retail ordinal.
The rule sweep rejects 70 prohibited fixtures and accepts 18 permitted cases.

`tools/check_model_collision.py` extracts unchanged original C transformed
wrappers and their dependencies. The production store matches 10,432 Q1 rows
(9 words each) and 15,100 rows per Q2/Q3 rule (13 words each), bit for bit.
Cases include native model ordinals, Q1 hull offsets/width thresholds,
asymmetric boxes, compound rotations, large-coordinate rounding and Q3's
native double centering. Q1 ignores model angles, Q2 restores normals through
negated angles, and Q3 uses the native transpose basis. Identical-row controls
pass; one-bit fraction and flags/point mutations fail as expected. These
controls prove comparator sensitivity, not sensitivity to every possible
implementation mutation. The full brush runs include actual sloped contacts;
the appended cases named nonaxial-contact alone do not establish that coverage.
No Rust calling-thread allocations, reallocations or requested bytes occur
inside complete row evaluation; each allocation positive control reports one.

The transformed timing probe uses core 23, baseline CPU, no debugger,
60 warm-up and 600 measured batches of 64 queries. Q1 times traces only;
Q2/Q3 time trace plus point contents. Samples include raw result comparisons.

| Transformed synthetic workload | Median (ns/batch) | p99 (ns/batch) | Median (ns/query) | Batch p99 / 64 (ns) |
| --- | --- | --- | --- | --- |
| Q1 hull trace | 3,880 | 4,150 | 60.625 | 64.844 |
| Q2 brush trace + point | 21,530 | 142,440 | 336.406 | 2,225.625 |
| Q3 brush trace + point | 9,635 | 79,930 | 150.547 | 1,248.906 |

All timed samples retain native results and zero measured Rust heap work.
There is no matching pre-transform baseline; these new rows establish a
bounded measurement, rather than qualify a regression comparison. They do
not measure native C heap, workers, retail loading, linked-scene merge/filter
semantics, guest ABIs, temporary boxes, Q1 point contents, patches or capsules.

The existing world-query workloads run ABBA on the same core and candidates.
Q1 compares all 30,000 original-C e1m1 rows, then times 600,000 traces per hull
after 600 warm-up traces over the same 10,000 seeds. Brush runs compare all
15,045 native rows per rule, then use 600+60 batches of 64 trace/point pairs.
Contacts/enclosed counts and raw results match throughout; all runs have zero
measured Rust heap work.

| Existing world workload | Before medians | After medians | Before p99 | After p99 |
| --- | --- | --- | --- | --- |
| Q1 point (ns/trace) | 160 / 160 | 170 / 170 | 1,060 / 1,060 | 1,060 / 1,060 |
| Q1 player (ns/trace) | 160 / 160 | 160 / 160 | 730 / 740 | 750 / 750 |
| Q1 large (ns/trace) | 150 / 150 | 150 / 150 | 630 / 640 | 640 / 640 |
| Q2 brush pair (ns/batch) | 19,085 / 19,185 | 18,985 / 19,050 | 157,200 / 156,900 | 161,780 / 156,300 |
| Q3 brush pair (ns/batch) | 7,425 / 7,350 | 7,260 / 7,435 | 89,440 / 89,570 | 86,440 / 86,570 |

The largest change in paired mean medians is Q1 point +6.25%, within 10%.
Headless evidence lives in developer cache `THE-2868-inline-geometry`:
`native-models/report.json`, `native-models-timed/report.json`,
`headless-abba/result.json`, `rule-fixtures/result.json`, and their saved
C sources, raw native/Rust rows, timing reports and mutation controls.
Live e1m1 doors/platforms blocking and pushing, guest pose ABI wiring and
installed gameplay acceptance remain open on THE-2868/THE-2883.


Private rendering checks use copied candidates and profiles, forced X11 on
owned displays and disk audio sinks. CPU runs use one band at 640x400, core 23,
vsync off, 60 warm-up and 600 measured static frames, in ABBA order per map.

| Static CPU draw | Before medians (ms) | After medians (ms) | Before p99 (ms) | After p99 (ms) |
| --- | --- | --- | --- | --- |
| e1m1 | 2.032 / 2.025 | 2.020 / 2.052 | 2.393 / 2.356 | 2.378 / 2.406 |
| base1 | 4.527 / 4.533 | 4.530 / 4.504 | 4.888 / 4.852 | 5.068 / 4.822 |
| q3dm1 | 15.326 / 15.343 | 15.259 / 15.245 | 15.920 / 16.210 | 15.905 / 16.449 |

Pair-mean median changes are +0.38%, -0.28% and -0.54%; pair-mean p99
changes are +0.75%, +1.54% and +0.70%. All twelve CPU runs have identical raw
RGBA/depth, workload and cache statistics, with zero measured calling-thread
Rust allocations/reallocations/requested bytes. One band creates no workers.
Base1 and q3dm1 remain above the 4 ms target in this bounded static draw;
q3dm1 speed work remains paused by the owner's core order.

Six GL host runs use 600+60 frames with the same copied saved settings.
Initial fixed-time world screenshots, scene metadata and driver identity
match exactly before/after for each map. The instrumented Rust allocation
gate passes with zero measured heap work; these runs have one calling thread
and no worker threads. Native SDL/driver heap is not measured. Driver:
llvmpipe (LLVM 22.1.8, 256 bits), OpenGL 4.6 Core, Mesa 26.2.2-arch1.1:
**software GL**. Host-driven animation lacks a matched time sequence, so no
GL performance comparison is claimed.

All eighteen runs quit normally, preserve the owner's original profile and
copied candidate, and leave no owned PIDs. CPU probes locate the copied
profile; GL hosts consume the saved settings. These runs establish bounded
rendering fidelity, not full original presentation, walks, gameplay or mover
integration. No new binary was installed; the separate render preview remains
`08ad0eb4`. Evidence: `THE-2868-inline-geometry/private-comparison.json` and
per-run `result.json`, `runtime.log`, raw RGBA/depth and window captures.

## THE-2884: shared rule identity

Baseline `ad3369bf` and the canonical `RuleSetId` source snapshot use portable
release builds with allocation tracking and no proof input. Core, session and
console recompile in both snapshots; build times are 43.014 s and 41.989 s.
This slice deletes the separate movement, console-source and think-timing
identity enums, preserving their five source columns and arithmetic.
Module tick/link selection and independent player trace roles remain the next
integration step, rather than evidence supplied by the type replacement.

The following headless workloads run ABBA on core 23, without a debugger,
with 60 warm-up and 600 measured frames. The host uses a deterministic event
time sequence, 64 bots, binds, console commands, local packets and shared
sound/effect/print output. Both fixtures report identical counters and zero
measured calling-thread Rust allocations/reallocations/requested bytes.
Authoritative and prediction states match within every movement run.

| Workload | Before median (ns) | After median (ns) | Before p99 (ns) | After p99 (ns) |
| --- | --- | --- | --- | --- |
| 64-client mixed-rule movement | 2,515,197 / 2,519,187 | 2,535,342 / 2,542,632 | 3,364,702 / 3,418,343 | 3,391,903 / 3,403,063 |
| Common host with console/binds/bots/output | 162,150 / 161,460 | 162,140 / 161,705 | 251,470 / 252,610 | 244,070 / 255,650 |

Pair-mean median changes are +0.866% and +0.073%; p99 changes are +0.176%
and -0.865%. This is a matched structural comparison, not gameplay or renderer
performance qualification. Numeric think dispatch also matches before/after:
4,000,000 callbacks over 10,000 frames with zero measured allocations.

Original think functions match 13,390 raw rows across Q1/QW/Q2/Q2RR/Q3,
with zero calling-thread heap work and a positive allocation control.
Original token/separator functions match 10,060 records; Q2 rerelease uses
the shared classic parser. Analytic movement compares 1,152 states per game:
Q2 is exact; Q3's maximum position/velocity error is 0.0000112 with exact
integer flags/timers. The complete Rust movement rows are byte-identical
between the baseline and candidate. These are scoped native-function checks,
not complete native frames, retail walks, guest ABIs or legacy network proof.

The C-port numeric helper matches all 100,080 records. All 344,250 cvar-view
results are byte-identical before/after, but that C comparison initially fails
on three QW skin writes. THE-2885 records the pre-existing developer reference
bug: it omits `Output.prefix`; the production cvar path already writes it.
The failed comparison and raw baseline/candidate outputs are retained.
Do not treat the initial cvar-view oracle as passing.

Rule checks pass, including 97 fixtures: 79 rejected before compilation and
18 permitted controls. Evidence is in developer cache
`THE-2884-rule-identity`: `headless-abba/result.json`, `native-thinks/report.json`,
`native-commands/comparison.json`, `native-movement/result.json`,
`cvar-views/before-after.json` and `rule-fixtures/result.json`.

Twelve successful private normal-candidate runs cover e1m1, base1 and q3dm1,
CPU and GL, before/after, at 640x400 with 600 measured frames after 60 warm-up.
Initial world captures and scene metadata are identical. Each run consumes a
fresh copy of saved settings, quits normally and preserves the original
profile and candidate; every recorded PID is cleaned up. CPU uses one band.
Rust allocation gates count one calling thread and no workers and report
zero allocations, reallocations and requested bytes. SDL/driver heap is not
measured. GL is **software GL**, llvmpipe LLVM 22.1.8, 256 bits, Mesa
26.2.2-arch1.1, OpenGL 4.6 Core. No matched animated host/GL timing claim is
made. These runs do not qualify gameplay, live role composition, modules,
network, a complete original image or installation.

One earlier q3dm1 CPU baseline attempt failed before launching the game when
Xvfb could not finish its display-number write. Its logs and cleanup results
are retained; a fresh retry succeeded. THE-2886 records the harness's
single-read displayfd bug. Private evidence: `private-comparison.json` and
per-run `result.json`, `runtime.log` and `window.png`. The installed preview
remains `08ad0eb4`; neither primitive slice replaces it.

THE-2885 then corrects only the developer reference's write serialization:
the length and bytes include both prefix and suffix, as the production cvar
path already does. The full C-port helper comparison now passes all 100,080
numeric cases and 344,250 conversion cases, including all three QW skin rows.
Evidence: developer cache `THE-2885-cvar-prefix/comparison.json`, saved C helper
source, fixture bytes, native/Rust output and build logs. This does not prove
live userinfo or legacy protocol interoperability.

THE-2886/THE-2887 fix the developer display startup: consume the whole
newline-terminated Xvfb reply and close both pipe ends on every spawn/read
failure. All 21 tool tests pass, including split digits/newline, EOF,
malformed/oversized replies, timeout and a mocked spawn failure. The focused
fixtures reject both the old single-read behavior and the old descriptor leak;
all deliberately opened control descriptors are cleaned up.

Real private q3dm1 CPU and GL runs finish 600+60 frames, quit normally,
preserve candidate and original saved settings, and leave no owned PIDs.
A final GL run also verifies the amended startup error handling. The same
41.989 s normal engine build supplies these checks; no engine rebuild or
installation is involved. Instrumented Rust heap counts are zero; GL remains
Mesa llvmpipe software rendering. These validate developer harness behavior,
not gameplay, performance targets or native SDL/driver allocation. Evidence:
`THE-2886-complete-display/report.json`, `unit-tests-final.log`,
`comparator-controls.json` and owned per-run logs/captures.


## THE-625 / THE-891: independent player trace rules

Each player now carries separate movement and trace `RuleSetId` values.
The one movement entry resolves every clipping and entity-filtering probe from
that explicit trace id; prediction copies it with the hot state. The app's
`--trace-rules` selects it independently. Native presets retain their previous
pairing at the current command-line boundary. Client-module tick/link policy
is the next slice; this does not complete native module integration.

The exact tested source tree is `334e1ea962f4e6ec3268e13044bb42555000042a`.
Portable release builds against `002f78d7` took 39.055 s before and 39.140 s
after, with fat LTO, one codegen unit, allocation tracking and no proof input.
Core, movement and session were rebuilt for each copied source snapshot.
Workspace all-target tests and the app allocation-feature check pass. All
100 checker fixtures pass: 82 forbidden cases rejected before Cargo and
18 permitted cases admitted, including rejection of movement-derived trace
selection. The 25 movement/trace combinations use a separate native-policy
oracle; a session fixture distinguishes Q2's 1/32 and Q3's 1/8 contact offsets
on Q2 geometry while retaining Q3 physics and copying the trace id to prediction.

Original Q2/Q3 Pmove comparisons pass 1,152 analytic states each. Q2 is exact.
Q3 integer flags/timers are exact; its maximum float difference is 0.0000112
units across 544 differing components, unchanged from the earlier comparison.
Raw Rust state rows are byte-identical before/after for both native presets.
These checks do not prove all movement modes or module/wire compatibility.

Pinned core 23, ABBA order, 60 warm-up plus 600 measured frames, no debugger:

| Workload | Before medians (us) | After medians (us) | Median change | Before p99 (us) | After p99 (us) |
|---|---|---|---:|---|---|
| 64 mixed-rule clients, SERVER movement and prediction | 2925.502 / 2524.536 | 2717.206 / 2562.951 | -3.12% | 5171.533 / 3406.823 | 5062.713 / 3521.952 |
| Host console/binds, 64 bots, local commands and outputs | 162.035 / 163.400 | 160.620 / 161.060 | -1.15% | 262.870 / 253.440 | 252.830 / 253.711 |

Changes compare the mean of each pair of medians. The variability in the
movement rows is retained; this is a regression check, not an optimization
claim. Reported workload/fidelity counters match and measured calling-thread
Rust allocations/requested bytes are zero. SERVER/prediction states match
within each movement run. A separate 4,000,000-callback dispatch run retains
its counters and zero allocation after load. These synthetic workloads do
not qualify renderer targets, installed gameplay or native heap behavior.

Twelve private normal-candidate runs compare before/after e1m1, base1 and
q3dm1 on CPU and GL at 640x400, with 600+60 frames, copied saved settings,
fixed initial captures and metadata, normal quit and no remaining owned PIDs.
Captures/scene metadata match; measured Rust heap work is zero (one calling
thread, no workers). GL is Mesa llvmpipe software rendering, version 4.6 Core
Profile, Mesa 26.2.2-arch1.1, LLVM 22.1.8. No SDL/driver heap measurement or
animated-frame performance comparison is claimed.

A separate private base1 CPU run selects Q3 movement and Q2 trace rules,
uses 24 actual X11 key repeats, observes displacement, finishes 180 measured
frames after 60 warm-up frames and quits normally with zero counted Rust
allocations. Its first attempt used the unsupported test option `--set`;
that pre-window failure is retained, with preserved profile/candidate and
complete owned-PID cleanup. The corrected `+set` run passes. This is bounded
command-line/routing evidence, not the three-game gameplay gate.

Evidence is in developer cache `THE-625-trace-role`: source archives/build
reports; `trace-role-tests.log`; `rule-fixtures/result.json`;
`native-movement/result.json`; `headless-abba/result.json` and raw state rows;
`private-comparison.json`; `private-override.json`; and per-run runtime logs,
results and window captures. No installation replaces the existing preview.
Native references: Q2 `qcommon/cmodel.c` DIST_EPSILON and `server/sv_world.c`
SV_Trace/SV_PointContents; Q3 `qcommon/cm_local.h` SURFACE_CLIP_EPSILON and
`server/sv_world.c` SV_Trace/SV_PointContents. No Muse code was used.

## THE-884 / THE-650 / THE-2888 / THE-2889: explicit client policy

The built-in walk-through client now selects tick rate and first-link order
from its own `RuleSetId`, independently of BSP syntax, movement and trace ids.
`--client-module` supplies that preset; stock defaults use the winning mount's
existing product metadata. Unknown or ambiguous roots require an explicit
choice. Movement and trace defaults each follow the client independently.
The first link uses the resolved order; the old corrective second app link
is deleted. This is policy for the built-in gate, not a loaded guest module.

Q3 reads its cached sv_fps handle through the selected client view. Its original
below-one write sets sv_fps to 10 and yields 100 ms, preserving the active console
dialect. Native references: Q3 `server/sv_main.c:772-775` SV_Frame;
Q1 `WinQuake/world.c` SV_LinkEdict; Q2 `server/sv_world.c` SV_LinkEdict;
Q3 `server/sv_world.c` SV_LinkEntity. QW native clock clamps, loaded provider
clocks and live rate changes remain integration work. The existing one-ms
minimum above 1000 Hz is retained, not claimed as original Q3 behavior.

Saved settings use actual product-root and edition metadata. Retail reader
fixtures put Q1, Q2 and Q3 BSP syntax into foreign stock roots and retain the
root's correct settings key, including q1/rerelease/id1 and
q2/rerelease/baseq2. Nested custom roots require an explicit client and use
its settings namespace. The redundant settings key on LoadedMap is deleted.

Exact tested source tree: `5922fb3bf7ec1c8d014daae1763947521440b8ff`.
Portable allocation-instrumented release builds took 38.007 s at `9f906b21`
and 38.314 s after. Both use the shared target directory, fat LTO, one codegen
unit and no proof input; session, content, gameplay and app were recompiled.
Workspace all-target tests, the app allocation-feature check and the ignored
retail settings fixture pass. All 103 checker fixtures pass: 85 forbidden
cases rejected before Cargo, 18 allowed cases admitted. The new checks cover
the retired app map-derived policy patterns, not every possible semantic
bypass. Focused fixtures verify client/role choices, the first area-list order,
prediction copies, Q3 rate repair and actual product-root selection.

The first core-23 ABBA host comparison fails the 10% median guard: +30.50%.
Its own baseline changes from 170.755 to 279.125 us (+63.47%). All failed rows
are retained. Concurrent activity was observed, without proving its effect.
Four complete additional ABBA cycles establish a stable comparison, with no
other root build/check during measurement. Each run has 60 warm-up and 600
measured frames, identical reported workload/fidelity and zero calling-thread
Rust allocations/requested bytes, without a debugger:

| Synthetic workload | Before mean median (us) | After mean median (us) | Median change | Before mean p99 (us) | After mean p99 (us) | p99 change |
|---|---:|---:|---:|---:|---:|---:|
| 64 mixed-rule clients, SERVER movement and prediction | 2531.412 | 2556.382 | +0.99% | 3432.490 | 3475.875 | +1.26% |
| Host console/binds, bots, local commands and outputs | 160.946 | 161.291 | +0.21% | 251.303 | 253.008 | +0.68% |

Each summary averages eight medians or eight p99 values; raw runs remain in
the evidence. These are regression comparisons, not gameplay or renderer
qualification. A separate 4,000,000-callback dispatch has identical counters
and zero allocations after load. Q2/Q3 analytic movement outputs retain all
1,152 state rows byte-for-byte before/after and match the earlier original-C
comparison outputs; no new original-C compilation is claimed for this slice.

Twelve private normal-candidate runs compare stock e1m1, base1 and q3dm1 on
CPU and GL at 640x400, 600 measured plus 60 warm-up frames. Fixed initial
pixels and scene/driver metadata match; copied saved settings are consumed;
normal quit, original-profile/candidate preservation and owned-PID cleanup
pass. Counted Rust allocations/reallocations/requested bytes are zero for
one calling thread and no workers. GL is Mesa llvmpipe software rendering,
LLVM 22.1.8, OpenGL 4.6 Core Profile, Mesa 26.2.2-arch1.1. Native SDL/driver
heap and animated-frame performance are unmeasured.

Eight additional private CPU cases finish 180+60 frames at the explicit
85 fps cap. Q3 client policy on Q2 geometry, with a Q2 console dialect, yields
50 ms at its default 20 Hz, 25 ms at 40 Hz, and 100 ms at zero/negative input.
Foreign movement retains the client trace preset unless explicitly changed.
Q2 rerelease policy on Q1 geometry yields 25 ms/tail order; Q1 policy on Q3
geometry stays frame-driven/tail order. All 240 observed frame diagnostics
per case report two intakes; world tick counts match the selected timeline.
One case observes 24 actual X11 key repeats. Rust allocation counts are zero,
normal quit and profile/candidate/PID containment pass. The Q3-on-Q2 cases
have no matching saved input files, so do not prove imported preferences.

A native Q2 rerelease base1 attempt imports the correct 39 cvars/46 bindings
but fails before window readiness: `env/unit1_rt.pcx` is absent although its
TGA exists. The current indexed sky resource requires PCX before its TGA
alternate. THE-2890 records this existing material-path defect; the failed
run is retained and not counted as a successful rerelease render. A separately
labelled owned rerelease-root fixture with classic retail assets imports those
settings and quits normally. Native Q1 rerelease e1m1 renders and quits, but
there are no matching saved settings files, so its settings namespace is
proved only by reader fixtures. Full native presentation/gameplay is pending.

Evidence: developer cache `THE-884-client-policy`, source/build reports,
`client-policy-frozen-tests.log`, `client-profile-retail.log`,
`rule-fixtures/result.json`, `measurement-retry.json`, failed `headless-abba`
rows, `headless-stable-abba/result.json`, `private-comparison.json`,
`private-policy-cases.json` and per-run captures/logs/results. No installation
replaces the existing preview or qualifies gameplay. Module phases/touch logs,
damage rules, wire behavior, multi-seat settings and the full three-game gate
remain open. No Muse code was used.

The final CI sweep finds pre-existing workspace rustfmt differences in six
untouched files and four Clippy errors in unchanged world functions. The
changed Rust files pass formatting. THE-2891 tracks the separate CI repair;
neither workspace formatting nor Clippy is reported as passing for this tree.

## THE-2891: restore local CI checks without runtime changes

The separate repair applies rustfmt and 84 item-scoped Clippy expectations
across 45 Rust files. Expectations document deliberate native formulas,
explicit source indices and bounds, packed data, flat measured ownership,
independent inputs and oracle failure/lifetime controls. They introduce no
blanket crate/module suppression. Removing only those lint attributes and
normalizing both revisions with rustfmt reproduces the entire previous source,
including literal text, byte-for-byte against `cc5405cb`. Executable statements,
types, layouts, APIs and test oracles remain unchanged.

Local required checks pass: workspace formatting, all-target Clippy with
warnings denied, all-target workspace tests, 21 developer tool tests, build
rule/catalog freshness checks, all 103 rule fixtures and the app allocation
feature check. Incremental Clippy finishes in 0.14 s; test compilation finishes
in 3.72 s. The earlier compiled catalog comparison matches all 26,460 metadata
cells across 1,260 untouched owner rows. These are developer/compilation checks,
not a new release build, private binary run, installation or performance claim.

Evidence: developer cache `THE-2891-ci/source-equivalence.json`, normalized
comparison helper, per-check logs, `rule-fixtures/result.json` and the catalog
comparison from `THE-884-client-policy/catalog-cells`. Prior failed Clippy logs
are retained. THE-2893 records that the GitHub workflow still declares SDL2
while platform links SDL3. No clean Ubuntu runner was tested here; local checks
do not qualify remote CI dependency provisioning. Core work keeps its priority.

THE-2872/THE-2865 measured the caller-participating dispatcher and automatic
bands on engine commit `5862373d`, baseline CPU, with process affinity to eight
physical cores (4–11), no debugger, 60 warm-up and 600 measured frames. Fixed
retail CPU draw rows at 640x400 were:

| Map | Bands | Draw median ms | Draw p99 ms |
| --- | ---: | ---: | ---: |
| e1m1 | 1 | 1.987 | 2.057 |
| e1m1 | 2 | 1.198 | 1.512 |
| e1m1 | 4 | 0.874 | 0.963 |
| e1m1 | 8 | 0.796 | 1.300 |
| base1 | 1 | 4.455 | 4.642 |
| base1 | 2 | 2.498 | 2.725 |
| base1 | 4 | 1.541 | 1.717 |
| base1 | 8 | 1.180 | 1.805 |
| q3dm1 | 1 | 14.583 | 16.246 |
| q3dm1 | 2 | 12.076 | 13.401 |
| q3dm1 | 4 | 9.132 | 9.788 |
| q3dm1 | 8 | 6.733 | 8.535 |
| q3dm1 | auto (8) | 6.250 | 7.250 |

The second q3dm1 fixed-eight row measured 6.559/7.819 ms. Baseline `39aaea41`
fixed-eight rows measured 7.036/9.038 and 7.221/9.406 ms. Mean matched medians
fell 6.77%; auto is within 10% of the best fixed row. The matched-eight sequence
was A/B/A/B with intervening workloads, not ABBA. Baseline one band measured
15.399/17.840 ms. These data preserve the band gain but still exceed the CPU
under-4-ms target; speed work remains paused under the core order.

All three maps have bit-identical RGBA and inverse depth across 1/2/4/8 bands.
q3dm1 auto and baseline rows match those buffers. All 17 private benchmark runs
recorded zero calling-thread/worker Rust allocations, normal quit, preserved
copied candidates/original settings, and no remaining owned PIDs. The scope is
fixed-scene CPU preparation/raster/dispatch/barriers, excluding host simulation,
presentation and native heap; no GL/gameplay/installation claim follows.

The pinned dispatcher probe, 32 jobs with no arithmetic, measured medians/p99
of 0.130/0.140, 6.710/8.240, 8.820/10.970 and 28.060/43.040 microseconds at
1/2/4/8 execution lanes. Caller plus lanes-minus-one background workers use the
same API; all job outputs agree and every measured frame counts zero Rust heap.
Cheap work exposes wake/barrier overhead rather than a parallel speedup. The
1024-iteration rows and raw results are retained separately.

Release builds: baseline 37.55 s, candidate 38.05 s, proof disabled. Evidence:
`THE-2872-2865/{dispatch-comparison,retail-comparison,pause-cleanup}.json`, raw
rows and before/after source archives under the task cache.

After the account switch, the saved normal release candidate completed all six
private normal-app runs: e1m1/base1/q3dm1 on CPU automatic bands and GL at 640x400,
60 warm-up and 600 measured frames, pinned to cores 4–11. Each CPU launch selected
eight bands and seven background workers with one total 32 MiB surface cache.
Every run consumed a fresh copied owner profile, presented its world without
rejected views, quit normally and counted zero allocations, reallocations and
requested bytes across all instrumented Rust threads. Original settings and the
candidate stayed unchanged; all 18 recorded owned PIDs exited.

GL identity was `llvmpipe (LLVM 22.1.8, 256 bits)`, OpenGL 4.6 Core Profile,
Mesa 26.2.2-arch1.1: these are software GL rows. This suite proves normal-app
rendering, automatic selection and the Rust allocation gate. It does not compare
host performance, measure native SDL/driver heap, prove gameplay, or qualify an
installation. `normal-qualification.json`, `normal-cleanup.json`, the six
`normal-{cpu,gl}-{map}/` receipts, screenshots and runtime logs retain the
evidence. The next core slice is THE-889; the historical account-switch state
remains in [the resume note](handoff/2026-10-08-account-switch.md).
## THE-889 shared human/bot command construction

The builder accepts unscaled intent for both human seats and SERVER bots.
Its native keyboard arithmetic matched 10,000 seeded cases each from original
NQ, QW, Q2 and Q3 input functions, including per-contribution short/integer
narrowing. This comparison covers already-sampled key fractions, not mouse,
controller, view, KEX client or gameplay behavior. The existing hold-state
comparison passed 364 cases and 10,000 frames. Existing Q2/Q3 movement checks
passed 1,152 states each; Q2 matched exactly, while Q3 retained its accepted
float tolerance with exact flags/timers. No complete native physics claim follows.

Portable release developer examples took 15.80 seconds before and 18.30 seconds
after. Core 23 was pinned, without a debugger, for 60 warm-up and 600 measured
frames. A batch of 64 mixed-rule intents measured 1.380 microseconds median and
1.430 microseconds p99 with zero measured Rust allocations/bytes. The bot host
probe built 12,288 commands with zero measured heap counts; the 512-client probe
also passed its allocation gate. These are headless workloads, excluding
native heap, rendering, gameplay and network framing.

The matched human bind/console/UDP host ABBA rows measured before medians
210.430/210.500 microseconds and after 208.656/208.471 microseconds, with matching
packet/repeat/provider counters and zero allocations. The initial baseline
overlapped a reference-helper compilation, so this set is retained as bounded
regression evidence rather than an isolated speed comparison.

Raw comparisons, source extracts, binaries/build metadata, allocation controls
and logs are in `~/.cache/qa-rust/THE-889-unified/`. The pending rule-checker
extensions and new check_usercmd.py were removed per the owner's correction;
the existing checker remains unchanged. Centerview and the issue's private
combined-seat/qualified installed gameplay acceptance remain open.

The following centerview slice registers the shared console command, applies
Q2/Q3 delta-pitch centering, and advances NQ/QW pitch drift in CLIENT after
command construction. Its 20,000 synthetic view frames matched the original
NQ/QW pitch bits, covering ground loss, NQ noclip inhibition, manual stops,
restart and automatic drift. Native guest ideal-pitch population remains open.
The four-seat command/view probe measured 0.800 microseconds median and 0.830
microseconds p99, with zero calling-thread allocations/bytes and an allocation
positive control. This is a distinct workload from the 64-intent batch above.
A fresh host ABBA check with no concurrent owned compilation retained identical
fidelity counters and zero allocation counts; mean medians changed -1.67%.
Rows and cleanup are in `centerview-timings/result.json`, with the original
view-function extracts and raw comparison rows in `native-view/`.

The clean normal allocation-tracked candidate at `a82e70a4` built in 28.75 seconds.
At slice completion it rendered e1m1/base1/q3dm1 on CPU automatic eight bands
and GL, at 640x400, with fresh copied owner settings. All six private X11 runs
quit normally and counted zero calling/worker Rust heap over 600 measured frames
after 60 warm-up frames. Each recorded 17 real key repeats; all 18 owned PIDs
were absent afterward. GL was Mesa 26.2.2 llvmpipe LLVM 22.1.8 software GL.
The initial CPU Q3 lookup row was inconclusive. An isolated retest of the same
candidate at controlled 60 fps reached -56.24279 degrees and returned pitch to
zero after centerview, with normal quit and zero calling/worker Rust heap over
600 measured frames. Evidence is `q3-cpu-centerview-retest/verification.json`.
Full-button, combined-seat and qualified installed gameplay acceptance remain
open. No install or game-audio proof.

## THE-2852 common numeric HUD values

One load-sized bank supplies arbitrary native fields to the existing player/HUD
snapshot path. The initial native-declaration fixture matched 1,344 field/width
records, including highest slots and signed extremes for NQ/QW32, Q232, Q2RR64
and Q3 stats/persistent16. Additional semantic checks retain raw float bits,
separate module reset ranges, session/persistent values, layout text and bounded
capacity failure. Native guest/layout drawing and packet interoperability are
not established by this component evidence.

A portable release developer build took 8.34 seconds. Core23, no debugger,
60 warm-up and 600 measured frames: 64 HUD snapshots with 192 numeric fields each
measured 8.450 microseconds median and 8.590 microseconds p99, with zero measured
calling-thread allocations/bytes and a positive allocation control. The unchanged
headless human host ABBA workload kept matching counters and zero heap; mean
medians increased 0.044%. Its zero-connected-client workload does not measure
the numeric snapshot work; the separate 64-client row does. Evidence is in
`~/.cache/qa-rust/THE-2852-numeric/`, including native declarations/raw rows,
`result.json`, `host-abba.json`, release examples and cleanup metadata.
After moving the generic width/binding operations into core, the final release
example build took 23.77 seconds. All 1,344 native rows stayed identical; the
same 64-client workload measured 8.390/8.730 microseconds median/p99, zero heap
and the same checksum. This final row is `result-core-binding.json`.

## THE-2869 per-client local transport

The safe typed-header/byte FIFO now backs both system events and each local
client/direction. Admission never overwrites. Native NetQuake's unchanged
`IntAlign`, `Loop_SendMessage` and `Loop_GetMessage` bodies delivered exactly
the same 3,959,206 output bytes as the Rust event-queue path across 1,000
seeded messages, including 1,401 and 8,000 bytes. This comparison scaffolds
native buffers and checks admitted payloads. It does not qualify a live NQ
signon or the native reliable adapter. Separate capacity tests cover 64,000
bytes, independent clients, header/byte exhaustion, wrap and event backpressure.

Core 23, portable release, no debugger, 60 warm-up and 600 measured frames:
eight messages across four clients include 8,000/64,000-byte payloads, four
counted Full returns each frame and complete byte comparisons. Median/p99
were 7.190/7.350 microseconds. The calling Rust thread counted zero allocations,
reallocations and requested bytes, with a positive control. There are no
workers in this transport workload; native heap activity is unmeasured.

A matched headless host ABBA comparison against `4f06269a`, with the same
console/bind/local-packet workload, preserved 1,396 packets, 659 repeats and
provider counters `[210,105,421,210]`, with zero measured Rust heap. Mean medians
were 147.815 microseconds before and 151.243 after, an increase of 2.32%.
Individual medians/p99 were 148.090/232.490 and 147.540/228.430 before,
150.820/233.660 and 151.665/239.520 after. This is host overhead, not gameplay
or a renderer timing. All four recorded host PIDs were absent afterward.

Evidence: `~/.cache/qa-rust/THE-2869-loopback/`, including unchanged native
function bodies, raw binary outputs, `native-comparison.json`,
`transport-timing-final.json`, `host-abba.json` and workspace/checker logs.
The final release probe build took 22.87 seconds. An intermediate allocation
recheck overlapped tests; its timing row is excluded and retained in `bench-final.log`.
No installation or live native-protocol acceptance is claimed.

At slice completion, clean normal non-proof commit `11af1d03` built in 40.19
seconds at 2026-10-09T06:16:10Z with allocation tracking. Fresh copied owner
profiles privately rendered e1m1/base1/q3dm1 on CPU automatic eight bands and
GL at 640x400, with controlled 120 fps, real key repeats and normal quits.
All six runs counted zero calling/all-worker Rust heap over 600 measured frames
after 60 warm-up. CPU used seven workers. GL used Mesa 26.2.2 llvmpipe LLVM
22.1.8 software rendering; these are not hardware GL timing rows. Profile and
candidate bytes stayed unchanged. All 18 owned PIDs were absent afterward.
Receipts are `normal-tracked-{cpu,gl}-{map}/`,
`normal-tracked-qualification.json` and `normal-cleanup.json` in the same
evidence directory. These runs qualify render/queue containment and Rust heap
counts, not native signon, complete gameplay, SDL/driver heap or installation.

## THE-697/890: independent output retirement and HUD leases

The destructive drain and recyclable text rows are replaced by one server-owned
ring, fixed per-client/module delivery state and independently leased HUD text.
Publication visits only registered consumers. An initial scan of every reserved
consumer row increased the matched host median by 46.08%; that implementation
was corrected before commit. The initial ABBA receipt remains in
`host-abba-initial.json` for comparison.

Portable release, core 23, no debugger, 60 warm-up/600 measured frames:

| Workload | Median | p99 | Rust heap |
|---|---:|---:|---:|
| Stalled reliable peer, healthy peer, 10/20/40-Hz modules, 40-Hz world and one local HUD | 2.220 us | 2.840 us | 0 |
| Matched host before, ABBA A1 (`6d164177`) | 152.191 us | 243.620 us | 0 |
| Matched host after, ABBA B1 | 155.636 us | 246.961 us | 0 |
| Matched host after, ABBA B2 | 156.165 us | 246.710 us | 0 |
| Matched host before, ABBA A2 | 153.690 us | 235.840 us | 0 |

The retirement fixture delivered all 2,304 records to the healthy peer, with
1,152 sound callbacks, valid text, independent module deliveries
`[2292,2300,2304]` and 1,650 measured SERVER/provider ticks. Slower modules
retain their last unconsumed records. The stalled peer submitted 30 reliable
records, supplied no ACK, then reached one bounded overflow/resync: 32 retained
records cancelled and 2,272 subsequent records skipped during resync. The
healthy peer had no overflow. HUD disconnect released its display leases while
a slower module still retained the last payload. This models native receipts;
no live legacy channel, guest module or audio backend is claimed.

The matched host uses the same console/bind/loopback/output workload and emits
byte-identical console output. Both sides retain 1,396 packet deliveries,
659 repeats, `[210,105,421,210]` timeline counters and 1,980 sound/effect callbacks.
Mean medians changed from 152.940 to 155.900 us (+1.94%). The four recorded PIDs
were absent afterward. This compares host overhead rather than gameplay or
renderer performance.

The fixed event probe delivered 2,560,000 records of each kind over 10,000
frames with no stale text, remaining records, overflow or Rust allocation.
The leased HUD probe produced 30,000 snapshots over 10,000 frames without
allocation. Allocation positive controls pass. These workloads have one
instrumented calling thread, no workers and no measured native heap.

Evidence: `~/.cache/qa-rust/THE-697-retirement/`, including
`output_retirement.log`, `host-abba.json`, allocation logs, native-ACK source
references in `docs/output-events.md`, workspace/Clippy/checker results and the
23.61-second release probe build log. Native wire ACKs, combined guest output,
shotgun/private audio and installed gameplay remain open acceptance criteria.

At slice completion, exact clean normal d240a025 built in 30.18 seconds at
2026-10-09T07:10:02Z with allocation tracking and proof disabled. All six private
e1m1/base1/q3dm1 CPU/GL runs at 640x400 and controlled 120 fps completed 600
measured frames after 60 warm-up frames, with 17 real X repeats each and
normal quit. Calling/all-worker Rust allocation, reallocation and requested
bytes were zero; CPU selected eight automatic bands/seven workers, GL used
llvmpipe LLVM 22.1.8 / Mesa 26.2.2 software rendering with no workers. Fresh
copied saved settings and candidate bytes stayed unchanged, and all 18 recorded
owned PIDs were absent. The adjacent `normal-tracked-qualification.json`,
`normal-tracked-{cpu,gl}-{map}/` and `normal-cleanup.json` retain these receipts.
These are private render/queue/heap checks; actual gameplay and installation
remain unqualified. THE-859 still requires installed multiple-seat/device and
combined-movement acceptance; the normal app currently creates one seat.

## THE-617 console and renderer name consumers (2026-10-09)

Core NameTable now supplies one folded bucket lookup, stable exact IDs and
load-reserved registration. Console NameIndex/hash and local comparison are
deleted; commands, cvars and aliases dispatch from a single token lookup into
numeric bindings. Flag members and native default roles resolve at load.
Materials/images use canonical-path NameIds and typed recipe keys; their
String-key and duplicate path-conversion implementations are deleted. Parser
keyword handling is unchanged. See names.md for storage/lifetime rules.

Checker, 567 workspace tests (129 suites) and all-target Clippy with allocation
tracking passed. Release probe rebuild: 16.60 s. The unchanged qsrc Q3
Q_stricmpn function (q_shared.c:727-764) matched all 10,256 ASCII ordering pairs,
including punctuation. Original Q1/QW/Q2/Q3 target comparisons still match all
12,712 rows, with zero counted calling-thread allocations after cold load.
Owner catalog cells remain identical (26,460 cells); numeric/pure conversion
comparisons match 100,080 number and 344,475 view records. These fixtures retain
the earlier C-string/native namespace limitations.

The release name_consumers example ran on CPU 23 with 60 warm-up and 600
measured frames, 64 lookup groups per frame. It checks exact-name distinction,
missing lookups, stable registrations, repeated image/material registration,
Q3 first-image sampler ownership, and alias replacement/removal with native
console output. Median 11,080 ns, p99 14,190 ns; zero Rust calling-thread
allocation/reallocation calls and requested bytes. Positive control: one
allocation. This is a headless lookup workload, without rendering or workers.

Matched host ABBA against 84aae4af (engine d240a025), CPU 23, same console/bind,
output and local UDP workload, 60 warm-up plus 600 measured frames per run:

| Run | Median ns | p99 ns |
| --- | ---: | ---: |
| Before A1 | 155,630.5 | 247,040 |
| After B1 | 153,810 | 252,280 |
| After B2 | 153,590 | 245,110 |
| Before A2 | 155,900 | 246,060 |

Mean medians: 155,765.25 -> 153,700 ns (-1.33%). All counters and the complete
stdout prefix match; every run counted zero Rust calling-thread allocations.
This small median difference is bounded host-path evidence, not a renderer or
gameplay speed claim. All four recorded processes exited. Fixed frame-scratch
allocation qualification also passed.

Evidence: local THE-617-consumers directory, names.json, host-abba.json,
order/comparison.json, targets-native/report.json, cvar-cells/comparison.json,
cvar-views/comparison.json, tests.log, clippy.log and rules.json. No gameplay
installation, retail trigger/door execution or native module/protocol acceptance
is asserted by these checks.

The exact clean normal candidate cac06994 built in 32.293 s at
2026-10-09T07:57:44Z (portable baseline, allocation instrumentation, no proof
feature). Private copied-profile/candidate checks rendered e1m1, base1 and
q3dm1 on CPU automatic eight bands and GL at 640x400, 120 fps cap, 60 warm-up
and 600 measured frames. All six normal exits logged 23 key downs and 17 real
X11 auto-repeat events. Every measured frame counted zero Rust allocation,
reallocation and requested bytes, including all seven CPU workers; GL had none.
Copied candidates and original owner profiles remained unchanged. All 18
recorded owned PIDs are absent after cleanup. Screenshots show the loaded worlds.

GL driver: llvmpipe (LLVM 22.1.8, 256 bits), Mesa 26.2.2-arch1.1, 4.6 Core;
these are software GL checks. Evidence: THE-617-consumers normal-tracked-* run
receipts/screenshots, normal-tracked-qualification.json and normal-cleanup.json.
Every run still reports gameplay_reached=false. No install, native driver heap
measurement, audio acceptance or comparable gameplay/renderer timing is claimed.

THE-697/890 continuous host check (2026-10-09): the release `output_retirement`
example now asserts healthy delivery and 40-Hz world progress on every frame,
not only final totals. CPU 23, 60 warm-up and 600 measured frames per case:
reliable-without-ACK median 2,265 ns / p99 2,810 ns; Unsent median 2,170 ns /
p99 2,810 ns. Both cases enter bounded stalled-consumer resync on frame 10,
continue through all 600 measured frames with 1,650 SERVER/provider ticks,
deliver 2,304 records to the healthy consumer, preserve the module counts
[2,292, 2,300, 2,304] and record zero Rust allocations or requested bytes.
Each stalled consumer has one overflow/resync, zero ACKs, 32 cancelled records
and 2,272 subsequently skipped records; the healthy consumer has zero overflow.
Evidence: `THE-697-retirement/continuous-host/pinned.jsonl`, checker and workspace
logs under the private evidence root. This is the complete headless host output
path with modeled native delivery results, not a live native channel, gameplay
qualification, an installation or game-audio evidence. No production path changed.

THE-859/656 normal split-seat integration (2026-10-09): engine commit
`299427a66ca899ff5f03e91a0806ebf4e721af62`, clean portable normal candidate,
allocation tracking enabled and proof disabled. Build 27.5055 seconds at
2026-10-09T08:53:08Z. Eight private 640x400 runs consume fresh copied owner
settings, quit normally and count zero allocations/reallocations/bytes over
600 measured frames after 60 warm-up frames, including seven CPU workers.
Keyboard and aggregate pointer produce independent commands; holds use real
X-server repeat (40 or 41 repeats per run). Screenshots show two distinct views.
Trace policies stay explicit; additional authored spawn anchors are fixtures
until native module selection/telefrag handling is loaded.

| Map and movement pair | Backend | Draw median / p99 (ms) | Total median / p99 (ms) |
| --- | --- | ---: | ---: |
| e1m1 Q1 + Q3 | CPU edge/span | 1.297 / 1.631 | 7.956 / 8.345 |
| e1m1 Q1 + Q3 | OpenGL (Mesa software) | 5.624 / 8.143 | 8.134 / 10.347 |
| base1 Q2 + Q3 | CPU edge/span | 1.277 / 2.522 | 7.961 / 8.485 |
| base1 Q2 + Q3 | OpenGL (Mesa software) | 2.188 / 17.214 | 8.048 / 18.334 |
| q3dm1 Q3 + Q1 | CPU edge/span | 99.462 / 105.345 | 101.278 / 107.272 |
| q3dm1 Q3 + Q1 | OpenGL (Mesa software) | 2.523 / 19.039 | 8.090 / 20.657 |
| base1 Q1 + Q3 | CPU edge/span | 1.234 / 2.410 | 7.962 / 8.397 |
| base1 Q1 + Q3 | OpenGL (Mesa software) | 2.065 / 18.231 | 7.989 / 20.196 |

Affinity: physical CPUs 4,5,6,7,8,9,10,11; frame cap 120, so total rows include cap wait.
Mesa software GL: llvmpipe LLVM22.1.8, 256-bit, Mesa26.2.2-arch1.1 GL4.6.
These moving two-view workloads are not comparisons with prior single-view
fixed-camera renderer timings, and do not qualify the R12 targets. q3dm1 speed
work remains paused by the core-first order. CPU rejects: e1m1 initial27/max29
(THE-3164 attribution pending); base1 initial0/max2. GL and q3dm1 CPU reject0.
Render fidelity, complete native physics, game audio, native protocol/module
integration and installed gameplay remain open. No installation occurred.

The first reporter failed on the three-column sample format; it was corrected
and re-read the completed exact-candidate run. The initial q3dm1 CPU action
window failed independent-mouse evidence: press/release arrived within one
long frame. Its raw run remains in `q3dm1-cpu-short-input-attempt`. The harness
now keeps each control held until eight host frames observe the intended
device phase. The corrected q3dm1 CPU run has17 keyboard-only,10 mouse-only
and77 simultaneous command frames; input remains private XTest OS input,
not a shipped input player. No engine receive/poll location changed.

Evidence: `THE-859-local-seats/normal-candidate/build.json`,
`normal-qualification/qualification.json`,
`base1-q1-q3-qualification/qualification.json`, per-run logs/screenshots,
`cleanup.json` (all30 owned PIDs absent, including development/failed attempts),
`workspace-tests-final.log` (569 passes), retail loads (six passes), Clippy,
unchanged checker and developer-tool logs under the private evidence root.
No old single-seat startup or singular map spawn API remains.

## THE-650: attachment transport and area links

The shared authoritative body commit now transports attachments through the
existing entity SoA and area grid. Follow modes and parent-first insertion-order
semantics come from the proven C port's `src/world/body.c`. Its unchanged
`qa_world_attach` (389-407) and transport/helper block (640-693) were compiled
with developer body-access/link-count stubs. Seed `0x650`, 512 cases, 16,384 body
rows and two transports per case matched all final position, velocity and local
bound bits plus both link counts. Transport heap activity was zero. This is a
C-port behavior comparison; it is not an original native module/touch comparison.

Portable release probes were built in 23.046 s; the final developer-probe lint
cleanup rebuilt in 5.283 s. Pinned core 23, 60 warm-up frames and 600 measured
frames, no debugger, workers, display or game: a reverse-inserted chain exercises
all three follow modes. Every moved body retains its velocity/angles and reaches
the expected exact pose. A second transport remains unchanged. Allocation
qualification also includes detach/reattach and 8,192 explicit unlink/link cycles
per frame. The positive control records one allocation.

| Capacity / followed bodies | Transport median / p99 µs | 8,192 link cycles median / p99 µs |
| --- | ---: | ---: |
| 64 / 63 | 2.120 / 2.870 | 182.891 / 193.560 |
| 1,024 / 1,023 | 26.780 / 31.980 | 148.265 / 156.910 |
| 8,192 / 8,191 | 213.960 / 220.931 | 147.650 / 156.080 |

All three probes record zero Rust allocations, reallocations, requested bytes and
fidelity mismatches across every measured frame. No prior Rust attachment
implementation existed, so these are new-workload measurements with no claimed
before/after speedup. Native touch logs, installed mixed pickups, live module
binding and active-client attachment prediction remain open; THE-650 remains
In Progress. No renderer qualification or installation is claimed for this slice.

Evidence under `THE-650-attachments`: `pinned.json`, capacity-specific reports,
`c-port-comparison-final/comparison.json` and exact extracted helper source,
`build-time.json`, `build-time-final.json`, workspace tests, Clippy and unchanged
checker logs. All three recorded probe PIDs are absent after their normal exits.
See [body attachments](body-attachments.md) for ownership and invocation details.

### THE-650: predicted attachment poses

The CLIENT phase now uses the same attachment walk after per-seat movement.
Its predicted pose view retains computed intermediate-anchor positions in
load-sized scratch, while the physical world and link lists stay frozen. A full
FrameHost fixture follows a Q3-movement local anchor with a Q1-movement local
client through the normal queue, bind, SERVER, snapshot and prediction paths.
The world fixture also covers an intermediate body without a local pose and a
remote authoritative anchor. Neither fixture establishes native module gameplay.

The unchanged C-port comparison still matches all 512 cases / 16,384 body rows
and both transport counts. Checker, 578 workspace tests and allocation-feature
Clippy pass. Portable release developer probes built in 22.341 s. Pinned core23,
60 warm-up / 600 measured frames; matched physical transport ABBA comparison
against 5a2e7517 preserves all workload, move/link, fidelity and allocation counts.

| Capacity | Before / after mean transport median µs | Median change | Before / after mean p99 µs |
| --- | ---: | ---: | ---: |
| 64 | 1.675 / 1.780 | +6.269% | 1.745 / 1.830 |
| 1,024 | 26.560 / 28.455 | +7.135% | 30.480 / 32.620 |
| 8,192 | 213.245 / 229.195 | +7.480% | 232.730 / 238.390 |

The separate two-client predicted workload measures the new feature, including
unmapped bodies between the predicted clients:

| Capacity / followed bodies | Prediction median / p99 µs |
| --- | ---: |
| 64 / 63 | 0.880 / 0.890 |
| 1,024 / 1,023 | 13.700 / 17.430 |
| 8,192 / 8,191 | 109.706 / 116.510 |

Every measured frame has zero Rust allocation/reallocation/requested bytes and
physical pose/link mismatches. The headless host ABBA workload retains its exact
fixture counters and zero heap activity: mean median153.225 to151.263µs (-1.281%),
mean p99242.420 to231.216µs (-4.622%). That host workload has no map geometry;
it does not measure the new prediction feature or qualify gameplay/installation.

Evidence: `THE-650-attachments/prediction/{physical-abba.json,host-abba.json,
predicted-*.jsonl,c-port-comparison/comparison.json,workspace-final.log,
clippy-final.log,checker-final.json,build-time.json,cleanup.json}`. Initial failed
host fixture logs are retained; its prefilled queue violated the existing source's
empty-at-intake invariant, and the corrected fixture injects at physical intake.
Native touch/pickup/module/mover and installed acceptance remain open. This closes
the headless current-client attachment prediction gap, not those native gates.

### THE-650 finished-slice normal candidate checks

Clean portable normal candidate `3b85f115`, allocation tracking enabled and proof
input disabled, built2026-10-09T10:15:10Z in27.675s. The finished slice ran eight
copied-profile private Xvfb/Openbox checks: e1m1/base1/q3dm1 CPU(auto8) and GL,
plus base1 Q1+Q3 movement on both backends. Each used640x400,60warm-up/600measured
frames,cap120 and affinity4,5,6,7,8,9,10,11. All quit normally, retain two distinct
views, observe40-42 actual key repeats and independent keyboard/pointer/both
phases, release controls to neutral and use exactly two intakes through the common
queue. All eight measured allocation gates pass with zero allocations,
reallocations or requested bytes: one caller plus seven CPU workers, or no GL
workers. The candidate and original profile remain unchanged. All24 recorded
game/window-manager/display PIDs are absent. All eight captures were inspected.

| Map / movement choices | Backend | Draw median / p99 ms | Total median / p99 ms |
| --- | --- | ---: | ---: |
| e1m1 Q1+Q3 | CPU edge/span | 1.433 / 2.290 | 7.942 / 9.455 |
| e1m1 Q1+Q3 | GL (Mesa software) | 5.523 / 8.833 | 8.141 / 11.137 |
| base1 Q2+Q3 | CPU edge/span | 1.572 / 3.897 | 7.952 / 9.427 |
| base1 Q2+Q3 | GL (Mesa software) | 2.277 / 19.181 | 8.101 / 20.841 |
| q3dm1 Q3+Q1 | CPU edge/span | 3.598 / 14.432 | 8.031 / 16.177 |
| q3dm1 Q3+Q1 | GL (Mesa software) | 3.732 / 18.484 | 9.594 / 20.614 |
| base1 Q1+Q3 | CPU edge/span | 1.518 / 2.688 | 7.964 / 8.599 |
| base1 Q1+Q3 | GL (Mesa software) | 1.959 / 16.994 | 7.981 / 18.814 |

GL identity: llvmpipe(LLVM22.1.8,256bits),Mesa26.2.2-arch1.1,GL4.6Core.
The moving camera/input paths do not hold the prior run's camera states or draw
work constant. In particular the changed q3dm1 draw distribution is not evidence
of a renderer speedup or the R12 target. Total includes the frame-cap wait.
e1m1 CPU rejects27initial/max29; base1 CPU max2; GL/q3dm1 CPU reject0.
Fidelity remains unqualified (THE-3164 and existing render issues). These runs
have no bound attachments or native gameplay modules, so they exercise regressions
around the new path, not native attachment/touch/mover acceptance. Game audio,
native wire/module and installed gameplay remain open; no installation occurred.

Evidence under `THE-650-attachments/prediction`: `normal-candidate/build.json`,
`normal-qualification/qualification.json`,
`base1-q1-q3-qualification/qualification.json`, per-case logs/captures and
`normal-cleanup.json`. The gameplay/timing installation gates remain unchanged.

## THE-709 load-resolved think policy (2026-10-09)

This section records committed `bebdaeb0`, measured by `abba-summary.json`.
Its 6.107–7.222% median increases do not meet the owner's subsequent requirement
to stay at or below the pre-commit baseline. `pointer-abi-abba.json` belongs to
a rejected intermediate design, not `bebdaeb0`: its three medians were
660→1,140 ns, 650→1,050 ns and 76,950→132,750 ns.

Module registration now resolves seconds/milliseconds, frame-end comparison,
Q2 tolerance, Q3 integer-to-float comparison and QW repetition into private
timing data. Callback function pointers remain resolved at load. The shared
per-entity entry evaluates this data without selecting a game identity on each
call. The old selector is deleted. The slot scan continues to use only
`EntityTable::next_active`, which reads the authoritative liveness bitset and
re-resolves lifetimes after callback mutations. Native physics providers still
own the phase at which they call this entry.

Release `think_dispatch` compares the production dispatcher against parent
`41d294e5` using the same probe source and mixed five-rule workload. Each run
uses CPU 23, 60 warm-up and 600 measured frames. A/B/B/A averages follow:

| Capacity / live actors | Before / after median µs | Median change | Before / after p99 µs |
| --- | ---: | ---: | ---: |
| 64 / 63 | 0.605 / 0.645 | +6.612% | 0.765 / 0.990 |
| 8,192 / 64 | 0.655 / 0.695 | +6.107% | 0.700 / 0.740 |
| 8,192 / 8,191 | 76.920 / 82.475 | +7.222% | 81.885 / 88.040 |

Per-rule callback counts and final timestamps match in every run; measured
allocations, reallocations and requested bytes are zero, with allocation
positive control 1. This measures a headless calling thread, without workers,
native modules or rendering. It is not a gameplay performance qualification.
The separate four-million-callback and 30,000 HUD-snapshot probes also report
zero allocation/reallocation calls after load. Their older counter does not
measure requested bytes.

The unchanged native-function comparator matches all 13,390 rows: Q1 2,965,
QW 2,965, Q2 2,690, rerelease 2,290 and Q3 2,480. Native float narrowing,
signed integer time, callback preservation, QW repetition and scoped rejection
retain the comparator's existing bounds. It does not execute guest modules or
native monster physics phases. Workspace tests pass 578 cases; all-target
allocation-feature Clippy and the unchanged rule checker pass. Final release
probe build took 7.066 seconds. Two measured indirect timing-call designs were
rejected for median regressions between 61% and 86%; their receipts are retained
alongside the committed policy-data measurements.

THE-656/702 source audit finds one production PlayerState definition, one
HudState definition and one load-sized client array. The existing CLIENT loop
projects every connected client's PlayerState through the same HudBindings,
preserving its text leases and layout. No alternate per-game player/HUD state
or stock HUD drawing implementation is present. The existing Q1+Q3 seat runs
on e1m1/base1 and the three-map normal-candidate receipts above remain evidence
for candidate render/input integration, not installed gameplay. No new private
game run or install was needed for this headless dispatch change.

Evidence is in `THE-709-load-rules-20261009` under the local QA evidence cache:
`checks.json`, `comparison/report.json`, `abba-runs.json`, `abba-summary.json`,
`source-audit.json`, `dispatch_allocations.log`, `hud_allocations.log` and the
workspace/Clippy/checker/release logs. Run the probe with
`cargo run --release -p qa-platform --example think_dispatch --features
allocation-tracking -- --capacity 8192 --live 64` under the chosen CPU affinity.
Native HUD drawing, module-phase think callers, installed five-scene think
acceptance, native protocol connections and qualified qa-rust installation
remain open on THE-656/702/709/859.

## THE-709 spawn-bound function entries (2026-10-09)

Each entity's native think function now binds to an entry in the one flat
function table at load, spawn or a native function/owner write. That entry
holds its module, adapter, native function index and arithmetic/repetition
policy. The due-call path no longer resolves the module table or resolves the
entity a second time inside `invoke`. Arithmetic still preserves native widths;
QW rereads the current binding after a callback, and the ascending scan still
uses the authoritative liveness bitset. Clearing a deadline preserves the
binding. Releasing or replacing a lifetime clears both. Internal entry indices
are never native callback identities or protocol/save fields.

All callers have moved to binding at those boundaries; the old writable native
callback column is private. Null and invalid functions retain their native
value and module policy, clear a due deadline and reject only that call. Missing
modules retain the deadline. The original-C comparator and its 13,390 fixture
rows are unchanged and pass, including native function changes and QW repeats.
Twenty focused tests cover the binding lifetime and existing dispatch behavior.

Release CPU 23 A/B/B/A, 60 warm-up plus 600 measured frames, compares against
the original `41d294e5` binary used before `bebdaeb0`. The measured loop and
callback fixture are unchanged; only cold setup binds the entries:

| Capacity / live actors | Before / after median ns | Median change | Before / after p99 ns |
| --- | ---: | ---: | ---: |
| 64 / 63 | 610 / 585 | −4.098% | 655 / 630 |
| 8,192 / 64 | 660 / 635 | −3.788% | 715 / 675 |
| 8,192 / 8,191 | 77,867.5 / 74,792.75 | −3.949% | 90,095 / 81,935 |

Every median meets the at-or-below-baseline gate. Per-rule counts match:
`[8580,8580,8580,7920,7920]`, `[9240,7920,8580,8580,7920]` and
`[1081740,1081080,1081080,1081080,1081080]`. The probe checks final native
timestamps, zero fidelity mismatches, zero allocations/reallocations/requested
bytes and an allocation positive control of one. Separate four-million-call
and 30,000 HUD-snapshot probes also count zero allocations after load. These
are headless calling-thread measurements, without gameplay, workers or native
module execution. Release probe build took 9.63 seconds.

Evidence: local QA cache `THE-709-cost-recovery-20261009/final-abba.json`,
`comparison/report.json`, `dispatch_allocations.log`, `hud_allocations.log`,
`workspace.log`, `clippy.log` and `release.log`. `attempt1-abba.json` retains the
rejected extra-indirect-call prototype. Workspace tests, all-target
allocation-feature Clippy and the unchanged checker pass. Native physics-phase
callers and installed five-scene acceptance remain open; this headless slice
does not qualify a gameplay install.

### THE-709 measurement repeatability check

A fresh command verification rebuilt `think_dispatch` and reran all 20 focused
dispatcher tests successfully. The original `final-abba.json` raw samples
recompute to the table above. A subsequent unconditioned ABBA run failed the
sparse 8,192/64 gate: before medians 640/640 ns, after medians 1,080/630 ns,
so its averages were 640/855 ns. The other two workloads passed. This failed
run is retained as `hook-stop8-verification.json`; an unconditional repeatability
claim is withdrawn. The cause of the elevated first sparse B run is unproven.

To check sensitivity to process-start conditions, each A and B process then
received the same 250 ms CPU-bound preconditioning on CPU 23 before executing
its binary. No governor or machine setting changed. Each binary still performs
60 fixture warm-up and 600 measured frames with unchanged callback counts and
timestamps. Three complete ABBA blocks per workload all passed:

| Capacity / live | Block 1 before / after median ns | Block 2 | Block 3 |
| --- | ---: | ---: | ---: |
| 64 / 63 | 610 / 585 | 610 / 585 | 605 / 585 |
| 8,192 / 64 | 660 / 630 | 655 / 645 | 660 / 635 |
| 8,192 / 8,191 | 77,042.5 / 73,880 | 77,195 / 74,360 | 77,067.5 / 73,625 |

All 36 processes report matching fixture counts, zero fidelity mismatches,
zero allocations/reallocations/requested bytes and positive control one.
`hook-stop8-conditioned-abba.json` retains every run and its preconditioning
metadata. This supports the headless comparison under those stated conditions;
it does not establish the cause of the unconditioned variance or qualify live
gameplay. No engine changes were made during this verification.


## THE-656/THE-702: app-selected client capacity

`Runtime::load` now receives the common client capacity explicitly. The app's
`--max-clients N` selects it at load (default 64), independently of the native
limits negotiated by each connection. It must cover the local seats and fit
the current entity reservation. The one server array, output consumer capacity
and per-client loopback storage use that value. All 21 existing app/example
callers migrated; the client-state probe now also uses the app loader rather
than constructing a smaller server directly. No implicit-64 loader remains.
Native client limits, entity namespaces and wire widths are unchanged.

The host fixture fills 257 clients with mixed Q1/Q2/Q3 inventories, projects
all their PlayerStates into the one HudState primitive, and retains the existing
text leases. Client 256's local packet enters SysEventQueue and reaches the
packet ingress consumer; the next out-of-range client is rejected. This is a
host fixture, not native packet framing. The app-loaded client-state allocation
probe uses 512 clients, highest ID 511, and 600 reconnects after 60 warm-up
iterations: zero measured calling-thread allocation/reallocation calls and
requested bytes, with allocation positive control 1. The existing 64-client
numeric HUD probe also retains checksum 892087680 and zero measured heap work.

The release probe build took 16.541 s, starting 2026-10-09T12:24:41Z, portable
CPU and allocation tracking enabled. Workspace all-target tests (580), tracked
Clippy, unchanged rules and `git diff --check` pass. Evidence is in
`~/.cache/qa-rust/THE-656-app-capacity-20261009/`: `checks.json`,
`caller-inventory.json`, `checker.json`, `client_state.log`, `hud_values.log`
and the raw `abba-*.log` files.

The matched headless host workload uses the unchanged 64-client fixture,
console/binds/bots/output plus local packet admission, CPU 23, 60 warm-up and
600 measured frames per process. Every A/B process receives the same 0.25 s
CPU preconditioning before its warm-up. Baseline is main 17277890; the candidate
only changes load-time capacity selection and callers. Mean of the two process
medians in ABBA order:

| Build | Median ns | Mean process p99 ns | Max Rust heap calls/bytes |
| --- | ---: | ---: | ---: |
| Before | 174505.25 | 266995 | 0 / 0 |
| After | 173172.5 | 263370 | 0 / 0 |

The median change is -0.764%; this cold-load adoption is not a claimed hot-path
optimization. All non-timing receipt fields match, including 12288 measured
bot commands, 1396 packets, 659 repeats, provider counters [210,105,421,210],
1980 sounds/effects and one output drain per frame. No workers or foreign heaps
are measured by these headless probes.

Stock HUD drawing and native module/transport integration are still absent.
Existing private render/input walks are candidate evidence, not installed
qa-rust gameplay. The gameplay/timing installer gates remain intact, and these
issues remain In Progress for their original installed/native acceptance.


## THE-617: converted cvar names use the shared name primitive

The remaining two cvar-name comparisons in `console::conversion` selected the
text autoswitch alias by scanning its name on read and write. Both now compare
NameIds resolved once through the cvar table's NameTable at load. The converted
read/write/default callers and the existing C-reference example migrated to
that input; the old Binding/string input is deleted. Value parsing (including
`never`/`new` alias details) retains its native behavior. No generated catalog,
checker rule or wire field changes.

Workspace all-target tests (581), tracked Clippy and the unchanged checker pass.
The focused fixture checks uppercase alias lookup, retained text details,
canonical invalidation and numeric autoswitch modes across all five sources.
Extracted C-port comparison: all 100080 number records and 344475 conversion
records remain byte-identical. This compares headless conversion helpers, not
native game modules or installed trigger/door dispatch.

Portable tracked release probes built 2026-10-09T12:32:54Z in 17.497 s. Matched
64-client headless host ABBA against ee5cb1ff, CPU23, identical 0.25 s CPU
preconditioning before 60 warm-up plus 600 measured frames: mean process medians
231257.5 ns before / 227353 ns after (-1.688%). Every non-timing fixture field
matches and measured Rust heap calls/bytes are zero. The separate app-loaded
512-client/reconnect and numeric HUD probes also retain zero measured heap and
HUD checksum 892087680. These runs create no raster workers and do not measure
foreign library heaps; host timing does not isolate conversion cost.

Evidence: `~/.cache/qa-rust/THE-617-conversion-names-20261009/` has `checks.json`,
`adoption.json`, `abba.json`, raw logs, `client_state.log`, `hud_values.log` and
`c-reference/comparison.json`. Original installed target/door/native module
acceptance remains open; no install or gate waiver occurred.


## THE-3165: renderer uses core vector math

The adoption sweep found 11 copied vector helpers in renderer geometry, patch
normals and stage evaluation. All their callers now use core Vec3 operators and
math; all 11 local implementations are deleted. Core owns the native fast
normalization estimate. Output normalization delegates to the existing core
normalization and clears zero components as Q3 VectorNormalize2 requires;
in-place normalization still retains signed zeros. No arithmetic reassociation,
new unsafe code, checker rule or renderer algorithm is introduced.

Unchanged checker, tracked Clippy and 582 workspace all-target tests pass.
Original-C math: 3009 bit-exact cases across three profiles, zero post-load
allocations. Original Q3 patch comparison: 58 fixtures / 119523 float components,
zero differing bits and zero maximum error; the reference retains the previously
approved reverse-endpoint bounds correction. The 1273938-byte Rust patch output
is byte-identical before/after, as are its fixture inputs. Q1/Q2 original warp
subdivision comparisons and stage fixtures also pass.

Portable tracked CPU draw/client-state probes built 2026-10-09T12:46:04Z in
32.431 s. The first retail build command omitted the app's allocation feature
and failed before a run; the corrected baseline and candidate both enable
`allocation-tracking,qa-app/allocation-tracking`. The private CPU draw ABBA uses
CPU23, one band/no workers, q3dm1 at640x400, fixed camera/shader time, copied owner
settings and copied binaries, 60 warm-up plus600 measured frames per process.

| Build | Mean process median ns | Mean process p99 ns |
| --- | ---: | ---: |
| Before a696f10c | 14626553 | 15045811.0 |
| After | 14608365.25 | 14873026.0 |

Median change is -0.124%, within measurement variance; no optimization or R12
qualification is claimed. All four runs have identical workload/stat fields,
256000-pixel RGBA bytes and inverse-depth bytes. All measured Rust calling-thread
allocation/reallocation calls and requested bytes are zero. Each run quits
normally; its copied profile/candidate stays unchanged and owned PIDs stop.
The app-loaded512-client reconnect allocation probe also retains zero heap.
No GL or foreign-heap measurement is claimed, and q3dm1 speed work stays paused.

Evidence: `~/.cache/qa-rust/THE-3165-math-adoption-20261009/` contains
`checks.json`, `checker.json`, `adoption.json`, `abba.json`, `A1/B1/B2/A2/`,
`patch-before-output.txt`, `patch-after-output.txt`, `math-reference/`,
`patch-reference/`, `grid-reference/` and `client_state.log`. These are developer
comparison/draw probes, not shipping candidates or installed gameplay proof.

# THE-863 typed module services foundation

Portable release probe built 2026-10-09T13:48:02Z in 13.726 seconds. CPU23,
60 warm-up and 600 measured headless frames; 64 service groups per frame cover
spawn, explicit link, free, cached cvar writes, two module configstring ranges,
byte-exact print and best-effort presentation retirement. Three runs measured
median/p99 105550/111960, 104380/110120 and 104490/111640 ns. Every run retained
42240 publications and checksum 27941760, with zero measured calling-thread
Rust allocations, reallocations or requested bytes; positive control counted one.

This is a new services workload with no equivalent prior module-host baseline,
not a speedup claim. It opens no display or worker, performs no native ABI/VM
execution and does not qualify gameplay, installation, filesystem-call allocation
or driver memory. Workspace, unchanged checker and tracked Clippy pass. Evidence:
`~/.cache/qa-rust/THE-863-services-20261009/` (`checks.json`, logs and `probe-*.json`).

## THE-801 / THE-863 QVM interpreter foundation (2026-10-09)

Portable release probe built at 14:10:03 UTC in 8.795 seconds. CPU 23, 60
warm-up and 600 measured frames, 1,000 calls per frame to the same eight-opcode
integer fixture. Three runs measured median/p99 frame times of 28.520/31.770,
26.510/30.031 and 26.610/30.430 microseconds. Each returned checksum
2,610,685,110,000, zero calling-thread allocations/reallocations/requested
bytes, and zero hook instructions; the allocator positive control counted one.
This new VM workload has no earlier comparable Rust module-host baseline and
is not a gameplay or speedup claim. No worker, driver or filesystem allocation
qualification is implied.

Evidence: `$XDG_CACHE_HOME/qa-rust/THE-801-qvm-20261009/` (or the user's
default `.cache/qa-rust`), `checks.json`, `timing-{1,2,3}.json`,
`reference/comparison.json`, original/Rust binary comparison rows and
`retail.json`. The unmodified original interpreter matched all 8,151 defined
seeded cases byte for byte, including return values and 192 memory bytes per
case. Baseq3 pak8 and Team Arena pak0 qagame/cgame/ui images all passed the
reader; no retail module entry point or gameplay was executed. Checker,
workspace tests and Clippy passed. Native channels, native libraries, services
ABI integration and installed acceptance remain open.

## THE-863 numbered module calls (2026-10-09)

Portable release probe built at 14:25:29 UTC in 13.703 seconds. CPU23, 60
warm-up and 600 measured frames, 64 QVM calls per frame. Each call sets fov,
prints, appends a console command and reads the cvar through Q3 server import
ordinals; the frame retires prints through existing best-effort submission and
executes the shared console buffer. Three median/p99 measurements were
149.9105/157.110, 149.230/157.340 and 150.2205/159.040 microseconds per frame.
All runs returned checksum 4,435,200, published 42,240 records, counted no
unknown imports or ordinary-loop hook instructions, and recorded zero
calling-thread allocations/reallocations/requested bytes with positive control
one. This is a new service workload, not a comparable optimization baseline.

Evidence: `$XDG_CACHE_HOME/qa-rust/THE-863-abi-20261009/` or the default
`.cache/qa-rust`, `checks.json`, `services.qvm`, `timing-{1,2,3}.json` and
`reference/comparison.json`. The original 8,151 execution/memory comparison
rows remain exact. Checker, 593 workspace tests and Clippy pass. Boundary tests
exercise full-width addresses above 32 bits, original role-specific ordinals,
atoi versus float views, byte strings, bounded unknown logs and supplied clock
width. No native machine code, retail gameplay, driver heap or installed
acceptance is qualified. Native entity binding, dynamic cvars, registrations,
remaining imports and native transport remain open.

## THE-793 / THE-863 QuakeC interpreter foundation (2026-10-09)

Portable tracked probes built at 14:49:05 UTC in 21.256 seconds. CPU23, 60
warm-up and 600 measured frames, 1,000 calls per frame to the same three-opcode
QC arithmetic/return fixture. Three median/p99 frame times were 19.960/24.490,
21.390/29.160 and 20.180/24.770 microseconds. Each returned checksum
821,631,974,078,520, zero calling-thread allocations/reallocations/requested
bytes and zero ordinary-loop hook instructions; allocator positive control
one. This new QC workload has no previous comparable Rust baseline and
does not qualify native module gameplay or driver/worker heap.

Evidence: `$XDG_CACHE_HOME/qa-rust/THE-793-quakec-20261009/` or the default
`.cache/qa-rust`, `checks.json`, `reference/comparison.json`, `retail.json`,
`timing-{1,2,3}.json` and `qvm-hooks-abba.json`. All 5,136 defined seeded rows
match unmodified PR_EnterFunction/PR_LeaveFunction/PR_ExecuteProgram: 64 global
words and 192 entity bytes per row. This includes signed-zero branches, raw
strcmp results, overlapping component/return copies, native local restoration,
mission-pack parameters outside the saved-local span and OP_STATE. The five
retail QC/CSQC images load with zero invalid prepared statements; no retail
entry point is executed. The original 8,151 QVM rows remain exact.

The shared hook/numeric implementation was also checked on the existing QVM
engine-call workload against its saved 6828f011 probe, CPU23 ABBA with 60+600
frames each. Median of the two run medians: 149.1125 us before, 149.145 us after
(+0.0218%). Individual medians overlapped at 148.700/149.525 versus
148.890/149.400 us; individual p99s were 170.320/156.730 versus
157.200/155.650 us. All fixture counters and zero-allocation results matched.
This is no measured speedup claim; the extraction removes duplicate hook and
numeric implementations. Checker, 597 workspace tests and Clippy pass. QC
builtins, native entity binding, native execution and installed acceptance remain
open.


### THE-2563/THE-796/THE-863: inert PE image reader

The portable tracked release probes built in 21.63 seconds. This slice is load
work: it does not execute native instructions or bind runtime imports. The
unchanged C port PE headers/relocation/import/export/TLS functions match all
18 retail cases at preferred, +1 MiB and -1 MiB bases, including 17,019,744
output bytes. Fixtures cover export aliases/ordinals, full-width pointers,
malformed headers/mapped gaps, import hints/ordinal bits/termination, split-word
relocations and inert TLS callback addresses. ELF and native runtime execution
remain open; no native host or performance qualification is claimed.

The existing typed-services workload was rerun on CPU23, 60 warm-up plus 600
measured frames, 64 groups per frame. Median/p99 ns were 105220/140550,
107670/125890 and 105870/120240. Each run had 42240 publications, checksum
27941760, zero measured caller heap/requested bytes and allocator positive
control one. No workers ran in this headless workload; the reader's load-time
allocations are outside its measurement. These timings describe the existing
services fixture, not native module execution or a loader speedup.

Evidence: `~/.cache/qa-rust/THE-2563-pe-20261009/` contains the comparison reports,
retail inspection, build/check logs and services allocation/timing rows. No
full private renderer qualification was repeated for this unused cold reader.
The separately accepted installed development build remains `a1d32b8c`;
its six private map/backend smoke runs are in
`~/.cache/qa-rust/THE-863-install-20261009/smoke-installed/`.

## Core-first engine acceptance refresh (2026-10-09)

THE-611/THE-709/THE-2875 delete the unused core Entity/Think aggregates.
EntityTable's SoA columns remain the only entity/think storage; the null-think
fixture now writes those columns directly. No production execution, wire fields,
timing arithmetic, or native callback widths change. This is dead API removal,
not a claimed performance optimization. THE-3169 under THE-863 retains native
module and installed acceptance for the primitive/event issues.

Fresh main `230ac80c` plus this deletion passes the unchanged checker, 601 workspace
all-target tests and tracked Clippy. Portable tracked release examples built in
37.221 seconds. CPU 23, no debugger, 60 warm-up / 600 measured frames, A/B/B/A
against copies of the pre-deletion release probes:

| Workload | Before / after mean median ns | Before / after mean p99 ns | Median change |
| --- | ---: | ---: | ---: |
| entities | 800 / 790 | 1075 / 885 | -1.250% |
| think-dense | 75835 / 75545 | 93560.5 / 89240 | -0.382% |
| names | 11125 / 11145 | 15870.5 / 16200 | +0.180% |
| hud | 8455 / 8712.5 | 12665 / 13580 | +3.046% |

All non-timing receipt fields match in each ABBA workload, including fixture
counts/order, timestamps, checksums, exact-name identity, cache reuse and
allocation controls. Every measured allocation/reallocation/byte field is zero.
These small positive/negative timing changes are recorded; no speedup is claimed.

Fresh original-C comparison tools pass 27,367 lifetime rows, 12,712 target rows
and 13,390 think rows. Their reports retain their native-width and fixture limits.
The linked-hit merge and transformed-model comparisons pass for all three caller
rules, including their byte/field mutation controls. Kernel/model comparisons
are geometry fixtures; they do not prove installed native scene trace logs.

The current client probe exercises 512 rows and 600 reconnects of ClientId 511,
with stable inventory storage and zero heap activity. The HUD probe checks
checksum 892087680; the sparse entity probe checks 38,400 ordered callbacks and
zero unnamed target refreshes at capacities 128, 1024, 8192. Shared attachment
transport moves 4,914,600 bodies and performs 8,192 link/unlink cycles per measured
frame with unchanged pose/link fixtures and zero heap counts. Core's safe scratch
probe counts its allocation/reallocation positive controls and zero measured
heap calls. These are instrumented calling-thread results, without workers,
foreign heaps, native modules, live touch callbacks or game audio.

Evidence: developer cache `core-priority-20261009/`, including
`post-delete-checks.json`, `final-probes.json`, `lifetimes/verification.json`,
`targets/report.json`, `thinks/report.json`, `merge/result.json`, `models/`, and
raw probe logs. `primitive-checks.json` retains one rejected invocation of a
nonexistent target_reference example; `primitive-checks-corrected.json` records
the corrected target_native build and successful comparisons. See
[core-adoption.md](core-adoption.md) for current callers and deferred bypasses.

### Event-system engine acceptance refresh

The same CPU 23 portable tracked examples use 60 warm-up and 600 measured frames.
These current-run medians/p99 are bounded fixture measurements, not new speedup
comparisons or installed gameplay timings:

| Workload | Median ns | p99 ns |
| --- | ---: | ---: |
| Queue/seat dispatch plus loopback UDP | 1,990 | 2,050 |
| Four-client FIFO, 8 KB/64 KB payloads, counted Full | 7,240 | 8,750 |
| 64 mixed-rule human/bot intents | 1,490 | 1,550 |
| Com_Frame, reliable stalled peer plus healthy peer | 2,270 | 2,810 |
| Com_Frame, unsent stalled peer plus healthy peer | 2,270 | 2,820 |
| Com_Frame, bind/alias/console/bot/local packets/output | 185,145 | 315,651 |

Every measured probe counts zero instrumented calling-thread Rust heap and
validates its positive control. The host has two drains and one output pass per
frame, 1,396 packets, 659 repeats, 12,288 measured bot commands, and provider/world
counters [210, 105, 421, 210]. These repeats are synthetic headless events; real X
repeat evidence remains in the earlier private receipts and deferred installed
acceptance. The event/UDP probe delivers 660 packets and 660 characters. FIFO
checks 107,448,000 payload bytes, 8 messages and 4 Full responses per fixture frame.

Both stalled-peer host cases retain continuous healthy/server progress through
all 600 measured frames: 2,304 healthy deliveries, 1,650 SERVER/provider ticks,
module deliveries [2292, 2300, 2304], zero stale text or healthy overflow. The stalled
consumer receives no ACK, enters exactly one bounded resync, cancels 32 retained
records and skips 2,272 while resyncing. Reliable and unsent cases are separate;
real native channel receipts are not exercised. Disconnecting the HUD does not
invalidate the slower module's retained payload.

Fresh unchanged original tokenizer/separator comparisons match 10,060 records
across Q1/QW/Q2/Q3. Native key-name/hold comparison matches 364 canonical key
cases and 10,000 frames, including 31,005 two-key holds. Existing workspace tests
cover bare cvar argv 1, command priority, aliases, overflow atomicity, button
release/focus loss, startup duration, two physical polls, native millisecond cap
boundaries and per-client loopback backpressure. An isolated planted
Instant::now outside platform is rejected by the unchanged checker; its clean
baseline passes. No new checker rule was added.

Evidence: `core-priority-20261009/final-probes.json`, `final-{host,output,
system-events,loopback,usercmd}.log`, `console-comparison/`, `bind-comparison/`,
and `clock-fixture.json`. Native signon, exact legacy channel framing, real ACKs,
stock HUD/audio/effects and installed walks are consolidated in THE-3169 under
THE-863. The explicit character editing and direct local command submission
sites are listed in [core-adoption.md](core-adoption.md).


## THE-916: exact eye-plane wall at the base1 spawn camera

The retail CPU benchmark's unchanged startup reject gate failed on the existing
base1 renderer at `d7720be1`. A cold, single-surface draw attributes the rejection
to face 2154, `e1u1/ggrat4_4`, whose x=128 plane contains the native spawn camera.
It has five non-collinear vertices, flags zero, texture extents 32x96 and valid
style-zero lighting. This is drawable geometry, not an empty-face exemption.
At the copied owner's 120-degree FOV, projected f32 rounding produces a tiny
area; native gradient construction then divides by zero. The indexed path now
skips an exact eye-plane wall before projection. Other invalid gradients and
unsupported materials still count as rejections.

The retail-coordinate regression reproduces one rejection before the change,
then verifies no rejection or pixel writes at the eye plane and successful
indexed writes from an offset camera. Q2 `ref_soft/r_bsp.c:573-600` admits world
plane faces only outside `BACKFACE_EPSILON`, so its exact zero-distance case
also contributes no coverage. This narrow correction does not claim complete
native culling or presentation parity.

Evidence directory: `THE-2866-cache-20261009`. The original failure and its
single-face attribution are in `before-1-base1/` and
`diagnose-final-base1/runtime.log`. `fixed-baseline-source.json` records the
`d7720be1` source extraction with only this correction and its regression test;
`fixed-baseline-build.json` records passing workspace tests and a portable
release build (34.237 s). Its private 640x400 spawn-camera draws pass all three
maps at one and eight bands, after 60 warm-up and 600 measured frames per leg,
with zero measured caller/worker Rust heap activity and normal exit. Profiles
and copied candidates remain unchanged; no owned PIDs remain. These developer
CPU draws do not qualify an install, gameplay, stock HUD, GL or native visual
parity. The source snapshot and receipts preserve the comparison boundary for
the separate caching work.


## THE-862 / THE-2866: grouped cache spans and parallel preparation

Static indexed, RGB-product and independent-factor spans now resolve assets and
borrow rover texels per surface/mip group within a bounded scanner flush.
Changing stages retain the common stage executor. Group membership uses core
StampSet; overlay barriers, each surface's span order and per-span mip choices
remain native. Replaced static per-span consumers and the raster-only job API
are deleted; the app and developer draws use the same scoped CPU dispatcher.

One reference preparer serves serial and parallel execution. Fixed contiguous
chunks own disjoint load-sized outputs; one view-owned color table is resolved
before workers run. Ordered merging adjusts stage/coverage ranges and preserves
primitive order, draw ranks, depth keys and native counters. Static admission
and actual output checks select bounded serial fallback for oversized chunks.
The final scheduling data requires 64 loaded boundary primitives per job:
smaller views stay serial, avoiding the extra worker barrier. The exact-row
fixtures force parallel execution separately from that performance policy.

Portable release, 640x400, fixed native spawn camera, copied owner profile
(120-degree FOV), 60 warm-up plus 600 measured draws per ABBA leg. One band pins
the caller to CPU23; eight bands use mask16-23 with seven workers and the caller
participating. Workers inherit the process mask; individual workers are not
pinned. The following numbers pool each variant's two 600-frame legs. These
are CPU preparation/raster timings, excluding presentation, input, movement,
native heap and gameplay. Diagnostic nested timers are disabled.

| Map | Bands | Before median / p99 ms | After median / p99 ms | Median / p99 change |
| --- | ---: | ---: | ---: | ---: |
| e1m1 | 1 | 1.976 / 2.001 | 1.843 / 1.888 | -6.72% / -5.65% |
| base1 | 1 | 4.576 / 4.618 | 4.345 / 4.381 | -5.06% / -5.12% |
| q3dm1 | 1 | 14.597 / 14.873 | 13.523 / 14.016 | -7.35% / -5.76% |
| e1m1 | 8 | 0.743 / 0.995 | 0.701 / 0.870 | -5.64% / -12.58% |
| base1 | 8 | 1.149 / 1.691 | 1.098 / 1.621 | -4.44% / -4.14% |
| q3dm1 | 8 | 6.641 / 7.151 | 5.837 / 6.457 | -12.10% / -9.71% |

All 24 private legs exit zero, preserve copied candidates and owner profiles,
and leave no owned PIDs. Exact RGBA/depth bytes, immutable workloads and native
polygon/span/pixel/stage counters match. Calling-thread and every-dispatch worker
Rust allocations, reallocations and requested bytes are zero in measured frames.
This proves the instrumented Rust path; SDL/driver heap work is excluded.

| Map | Bands | Cache hits per frame before / after | Measured fills / evictions / rejects |
| --- | ---: | ---: | ---: |
| e1m1 | 1 | 3712 / 123 | 0 / 0 / 0 |
| base1 | 1 | 6278 / 211 | 0 / 0 / 0 |
| q3dm1 | 1 | 19772 / 1754 | 0 / 0 / 0 |
| e1m1 | 8 | 3712 / 195 | 0 / 0 / 0 |
| base1 | 8 | 6278 / 289 | 0 / 0 / 0 |
| q3dm1 | 8 | 19772 / 1948 | 0 / 0 / 0 |

The eight-band preparation arrays occupy 39,042,200 / 49,170,076 / 89,544,700
bytes for e1m1/base1/q3dm1 (37.23 / 46.89 / 85.40 MiB), including one color table
and all private outputs. This is separate from the unchanged total 32 MiB rover
budget, span groups, coverage bins and shared immutable catalogs. Dividing
output capacity across jobs removes 74-75% of the full-output prototype's
preparation storage. Canonical color tables are counted once.

Earlier results are retained: grouped-only `counted-abba-summary.json` missed
two timing rows; `parallel-abba-diagnostic-summary.json` records exact pixels but
an omitted static-stage counter merge; `bounded-abba-summary.json` and
`final-abba-summary.json` miss tail/median rows. Concurrent compilation was
observed, with base1 baseline medians varying 1.076-2.374 ms; it does not establish
that every miss was contention. The final minimum-work policy specifically
addresses the repeatedly slower e1m1 parallel-preparation tail.

Evidence directory `THE-2866-cache-20261009`: final `grain-abba-{raw,summary}.json`,
per-leg raw frames and private receipts, `grain-checks-build.json` (checker,
602 workspace tests), `grain-final-build.json` (Clippy and 34.191 s release build).
The Q3 eight-band result still exceeds the under-4-ms R12 target. Native visual
parity, gameplay and installed acceptance are not inferred from these draws.

The final direct diagnostic records Q3 preparation median/p99 of 2.317/2.873 ms
at one band and 1.412/1.823 ms at eight bands. The eight-band view runs exactly
one preparation dispatch with eight jobs, followed by two raster dispatches;
one-band preparation has no worker dispatch. Nested times fit within total
draw time, RGBA/depth remain exact, and measured caller/worker Rust heap remains
zero. These are diagnostic stage samples, separate from the normal ABBA timing
gate. Evidence: `grain-prepare-profile-summary.json` and
`grain-profile-q3dm1-{1,8}/`. The normal tracked app build took 27.123 s; its
private three-map CPU(auto)/GL checks are pending at this commit boundary.


The finished-slice normal app matrix at `09243a9e` then completed all six
private CPU(auto)/GL launches, rendering e1m1, base1 and q3dm1 and exiting 0.
Each copied candidate/profile remained unchanged and every recorded owned PID
was cleaned. The CPU selected eight bands with seven workers; the total rover
budget stayed 32 MiB. All six allocation gates counted zero allocations,
reallocations and requested bytes across the caller and all workers. These
integration runs used 60 warm-up plus **660** measured frames, not the frozen
ABBA workload's 600. Screenshots show each map; no native-image parity or
input/gameplay/HUD/audio/network/save acceptance is inferred. No installation.
The Q1 profile reported seven unsupported settings; Q2/Q3 reported zero.

| Normal app 640×400 | CPU auto draw median / p99 ms | Mesa software GL draw median / p99 ms |
|---|---:|---:|
| e1m1 | 0.950 / 1.265 | 5.713 / 7.430 |
| base1 | 1.332 / 1.761 | 11.202 / 13.147 |
| q3dm1 | 104.052 / 121.851 | 18.122 / 21.602 |

GL driver: llvmpipe (LLVM 22.1.8, 256 bits), GL 4.6 Core Profile,
Mesa 26.2.2-arch1.1. These software GL rows do not measure hardware GPU speed.
The normal app uses live shader time and includes scene/client presentation;
these draws are not fidelity-matched before/after speedup measurements.
The live q3dm1 row is materially slower than the frozen cache checkpoint:
its final frame records 1,845 RGBA cache hits, 47 fills and 45 evictions,
with zero rejects; cumulative including startup/warm-up is 34,121 fills and
32,132 evictions. The cause remains unverified. Static-scene zero fills do
not establish live cache reuse. This gap remains open under THE-862.
Evidence: `grain-normal-summary.json`, `grain-normal-matrix.json`, and
`grain-normal-{cpu,gl}-{e1m1,base1,q3dm1}/{runtime.log,result.json,window.png}`
under the same THE-2866 evidence root.


### THE-860 / THE-923 message-codec checkpoint, 2026-10-09

One borrowed bounded reader/writer now supports native little-endian byte/OOB
messages, raw LSB-first bit streams and Q3's fixed MSG Huffman stream.
The numeric Q3 table, including its reserved NYT symbol, comes from unchanged
qsrc `huffman.c` and `msg.c` `msg_hData`. No C code is linked into the engine.
`tools/check_message.py` compiles the original functions only for comparison.
768 seeded message cases containing 49,152 fields match original C output bytes
and decoded values exactly. The original OOB WriteLong's unused bit counter
advances eight rather than 32; comparison reports its actual byte cursor, without
changing the original function. Native scalar/float bit patterns, signed widths,
OOB signature, guarded capacity failure and malformed-message offsets have
focused checks. This is a codec checkpoint; adaptive connect compression,
channel/reliable retirement, delta fields, captures and live clients remain open.

The portable release probe built in 7.938 s. On CPU23, 16 fixed mixed-encoding
peers encode/decode 64 fields each over 60 warm-up and 600 measured frames.
Median/p99 is 15.765/19.720 microseconds, 2,274 packet bytes/frame and 675,840
checked decoded fields including warm-up. The allocation positive control
counts one; measured allocations/reallocations/requested bytes are all zero.
This measures Rust codec work only, with no channel, workers or native heap.
Workspace 607 tests, Clippy and the unchanged checker pass. Evidence:
`THE-860-message-20261009/{comparison.json,fixture.bin,original.bin,rust.bin,
build.json,workspace.json,timing.json}` under the local QA evidence cache.
The owner's 13:49 ruling returns priority to THE-862's live-cache blocker before
further THE-860 work.

### THE-862 / THE-3171: live cache attribution

The normal-app q3dm1 case was sampled on the same copied 09243a9e candidate,
640x400, automatic eight bands, mask16-23 and copied owner settings. Owned
Xvfb/X11 and dummy audio isolate the run. `perf record` attached only to the
recorded owned game PID after ten seconds of rendering: cycles:u, 997 Hz,
15 seconds, DWARF call chains. It recorded 16,880 samples with no lost samples.
Profiler-run timings are discarded. Nearest repeat sampling accounts for
28.04% of self samples, bilinear clamp sampling 22.42%, and the span-group
consumer (including inlined cache-fill code) 21.98%; roundf, fmodf and linear
lightmap taps account for another 5.13%, 4.65% and 4.34%. Resolved call chains
include Product fill and prepare_rgba. Some DWARF chains are truncated, so
these self percentages are not exclusive nested wall-time attribution.

Bounded counters in the one rover distinguish successful absent-slot fills
from changed-input refills, count filled/evicted payload bytes, and report
resident bytes. A normal-app diagnostic build (portable release, proof input
disabled, 38.845 seconds) then ran 60 warm-up plus 120 measured frames with the
same private matrix. All 9,461 cumulative fills were nonresident fills;
changed-input refills were zero. 762,007,192 payload bytes were filled and
744,939,536 evicted. The final frame again has 47 fills and 45 evictions.
Resident payload is 17,067,656 bytes across eight independent 4 MiB arenas;
fragmentation and uneven band usage mean this is not the total working-set
requirement. Median draw is 103.919 ms. Thus this run demonstrates repeated
residency loss rather than shader-time invalidation of static keys.

The catalog's conservative aligned reservation sum over every registered mip
is 264,146,144 bytes. This includes optional recipes and layouts and is not a
visible-view estimate or a per-band residency guarantee. The diagnostic does
not change the 32 MiB total budget or recipe eligibility. Native-state fixtures
check both resident-refill classifications and indexed/RGBA payload counts.
The unchanged checker, 607 workspace tests and Clippy pass. The private run
exits zero, preserves the owner's profile and copied candidate, cleans every
owned PID, and measures zero Rust heap activity across the caller and seven
workers. SDL/driver heap and gameplay remain outside that allocation proof.

C-port source inspection finds a separate fixed 32 MiB cache at
`src/render/cpu/surface_cache.c:9`, with stamp-based resident reuse and rover
allocation in `cpu_surface_cache_prepare`. This establishes a similar fixed
capacity policy, not a measured C-port regression. No C runtime claim follows.

Evidence directory: `THE-3171-live-cache-20261009`, including `perf.data`,
`perf-record.log`, `perf-self.txt`, `perf-report.txt`, `profile-receipt.json`,
`diagnostic-build.json`, `diagnostic-workspace.log`, `diagnostic-clippy.log`,
`diagnostic-summary.json` and `diagnostic-q3dm1/{runtime.log,result.json,window.png}`.
The owner's 13:49 ruling now permits load-time map-working-set sizing; the
budget correction and matched live timing remain the next step.

### THE-862 / THE-3171: load-sized rover budget

The default CPU budget now comes from the loaded maps' registered, aligned mip
reservations. It has a 32 MiB floor, also fits the largest mandatory surface in
every selected band, and rounds the total to eight-byte band shares. The one
rover, cache keys, spans and animated-stage executor are unchanged. The app and
developer draws both use this policy; a nonzero CpuLimits.cache_bytes remains
a fixed diagnostic override. The earlier fixed-budget policy is superseded by
the owner's 13:49 ruling. Recipe admission no longer rejects a static chart
merely because the former total budget was 32 MiB. No sampler arithmetic,
shader clock or animation was changed.

A portable normal-app build took 34.469 seconds, with allocation tracking and
proof input disabled. Its build receipt identifies 447d15f9 plus the recorded
source diff (`source_tree_dirty=true`); it is not an installed candidate.
Private normal-app CPU comparisons use 640x400, copied owner settings, automatic
eight bands/seven workers on mask16-23, 60 warm-up and 600 measured frames per
leg. Initial time-zero screenshots have identical decoded RGBA bytes in all
before/after legs for all three maps. Live shader time remains active; these
screenshots do not prove that differently timed later animation frames match.

| Live CPU draw | Order | Before median / p99 ms | After median / p99 ms | Median / p99 change |
|---|---|---:|---:|---:|
| q3dm1 | ABBA | 103.363 / 105.875 | 5.264 / 6.164 | -94.91% / -94.18% |
| e1m1 | BAAB repeat | 1.210 / 1.828 | 1.196 / 1.642 | -1.11% / -10.20% |
| base1 | BAAB repeat | 1.867 / 3.205 | 1.863 / 2.943 | -0.19% / -8.18% |

The Q3 live median is below the earlier frozen eight-band checkpoint's
5.837 ms, satisfying the owner's roughly 25% proximity gate. This compares
different live/frozen workloads, not an additional fidelity-matched speedup.
The separate under-4-ms R12 target remains unmet.

The first e1m1/base1 ABBA series failed the no-regression gate: e1m1
0.886/1.354 -> 0.993/1.429 ms (+12.13%/+5.53%), base1
1.285/2.217 -> 1.312/2.759 ms (+2.14%/+24.43%). It is retained in
`live-abba-summary.json`. The unchanged candidate's reversed-order repeat above
has no median or p99 increase. Concurrent unowned cc1 processes were observed
with affinity0-95 during that repeat; they were not stopped. This observation
does not establish the cause of every earlier timing increase. The spread
limits precision of small e1m1/base1 percentage comparisons.

| Map | Total / band arena bytes | Resident payload bytes | Cumulative cold fills | Changed-input refills / evictions | Final fills / evictions |
|---|---:|---:|---:|---:|---:|
| e1m1 | 73,270,528 / 9,158,816 | 610,120 | 200 | 0 / 0 | 0 / 0 |
| base1 | 87,410,176 / 10,926,272 | 1,062,864 | 353 | 0 / 0 | 0 / 0 |
| q3dm1 | 264,146,176 / 33,018,272 | 17,630,960 | 2,011 | 0 / 0 | 0 / 0 |

These counters include startup and warm-up. The Q3 before legs each fill
2,945,692,504 bytes and evict 2,928,624,848 bytes in 661 rendered frames;
after legs fill 17,630,960 bytes once with no evictions. Q3's 47-fill/45-eviction
final frame becomes zero/zero. The map-sized bound is conservative: it includes
every mip and optional indexed/RGB layouts, and therefore allocates much more
space than this view's resident payload. It is a total budget divided among
bands, not a guarantee that duplicated band copies fit for every possible view.
All storage is sized at load; no arena grows during frames.

All 20 CPU legs and three finished-slice GL checks exit zero, preserve the
owner's profile and copied binaries, clean every owned PID, and report zero
measured Rust allocations/reallocations/bytes across the caller and workers.
The same existing load fixture now checks that an 8 MiB native chart keeps
eight automatic bands with map sizing but reduces to four under an explicit
32 MiB budget. Fixed-budget rejection and byte-exact cache/span fixtures remain
in place. The checker, 607 workspace tests and Clippy pass.

The three GL rows are Mesa **software GL**, not hardware GPU measurements:
llvmpipe (LLVM22.1.8,256bits), GL4.6 Core Profile, Mesa26.2.2-arch1.1.
At640x400 their draw median/p99 ms are e1m1 6.625/8.967,
base1 14.303/15.810, q3dm1 19.401/24.100. They prove rendering and normal exit,
not hardware targets, native-image parity or gameplay. No install, stock HUD,
module/network, audio or save acceptance is claimed by this cache slice.

Evidence directory `THE-3171-live-cache-20261009`: `map-budget-build.json`,
`map-budget-source.patch`, `map-budget-boundary.log`,
`map-budget-final-workspace.log`, `map-budget-final-clippy.log`,
`map-budget-checker.log`, `live-abba-{raw,summary}.json`,
`repeat-baab-{raw,summary}.json`, `gl-matrix-raw.json`, and per-leg
`{runtime.log,result.json,window.png}` under `live-abba-*`, `repeat-baab-*`
and `map-budget-gl-*`. Earlier profiler and diagnostic receipts remain beside
these. THE-862 still owns broader material/presentation work and R12's target.

### THE-860 / THE-949 native header checkpoint

One connected-packet header codec uses the shared message reader/writer and
load-selected layout data. It preserves NetQuake's big-endian length/flags and
independent sequence, QW/Q2's reliable-toggle/ACK words, client-only qport,
negotiated q2repro short/byte/omitted qport, Q3's offset/length fragments and
Q2pro's offset continuation bit. Q3 does not acquire an invented header ACK.
The parser borrows the payload, checks native declared lengths, and returns
scoped errors for connectionless or truncated packets. The caller supplies its
fixed send buffer/MTU; receive admission can retain legacy sizes such as QW's
1450-byte payload plus ten-byte client header.

`tools/check_network_headers.py` compiles extracted original NQ, QW client and
server, Q2, q2repro old/new and Q3 transmit functions. NetQuake ACK construction
uses its original GetMessage statements. Transport, cvars, byte writes and cold
channel structs are harness bindings; no C is shipped. 2,304 seeded cases in
18 layout/direction modes match complete native header/payload bytes exactly.
The Rust decoder also checks projected fields and borrowed payloads. Focused
fixtures check known native byte vectors, qport omissions, Q3's zero-length
final fragment, Q2pro continuation, declared-length truncation and connectionless
classification. This is header construction/parsing evidence, not a channel
loss, reliable-delivery or original-client connection proof.

Portable release build 5.150 seconds, source54de74ba plus the recorded header
slice, proof input disabled. CPU23, 60 warm-up/600 measured frames, 16 mixed
native encode/decode peers: median **830 ns**, p99 **890 ns**, 4,726 packet bytes
per frame and 10,560 checked packets including warm-up. Inputs and encoded
packets pass optimisation barriers before validation. Calling-thread Rust heap
allocations/reallocations/requested bytes are zero; the allocation positive
control counts one. No workers, native heap, transport or gameplay are measured.
The unchanged checker, 611 workspace tests and Clippy pass. No private game or
installation was required for this bounded codec checkpoint.

Evidence directory `THE-949-headers-20261009`: `comparison.json`, extracted
`original-*.c`/executables, `fixture.bin`, original packet bytes and
`rust-packets.bin`, `build.json`, `final-build.log`, `final-timing.json`,
`final-workspace.log`, `final-clippy.log`, `final-checker.log` and the copied
`network-headers` probe. Earlier unguarded timing remains `timing.json` and is
not the final gate. Shared channel state, native reliable receipts, fragment
assembly, field-table deltas and migration of host direct submit/output ACK
adapters remain the next THE-860 work; no network issue is closed by this slice.

### THE-860 / THE-949 connected receive checkpoint

One load-sized `Channel` selects native receive policy independently of game
roles. It borrows complete packet payloads and uses one owned assembly buffer
for ordered fragments. NetQuake's reliable and unreliable sequence spaces stay
separate; reliable DATA packets generate native ACK replies even when duplicate
or out of order. The replies use the existing core typed payload FIFO, now
public for capability consumers. QW/Q2 toggle state changes only on new packets.
Q2pro observes reliable ACK bits before fragment completion or offset rejection,
as its original does. Q3 has no header ACK and a 1300-byte fragment still needs
a shorter final fragment, including a zero-length terminator for exact multiples.
Oversize input and full control storage fail only the affected receive operation.

`tools/check_network_receive.py` compiles the unchanged original receive
functions from WinQuake, QW client/server, Q2, q2repro old/new and Q3. Cold
structs, byte reading, already-matched peer addresses, clocks and transport are
harness bindings; no C is shipped. Across 2,048 seeded transcripts in 16
policy/direction modes, **22,528 packet rows** match delivered payload bytes,
incoming sequence/reliability and fragment state, drop counts and native ACK
reply bytes. The transcripts include stale, duplicate, lost/out-of-order,
fragment-prefix reuse and empty-final packets. Six focused Rust tests cover
the bounded error paths as well as native delivery cases.

The initial portable release example build reported 15.36 seconds; its final
cached rebuild measured 0.083 seconds. Source4ffe786a plus the recorded receive
slice, allocation tracking enabled, proof input disabled. CPU23, 60 warm-up and
600 measured frames, 16 mixed peers: header construction, sequence processing
and ordered receive assembly measured **1,040 ns median / 1,100 ns p99**.
The workload checks 14,520 packets, 10,560 deliveries and 7,340,520 delivered
bytes including warm-up. Inputs pass optimisation barriers. Calling-thread
Rust allocations/reallocations/requested bytes are zero; the allocation positive
control counts one. There are no workers, physical transport or game modules
in this probe. The unchanged checker, 617 workspace tests and Clippy pass.

This checkpoint exposes native controls without validating them against any
outgoing flight. It cannot retire a reliable output record. Transmit/resend
state, reliable command storage, Q3 payload ACK/XOR, delta tables, handshakes,
host/loopback caller migration and original-client live connections remain open
on THE-860/THE-949. No installation or gameplay proof is claimed.

Evidence directory `THE-949-receive-20261009`: `comparison.json`, extracted
`original-*.c`/executables, grouped `*-fixture.bin`, `*-original.bin` and
`*-rust.bin`, `network-receive`, `build.json`, `final-build.log`, `timing.json`,
`workspace.log`, `clippy.log`, `checker.log` and `committed-source.patch`.

### THE-860 / THE-949 native transmit checkpoint

The same Channel now owns a bounded reliable-record FIFO, one native flight,
fragment storage and a fixed 1400-byte output packet. The existing core FIFO
provides indexed reads for batch construction; no second queue implementation
was added. A prepared packet survives rejected transport admission unchanged.
Sequence advancement follows successful submission. NetQuake reliable retry
uses its own last-send clock, separate from unreliable packets, and advances
fragments only on the matching native ACK. QW/Q2 preserve toggle ACKs and their
classic next-sequence resend threshold; q2repro uses its current-sequence
threshold. Q2pro fragment flags retain its existing-flight behavior. Q3 keeps
1300-byte fragments and an empty final packet for exact multiples.

Native ACK receive processing returns the internal receipts of retired NQ/QW/
Q2 records; receipts never enter the wire or come from a transmit watermark.
They are consumed before the next receive. Q3 command reliability still belongs
to its payload layer: this checkpoint rejects its generic reliable enqueue and
cannot fabricate a header receipt. Eleven focused tests include queue overflow,
transport rejection, stale/duplicate ACKs, batched final-ACK retirement, pending
Q2pro ACKs, offset-width limits and an ACK arriving during a blocked resend.

`tools/check_network_transmit.py` compiles unchanged original send and receive
functions with cold transport/clock/byte-buffer bindings. Across 1,024 seeded
transcripts and 16 policy/direction modes, **13,312 operations** match outgoing
packet bytes and native sequence, reliable-flight and fragment state exactly.
Queued native message input is assembled by the tooling; NetQuake's next send
released inside GetMessage is normalized to the shared channel's deferred
prepare/submission boundary. Rejected transport is covered by Rust fixtures,
not claimed as original-C equivalence. The unchanged receive comparison still
matches all 22,528 rows after this extension.

Portable release example rebuild **8.370 seconds**, source4a738aab plus the
recorded transmit slice, empty RUSTFLAGS, allocation tracking enabled and proof
disabled. CPU23, 60 warm-up/600 measured frames, 16 mixed sender/receiver peers:
**4,140 ns median / 4,300 ns p99** for header encoding, prepare/submission,
ordered receive and NQ/QW/Q2 native ACK receipts. Including warm-up: 33,000
packets, 10,560 delivered messages, 15,079,680 delivered bytes and 9,240 reliable
receipts. Calling-thread Rust allocations/reallocations/requested bytes are zero;
positive control one. No workers, physical transport or game modules are timed.
The final receive-only ABBA average median/p99 is 1,075/1,150 ns before versus
1,062.5/1,110 ns after, with identical counts and zero heap on every leg.
The checker, 628 workspace tests and warning-denied Clippy pass.

Evidence directory `THE-949-transmit-20261009`: `final-native/comparison.json`,
`final-receive/comparison.json` and their extracted C/fixture/output bytes,
`network-{transmit,receive}`, `build.json`, `final-build.log`, `timing.json`,
`final-receive-abba.json`, final workspace/Clippy/checker logs and
`committed-source.patch`. Earlier pre-boundary receipts are retained separately.
Q3 command ACK/XOR, native rate/handshake policy, field-table deltas, snapshot
rings and host/loopback adoption remain open. No install, loss-simulation live
run, original-client connection or full network acceptance is claimed.

### THE-860 / THE-949: queued native ingress and output ACK adoption

The host replaces its counting-only PacketReceiver with one load-sized
connection table. Each client can bind independent client/server Channel
endpoints; loopback and socket Packet events use the same router. The admitted
native NQ/QW/Q2 receipts retire the binding's original output-consumer generation
inside ordinary host dispatch. A delayed ACK cannot target a replacement client
consumer. Unknown routes do not mutate channels. Q2's early native ACK survives
rejection of its fragment body. No physical intake point was added.

Six focused tests cover native ACKs through the actual host queue, one peer
stalled while another retires, unsent and duplicate ACKs, consumer generation
reuse, socket/loopback route isolation, invalid bindings and Q2 early ACKs.
The unchanged checker, 634 workspace tests and warning-denied Clippy pass.

Portable release example build: **16.691 seconds**, e486a2ab plus the recorded
slice, empty RUSTFLAGS, allocation tracking enabled and proof disabled.
`network_ingress`, CPU23, 60 warm-up / 600 measured iterations, 16 mixed peers
and two queued-memory drains per iteration: **3,610 ns median / 3,700 ns p99**.
Including warm-up: 19,140 packets, 10,560 nonempty payloads and 8,580 output
records retired by native ACK. Rust caller allocations, reallocations and
requested bytes are zero; positive control one. The probe includes publication,
prepare/accepted loopback submission, SysEventQueue dispatch and output-ring
retirement. Q3 tests framing only, with no fabricated header ACK. No workers,
OS intake, module decoding or gameplay are measured.

The existing host workload was rebuilt from exact archived e486a2ab source
using the same release settings, **23.829 seconds**. CPU23 60+600 ABBA average
median/p99: **6,732.5 / 12,885 ns before**, **7,010 / 14,705 ns after**.
Median increased **4.12%**; this is channel-routing adoption, not a speedup.
The four legs retain identical packet, repeat, native-tick and drain counts and
zero measured caller heap. Their 16-ms pacing is outside the measured region.
The earlier single host sample and ingress sample remain separate evidence.

Evidence directory `THE-860-ingress-20261009`: `focused.log`,
`routing-tests.log`, `final-workspace.log`, `final-clippy.log`, `checker.log`,
`build.json`, `baseline-build.json`, `final-timing.json`, `host-abba.json`,
individual host legs, archived baseline source, copied before/after probes and
`source.patch`. Host direct local command submit and FrameSource remote output
submission still remain for the next integration slices. This checkpoint does
not implement payload codecs, Q3 command ACK/XOR, native handshake, snapshot
deltas or installed interoperability. The accepted installation is unchanged.

### THE-860: shared native command delta field tables

One scalar table walker now writes and reads the QW and Q2 mask records and
Q3 keyed command deltas. Static tables select native field order, signed
widths, float-angle equality and projection, mandatory duration/light bytes,
aggregate change bits and time compression. The Q3 table preserves original
MSG_ReadDeltaKey's kbitmask[bits] extra key bit; native char/byte projections
still narrow at their boundary. No per-protocol delta algorithm was added.

`tools/check_command_delta.py` compiles unchanged original QW/Q2 command
functions, QW angle helpers and Q3 keyed command/MSG/Huffman functions with
cold byte-buffer and struct bindings. **3,072 cases** match encoded bytes and
all decoded record words exactly, including all field-mask combinations,
signed zero, equal-quantized/different-float angles, signed width extremes,
short/full/negative time differences and command keys. Three focused Rust
fixtures also check mandatory trailing bytes and truncated message rejection.

Portable release probe build **8.096 seconds**, source e8112142 plus the
recorded command-codec slice, empty RUSTFLAGS, allocation tracking and no proof.
CPU23, 60 warm-up / 600 measured iterations, 16 encode/decode records per
iteration with original-C fidelity checks: **3,225 ns median / 6,750 ns p99**.
Including warm-up: 10,560 checks and 101,618 encoded bytes. Calling-thread Rust
allocations, reallocations and requested bytes are zero; positive control one.
No workers, physical transport, full packet framing or gameplay are timed.
The unchanged checker, 637 workspace tests and warning-denied Clippy pass.

Evidence directory `THE-860-command-delta-20261009`: `final-original/` contains
the extracted C, fixture, native/Rust output and comparison.json; `build.json`,
`final-timing.json`, copied `command-delta`, focused/workspace/Clippy/checker logs and
`source.patch`. These are command delta records. NQ and rerelease payloads,
full native command packets, host local framing, Q3 XOR/command reliability,
entity/player snapshot tables and installed interoperability remain required.
The accepted installation is unchanged.

### THE-860: native move packets in the ordinary host

Local CLIENT commands now pass through the native payload encoder, Channel,
Loopback and SysEventQueue. Only SERVER Packet dispatch submits a decoded
command; the direct CLIENT-to-SERVER submit caller is deleted. Bindings retain
the original output-consumer generation, so an old connection cannot mutate
a reused client slot. NQ duration comes from the SERVER frame rather than its
ping timestamp; QW/Q2 advance command time by their native byte duration.
Q3 ignores commands at or before its last accepted time.

The release packet comparator matched all 2,048 seeded cases in both encoded
wire bytes and 33 decoded native field words. It compiles unchanged NQ
CL_SendMove, original QW/Q2/Q3 construction blocks, native CRC/delta readers,
Com_HashKey, CL_Netchan_Encode and Q3 MSG/Huff. Cold bindings supply native
struct/global fields and primitive byte IO. Cases include signed movements,
float angles, sequence CRCs, three-command deltas, full-width checksum/challenge
keys, percent/high-byte XOR substitutions and varied native command strings.
`tools/check_command_packets.py` and the platform `command_packets` example
repeat this comparison without ever sending fixture input to a game.

The `command_host` release probe uses the ordinary FrameHost with four
independently selected local protocols 15/28/34/68 and movement roles. Four
controller event sources feed the one input builder. On CPU23, after 60 warm-up
frames and over 600 measured frames, it measured **3,950 ns median / 4,030 ns
p99**. The calling-thread allocation counter recorded zero allocations,
reallocations and requested bytes; its one-allocation positive control passed.
No worker dispatch or SDL/driver allocation is measured by this headless case.
Inclusive warm-up counts are 2,636 native packets, 2,635 submitted commands,
1,320 physical intake calls, 660 presents and one skipped initial Q3 command
at time zero. The measured region advanced 600 SERVER ticks. This is a new
framed workload, not a claimed speedup over the former direct-submit path.

The exact final release examples build took **19.247841 seconds**, portable
baseline CPU, empty RUSTFLAGS, fat LTO and allocation-tracking enabled; proof
was disabled. The unchanged checker, warning-denied Clippy and all 643
workspace/all-target tests passed.

The development host explicitly defaults to QW28, independent of map and rule
roles, with `--local-protocol 15/28/34/68` and per-client
`--seat-protocol seat:15/28/34/68` overrides. QW/Q2 native packet APIs accept
three caller-supplied commands; this host has no input history and sends two
zero older commands plus the current one. Its Q3 bootstrap uses an explicit
zero handshake/command context and native weapon NONE. Native Q3 command
history, negotiated keys and reliable command acknowledgements, NQ666/999,
rerelease transport, entity/player deltas, native providers, snapshots and live
interoperability remain required. The FrameSource remote output adapter is the
next adoption target. No map, gameplay, installation or network acceptance is
claimed here; qfiles/qa-rust remains unchanged.

Evidence directory `THE-860-command-packets-20261009`: `final-original/`
contains original C, fixture, original/Rust outputs and comparison JSON;
`final-build.json`, `final-timing.json`, `checker.log`, `workspace.log`,
`clippy.log` and `source.patch` retain the bounded build/check receipts.

### THE-860: native print delivery and output retirement

Remote print/center/layout payloads now encode their connection's NQ15/QW28/
Q2 34 fields, queue in the shared Channel, and transmit through primitive byte
transport. Only an actual native ACK arriving in ordinary Packet dispatch
retires a reliable output receipt. The FrameSource output/resync adapters are
deleted, including the old modeled timing probe. Native priority is an explicit
byte in PrintEvent; QW/Q2 priority 0 and 1 are preserved independently of its
HUD kind. Byte fixtures cover all 256 priorities, raw palette-font bytes, NUL
termination, concatenated records and capacity/unsupported boundaries.
The references are WinQuake host.c SV_ClientPrintf, QW server/sv_send.c
SV_PrintToClient, and Q2 server/sv_send.c SV_ClientPrintf. These fixtures are
source-derived; this slice has no new compiled original-C print comparator.

CPU23, portable release, 60 warm-up / 600 measured frames, zero Rust calling-
thread allocations, reallocations or requested bytes in all probes:

| Ordinary headless workload | Median ns | p99 ns | Fidelity |
| --- | ---: | ---: | --- |
| Sixteen NQ/QW/Q2 peers, print decode and real ACK ingress | 11,370 | 109,530 | 9,600 measured sends, 600 SERVER ticks; 10,560 prints and 10,544 real receipts including warm-up |
| Healthy QW peer plus peer withholding ACKs | 3,550 | 5,200 | 1,152 healthy prints/ACKed records; 600 continuing measured frames |
| Healthy QW peer plus rejected transport | 2,325 | 2,760 | Same healthy prints/ACKs and SERVER progress |
| Four local move protocols through native packets | 4,060 | 4,180 | 2,636 packets, 2,635 commands, 1,320 intake calls including warm-up |

Both stalled cases disconnect the affected connection at frame 20, count one
output overflow and retire 32 bounded records. Their 10/20/40-Hz module
consumers receive 1,146/1,150/1,152 prints, and independent HUD text leases
remain valid. A stale receipt cannot acknowledge the detached cursor. This
is bounded disconnection, not a claim of native reconnect or sign-on resync.
The native retirement probe is print-only; its former modeled sound-and-print
workload is not a comparable speed baseline. The sixteen-peer tail is reported
as measured and is not a renderer/gameplay performance result.

The release example build took 14.88 seconds (Cargo-reported), proof disabled,
allocation tracking enabled. The unchanged checker, workspace/all-target tests
and warning-denied Clippy pass. Developer evidence is in
`THE-860-native-output-20261009/`: build/check logs, channel.json,
retirement.jsonl, command-host.json and the source patch.
Q3 command-window output, sound/effect native mappings, entity/player/config
snapshot fields, full sign-on, rerelease and live interoperability remain open.
Local HUD output still consumes the common in-process event ring. This does
not complete THE-860 or the deferred THE-3169 acceptance.

The owner-requested installer option `--owner-smoke --smoke-movement` adds
real XTest held-key input after initial config, checks key repeats, authoritative
horizontal movement and two ordinary intake phases in each 300-frame map/
backend run, and records the native local protocol explicitly. It uses a normal
candidate without a proof input player. The six fresh runs and install are
performed after this commit; this note does not claim they have passed yet.

### THE-3174: twelve audited primitive adoptions, accepted at 18:49

All twelve audited sites are recorded in [core-adoption.md](core-adoption.md).
The release candidate preserves the compared movement, collision, sky endpoint,
retail RGBA and depth results. The unchanged checker, all 648 workspace tests
and warning-denied Clippy pass. Core remains safe; no checker extension is added.

The initial seven-byte entity policy enlarged TraceQuery to 128 bytes. Packing
its behavior flags restores 120 bytes. An explicit internal query layout also
restores start/end/mins/maxs offsets 68/80/92/104. Stored cvar NameId follows its
numeric columns so numbers/integers retain offsets 72/92 in a 120-byte row.
An alias-ID caching experiment and the extra callback NameId argument were
removed. The final command table holds function callbacks and button actions
under the same numeric name key. Empty cvar refreshes do no dirty-set work.

CPU23, portable release, no debugger, 60 warm-up and 600 measured frames per
leg. Headless rows aggregate five ABBA blocks, ten baseline and ten candidate
legs each. Retail rows aggregate one ABBA block at 640x400, one CPU band and
zero background workers. Each workload and its fidelity counters match; all
measured Rust allocations, reallocations and requested bytes are zero. Retail
RGBA and float-depth files match byte for byte in all four legs.

| Workload | Before median ns | After median ns | Change | Before p99 ns | After p99 ns |
| --- | ---: | ---: | ---: | ---: | ---: |
| movement | 3015734.50 | 3004118.50 | -0.385% | 4179813.00 | 4166683.00 |
| names | 11218.05 | 11332.50 | +1.020% | 14839.00 | 14839.00 |
| q1 | 4026.50 | 4078.00 | +1.279% | 4308.00 | 4411.00 |
| q2 | 21798.55 | 21853.00 | +0.250% | 146207.20 | 151854.10 |
| q3 | 9440.50 | 9535.00 | +1.001% | 76675.20 | 78819.00 |
| e1m1 CPU draw | 1867784.25 | 1845219.25 | -1.208% | 1965556.50 | 1895797.00 |
| base1 CPU draw | 4433988.00 | 4349735.50 | -1.900% | 4554408.50 | 4489748.50 |
| q3dm1 CPU draw | 13767227.50 | 13910185.25 | +1.038% | 15189646.00 | 15358701.00 |

The supervisor accepted `54708d5b` at 18:49 with all twelve audited sites
applied and recorded, and directed main push followed by THE-860. That commit
is now on main and its superseded WIP branch is deleted. Positive median shifts
remain in names, Q1/Q2/Q3 model traces and q3dm1 CPU draw; these measurements
do not establish a strict zero-increase timing gate. Repeated older trials are retained and are not substituted for this
final candidate matrix. Earlier diagnostic runners also have failed logs,
including a twelve-versus-twenty row-count assertion and a failed continuation.
Those incomplete series are not acceptance evidence.

Original-C checks retain 30,000 retail hull rows, 40,632 transformed-model rows,
20,140 brush rows, 1,188 hit-merge rows and 16,384 sky integer endpoints. Another
16,384 sky rows match the replaced Rust arithmetic. Q2/Q3 movement stays
byte-identical to the pre-adoption Rust baseline over 1,152 rows each. Original
Q2 is exact; Q3 keeps its pre-existing 544 float-component differences with
maximum error 0.0000112 and exact flags/timers. THE-3175 tracks attribution
after the current THE-860 step. No new native gameplay, module,
foreign-heap, GL speed, multiplayer or installation acceptance is claimed.

The final example build took 36.67 seconds, Cargo-reported. Evidence is under
`THE-3174-adoption-20261009/`: query-layout-headless/summary.json,
retail-query-layout-summary.json, sky-final-candidate/result.json, the original-C
comparison directories, final checker/workspace/Clippy logs and build logs.

### THE-860: native Q3 reliable command windows through the ordinary host

The one Channel owns a 64-entry native command window in each direction.
Acknowledged strings remain in their native `sequence & 63` slots until reuse,
serving Q3's MSG/Huffman, XOR and usercmd keys. Client packets carry the native
serverId, message ACK and reliable-command ACK; server payloads carry the native
client-command ACK. Commands retire only after native submission and ACK,
including complete fragment flights. No event watermark becomes a wire ACK.

The ordinary app output path now emits Q3 `svc_serverCommand` records for
cp/print/chat through that Channel; Packet ingress dispatches native client
commands and receipts. Snapshot/gamestate payload readers and native client
module execution are still deferred. This step does not claim visible stock
HUDs, native signon, live reference connections or complete THE-860 acceptance.

The extracted original Q3 SV_AddServerCommand, SV_UpdateServerCommandsToClient,
MSG string/Huffman and SV/CL Netchan XOR functions match 512 seeded cases and
1,081,803 output bytes, including high bytes, percent filtering and string
boundary/cursor behavior. Focused tests cover both ACK directions, retained
keys, duplicates/gaps, future/unsent ACKs, 64-command capacity and fragments.
The unchanged checker, 651 workspace tests and warning-denied Clippy pass.
The four release examples built in 24.04 seconds, Cargo-reported.

CPU23, portable release, 60 warm-up plus 600 measured frames per invocation;
five ABBA blocks for each pre-existing host workload. Baseline binaries were
saved before this command-window work at a7ec14a2; the candidate also includes
the accepted THE-3174 primitive adoption. Counters match in every leg, and
allocations, reallocations and requested bytes are zero. These are headless
host/peer-codec timings, including fixture decode/ACK construction; no socket
syscalls, map rendering, native signon or gameplay are measured.

| Workload | Before median ns | After median ns | Change | Before p99 ns | After p99 ns |
| --- | ---: | ---: | ---: | ---: | ---: |
| Four native local command seats | 3966 | 4028 | +1.56% | 4367 | 4628 |
| Sixteen NQ/QW/Q2 output peers | 11618.5 | 12197.5 | +4.98% | 40774 | 39927 |

The new sixteen-peer NQ/QW/Q2/Q3 matrix averages 19,690 ns median and 53,552.5 ns
p99 over four runs. Each receives 10,560 prints, consumes 10,544 real native
receipts (including warm-up), makes exactly 1,320 physical intake calls and
sends 9,600 output packets in the measured frames. This is a different workload
from the classic-protocol row and has no comparable pre-change Q3 output path.

Sixteen additional QW/Q3 ordinary-host runs cover reliable-without-ACK and
unsent peers. Every run keeps a healthy peer and SERVER advancing on all 600
measured frames; 10/20/40 Hz modules receive [1146,1150,1152] events. The stalled
peer alone disconnects at frame 20 after its 32-slot output capacity is reached;
all 1,152 healthy prints retire on real ACKs, with no healthy overflow, stale
texts or measured heap activity. HUD display leases remain independent of
module delivery. This proves bounded host progress and native receipt
retirement in the connected fixture, not reconnect or installed multiplayer.

Evidence is retained in `THE-860-q3-host-20261009/`: original/comparison.json,
abba-summary.json and twenty raw legs per existing workload, q3-output rows,
retirement-summary.json, and checker/workspace/Clippy/build logs. No checker
rule, input history, journal or replay is introduced.

### THE-3175: Q3 float discrepancy attributed and native angle/order fixes

The unchanged original-C fixture at f800e2c1 reproduces 544 different Q3 float
components over 1,152 rows (maximum absolute difference 0.0000112), with exact
flags/timers. First differing binary32 position: scenario 2, frame 98. Decimal
formatting differences are ignored; comparison uses float bits.

Two engine defects are corrected in the shared movement entry. Original
bg_pmove.c PM_UpdateViewAngles (:1811-1822) narrows every combined command/delta
angle to signed short before SHORT2ANGLE, and clamps pitch to +/-16000 units
while adjusting delta_angles. The old Q3 path retained 180/270 degree yaw as
positive angles and clamped pitch to +/-89 degrees. The existing short-angle
conversion now serves Q2 and Q3, with their own pitch policy and no second
movement implementation. PM_WaterMove (:512-514) multiplies scale into each
basis before command components and their sum; factoring scale outside that
sum changed binary32 rounding. The shared wish calculation retains native
water ordering. Other rule sets keep their previous arithmetic.

The remaining native-C difference is fully isolated to PM_CmdScale (:308):
`(float)pm->ps->speed * max / (127.0 * total)`. GCC 16.2.1, x86_64-pc-linux-gnu,
`-O2 -ffp-contract=off -fno-strict-aliasing` emits double MULSD/DIVSD followed by
CVTSD2SS for the unsuffixed denominator. The diagnostic changes only `127.0`
to `127.0f`; its 1,152 rows and all 6,912 float components match corrected Rust
exactly, including flags/timers. The unchanged native oracle remains visible:
324 differing components, maximum 0.0000112. No tolerance was increased.

Original Q3 lcc, compiled as a 32-bit host, emits CNSTF4 127.0, MULF4 and DIVF4
for the unchanged original PM_CmdScale body. Its bytecode target declares
float/double/long double as four bytes (lcc/src/bytecode.c:328-330); interpreter
OP_MULF/OP_DIVF operate on float (vm_interpreted.c:854-860). THE-711 requires
Q3's binary32/QVM arithmetic, so the engine retains that path rather than
introducing a native-C numeric profile. This proves the scale-expression
attribution and diagnostic C rows, not execution of a complete QVM or the
installed 30-second strafe-jump gate. No x87 arithmetic or fused multiply-add
instructions were found in the native reference disassembly. The earlier
64-bit lcc helper crashed in its declaration parser; its failed diagnostic is
retained, and no bytecode result from that process is used.

Focused angle tests cover signed yaw/roll wrap, native pitch limits, the
wire-unsigned command angle used for delta adjustment, and repeated-command
stability. Q2's 1,152 original-C rows remain exact. The unchanged checker,
workspace tests and Clippy are recorded with the final evidence.

CPU23, portable release, three ABBA blocks, 60 warm-up plus 600 measured frames
per leg: the existing 64 mixed-rule clients/server-and-prediction trace fixture
measured 2,988,837.83 -> 2,978,536.17 ns median (-0.345%) and 4,249,764.67 ->
4,123,766.33 ns p99. Both sides execute 38,400 SERVER steps; server/prediction
state matches throughout each run, with zero measured allocations/bytes. This
workload is held constant between these two builds; its generic
matched_previous_workload flag refers to older unrelated fixture revisions.
Corrected native view/order results are intentional, not claimed byte-identical
to the buggy engine. No worker, foreign heap, renderer or gameplay time is measured.

Reproduce with tools/check_movement.py --q3-attribution; optionally pass the
original 32-bit lcc rcc through --q3-rcc to emit its scale bytecode. The native
C oracle is still compiled without source changes; the single-literal variant
is explicitly diagnostic. Evidence is in `THE-3175-q3-arithmetic-20261009/`:
final/result.json, native disassembly and QVM scale assembly, baseline and
intermediate row files, movement-abba.json, twelve final raw timing legs and
verification/build logs. Remaining native/QVM gameplay acceptance stays on
THE-891/THE-3169.

### THE-860: Q3 entity/player fields on the existing delta walker

The common scalar walker now handles last-changed field counts, native float
shortcuts, entity zero shortcuts and optional array masks. Q3 supplies its 51
entity fields, 48 player scalar fields and four 16-word player arrays as table
data. Fixed temporary wire projections follow that native field order; they do
not introduce another engine entity or PlayerState store. Native entity numbers
are supplied explicitly, and unrepresentable numbers are omitted at the boundary.
Removal and forced unchanged records retain their original control bits.

The unchanged original MSG_WriteDeltaEntity, MSG_ReadDeltaEntity,
MSG_WriteDeltaPlayerstate and MSG_ReadDeltaPlayerstate functions match 4,096
seeded/boundary cases and 2,127,492 comparison bytes, including decoded words.
The helper uses the original field tables, struct layouts and Huffman functions;
offsetof only replaces the original null-pointer offset macro. The production
table names/order/widths are independently checked against qsrc. Existing
QW/Q2/Q3 usercmd comparisons remain exact over all 3,072 cases and all 256 native
QW/Q2 masks. Four focused tests cover control bits, signed arrays, float limits,
IEEE payload preservation, invalid counts and every truncated entity prefix.
Nonfinite payload tests are Rust boundary tests, not a claim about C's undefined
float-to-integer conversion. The unchanged checker, 656 workspace tests and
warning-denied Clippy pass.

CPU23, portable release, five ABBA blocks, 60 warm-up plus 600 measured frames
per leg; sixteen encode/decode records with original-C fidelity checks per frame.
The pre-slice command binary is retained with the evidence. Table metadata
eliminates extended state-value branches from command code, and group decisions
are resolved before field iteration. Selective reader inlining recovers the
initial command slowdown; the earlier rejected measurements remain available.
The same source implements every table walk. These are codec timings, excluding
host, workers, transport syscalls, snapshots, rendering and gameplay.

| Usercmd workload | Before median ns | After median ns | Change | Before p99 ns | After p99 ns |
| --- | ---: | ---: | ---: | ---: | ---: |
| Mixed QW/Q2/Q3 | 3164.5 | 3146 | -0.585% | 6225 | 6244 |
| QW | 2027.5 | 1772 | -12.602% | 2765 | 2404 |
| Q2 | 2229 | 2099.5 | -5.810% | 2871 | 2873 |
| Q3 | 5178.5 | 5117.5 | -1.178% | 6260 | 6275 |

All medians improve; the mixed/Q2/Q3 p99 changes are small increases of
0.305%, 0.070% and 0.240%, respectively. Fixture checks and wire-byte totals
match in every leg. Allocations, reallocations and requested bytes are zero,
with a one-allocation positive control. The new entity/player workload has no
matched prior engine path: four runs average 25,778.75 ns median and 49,262.5 ns
p99 per sixteen records, each with 10,560 checks and 584,495 wire bytes including
warm-up; measured heap activity is zero.

Evidence is in `THE-860-state-delta-20261009/`: final-original/comparison.json,
final-commands/comparison.json, final-command-abba-summary.json and its
group-policy-write-hint raw legs, final-state-timing-summary.json and four raw
state runs, plus checker/workspace/Clippy logs. Reproduce the native records
with tools/check_state_delta.py and the platform state_delta example. This
checkpoint supplies record codecs; common-state ABI projections, snapshot
framing/rings, other protocol state tables, signon, captures, live connections
and installed multiplayer acceptance remain on THE-860/THE-3169.

The preceding normal candidate at 9e01e940 also passed the owner's six private
300-frame smoke cases: Q1 start, Q2 base1 and Q3 q3dm1 on CPU and GL, with real
key-repeat movement and normal exit. Rust heap activity was zero in all measured
frames across the calling thread and seven CPU workers. GL used llvmpipe LLVM
22.1.8, Mesa 26.2.2 (software GL); these runs do not measure hardware GL speed
or foreign heaps. Saved settings and the candidate stayed unchanged, and all
owned helper PIDs were cleaned up. Evidence is retained under
`THE-3175-q3-arithmetic-20261009/private-smoke-final/`. This is smoke evidence
for that earlier candidate, not a new installation or full gameplay acceptance.

### THE-860: native Q2 player state on the same field tables

Q2 protocol 34 supplies 36 scalar columns and 32 signed-short stats to the
existing delta walker. Its table presets the always-sent weapon-index flag and
marks six gun offsets/angles as dependent on gunframe. Those dependent fields
are sent when the frame flag is present, but do not raise it themselves.
Scaled values retain signed quarter-unit offsets, short-angle conversion,
byte FOV and blend-byte division by 255. Integer pmove words are already
narrowed at the native connection boundary. Temporary wire words do not add
another engine player-state array. The old fixed-size array-field constructor
is replaced by one generic mask-field constructor serving Q3 and Q2.

The unchanged Q2 SV_WritePlayerstateToClient and CL_ParsePlayerstate functions
join the existing original Q3 oracle: all 6,144 records, 3,158,137 comparison
bytes and decoded words match. Native struct/byte bindings stay in the
developer-only C helper. The 3,072 QW/Q2/Q3 usercmd cases also remain exact.
The focused dependency test covers an unchanged frame with changed offsets,
the always-sent/truncated weapon index, signed offsets, angles, blend and stat
31. The unchanged checker, 657 workspace tests and Clippy pass.

CPU23, portable release, five ABBA blocks, 60 warm-up and 600 measured frames,
sixteen records per frame with original-C bytes/words checked. Baselines are
the saved f03c3d68 codec examples and their unchanged fixtures. The initial
general trigger bitset raised the Q3 command median by 3.30%; the final compact
dependent range removes that regression. No earlier slower design is retained.

| Existing codec workload | Before median ns | After median ns | Change | Before p99 ns | After p99 ns |
| --- | ---: | ---: | ---: | ---: | ---: |
| Mixed usercmds | 3289 | 3238.5 | -1.535% | 6683 | 6694 |
| QW usercmds | 1872.5 | 1824 | -2.590% | 2658 | 2747 |
| Q2 usercmds | 2157.55 | 2082 | -3.502% | 3056 | 2893 |
| Q3 usercmds | 5298.5 | 5203.5 | -1.793% | 6928 | 6817 |
| Q3 entity/player records | 26509.55 | 25867 | -2.424% | 54275 | 52702 |

All five medians improve. Mixed/QW p99 increase by 0.165% and 3.348%; other
tails improve. Every leg retains its fixture checks and byte totals, with zero
allocations, reallocations and requested bytes and a positive control. The new
Q2 player workload has no prior engine path: four runs average 11,815 ns median
and 15,722.5 ns p99 per sixteen encode/decode records; each performs 10,560
fidelity checks and 447,317 wire bytes including warm-up, with zero measured
heap activity. Timings exclude host, workers, sockets, rendering and gameplay.

Evidence: `THE-860-q2-player-20261009/`, final-original/comparison.json,
final-commands/comparison.json, final-abba-summary.json and range-dependency
raw legs, final-q2-state-summary.json and its four runs, plus verification
logs. The common-state projection, snapshot/channel integration, Q2 entity,
QW/NQ/rerelease state tables and native/installed acceptance remain open on
THE-860/THE-3169. This checkpoint is a record-codec proof, not complete network
or gameplay acceptance.


### THE-860: QW entity records and the shared message decoder
QW protocol 28 supplies one entity field table to the existing walker. The
prefix carries the native entity number and mask; table metadata removes
prefix-only comparisons and projections from inline-mask/player/command paths.
Origin differences retain float subtraction followed by comparison to the
original double 0.1 literals. Coordinates and angles retain their native
signed decoding. MOREBITS is selected before SOLID, and the decoded header
retains MSG_ReadShort sign extension. Removal and forced unchanged records
preserve the native controls; out-of-range entity numbers are omitted.
Every field constructor now uses the one common constructor; the old command
constructor is deleted. One generic entity-delta result replaces the Q3-only
result structure, with aliases for each temporary wire shape.

The shared message reader replaces its bit-at-a-time Huffman tree with one
2,048-entry prefix table derived from the same original fixed codes. The tree
and old walk are deleted. Reserved-symbol errors and truncated-code offsets/
bit cursors remain checked. Byte-stream writers omit whole-buffer clearing
because every emitted byte is overwritten; bit/Huffman streams still clear
for their zero padding. Reused dirty buffers match clean streams.

Unchanged QW SV_WriteDelta and CL_ParseDelta, Q2 player writer/parser and Q3
entity/player MSG functions match all 8,192 records, 4,113,114 comparison bytes
and decoded words. The QW removal uses the original SV_EmitPacketEntities
statement; this does not compare the full packet emitter. Native struct and
byte bindings stay in the developer-only helper. The earlier 3,072 command
cases and all 256 QW/Q2 masks remain exact. The original MSG/Huff oracle also
matches 768 cases and 49,152 fields. Focused tests cover epsilon boundaries,
SOLID ordering, sign extension, dirty-buffer padding, reserved symbols and
truncation cursors. The unchanged checker, 659 workspace tests and Clippy pass.

CPU23, portable release, five ABBA blocks, 60 warm-up and 600 measured frames
per leg, sixteen records with original-C byte/word checks per frame. The
baseline binaries and fixtures are retained from 0d8a7600. Earlier expanded
comparison branches and general lazy buffer initialization were slower and
are discarded; their measurements remain in the evidence. The final metadata
and fixed-prefix decoder meet the median gate for all six existing workloads.

| Existing codec workload | Before median ns | After median ns | Change | Before p99 ns | After p99 ns |
| --- | ---: | ---: | ---: | ---: | ---: |
| Mixed usercmds | 3155.5 | 3112 | -1.379% | 6270 | 4449 |
| QW usercmds | 1797.5 | 1772 | -1.419% | 2793 | 2825 |
| Q2 usercmds | 2060 | 1996 | -3.107% | 2911 | 3290 |
| Q3 usercmds | 5137.5 | 3563 | -30.647% | 6652 | 4771 |
| Q3 entity/player records | 23882 | 18078 | -24.303% | 47896 | 34526 |
| Q2 player records | 11877.5 | 11760.5 | -0.985% | 15923 | 16062 |

All medians improve. QW/Q2-command/Q2-state p99 increases are 1.146%,
13.020% and 0.873%; the other tails improve. Every leg retains its workload,
byte total and 10,560 fidelity checks including warm-up. Measured allocations,
reallocations and requested bytes are zero, with a one-allocation positive
control. Four new QW entity runs average 2,617.5 ns median and 3,202.5 ns p99
per sixteen records, 55,725 wire bytes and 10,560 checks including warm-up,
with zero measured heap activity. There is no prior QW entity engine path for
a matched before/after measurement. These probes exclude host, workers,
sockets, rendering and gameplay.

Evidence: `THE-860-qw-entity-20261009/`, final-original/comparison.json,
final-commands/comparison.json, final-message/comparison.json,
final-abba-summary.json and lookup-final raw legs, final-qw-summary.json and
four QW runs, checker/workspace/Clippy logs. This remains a record-codec
checkpoint: common-state projections, snapshot framing/rings, Q2 entity and
NQ/rerelease state tables, native signon, captures, live connections and
installed acceptance remain on THE-860/THE-3169. No snapshot/gameplay caller
is claimed for these tables yet.


### THE-860: Q2 entity records on the common walker

Protocol 34 registers twenty native entity words in the same scalar table.
The native frame field has two ordered descriptors because its parser reads
byte and short flags independently. Skin/effects/render flags choose byte,
short or long widths from signed/unsigned threshold data. Byte decoding stays
unsigned and short decoding signed, including the original skin-short result.
Transient events compare to zero and clear when absent. Old origin defaults
to the previous origin and is forced by new-entity/beam policy. Entity number
is explicit; U_NUMBER16 precedes the unchanged-record check. Native flag
extension bytes, removals and field order remain unchanged.

Static group metadata excludes variable-width/transient branches from existing
tables. No second encoder, reader or engine entity store is introduced. The
unused fixed-width accessor is deleted; the generic entity result is reused.
Q2 player decoding is inlined at its fixed-array boundary after the first
measurement showed a 9.609% median increase in that older workload.

The original MSG_WriteDeltaEntity, CL_ParseEntityBits and CL_ParseDelta bodies
join the existing Q3/Q2-player/QW oracle. All 10,240 records, 5,090,044 comparison
bytes and decoded words match. Native removal statements are extracted from
SV_EmitPacketEntities; the whole packet emitter is not compared. An additional
original-parser assertion and Rust test agree on both frame flags and their
seven-byte cursor. Focused tests also cover signed width boundaries, event
reset, old-origin defaults/beam forcing, unchanged entity 256 and every
truncated active-record prefix. Earlier 3,072 command records and 768 MSG/Huff
cases remain exact. The unchanged checker, 660 workspace tests and Clippy pass.

CPU23, portable release, five ABBA blocks, 60 warm-up plus 600 measured frames,
sixteen records with original-C fidelity checks per frame. Baselines and
unchanged fixtures are retained from f38fe5a2. Measured codec allocations,
reallocations and requested bytes are zero; the positive control counts one.

| Existing codec workload | Before median ns | After median ns | Change | Before p99 ns | After p99 ns |
| --- | ---: | ---: | ---: | ---: | ---: |
| Mixed usercmds | 3083 | 3002 | -2.627% | 4304 | 4472 |
| QW usercmds | 1859.5 | 1826 | -1.802% | 3009 | 2889 |
| Q2 usercmds | 2026.5 | 1852 | -8.611% | 2976 | 2834 |
| Q3 usercmds | 3521 | 3522.5 | 0.043% | 4700 | 4293 |
| Q3 entity/player records | 20375.5 | 20274.5 | -0.496% | 37682 | 40781.1 |
| Q2 player records | 12720.5 | 13111.55 | 3.074% | 18289 | 19052 |
| QW entity records | 2774.5 | 2544 | -8.308% | 3742 | 3373 |

Q3-command/Q2-player medians increase by 0.043%/3.074%; other medians improve.
Mixed-command/Q3-state/Q2-player p99 increases are 3.903%/8.225%/4.172%; other
tails improve. This meets the existing 10% median guard, with the increases
reported explicitly. Every leg retains its byte total and 10,560 fidelity
checks including warm-up. Four new Q2 entity runs average 4,943.75 ns median
and 6,647.5 ns p99 per sixteen records, 169,004 wire bytes and 10,560 checks
including warm-up, with zero measured heap. There is no comparable prior Q2
entity engine path. These timings exclude host, transport, workers and gameplay.

Evidence: `THE-860-q2-entity-20261009/`, final-original/comparison.json,
final-commands/comparison.json, final-message/comparison.json,
final-abba-summary.json and inline-q2-read raw legs, final-q2-entity-summary.json
and four runs, final-checker.json, workspace summary/log and Clippy log.
Native implicit unchanged-entity application still belongs to the packet-frame
path: this record probe normalizes an omitted record to its supplied baseline.
Common-state projection, snapshot framing/32-slot rings, NQ/rerelease state
coverage, native signon/captures/live/combined clients and installed acceptance
remain on THE-860/THE-3169. No snapshot/gameplay caller or install is claimed.


### THE-860: NetQuake signon-baseline entity records

Protocol 15 uses the same scalar walker. Its first five target fields retain
QuakeC floats until comparison against integer signon-baseline values and wire
truncation. Fractional fields can therefore change a flag while sending the
same byte. Native MSG_WriteAngle truncates to integer degrees before integer
scaling, unlike QW/Q2. Shared angle metadata selects that order; signed angle
and coordinate decoding and the native double-literal origin epsilon remain
shared. Static float-byte group metadata removes these conversions from older
tables. No second encoder or entity store is added.

Every visible entity emits an update, including unchanged values. Native signal,
MOREBITS, long number and no-lerp flags remain exact. The final projection word
reports the wire no-lerp flag, not scene interpolation state. Inputs use explicit
native numbers; the client permits entity zero, while the server omits it.
Protocol-15 short numbers are signed; out-of-range server numbers are omitted.
Absence from an unreliable datagram is neither a removal record nor an ACK.

Unchanged SV_WriteEntitiesToClient and CL_ParseUpdate functions are compiled
with private edict/model/visibility bindings. All 12,288 mixed records,
6,060,382 comparison bytes and decoded words match; 2,048 NQ records cover all
15 update bits and native number-width boundaries. The expanded fixture entity
binding covers up to 32,767; it does not qualify stock MAX_EDICTS 600, native
signon, visibility selection, interpolation or presentation side effects.
Earlier 3,072 command and 768 MSG/Huff cases remain exact. Eight focused tests,
661 workspace tests, the unchanged checker, formatting and Clippy pass. The
comparison helper's initial long-number footer error was corrected by capturing
the original CL_EntityNum argument; no original function body was altered.

CPU23 portable release, five ABBA blocks, 60 warm-up plus 600 measured frames,
sixteen records/frame with original-C byte/word checks. Baselines and fixtures
are retained from 3141177c. No other owned benchmark/build runs alongside the
final serial matrix. Earlier overlapping and prototype runs are retained but
do not qualify this matrix. Final example rebuild: Cargo 5.61 s; allocation
tracking enabled, no proof feature or installation candidate.

| Existing codec workload | Before median ns | After median ns | Change | Before p99 ns | After p99 ns |
| --- | ---: | ---: | ---: | ---: | ---: |
| Mixed usercmds | 3000.5 | 3088 | +2.916% | 4356 | 4297 |
| QW usercmds | 1753.5 | 1736.5 | -0.969% | 2636 | 2770 |
| Q2 usercmds | 1861 | 1960.05 | +5.322% | 2746 | 3087 |
| Q3 usercmds | 3548 | 3540.5 | -0.211% | 4959 | 5191 |
| Q3 entity/player records | 18562 | 18873 | +1.675% | 35844 | 36504 |
| Q2 player records | 13274.5 | 13301.5 | +0.203% | 20285 | 18851 |
| QW entity records | 2525 | 2564.5 | +1.564% | 3667 | 3599 |
| Q2 entity records | 5061.5 | 5306.5 | +4.840% | 7199 | 7070 |

Positive medians range from 0.203% to 5.322%; QW/Q3 commands improve. QW/Q2/Q3
command and Q3-state p99 increase; the other tails improve. All meet the existing
10% median guard; this addition is not reported as a general speedup. Each leg
retains its wire-byte total and 10,560 checks including warm-up, with zero
measured allocations/reallocations/requested bytes and a one-allocation positive
control.

Four new NQ entity runs average 3732.5 ns median / 5627.5 ns p99
per sixteen records, 134,427 wire bytes and 10,560 checks including warm-up,
with zero measured heap. No comparable prior NQ entity engine path exists. These
probes exclude host, workers, sockets and gameplay.

Evidence: `THE-860-nq-entity-20261009/`, final-original/comparison.json,
final-commands/comparison.json, final-message/comparison.json,
nq-flag-coverage.json, final-abba-summary.json and final-serial raw legs,
final-nq-entity-summary.json/four runs, checker/workspace/Clippy/build logs.
Common-state projection, snapshot framing/32-slot rings, NQ client data,
negotiated 666/999/rerelease coverage, captures/live/native/installed acceptance
remain on THE-860/THE-3169. These state tables have no snapshot/gameplay caller
yet; no install is claimed.
