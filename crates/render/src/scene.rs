//! Q3-style append-only scene ranges in alternating owned packets.
use crate::assets::{Assets, MaterialId, ModelId, PaletteId, Vertex};
use crate::surface_cache::IndexedLighting;
use crate::world::{VisibleSurface, WorldId};
use qa_core::primitives::Vec3;
use std::cmp::Ordering as SortOrdering;
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CpuPresentation {
    #[default]
    Rgb,
    Indexed {
        palette: PaletteId,
        lighting: IndexedLighting,
        ambient: u8,
        fullbright: bool,
    },
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PerspectiveStep {
    #[default]
    Eight,
    Sixteen,
}
impl PerspectiveStep {
    pub fn pixels(self) -> u32 {
        match self {
            Self::Eight => 8,
            Self::Sixteen => 16,
        }
    }
}
/// Both representations come from the lightstyle provider. They are copied
/// into a view packet so module memory never reaches a backend.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightStyle {
    pub rgb: [f32; 3],
    pub indexed_scale: u16,
}
impl Default for LightStyle {
    fn default() -> Self {
        Self {
            rgb: [1.0; 3],
            indexed_scale: 256,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PaletteShift {
    pub destination: [i32; 3],
    /// Native 0..256 fixed-point weight, before gamma-table application.
    pub percent: i32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaletteTransform {
    /// Native contents, damage, bonus and powerup order.
    pub shifts: [PaletteShift; 4],
    pub gamma: [u8; 256],
    pub operation: PaletteOperation,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PaletteOperation {
    #[default]
    SequentialShifts,
    /// ref_soft uses refdef's combined blend before gamma lookup.
    ScreenBlend,
}
impl Default for PaletteTransform {
    fn default() -> Self {
        Self {
            shifts: [PaletteShift::default(); 4],
            gamma: std::array::from_fn(|i| i as u8),
            operation: PaletteOperation::SequentialShifts,
        }
    }
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
    /// Cached presentation identity light (Q3 native overbright response).
    pub identity_light: f32,
    pub blend: [f32; 4],
    pub blend_phase: BlendPhase,
    /// Final palette shifts also cover a seat's statusbar/console outside the
    /// camera viewport. None uses the camera viewport for standalone scenes.
    pub blend_viewport: Option<Viewport>,
    /// Presentation is independent of the map format, movement and modules.
    pub cpu_presentation: CpuPresentation,
    pub perspective_step: PerspectiveStep,
    /// Native software color shifts preserve integer rounding and gamma;
    /// GL continues to use the combined screen blend. None is unshifted.
    pub palette_transform: Option<PaletteTransform>,
    pub lightstyles: [LightStyle; 256],
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
            identity_light: 1.0,
            blend: [0.0; 4],
            blend_phase: BlendPhase::AfterView,
            blend_viewport: None,
            cpu_presentation: CpuPresentation::Rgb,
            perspective_step: PerspectiveStep::Eight,
            palette_transform: None,
            lightstyles: [LightStyle::default(); 256],
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct SceneEntity {
    pub model: ModelId,
    pub origin: Vec3,
    pub axes: [Vec3; 3],
    pub non_normalized_axes: bool,
    pub color: [u8; 4],
    pub material: Option<MaterialId>,
    pub frame: u32,
    pub old_frame: u32,
    pub back_lerp: f32,
    pub depth_hack: bool,
    pub shader_time: f32,
    pub shader_texcoord: [f32; 2],
    pub lighting: Option<crate::stage::EntityLighting>,
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
            non_normalized_axes: false,
            color: [255; 4],
            material: None,
            frame: 0,
            old_frame: 0,
            back_lerp: 0.0,
            depth_hack: false,
            shader_time: 0.0,
            shader_texcoord: [0.0; 2],
            lighting: None,
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
    pub surfaces: Span,
    pub draws: Span,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct SurfaceRef {
    pub world: WorldId,
    pub surface: u32,
    pub depth_key: u32,
    /// View-local position in the one sorted draw list. Exact-depth ties use
    /// the later draw, matching the shared LEQUAL stage order.
    pub draw_rank: u32,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DrawKind {
    #[default]
    Surface,
    Entity,
    Poly,
}
/// Absolute payload index in this packet. Sorting moves this record alone,
/// leaving visibility/depth order and every earlier view's payload intact.
#[derive(Clone, Copy, Debug, Default)]
pub struct DrawItem {
    pub kind: DrawKind,
    pub index: u32,
    key: DrawKey,
}
#[derive(Clone, Copy, Debug, Default)]
struct DrawKey {
    sort: f32,
    material: u32,
    instance: u32,
    lightmap: u32,
}
impl DrawKey {
    fn compare(self, other: Self) -> SortOrdering {
        // Q3 SortNewShader preserves fractional shader sort and registration
        // order. Its draw key then groups entity instance and lighting data.
        // Signed zero sorts equally, as in the native float comparisons.
        let sort = if self.sort == other.sort {
            SortOrdering::Equal
        } else {
            self.sort.total_cmp(&other.sort)
        };
        sort.then_with(|| self.material.cmp(&other.material))
            .then_with(|| self.instance.cmp(&other.instance))
            .then_with(|| self.lightmap.cmp(&other.lightmap))
    }
}
#[derive(Clone, Copy, Debug)]
pub struct View {
    pub refdef: Refdef,
    pub scene: SceneRanges,
    pub hidden_areas: Span,
}
#[derive(Clone, Copy, Debug, Default)]
#[expect(
    clippy::large_enum_variant,
    reason = "Load-sized command slots own copied refdefs inline; boxing view commands would allocate during frame submission."
)]
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
    pub surfaces: usize,
    /// Zero derives the sum of payload capacities at load time.
    pub draws: usize,
}
impl Limits {
    /// Resolve the load-time draw bound shared by front-end and consumers.
    pub fn draw_capacity(self) -> Option<usize> {
        if self.draws != 0 {
            Some(self.draws)
        } else {
            self.surfaces
                .checked_add(self.entities)?
                .checked_add(self.polys)
        }
    }
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
            surfaces: 65536,
            draws: 0,
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
    surfaces: Box<[SurfaceRef]>,
    draws: Box<[DrawItem]>,
    command_count: usize,
    entity_count: usize,
    poly_count: usize,
    vertex_count: usize,
    light_count: usize,
    area_count: usize,
    surface_count: usize,
    draw_count: usize,
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
            surfaces: vec![SurfaceRef::default(); l.surfaces].into_boxed_slice(),
            draws: vec![DrawItem::default(); l.draws].into_boxed_slice(),
            command_count: 0,
            entity_count: 0,
            poly_count: 0,
            vertex_count: 0,
            light_count: 0,
            area_count: 0,
            surface_count: 0,
            draw_count: 0,
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
    pub fn surfaces(&self, range: Span) -> &[SurfaceRef] {
        &self.surfaces[range.range()]
    }
    pub fn draws(&self, range: Span) -> &[DrawItem] {
        &self.draws[range.range()]
    }
    pub fn surface(&self, index: u32) -> &SurfaceRef {
        &self.surfaces[index as usize]
    }
    pub fn entity(&self, index: u32) -> &SceneEntity {
        &self.entities[index as usize]
    }
    pub fn poly(&self, index: u32) -> &Poly {
        &self.polys[index as usize]
    }
    fn reset(&mut self, frame: u64) {
        self.command_count = 0;
        self.entity_count = 0;
        self.poly_count = 0;
        self.vertex_count = 0;
        self.light_count = 0;
        self.area_count = 0;
        self.surface_count = 0;
        self.draw_count = 0;
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
    pub fn load(mut limits: Limits) -> Result<Self, &'static str> {
        limits.draws = limits
            .draw_capacity()
            .ok_or("scene draw capacity overflow")?;
        if [
            limits.commands,
            limits.entities,
            limits.polys,
            limits.vertices,
            limits.lights,
            limits.area_bytes,
            limits.surfaces,
            limits.draws,
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
    #[expect(
        clippy::result_large_err,
        reason = "Rejected recycling returns ownership of the packet's existing arenas without boxing or cloning during a frame."
    )]
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
            surfaces: Span {
                first: self.list.surface_count as u32,
                count: 0,
            },
            draws: Span {
                first: self.list.draw_count as u32,
                count: 0,
            },
        }
    }
    pub fn clear_scene(&mut self) {
        self.first = self.current();
    }
    pub fn add_entity(&mut self, entity: SceneEntity) -> bool {
        if self.list.entity_count == self.list.entities.len()
            || self.list.draw_count == self.list.draws.len()
        {
            self.list.rejected += 1;
            return false;
        }
        self.list.entities[self.list.entity_count] = entity;
        self.draw(DrawKind::Entity, self.list.entity_count as u32);
        self.list.entity_count += 1;
        true
    }
    pub fn add_poly(&mut self, material: MaterialId, vertices: &[Vertex]) -> bool {
        if vertices.len() < 3
            || self.list.poly_count == self.list.polys.len()
            || self.list.draw_count == self.list.draws.len()
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
        self.draw(DrawKind::Poly, self.list.poly_count as u32);
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
    /// Copy the one frontend visibility result into this owned packet. Both
    /// consumers receive these ids and the CPU's original BSP depth keys.
    pub fn add_world(&mut self, world: WorldId, surfaces: &[VisibleSurface]) -> bool {
        if surfaces.len() > self.list.surfaces.len() - self.list.surface_count
            || surfaces.len() > self.list.draws.len() - self.list.draw_count
        {
            self.list.rejected += 1;
            return false;
        }
        let first = self.list.surface_count;
        for (target, source) in self.list.surfaces[first..first + surfaces.len()]
            .iter_mut()
            .zip(surfaces)
        {
            *target = SurfaceRef {
                world,
                surface: source.surface,
                depth_key: source.depth_key,
                draw_rank: 0,
            };
        }
        for index in first..first + surfaces.len() {
            self.draw(DrawKind::Surface, index as u32);
        }
        self.list.surface_count += surfaces.len();
        true
    }
    fn draw(&mut self, kind: DrawKind, index: u32) {
        self.list.draws[self.list.draw_count] = DrawItem {
            kind,
            index,
            key: DrawKey::default(),
        };
        self.list.draw_count += 1;
    }
    pub fn render_scene(&mut self, refdef: Refdef, hidden_areas: &[u8], assets: &Assets) -> bool {
        if self.list.command_count == self.list.commands.len()
            || hidden_areas.len() > self.list.area_bytes.len() - self.list.area_count
        {
            self.list.rejected += 1;
            return false;
        }
        // Frozen numeric handles are resolved at the scene boundary, once per
        // item. Invalid client submissions are omitted without affecting peers.
        let first_draw = self.first.draws.first as usize;
        let mut valid = first_draw;
        for index in first_draw..self.list.draw_count {
            let mut draw = self.list.draws[index];
            if let Some(key) = draw_key(&self.list, assets, draw) {
                draw.key = key;
                self.list.draws[valid] = draw;
                valid += 1;
            } else {
                self.list.rejected += 1;
            }
        }
        self.list.draw_count = valid;
        native_draw_sort(&mut self.list.draws[first_draw..valid]);
        for (rank, draw) in self.list.draws[first_draw..valid].iter().enumerate() {
            if draw.kind == DrawKind::Surface {
                self.list.surfaces[draw.index as usize].draw_rank = rank as u32;
            }
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
            surfaces: Span {
                count: current.surfaces.first - self.first.surfaces.first,
                ..self.first.surfaces
            },
            draws: Span {
                count: current.draws.first - self.first.draws.first,
                ..self.first.draws
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
fn draw_key(list: &CommandList, assets: &Assets, draw: DrawItem) -> Option<DrawKey> {
    let (material, instance, lightmap) = match draw.kind {
        DrawKind::Surface => {
            let surface = list.surface(draw.index);
            let binding = assets
                .world(surface.world)?
                .bindings()
                .get(surface.surface as usize)?;
            (binding.material, u32::MAX, binding.lightmap.0)
        }
        DrawKind::Entity => {
            let entity = list.entity(draw.index);
            let model = assets.model(entity.model)?;
            (entity.material.unwrap_or(model.material), draw.index, 0)
        }
        DrawKind::Poly => (list.poly(draw.index).material, u32::MAX, 0),
    };
    Some(DrawKey {
        sort: assets.material(material)?.settings.sort,
        material: material.0,
        instance,
        lightmap,
    })
}
/// Q3 tr_main.c qsortFast/shortsort. Equal keys deliberately retain the native
/// swap order: alpha draws observe it. SortNewShader supplies the float/material
/// ordering; neither camera distance nor an insertion ordinal enters this key.
fn native_draw_sort(draws: &mut [DrawItem]) {
    if draws.len() < 2 {
        return;
    }
    // The larger partition is stacked and the smaller is processed first.
    // Packet capacity is at most u32::MAX; this fixed stack therefore suffices.
    let mut pending = [(0_usize, 0_usize); u32::BITS as usize];
    let mut pending_count = 0;
    let mut lo = 0;
    let mut hi = draws.len() - 1;
    loop {
        let size = hi - lo + 1;
        if size <= 8 {
            let mut last = hi;
            while last > lo {
                let mut maximum = lo;
                for index in lo + 1..=last {
                    if draws[index].key.compare(draws[maximum].key) == SortOrdering::Greater {
                        maximum = index;
                    }
                }
                draws.swap(maximum, last);
                last -= 1;
            }
        } else {
            draws.swap(lo + size / 2, lo);
            let mut low = lo;
            let mut high = hi + 1;
            loop {
                loop {
                    low += 1;
                    if low > hi || draws[low].key.compare(draws[lo].key) == SortOrdering::Greater {
                        break;
                    }
                }
                loop {
                    high -= 1;
                    if high <= lo || draws[high].key.compare(draws[lo].key) == SortOrdering::Less {
                        break;
                    }
                }
                if high < low {
                    break;
                }
                draws.swap(low, high);
            }
            draws.swap(lo, high);
            let left_count = high - lo;
            let right_count = hi + 1 - low;
            if left_count >= right_count {
                if left_count > 1 {
                    pending[pending_count] = (lo, high - 1);
                    pending_count += 1;
                }
                if right_count > 1 {
                    lo = low;
                    continue;
                }
            } else {
                if right_count > 1 {
                    pending[pending_count] = (low, hi);
                    pending_count += 1;
                }
                if left_count > 1 {
                    hi = high - 1;
                    continue;
                }
            }
        }
        if pending_count == 0 {
            break;
        }
        pending_count -= 1;
        (lo, hi) = pending[pending_count];
    }
}
impl Frame {
    pub fn finish(self) -> CommandList {
        self.list
    }
}
