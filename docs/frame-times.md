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
| e1m1 GL | 2,330 / 4,540 | 2,640 / 4,240 | 41,975 / 52,780 | 5,507,429 / 6,900,165 | 5,776,044 / 5,943,065 |
| base1 CPU | 100,050 / 108,990 | 87,110 / 94,250 | 81,255 / 89,230 | 5,076,149 / 5,113,704 | 1,822,141.5 / 1,899,731 |
| base1 GL | 100,210 / 110,600 | 86,740 / 95,130 | 96,600 / 109,000 | 8,500,266 / 12,057,568 | 4,462,483 / 4,844,884 |
| q3dm1 CPU | 31,705 / 40,900 | 22,120 / 27,900 | 175,480 / 205,631 | 44,368,553 / 45,978,173 | 1,855,287 / 2,311,122 |
| q3dm1 GL | 31,730 / 36,070 | 22,040 / 28,230 | 187,610 / 201,140 | 18,311,098.5 / 21,877,306 | 5,414,184 / 8,241,826 |

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
