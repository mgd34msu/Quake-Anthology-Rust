//! A per-boundary clipping graph preserves the original shader-before-clip
//! interpolation while sharing geometry clipping across material stages.
use super::{Camera, ClipVertex, ScreenVertex, evaluated_vertex};
use crate::assets::Vertex;
use crate::edges::ProjectedVertex;
use crate::stage::{DeformOp, PreparedStage, StageEvaluator};

#[derive(Clone, Copy)]
enum ClipOperation {
    Source(usize),
    Intersection {
        left: usize,
        right: usize,
        fraction: f32,
    },
}

impl Default for ClipOperation {
    fn default() -> Self {
        Self::Source(0)
    }
}

#[derive(Clone, Copy, Default)]
struct ClipRecord {
    operation: ClipOperation,
    base: ClipVertex,
}

pub(super) struct ClipGraph {
    originals: Box<[Vertex]>,
    nodes: Box<[ClipRecord]>,
    attributes: Box<[ClipVertex]>,
    geometry: Box<[ScreenVertex]>,
    input: Box<[usize]>,
    output: Box<[usize]>,
    distances: Box<[f32]>,
    node_count: usize,
    count: usize,
}

impl ClipGraph {
    pub fn capacity_bytes(&self) -> usize {
        std::mem::size_of_val(&*self.originals)
            + std::mem::size_of_val(&*self.nodes)
            + std::mem::size_of_val(&*self.attributes)
            + std::mem::size_of_val(&*self.geometry)
            + std::mem::size_of_val(&*self.input)
            + std::mem::size_of_val(&*self.output)
            + std::mem::size_of_val(&*self.distances)
    }
    pub fn load(max_vertices: usize) -> Result<Self, &'static str> {
        let nodes = max_vertices
            .checked_mul(7)
            .ok_or("world clipping graph capacity")?;
        if max_vertices < 3 {
            return Err("world clipping graph vertex limit");
        }
        Ok(Self {
            originals: vec![Vertex::default(); max_vertices].into_boxed_slice(),
            nodes: vec![ClipRecord::default(); nodes].into_boxed_slice(),
            attributes: vec![ClipVertex::default(); nodes].into_boxed_slice(),
            geometry: vec![ScreenVertex::default(); max_vertices].into_boxed_slice(),
            input: vec![0; max_vertices].into_boxed_slice(),
            output: vec![0; max_vertices].into_boxed_slice(),
            distances: vec![0.0; max_vertices].into_boxed_slice(),
            node_count: 0,
            count: 0,
        })
    }

    pub fn sources(&mut self, count: usize) -> Option<&mut [Vertex]> {
        self.originals.get_mut(..count)
    }

    /// Source vertices are supplied through sources() before this call.
    pub fn build(
        &mut self,
        camera: &Camera,
        count: usize,
        deforms: [DeformOp; 3],
        base: Option<PreparedStage>,
        evaluator: &StageEvaluator,
    ) -> Option<usize> {
        if count > self.originals.len() {
            return None;
        }
        self.node_count = count;
        self.count = count;
        for index in 0..count {
            let source = self.originals[index];
            // Native indexed and precombined surfaces use the original source
            // positions. Generic stages share one deformation of each vertex.
            let original = if base.is_some() {
                evaluator.apply_deforms(deforms, source)
            } else {
                source
            };
            self.originals[index] = original;
            let vertex = evaluated_vertex(original, [DeformOp::None; 3], base, evaluator);
            let clip = camera.vertex(vertex, vertex.position);
            if !clip.finite() {
                return None;
            }
            self.nodes[index] = ClipRecord {
                operation: ClipOperation::Source(index),
                base: clip,
            };
            self.input[index] = index;
        }
        for plane in 0..6 {
            if self.count < 3 {
                self.count = 0;
                return Some(0);
            }
            // qsrc r_draw.c:268-277 leaves accepted edges untouched. Classify
            // only this plane's active vertices: earlier planes can remove a
            // source whose later distance overflows, or introduce new vertices.
            let mut inside_count = 0;
            for index in 0..self.count {
                let distance = camera.distance(self.nodes[self.input[index]].base, plane);
                if !distance.is_finite() {
                    return None;
                }
                self.distances[index] = distance;
                inside_count += usize::from(distance >= 0.0);
            }
            if inside_count == self.count {
                continue;
            }
            if inside_count == 0 {
                self.count = 0;
                return Some(0);
            }
            let mut out = 0;
            let mut previous = self.input[self.count - 1];
            let mut previous_distance = self.distances[self.count - 1];
            for index in 0..self.count {
                let current = self.input[index];
                let distance = self.distances[index];
                let inside = distance >= 0.0;
                if inside != (previous_distance >= 0.0) {
                    let denominator = previous_distance - distance;
                    if !denominator.is_finite()
                        || out == self.output.len()
                        || self.node_count == self.nodes.len()
                    {
                        return None;
                    }
                    let fraction = previous_distance / denominator;
                    let vertex = self.nodes[previous]
                        .base
                        .lerp(self.nodes[current].base, fraction);
                    if !vertex.finite() {
                        return None;
                    }
                    self.nodes[self.node_count] = ClipRecord {
                        operation: ClipOperation::Intersection {
                            left: previous,
                            right: current,
                            fraction,
                        },
                        base: vertex,
                    };
                    self.output[out] = self.node_count;
                    self.node_count += 1;
                    out += 1;
                }
                if inside {
                    if out == self.output.len() {
                        return None;
                    }
                    self.output[out] = current;
                    out += 1;
                }
                previous = current;
                previous_distance = distance;
            }
            self.count = out;
            std::mem::swap(&mut self.input, &mut self.output);
        }
        if self.count < 3 {
            self.count = 0;
        }
        Some(self.count)
    }

    pub fn project_base(
        &mut self,
        camera: &Camera,
        screen: &mut [ScreenVertex],
        coverage: &mut [ProjectedVertex],
    ) -> Option<usize> {
        if self.count > screen.len() || self.count > coverage.len() {
            return None;
        }
        for index in 0..self.count {
            let vertex = camera.project(self.nodes[self.input[index]].base);
            if !vertex.finite() {
                return None;
            }
            screen[index] = vertex;
            self.geometry[index] = vertex;
            coverage[index] = ProjectedVertex {
                xy: vertex.xy.map(|value| value - 0.5),
                inverse_depth: vertex.inverse_depth,
                texcoord_over_depth: vertex.texcoord_over_depth,
            };
        }
        Some(self.count)
    }

    #[expect(
        clippy::needless_range_loop,
        reason = "Screen, geometry and input attributes retain their distinct explicit index mappings"
    )]
    pub fn project_stage(
        &mut self,
        stage: PreparedStage,
        evaluator: &StageEvaluator,
        screen: &mut [ScreenVertex],
    ) -> Option<usize> {
        if self.count > screen.len() {
            return None;
        }
        for index in 0..self.node_count {
            let vertex = match self.nodes[index].operation {
                ClipOperation::Source(source) => {
                    let vertex = evaluated_vertex(
                        self.originals[source],
                        [DeformOp::None; 3],
                        Some(stage),
                        evaluator,
                    );
                    ClipVertex {
                        camera: self.nodes[index].base.camera,
                        texcoord: vertex.texcoord,
                        lightmap_coord: vertex.lightmap_coord,
                        color: vertex.color.map(|channel| channel as f32 / 255.0),
                    }
                }
                ClipOperation::Intersection {
                    left,
                    right,
                    fraction,
                } => self.attributes[left].lerp(self.attributes[right], fraction),
            };
            // Validate intermediate nodes too: a discarded vertex can overflow
            // during interpolation and the original clipper rejected it then.
            if !vertex.finite() {
                return None;
            }
            self.attributes[index] = vertex;
        }
        for index in 0..self.count {
            let attributes = self.attributes[self.input[index]];
            let geometry = self.geometry[index];
            let vertex = ScreenVertex {
                xy: geometry.xy,
                inverse_depth: geometry.inverse_depth,
                texcoord_over_depth: attributes
                    .texcoord
                    .map(|value| value * geometry.inverse_depth),
                lightmap_over_depth: attributes
                    .lightmap_coord
                    .map(|value| value * geometry.inverse_depth),
                color_over_depth: attributes.color.map(|value| value * geometry.inverse_depth),
            };
            if !vertex.finite() {
                return None;
            }
            screen[index] = vertex;
        }
        Some(self.count)
    }
}
