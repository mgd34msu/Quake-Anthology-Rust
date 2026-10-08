//! Client composition uses the same scene packets for either renderer.
use qa_platform::{Stopwatch, Window};
use qa_render::{Assets, BackendStats, FrontEnd, Limits, cpu::CpuBackend, gl::GlBackend};

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
}
enum Backend {
    Cpu(CpuBackend),
    Gl(GlBackend),
}
pub struct Renderer {
    frontend: FrontEnd,
    assets: Assets,
    backend: Backend,
    pub sample: Sample,
    width: u32,
    height: u32,
}
impl Renderer {
    /// Assets are frozen while a backend and its packets reference them.
    pub fn load(kind: Kind, window: &Window, width: u32, height: u32) -> Result<Self, String> {
        let assets = Assets::load();
        let limits = Limits::default();
        let frontend = FrontEnd::load(limits)?;
        let backend = match kind {
            Kind::Cpu => Backend::Cpu(CpuBackend::load(width, height)?),
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
            sample: Sample::default(),
            width,
            height,
        })
    }
    pub fn frame(&mut self) {
        let timer = Stopwatch::start();
        // THE-862 supplies world visibility/materials; this remains a window
        // shell until a loaded client scene is submitted at the shared API.
        let Some(frame) = self.frontend.begin_frame([18, 26, 34, 255]) else {
            self.sample.stats.rejected += 1;
            return;
        };
        let packet = frame.finish();
        self.sample.frontend_ns = timer.elapsed().as_nanos() as u64;
        let timer = Stopwatch::start();
        self.sample.stats = match &mut self.backend {
            Backend::Cpu(cpu) => cpu.render(&packet, &self.assets),
            Backend::Gl(gl) => gl.render(&packet, &self.assets),
        };
        self.sample.backend_ns = timer.elapsed().as_nanos() as u64;
        let _ = self.frontend.recycle(packet);
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
