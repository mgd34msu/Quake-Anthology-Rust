# Core adoption and deferred native acceptance

The owner 2026-10-09 11:0x ruling separates engine acceptance from native-module
and installed acceptance. THE-3169 is the one deferred item under THE-863. It
retains the original live requirements; no render smoke or headless fixture
counts as gameplay. The engine audit below covers current Rust callers, including
examples and tests, rather than future gameplay modules.

Verified native-reader main was `230ac80c`. Unfinished ELF work is pushed on
`wip/THE-2575-2026-10-09` at `2db27ef9`; that lane is stopped. The accepted
installed `qfiles/qa-rust` is `a7ec14a2`, built 2026-10-09T22:19:17Z in 29.39 s.
Its owner-authorized private start/base1/q3dm1 CPU/GL smoke passed at 300 frames
per run with normal exits and movement through native local QW28 packets.
Native hosts, snapshot delta tables, sign-on, stock HUD and gameplay remain
unfinished. The development install does not qualify those deferred features.

## Primitive ownership and adoption

| Issue | One implementation and current callers | Removed copies / remaining sites |
| --- | --- | --- |
| THE-611 | `world/src/entities.rs:269` EntityTable owns generations and hot columns; `session/src/clients.rs:79` creates it; `compat/src/services.rs:266` allocates and `:275` releases through it; area, targets and trace borrow it | Removed unused core Entity aggregate and Think aggregate; the remaining test uses actual SoA columns. No alternate current entity store found. Native spawn/free adapters remain THE-3169. |
| THE-617 | `core/src/names.rs:49` NameTable owns exact bytes and cached folded groups; `world/src/targets.rs:68` updates the index; `console/src/cvars.rs:129` and commands use that implementation; `render/src/assets.rs:317` interns paths and image/material keys retain NameIds | Earlier NameIndex/hash and duplicated renderer canonicalisation are deleted. No alternate name-index or renderer cache-key implementation found. Shader parsing/VFS native path grammar is load/boundary work, not a second name identity. Native target callbacks remain THE-3169. |
| THE-625 | `world/src/collision/scene.rs:47` WorldTrace clips world plus linked bodies; `session/src/clients.rs:251`, `app/src/host.rs:452`, and `compat/src/services.rs:210` call it. CollisionStore alone owns loaded hull/brush models. | No current production caller skips linked collision by calling a kernel directly. Kernel examples intentionally compare their specified geometry-only scope. Native cgame/QuakeC collision adapters remain THE-3169. |
| THE-650 | `world/src/area.rs:224` link distinguishes Explicit from Commit; `session/src/clients.rs:267` commits movement through it; `compat/src/services.rs:247` makes explicit links; shared attachment transport serves SERVER and prediction | No second current link/query/attachment implementation found. Link order and model bounds are entity-role data; module touch callbacks and native pickup scenes remain THE-3169. |
| THE-656 | `session/src/clients.rs:36` Server owns the one load-sized client array, each row containing core PlayerState; app Runtime chooses capacity. Prediction copies hot fields into the same type, not an alternate native player store. | The implicit-64 app loader is deleted. No per-game player-state copy found. THE-860 now frames local commands in `app/src/lib.rs:207`, sends them through Channel and Loopback, and submits only the decoded Packet event in `app/src/host.rs:391`. Native module/provider phases remain THE-3169. |
| THE-702 | `ui/src/hud.rs:46` projects PlayerState into core HudState; `app/src/host.rs:513` updates every connected row; `app/src/output.rs:103` feeds print events into the same HUD and leases | No per-game HUD-state copy found. Stock Q1 sbar, Q2 layouts and Q3 cgame consumers do not exist yet; these are explicitly deferred consumers under THE-3169, not proved drawing. |
| THE-709 | `session/src/dispatch.rs:276` runs a spawn-bound numeric entry with native timing data; `:335` scans EntityTable's live bitset. Original-C probes and all current think callers use it. | Old Think aggregate is deleted; no alternate dispatcher found. Native physics-phase callers are absent: `app/src/main.rs:401` supplies no module providers. Integrating those callers remains THE-3169. |
| THE-2884 | `core/src/primitives.rs:345` RuleSetId is the sole five-value rule identity; client_policy selects client, movement and trace independently; movement, console, damage and scheduling consume it | MovementRules, console Source and ThinkTiming identities are deleted. Renderer texture Source enums describe resources, not game identities. Native role composition remains THE-3169. |
| THE-2875 | `world/src/entities.rs:277` free_bits is the sole liveness record; active/think walks use it; named changes alone update TargetIndex. Core forbids unsafe code. | FrameArena, its test/use and the separate live bool array are deleted. Unnamed churn produces zero target refreshes in the measured fixture. Retail think/touch and rocket/nail scenes remain THE-3169. |

File references in this table are relative to `crates/`. Line numbers describe
the audited tree; the core aggregate deletion shifts later primitives.rs lines.
No new checker rule or primitive-use tool was introduced. The existing checker
is unchanged.

## Event-system ownership and adoption

| Issues | One implementation and migrated callers | Remaining bypass / deferred consumer |
| --- | --- | --- |
| THE-859 | `core/src/sys_events.rs:119` owns SysEventQueue; platform/EventPump alone collects SDL/stdin/UDP and time; `app/src/host.rs:344` dispatches input, console lines and packets. SysEventQueue and Loopback share `core/src/payloads.rs:18` storage. | No current physical-input/clock/socket bypass found. `app/src/host.rs:524` is still a no-op character editing target; console front-end editing remains THE-927/THE-1135. Native/installed multiple-seat/device proof remains THE-3169. |
| THE-884 | `app/src/host.rs:190` owns Com_Frame; physical polls are only at `:213` and `:285`, each followed by drain and command execution. `session/src/timing.rs:97` schedules world/provider clocks independently. Cap waits collect no input. | `app/src/main.rs:401` still loads no native providers. Native phase-order logs remain THE-3169. |
| THE-885/THE-2869 | `core/src/loopback.rs:43` owns per-client/per-direction bounded byte FIFOs. `:116` transfers in endpoint/send order into SysEventQueue; `app/src/host.rs:401` consumes queued memory through ordinary Packet dispatch. Full never overwrites. | No old overwrite transport or host direct receive loop remains. Local command submission now follows native move framing, Channel, Loopback and SysEventQueue; the direct CLIENT-to-SERVER caller is deleted. Native signon remains THE-3169/THE-860. |
| THE-887 | `console/src/command_buffer.rs:13` is the single fixed command buffer; `console/src/commands.rs:222` borrows tokenizer argv; `:254` bare cvar write takes argv1. Platform ConsoleLine, binds and EngineServices append into that buffer. | No second current command buffer/tokenizer path found. Installed alias/wait/vstr/macro runs remain THE-3169. |
| THE-888 | `input/src/lib.rs:528` dispatches queue events into per-seat bindings; `console/src/commands.rs:295` bind parsing and button commands use those same tables. Held-source reuse leaves original press time intact. | No alternate current bind/hold implementation found. Real-button installed gameplay remains THE-3169. |
| THE-889 | `input/src/command.rs:9` owns UserCmdBuilder; `input/src/lib.rs:915` handles human intents, `session/src/clients.rs:210` builds bots in SERVER ticks. Rule data selects native scaling/narrowing; `network/src/commands.rs` owns protocol/ABI projection values. | No second current human/bot builder found. Guest ideal-pitch updates and original native channel delivery remain THE-3169. |
| THE-697/THE-890 | `core/src/events.rs:83` owns the one ring, payload leases and per-client/module cursors; Server allocates it; app/output uses it once per CLIENT/quit frame. Module output consumes its own cursor at native ticks. `:383` accepts actual native ACK receipts, never a send watermark. | No destructive-drain ring, shared publication wait or frame-reset text ownership remains. `app/src/output.rs:123` still awaits the real native submission/ACK adapter; stock audio/particle/HUD consumers remain deferred. |

The shared `session/src/events.rs` consumer helper also uses EventRing's
batch/submit API; it has no queue, text store or independent retirement rule.
Current examples and tests exercise either those primitives or app/FrameHost.
Raw character delivery is tested in the input fixture, but an editable console
UI is not claimed. Native wire framing was not replaced by a private invented
protocol to hide the pending direct-submit site.

## Evidence and limits

Fresh release probes and original-C comparisons are in the developer cache
`core-priority-20261009/`, with commands, exit codes and build times. Pinned
CPU23 probes use 60 warm-up and 600 measured frames; each checks its stated
fixture outputs and Rust calling-thread allocation gate. See
[frame-times.md](frame-times.md) for measured scopes and results. These checks
exclude native modules, foreign heaps, game audio and installed gameplay.

The deferred native-path sites are concrete:

* `app/src/main.rs:401` constructs the host with no native providers.
* THE-860 removes the direct local usercmd submission.
  `app/src/lib.rs:207` sends original NQ15/QW28/Q2 34/Q3 68 move packets;
  `app/src/host.rs:391` submits only the decoded Packet event. The development
  host explicitly defaults to QW28 independently of map/client/movement rules.
  `--local-protocol` and `--seat-protocol` select native framing per connection.
  This is a connected development host, not a native handshake or signon.
* THE-860 replaces the counting-only PacketReceiver with load-sized endpoint
  bindings in `network/src/ingress.rs`. `app/src/host.rs:369` dispatches queued
  packets through the bound native Channel and `:381` retires actual channel
  receipts against the binding's original output-consumer generation.
  Move payload decoding and automatic local channel binding are now adopted.
  Full native command strings/history and negotiated Q3 keys, NQ666/999,
  rerelease transport, handshake, Q3 command reliability and sound/effect native transmission
  remain unfinished. The native acceptance is retained on THE-860/THE-3169.
* `app/src/output.rs` now encodes NQ/QW/Q2 remote print records, queues the
  shared Channel's native receipt and transmits through `FrameSource::send_packet`.
  The output/resync callback adapters are deleted. ACK receipts enter only
  through Packet dispatch. Q3 command-window output and native sound/effect
  mappings remain THE-860; local HUD consumers still use the in-process ring.
  Bounded stalled-client disconnection is not native reconnect/sign-on proof.
* `ui/src/hud.rs:46` implements projection, with stock layout drawing deferred.

Engine review and native acceptance are separate. Linear remains the source of
truth; the supervisor closes reviewed engine scopes. Agents never set Done.

## THE-3174: twelve supervisor-audited call sites

The 2026-10-09 adoption audit resolves the following twelve sites together.
References are relative to `crates/`. Arithmetic keeps its previous operation
order; length-only replacements retain division rather than reciprocal
normalization. No checker rule is added or changed.

| # | Audited site | Adopted implementation and preserved behavior |
| --- | --- | --- |
| 1 | `render/src/stage.rs:17` | Entity-view subtraction uses Vec3; non-normalized axis compensation uses core length and the existing zero-length branch. |
| 2 | `render/src/sky.rs:154` | Sky rotation uses core cross with the same component product/subtraction order. |
| 3 | `render/src/sky.rs:114` | Sphere projection uses core length; finite/positive rejection and scale divided by length remain in place. |
| 4 | `render/src/sky.rs:300` | Cloud intersection uses core length; intersection divided by norm and acos order remain in place. |
| 5 | `render/src/cpu/sky.rs:211` | CPU screen rays use Vec3 multiply/subtract/add and core normalized. Integer endpoint fixtures compare the previous arithmetic and original WinQuake C. The zero-ray branch preserves the previous NaN-to-integer result. |
| 6 | `render/src/world.rs:144` | Frustum normals use Vec3 multiply/add/subtract/negate in the previous order; no normalization is introduced. |
| 7 | `movement/src/physics.rs:519` | Movement-axis magnitude uses core length with the same left-associated dot products. |
| 8 | `world/src/collision/boxes.rs:49` and `store.rs:355` | Box-local translation and hit translation use Vec3 subtraction/addition; transformed model normals use vector negation for the right basis and inverse angles. Native centering and hull-offset order are retained. |
| 9 | `world/src/collision/mod.rs:23` | EntityTraceRules is deleted. Caller-selected trace_policy(RuleSetId) produces behavior fields for world-entity assignment, link role, query kind, rejection gates, point contents and hit merging. Every existing caller, comparison probe, example and test uses that policy. The packed policy is four bytes; TraceQuery keeps its previous float offsets and 120-byte size through an explicit internal layout. Protocol projections still read named fields. |
| 10 | `console/src/cvars.rs:602` | Each value stores its folded NameId at load; conversion side effects bind that numeric identity rather than resolving its text again. Exact display names remain distinct. Numeric columns retain their previous offsets and the row stays 120 bytes. |
| 11 | `console/src/cvars.rs:116` | Dirty-value and dirty-projection membership use core StampSet. Refresh order and dependent deduplication are unchanged; the old boolean marks and per-entry clears are deleted. An empty dirty list does no refresh work. |
| 12 | `console/src/commands.rs:136` | Registered button actions use a folded-NameId-indexed table. The same numeric dispatch table holds function callbacks and button actions; no button action string search remains. Command listing retains its ordered exact-name vector. |

Remaining references to the removed identity in `tools/check_rules.py` are
unchanged checker text, not executable identity definitions or callers.
The twelve production sites have no remaining local arithmetic, identity,
name-rebinding, dirty-boolean or button-action lookup copy from the audit.
Native module and installed acceptance still remain on THE-3169.

Developer evidence is retained under `THE-3174-adoption-20261009/`. The
original-C checks cover 30,000 retail e1m1 hull traces, 40,632 transformed-model
rows, 20,140 brush rows, 1,188 linked-merge rows and 16,384 sky endpoint rows.
A second 16,384-row sky test compares the replaced Rust arithmetic directly.
Q2/Q3 movement output is byte-identical to pre-adoption main across 1,152 rows
per game. Q2 matches original C exactly; Q3 retains its previously measured
544 differing float components with maximum error 0.0000112, while flags and
timers remain exact. This slice does not claim to remove that existing gap.
See [frame-times.md](frame-times.md) for the matched CPU23 timing and allocation
scope; synthetic comparison results do not establish native gameplay parity.
