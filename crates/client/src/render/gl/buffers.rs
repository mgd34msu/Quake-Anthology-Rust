//! Client-array geometry packing (donor `src/render/gl/buffers.ts`).

use crate::render::error::RenderError;
use crate::render::types::{BatchLighting, BatchPrimitive, BatchVertices, DrawBatch};

/// Packed client-array storage for one prepared draw.
#[derive(Debug, Clone, Default)]
pub struct GeometryArrays {
    pub positions: Vec<f32>,
    pub colors: Vec<f32>,
    pub coordinates: Vec<f32>,
    pub coordinates2: Vec<f32>,
    pub world_positions: Vec<f32>,
    pub normals: Vec<f32>,
    pub indices: Vec<u32>,
}

fn lighting_channels(lighting: &BatchLighting) -> (usize, usize) {
    match lighting {
        BatchLighting::Vertex => (0, 0),
        BatchLighting::Q2World { .. } => (3, 3),
        BatchLighting::Q2ModelShadow { .. } => (3, 0),
    }
}

/// Pack `batch` into fresh vectors.
#[must_use]
pub fn pack_geometry(batch: &DrawBatch) -> GeometryArrays {
    let mut buffer = GeometryBuffer::new();
    buffer.pack(batch);
    std::mem::take(&mut buffer.arrays)
}

/// Reusable packing storage; one buffer serves one prepared draw at a time and
/// returns to the renderer's idle slot on cleanup.
#[derive(Debug, Default)]
pub struct GeometryBuffer {
    arrays: GeometryArrays,
}

impl GeometryBuffer {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Pack `batch`, reusing this buffer's allocations.
    pub fn pack(&mut self, batch: &DrawBatch) {
        let (vertex_count, paired) = match &batch.vertices {
            BatchVertices::Single(vertices) => (vertices.len(), false),
            BatchVertices::Pair { vertices, .. } => (vertices.len(), true),
        };
        let (world, normal) = lighting_channels(&batch.lighting);
        self.arrays.positions.resize(vertex_count * 4, 0.0);
        self.arrays.colors.resize(vertex_count * 4, 0.0);
        self.arrays.coordinates.resize(vertex_count * 2, 0.0);
        self.arrays.coordinates2.resize(vertex_count * 2, 0.0);
        self.arrays.world_positions.resize(vertex_count * world, 0.0);
        self.arrays.normals.resize(vertex_count * normal, 0.0);
        self.arrays.indices.resize(batch.indices.len(), 0);
        pack_into(batch, vertex_count, paired, &mut self.arrays);
    }

    #[must_use]
    pub fn arrays(&self) -> &GeometryArrays {
        &self.arrays
    }
}

fn pack_into(batch: &DrawBatch, vertex_count: usize, paired: bool, arrays: &mut GeometryArrays) {
    let size = match batch.primitive {
        BatchPrimitive::Triangles => 3,
        BatchPrimitive::Lines { line_width } => {
            if !line_width.is_finite() || line_width <= 0.0 {
                panic!(
                    "{}",
                    RenderError::BadBatch {
                        index: 0,
                        detail: "OpenGL line width must be positive and finite".to_string()
                    }
                );
            }
            2
        }
    };
    if !batch.indices.len().is_multiple_of(size) || batch.indices.len() > 0x7FFF_FFFF {
        panic!(
            "{}",
            RenderError::BadBatch {
                index: batch.indices.len(),
                detail: "OpenGL primitive index count is invalid".to_string()
            }
        );
    }
    if vertex_count > 0x1FFF_FFFF {
        panic!(
            "{}",
            RenderError::BadBatch {
                index: vertex_count,
                detail: "OpenGL vertex allocation is too large".to_string()
            }
        );
    }
    for (slot, index) in batch.indices.iter().enumerate() {
        if (*index as usize) >= vertex_count {
            panic!(
                "{}",
                RenderError::BadBatch {
                    index: slot,
                    detail: "OpenGL vertex index is outside its allocation".to_string()
                }
            );
        }
        arrays.indices[slot] = *index;
    }
    if !paired {
        arrays.coordinates2.fill(0.0);
    }
    match &batch.vertices {
        BatchVertices::Single(vertices) => {
            for (slot, vertex) in vertices.iter().enumerate() {
                let four = slot * 4;
                let two = slot * 2;
                arrays.positions[four] = vertex.position.x;
                arrays.positions[four + 1] = vertex.position.y;
                arrays.positions[four + 2] = vertex.position.z;
                arrays.positions[four + 3] = vertex.position.w;
                arrays.colors[four] = vertex.color.x;
                arrays.colors[four + 1] = vertex.color.y;
                arrays.colors[four + 2] = vertex.color.z;
                arrays.colors[four + 3] = vertex.color.w;
                arrays.coordinates[two] = vertex.tex_coord.x;
                arrays.coordinates[two + 1] = vertex.tex_coord.y;
            }
        }
        BatchVertices::Pair { vertices, .. } => {
            for (slot, vertex) in vertices.iter().enumerate() {
                let four = slot * 4;
                let two = slot * 2;
                arrays.positions[four] = vertex.base.position.x;
                arrays.positions[four + 1] = vertex.base.position.y;
                arrays.positions[four + 2] = vertex.base.position.z;
                arrays.positions[four + 3] = vertex.base.position.w;
                arrays.colors[four] = vertex.base.color.x;
                arrays.colors[four + 1] = vertex.base.color.y;
                arrays.colors[four + 2] = vertex.base.color.z;
                arrays.colors[four + 3] = vertex.base.color.w;
                arrays.coordinates[two] = vertex.base.tex_coord.x;
                arrays.coordinates[two + 1] = vertex.base.tex_coord.y;
                arrays.coordinates2[two] = vertex.tex_coord2.x;
                arrays.coordinates2[two + 1] = vertex.tex_coord2.y;
            }
        }
    }
    match &batch.lighting {
        BatchLighting::Vertex => {}
        BatchLighting::Q2World {
            world_positions,
            normals,
            ..
        } => {
            if world_positions.len() != vertex_count {
                panic!(
                    "{}",
                    RenderError::BadBatch {
                        index: world_positions.len(),
                        detail: "Q2 world positions must match the draw's vertex count".to_string()
                    }
                );
            }
            if normals.len() != vertex_count {
                panic!(
                    "{}",
                    RenderError::BadBatch {
                        index: normals.len(),
                        detail: "Q2 normals must match the draw's vertex count".to_string()
                    }
                );
            }
            for (slot, position) in world_positions.iter().enumerate() {
                arrays.world_positions[slot * 3] = position.x;
                arrays.world_positions[slot * 3 + 1] = position.y;
                arrays.world_positions[slot * 3 + 2] = position.z;
            }
            for (slot, normal) in normals.iter().enumerate() {
                arrays.normals[slot * 3] = normal.x;
                arrays.normals[slot * 3 + 1] = normal.y;
                arrays.normals[slot * 3 + 2] = normal.z;
            }
        }
        BatchLighting::Q2ModelShadow { world_positions, .. } => {
            if world_positions.len() != vertex_count {
                panic!(
                    "{}",
                    RenderError::BadBatch {
                        index: world_positions.len(),
                        detail: "Q2 world positions must match the draw's vertex count".to_string()
                    }
                );
            }
            for (slot, position) in world_positions.iter().enumerate() {
                arrays.world_positions[slot * 3] = position.x;
                arrays.world_positions[slot * 3 + 1] = position.y;
                arrays.world_positions[slot * 3 + 2] = position.z;
            }
        }
    }
    let attributes: [&[f32]; 6] = [
        &arrays.positions,
        &arrays.colors,
        &arrays.coordinates,
        if paired { &arrays.coordinates2 } else { &[] },
        &arrays.world_positions,
        &arrays.normals,
    ];
    for values in attributes {
        for (index, value) in values.iter().enumerate() {
            if !value.is_finite() {
                panic!(
                    "{}",
                    RenderError::BadBatch {
                        index,
                        detail: "OpenGL attributes must be finite float32 values".to_string()
                    }
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::types::{
        BatchLighting, BatchPrimitive, BatchVertices, CullFace, DrawBatch, RenderState, RenderVertex, TextureBinding,
    };
    use qa_core::math::{vec2, vec4};

    fn batch(vertices: usize, indices: Vec<u32>) -> DrawBatch {
        DrawBatch {
            fog: None,
            luminance_alpha: false,
            indices,
            texture: TextureBinding::RetainCurrentTexture,
            state: RenderState::opaque(CullFace::Back),
            lighting: BatchLighting::Vertex,
            primitive: BatchPrimitive::Triangles,
            vertices: BatchVertices::Single(vec![
                RenderVertex {
                    position: vec4(0.0, 0.0, 0.0, 1.0),
                    tex_coord: vec2(0.0, 0.0),
                    color: vec4(1.0, 1.0, 1.0, 1.0)
                };
                vertices
            ]),
        }
    }

    #[test]
    fn packs_single_textured_triangle() {
        let arrays = pack_geometry(&batch(3, vec![0, 1, 2]));
        assert_eq!(arrays.positions.len(), 12);
        assert_eq!(arrays.indices, vec![0, 1, 2]);
        assert!(arrays.coordinates2.iter().all(|value| *value == 0.0));
        assert!(arrays.world_positions.is_empty());
    }

    #[test]
    fn geometry_buffer_repacks() {
        let mut buffer = GeometryBuffer::new();
        buffer.pack(&batch(3, vec![0, 1, 2]));
        assert_eq!(buffer.arrays().indices.len(), 3);
        buffer.pack(&batch(4, vec![0, 1, 2]));
        assert_eq!(buffer.arrays().positions.len(), 16);
    }

    #[test]
    #[should_panic(expected = "outside its allocation")]
    fn rejects_out_of_range_index() {
        let _ = pack_geometry(&batch(2, vec![0, 1, 5]));
    }

    #[test]
    #[should_panic(expected = "line width")]
    fn rejects_bad_line_width() {
        let mut bad = batch(2, vec![0, 1]);
        bad.primitive = BatchPrimitive::Lines { line_width: 0.0 };
        let _ = pack_geometry(&bad);
    }
}
