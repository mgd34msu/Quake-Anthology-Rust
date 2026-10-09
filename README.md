# Quake Anthology Rust

A fresh native Rust engine built around common primitives for Quake, QuakeWorld,
Quake II, Quake III and mixed configurations.

R0 currently defines the workspace and shared values. Gameplay is not implemented.
Behaviour comes from qsrc, features from quake-typescript, and proven algorithms
and fixes from the C port. Neither port's incompatible structure is retained.

Muse's retired tree remains available through the annotated `muse-final` tag.
The owner retired its branches and worktrees. Main continues through normal
descendant commits.

`cargo build --release` builds `target/release/qa-rust`. The portable default uses
Rust's baseline CPU. Machine-specific timing builds may use
`RUSTFLAGS="-C target-cpu=native"`; record that choice with the measurements.
Debug symbols are split from the installation candidate by the build tool in R0.


`--max-clients N` sizes the shared client array at startup (default 64), including
humans, remote clients and bots. Native protocols retain their own client limits.
