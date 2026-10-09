# Core adoption and deferred native acceptance

The owner 2026-10-09 11:0x ruling separates engine acceptance from native-module
and installed acceptance. THE-3169 is the one deferred item under THE-863. It
retains the original live requirements; no render smoke or headless fixture
counts as gameplay. The engine audit below covers current Rust callers, including
examples and tests, rather than future gameplay modules.

Verified native-reader main was `230ac80c`. Unfinished ELF work is pushed on
`wip/THE-2575-2026-10-09` at `2db27ef9`; that lane is stopped. The accepted
installed `qfiles/qa-rust` is `a1d32b8c`, built 2026-10-09T15:26:36Z in 38.94 s.
Its private three-map CPU/GL smoke passed; native hosts, delta channel, stock HUD
and gameplay remain absent. The earlier statement that no install occurred is
superseded by that owner's explicit smoke-install authorization.

## Primitive ownership and adoption

| Issue | One implementation and current callers | Removed copies / remaining sites |
| --- | --- | --- |
| THE-611 | `world/src/entities.rs:269` EntityTable owns generations and hot columns; `session/src/clients.rs:79` creates it; `compat/src/services.rs:266` allocates and `:275` releases through it; area, targets and trace borrow it | Removed unused core Entity aggregate and Think aggregate; the remaining test uses actual SoA columns. No alternate current entity store found. Native spawn/free adapters remain THE-3169. |
| THE-617 | `core/src/names.rs:49` NameTable owns exact bytes and cached folded groups; `world/src/targets.rs:68` updates the index; `console/src/cvars.rs:129` and commands use that implementation; `render/src/assets.rs:317` interns paths and image/material keys retain NameIds | Earlier NameIndex/hash and duplicated renderer canonicalisation are deleted. No alternate name-index or renderer cache-key implementation found. Shader parsing/VFS native path grammar is load/boundary work, not a second name identity. Native target callbacks remain THE-3169. |
| THE-625 | `world/src/collision/scene.rs:47` WorldTrace clips world plus linked bodies; `session/src/clients.rs:251`, `app/src/host.rs:452`, and `compat/src/services.rs:210` call it. CollisionStore alone owns loaded hull/brush models. | No current production caller skips linked collision by calling a kernel directly. Kernel examples intentionally compare their specified geometry-only scope. Native cgame/QuakeC collision adapters remain THE-3169. |
| THE-650 | `world/src/area.rs:224` link distinguishes Explicit from Commit; `session/src/clients.rs:267` commits movement through it; `compat/src/services.rs:247` makes explicit links; shared attachment transport serves SERVER and prediction | No second current link/query/attachment implementation found. Link order and model bounds are entity-role data; module touch callbacks and native pickup scenes remain THE-3169. |
| THE-656 | `session/src/clients.rs:36` Server owns the one load-sized client array, each row containing core PlayerState; app Runtime chooses capacity. Prediction copies hot fields into the same type, not an alternate native player store. | The implicit-64 app loader is deleted. No per-game player-state copy found. `app/src/host.rs:447` still directly submits built local commands pending THE-860 native framing; this deferred bypass is tracked on THE-3169. |
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
| THE-859 | `core/src/sys_events.rs:119` owns SysEventQueue; platform/EventPump alone collects SDL/stdin/UDP and time; `app/src/host.rs:344` dispatches input, console lines and packets. SysEventQueue and Loopback share `core/src/payloads.rs:18` storage. | No current physical-input/clock/socket bypass found. `app/src/host.rs:524` is still a no-op character editing target; console front-end editing is not implemented. Native/installed multiple-seat/device proof remains THE-3169. |
| THE-884 | `app/src/host.rs:190` owns Com_Frame; physical polls are only at `:213` and `:285`, each followed by drain and command execution. `session/src/timing.rs:97` schedules world/provider clocks independently. Cap waits collect no input. | `app/src/main.rs:401` still loads no native providers. Native phase-order logs remain THE-3169. |
| THE-885/THE-2869 | `core/src/loopback.rs:43` owns per-client/per-direction bounded byte FIFOs. `:116` transfers in endpoint/send order into SysEventQueue; `app/src/host.rs:401` consumes queued memory through ordinary Packet dispatch. Full never overwrites. | No old overwrite transport or host direct receive loop remains. `app/src/host.rs:447` is a direct local command submission pending original per-client protocol framing; native signon/channel adoption is retained on THE-3169/THE-860. |
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
* `app/src/host.rs:447` directly submits local usercmds until the original
  per-client protocol/channel exists; no invented local wire format was added.
* `network/src/ingress.rs:14` PacketReceiver currently counts ingress,
  without native handshake, channel or snapshot decoding.
* `app/src/output.rs:123` requests remote submission through FrameSource; the
  actual protocol ACK/submission adapter remains THE-860/THE-3169.
* `ui/src/hud.rs:46` implements projection, with stock layout drawing deferred.

Engine review and native acceptance are separate. Linear remains the source of
truth; the supervisor closes reviewed engine scopes. Agents never set Done.
