# System events

THE-859 replaces immediate SDL callbacks and host clock reads with one ordered
system event queue. Q3 `qcommon.h`'s sysEvent_t and `common.c`'s Com_EventLoop
supply the input, packet and time contract. All games and mixed sessions use
this queue; it is distinct from the output sound/effect/print ring.

Core owns EventTime in nanoseconds, DeviceId, SeatId and SysEventQueue. The
queue has a fixed event ring and fixed byte arena allocated at load. Console
lines and datagrams copy into that arena; dispatch borrows their bytes until
the next mutable queue operation. FIFO release reuses storage, including wrap
padding. Failed admission changes no existing event or payload. Ordinary
events reserve the last slot for the frame's time marker.

Platform owns the monotonic clock, SDL and nonblocking UDP sockets. It polls
before server work, afterwards, and during the cap wait, stamping each input
event and appending Time after each poll.
SDL polling retains input in SDL when the ring cannot fit a complete text
event plus Time. UDP polling reads at most 64 datagrams per socket per poll;
full event/payload storage drops datagrams and increments a counter. There is
no receive thread. Device hotplug handles remain in platform; seat ownership
belongs to input. Developer timing uses platform's Stopwatch too.

The host drains each pass in FIFO order. Input goes through one bind table, chars
go to the console/menu target, console lines enter the single command buffer,
and packets go to the network boundary. The Time event supplies frame time.
The current R2 shell's character target awaits R2 focus/editing, and its
network receiver only counts delivered datagrams; R11 supplies channels and
protocol decoding. Neither is gameplay proof.

Input maps devices to four seats. Two held keys can drive one action; repeated
down for the same device/control does not change its press time. Partial-frame
hold time supplies movement fractions. Focus loss and controller removal
release acquired bindings. A UI-consumed press does not acquire a binding.
Release a live control before rebinding or reassigning its device. Human
intents and bot intents use the same UserCmdBuilder. Movement speed, mouse
policy and seat settings enter through THE-735's cached handles; frame pacing
remains part of the R2 host-loop work.
the current shell's 127 units and 0.022 mouse scale are routing fixtures.
The held-key reference is Q3 `cl_input.c` IN_KeyDown, IN_KeyUp and CL_KeyState.
Native wire projections remain in the shared network command module.

`--udp-listen 127.0.0.1:0` opens a host socket and reports its actual address.
`--controller-seat INSTANCE:SEAT` supplies device ownership, with seats 0..3.
SDL2 identifies controllers individually but combines physical keyboards.
Real multi-controller walkthroughs and owner profile application remain for
integration; headless two-device commands do not prove those runs.

The rule checker rejects OS clock types, epoch/POSIX clock sources, SDL
symbols/crate imports and foreign link attributes, stdin, thread spawn/sleep
and direct socket types outside platform, including the imported aliases and
examples exercised by its fixtures. Shared sound/effect/print/packet queue
definitions belong to core. CI runs planted rejection and permitted-boundary
fixtures. These source checks cover the listed patterns, not arbitrary Rust
name resolution. There is no queue journal. The development SDL input player
is a replay release blocker under THE-893; do not add recording or replay
pending the owner's decision.

THE-884 implements the Com_Frame order in [AGENTS.md](../AGENTS.md):
drain/commands, provider-rate server ticks, a second drain/commands, then the
client frame. The shell client builds commands and presents its window;
snapshot application, movement/prediction and scenes still await integration.
The cached com_maxfps handle uses the original integer-millisecond client cap,
including its aliases. Linux poll waits on readable sockets for at most 2 ms,
then polls SDL too; other platforms currently use the same bounded interval
with a timer wait. This removes the fixed 16 ms sleep and drains while waiting.
THE-885 adds fixed local loopback buffers feeding the same packet path.

`qa_session::timing::Timeline` schedules the world and loaded provider frame
functions on one event timeline. Rates are data selected at load, independent
of map format and console grammar: Q1 can be frame-driven, Q2 uses 100 ms,
Q2 rerelease 25 ms, Q3 1000/sv_fps ms. Startup/world-load time seeds the clocks;
residual time is retained. Due ticks merge by timestamp, then world/provider
id. Provider callbacks retain observable entity/think ordering. The app's
current empty server has a frame-driven world and no game providers. Native
rate counter probes exercise scheduling, not loaded game-module behaviour.

```sh
timeout 300 python3 tools/build.py --check-only
timeout 300 python3 tools/verify_rules.py --evidence "$EVIDENCE/rule-fixtures"
timeout 300 cargo test --workspace --all-features
timeout 300 cargo build --release -p qa-platform --example system_events \
  --features allocation-tracking
timeout 300 taskset -c "$CORE" target/release/examples/system_events
```

The headless example warms 60 frames, then measures 600 frames of the actual
queue/seat/network boundary with one loopback UDP packet each frame. A positive
allocation control must detect one allocation and one reallocation before
accepting zero. Its timing excludes the sender, SDL, console editing and
rendering. Map walks, two real seats and game-frame allocations remain open
at R3.5; see [frame times](frame-times.md) for bounded measurements.

THE-861 will connect shared scene building to both renderer back ends using
two fixed command lists, initially on one thread. THE-860 will add one delta
codec driven by protocol field tables and fixed snapshot/reliable rings.
Neither later capability polls input, sockets or the OS clock.
