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
only before SERVER and before CLIENT, stamping each input event and appending
Time after each poll. The frame-cap wait reads platform time without collecting input.
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
intents and bot intents use the same stateless UserCmdBuilder. Both supply
unscaled positive/negative key fractions, ordered device axes and an explicit
movement policy. The builder owns scaling and native accumulation: NQ and Q2
rerelease retain floats, QW/Q2 narrow each assignment to signed shorts, and Q3
uses integer contributions and signed-byte bounds. Human-only movement assembly
and the bot path accepting already-scaled native units are removed. Duration
and absolute server time are supplied by the caller. THE-889 seeds human command
time at the first Time event and builds all 64 connected bot slots in SERVER
world ticks, in client-id order. Client frames do not override bot commands.
The bot module supplies each client's primitive intent; navigation/AI and
physics remain later work. Movement speed, mouse policy and seat settings
enter through THE-735's cached handles. The current shell's 127 units and
0.022 mouse scale are routing fixtures.
The held-key reference is Q3 `cl_input.c` IN_KeyDown, IN_KeyUp and CL_KeyState.
Native wire projections remain in the shared network command module. NQ/QW/Q2
short fields, Q3 byte fields and Q2 rerelease float movement remain distinct
boundary data. Rerelease jump/crouch/holster use its native button bits; its
server_frame is supplied at the boundary. These projections do not establish
packet framing or interoperability with original servers. The crouch alias and
holster action use the existing bind storage. Centerview resolves the calling
seat's current policy: Q2/Q3 subtract its authoritative delta pitch, while
NQ/QW start the one pitch-drift state. CLIENT advances that state after command
construction; NQ uses supplied ideal pitch, QW targets zero. Ground state,
manual pitch input and native float narrowing are preserved. The player's
ideal pitch is copied through prediction; native guest population remains open.
The full private button/combined-seat walks and qualified installed acceptance
remain open.

THE-2852 supplies one core ValueBank for arbitrary player/HUD numeric fields.
It stores raw 32-bit payloads behind internal ValueIds. Cold ValueBindings select
native signed-short, signed-int or negotiated float projection; these handles
never replace native ordinals on the wire. The stock schema binds NQ/QW32,
Q232, Q2RR64 and Q3 stats/persistent16 each into one bank. Extra load-time fields
use the same storage and interned names. Each module may supply an independent
binding range, regardless of the selected presentation or protocol.

The app allocates matching player and HUD bank sizes at load; its existing HUD
update copies the bank into the snapshot and counts clipped values without
growing storage. Layout and text state are preserved. A module life reset touches
only its bound life fields; persistent/session values survive. Session reset
clears all values while retaining the allocation. Native module imports,
extension negotiation, live layouts and legacy packet integration remain open.
THE-697/890 now supplies independent display leases; native module/channel
acceptance remains open.

RuleSetId is per player, independent of map, module and client protocol.
At the movement boundary, Q1 duration clamps to 1..100 ms, QW/Q2 replace values
above 250 ms with 100 ms, and Q3 clamps to 1..200 ms. Q2 rerelease is bounded by
its native byte msec field; its closed KEX client's timing policy is not claimed.
Absolute server time is retained. These are duration limits, not the full QW
step bisection, Q3 outer Pmove catch-up/subdivision or movement implementation;
THE-891 supplies the common physics entry and its selected rule set.

The duration references are Q1 `WinQuake/host.c:Host_FilterTime`, QW/Q2
`client/cl_input.c:CL_FinishMove`, Q3 `game/bg_pmove.c:PmoveSingle` and the Q2
rerelease `rerelease/game.h:usercmd_t` byte field. The private diagnostic check
uses the shell's Q3 movement policy after a one-second startup hold:

```sh
python3 tools/check_sys_events.py --binary "$QA_CANDIDATE" --owner-profile "$QA_PROFILE" --evidence "$QA_EVIDENCE/command-time" --check-command-time --console-source q3
```

`--udp-listen 127.0.0.1:0` opens a host socket and reports its actual address.
`--controller-seat INSTANCE:SEAT` and `--mouse-seat INSTANCE:SEAT` supply
explicit device ownership, with seats 0..3. Mouse instance 0 is SDL's aggregate
pointer for non-relative events. `--seat-policy SEAT:CLIENT:MOVEMENT:TRACE`
connects that seat and all preceding seats; unset seats inherit the first
client's explicit preset. Each role can select q1/qw/q2/q2rr/q3 independently.
No native entity namespace is inferred before a module is loaded. The existing
client array, prediction states, queue, builder, collision store and scene API
serve every seat. Distinct authored spawn anchors are selected at load; extra
anchors are labeled fixtures until native module spawn/telefrag selection runs.
The first seat retains the previous native initial anchor. A map without enough
distinct anchors rejects the startup request before play. Only seat zero's
saved profile is imported; additional saved-seat settings remain pending.
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
including its aliases. The cap wait uses platform time only. Physical intake occurs before SERVER
and before CLIENT, following the owner's two-intake ruling; waits do not poll
SDL, stdin or sockets.
THE-2869 replaces THE-885's shared overwrite rings with one byte FIFO per
client and endpoint, sized from explicit load-time connection limits. The one
safe typed-header/payload FIFO implementation also backs `SysEventQueue`.
Admission returns counted `Full` without overwriting any packet; oversize and
unknown-client errors leave other clients untouched. Loss simulation belongs
to netchan. No local storage pointer escapes the event queue.

After physical system events run out, local packets enter `SysEventQueue` in
client-endpoint then server-endpoint order, retaining send order across clients.
The host consumes only that queue through the same packet receiver as UDP.
Rejected queue admission leaves the local message pending. Additional batches
consume queued memory only; the frame still has exactly two physical intakes.
Core owns the single socket/local `Peer` type; destination sockets are reserved
internal ids. They add no native wire fields or ACKs.

The current app reserves 64,000-byte local messages and 128,000 payload bytes
per client/direction at load. This is a boot capability ceiling while native
connection negotiation remains absent; each future connection must supply its
actual negotiated limits, including native framing overhead. Headless 8,000-
and 64,000-byte payloads prove transport capacity, not an NQ module signon.
Provider-generated packets enter the second drain before CLIENT. Netchan,
codec, reliability/ACK and live signon integration remain THE-860/THE-2869;
the current receiver counts and identifies packets only.

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

Normal split-seat private qualification (THE-859/656):

```sh
python3 tools/build.py --allocation-tracking
python3 tools/qualify_local_seats.py --binary target/candidate/qa-rust \
  --owner-profile "$PROFILE" --content "$RETAIL_ROOT" --evidence "$EVIDENCE"
```

The harness copies the candidate and owner settings for each of six GL/CPU
runs. Owned XTest keyboard and pointer events drive one seat each, separately
and together; key holds use actual X-server repeat. e1m1 combines Q1 and Q3
movement, base1 Q2 and Q3, q3dm1 Q3 and Q1. Trace rules remain explicit and
independent. It checks two rendered views, per-seat authoritative movement,
queue drains, real repeats, release and all instrumented Rust-thread allocation
counts. The run is a renderer/walk integration with no gameplay modules, not
the installed Done-when. It preserves the qualified installer's gameplay gate.
