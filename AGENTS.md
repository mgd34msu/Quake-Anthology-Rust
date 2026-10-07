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
Then R1 primitives/VFS/formats, R2 console/cvars/input/frame loop, R3 renderers,
R4 Q1 e1m1 and a mixed configuration, R5 Q1 complete, R6 Q2, R7 Q3/TA and bots,
R8 mods together, R9 combined mode, R10 audio/menus/UI, R11 network,
R12 measured performance, R13 release.
