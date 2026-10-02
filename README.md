# Quake Anthology — Rust port (`qa-muse`)

A Rust port of the `quake-typescript` Quake anthology engine: one Cargo
workspace covering server, client, renderer, audio, input, UI, platform
backends, network transports, bots, tools, and game content for the
Quake family (Q1, Q2, Q3, QuakeWorld, rereleases).

Port source (hard boundary): the `quake-typescript` donor tree, scoped
to `src/`, `tests/`, `tools/`, `docs/`, `verification/`. All behavior
questions resolve against the TypeScript donor bodies. See
[docs/PORT_PLAN.md](docs/PORT_PLAN.md) for the crate layout,
donor-to-crate mapping, and design rules.

## Status

The headless simulation runs end to end: option parsing, startup,
fixed-timestep server ticks, headless client seats, and demo framing —
no window, no sockets, and no game data required. The app-level
simulation bootstrap is the final code merge in flight; windowed
presentation, native audio routing, and socket transports build on the
ported platform/network layers.

## Build, run, test

Requires Rust 1.98+ (workspace uses edition 2021, resolver 2).

```sh
cargo build                      # both binaries
cargo test --workspace           # full suite incl. headless E2E
cargo fmt --all -- --check       # formatting gate
cargo clippy --workspace --all-targets -- -D warnings   # lint gate
```

## Binaries

- `qa-muse` (`src/main.rs`) — main entry point. Parses the full donor
  option set, then runs the headless host loop.
- `qa-dedicated` (`src/bin/qa-dedicated.rs`) — dedicated-server entry
  point. Injects `--dedicated` when absent and refuses `--menu`.

```sh
cargo run --bin qa-muse -- --help
cargo run --bin qa-muse -- --version
cargo run --bin qa-muse -- --dedicated --movement q3 --frames 120
cargo run --bin qa-dedicated -- --movement q1 --frames 120
cargo run --bin qa-muse -- --list-content
```

Headless runs print a summary line, e.g.
`Ran 120 host frames, 120 server ticks, 5 entities (120 render frames)`.
Runs without `--frames` continue until quit (Ctrl-C).

Selected options (full list in `--help`): `--game`, `--map-game`,
`--map`, `--movement q1|q2|q3|qw|PRODUCT`, `--character`, `--model`,
`--renderer`, `--width/--height`, `--seats 1..4`, `--mode`, `--rules`,
`--skill`, `--bot-skill`, `--dedicated`,
`--listen/--listen-q2/--listen-unified`,
`--connect-q1/--connect-qw/--connect-q2/--connect-q3/--connect-unified`,
`--q1-protocol`, `--q2-protocol`, `--bind`, `--ipx-dosbox/--ipx-native`,
`--seed`, `--frames`, `--hidden`, `+command` startup lines, `--preset`.

## Crate overview

| Crate | Donor scope | Contents |
| --- | --- | --- |
| `qa-core` | `core`, contracts (math/numeric/time/identity/common) | fround-ordered `Vec3` math, numeric profiles, `Qrand`, source clocks, generational identity, command buffer, cvar registry |
| `qa-content` | `content`, `formats` | resource paths, VFS/mounts, Q1–Q3 BSP/MDL/MD2/SPR/WAD + MD3/MD4/MD5 decoders, image codecs, packed lighting + `.lit` overrides, Q1–Q3 game content |
| `qa-world` | `world`, `movement`, `persistence` (save kernel) | actor registry, bodies, spatial index, collision, q1–q3 movement, combat/inventory, headless `Simulation` + deterministic `Server` tick, saves |
| `qa-net` | `network` | bounded byte buffer, q1/q2/q3/quakeworld codecs, demo framing, protocol identities, transports, sessions, services |
| `qa-guest` | `guest`, `compat/qc,qvm`, `persistence` (execution) | entity fields, module registry, save/checkpoint records, x86/x64 VM, ELF/PE loaders, ABI runner |
| `qa-compat` | `compat/q2,q3` + shims | cross-family versions, demo kinds, userinfo, game adapters |
| `qa-client` | `render`, `materials`, `text`, `media`, `audio`, `input`, `ui`, `camera`, `capture` | prediction histories, view/HUD, seats/bindings, mixer channel pool, `RendererBackend` + CPU/GL renderers, spline cameras, screenshot/levelshot capture, shader/material data, text layout/fonts/localization, cinematic containers with CIN/RoQ/OGV decode |
| `qa-app` | `app`, `console`, `settings`, `debug`, `llm`, `main.ts`, `persistence` (providers) | CLI options, startup/config, host main loop, CLI dispatch, console core, seat/server settings, debug shapes, saved-game envelopes, per-family providers, unified save image |
| `qa-platform` | `platform` | native backends: windows, audio, controllers, fonts, codecs, sockets |
| `qa-bots` | `bots` | navigation (AAS reachability/routing, Kex NAV2/NAV3) and behavior |
| `qa-tools` | `tools/` | verification runner, inventories, reference captures, policy checks |

Dependency direction is acyclic: `app` drives `world` (server),
`client` (seats/render/audio), and `net`; `world` never depends on
`client` or `net`.

## Headless E2E

`crates/app/tests/headless_e2e.rs` exercises the whole stack without a
window or network: it parses CLI options, opens the server, spawns the
stub map (worldspawn + four player starts), runs 32 host frames with
deterministic synthetic client input for two seats (forward ramp,
strafe jitter, attack/jump cadence), then round-trips the per-frame
origins through both the Q2 (`.dm2`) and Q3 (`.dm3`) demo codecs. It
asserts tick counts, command-history depth, entity/origin sanity, Q2
record offsets and bytes, and Q3 sequences, payloads, and terminator.

## Porting notes

- Behavior fidelity over idiom: exact fround/operation order in math,
  per-family numeric profiles, sync port of the donor's async seams.
- No class hierarchies: traits + enums + generics. `Result<T, E>` with
  `thiserror` enums per crate — no null/undefined unions.
- Every ported file carries a donor provenance header naming the
  `quake-typescript` source it was ported from.
- Test footprint: 1300+ test files across the workspace, including
  byte-vector integration tests.

## License

`GPL-2.0-or-later` (declared in `Cargo.toml`).
