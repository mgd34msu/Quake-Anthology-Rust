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

Supervisor review at 18:49 accepted `54708d5b`; it is pushed on main and its
superseded adoption WIP branch is deleted. THE-3175 tracks the pre-existing
Q3 float discrepancy for attribution after the current THE-860 step.

## THE-862: materials, surface cache and view preparation

All current world loaders and scene consumers use the shared material path.
The remaining load-time bypass was Q1/Q2 faces ignoring matching authored
`textures/<name>` scripts. `render/src/material/world_load.rs:449` now resolves
those names through the existing catalog and compiler. Unmatched faces keep
their generated native materials; each face retains its own lightmap binding,
region and projection scale. The unconditional generated-material branch is
deleted. Registration remains load-time work.

References below are relative to `crates/`.

| Capability | Implementation and current callers | Remaining bypass sites |
| --- | --- | --- |
| Material registration | `app/src/map.rs:315` calls `render/src/material/world_load.rs:167` for every map family. `render/src/material.rs:24` discovers scripts through the VFS. Authored, default and legacy-generated stages all register through `render/src/assets.rs:470`; worlds bind numeric material/image handles at `:562`. | None found among current loaders. Native module/media registration is not yet a caller. |
| Stage semantics | `render/src/stage.rs:414` owns StageEvaluator. CPU world, model, polygon and 2D draws and GL stage draws consume the same material table and prepared stage state. Generated native flags are material data, not a second executor. | No current per-game material table or stage executor found. |
| Visibility and surface records | `render/src/world.rs:52` owns the common World and per-view WorldView query. CPU and GL consume its geometry, bindings and shared world visibility traversal. Each view owns its mutable visibility scratch. | No backend-owned replacement PVS traversal found. |
| Surface cache | `render/src/surface_cache.rs:859` and `:912` expose indexed and RGBA fills; both use `:991` prepare_slot and `:1073` rover allocation. Bands borrow one immutable SurfaceCatalog with private resident arenas and pins. | No separate per-family rover or invalidation implementation found. |
| Load-sized budget | `render/src/cpu/world.rs:655` builds the shared catalog; `:677` selects the aligned map-mip sum, a 32 MiB floor and each band's mandatory surface minimum. A nonzero diagnostic override remains explicit. | No current fixed-default 32 MiB bypass remains. |
| Per-surface lookup | `render/src/cpu/world/span_groups.rs:20` uses core StampSet to group each bounded scanner flush by surface/mip. `render/src/cpu/world.rs:2212` consumes those groups; indexed/product/factor paths borrow texels until the same rover batch ends. | The former per-span cache setup copies are deleted. |
| Static and animated materials | `render/src/cpu/rgba.rs:343` admits static product/factor recipes, including bilinear lightmaps. Animated textures, changing tcMods and wave color/alpha use the shared stage executor rather than invalidating a whole static surface every frame. Cache stamps represent changed inputs. | No shader-clock field is added to static cache identity. Live module lighting still needs supplied changed inputs. |
| Preparation and raster jobs | `render/src/cpu/world/jobs.rs:56` is the one CPU job entry; `:122` prepares bounded private chunks and merges in order. Small or capacity-limited work uses the same serial preparer. `app/src/renderer.rs:400` dispatches preparation and screen bands through the existing platform pool. | No renderer-owned worker pool or alternate preparation kernel found. |

The audit found zero remaining current material/cache/preparation bypass sites.
This count excludes missing consumers: native hosts, module-driven media,
stock HUD drawing and guest/live lighting integration remain THE-3169 and their
feature issues. It does not claim that those paths already exist or are migrated.

The authored-material fixture covers both Q1 and Q2, a matching face and an
unmatched face, native fallback without a script, folded catalog lookup,
animated stage registration and preserved face lightmaps. The existing cache,
native fill, stage, chunk/band, clipped-pixel and invalidation fixtures remain
unchanged. The checker, Clippy and 664 workspace tests pass.

Fresh normal-app evidence is retained under `THE-862-engine-20261009/`:
fifteen private CPU/GL runs, 60 warm-up and 600 measured frames each, normal
exit and zero measured Rust heap activity across the caller and CPU workers.
Stock CPU time-zero RGBA matches before/after for e1m1, base1 and q3dm1.
This is render integration, not installed gameplay or original-engine image
parity. The live Q3 CPU median is 5.689 ms, 7.744% above this series' baseline;
the prior frozen checkpoint is 5.837 ms. R12's under-4-ms target remains open.
See [frame-times.md](frame-times.md) for raw scopes, cache bytes and host load.

Engine scope is submitted for supervisor review. THE-3169 retains the installed
authored-face animation/screenshots and combined shader-pack run required by
THE-862, along with native-module and stock-HUD acceptance. No installation is
claimed by this slice.

## THE-860: connection snapshot storage and native Q3 stream

`network/src/snapshots.rs:59` owns one typed 32-slot Ring implementation.
Protocol field counts are const parameters, not separate game stores. Slots
retain native projection words, ordered native entity numbers, area bytes and
sequence/clock metadata; they do not own another gameplay PlayerState or
EntityTable. Entity storage, baselines and decode scratch are allocated at
connect. Baselines seal before the first frame. Missing/stale deltas are
consumed without publication; bounded capacity overflow is counted and never
publishes a partial frame. Q3 retention and delta-distance limits are policy
data at that boundary.

`network/src/snapshots.rs:213` and `:336` implement Q3 snapshot bodies using
the existing entity/player field walker. Sorted merges preserve insertion,
removal and implicit unchanged rows. New entities use the connection's baseline;
full frames use the native zero player baseline. Reserved/out-of-width entity
numbers and excess area bits are omitted at the Q3 boundary. Native frame
requests are distinct from output ACK receipts.

`network/src/channel/commands.rs:281` and `:306` extend the existing server MSG
writer/reader in place. Commands, snapshots, Huffman, XOR, EOF and the existing
Channel framing share that stream. All callers of the old command-only methods
are migrated and those methods are deleted. A full 64-command window followed
by a snapshot now reaches EOF; the former command-only opcode limit is removed.
`network/src/ingress.rs:245` exposes a borrowed accepted Snapshot through the
existing Packet event consumer. There is no new intake point, packet queue or
receive thread. Cold snapshot storage is boxed only for the relevant client
endpoint, rather than inflating every protocol's Channel with inline frames.

Remaining adoption sites are explicit: `app/src/host.rs:369` still ignores
CLIENT snapshot callbacks, `:480` copies SERVER state directly into prediction,
and `app/src/lib.rs:201` seeds prediction from SERVER state. Common-state
projection and SERVER snapshot emission are the next THE-860 step. Other
protocols' snapshot framing, negotiated NQ666/999/rerelease fields, native
gamestate/signon, captures/live/combined/installed proof remain THE-860/THE-3169.
This slice does not claim those callers are migrated or legacy play is ready.

The original Q3 snapshot writer/parser bodies match all 512 seeded cases:
571,602 wire bytes, 5,864 decoded entity rows, player/area fields, native frame
validity and read-bit positions. Accepted frames number 488; 24 missing-base
frames are consumed and discarded. Existing 16,384 native state comparisons
and 512 reliable-command comparisons still match. Nine focused snapshot tests,
673 workspace tests, unchanged checker and Clippy pass. See
[frame-times.md](frame-times.md) and `THE-860-snapshots-20261009/` for CPU23
release timing, allocation gates and the exact measured scopes. No install.

## THE-3176: second supervisor adoption audit

All eight audited groups now call the shared implementation. Their former
production helpers/loops were deleted in this slice; no checker extension was
added. Operation order and native signed narrowing remain unchanged.

| Audited group | Shared implementation and migrated callers |
| --- | --- |
| Camera clipping | `render/src/cpu.rs:47,66` stores `Vec3` and uses `Vec3::lerp`; texture/lightmap/color interpolation is unchanged. Camera subtraction at `cpu.rs:159` also uses `Vec3`. |
| Short angles | `core/src/math.rs:78-93` owns `angle_to_short` and `short_to_angle`. `network/src/commands.rs:158,221` and `network/src/delta.rs:130,173` use them, as does `movement/src/physics.rs:727`. Width/sign conversions remain at native boundaries. `anglemod` also uses this helper. |
| Mip lengths | `render/src/cpu/world.rs:2806` uses `math::length`, preserving component sum order and the following average/thresholds. |
| Bounds | `core/src/primitives.rs:289-306` owns `Bounds::empty/add_point/add_bounds`. All MDL/MD2/MD3/MD5/sprite readers use it; `formats/src/model/read.rs` no longer defines bounds helpers. `render/src/world/geometry.rs:712` retains its first-point initialization and finite-input rejection, then calls `add_point`. |
| Entity syntax | `app/src/map.rs:312,379` passes the load-stored `native_source` to the existing `entity_syntax`; the second spawn-syntax family match is deleted. |
| Image defaults | `render/src/material/resources.rs:56` takes `RuleSetId`. `app/src/render_settings.rs:6`, resource tests and the name-consumer example pass it directly. `material/world_load.rs:175` receives the BSP natural `RuleSetId` directly at the fallback boundary. Explicit presentation settings still win. |
| Local points | `core/src/math.rs:97` owns `transform_point`: origin, then axis 0, 1, 2 in that order. CPU entity rendering at `cpu.rs:595`, CPU sky collection at `cpu/world.rs:1046`, and GL sky collection at `gl.rs:984` all use it. |
| Path bytes | `core/src/names.rs:15` exposes the existing `path_byte`; `content/src/vfs.rs:113` now uses it. VFS component rejection, parent traversal and fixed output capacity are unchanged. |

The short-angle policy preserves three arithmetic forms in one core helper:
networking's float multiply then divide, movement's float precomputed factor,
and native `anglemod`'s double precomputed factor. `AngleShortForm` selects
arithmetic, not a game identity. Replacing these with one arithmetic order
would change rounding before integer narrowing. The shared decoder retains
signed integer input, including Q3 pitch-delta corrections outside i16 range.
Eight-bit angle protocol encodings retain their different widths, including
NetQuake's integral-value rejection; they are not 16-bit short conversions.

Read every line of the 2,222-line pre-slice `render/src/gl.rs`. Two further
host-side Vec3 additions at current `gl.rs:1170,1272` now use the core operator.
No other local host implementation of a core helper was found in that full
read. GL matrices, viewport checks, uniform packing and texture state have no
core equivalent. The embedded GLSL shader executes fast normalization and
noise on the GPU; it cannot call Rust's core helper. Its shader instructions
are unchanged, with prepared stage/wave data still supplied by the one stage
executor. This is not a claim that native modules or all engine consumers are
wired. EventRing Sound-to-mixer and compat VM/CallTable session dispatch remain
explicitly tracked on THE-3169 after THE-860.

Evidence directory: `$HOME/.cache/qa-rust/THE-3176-adoption-20261009/`.
The normal release app and developer comparison binaries built in 38.93 s.
The initial command omitted the developer examples' required platform allocation
feature and failed; the corrected command succeeded. Neither command enabled
proof input. All 676 workspace tests, the unchanged checker, format and diff
checks pass. New numeric checks cover 65,536 seeded float patterns across each
angle form, all 65,536 signed shorts, 16,384 transforms/clipping interpolations,
and signed-zero bounds unions. Existing CPU pixel/reference, geometry, model,
path and console tests pass.

`comparisons.json` records exact equality to both previous Rust and original C
outputs for 16,384 native state cases (8,039,745 comparison bytes) and 512 Q3
snapshot cases (2,035,577 comparison bytes). Q2 and Q3 movement each retain all
1,152 prior Rust rows exactly. Q2 still matches C completely. Q3 still has the
324 native-C float-component differences attributed on THE-3175, maximum
0.0000112, with exact flags/timers. These are analytic fixtures, not gameplay.
No timing run, game/window run, allocation measurement or installation was
performed for this adoption slice, per the owner's cadence ruling.

## THE-860: native snapshot delta request

`network/src/commands/connection.rs:58` now selects Q3's native `clc_move`
only when the last acknowledged server-message sequence names an accepted
snapshot in the existing channel ring. A command-only message or a discarded
snapshot selects `clc_moveNoDelta`. This follows original
`quake-iii-arena/code/client/cl_input.c:746-752`; no input history or new
protocol field is added.

The same connection decoder stores the native message ACK as `delta_request`
on `clc_move` and clears it on `clc_moveNoDelta`, before duplicate/stale usercmd
filtering, matching `server/sv_client.c:1339-1343`. Snapshot writers consume
this request, rather than a transmit watermark. The old unconditional no-delta
selection and ignored move flag are removed. The existing native opcode writer,
ACK/XOR handling, 32-slot ring and field-table delta encoder remain the only
implementations.

Two connected-channel tests cover initial full requests, full and valid delta
frames, a command-only server message, missing delta bases and stale usercmds.
All 678 workspace tests and the unchanged checker pass; the normal release
workspace build completed in 34.63 s. Evidence is in
`$HOME/.cache/qa-rust/THE-860-delta-request-20261009/`. No timing, game/window,
allocation measurement or installation was run.

THE-860 remains In Progress. The app still copies SERVER player state directly
into prediction at `app/src/host.rs:480` and initial local connect at
`app/src/lib.rs:201`; its CLIENT packet callback still ignores Snapshot
records. Common player projection, snapshot submission and migration of these
callers are the next integration. Other protocols' snapshot framing,
signon/native hosts and original-client/server interoperability remain required.

## THE-860: common player projection

`network/src/projection.rs` adds one field walker over the existing common
PlayerState. Connection load selects static NQ15, QW28, Q2-34 or Q3-68 layout
data. It writes the existing native codec words into caller-owned storage;
there is no second engine player store or scalar delta encoder. Movement and
trace RuleSetIds remain independent and unchanged.

Native client, ground, weapon and weapon-model numbers, effective speed and
gravity, QW visibility flags, command age and body yaw are explicit context.
No common ClientId, EntityId or registry handle is cast to a native ordinal.
QW body yaw is separate from command/view yaw, matching its corpse command
path. Q2 movement uses signed eighth units, eight-millisecond timers and signed
short delta angles. Q3 delta angles use unsigned 16-bit fields. Both use the
original ANGLE2SHORT multiply-then-divide order through core math. The static
flag data preserves simultaneous native timer flags; mode mapping drops modes
the destination cannot carry to its native equivalent. Q2 stock health, armor
and frags and Q3 health, armor and score use the common hot fields.

Load-resolved ValueBindings provide native module fields from the existing
common value bank, in registration order. A later binding overrides a default
field. Out-of-layout destinations are omitted and counted at load; a missing
common bank field retains the hot value or zero. A short output is rejected
without partial mutation. Inventory ordinals, icons, animation and other
module-specific fields still require their explicit bindings; zero defaults
are not evidence of native gameplay integration.

Four focused tests pass: native codec round trips for all four implemented
layouts, independent roles, native ordinals/mode limits, module overrides,
missing bindings, short output rejection and 4,096 seeded delta angles checked
against the original macro order and signedness. NetQuake health/armor are
QuakeC float inputs to its writer but native integer outputs from its decoder;
the assertions check that distinction. Original field/flag sources are
`quake/WinQuake/sv_main.c`, `quake/QW/server/sv_ents.c`,
`quake-2/game/q_shared.h`, and `quake-iii-arena/code/game/q_shared.h` and
`bg_public.h`. Existing codec implementations are unchanged.

THE-860 remains In Progress. The projection has no app snapshot provider yet;
the remaining direct prediction copies at `app/src/host.rs:480` and
`app/src/lib.rs:201`, and ignored CLIENT Snapshot dispatch, still need
migration. Q2 rerelease, negotiated NQ666/999, other snapshot streams, signon,
native/combined/installed acceptance remain required. Evidence is in
`$HOME/.cache/qa-rust/THE-860-player-projection-20261009/`; no timing,
allocation measurement, game/window or installation run is claimed.
All 682 workspace tests, the unchanged checker, formatting and diff checks
pass. The final normal release workspace build completed in 30.72 s.

## THE-860: shared packet-entity merge and Q2 frames

`network/src/snapshots.rs:370,561` now owns one ordered read merge and one
ordered write merge for native packet entities. Q3's separate loops were
deleted; its existing snapshot/channel callers use these loops. Q2 frame
codecs use the same loops and the same generic 32-slot Ring. Native prefixes,
terminators, player layouts and unchanged-row policy select the data and codec
entries; there is no second engine entity/player store or scalar encoder.

The existing Q2 record reader at `states.rs:712` now delegates to its one
prefix/body decoder. `states.rs:735` owns Q2's unchanged-row reset: old_origin
receives the prior origin and event clears, including omitted prefix and
trailing rows. Both packet and record callers use it. Q2 frame headers retain
native frame/lastframe, suppression count, area bytes, playerinfo and
packetentities. Frame time is the protocol's native frame * 100 milliseconds,
independent of movement or map. The native client count is supplied explicitly
for the player old-origin rule. Delta selection uses the actual requested
frame and native 29-frame cutoff; unrepresentable entity numbers are omitted.

The one ring publication path bounds capacity, counts missing bases/overflow
and retains typed slot storage. Original Q2 CL_ParseFrame retains parsed
invalid frames, then tests the addressed sequence/row age independently of
old.valid on a later delta. The shared slot has that validity metadata;
invalid frames remain unavailable through frame(). Q3 retains its existing
discard behavior. Overflow never publishes a partial frame.

The developer snapshot probe and original-C tool were extended rather than
duplicated. Whole original Q2 SV_EmitPacketEntities/SV_WriteFrameToClient and
CL_DeltaEntity/CL_ParsePacketEntities/CL_ParseFrame bodies run with private
native state and stubbed presentation callbacks. All 512 Q2 cases match
913,057 comparison bytes, decoded player/entities, flags, validity and cursors.
All existing 512 Q3 cases still match 2,035,577 bytes. The existing 16,384
native state cases match all 8,039,745 bytes and decoded words. These fixtures
do not prove signon, native hosts or gameplay.

The release snapshot probe records zero caller Rust allocations, reallocations
and requested bytes across each protocol's 512 encode/decode cases, with a
one-allocation positive control. This is a headless codec gate; it does not
measure workers, driver/SDL or host frames. No timing runs were performed.
Three new tests cover full/delta/removal, unchanged prefix/trailing resets,
missing-base retention, overflow recovery and area limits. All 685 workspace
tests, unchanged checker, warning-denied network/platform Clippy, formatting
and diff checks pass. The normal release app and comparison binaries built in
32.45 s. Evidence: `$HOME/.cache/qa-rust/THE-860-q2-frames-20261009/`.

THE-860 remains In Progress. Q2's frame codec is not connected to ingress/app
yet. Direct prediction copies remain `app/src/host.rs:480` and
`app/src/lib.rs:201`; CLIENT Snapshot dispatch still ignores decoded Q3
frames. QW/NQ frame streams, negotiated protocols/rerelease, native signon,
common-state provider/client adoption and combined/installed proofs remain.
No game/window run or installation is claimed.


## THE-3176: BSP rule identity follow-up

`formats/src/bsp.rs:25` replaces `BspFormat::family() -> u8` with
`rule_set() -> core RuleSetId`: Quake, HalfLife, Bsp2, Psb2 and Quake64 select
Quake; Quake2 and Qbsp select Quake2; Quake3Test, Quake3 and QuakeLive select
Quake3. This is file provenance, independently of movement, trace, module,
client and presentation roles. No numeric family identity remains in the API.

Every former caller is migrated in the same slice:

| Caller | Adoption |
| --- | --- |
| `formats/src/bsp.rs`, `bsp/decode.rs`, `bsp/validate.rs` | Directory clamping, record decoding, extension discovery and visibility validation match the natural RuleSetId. Native layout widths, finite/reference checks and compressed visibility rules are unchanged. |
| `render/src/material/world_load.rs:174,514,882` | ImageSettings::native receives the id directly; lightmap preparation and legacy materials take RuleSetId. Palette, colormap, texture flags, RGB lightmap arithmetic and native sky/liquid rules keep their original selection. |
| `render/src/world.rs:183`, `world/geometry.rs` | Visibility and geometry conversion use RuleSetId matches, preserving source leaf/cluster membership, flags, extents and light encodings. |
| `app/src/map.rs:237,374,503` | Map source initialization, spawn data and collision loading receive the natural id without a second numeric conversion. Stored entity syntax and caller movement/trace rules remain independent. |
| `app/tests/retail_collision.rs`, `platform/examples/visibility.rs` | Existing load probes use RuleSetId. The example stores the id and checks the three required sources without a second numeric identity; its pre-existing JSON `family` numbers are formatted only at the report boundary to preserve its output schema. |

Repository search finds no `family()` call or method in crates. The existing
synthetic layout/material/geometry tests and the retail load checks validate
source-specific results; this slice adds no rendering, gameplay or timing
qualification claim. The checker is unchanged. Evidence is in
`$HOME/.cache/qa-rust/THE-3176-bsp-rules-20261009/`.

All 685 workspace tests and six existing retail collision/inline-model load
checks pass, alongside the unchanged checker and warning-denied Clippy. The
normal release app and developer visibility example built in 38.22 s. Format
and diff checks pass. No timing, game/window or installation runs were made.


## THE-860: Q2 connected snapshot delivery

`network/src/snapshots.rs` now supplies one load-selected native storage
variant per CLIENT channel. Q2's 68/20-word records and Q3's 112/51-word
records use the existing generic Ring implementation, not separate engine
player/entity stores. Both remain 32-slot, with native parse-row retention
limits and a zero baseline. Channel's previous Q3-only mutable-ring API is
removed; its public borrowed snapshot view now carries the native record shape.

`network/src/ingress.rs` configures native payload storage at connection bind,
because QW and Q2 share channel header rules. Ordinary connected CLIENT Packet
delivery now drains Q2 svc_frame and native prints through the same callback as
Q3 commands/frames. The Q2 adapter borrows the existing Prints parser for layout,
print priority/text and centerprint; no second print reader is introduced.
Nop and byte-end termination follow CL_ParseServerMessage. Signon/configstrings,
sounds/effects, inventory and stuffed commands still return explicit unsupported
service errors; this is not a complete native server-message implementation.

`network/src/commands/connection.rs` preserves Q2's payload frame number as
clc_move lastframe, independently of channel sequence. The same per-connection
delta-request value serves CLIENT's accepted current frame or SERVER's received
native request. Missing, truncated or otherwise rejected current frame streams
clear it so the next command requests a full frame; older ring bases remain
available for native recovery. Q3 still selects clc_move/clc_moveNoDelta from
its native acknowledged server-message snapshot. Existing production Packet
routing and both output developer probes use the shared Incoming callback;
there is no additional physical intake or packet queue.

Two connected fixtures cover full/delta frames whose numbers differ from
channel sequences, native old-origin advancement, prints before/after a frame,
dropped packet/missing base/full recovery, truncated frames, bounded unsupported
services, idempotent binding and same-header payload mismatch. Existing Q3
connected fixtures remain. The developer snapshot example's --connected-heap
mode brackets Q2 store/write, Channel delivery, CLIENT frame/print dispatch and
native move feedback after load, without collecting timing samples.

Current app bypasses remain `app/src/host.rs:480` and `app/src/lib.rs:201`
(direct SERVER-to-prediction copies); app's CLIENT callback still does not
apply decoded native snapshots. Common projection/inverse application,
QW/NQ framing, rerelease/666/999, signon, captures/live legacy peers and
THE-3169 native/combined/installed acceptance remain required. No app
snapshot adoption, game run or installation is claimed for this component.
Evidence: `$HOME/.cache/qa-rust/THE-860-q2-connected-20261009/`.

All 687 workspace tests, the unchanged checker, warning-denied Clippy and
format/diff checks pass. The normal release app and developer codec examples
built in 33.20 s. Original-C comparison remains exact for 512 Q2 whole-frame
cases (913,057 bytes), 512 Q3 cases (2,035,577 bytes) and 16,384 scalar-state
cases (8,039,745 bytes and decoded words). The connected heap probe passes
660 iterations, including 60 warm-up and 600 measured iterations: positive
allocation control 1, zero caller allocations/reallocations/requested bytes
and zero command errors. This counter covers the connected native codec path,
not app frames, workers, OS/driver allocations or gameplay. No timing run.

## THE-3176: fourth audit, texture projection

`render/src/world/geometry.rs:289` now uses
`point.dot(Vec3([p[0], p[1], p[2]])) + p[3]` for load-time texture projection.
The local dot-product expression is deleted. Core Vec3::dot retains the same
three products and left-to-right additions; the projection offset is still
added last. Non-finite rejection, extrema and extent conversion are unchanged.

All 687 workspace tests, the unchanged checker and format/diff checks pass.
The normal release app built in 38.02 s. A separate release-mode expression
probe matches all 16,384 seeded projection rows bit for bit against the
compiled core helper. Existing geometry tests retain signed-edge, extent,
lighting and non-finite projection checks. No timing, game/window or install
run. Evidence: `$HOME/.cache/qa-rust/THE-3176-projection-dot-20261009/`.

## THE-860: decoded player fields

The existing `network/src/projection.rs` table now serves both SERVER reduction
and CLIENT application. There is no second player store, scalar decoder or
per-game import loop. Application consumes the existing decoder's words:
NQ QC float stats have become native integers, Q2 position/velocity use signed
eighth units and timers use 8-ms units, and Q2/Q3 delta angles retain their
signed/unsigned short forms through core short_to_angle.

Movement and trace RuleSetIds, absent hot fields, tuning, inventory arenas and
foreign module tails remain owned by the recipient. A native mode that cannot
distinguish existing common modes retains that choice; otherwise load-selected
mode data supplies its common meaning. Simultaneous represented timer flags
are restored without clearing a timer the protocol cannot express. NQ's water
bit changes only the known below/above-two boundary. QW's writer-only body yaw
placeholder never overwrites incoming context.

Client, weapon/model and ground ordinals stay in explicit native context; a
caller callback resolves Q3 ground into its own current lifetime namespace.
Native none clears contact without calling the resolver, and native world may
remain grounded without a common entity handle. No ordinal is cast to an
EntityId or registry handle. Effective native gravity/speed stay in context
until the movement adapter accounts for its chosen tuning/multipliers. Existing
ValueBinding imports update the same load-sized ValueBank; missing bindings
drop without allocating.

The existing four-protocol codec test now applies each decoded record to a
player with foreign movement/trace roles. Connected Q2/Q3 fixtures apply
borrowed CLIENT frames; two focused tests cover lossy modes, timer flags,
ground resolution/none, module tails, bindings and short-input rejection.
The connected heap probe includes common reduction/application.

App CLIENT application and the direct prediction copies at
`app/src/host.rs:480` and `app/src/lib.rs:201` remain for the next integration
slice. QW/NQ connected streams, rerelease/666/999, native signon, live legacy
peers and THE-3169 native/installed acceptance remain open. This component
does not claim native prediction, complete HUD/stat imports or gameplay.
Evidence: `$HOME/.cache/qa-rust/THE-860-player-import-20261010/`.

All 689 workspace tests, unchanged checker, warning-denied Clippy and
format/diff checks pass. The normal release app and developer codec examples
built in 33.77 s. Existing original-C codec comparisons remain exact for
512 Q2 frame cases (913,057 bytes), 512 Q3 cases (2,035,577 bytes) and
16,384 scalar-state cases (8,039,745 bytes and decoded words). These compare
the native codecs, not a complete player ABI or prediction implementation.
The connected caller-heap probe includes reduction and decoded application:
60 warm-up plus 600 measured iterations, positive allocation control 1,
zero allocations/reallocations/requested bytes and command errors. No worker,
OS/driver or app-frame claim. No timing, game/window or installation run.
