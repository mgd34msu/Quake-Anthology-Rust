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

Run development binaries only with `tools/private_run.py`, which unsets
`WAYLAND_DISPLAY`, forces X11 and captures audio privately. The current loop is
a window shell. Its counts and the headless probe do not establish zero
allocations on e1m1; THE-786 remains In Progress until a gameplay run records
zero after warm-up. No qualified installation is claimed.
