# Quake-Rust instructions

Linear project Quake-Rust, P-THE-3, team The Artificery, is the source of truth.
The owner rules and working protocol apply. Set issues In Progress when starting,
put THE ids in commit subjects, and leave completed work In Review with evidence.
Never set Done. Post status, builds, installs, retests and progress to Slack
#quake-rust (`C0C7J4B0QAE`) without an identifier prefix; keep Linear current on
every issue with its status, commit ids and evidence.
Slack #quake-discussion (`C0C7Z7CLE3E`) is for design discussion only.
Prefix every message sent there with `[QA-RUST]` and post in the channel,
never in threads. During active work, check for posts mentioning `[QA-RUST]`
and follow channel replies to questions and discussions you participate in.

One engine serves every game and every mix through common primitives. Behaviour
comes from qsrc, features from quake-typescript, proven algorithms and fixes from
the C port. Copy neither port's wrong structure. One console, cvar table, HUD
system, VFS, save path and load path. No SHA or content fingerprints.

Private displays and captured private audio only. Stop recorded owned PIDs only.
Install into qfiles/qa-rust only through the qualified installer after an exact
candidate run using a fresh copy of the owner's saved profile reaches gameplay
and quits normally. Preserve the original profile. R0 windows are not gameplay.
Owner decision THE-2882 (2026-10-08 19:1x): the exact qa-rust-preview destination
may receive a clean normal render preview after copied-profile private GL/CPU
runs of e1m1, base1 and q3dm1 quit normally. The installer writes the adjacent
qa-rust-preview.txt with commit, build time, supported maps/backends, limits and
run examples. Preview receipts explicitly have no gameplay/timing qualification;
later previews replace only that destination and notes. qa-rust keeps both its
gameplay and comparable measured gameplay timing gates.

## Workspace

`crates/core/src/primitives.rs` owns entity, body, player state, usercmd, item,
weapon, damage, sound/effect event, HUD state and cvar handle values.

THE-2884: core owns the one `RuleSetId` identity type. Every capability role
selects its own value; source provenance, module policy, movement, damage,
trace, linking and scheduling must not inherit one another implicitly.
Preserve its five source-column discriminants (Q1/QW/Q2/Q2RR/Q3 = 0/1/2/3/4).
Do not recreate MovementRules, console Source or ThinkTiming identity enums,
including renamed copies. TickRate, LinkOrder, ModelRules, TraceRules and
DamageRules are capability data, not additional game identity types.

THE-2868 keeps one CollisionStore with one flat model table for every loaded
hull or brush resource. GeometryId generations are internal lifetimes; native
inline ordinals and protocol fields remain unchanged. World and linked traces
pass explicit resource/model identities and caller-selected trace rules. Native
load bounds expand once by one unit; entity linking has a separate expansion.
Entity-role ModelRules select rotation and link bounds independently of map
format or movement. Store-owned transforms preserve Q1 hull-offset arithmetic,
Q2 inverse-angle normals and Q3 double centering/transpose normals. THE-2883
retains an optional common point-contents pose for native ABIs whose published
pose differs from their physical trace/link pose; it never changes the area
index or adds another collision implementation. Guest ABI and live mover
acceptance remain required beyond structural and headless checks.

THE-656 stores clients in one array sized at load. Internal ClientId is u32;
native limits (including Q2/RR 256 and Q3 64) and wire widths belong at each
protocol/module boundary. A connection supplies its native entity namespace
explicitly, or None before binding a native module. Never derive native entity
numbers from common entity reservations or the module executing gameplay:
Q3 client zero is native entity zero; Q2 client zero is native edict one.

THE-617 keeps byte-exact name identities in one load-built arena. ASCII-folded
equivalence is a cached numeric lookup, never a reason to merge exact names.
Target queries select exact matching for Q1/QW or folded matching for Q2/Q3
at the caller boundary; map geometry does not select string semantics. Native
item/function classname dispatch uses exact names per qsrc strcmp. Optional target fields retain
the difference between an absent string and an explicit empty NameId(0).
Q1 native import maps its zero string offset to that explicit empty value.

Capability crates are core, world, movement, formats, content for the VFS,
render, audio, network, session, gameplay, compat for module hosts, navigation,
bots, persistence, console, input, ui, platform and app. Games convert into
primitives at file, wire and module ABI boundaries. Game-specific rules belong
inside gameplay modules. There are no per-game console, cvar or HUD crates.
Core has no dependencies. Capability crates depend on core; app composes them.
The engine and everything shipped must be Rust. Python is allowed for developer
tooling such as builds, checks, private harnesses, installation, timing and
generators; that tooling is not part of the shipped platform.

`cargo build --release` uses opt-level 3, fat LTO, one codegen unit and abort
panics. `python3 tools/build.py` splits debug symbols into qa-rust.debug and
produces the installation candidate. Baseline CPU is the portable default;
`--cpu native` is a measured machine-specific build. One target directory is
used by all build modes: target. No global fast-math.

## Order

R0: THE-596, THE-597, THE-598, THE-599, THE-600, THE-601, THE-603, THE-602.
R0 additions: THE-605, THE-678, THE-626 (closed by the supervisor), THE-633,
THE-640, THE-644. Muse lanes and worktrees have been retired by the owner;
do not recreate them. Fetch with prune before pushing.
Then R1 primitives/VFS/formats, R2 console/cvars/input/frame loop, R3 renderers,
R3.5 THE-839 three-game walk-through gate, R4 Q1 e1m1 and a mixed configuration,
R5 Q1 complete, R6 Q2, R7 Q3/TA and bots,
R8 mods together, R9 combined mode, R10 audio/menus/UI, R11 network,
R12 measured performance, R13 release.

R1 primitive implementation order, authorised after the supervisor's R0 review:
THE-611, THE-617, THE-625, THE-636, THE-642, THE-650, THE-656, THE-673,
THE-680, THE-691, THE-697, THE-702, THE-709, THE-711, THE-718, THE-726,
THE-790, THE-770, THE-776, THE-777, THE-780, THE-792, THE-784, THE-786.
R0 THE-599/600/601/603/640/678 remain In Progress until gameplay proves their
acceptance criteria. Build the R1 capabilities in order; R4 supplies the map
spawning, mixed-play and installed-binary evidence they require.

Completed R2 structural order:
THE-613, THE-623, THE-630, THE-639, THE-859, THE-892, THE-884, THE-885,
THE-887, THE-888, THE-889, THE-890, THE-886, THE-901, THE-891.
The 2026-10-08 18:50 ruling prioritises THE-2882's retail Q2 plane-load
regression and the requested qualified installation, then THE-2868. The
approved collision design uses one flat model store with generation handles,
caller-selected trace rules, entity-role pose/link rule tables and distinct
load/link bounds expansion. Delete the format enum, model-0 path and duplicate
model tables in the same slice; internal geometry handles never go on the wire.
THE-2879's later format directive follows the current primitives slice:
THE-1681/THE-2438 .lit lighting and THE-938 KPF/PKZ first, then
THE-974/THE-1260/THE-979 fonts, THE-794/THE-1928 demos and THE-1218/THE-1950
cinematics. Every format extends the one reader for its kind. AAS (THE-2026)
and NAV2 (THE-2030/THE-1182) accompany R7 bots. MP3/FLAC/Opus music (THE-959)
and IQM models (THE-1483) are required. Stock presentation remains original;
new streaming readers use fixed load-sized state and zero frame allocation.

The remaining 2026-10-08 18:10 core order remains required:
THE-2875 removes unused FrameArena and duplicate liveness storage, then THE-2868
puts inline collision models into the shared geometry store. One rule-set id
type selects rules per role: movement, damage, link order, tick rate and trace;
client module rules, never map format, choose tick rate and insertion order.
Then THE-2872/THE-2865 supplies one job dispatcher and automatic raster bands,
THE-889, interned NameIds for renderer material/image caches and folded cvar/
command lookup, THE-2869, then THE-859 and THE-697/890. The shared StampSet and
THE-892 duplicate checker apply to collision, visibility and GL sky marking.
Unfinished R1 work remains required. THE-862 q3dm1 speed work stays paused.
Earlier structural commits do not satisfy live acceptance criteria by themselves.
R1 issues with live acceptance criteria remain In Progress for integration at
the three-game gate. `tools/gen_cvars.py` compiles the vendored owner CSV in
`data/` into one Rust catalog. `tools/build.py` rejects stale generated output
before compiling. Preserve the CSV's per-source defaults, flags, aliases and
conversions; inferred type/range hints are metadata, not runtime validators.

Design every R1-R3 primitive and shared service against Q1, Q2 and Q3 now.
Before R4 monsters, weapons or saves, THE-839 must prove e1m1, base1 and q3dm1
loading, rendering on GL and CPU, and walking with their own movement and with
another game's movement. Use the same VFS, readers, entities, traces, area index,
usercmd, player state, console/cvars, event ring and renderers. Per-game code
contains movement rules and boundary conversions only. The shipped candidate's
private-harness evidence must include world screenshots, real key-repeat walks
with wall/step collision, Q1/Q3 cvar aliases in every map, and measured timings.

## Unified engine architecture

The [Unified engine architecture](https://linear.app/the-artificery/document/unified-engine-architecture-1f0df1cfc792)
is the target. These are ownership contracts, not claims that integration is
complete. Every capability must accept independent choices per world, entity,
player and client. Each playable feature needs per-game and combined-mode proof,
such as a Q1 map with Q3 movement, Q2 monsters and a Q2 client.

PlayerState stores independent movement and trace RuleSetIds. Native presets
initialize both at the boundary; an explicit trace choice survives SERVER,
prediction and bot processing. Movement probes resolve the caller trace id,
never movement or geometry. CLI `--trace-rules` permits the independent choice.

This is a new unified engine, not a Q3 engine extended with other games. qsrc
defines gameplay results, stock appearance and byte-exact legacy protocols;
it does not prescribe the engine's storage, allocator or scheduling structure.
Every structural choice earns its place through pinned timings, memory safety
or simplicity. Preserve observable native ordering and arithmetic without
copying recursive stacks, global mutable scratch or per-game capability code.

Every primitive has exactly one implementation. When it replaces an old copy,
delete that copy in the same slice and extend the duplicate checker. Collision,
visibility, GL sky marking and surface-cache pins use the one core StampSet.
The checker rejects duplicate public core types and renamed epoch-mark loops
in production, examples and tests. Core forbids unsafe
code; use safe typed owned storage allocated at load. Other unsafe code needs
an explicit safety invariant and focused memory-safety proof, and belongs only
at OS/SDL boundaries or in paths whose measured gain justifies it.

* A. THE-859/885/886: platform alone owns SDL, sockets, files, OS clocks and worker creation; one fixed core system-event ring carries timed input, console lines and packets, including fixed local loopback rings. No receive thread, journal, input recording or replay may be added pending the owner.
* B. THE-884: Com_Frame performs two nonblocking physical intake drains: before SERVER and before CLIENT, each followed by commands, matching qsrc Com_Frame/Com_EventLoop. SERVER providers advance at their own native rates on one timeline; CLIENT applies snapshots, predicts and presents. No other physical intake point is allowed, including frame-cap waits or final-ACK retirement. The 2026-10-08 16:12 owner ruling supersedes the earlier draining-wait requirement; queued events may still be consumed without polling OS sources. The cap waits on platform time before the first intake, using native integer-millisecond boundaries and a zero startup baseline. Q1 nextthink seconds, Q2 10 Hz, Q2 rerelease 40 Hz and Q3 sv_fps are independent of movement rules and usercmd duration.
* C. THE-691/890: all modules produce sound, effect and print primitives into the one core output ring and text arena; the client drains it once into audio, particles and per-seat HUD/notify consumers. Load-sized arenas, fixed rings and hot SoA state mean zero Rust heap allocation per frame; instrumented qualification fails on any measured-frame allocation.
* D. THE-860: one netchan owns sequence, ack, reliable bit, qport, fragments and fixed packet buffers; protocol tables select reliability, Huffman/XOR and one field-table delta encoder. Each client has a 32-slot snapshot ring and zero baseline, with its own NQ/QW/Q2/Q2RR/Q3 protocol over the same world; packet limit is 1400 bytes.
* E. THE-861/862: one scene API registers assets, clears a scene, adds entities/polys/lights, renders a refdef and submits 2D draws to one double-buffered command list. One Q3 stage material table converts Q1/Q2 surface flags at load into shared materials and one lightmap atlas. Shared iterative leaf/PVS/visframe/dlightframe/frustum traversal serves every map. GL uses static map VBOs, persistent dynamic buffers, material/lightmap batching and a state cache, never glFinish; CPU rendering uses depth-ordered edge/spans, a rover surface cache per mip and a 1/Z buffer with SIMD; standalone Q1/Q2 default to native palette/colormap lighting, including Q2 6-bit gray lightmaps. RGB and colored CPU lighting are opt-in or custom-game presentation.
* F. THE-863: one EngineServices table provides trace, link entity, sound, print, cvar, configstring and file operations. Thin numbered QVM, native dllEntry/game_import_t and QuakeC builtin mappings call those services. Module memory is checked at load; several module formats coexist and retain their own tick rates.
* G. THE-889/891: one usercmd builder serves every client, including bots in SERVER ticks rather than local-seat overrides. One Pmove-style entry chooses movement rules per player and runs identically for server, prediction and bots over shared trace services. AAS and NAV2 are data behind one bot/navigation interface.
* H. THE-887/888: one console tokenises its fixed text buffer in place with borrowed argv spans, sorted command lookup and cached cvar handles; a bare cvar command uses argv 1 per qsrc. One bind table dispatches normal and +/- commands into the same console and per-client builder. Every alias works in every game; bare text is a command/cvar, chat only via say.

Legacy interoperability is mandatory. Every game must connect to original or
reference servers and accept their clients using NQ 15 (666/999 only when
negotiated), QW 28, Q2 34 and the rerelease protocol, and Q3 68/Team Arena.
Unified entity, player, usercmd, event and configstring records must always
convert to each protocol's exact fields and widths. Values or capabilities a
protocol cannot carry are mapped or dropped at its boundary, never fatal.
Handshake, challenge, channel framing and delta rules stay byte-exact; extensions
use that protocol's own negotiation. Check every primitive and event change
against this rule. THE-860 requires round trips against original captures and
live connections with original or reference servers and clients for each protocol.

THE-650 distinguishes explicit module LinkEntity from an internal body commit.
An explicit relink always unlinks and reinserts; head/tail insertion is rule data
of the entity's game. Unchanged internal body commits stay no-ops. THE-625/1862
trace calls carry the caller's clipping, epsilon and filtering rules independently
of the map geometry; stock results remain bit-exact. Linked-body hits use the
caller's merge rule: Q1/Q2 replace on allsolid, startsolid or nearer fraction,
preserving any earlier startsolid; Q3 keeps its native solid-flag and nearer-only
replacement semantics (qsrc Q2 sv_world.c:566-578; Q3 sv_world.c:570-586).
THE-697/890 output payload
pages belong to ring slots and retire after every applicable module/client
consumer has completed its native delivery rule. Reliable records wait for a
real native ACK; best-effort records retire after successful native submission;
unsent records remain retained. Delivery rules are protocol data, and a transmit
watermark never counts as an ACK. NetQuake unreliable datagrams have no ACK
(qsrc WinQuake/net_dgrm.c:370-395,427-431); no new wire field may be added to
retire them. Count overflow and bounded slow-client resync separately from ACKs.
HUD text holds an independent display lease throughout its lifetime; no event
payload cloning or frame arena reset may invalidate a slower consumer.

Use modern techniques where pinned timings prove a gain: fixed multicore
partitions with ordered merges, SIMD, modern GL and cache-friendly storage.
Partition load work per file/lump, scenes per view, raster per screen band,
surface fills into serially reserved cache slots, snapshots per client, bots
and read-only traces over a frozen world. Merge bot commands by client id.
Keep observable qsrc think/entity ordering serial. Platform owns worker threads;
bounded audio mixahead and a render worker are adopted only after measurement.
Report pinned median and p99 over 600 measured frames after 60 warm-up frames,
with matched workload and fidelity, without a debugger.

THE-892 checks platform ownership and duplicate event/output storage, including
examples and imported aliases. Developer timers also use platform. CPU raster
selects 1/2/4/8 bands at load, dividing one total 32 MiB cache budget across them.
Platform owns the persistent worker pool. Runtime allocation qualification sums
the instrumented calling thread and every worker after every completed or
rejected dispatch, then consumes those counts once in ordinary and quit frames.
Discard startup counts; do not report only the final batch. SDL/driver heap work
needs separate measurement and is not proved by the Rust counter.

THE-861 scene contract: asset registration happens at load and returns numeric
material/model handles. The shared front end clears a scene, adds entities,
polygon vertices and lights, renders a copied refdef, and appends 2D draws.
Two command lists own their entity, polygon, light and vertex arenas; commands
contain ranges into that same list, never pointers into a client module. Lists
alternate after submission, and a list is reused only after its consumer has
finished. Capacity failure drops the affected submission and is counted; it
does not grow buffers or terminate play. Refdefs carry viewport, camera axes,
FOV, time, area mask and screen blend. Blend phase preserves native presentation:
GLQuake blends before 2D drawing; Q1 CPU shifts its final palette after the HUD.
The one view input is applied once in its specified phase. Area masks use
1=hidden; Q2's visible area bits are inverted at its boundary. Start with one
render thread; ownership
must permit a later measured handoff without changing the scene API.

THE-862 load conversion uses one material stage table and one world surface
record. Q1/Q2 flags and Q3 shaders become material ids, while surfaces retain
their plane, polygon boundary, texture projection and lightmap coordinates for
the native CPU span path as well as static GL triangles. World owns the common
iterative point-in-leaf/PVS traversal; render owns per-view leaf/node/surface
visframe stamps, frustum tests and dynamic-light stamps. Different views/worlds
must not share mutable visibility scratch. CPU palette and colormap resources
belong to the selected presentation, independently of movement and modules.
SDL3 owns GL context creation/currentness/swap and streaming CPU presentation;
render resolves GL functions once through platform and owns GPU resources.

THE-886 reads Linux stdin through an independent nonblocking file description,
bounded per poll, into ConsoleLine events. Commands use the shared console;
no terminal reader belongs in app or a game module. Private checks use an owned
pipe or PTY, never the owner's terminal. Other OS stdin sources remain pending.

THE-891 owns `qa_movement::pmove(UserCmd, &mut PlayerState, trace)` and a
function entry per movement-role RuleSetId value. SERVER consumes local, remote and bot
commands through that entry; current-command prediction calls it on separately
owned hot state. Modules, wire protocols and map geometry never choose physics.
MovementState owns grounding, stance, timers and cached tuning independently
of the module tail. Contact/surface metadata comes from the shared trace API.
The app enables these callers when geometry is loaded; the R0 shell has none.
No command history, input recording or replay is introduced. THE-821 supplies
later network acknowledgement/correction. Native movement completion remains
THE-635/766/606/609 and the THE-839 retail-map and combined-mode gate.
Use `tools/check_movement.py` for original Q2/Q3 function comparisons and the
platform `movement` example for pinned allocation/timing checks. Analytic
fixtures do not qualify gameplay, installation or complete native physics.

Standalone games must look original. CPU uses that game's software look;
GL uses GLQuake, ref_gl or Q3 presentation and original cvar defaults. Native
Q1 r_wateralpha is 1. Modern internals do not change the default image.
See-through liquids, RGB/colored CPU light and new effects require explicit
cvars or custom/combined games. Mods use their target engine's presentation;
THE-896 Arcane Dimensions targets Quakespasm-Spiked.

THE-709 stores think deadlines and function handles in independent entity SoA
columns. Clearing a due deadline preserves its function. The shared per-entity
entry selects the owning module's scheduling rule and supplied native clock,
independently of map and movement. Seconds and signed integer milliseconds stay
tagged; Q1/QW/Q2 float narrowing, Q3 float comparison of native integer times
and rerelease exact int64 milliseconds remain native. Callback handles are u32,
without an invented 16-bit function limit. QW repeats due reschedules after
re-resolving the same lifetime and rereading its function/owner. Module adapter
rejection stops that entity; VM execution budgets belong to the module host.
Native physics providers call this entry at their original phase positions;
the slot-scan helper does not establish a universal pre-physics think phase.

THE-895 keeps compiled gameplay PVS for module sight, snapshot culling and
sound PHS. Enhanced render visibility is separate and enabled only by liquid
alpha below 1, custom games or a mod that targets it; Q2/Q3 retain shipped
translucency. That issue permits reusing the already-computed qsrc map CRC
with the map name for its render-PVS cache, without a new content hash.

Each shared capability supports the best id/source-port feature level.
Compatibility adapters expose original limits, timing, gameplay PVS, builtins
and syscall semantics to stock modules. Extensions and raised limits become
visible only through native checkextension, protocol or API-version
negotiation. For AD, FitzQuake 666/QSS 999 are data on the common channel;
its requested effects, limits and builtins use the same services and renderers.

THE-896 requires Arcane Dimensions to work natively with any map, movement,
HUD or module family, both standalone and combined. Its audited inventory is
`port-audits/2026-10-07/arcane-dimensions-requirements.md`: 53 called extension
builtins beyond stock, CSQC with nine entry points and 35 called builtins,
effectinfo/weather particles, skeletal operations, surfaces, strings/files,
sprintf, stats, skyboxes, .lit and fog. Each is a shared capability on the
unified primitives. THE-863 hosts CSQC as a client module alongside Q3 cgame/ui;
THE-861 supplies its common scene and 2D interface. Effectinfo feeds the one
particle system. Native semantics come from FTE pr_bgcmd.c, pr_cmds.c,
pr_csqc.c and Quakespasm; extension exposure still requires native negotiation.
Carry these requirements into the existing R1-R3 services and R8 module work;
do not add a Q1-only AD implementation or change the authorised issue order.

THE-901 follows the current R2 architecture items and moves platform to SDL3.
Use SDL_SyncWindow after state changes and SDL3 events, audio streams, gamepads
and timers through platform only. It requires private host-frame/input/time
and allocation checks under owned headless sway and weston as well as Xvfb.
X11 launches retain the forced-X11 environment. The Wayland checks must use a
separate private runtime directory and an explicitly selected owned compositor
socket; never inherit the owner's WAYLAND_DISPLAY or desktop socket.
`tools/check_sdl3.py` exercises owned Xvfb, sway and Weston with copied candidates
and profiles. Use SDL3 hint names as well as the mandated legacy names:
SDL_VIDEO_DRIVER, SDL_AUDIO_DRIVER and SDL_AUDIO_DISK_OUTPUT_FILE. Future window
state changes must call SDL_SyncWindow before reporting completion. The private
PCM probe accepts only explicit disk/dummy sinks; it does not prove game audio.

THE-893 is a release blocker: `crates/app/src/proof.rs` and platform's gated SDL
input injector form an input player. Remove recording/replay before any public
release. A development proof build must never qualify as a shipping candidate.

## Salvage

Read the [Salvage map from muse-final](https://linear.app/the-artificery/document/salvage-map-from-muse-final-071964446627)
before mining retired code. It lists reusable algorithms and readers, and banned
architecture. Keep the primitive-first workspace. Each R1-R11 issue using salvage
must cite the exact `muse-final:path` it read in its evidence comment and commit.

# Owner rules for the Rust port

These are Mike's standing rules. They apply to every commit.

## One single engine

* One implementation per capability, serving every game (Q1 classic and rerelease with mission packs, QuakeWorld, Q2 classic and rerelease with packs, Q3/Team Arena) and every combined-mode mix.
* Per-game code holds only that game's rules. Same-game assumptions (map game = movement game = weapon game) are defects.
* Build from common primitives (entity, body, player state, usercmd, item, weapon, damage, sound/effect events, HUD state, cvar); each game converts at its boundary.
* Use Rust for what it's good at: plain structs, structure-of-arrays for hot data, enums for tagged unions, function tables chosen at load time, arenas, no per-frame allocation, no layers of validation wrappers.

## Mods, all together

* Multiple mods at once; Q1 mods in Q2, Q3 and the rereleases, and the reverse.
* QuakeC progs.dat, Q2 game .so/.dll and Q3 QVM loaded together; .pak and .pk3 in one VFS.
* Any module format on any OS: interpret the file regardless of what it is or which OS is running.
* Custom game: any map, monsters, weapons, movement or character from any game, all interchangeable.

## Console and cvars

* One console and one cvar table, normalised to Q3 names plus the cvars other games add (`port-audits/2026-10-06/unified-cvars.csv`).
* Any cvar name works in every game with its expected function (`fov` = `cg_fov`, `cl_maxfps` = `com_maxfps`).
* The console never needs a slash: typed text runs as a command or cvar; chat only via `say`.

## No hashing

* No SHA or crypto hashing. The only exception is the LLM OAuth PKCE S256.
* No content fingerprints for identity, change detection, cache keys or saves.
* Hash-table bucket keys are fine. Original protocol checksums stay (MD4, CRC_Block, QW map checksum2, Q3 pure checksums, progs CRC16, zip CRC32).

## Saves

* Game state only, lightweight.
* Vanilla sessions read and write the original formats (Q1 v5/v6 text, Q2 save/<slot>, Q3 per qsrc).
* Custom games use one compact format and their own slot names.
* One save path and one load path. Autosave only at level entry and `target_autosave`.

## Performance: lightning fast

* Targets per frame: GL 1080p under 2 ms; CPU renderer 640×400 under 4 ms; CPU 320×200 under 2 ms.
* Judged only by measurement (pinned medians and p99, no debugger), never by line counts or test coverage.
* No tests written to raise a number; tests check behaviour against qsrc and retail data.

## Authorities

1. Behaviour: the original id sources in the `qsrc` reference checkout (and the rerelease sources there).
2. Features: quake-typescript (feature list only, never its structure).
3. Proven implementation: the C port the `quake-anthology` reference checkout (behaviour and fixes, not its per-game structure).

## Never

* No TypeScript structure: no Rc<RefCell> entity pools, string-keyed ids/thinks/providers, NumericOps/fround emulation, SaveJson, mirror/seam/donor shims, async pumps.
* No fatal checks during play: a failure affects only the client, command, bot or entity involved.
* No game windows or audio on the owner's desktop or speakers during agent runs; stop only your own process ids.
* Strip all keyboard/input recording and replay before any public release.
# Working protocol

## Linear (source of truth)

* Project **Quake-Rust** (team The Artificery, keys THE-NNN). Work in milestone order; the first open milestone comes first.
* Set an issue **In Progress** when you start it.
* Put the issue id (THE-NNN) in every commit subject.
* When a change is committed, installed and proved, set **In Review** and add one comment: commits, build time, what was proved and how (evidence paths), and any limits.
* **Never set Done.** The supervisor reviews the evidence and closes.
* File every new defect you find as an issue (search first to avoid duplicates).

## Slack

* Channel **#quake-rust** (`C0C7J4B0QAE`). Post status, builds, installs, retests and progress without an identifier prefix. Include issue ids, commits, actual build time, evidence and limits; request retests only for qualified installed changes.
* Channel **#quake-discussion** (`C0C7Z7CLE3E`) is for design discussion only. Use `[QA-RUST]`, post in the channel rather than threads, and check mentions and discussions during active work. Each proposal also checks whether the C port has the same gap.
* The supervisor posts check-in summaries and owner retest requests there.

## Evidence

* Objective evidence closes issues: screenshots of the exact spot, captured audio with cue names and non-silent statistics, event/console logs, save headers, numbers compared with qsrc and retail data.
* Unit tests are not proof of a live feature. Proof is the shipped binary running real content.
* Report partial results as partial. Withdraw overclaims. Never claim a fix from a commit alone.
* Test the owner's conditions: a copy of his saved settings, real key auto-repeat, his frame cap and high uncapped rates, and the menu route as well as command-line launches.

## Installs

* Every install into `qfiles/qa-rust` goes through a qualified installer (port the C tool `quake-anthology/tools/install_qualified_build.py`): a private launch with a fresh copy of the owner's saved profile reaches gameplay and quits normally; the owner's original profile stays unchanged.

## Private runs

* On this host Xvfb alone is insufficient: every game launch must unset
  WAYLAND_DISPLAY and force SDL_VIDEODRIVER=x11. Use SDL_AUDIODRIVER=dummy for
  silent checks, or a private sink/disk capture for sound proof. The harness
  enforces `env -u WAYLAND_DISPLAY SDL_VIDEODRIVER=x11 SDL_AUDIODRIVER=disk`.
* Game windows and audio only on private displays and audio servers (Xvfb or a private X server on the spare RTX 5060 Ti; the RTX 3090 drives the owner's desktop), with a window manager; use window capture for screenshots.
* Copy binaries and profiles; never write into `qfiles` except through the installer.
* Stop only process ids you recorded; never pkill/killall by name (the C agent runs a similarly named binary).
* Wrap commands in `timeout 300`. This agent uses `CARGO_TARGET_DIR=target`;
  delegated agents must use their own target directories.
* Timing runs: pinned cores, no debugger, 600 frames after a 60-frame warm-up,
  vsync off. Report median and p99 for sim, scene, draw, present and audio once
  those stages exist. R0 shell stages do not qualify renderer performance.
* The installer must reject a measured regression over 10% against a comparable
  baseline. Shell timings cannot qualify gameplay or supply that baseline.
  See the [Proof and measurement protocol](https://linear.app/the-artificery/document/proof-and-measurement-protocol-045c1cc12ab8).

## Repo

* Unit tests cover format readers, math and rule tables against qsrc values.
  Put tests in tests/ or a small trailing test module. Production files with
  more than 40% test-only content fail the build checker. Never assert message
  wording. End-to-end proof goes through the normal input path in a binary,
  with scripted input only in the development proof candidate.
* Do not create GitHub releases or version tags unless the owner explicitly asks.
* Keep `AGENTS.md` in the repo root with these rules and the current milestone order, so they survive restarts and context compaction.
* Small commits, one capability or fix each, with the qsrc reference in the message.
## C-port lessons to check at each milestone

Use indexed values, arenas and structure-of-arrays for hot state, with function
tables chosen at load. Test mixed configurations with each playable feature.
Resolve cvar handles once. Validate once at external boundaries, trust owned
state, and keep gameplay errors scoped to the client, command, bot or entity.

Check Q1 jump and temporary-entity sounds, teleporter fixangle/hold, spawnflag
filters in every edition, skill rounding/carry, item removal/rotation, centred
centerprint, sbar backtile, fence alpha at every mip and worldspawn music.
Q2 uses its layout interpreter, original gun visibility/hand and spawn facing;
rerelease crouch dimensions preserve the floor. Q3 preserves mouse angles,
ignores repeated keydown for held +binds, honours the 85 fps default, draws sky
once, preserves HUD alpha and clears bot slots on travel. Notify lines keep
newlines. Original behaviours win, including retail high-fps jump rebound.

Measure before/after: no glFinish or loop delay, persistent buffers/batching,
GPU skinning, fog prepared per span and skipped at zero density, integer samplers
selected per draw, SIMD blends, inline spans and parallel geometry/raster.
Estimates are not performance claims. Preserve vanilla saves, distinct custom
slots and the owner's original profile. Prove features on the shipped binary.

Full lessons: https://linear.app/the-artificery/document/lessons-from-the-c-port-apply-from-day-one-401c0853f3f8


THE-884/THE-650 built-in walk-through policy: `--client-module q1/qw/q2/q2rr/q3`
selects a client policy without claiming a guest module is loaded. Stock defaults
use the winning content mount's product-root metadata, including edition;
unknown or ambiguous roots require an explicit choice. Movement and trace ids
default independently to that client. Tick rate and first-link insertion order
come from the client policy, never BSP format or movement. The one timeline
and area index consume the resolved rate/order; the app has no second link.
Saved settings retain the recognized product's root and rerelease directory;
unknown products use the explicit client's settings namespace. The cached Q3
sv_fps view is read for its client id, independent of console dialect. Values
below 1 receive the original sv_fps=10 write. Live module changes remain THE-1890.
