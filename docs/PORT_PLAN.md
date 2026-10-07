# Rust Port Plan — qa-muse

## 1. Source roles (strict)

Two references, two jobs. quake-typescript
(`/home/buzzkill/Projects/quake-typescript`: code under `src/`, `tools/`,
`tests/`, plus design docs under `docs/` and `verification/`) is the
source of FEATURES: what the unified engine must do, screen by screen and
system by system. `~/Projects/qsrc` (original Q1/QW/Q2/Q3 source,
quake-rerelease-qc, quake2-rerelease-dll, plus quakespasm/ironwail/q2repro
/fteqw) is the source of FUNCTIONALITY: how each game, expansion, and
rerelease actually behaves. Nothing is forbidden; both trees are required
reading.

Required process for every subsystem, no exceptions:

1. Read qsrc for how it should work (the engines are ground truth).
2. Read quake-typescript for how it was done there (features + prior
   unification attempt).
3. Design the idiomatic, efficient Rust version.

Do NOT carry TypeScript workarounds into the port: per-operation fround
emulation outside places that truly need bit-exact results, CPU emulation
of stock game DLLs for built-in gameplay, guest-memory projection, deep
copies, promise-shaped flow. When the donor and qsrc disagree on behavior,
qsrc wins; record the disagreement in the change description.

Grounded donor facts inspected for this plan:

- `package.json`: `start/dev` run `src/main.ts`, `typecheck` is
  `tsc --noEmit`, `test` is `bun test`, plus `policy`, `plan:check`,
  inventory, reference, and `verify` tools; no runtime dependencies.
- `tsconfig.json`: `strict`, `noUncheckedIndexedAccess`,
  `exactOptionalPropertyTypes`, `noUnusedLocals/Params`.
- `src/main.ts`: `parseApplicationCommand` with
  `version` / `help` / `weapon-behavior` / `list-content` branches, then
  `LlmSettingsService` + `StartupApplication` / `openApplication`,
  SIGINT/SIGTERM request quit.
- `src/core/math.ts`: Q3-pattern `vec3` with per-component `Math.fround`;
  `dot3` rounds each product and each ordered addition.
- `src/core/numeric.ts`: `int32`/`uint32` wrap, `float32` fround,
  `qvmFloatToInt` (binary32 in, `INT_MIN` on NaN/out-of-range),
  `Q3_BINARY32_PROFILE` vs `Q1/Q2_DONOR_PROFILE` (`donor-binary64`).
- `docs/contracts.md`: 15-contract table (math, numeric, identity, time,
  content, common, world, gameplay, scene, render, movement, protocol,
  execution, ui, session); generational identity handles, sync callbacks,
  `Simulation` has no renderer/SDL dependency.
- `docs/reference-contracts.md` RC01–RC10: Q3 typed vectors + explicit
  binary32 ops; Q3 command buffer / cvar registry ownership with
  per-family dialects; shared bounded byte storage with separate codecs.
- Subsystem layout: `src/{app,audio,bots,compat(q2,q3,qc,qvm),console,
  content,contracts,core,formats(q1-map,q2-map,q3-map,q12-model,q3-model,
  images),guest(abi,core,elf,pe,x86,x64,floating-point),input,llm,
  materials,media,movement(q1,q2,q3),network(common,q1,q2,q3,services,
  unified),persistence,platform,render,settings,text,ui,world,
  camera,capture,debug,types}`.

## 2. Rust workspace layout

Cargo workspace at repo root. One binary, library crates per boundary:

```text
Cargo.toml            # [workspace] resolver 2, members below
src/main.rs           # CLI binary `qa-muse` (port of src/main.ts)
crates/core/          # math, numeric, rng, time, identity, cmd, cvar
crates/content/       # vfs/mounts, catalog, formats (maps/models/images)
crates/world/         # actors, bodies, spatial, collision, movement,
                      # gameplay/combat/inventory, session/save
crates/net/           # bounded byte buffer, q1/q2/q3 codecs, transport
crates/guest/         # qvm interp, qc, elf/pe loaders, x86/x64, fp env
crates/compat/        # q1/q2/q3 + rerelease game shims over guest+world
crates/client/        # render (cpu + gl trait), audio mixer, input,
                      # ui seats, media decode, platform sdl
crates/app/           # bootstrap, application loop, bots, persistence,
                      # settings, llm, console wiring
docs/PORT_PLAN.md     # this file
tests/                # cross-crate integration tests (byte vectors)
```

Dependency direction (acyclic): `app -> compat -> guest -> world ->
content -> core`; `app -> client -> world`; `app -> net -> core`.
`core` depends on nothing internal. `world` never depends on `client`
or `net` (headless `Simulation`, per session contract).

## 3. Donor → crate mapping

| Donor | Rust target |
| --- | --- |
| `src/contracts/math,numeric,time,identity,common` + `src/core` + console cmd/cvar cores | `crates/core` |
| `src/content` + `src/formats` + `contracts/content,scene(images/models)` | `crates/content` |
| `src/world` + `src/movement` + `contracts/world,gameplay,movement,session` | `crates/world` |
| `src/network` + `contracts/protocol` | `crates/net` |
| `src/guest` + `src/compat/qc,qvm` + `contracts/execution` | `crates/guest` |
| `src/compat/q2,q3` + rerelease/native-mod shims | `crates/compat` |
| `src/render,materials,media,audio,input,ui,text,platform,camera,capture` + `contracts/render,ui,presentation` | `crates/client` |
| `src/app,src/bots,src/console,src/llm,src/settings,src/persistence,src/debug,src/main.ts` | `crates/app` + `src/main.rs` |

Large donor modules split on port, not transliterated: simulation
runtime → `world::{registry,scheduler,session}`; application bootstrap →
`app::{options,startup,application}`; menu/script blobs → `client::ui`
submodules + data tables.

## 4. Shared core design (`crates/core`)

- `math`: `#[repr(C)] #[derive(Copy,Clone)] struct Vec3 { x:f32,.. }`,
  free functions `add3/sub3/scale3/dot3/cross3/length3/normalize3` with
  qsrc numeric behavior: `f32` storage and C float semantics as the
  originals declare them. No `glam` (would reorder ops).
  `MutableVec3` becomes `&mut Vec3` out-params.
- `numeric`: qsrc numeric behavior per dialect (f32 storage, C float
  semantics, Q3 binary32); distinct `wrapping_*`,
  `checked_float_to_int` (RangeError → `Err`), `qvm_float_to_int`
  (INT_MIN) functions. `NumericProfile` selects per call chain, never
  global state. No donor-reproducing profiles: C `if (x)` is true for
  NaN, and QuakeC opcodes store binary32 after every op.
- `rng`: `struct Qrand(u32)` with `next()`, `frac01()`, `crandom()`;
  explicit seed + draw count; checkpoint = `(seed, draws)`.
- `identity`: generational `struct ActorId { slot, generation, session }`
  with private constructor; registry capability type owns creation;
  `SavedActorId { slot, generation }` for saves. No integer IDs.
- `time`: `enum SourceTime { SecF32(f32), MilliI32(i32) }`,
  `struct FrameContext`, per-source clock profiles; no host clock in sim.
- `cmd`/`cvar`: one bounded `CommandBuffer` + one `CvarRegistry`,
  parameterized by `enum Dialect { Q1Net, Q1Qw, Q2, Q3 }` for ordering,
  case rules, latch, alias/wait behavior. Instance-owned, sync dispatch,
  `&mut` world — no async callbacks, no process globals.

Errors: `thiserror` enums per crate (`MathError`, `SaveError`,
`ProtocolError`, …); `Result<T, E>` everywhere — no null/undefined
union returns, no exceptions-as-control-flow.

## 5. No TS-ism rules

- No class hierarchies → traits (`MovementProvider`, `CombatPolicy`,
  `RendererBackend`) + enums + generics.
- No `any`/structural soup → explicit structs, `#[non_exhaustive]`
  only at FFI seams.
- No GC handles → generational indices, arenas, `slotmap`-style tables
  (hand-rolled in `world`, no dep needed).
- No optional-chaining defaults → `Option` + `?`, total functions for
  hot paths.
- No promise callbacks in sim → sync `fn(&mut World, …)`; async only
  at `app` edges (content scan, llm browser open) via `std::thread`,
  no async runtime in v1.

## 6. DRY strategy

One owner per mechanism, dialect/profile parameters at call sites: one
math lib, one numeric profile set, one command buffer, one cvar
registry, one VFS + mount-plan resolver, one bounded net byte buffer
with per-family codecs, one RNG owner per provider, one render backend
trait with cpu/gl impls. Cross-game tables (items, weapons, pickups)
live in `world::data` as versioned `const`/generated tables, not three
copies. Contract traits live next to their owner crate (`world::traits`,
`net::codecs`), not in a parallel `contracts/` tree.

## 7. Performance approach

- Float discipline: `f32` storage everywhere in world/render, with
  qsrc numeric behavior (C float semantics per the originals, Q3
  binary32). Bit-pattern tests pin `dot3`, angle encode, int
  conversion boundaries, signed zero.
- No per-frame allocation: pools/arenas for actors, messages, draw
  cmds; `SmallVec<[T; N]>` / fixed `[T; N]` + len for frame temps;
  `&mut` out-params on hot math; frame scratch bump buffer reset per
  tick, never grown mid-frame.
- Zero-copy parsing: format decoders borrow `&[u8]` (`from_le_bytes`,
  slice splits, validated offsets); owned `Vec` only at catalog upload
  (textures, static geometry). No per-lump re-parse in loops.
- Layout: `repr(C)` math, SoA component tables in `world`, entity
  iteration by dense index; spatial queries return borrowed ids, not
  cloned actors.
- Budgets enforced by tests: allocation-count test on a fixed tick
  (custom allocator hook in `tests/`), plus `criterion`-free timing
  smoke only after Phase 3 (keep deps minimal until then).

## 8. Minimal dependency set (v1)

```toml
thiserror = "2"      # typed errors
clap = { version = "4", features = ["derive"] }  # CLI (port of options.ts)
serde = { version = "1", features = ["derive"] } # manifests, saves
serde_json = "1"
smallvec = "1"
bitflags = "2"
```

Planned, all in scope: native backends via `libloading`
for the donor's SDL/GL/FreeType/Vorbis/Theora surface, image decoders,
`mio`, compression. UDP starts on `std::net::UdpSocket`. No `tokio`,
`bevy`, `glam`, `nom` in v1 — hand-rolled math and decoders follow
qsrc numeric behavior and keep the graph auditable.

## 9. Phased build order (verifiable units)

Gates every phase: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
`cargo test --workspace`. Toolchain verified: rustc/cargo 1.98,
clippy 0.1.98, rustfmt 1.9.0.

1. **P0 scaffold**: workspace `Cargo.toml`, 8 crates, `src/main.rs`
   (`version/help` only), this doc. Verify: `cargo build`, gates green.
2. **P1 core**: math/numeric/rng/time/identity/cmd/cvar + unit tests
   porting donor probes (`dot([16777216,1,-16777216]…)==0` on binary32
   path, `qvmFloatToInt` NaN→INT_MIN, Q_rand sequence, dialect cmd/cvar
   order tests). Verify: `cargo test -p qa-core`.
3. **P2 content**: VFS + mount plans + read-only map/model/image
   decoders over `&[u8]`; round-trip + byte-offset tests on donor
   fixtures. Verify: `cargo test -p qa-content`.
4. **P3 world**: registry/scheduler/spatial/collision/q1-q3 movement
   providers, combat/inventory ownership, headless tick + save/restore
   round-trip. Verify: `cargo test -p qa-world`, alloc-budget test.
5. **P4 net**: bounded buffer + q1/q2/q3 codecs, angle/msg byte vectors
   (e.g. trunc-vs-scale cases), loopback client/server test on std UDP.
   Verify: `cargo test -p qa-net`.
6. **P5 guest+compat**: QVM interpreter + syscall shims, QC/Q2/Q3 game
   adapters, checkpoint/restore of guest views. Verify:
   `cargo test -p qa-guest -p qa-compat`.
7. **P6 client**: CPU rasterizer triangle kernel, mixer `snd_mix` port,
   input/ui seats, media decode; headless render-frame hash test.
   Verify: `cargo test -p qa-client`.
8. **P7 app**: full CLI (`weapon-behavior`, `list-content`, dedicated),
   application loop, bots, settings/persistence, llm edge. Verify:
   end-to-end `cargo test --workspace` + `cargo run -- --help`.

## 10. File layout (target)

```text
qa-muse/
  Cargo.toml  Cargo.lock  rustfmt.toml  clippy.toml
  src/main.rs
  crates/<core,content,world,net,guest,compat,client,app>/
    Cargo.toml  src/lib.rs  src/<module>.rs  tests/<unit>.rs
  tests/  # cross-crate vectors (net bytes, save images, frame hashes)
  docs/PORT_PLAN.md
  verification/  # ported byte vectors + manifests (donor-derived)
```

Naming: crates `qa-core`, `qa-content`, `qa-world`, `qa-net`,
`qa-guest`, `qa-compat`, `qa-client`, `qa-app`; modules snake_case
mirroring donor file names where 1:1, merged where the donor split
only for TS size.

## 11. Q1 native cutover (scheduled after the e1m1 monster set)

Rule: one implementation per capability. The live Q1 path
(`spawn_map_entities` → `native_q1_*`) is now a second Q1
monster+missile system alongside the older `qa_content::q1` one, and
the old one must go. Verified evidence (2026-10-05):

- `crates/content/src/q1/base/projectiles.rs::create_missile` is
  still called by `base/monster_actions.rs`,
  `base/map_entities.rs`, `addons/monsters/ordinary/rocket_ogre.rs`,
  plus mission-pack callers (`armagon.rs`, `rogue_ending.rs`).
- `app/bootstrap/simulation/monster_sources.rs` registers base,
  addon and mission-pack Q1 monsters from `qa_content::q1` (9 refs);
  `runtime.rs` has 271 `qa_content::q1` refs.
- `native_q1_*` uses none of it, with one leak:
  `native_q1_pusher.rs:12-13` imports `Q1EntityServices` /
  `Q1ThinkFrame` from `qa_content::q1` (must be removed in step 1).

Steps, in order (each gated: clippy `-D warnings`, workspace
units, `QA_REQUIRE_LIVE=1` live proofs):

1. Cut the leak: remove the two `qa_content::q1` imports from
   `native_q1_pusher.rs` (inline or move what it needs into the
   native modules). Verify: `grep -l qa_content::q1
   native_q1_*.rs` prints nothing.
2. Move add-on and mission-pack Q1 monsters onto the native path:
   re-implement per qsrc hipnotic/rogue progs (functionality) on
   `Q1Missile` / `Q1TempEnt` / native monster AI, with live proofs
   on their maps. Verify: each moved monster spawns and fights in
   the windowed run.
3. Repoint `monster_sources.rs` registrations to the native
   implementations; delete the `qa_content::q1` monster and
   projectile modules once nothing reaches them (check the
   reachability scan, not just grep). Verify: the deleted code is
   gone and the full gate is green.

Non-goals: Q2/Q3 content systems (their own verticals); behavior
changes to the native path during the move (port, then delete).

## 12. Mods: everything together (owner rule 2026-10-06)

One session runs several mods at once, mods from any game in any
game, every module format on any OS, and every archive in one VFS.
One mod folder may hold progs.dat next to gamex86.dll, .so game /
cgame / ui modules and QVMs: all load and run together against one
world. No per-game mod folders, no "this is a Q1/Q3 mod" gate, no
format picked by game family, no OS-picked loader.

### Where the port stands (verified 2026-10-06)

Module formats (any format, any OS):

- progs.dat (QC): interpreter exists
  (`crates/guest/src/qc/`). Wiring into the live session: missing.
- .qvm: interpreter exists (`crates/guest/src/qvm/`).
  Wiring into the live session: missing.
- Windows .dll (PE): loader exists
  (`crates/guest/src/pe/`), x86/x64 emulation exists
  (`crates/guest/src/x86/`, `x64/`). Execution from the
  live session: missing (per the intent audit, the Q2/Q3
  native hosts return synthetic answers and never run
  guest code).
- Linux .so (ELF): loader exists
  (`crates/guest/src/elf/`). In-process loading: missing
  (no dlopen/libloading anywhere in the tree).
- ABI: `GuestServerLogic` (`crates/guest/src/server.rs`,
  `crates/world/src/server.rs`) invents a vmMain shape no
  Quake module has. Real per-kind entry points (QuakeC
  builtins, Q2 `GetGameAPI`, Q3 `vmMain` code) are not
  wired.

One-mod / game-tied assumptions to remove:

- `crates/content/src/catalog.rs:2696-2698`
  `is_q2_game_module` plus `:2833`
  (`base.family == GameFamily::Q2 && ...`): a game module
  is only recognized when the product family is Q2.
  Replace with content sniffing (PE/ELF magic, QVM
  header, progs version) with no family gate.
- `crates/content/src/catalog.rs:1388`
  (`source_product.family != GameFamily::Q3`): Q3-only
  program gate. Same fix.
- `crates/content/src/catalog.rs:2822-2831` (`qwprogs.dat`
  check, `game_x64.dll` vs `gamex86.dll` by edition):
  program identity picked by edition. Detect from the
  bytes, not the shelf label.
- `crates/app/src/options.rs:275`
  (`mods: Vec<ModSelection>`): multi-mod selection shape
  exists and `startup_selection.rs` fills it, but
  composition of several loaded mods in one live session
  (load order, conflict rules) is not implemented.
- One VFS: `crates/content/src/mounts.rs:484`
  (`MountedContent`, explicitly no process-global search
  path) with per-plan `default_order`
  (`mounts.rs:1253`). Wanted: one plan whose order spans
  every installed game and every loaded mod (.pak next
  to .pk3); verify current plans are not one-per-product
  and unify if they are.
- One world: separate server/world models per game
  (`simulation/q3/server_state.rs:82` `Q3ServerState`,
  `simulation/q3/runtime.rs:902` `RuntimeQ3World`,
  `simulation/network_q1.rs:248` `Q1ServerOptions`,
  three `Q2ServerDataParams` copies in `network.rs:82`,
  `network_q2_guest.rs:48`,
  `network_q2_rerelease_native.rs:59`). Mods must run
  against one shared entity/world/presentation model,
  not their own game's copy.
- Live gameplay is native-Q1-only
  (`play_world.rs` gives native spawns to Q1 maps; Q2/Q3
  classnames are stubs), so no mod of any game has a
  live world to run against yet. The Q2/Q3 verticals
  unblock this.

### Single shared design

One `ModHost` in `qa_guest`: given a folder, enumerate
every module by content sniff (progs version, QVM magic,
PE/ELF headers; never extension, family, edition or OS),
load each with its real ABI (QC builtins, Q2 API,
Q3 vmMain/cgame/ui entry points; host-ABI .so via
dlopen, foreign-ABI images via the PE/ELF
loader+emulator on every OS), and run them side by side
in one session against the one shared world, one
entity model, one presentation model, one VFS plan
(order: loaded mods in load order, then every installed
game), one console and the unified cvar table, with
documented load-order and conflict rules (later mod
wins on entity spawn functions, first mod wins on
cvar defaults, all mods see all mounts).
