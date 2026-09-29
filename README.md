# qa-muse

A Rust port of [quake-typescript](https://github.com/)'s Quake anthology engine: one
workspace, library crates, and two binaries covering the entire donor
tree — server, client, renderer, audio, input, UI, platform backends,
network transports, bots, tools, and game content. It currently runs the
headless simulation — option parsing, startup, fixed-timestep server
ticks, headless client seats, demo framing — with no window, no sockets,
and no game data required; windowed/GL rendering, native audio/input,
and socket transports land with the platform/net lanes now in flight.

Port source (hard boundary): `/home/buzzkill/Projects/quake-typescript`,
scoped to `src/`, `tests/`, `tools/`, `docs/`, `verification/`. Sibling
native trees are out of scope; all behavior questions resolve against the
TypeScript donor bodies. See [docs/PORT_PLAN.md](docs/PORT_PLAN.md) for the
crate layout, donor-to-crate mapping, and phased build order.

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
cargo run --bin qa-muse -- --list-content --content-root ~/Projects/qfiles
```

Headless runs print a summary line, e.g.
`Ran 120 host frames, 120 server ticks, 5 entities (120 render frames)`.
Runs without `--frames` continue until quit (Ctrl-C). Current
behavior while the remaining lanes land: `Menu` selections run the
headless loop (full menu behavior: UI/bootstrap lanes),
`weapon-behavior` actions other than `--help` report "not ported yet"
(tools lane), and `--list-content` reports raw corpus-root directory
names (content catalog lane).

Selected options (full list in `--help`): `--game`, `--map-game`, `--map`,
`--movement q1|q2|q3|qw|PRODUCT`, `--character`, `--model`, `--renderer`,
`--width/--height`, `--seats 1..4`, `--mode`, `--rules`, `--skill`,
`--bot-skill`, `--dedicated`, `--listen/--listen-q2/--listen-unified`,
`--connect-q1/--connect-qw/--connect-q2/--connect-q3/--connect-unified`,
`--q1-protocol`, `--q2-protocol`, `--bind`, `--ipx-dosbox/--ipx-native`,
`--seed`, `--frames`, `--hidden`, `+command` startup lines, `--preset`.

## Crate overview

| Crate | Donor scope | Contents |
| --- | --- | --- |
| `qa-core` | `core`, contracts (math/numeric/time/identity/common) | fround-ordered `Vec3` math, numeric profiles, `Qrand`, source clocks, generational identity, command buffer, cvar registry |
| `qa-content` | `content`, `formats` | resource paths, VFS/mounts, Q1–Q3 BSP/MDL/MD2/SPR/WAD + MD3/MD4/MD5 decoders, image codecs (`images/`: BMP/GIF/indexed/JPEG/MIP/palette/PNG/Q3/TGA/WAD + QLIT), Q1 packed lighting + `.lit` overrides; Q1-Quake64 geometry and game content land with the formats/content lanes |
| `qa-world` | `world`, `movement`, `persistence` (save kernel) | actor registry, bodies, spatial index, collision, q1–q3 movement, combat/inventory, headless `Simulation` + deterministic `Server` tick, saves; lossless JSON save codec (`$qts` tags, bigint, bytes, canonical base64), records/ownership/protection, world-state snapshot/restore |
| `qa-net` | `network` | bounded byte buffer, q1/q2/q3/quakeworld codecs, demo framing, protocol identities |
| `qa-guest` | `guest`, `compat/qc,qvm`, `persistence` (execution) | entity fields, module registry, save/checkpoint records; x86/x64 VM, ELF/PE loaders, ABI runner landing in the guest-vm lane |
| `qa-compat` | `compat/q2,q3` + shims | cross-family versions, demo kinds, userinfo, game adapters |
| `qa-client` | `render`, `materials`, `text`, `media`, `audio`, `input`, `ui`, `platform`, `camera`, `capture` | headless client core: prediction histories, view/HUD, seats/bindings, mixer channel pool, `RendererBackend` + `NullRenderer`, spline cameras (`.camera` parse/playback/view override), screenshot/levelshot capture (real TGA/PNG/JPEG encoders over `qa-content`, injectable for tests), shader/material data levels (`materials/`), text layout/fonts/localization/captions (`text/`, incl. TrueType rasterization and atlas builds), cinematic containers/timelines/presentation with full CIN/RoQ/OGV pixel+audio decode (`media/`) |
| `qa-app` | `app`, `console`, `settings`, `debug`, `llm`, `main.ts`, `persistence` (providers) | CLI options, startup/config, host main loop, CLI dispatch, console core (scrollback, edit fields, dispatch + builtins, log, session, metrics, discovery, dedicated stdin), seat/server settings + restart flow, debug-line shapes/store; saved-game read/write (Q1/Q2-classic/TS/Rerelease/Q3 envelopes), per-family providers/recipes, save policy, unified save image (`QTSAVE3`/`QTSAVE2`) |

Dependency direction is acyclic: `app` drives `world` (server),
`client` (headless seats/render/audio), and `net` (demos); `world` never
depends on `client` or `net`.

## Headless E2E

`crates/app/tests/headless_e2e.rs` exercises the whole stack without a
window or network: it parses CLI options, opens the server, spawns the
stub map (worldspawn + four player starts), runs 32 host frames with
deterministic synthetic client input for two seats (forward ramp, strafe
jitter, attack/jump cadence), then round-trips the per-frame origins
through both the Q2 (`.dm2`) and Q3 (`.dm3`) demo codecs. It asserts tick
counts (32 host frames → 32 Q3 server ticks), command-history depth,
entity/origin sanity (finite, inside world bounds), Q2 record offsets and
bytes, and Q3 sequences, payloads, and terminator.

## Status and limits

Project rule: if it is in the `quake-typescript` project, it gets
written in Rust here. The only exception is TypeScript-specific
machinery. No deferrals, no stubs left for later, no exceptions without
explicit user authorization. Ported already: protocol codecs, BSP/model/sprite/WAD
readers, server tick and game rules, console core, settings, debug,
camera/capture, persistence providers and save envelopes, materials,
text/media data levels, media codec engines (CIN/RoQ/OGV),
TrueType rasterization, and image formats. Still to port: `llm/` +
console draw/llm commands, `platform/` native backends, network
transports/sessions/services, the guest VM (x86/x64, ELF/PE, ABI),
compat bridges (QC/QVM/native), game content (`content/`), the renderer
(scene/CPU/GL), remaining audio/input/movement, UI, `app/bootstrap`,
`tools/`, and bots (navigation + behavior). `NullLogic`/`NullRenderer`
stand only until their owning port task lands.
