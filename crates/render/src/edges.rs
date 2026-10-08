//! Opaque world edge/span visibility, following qsrc r_draw.c and r_edge.c.
//!
//! The caller clips against the near plane and supplies convex projected
//! polygons. Partitioned geometry uses native BSP keys; unpartitioned or mixed
//! geometry uses affine inverse-depth planes on the same edge/span machinery.
//! Neither policy depends on the game family.
use crate::scene::Viewport;

const NONE: usize = usize::MAX;
const FRACTION_BITS: u32 = 20;
const FRACTION: i64 = 1 << FRACTION_BITS;
const BIAS: i64 = FRACTION - 1;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DepthPolicy {
    /// Native partitioned world surfaces: smaller traversal keys are nearer.
    #[default]
    BspKeys,
    /// Whole surfaces, curves and overlapping worlds require actual depth.
    PlaneDepth,
}

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
    original_start: u32,
    end: u32,
    delta: i32,
    polygon: usize,
    next: usize,
}

#[derive(Clone, Copy, Default)]
struct Surface {
    id: u32,
    key: u32,
    draw_rank: u32,
    winding: i32,
    depth: DepthPlane,
    activation: usize,
}

#[derive(Clone, Copy, Default)]
struct DepthPlane {
    x: f64,
    y: f64,
    origin: f64,
}

impl DepthPlane {
    fn at(self, x: u32, y: u32) -> f64 {
        self.x * f64::from(x) + self.row_origin(y)
    }

    fn row_origin(self, y: u32) -> f64 {
        self.y * f64::from(y) + self.origin
    }
}

pub struct Edges {
    width: u32,
    height: u32,
    viewport: Viewport,
    row_start: u32,
    row_end: u32,
    buckets: Box<[usize]>,
    edges: Box<[Edge]>,
    active: Box<[usize]>,
    surfaces: Box<[Surface]>,
    stack: Box<[usize]>,
    depth_stack: Box<[usize]>,
    spans: Box<[Span]>,
    edge_count: usize,
    surface_count: usize,
    active_count: usize,
    stack_count: usize,
    span_count: usize,
    collecting: bool,
    policy: DepthPolicy,
    activation: usize,
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
            || max_edges > isize::MAX as usize / size_of::<Edge>().max(size_of::<usize>())
            || max_polygons > isize::MAX as usize / size_of::<Surface>().max(size_of::<usize>())
            || max_spans > isize::MAX as usize / size_of::<Span>()
        {
            return Err("invalid world edge arena dimensions");
        }
        Ok(Self {
            width,
            height,
            viewport: Viewport::default(),
            row_start: 0,
            row_end: 0,
            buckets: vec![NONE; height as usize].into_boxed_slice(),
            edges: vec![Edge::default(); max_edges].into_boxed_slice(),
            active: vec![0; max_edges].into_boxed_slice(),
            surfaces: vec![Surface::default(); max_polygons].into_boxed_slice(),
            stack: vec![0; max_polygons].into_boxed_slice(),
            depth_stack: vec![0; max_polygons].into_boxed_slice(),
            spans: vec![Span::default(); max_spans].into_boxed_slice(),
            edge_count: 0,
            surface_count: 0,
            active_count: 0,
            stack_count: 0,
            span_count: 0,
            collecting: false,
            policy: DepthPolicy::BspKeys,
            activation: 0,
            stats: Stats::default(),
        })
    }

    /// Starts one view. Arena capacities and allocation remain unchanged.
    pub fn begin(&mut self, viewport: Viewport) -> bool {
        self.begin_with_policy(viewport, DepthPolicy::BspKeys)
    }

    /// Select by geometry partition metadata and view contents. Combining
    /// overlapping worlds requires PlaneDepth even if each is partitioned.
    pub fn begin_with_policy(&mut self, viewport: Viewport, policy: DepthPolicy) -> bool {
        if let Some(bottom) = viewport.y.checked_add(viewport.height)
            && self.begin_band(viewport, viewport.y..bottom, policy)
        {
            return true;
        }
        // Retain begin's established invalid-view behavior. begin_band itself
        // leaves an existing collection intact when its row window is invalid.
        self.reset();
        self.collecting = false;
        self.stats.rejected = 1;
        false
    }

    /// Scan a nonempty global row window using the full projection viewport.
    /// Each band receives the same polygons; edges retain the native trajectory
    /// computed at the full view's top. Invalid bounds preserve queued work.
    pub fn begin_band(
        &mut self,
        viewport: Viewport,
        rows: std::ops::Range<u32>,
        policy: DepthPolicy,
    ) -> bool {
        if viewport.width == 0
            || viewport.height == 0
            || viewport
                .x
                .checked_add(viewport.width)
                .is_none_or(|right| right > self.width)
            || viewport
                .y
                .checked_add(viewport.height)
                .is_none_or(|bottom| bottom > self.height || rows.end > bottom)
            || rows.start < viewport.y
            || rows.start >= rows.end
        {
            self.stats.rejected = self.stats.rejected.saturating_add(1);
            return false;
        }
        self.reset();
        self.viewport = viewport;
        self.row_start = rows.start;
        self.row_end = rows.end;
        self.policy = policy;
        self.collecting = true;
        true
    }

    fn reset(&mut self) {
        self.buckets.fill(NONE);
        self.edge_count = 0;
        self.surface_count = 0;
        self.active_count = 0;
        self.stack_count = 0;
        self.span_count = 0;
        self.stats = Stats::default();
    }

    /// Copies edges, surface identity and its visibility depth plane. Texture
    /// sampling remains with the caller. Differing planes need distinct surface
    /// ids, including individual curve triangles. Capacity failure is atomic.
    /// BSP keys belong to one partitioned world. Plane-depth ties use the one
    /// scene's draw rank instead, matching a later GL LEQUAL draw's overwrite.
    pub fn add_polygon(
        &mut self,
        surface: u32,
        key: u32,
        draw_rank: u32,
        vertices: &[ProjectedVertex],
    ) -> bool {
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
            match make_band_edge(
                previous,
                vertex,
                clockwise,
                self.viewport,
                self.row_start,
                self.row_end,
            ) {
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
        let depth = match self.policy {
            DepthPolicy::BspKeys => DepthPlane::default(),
            DepthPolicy::PlaneDepth => {
                let Some(depth) = depth_plane(vertices) else {
                    self.stats.rejected += 1;
                    return false;
                };
                depth
            }
        };
        let polygon = self.surface_count;
        self.surfaces[polygon] = Surface {
            id: surface,
            key,
            draw_rank,
            winding: 0,
            depth,
            activation: 0,
        };
        self.surface_count += 1;
        previous = vertices[vertices.len() - 1];
        for &vertex in vertices {
            // The first pass validated exactly these immutable edges.
            if let Ok(Some(mut edge)) = make_band_edge(
                previous,
                vertex,
                clockwise,
                self.viewport,
                self.row_start,
                self.row_end,
            ) {
                let index = self.edge_count;
                edge.polygon = polygon;
                self.edges[index] = edge;
                if edge.original_start < self.row_start {
                    self.active[self.active_count] = index;
                    self.active_count += 1;
                } else {
                    self.insert_new_edge(index);
                }
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
        self.sort_carried();
        for y in self.row_start..self.row_end {
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
            self.activation = 0;
            for &edge in &self.active[..self.active_count] {
                self.surfaces[self.edges[edge].polygon].winding = 0;
            }
            let mut run = None;
            let mut start = self.viewport.x;
            let mut index = 0;
            while index < self.active_count {
                let x = self.edge_x(self.active[index]);
                if self.policy == DepthPolicy::PlaneDepth {
                    self.emit_depth_interval(start, x, y, &mut flush);
                    start = x;
                }
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
                if self.policy == DepthPolicy::BspKeys {
                    let next = (self.stack_count != 0).then(|| self.stack[0]);
                    if next != run {
                        if let Some(polygon) = run {
                            self.emit(polygon, start, y, x - start, &mut flush);
                        }
                        start = x;
                        run = next;
                    }
                }
            }
            match self.policy {
                DepthPolicy::BspKeys => {
                    if let Some(polygon) = run {
                        self.emit(polygon, start, y, right - start, &mut flush);
                    }
                }
                DepthPolicy::PlaneDepth => self.emit_depth_interval(start, right, y, &mut flush),
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

    fn sort_carried(&mut self) {
        // Reconstruct R_StepActiveU's stable incoming AET at the band top.
        // At an exact-U crossing, the previous row orders larger steps first.
        // Coincident trajectories retain their native insertion history.
        for index in 1..self.active_count {
            let edge = self.active[index];
            let mut position = index;
            while position != 0 && self.carried_before(edge, self.active[position - 1]) {
                self.active[position] = self.active[position - 1];
                position -= 1;
            }
            self.active[position] = edge;
        }
    }

    fn carried_before(&self, first: usize, second: usize) -> bool {
        let a = self.edges[first];
        let b = self.edges[second];
        if a.u != b.u {
            return a.u < b.u;
        }
        if a.step != b.step {
            return a.step > b.step;
        }
        if a.original_start != b.original_start {
            return a.original_start > b.original_start;
        }
        if a.delta != b.delta {
            return a.delta > b.delta;
        }
        if a.delta > 0 {
            first > second
        } else {
            first < second
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
        self.surfaces[polygon].activation = self.activation;
        self.activation += 1;
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
        if self.policy == DepthPolicy::PlaneDepth && self.span_count != 0 {
            let previous = &mut self.spans[self.span_count - 1];
            if previous.surface == self.surfaces[polygon].id
                && previous.y == y
                && previous.x + previous.count == x
            {
                previous.count += count;
                self.stats.pixels += u64::from(count);
                return;
            }
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

    fn emit_depth_interval(
        &mut self,
        mut x: u32,
        end: u32,
        y: u32,
        flush: &mut impl FnMut(&[Span]),
    ) {
        if self.stack_count == 0 || x == end {
            return;
        }
        self.depth_stack[..self.stack_count].copy_from_slice(&self.stack[..self.stack_count]);
        while x < end {
            // At each edge/depth event order the fixed active stack. Exact
            // depth ties use native key/activation order, without an epsilon.
            for index in 1..self.stack_count {
                let surface = self.depth_stack[index];
                let mut position = index;
                while position != 0
                    && self.depth_before(surface, self.depth_stack[position - 1], x, y)
                {
                    self.depth_stack[position] = self.depth_stack[position - 1];
                    position -= 1;
                }
                self.depth_stack[position] = surface;
            }
            let visible = self.depth_stack[0];
            let mut next = end;
            for index in 1..self.stack_count {
                if let Some(crossing) =
                    self.first_overtake(visible, self.depth_stack[index], x, end, y)
                {
                    next = next.min(crossing);
                }
            }
            self.emit(visible, x, y, next - x, flush);
            x = next;
        }
    }

    fn depth_before(&self, first: usize, second: usize, x: u32, y: u32) -> bool {
        let first = self.surfaces[first];
        let second = self.surfaces[second];
        let first_depth = first.depth.at(x, y);
        let second_depth = second.depth.at(x, y);
        first_depth > second_depth
            || (first_depth == second_depth
                && (first.draw_rank > second.draw_rank
                    || (first.draw_rank == second.draw_rank
                        && first.activation < second.activation)))
    }

    fn first_overtake(
        &self,
        visible: usize,
        other: usize,
        x: u32,
        end: u32,
        y: u32,
    ) -> Option<u32> {
        let visible_plane = self.surfaces[visible].depth;
        let other_plane = self.surfaces[other].depth;
        let slope = other_plane.x - visible_plane.x;
        if slope <= 0.0 {
            return None;
        }
        let crossing = (visible_plane.row_origin(y) - other_plane.row_origin(y)) / slope;
        if !crossing.is_finite() || crossing >= f64::from(end) {
            return None;
        }
        // The analytic crossing gives adjacent integer candidates. Evaluate
        // the actual affine planes there to retain exact sample/tie ownership.
        let sample = crossing.floor().max(f64::from(x)) as u32;
        let first = if self.depth_before(other, visible, sample, y) {
            sample
        } else {
            sample + 1
        };
        (first > x && first < end).then_some(first)
    }

    fn flush(&mut self, flush: &mut impl FnMut(&[Span])) {
        if self.span_count != 0 {
            flush(&self.spans[..self.span_count]);
            self.span_count = 0;
            self.stats.flushes += 1;
        }
    }
}

fn depth_plane(vertices: &[ProjectedVertex]) -> Option<DepthPlane> {
    let origin = vertices[0];
    let mut largest_area = 0.0_f64;
    let mut basis = None;
    for index in 1..vertices.len() - 1 {
        let first = vertices[index];
        let second = vertices[index + 1];
        let ax = f64::from(first.xy[0]) - f64::from(origin.xy[0]);
        let ay = f64::from(first.xy[1]) - f64::from(origin.xy[1]);
        let az = f64::from(first.inverse_depth) - f64::from(origin.inverse_depth);
        let bx = f64::from(second.xy[0]) - f64::from(origin.xy[0]);
        let by = f64::from(second.xy[1]) - f64::from(origin.xy[1]);
        let bz = f64::from(second.inverse_depth) - f64::from(origin.inverse_depth);
        let determinant = ax * by - bx * ay;
        // The first nonzero fan triangle may be nearly collinear. Its rounded
        // f32 depth samples can lose a slope entirely. Use the widest projected
        // basis without an epsilon that would discard legitimate thin geometry.
        if determinant.abs() > largest_area {
            largest_area = determinant.abs();
            basis = Some([ax, ay, az, bx, by, bz, determinant]);
        }
    }
    let [ax, ay, az, bx, by, bz, determinant] = basis?;
    let x = (az * by - bz * ay) / determinant;
    let y = (ax * bz - bx * az) / determinant;
    let origin =
        f64::from(origin.inverse_depth) - x * f64::from(origin.xy[0]) - y * f64::from(origin.xy[1]);
    [x, y, origin]
        .iter()
        .all(|coefficient| coefficient.is_finite())
        .then_some(DepthPlane { x, y, origin })
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
        original_start: start as u32,
        end: end as u32,
        delta: if (a.xy[1] > b.xy[1]) == clockwise {
            1
        } else {
            -1
        },
        ..Edge::default()
    }))
}

fn make_band_edge(
    a: ProjectedVertex,
    b: ProjectedVertex,
    clockwise: bool,
    viewport: Viewport,
    row_start: u32,
    row_end: u32,
) -> Result<Option<Edge>, ()> {
    let Some(mut edge) = make_edge(a, b, clockwise, viewport)? else {
        return Ok(None);
    };
    if edge.end <= row_start || edge.start >= row_end {
        return Ok(None);
    }
    let start = edge.start.max(row_start);
    let change = edge
        .step
        .checked_mul(i64::from(start - edge.start))
        .ok_or(())?;
    edge.u = edge.u.checked_add(change).ok_or(())?;
    edge.start = start;
    edge.end = edge.end.min(row_end);
    Ok(Some(edge))
}
