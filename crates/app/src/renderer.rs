//! Client composition uses the same scene packets for either renderer.
use crate::host::ClientView;
use qa_core::{math::angle_vectors, sys_events::SeatId};
use qa_platform::{Stopwatch, Window};
use qa_render::{
    Assets, BackendStats, BlendPhase, FrontEnd, Limits, Refdef, Viewport,
    cpu::CpuBackend,
    gl::GlBackend,
    material::world_load::{LoadedWorld, PresentationDefaults},
    world::WorldView,
};

#[derive(Clone, Copy, Debug, Default)]
pub enum Kind {
    #[default]
    Cpu,
    Gl,
}
impl Kind {
    pub fn parse(value: &str) -> Result<Self, &'static str> {
        match value {
            "cpu" => Ok(Self::Cpu),
            "gl" => Ok(Self::Gl),
            _ => Err("renderer must be cpu or gl"),
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Gl => "gl",
        }
    }
    pub fn open(self, width: i32, height: i32) -> Result<Window, String> {
        match self {
            Self::Cpu => Window::open(width, height),
            Self::Gl => Window::open_gl(width, height),
        }
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Sample {
    pub frontend_ns: u64,
    pub backend_ns: u64,
    pub present_ns: u64,
    pub stats: BackendStats,
    pub presented: bool,
    pub visible_surfaces: u32,
}
enum Backend {
    Cpu(CpuBackend),
    Gl(GlBackend),
}
struct WorldScene {
    presentation: PresentationDefaults,
    views: [WorldView; SeatId::COUNT],
}
pub struct Renderer {
    frontend: FrontEnd,
    assets: Assets,
    backend: Backend,
    scenes: Box<[Option<WorldScene>]>,
    kind: Kind,
    pub sample: Sample,
    width: u32,
    height: u32,
}
impl Renderer {
    /// Assets are frozen while a backend and its packets reference them.
    pub fn load(
        kind: Kind,
        window: &Window,
        width: u32,
        height: u32,
        assets: Assets,
        worlds: Vec<LoadedWorld>,
    ) -> Result<Self, String> {
        let mut scenes: Vec<Option<WorldScene>> =
            (0..assets.worlds().len()).map(|_| None).collect();
        let mut max_surfaces = 0usize;
        for loaded in worlds {
            let world = assets.world(loaded.world).ok_or("missing loaded world")?;
            let index = loaded.world.0 as usize;
            if scenes[index].is_some() {
                return Err("duplicate world presentation".into());
            }
            for diagnostic in &loaded.diagnostics {
                qa_console::logger::console(format_args!(
                    "world {}: {diagnostic}\n",
                    loaded.world.0
                ));
            }
            max_surfaces = max_surfaces.max(world.visibility().surface_count());
            scenes[index] = Some(WorldScene {
                presentation: loaded.presentation,
                views: std::array::from_fn(|_| WorldView::load(world)),
            });
        }
        if scenes.iter().any(Option::is_none) {
            return Err("missing world presentation defaults".into());
        }
        let mut limits = Limits::default();
        limits.surfaces = limits.surfaces.max(
            max_surfaces
                .checked_mul(SeatId::COUNT)
                .ok_or("scene surface capacity")?,
        );
        let frontend = FrontEnd::load(limits)?;
        let backend = match kind {
            Kind::Cpu => Backend::Cpu(CpuBackend::load_with_assets(width, height, &assets)?),
            Kind::Gl => Backend::Gl(unsafe {
                GlBackend::load(
                    |name| window.gl_proc(name),
                    &assets,
                    width,
                    height,
                    limits.vertices,
                )?
            }),
        };
        Ok(Self {
            frontend,
            assets,
            backend,
            scenes: scenes.into_boxed_slice(),
            kind,
            sample: Sample::default(),
            width,
            height,
        })
    }
    pub fn frame(&mut self, views: &[Option<ClientView>; SeatId::COUNT]) {
        self.sample = Sample::default();
        let timer = Stopwatch::start();
        let Some(mut frame) = self.frontend.begin_frame([18, 26, 34, 255]) else {
            self.sample.stats.rejected += 1;
            return;
        };
        let count = views.iter().filter(|view| view.is_some()).count();
        let mut view_index = 0;
        let mut rejected = 0;
        for (seat, client) in views.iter().enumerate() {
            let Some(client) = client else {
                continue;
            };
            let viewport = viewport(self.width, self.height, view_index, count);
            view_index += 1;
            let Some(scene) = self
                .scenes
                .get_mut(client.world.0 as usize)
                .and_then(Option::as_mut)
            else {
                rejected += 1;
                continue;
            };
            let Some(world) = self.assets.world(client.world) else {
                rejected += 1;
                continue;
            };
            let basis = angle_vectors(client.angles);
            let fov_x = if client.fov_x.is_finite() {
                client.fov_x.clamp(1.0, 179.0)
            } else {
                90.0
            };
            let aspect = viewport.height as f32 / viewport.width.max(1) as f32;
            let fov_y = (2.0 * ((fov_x.to_radians() * 0.5).tan() * aspect).atan()).to_degrees();
            let mut refdef = Refdef {
                viewport,
                origin: client.origin,
                axes: [basis.forward, -basis.right, basis.up],
                fov: [fov_x, fov_y],
                time_ms: client.time_ms,
                ..Refdef::default()
            };
            scene.presentation.apply(&mut refdef);
            if matches!(self.kind, Kind::Gl) {
                refdef.blend_phase = BlendPhase::AfterView;
                refdef.palette_transform = None;
            }
            let Ok(surfaces) = scene.views[seat].query(world, refdef, None, &[]) else {
                rejected += 1;
                continue;
            };
            self.sample.visible_surfaces += surfaces.len() as u32;
            frame.clear_scene();
            if !frame.add_world(client.world, surfaces)
                || !frame.render_scene(refdef, &[], &self.assets)
            {
                rejected += 1;
            }
        }
        let packet = frame.finish();
        self.sample.frontend_ns = timer.elapsed().as_nanos() as u64;
        let timer = Stopwatch::start();
        self.sample.stats = match &mut self.backend {
            Backend::Cpu(cpu) => cpu.render(&packet, &self.assets),
            Backend::Gl(gl) => gl.render(&packet, &self.assets),
        };
        self.sample.backend_ns = timer.elapsed().as_nanos() as u64;
        self.sample.stats.rejected += rejected;
        if self.frontend.recycle(packet).is_err() {
            self.sample.stats.rejected += 1;
        }
    }
    pub fn present(&mut self, window: &mut Window) {
        let timer = Stopwatch::start();
        self.sample.presented = match &self.backend {
            Backend::Cpu(cpu) => window.present_pixels(self.width, self.height, cpu.pixels()),
            Backend::Gl(_) => window.present_gl(),
        };
        self.sample.present_ns = timer.elapsed().as_nanos() as u64;
    }
}

fn viewport(width: u32, height: u32, view: usize, count: usize) -> Viewport {
    let columns = if count > 1 { 2 } else { 1 };
    let rows = if count > 2 { 2 } else { 1 };
    let x = view as u32 % columns;
    let y = view as u32 / columns;
    let left = width * x / columns;
    let top = height * y / rows;
    Viewport {
        x: left,
        y: top,
        width: width * (x + 1) / columns - left,
        height: height * (y + 1) / rows - top,
    }
}
