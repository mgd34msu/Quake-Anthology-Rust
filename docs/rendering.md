# Shared scene rendering

THE-861 introduces the scene API and GL/CPU command consumers. This is the
entity, polygon and 2D foundation. Retail world visibility, native CPU surface
spans, palette lighting and complete material conversion follow in THE-862;
the three-game walk-through is THE-839. A window or fixture is not gameplay.

Assets register at load and return numeric image, material and model handles.
Backend registration freezes those assets for its lifetime. Scene clients use
`FrontEnd::begin_frame`, `clear_scene`, `add_entity`, `add_poly`, `add_light`,
`render_scene` and `draw_2d`. `render_scene` copies the view and its hidden-area
bits and advances the scene start ranges. Clearing a scene does not overwrite
previous views. Area bits mean hidden; Q2 visible-area bits convert at its ABI.

Two lists own fixed command, entity, polygon, vertex, light and area arenas.
Finishing a frame transfers its packet by ownership. The consumer borrows it
and returns it through `recycle`; an outstanding packet cannot be reused.
Capacity failures reject an entire affected submission, count it and retain
prior commands. Buffers never grow during a frame. This ownership also permits
a future platform render worker; the current consumers run synchronously.

Both consumers use the same camera convention: forward, left and up axes, with
screen right opposite the left axis. Static entity meshes, dynamic polygon fans
and ordered 2D draws share materials. Stages select texture or lightmap
coordinates, vertex color, blend, alpha test, depth test and depth writes.
An alpha-tested base writes depth only for surviving texels; a following
equal-depth lightmap pass leaves its holes untouched.

GL uses a core 4.4 context created and synchronized by SDL3 in platform. Render
resolves procedures once, uploads static meshes/images at load and uses a
bounded coherent persistent buffer with fences for dynamic vertices. State is
cached, and no `glFinish` is used. SDL swaps the owned context. GL resources
must be destroyed while that context is current, before its window is dropped.

CPU uses a load-sized RGBA framebuffer and inverse-depth buffer for the common
entity/polygon raster. Triangles clip against six camera planes in bounded
stack scratch; texture coordinates interpolate with perspective correction.
SDL presents the exact-size framebuffer through a fixed streaming texture.
This triangle raster does not replace THE-862's native world edge/span cache.

View blend phase is explicit. GLQuake blends after the world and before 2D;
Q1 software shifts its final palette after the HUD, console and menu. Final
blend bounds can include the seat's status bar outside the camera viewport.
The current final phase is an RGBA approximation. Native indexed palette
presentation remains required before stock Q1/Q2 image qualification.
Scene light submissions are retained and reported as pending; this foundation
does not yet implement dynamic world lighting or animated model evaluation.

`qa-rust --renderer cpu` and `--renderer gl` select consumers of the same scene
interface. The unloaded app still reports `gameplay:false` and timing scope
`window_shell`. Frontend, backend and presentation counters are separate;
CPU submission time is not GPU execution time. No installation is qualified
by an empty scene or the developer rendering fixture.

Native contracts: Q3 `renderer/tr_public.h`, `tr_scene.c`, `tr_cmds.c`,
`tr_backend.c` and `cgame/tr_types.h`; Q1 `WinQuake/view.c`, `screen.c`,
`gl_rmain.c` and `gl_screen.c`. The C port's retained world topology and cache
algorithms inform THE-862; its command reallocations and borrowed payload
pointers are not used.

THE-862 adds load-time primitives for the retail world path: one normalized
BSP visibility service, retained polygon/triangle boundaries, adaptive Q3
patch grids, one RGB lightmap atlas and typed Q3 material definitions loaded
from the winning VFS scripts. Q1/Q2 signed edges, native texture extents and
full light-style sample spans remain available to the CPU cache. Q3 lightmap
pages are stored once; source surface ids stay unchanged across conversion.
Cross-patch stitching and view-dependent patch LOD remain pending.

The native CPU primitives use fixed GET/AET arrays, a depth-ordered surface
stack and flushing span arena. A bounded rover cache owns original indexed
mips and native palette/colormap resources. Cache stamps include resource,
style and dynamic-light generations; batch pins protect pending spans from
eviction. Every opaque texel uses the supplied colormap, including fullbright
indices. Q2 RGB reduction follows the owner's brightest-channel rule; this
differs from original strict comparisons when red and green tie above blue.

`Frame::add_world` copies the frontend's visible surface ids and BSP depth keys
into its owned packet. GL can consume the world's packed static triangle
ranges. The native CPU span/cache primitives still require backend integration;
CPU counts world submissions as rejected until that path is attached. The app
still has no loaded map. Shader parsing retains animation, texture modifiers,
waves and deformations, but parsing alone does not implement those effects.
Native images, complete material conversion and the walk-through gate remain
unqualified.

CPU raster bands are automatic at renderer load. Platform counts physical cores
inside the process affinity, and the CPU renderer selects 1/2/4/8 bands while
respecting framebuffer rows and the mandatory surface-cache reservation. The
map-sized cache budget is shared across bands, with a 32 MiB floor and enough
room for each band's largest mandatory surface. The total sums registered mip
reservations, including optional recipes; it does not guarantee that duplicate
band copies all remain resident. `CpuLimits.cache_bytes = 0` selects this load
policy; a nonzero value selects a fixed diagnostic budget. Cache reports
distinguish absent-slot fills, changed-state refills and payload residency.
`r_cpuBands` defaults to `0` (auto)
in every native cvar view; `1`, `2`, `4` and `8` are archived, latched overrides.
A running CPU renderer holds writes until a renderer load boundary. The
`--cpu-bands` option takes precedence for benchmark runs.

All bands use `Workers::dispatch_scoped`: the caller and background workers
claim exclusive job indices, and consumers merge completed slots in index
order. One band has zero background workers and uses the same implementation.
Allocation qualification combines the calling thread with every worker after
each dispatch, including rejected batches, and consumes the aggregate once per
ordinary or quit frame. Native SDL/driver allocations are outside this counter.

For fixed-scene CPU timing, build the developer example and use the private tool:

```sh
cargo build --release -p qa-platform --example cpu_retail --example job_dispatch \
  -p qa-app --bin qa-rust --features qa-platform/allocation-tracking,qa-app/allocation-tracking
python3 tools/frame_timings.py --binary target/release/qa-rust \
  --cpu-retail-binary target/release/examples/cpu_retail \
  --owner-profile "$PROFILE" --content "$CONTENT" --map q3dm1 \
  --cpu-bands auto --cores "$CORES" --evidence "$EVIDENCE"
```

Use a fresh evidence directory and the same multicore affinity for auto and
fixed `1`/`2`/`4`/`8` rows. Each row records 600 draws after 60 warm-up frames,
raw RGBA/depth files, allocation counts and median/p99. The fixed scene excludes
host simulation, presentation and native heap work; these rows cannot qualify
an installation. Omitting `--cpu-retail-binary` measures host frames; supplying
`--content` and `--map` loads retail geometry. OpenGL rows retain driver identity
and explicitly label Mesa llvmpipe/softpipe software rendering.
