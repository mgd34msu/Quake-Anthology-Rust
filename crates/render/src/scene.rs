//! Q3-style append-only scene ranges in alternating owned packets.
use crate::assets::{MaterialId, ModelId, Vertex};
use qa_core::primitives::Vec3;
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT_FRONTEND: AtomicU64 = AtomicU64::new(1);
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Viewport {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BlendPhase {
    #[default]
    AfterView,
    FinalPalette,
}
#[derive(Clone, Copy, Debug)]
pub struct Refdef {
    pub viewport: Viewport,
    pub origin: Vec3,
    /// Forward, left, up: the native Q3 view axis convention.
    pub axes: [Vec3; 3],
    pub fov: [f32; 2],
    pub near: f32,
    pub far: f32,
    pub time_ms: u64,
    pub blend: [f32; 4],
    pub blend_phase: BlendPhase,
    /// Final palette shifts also cover a seat's statusbar/console outside the
    /// camera viewport. None uses the camera viewport for standalone scenes.
    pub blend_viewport: Option<Viewport>,
}
impl Default for Refdef {
    fn default() -> Self {
        Self {
            viewport: Viewport::default(),
            origin: Vec3::default(),
            axes: [
                Vec3([1.0, 0.0, 0.0]),
                Vec3([0.0, 1.0, 0.0]),
                Vec3([0.0, 0.0, 1.0]),
            ],
            fov: [90.0, 75.0],
            near: 4.0,
            far: 65536.0,
            time_ms: 0,
            blend: [0.0; 4],
            blend_phase: BlendPhase::AfterView,
            blend_viewport: None,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct SceneEntity {
    pub model: ModelId,
    pub origin: Vec3,
    pub axes: [Vec3; 3],
    pub color: [u8; 4],
    pub material: Option<MaterialId>,
    pub frame: u32,
    pub old_frame: u32,
    pub back_lerp: f32,
    pub depth_hack: bool,
}
impl Default for SceneEntity {
    fn default() -> Self {
        Self {
            model: ModelId(0),
            origin: Vec3::default(),
            axes: [
                Vec3([1.0, 0.0, 0.0]),
                Vec3([0.0, 1.0, 0.0]),
                Vec3([0.0, 0.0, 1.0]),
            ],
            color: [255; 4],
            material: None,
            frame: 0,
            old_frame: 0,
            back_lerp: 0.0,
            depth_hack: false,
        }
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Light {
    pub origin: Vec3,
    pub radius: f32,
    pub color: [f32; 3],
    pub additive: bool,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Poly {
    pub material: MaterialId,
    pub vertices: Span,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Draw2d {
    pub rect: [f32; 4],
    pub texcoords: [f32; 4],
    pub color: [u8; 4],
    pub material: MaterialId,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Span {
    pub first: u32,
    pub count: u32,
}
impl Span {
    pub fn range(self) -> Range<usize> {
        self.first as usize..self.first as usize + self.count as usize
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct SceneRanges {
    pub entities: Span,
    pub polys: Span,
    pub lights: Span,
}
#[derive(Clone, Copy, Debug)]
pub struct View {
    pub refdef: Refdef,
    pub scene: SceneRanges,
    pub hidden_areas: Span,
}
#[derive(Clone, Copy, Debug, Default)]
pub enum Command {
    #[default]
    Empty,
    Clear([u8; 4]),
    View(View),
    Draw2d(Draw2d),
}
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub commands: usize,
    pub entities: usize,
    pub polys: usize,
    pub vertices: usize,
    pub lights: usize,
    pub area_bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            commands: 4096,
            entities: 2048,
            polys: 8192,
            vertices: 65536,
            lights: 256,
            area_bytes: 8192,
        }
    }
}
/// Backends borrow a sealed packet. Client-owned pointers never cross this boundary.
pub struct CommandList {
    commands: Box<[Command]>,
    entities: Box<[SceneEntity]>,
    polys: Box<[Poly]>,
    vertices: Box<[Vertex]>,
    lights: Box<[Light]>,
    area_bytes: Box<[u8]>,
    command_count: usize,
    entity_count: usize,
    poly_count: usize,
    vertex_count: usize,
    light_count: usize,
    area_count: usize,
    owner: u64,
    slot: usize,
    pub frame: u64,
    pub rejected: u64,
}
impl CommandList {
    fn load(l: Limits, owner: u64, slot: usize) -> Self {
        Self {
            commands: vec![Command::Empty; l.commands].into_boxed_slice(),
            entities: vec![SceneEntity::default(); l.entities].into_boxed_slice(),
            polys: vec![Poly::default(); l.polys].into_boxed_slice(),
            vertices: vec![Vertex::default(); l.vertices].into_boxed_slice(),
            lights: vec![Light::default(); l.lights].into_boxed_slice(),
            area_bytes: vec![0; l.area_bytes].into_boxed_slice(),
            command_count: 0,
            entity_count: 0,
            poly_count: 0,
            vertex_count: 0,
            light_count: 0,
            area_count: 0,
            owner,
            slot,
            frame: 0,
            rejected: 0,
        }
    }
    pub fn commands(&self) -> &[Command] {
        &self.commands[..self.command_count]
    }
    pub fn entities(&self, range: Span) -> &[SceneEntity] {
        &self.entities[range.range()]
    }
    pub fn polys(&self, range: Span) -> &[Poly] {
        &self.polys[range.range()]
    }
    pub fn vertices(&self, range: Span) -> &[Vertex] {
        &self.vertices[range.range()]
    }
    pub fn lights(&self, range: Span) -> &[Light] {
        &self.lights[range.range()]
    }
    pub fn hidden_areas(&self, range: Span) -> &[u8] {
        &self.area_bytes[range.range()]
    }
    fn reset(&mut self, frame: u64) {
        self.command_count = 0;
        self.entity_count = 0;
        self.poly_count = 0;
        self.vertex_count = 0;
        self.light_count = 0;
        self.area_count = 0;
        self.rejected = 0;
        self.frame = frame;
    }
}
pub struct FrontEnd {
    lists: [Option<CommandList>; 2],
    next: usize,
    frame: u64,
    owner: u64,
}
pub struct Frame {
    list: CommandList,
    first: SceneRanges,
}
impl FrontEnd {
    pub fn load(limits: Limits) -> Result<Self, &'static str> {
        if [
            limits.commands,
            limits.entities,
            limits.polys,
            limits.vertices,
            limits.lights,
            limits.area_bytes,
        ]
        .iter()
        .any(|&n| n == 0 || n > u32::MAX as usize)
        {
            return Err("invalid scene capacity");
        }
        let owner = NEXT_FRONTEND
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| "scene owner ids exhausted")?;
        Ok(Self {
            lists: [
                Some(CommandList::load(limits, owner, 0)),
                Some(CommandList::load(limits, owner, 1)),
            ],
            next: 0,
            frame: 0,
            owner,
        })
    }
    /// A packet leaves the pool until its consumer returns ownership. Two
    /// outstanding packets exhaust the pool rather than allocating or waiting.
    pub fn begin_frame(&mut self, clear: [u8; 4]) -> Option<Frame> {
        let index = self.next;
        let mut list = self.lists[index].take()?;
        self.next ^= 1;
        self.frame = self.frame.wrapping_add(1);
        list.reset(self.frame);
        let mut frame = Frame {
            list,
            first: SceneRanges::default(),
        };
        frame.command(Command::Clear(clear));
        Some(frame)
    }
    pub fn recycle(&mut self, packet: CommandList) -> Result<(), CommandList> {
        if packet.owner != self.owner || self.lists[packet.slot].is_some() {
            return Err(packet);
        }
        let slot = packet.slot;
        self.lists[slot] = Some(packet);
        Ok(())
    }
}
impl Frame {
    fn command(&mut self, command: Command) -> bool {
        if self.list.command_count == self.list.commands.len() {
            self.list.rejected += 1;
            return false;
        }
        self.list.commands[self.list.command_count] = command;
        self.list.command_count += 1;
        true
    }
    fn current(&self) -> SceneRanges {
        SceneRanges {
            entities: Span {
                first: self.list.entity_count as u32,
                count: 0,
            },
            polys: Span {
                first: self.list.poly_count as u32,
                count: 0,
            },
            lights: Span {
                first: self.list.light_count as u32,
                count: 0,
            },
        }
    }
    pub fn clear_scene(&mut self) {
        self.first = self.current();
    }
    pub fn add_entity(&mut self, entity: SceneEntity) -> bool {
        if self.list.entity_count == self.list.entities.len() {
            self.list.rejected += 1;
            return false;
        }
        self.list.entities[self.list.entity_count] = entity;
        self.list.entity_count += 1;
        true
    }
    pub fn add_poly(&mut self, material: MaterialId, vertices: &[Vertex]) -> bool {
        if vertices.len() < 3
            || self.list.poly_count == self.list.polys.len()
            || vertices.len() > self.list.vertices.len() - self.list.vertex_count
        {
            self.list.rejected += 1;
            return false;
        }
        let first = self.list.vertex_count;
        self.list.vertices[first..first + vertices.len()].copy_from_slice(vertices);
        self.list.vertex_count += vertices.len();
        self.list.polys[self.list.poly_count] = Poly {
            material,
            vertices: Span {
                first: first as u32,
                count: vertices.len() as u32,
            },
        };
        self.list.poly_count += 1;
        true
    }
    pub fn add_light(&mut self, light: Light) -> bool {
        if self.list.light_count == self.list.lights.len() {
            self.list.rejected += 1;
            return false;
        }
        self.list.lights[self.list.light_count] = light;
        self.list.light_count += 1;
        true
    }
    pub fn render_scene(&mut self, refdef: Refdef, hidden_areas: &[u8]) -> bool {
        if self.list.command_count == self.list.commands.len()
            || hidden_areas.len() > self.list.area_bytes.len() - self.list.area_count
        {
            self.list.rejected += 1;
            return false;
        }
        let current = self.current();
        let scene = SceneRanges {
            entities: Span {
                count: current.entities.first - self.first.entities.first,
                ..self.first.entities
            },
            polys: Span {
                count: current.polys.first - self.first.polys.first,
                ..self.first.polys
            },
            lights: Span {
                count: current.lights.first - self.first.lights.first,
                ..self.first.lights
            },
        };
        let hidden = Span {
            first: self.list.area_count as u32,
            count: hidden_areas.len() as u32,
        };
        self.list.area_bytes[hidden.range()].copy_from_slice(hidden_areas);
        self.list.area_count += hidden_areas.len();
        self.command(Command::View(View {
            refdef,
            scene,
            hidden_areas: hidden,
        }));
        self.first = current;
        true
    }
    pub fn draw_2d(&mut self, draw: Draw2d) -> bool {
        self.command(Command::Draw2d(draw))
    }
}
impl Frame {
    pub fn finish(self) -> CommandList {
        self.list
    }
}
