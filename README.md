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
./target/release/qa-muse
```

With no arguments it opens the startup menu. `--help` lists every
option; `--version` prints the release (`Quake Anthology 0.1.0`).

## Game data

The engine needs your game files. The content root defaults to the
folder beside the executable (override with `--content-root PATH`).
Copy the `.pak`/game files from your Quake installs there, then check
what the engine sees:

```sh
./target/release/qa-muse --list-content
```

It lists every known product (`q1-classic-id1`, `q2-classic-baseq2`,
…) and tells you exactly which files are still missing for each one.
Pick what to run with `--game PRODUCT`, `--map NAME`,
`--map-game PRODUCT`, `--mod ID` (repeatable), and `--progs PATH` for
QuakeC game logic.

## Playing

```sh
# Quake II campaign, hard difficulty
./target/release/qa-muse --game q2-classic-baseq2 --skill 2

# Quake III with bots, nightmare bot skill
./target/release/qa-muse --game q3-base --bot-skill 5

# Two-player local splitscreen deathmatch
./target/release/qa-muse --game q1-classic-id1 --seats 2 --mode deathmatch --rules standard

# Software rendering instead of GL, custom window size
./target/release/qa-muse --renderer cpu --width 1280 --height 720 --gamma 1.2

# Mixed-game recipe
./target/release/qa-muse --preset q2-q1-q3
```

Movement, characters, and models follow the game you pick
(`--movement q1|q2|q3|qw`, `--character`, `--model`); `+command`
arguments run startup console commands (`'+bind x "+attack"'` as one
shell argument). `--frames N` runs N simulation steps and quits —
useful for smoke-testing a setup without a window.

## Hosting and joining

```sh
# Dedicated server (no window, no local seats)
./target/release/qa-dedicated --game q1-classic-id1 --listen 26000

# Host the selected game's native protocol / join servers
./target/release/qa-muse --game q2-classic-baseq2 --listen-q2 27910
./target/release/qa-muse --connect-q1 play.example.com
./target/release/qa-muse --connect-q2 play.example.com
./target/release/qa-muse --connect-q3 play.example.com:27960
./target/release/qa-muse --connect-qw play.example.com:27500

# Mixed-game hosting
./target/release/qa-muse --listen-unified 27960
./target/release/qa-muse --connect-unified play.example.com:27960
```

Server knobs: `--bind ADDRESS`, `--mode`, `--rules
standard|ctf|lmctf|tag|deathball|horde`, `--seed N`,
`--server-profile PATH`, `--q1-protocol`, `--q2-protocol`, and
`--ipx-dosbox`/`--ipx-native` for IPX play.

## Status

Pre-release (`0.1.0`), under active development. Verified working:
startup menu, game launch with installed content, local seats,
dedicated servers, native-protocol clients, and the headless
simulation (`--dedicated … --frames N` prints a per-run summary such
as `Ran 20 host frames, 20 server ticks, 5 entities`).

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
cargo test --workspace           # full suite incl. headless end-to-end
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

## License

`GPL-2.0-or-later` (declared in `Cargo.toml`).
