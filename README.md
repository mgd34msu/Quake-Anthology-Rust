# Quake Anthology Rust

A fresh native Rust engine built around common primitives for Quake, QuakeWorld,
Quake II, Quake III and mixed configurations.

Development covers shared input, movement, collision, networking, rendering and
module services. Full native-game and installed acceptance remains unqualified.
Behaviour comes from qsrc and features from quake-typescript.

Muse's retired tree remains available through the annotated `muse-final` tag.
The owner retired its branches and worktrees. Main continues through normal
descendant commits.

`cargo build --release` builds `target/release/qa-rust`. The portable default uses
Rust's baseline CPU. Machine-specific timing builds may use
`RUSTFLAGS="-C target-cpu=native"`; record that choice with the measurements.
`python3 tools/build.py` checks the source and splits debug symbols from the
installation candidate.


`--max-clients N` sizes the shared client array at startup (default 64), including
humans, remote clients and bots. Native protocols retain their own client limits.

To record input from launch, add `--record-journal walk.qaj` to your normal
command. To play it, use the same content, modules and saved settings with
`--play-journal walk.qaj`. Recordings preserve keyboard, text, mouse, controller,
console, packet and time events. Existing recording files are preserved.

During playback, recorded controls drive the game and external network output
is simulated. Closing the window stops playback; reaching the end also quits.
The journal currently requires the same initial session and asset/configuration
files. It does not embed those files or support seeking yet.
