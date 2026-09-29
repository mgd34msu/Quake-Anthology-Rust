# Rust Port Plan — qa-muse

## 1. Source boundary (strict)

Port ONLY from `/home/buzzkill/Projects/quake-typescript` (code under
`src/`, `tools/`, `tests/`, plus design docs under `docs/` and
`verification/`).

FORBIDDEN: sibling trees (other native donor checkouts) are out of scope.
Do not read, list, or reference them; treat as nonexistent. Emergency use
requires explicit disclosure in the change description. All behavior
questions resolve against the TypeScript donor bodies and its
`docs/reference-contracts.md` selections (RC01–RC10).

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
  the donor's exact fround/operation order. No `glam` (would reorder
  ops). `MutableVec3` becomes `&mut Vec3` out-params.
- `numeric`: `enum ArithmeticProfile { Binary32EachOp, DonorBinary64,
  X87(..), Sse(..) }`; distinct `wrapping_*`, `checked_float_to_int`
  (RangeError → `Err`), `qvm_float_to_int` (INT_MIN) functions.
  `NumericProfile` selects per call chain, never global state.
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

- Float discipline: `f32` storage everywhere in world/render; `f64`
  only inside explicitly-marked `DonorBinary64` paths. Bit-pattern
  tests pin `dot3`, angle encode, int conversion boundaries, signed
  zero.
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
`bevy`, `glam`, `nom` in v1 — hand-rolled math and decoders preserve
donor operation order and keep the graph auditable.

## 9. Phased build order (verifiable units)

Gates every phase: `cargo fmt --check`, `cargo clippy -- -D warnings`,
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
