# Shared movement

THE-891 adds a Pmove-style function selected by each player's RuleSetId.
The call takes UserCmd, mutable PlayerState and shared trace/point-contents
services. CollisionWorld selects the geometry algorithm independently. Module
state, inventory, client protocol and movement rules remain separate choices.

Server command consumption and current-command client prediction call this
function. Bots use the same SERVER usercmd builder and movement consumer in
client-id order. Entity columns receive the authoritative body. Prediction
copies only hot movement fields, without cloning inventory or module arenas.
The app runs these phases when its collision world is loaded. The current
window shell has no map and does not execute player movement.

The shared implementation contains acceleration, friction, fluid sampling,
ground classification, bounded plane clipping, 18-unit steps, stance and wire
quantization. Policy differences include NetQuake's precise duration, original
30-unit air cap, QW recursive duration halving (including discarded odd
milliseconds), Q2 eighth-unit signed coordinates and eight-ms timer decrement,
rerelease floating state, and Q3 absolute command time/66-ms steps, midpoint
air gravity and asymmetric overclip. Physics timers and surface flags live in
common primitives; they do not belong to a module-format union. Brush contacts
carry the NetQuake SOLID_BSP distinction. Cached tuning accepts cvar/module
values without per-command cvar lookup; live movement cvar wiring remains R2.

The entry is a walking foundation, not completed native gameplay. NetQuake's
QuakeC PlayerPreThink owns its jump impulse. Full module pre/post-think and
impact/touch order, Quake/QW unsticking and water jumps, Q2 ladders/currents,
rerelease movement extensions, Q3 events/animation and less common movement
modes require the native movement issues THE-635/766/606/609. The rerelease
reference here is the C port's derived movement provider, not an open KEX
client implementation or proof of retail fidelity. Native crouch dimensions
retain the floor; actual crouch paths still need retail-map proof.

There is one latest command slot per connected client, consumed once in the
SERVER phase; bots submit once per world tick. Current-command prediction
starts from authoritative movement state and advances one pending command.
This is not latency correction. Network command acknowledgement and correction
remain THE-821. There is no input journal, history player or replay mechanism.

## Verification

```sh
python3 tools/check_movement.py --qsrc "$QSRC" --output "$EVIDENCE/native"
cargo build --release -p qa-platform --example movement --features allocation-tracking
taskset -c "$CORE" target/release/examples/movement > "$EVIDENCE/movement.log"
```

The original Q2 and Q3 Pmove sources compile only into developer helpers. Each
comparison uses 1,152 command states across flat walking, stairs, walls/angled
strafe, jump/landing, crouch/pitch and swimming. The common analytic trace world
is implemented at the callback boundary in both helpers. It exercises native
movement functions without claiming native BSP collision equivalence. Q2
coordinates/velocities, ground/jump flags and timers must match exactly. Q3
flags/timers must match exactly and float components must differ by at most
0.00003 units; it is not a bit-match claim. Q3 SnapVector uses nearest-even
rounding in the reference, corresponding to the native engine callback.

The session test runs all five rules on Q2 brush geometry, with local, remote
and bot clients carrying a different module tail, and compares server and
prediction state. It also verifies that a consumed command is not run twice.
This is a primitive composition check. THE-839 must still load/render/walk
retail e1m1, base1 and q3dm1 with native and foreign movement in private runs.

The timing example measures 64 mixed-rule clients, including bot command
construction, authoritative movement and the same commands through prediction,
on analytic stairs/walls. It uses 60 warm-up and 600 measured frames, checks
identical states and fails on any measured Rust allocation. The SDL/native
allocator and other threads are outside that counter. This workload has no
previous movement implementation as a comparable baseline and does not qualify
renderer performance or installation.

Source references: qsrc quake/WinQuake sv_user.c and sv_phys.c; quake/QW/client
pmove.c and sv_user.c command splitting; quake-2/qcommon/pmove.c; Quake III
bg_pmove.c and bg_slidemove.c. Rerelease behavior comes from the C port's
src/movement/q2/rerelease.c. No movement code was mined from muse-final.
