# Frame scratch storage and allocation counts

`qa_core::arena::FrameArena` allocates one fixed backing buffer at load time.
It initializes typed `Copy` values, including types with alignment larger than
64 bytes, and resets by advancing a generation and clearing the used offset.
Typed blocks from another arena or an earlier generation cannot be read. Slice
borrows prevent concurrent allocation, reset or destruction. Exhaustion returns
an error without growing storage or invalidating existing blocks; the affected
consumer skips its work. Zero-sized element types are rejected.

The reset/reuse and address-alignment approach follows C `src/core/arena.c`
and original Quake `WinQuake/zone.c` hunk marks. The frame allocator reserves
capacity at load time instead of adding blocks during play. No retired Muse
allocator or per-game allocation structure was copied.

Each service owns its reusable scratch vectors and clears them between frames;
temporary typed arrays can use the arena. The headless probe exercises both
patterns over 10,000 iterations with a positive allocation/reallocation control.

```sh
cargo test -p qa-core --test arena
cargo run --release -p qa-platform --example frame_allocations \
  --features allocation-tracking
```

Development app builds use the platform's counting `System` allocator. With
`developer 1`, the shared diagnostics logger reports allocations, reallocations
and requested bytes for the calling Rust frame thread. Counts include input
diagnostics and exclude the counter's own reporting after the frame. Native SDL
allocations and other threads are outside this counter's scope. Debug builds
enable it automatically; optimized development builds use
`--features qa-app/allocation-tracking`. Normal release builds omit it.

THE-892 makes allocation-tracking qualification independent of `developer`.
After warm-up, any allocation or reallocation, including in a quit frame,
rejects the run after the window is destroyed. The final `allocation_gate`
record contains measured/failed frame counts and total requested bytes. A run
with no measured frames also fails. The development `proof` feature enables
allocation tracking automatically. Warm-up allocations are reported when
diagnostics are enabled but do not fail qualification.

THE-887 removes the owned command text and cvar-write allocations. Console
commands are now a zero-allocation workload, not an allocating control.
The platform host probe verifies its allocator first with an intentional
allocation/reallocation outside measurement, then counts real console frames.
Normal gameplay builds retain scoped gameplay errors; this failure belongs to
development qualification and occurs after cleanup.

Run development binaries only with `tools/private_run.py`, which unsets
`WAYLAND_DISPLAY`, forces X11 and captures audio privately. The current loop is
a window shell. Its counts and the headless probe do not establish zero
allocations on e1m1; THE-786 remains In Progress until a gameplay run records
zero after warm-up. No qualified installation is claimed.


THE-888 moves fixed engine text storage to core so input and console share it.
Bind slots, compiled clause spans and release-command scratch are reserved at
load; changing a config bind, dispatching a known button, expanding a +/- alias
or queuing a complete console input line does not grow heap storage. The pinned
host probe's `--binds` mode checks those paths and both local seats over the same
60 warm-up/600 measured-frame schedule. See `frame-times.md` for measured scope.

THE-889 reserves each client's bot intent in the existing 64-slot session
table. Its stateless command conversion and movement-duration selection use
no scratch allocation. The host probe's `--bots` mode connects all slots at
load and checks fixed intents, native duration policies and server-time output
after every frame. This measures command construction, not bot navigation/AI.
