# Quake Anthology

One engine for the whole Quake family: Quake, Quake II, Quake III
Arena, QuakeWorld, the mission packs and rereleases, and mods — local
play with up to 4 splitscreen seats, bots, dedicated servers, and
native-protocol online play against original servers.

This is the Rust port of
[Quake-Anthology-TS](https://github.com/mgd34msu/Quake-Anthology-TS).

## Quick start

Requires Rust 1.98 or newer.

```sh
cargo build --release
./target/release/quake-anthology
```

With no arguments it opens the startup menu in a window and runs
until quit (it needs a display). `--help` lists every option;
`--version` prints the release (`Quake Anthology 0.1.0`).

## Game data

The engine needs your game files. The content root defaults to the
folder beside the executable (override with `--content-root PATH`).
Copy the `.pak`/game files from your Quake installs there, then check
what the engine sees:

```sh
./target/release/quake-anthology --list-content
```

It lists every known product (`q1-classic-id1`, `q2-classic-baseq2`,
…) and tells you exactly which files are still missing for each one.
Pick what to run with `--game PRODUCT`, `--map NAME`,
`--map-game PRODUCT`, `--mod ID` (repeatable), and `--progs PATH` for
QuakeC game logic.

## Playing

```sh
# Launch a game and walk its maps (one local seat)
./target/release/quake-anthology --game q1-classic-id1
./target/release/quake-anthology --game q2-classic-baseq2 --skill 2

# Software rendering instead of GL, 20 frames, then quit
./target/release/quake-anthology --renderer cpu --width 1280 --height 720 --gamma 1.2 --frames 20
```

Movement follows the game you pick (`--movement q1|q2|q3|qw`,
`--character`, `--model`); `+command` arguments run startup console
commands (`'+bind x "+attack"'` as one shell argument). Campaigns with
monsters, items, and doors, bots, splitscreen seats, and mixed-game
presets are under construction (see Status). `--frames N` opens the
game, runs N frames, and quits — useful for smoke-testing a setup:

```sh
# Smoke test: opens a window, runs 600 frames, prints
# `Ran 600 windowed frames` and exits 0 (needs a display; use
# xvfb-run on a headless machine)
xvfb-run -a ./target/release/quake-anthology --windowed --frames 600
```

Without `--frames` (and without `--dedicated`), `quake-anthology` opens a
window and runs until quit; `--dedicated` runs the headless server
until stopped (add `--frames N` to stop it after N steps).

## Hosting and joining

```sh
# Dedicated server loop (no window, no local seats)
./target/release/qa-dedicated --game q1-classic-id1 --listen 26000
```

The dedicated server currently runs a placeholder scene while the
gameplay verticals land; native-protocol hosting and joining
(`--listen-q2`, `--connect-*`) and mixed-game hosting are under
construction (see Status) and not yet wired to the live server.

Server knobs: `--bind ADDRESS`, `--mode`, `--rules
standard|ctf|lmctf|tag|deathball|horde`, `--seed N`,
`--server-profile PATH`, `--q1-protocol`, `--q2-protocol`, and
`--ipx-dosbox`/`--ipx-native` for IPX play.

## Status

Pre-release (`0.1.0`), under active development. Verified working:
startup menu (opens in a window, runs until quit), game launch with
installed content and player movement, menu navigation, one local
seat, the dedicated server loop, and the smoke runs (`--dedicated …
--frames N` prints a per-run summary; `--windowed --frames 600`
prints `Ran 600 windowed frames` and exits 0).

Under construction, in order: playable Quake 1 (monsters, weapons,
items, doors, HUD), then Quake 2, then Quake 3; after that bots,
splitscreen seats, native-protocol online play, and mods. Until a
feature is listed here as working, its flags parse but do nothing.

## Development

One Cargo workspace, eleven library crates plus the two binaries:

- `qa-core` — math, numeric profiles, RNG, clocks, identity, cmd/cvar
- `qa-content` — VFS/mounts, catalog, map/model/image format decoders
- `qa-world` — actors, collision, movement, combat, headless simulation
- `qa-net` — q1/q2/q3/quakeworld codecs, transports, sessions
- `qa-guest` — game-module VM (x86/x64), ELF/PE loaders, QC/QVM
- `qa-compat` — cross-family shims (versions, demos, userinfo)
- `qa-client` — renderers, audio mixer, input, UI, media decode
- `qa-app` — options, startup, host loop, console, settings, saves
- `qa-platform` — native windows, audio, controllers, sockets
- `qa-bots` — navigation and behavior
- `qa-tools` — verification and inventory tooling

```sh
# full suite incl. headless end-to-end (always under xvfb, never bare)
WAYLAND_DISPLAY= SDL_VIDEODRIVER=x11 SDL_AUDIODRIVER=dummy xvfb-run -a cargo test --workspace -q
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

## License

`GPL-2.0-or-later` (declared in `Cargo.toml`).
