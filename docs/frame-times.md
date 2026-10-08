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
