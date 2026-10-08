//! Opaque world edge/span visibility, following qsrc r_draw.c and r_edge.c.
//!
//! The caller clips against the near plane and supplies convex projected
//! polygons in BSP order. Smaller keys are nearer. Static surfaces with equal
//! keys retain the already-active surface, as R_LeadingEdge does; inline brush
//! models' same-leaf inverse-depth ordering remains outside this scanner.
use crate::scene::Viewport;

const NONE: usize = usize::MAX;
const FRACTION_BITS: u32 = 20;
const FRACTION: i64 = 1 << FRACTION_BITS;
const BIAS: i64 = FRACTION - 1;

#[derive(Clone, Copy, Debug, Default)]
pub struct ProjectedVertex {
    pub xy: [f32; 2],
    pub inverse_depth: f32,
    pub texcoord_over_depth: [f32; 2],
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Span {
    pub surface: u32,
    pub x: u32,
    pub y: u32,
    pub count: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub polygons: u64,
    pub edges: u64,
    pub spans: u64,
    pub pixels: u64,
    pub flushes: u64,
    pub rejected: u64,
}

#[derive(Clone, Copy, Default)]
struct Edge {
    u: i64,
    step: i64,
    start: u32,
    end: u32,
    polygon: usize,
    delta: i32,
    next: usize,
}

#[derive(Clone, Copy, Default)]
struct Surface {
    id: u32,
    key: u32,
    winding: i32,
}

pub struct Edges {
    width: u32,
    height: u32,
    viewport: Viewport,
    buckets: Box<[usize]>,
    edges: Box<[Edge]>,
    active: Box<[usize]>,
    surfaces: Box<[Surface]>,
    stack: Box<[usize]>,
    spans: Box<[Span]>,
    edge_count: usize,
    surface_count: usize,
    active_count: usize,
    stack_count: usize,
    span_count: usize,
    collecting: bool,
    stats: Stats,
}

impl Edges {
    pub fn load(
        width: u32,
        height: u32,
        max_edges: usize,
        max_polygons: usize,
        max_spans: usize,
    ) -> Result<Self, &'static str> {
        if width == 0
            || height == 0
            || width > 8192
            || height > 8192
            || max_edges == 0
            || max_polygons == 0
            || max_spans == 0
        {
            return Err("invalid world edge arena dimensions");
        }
        Ok(Self {
            width,
            height,
            viewport: Viewport::default(),
            buckets: vec![NONE; height as usize].into_boxed_slice(),
            edges: vec![Edge::default(); max_edges].into_boxed_slice(),
            active: vec![0; max_edges].into_boxed_slice(),
            surfaces: vec![Surface::default(); max_polygons].into_boxed_slice(),
            stack: vec![0; max_polygons].into_boxed_slice(),
            spans: vec![Span::default(); max_spans].into_boxed_slice(),
            edge_count: 0,
            surface_count: 0,
            active_count: 0,
            stack_count: 0,
            span_count: 0,
            collecting: false,
            stats: Stats::default(),
        })
    }

    /// Starts one view. Arena capacities and allocation remain unchanged.
    pub fn begin(&mut self, viewport: Viewport) -> bool {
        self.buckets.fill(NONE);
        self.edge_count = 0;
        self.surface_count = 0;
        self.active_count = 0;
        self.stack_count = 0;
        self.span_count = 0;
        self.stats = Stats::default();
        self.collecting = viewport.width != 0
            && viewport.height != 0
            && viewport
                .x
                .checked_add(viewport.width)
                .is_some_and(|x| x <= self.width)
            && viewport
                .y
                .checked_add(viewport.height)
                .is_some_and(|y| y <= self.height);
        if self.collecting {
            self.viewport = viewport;
        } else {
            self.stats.rejected = 1;
        }
        self.collecting
    }

    /// Copies only edges and surface identity. Sampling planes remain with the
    /// caller. Validation and capacity failure drop the entire polygon.
    pub fn add_polygon(&mut self, surface: u32, key: u32, vertices: &[ProjectedVertex]) -> bool {
        if !self.collecting
            || vertices.len() < 3
            || vertices.iter().any(|vertex| {
                !vertex
                    .xy
                    .iter()
                    .chain(vertex.texcoord_over_depth.iter())
                    .all(|f| f.is_finite())
                    || !vertex.inverse_depth.is_finite()
                    || vertex.inverse_depth <= 0.0
            })
        {
            self.stats.rejected += 1;
            return false;
        }
        let mut area = 0.0_f64;
        let mut previous = vertices[vertices.len() - 1];
        for &vertex in vertices {
            area += f64::from(previous.xy[0]) * f64::from(vertex.xy[1])
                - f64::from(vertex.xy[0]) * f64::from(previous.xy[1]);
            previous = vertex;
        }
        if area == 0.0 || !area.is_finite() {
            self.stats.rejected += 1;
            return false;
        }
        let clockwise = area > 0.0;
        let mut needed = 0usize;
        previous = vertices[vertices.len() - 1];
        for &vertex in vertices {
            match make_edge(previous, vertex, clockwise, self.viewport) {
                Ok(Some(_)) => needed += 1,
                Ok(None) => {}
                Err(()) => {
                    self.stats.rejected += 1;
                    return false;
                }
            }
            previous = vertex;
        }
        if needed == 0 {
            return true;
        }
        if needed > self.edges.len() - self.edge_count || self.surface_count == self.surfaces.len()
        {
            self.stats.rejected += 1;
            return false;
        }
        let polygon = self.surface_count;
        self.surfaces[polygon] = Surface {
            id: surface,
            key,
            winding: 0,
        };
        self.surface_count += 1;
        previous = vertices[vertices.len() - 1];
        for &vertex in vertices {
            // The first pass validated exactly these immutable edges.
            if let Ok(Some(mut edge)) = make_edge(previous, vertex, clockwise, self.viewport) {
                let index = self.edge_count;
                edge.polygon = polygon;
                self.edges[index] = edge;
                self.insert_new_edge(index);
                self.edge_count += 1;
            }
            previous = vertex;
        }
        self.stats.polygons += 1;
        self.stats.edges += needed as u64;
        true
    }

    /// Flushes a fixed reusable span arena. The callback must consume its slice
    /// before returning; it is reused on the next flush, possibly in this row.
    pub fn scan(&mut self, mut flush: impl FnMut(&[Span])) -> Stats {
        if !self.collecting {
            return self.stats;
        }
        self.collecting = false;
        let right = self.viewport.x + self.viewport.width;
        let bottom = self.viewport.y + self.viewport.height;
        for y in self.viewport.y..bottom {
            let mut kept = 0;
            for index in 0..self.active_count {
                let edge = self.active[index];
                if self.edges[edge].end > y {
                    self.active[kept] = edge;
                    kept += 1;
                }
            }
            self.active_count = kept;
            self.sort_active();
            let mut edge = self.buckets[y as usize];
            let mut position = 0;
            while edge != NONE {
                // R_InsertNewEdges inserts before an equal-U existing edge,
                // retaining the GET's order by advancing past each insertion.
                while position < self.active_count
                    && self.edges[self.active[position]].u < self.edges[edge].u
                {
                    position += 1;
                }
                self.active
                    .copy_within(position..self.active_count, position + 1);
                self.active[position] = edge;
                self.active_count += 1;
                position += 1;
                edge = self.edges[edge].next;
            }
            self.stack_count = 0;
            for &edge in &self.active[..self.active_count] {
                self.surfaces[self.edges[edge].polygon].winding = 0;
            }
            let mut run = None;
            let mut start = self.viewport.x;
            let mut index = 0;
            while index < self.active_count {
                let x = self.edge_x(self.active[index]);
                // All events at this integer pixel boundary are atomic: no
                // intermediate zero-width span changes the visible surface.
                while index < self.active_count && self.edge_x(self.active[index]) == x {
                    let edge = self.edges[self.active[index]];
                    let before = self.surfaces[edge.polygon].winding;
                    let after = before + edge.delta;
                    self.surfaces[edge.polygon].winding = after;
                    if before <= 0 && after > 0 {
                        self.insert_surface(edge.polygon);
                    } else if before > 0 && after <= 0 {
                        self.remove_surface(edge.polygon);
                    }
                    index += 1;
                }
                let next = (self.stack_count != 0).then(|| self.stack[0]);
                if next != run {
                    if let Some(polygon) = run {
                        self.emit(polygon, start, y, x - start, &mut flush);
                    }
                    start = x;
                    run = next;
                }
            }
            if let Some(polygon) = run {
                self.emit(polygon, start, y, right - start, &mut flush);
            }
            for &index in &self.active[..self.active_count] {
                let edge = &mut self.edges[index];
                edge.u += edge.step;
            }
        }
        self.flush(&mut flush);
        self.stats
    }

    fn edge_x(&self, index: usize) -> u32 {
        (self.edges[index].u >> FRACTION_BITS).clamp(
            i64::from(self.viewport.x),
            i64::from(self.viewport.x + self.viewport.width),
        ) as u32
    }

    fn sort_active(&mut self) {
        // R_StepActiveU's insertion repair: most stepped edges remain ordered.
        for index in 1..self.active_count {
            let edge = self.active[index];
            let mut position = index;
            while position != 0 && self.edges[edge].u < self.edges[self.active[position - 1]].u {
                self.active[position] = self.active[position - 1];
                position -= 1;
            }
            self.active[position] = edge;
        }
    }

    fn insert_new_edge(&mut self, index: usize) {
        // qsrc R_EmitEdge sorts trailers after equal-U leaders. A later equal-U
        // leader goes before existing new edges; trailer ties retain order.
        let edge = self.edges[index];
        let u_check = i128::from(edge.u) + if edge.delta < 0 { 1 } else { 0 };
        let bucket = edge.start as usize;
        let mut previous = NONE;
        let mut current = self.buckets[bucket];
        while current != NONE && i128::from(self.edges[current].u) < u_check {
            previous = current;
            current = self.edges[current].next;
        }
        self.edges[index].next = current;
        if previous == NONE {
            self.buckets[bucket] = index;
        } else {
            self.edges[previous].next = index;
        }
    }

    fn insert_surface(&mut self, polygon: usize) {
        let key = self.surfaces[polygon].key;
        let mut position = 0;
        while position < self.stack_count && self.surfaces[self.stack[position]].key <= key {
            position += 1;
        }
        self.stack
            .copy_within(position..self.stack_count, position + 1);
        self.stack[position] = polygon;
        self.stack_count += 1;
    }

    fn remove_surface(&mut self, polygon: usize) {
        if let Some(position) = self.stack[..self.stack_count]
            .iter()
            .position(|&p| p == polygon)
        {
            self.stack
                .copy_within(position + 1..self.stack_count, position);
            self.stack_count -= 1;
        }
    }

    fn emit(
        &mut self,
        polygon: usize,
        x: u32,
        y: u32,
        count: u32,
        flush: &mut impl FnMut(&[Span]),
    ) {
        if count == 0 {
            return;
        }
        if self.span_count == self.spans.len() {
            self.flush(flush);
        }
        self.spans[self.span_count] = Span {
            surface: self.surfaces[polygon].id,
            x,
            y,
            count,
        };
        self.span_count += 1;
        self.stats.spans += 1;
        self.stats.pixels += u64::from(count);
    }

    fn flush(&mut self, flush: &mut impl FnMut(&[Span])) {
        if self.span_count != 0 {
            flush(&self.spans[..self.span_count]);
            self.span_count = 0;
            self.stats.flushes += 1;
        }
    }
}

fn make_edge(
    a: ProjectedVertex,
    b: ProjectedVertex,
    clockwise: bool,
    viewport: Viewport,
) -> Result<Option<Edge>, ()> {
    let (top, bottom) = if a.xy[1] <= b.xy[1] { (a, b) } else { (b, a) };
    // Native rows sample integer y; a bottom ceil row is never covered.
    let start = top.xy[1].ceil().max(viewport.y as f32);
    let end = bottom.xy[1]
        .ceil()
        .min((viewport.y + viewport.height) as f32);
    if start >= end {
        return Ok(None);
    }
    let step = (bottom.xy[0] - top.xy[0]) / (bottom.xy[1] - top.xy[1]);
    let u = top.xy[0] + (start - top.xy[1]) * step;
    let fixed_step = step * FRACTION as f32;
    // Widen before adding the native ceil bias. In f32, 0xfffff rounds
    // upward at ordinary integer screen coordinates and moves an edge a pixel.
    let fixed_u = f64::from(u) * FRACTION as f64 + BIAS as f64;
    if !fixed_step.is_finite()
        || !fixed_u.is_finite()
        || f64::from(fixed_step).abs() >= i64::MAX as f64
        || fixed_u.abs() >= i64::MAX as f64
    {
        return Err(());
    }
    let step = fixed_step as i64;
    let u = fixed_u as i64;
    let rows = (end as u32 - start as u32) as i64;
    // Include the final unused step so the hot AET update cannot overflow.
    if step
        .checked_mul(rows)
        .and_then(|change| u.checked_add(change))
        .is_none()
    {
        return Err(());
    }
    Ok(Some(Edge {
        u,
        step,
        start: start as u32,
        end: end as u32,
        delta: if (a.xy[1] > b.xy[1]) == clockwise {
            1
        } else {
            -1
        },
        ..Edge::default()
    }))
}
