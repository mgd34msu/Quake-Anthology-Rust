# Quake-Rust instructions

Linear project Quake-Rust, P-THE-3, team The Artificery, is the source of truth.
The owner rules and working protocol apply. Set issues In Progress when starting,
put THE ids in commit subjects, and leave completed work In Review with evidence.
Never set Done. Slack #quake-rust is only for qualified installation notices.

One engine serves every game and every mix through common primitives. Behaviour
comes from qsrc, features from quake-typescript, proven algorithms and fixes from
the C port. Copy neither port's wrong structure. One console, cvar table, HUD
system, VFS, save path and load path. No SHA or content fingerprints.

Private displays and captured private audio only. Stop recorded owned PIDs only.
Install into qfiles/qa-rust only through the qualified installer after an exact
candidate run using a fresh copy of the owner's saved profile reaches gameplay
and quits normally. Preserve the original profile. R0 windows are not gameplay.

## Workspace

`crates/core/src/primitives.rs` owns entity, body, player state, usercmd, item,
weapon, damage, sound/effect event, HUD state and cvar handle values.

Capability crates are core, world, movement, formats, content for the VFS,
render, audio, network, session, gameplay, compat for module hosts, navigation,
bots, persistence, console, input, ui, platform and app. Games convert into
primitives at file, wire and module ABI boundaries. Game-specific rules belong
inside gameplay modules. There are no per-game console, cvar or HUD crates.
Core has no dependencies. Capability crates depend on core; app composes them.

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
R4 Q1 e1m1 and a mixed configuration, R5 Q1 complete, R6 Q2, R7 Q3/TA and bots,
R8 mods together, R9 combined mode, R10 audio/menus/UI, R11 network,
R12 measured performance, R13 release.

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

* Channel **#quake-rust**. Post only when a new qualified `qfiles/qa-rust` is installed: build time, issue ids, what to retest. No progress chatter.
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
* Timing runs: pinned cores, no debugger, 600 frames after warm-up, report median and p99.

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
