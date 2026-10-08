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
