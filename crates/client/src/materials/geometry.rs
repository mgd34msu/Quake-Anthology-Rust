//! Material geometry: source Q3 vertex storage and deform state.
//!
//! Donor provenance: `src/materials/geometry.ts` (`MaterialVertex`,
//! `MaterialGeometry`, `MaterialDeformState`). Vertices keep source Q3
//! byte colors until stage evaluation returns normalized colors.

use qa_core::math::{Vec2, Vec3};

/// Source Q3 vertex with byte colors (0..255).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MaterialVertex {
    /// Position.
    pub position: Vec3,
    /// Normal.
    pub normal: Vec3,
    /// Texture coordinates.
    pub tex_coord: Vec2,
    /// Lightmap coordinates.
    pub lightmap_coord: Vec2,
    /// Byte color `[r, g, b, a]`.
    pub color: [u8; 4],
}

impl MaterialVertex {
    /// Build a vertex from parts.
    #[must_use]
    pub const fn new(
        position: Vec3,
        normal: Vec3,
        tex_coord: Vec2,
        lightmap_coord: Vec2,
        color: [u8; 4],
    ) -> Self {
        Self {
            position,
            normal,
            tex_coord,
            lightmap_coord,
            color,
        }
    }
}

/// Indexed material geometry.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MaterialGeometry {
    /// Vertices.
    pub vertices: Vec<MaterialVertex>,
    /// Triangle indices.
    pub indices: Vec<u32>,
}

impl MaterialGeometry {
    /// Empty geometry.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }
}

/// Mutable deform working state (`MaterialDeformState`).
#[derive(Debug, Clone)]
pub struct MaterialDeformState {
    geometry: MaterialGeometry,
    /// Reference definition time in milliseconds.
    pub refdef_time: f32,
    /// Render text rows for `deformVertexes text`.
    pub render_text: Vec<String>,
    /// Begun source material name.
    pub material: Option<String>,
}

impl MaterialDeformState {
    /// New deform state over owned geometry.
    #[must_use]
    pub fn new(
        geometry: MaterialGeometry,
        refdef_time: f32,
        render_text: Vec<String>,
        material: Option<String>,
    ) -> Self {
        Self {
            geometry,
            refdef_time,
            render_text,
            material,
        }
    }

    /// Snapshot the current geometry.
    #[must_use]
    pub fn snapshot_geometry(&self) -> MaterialGeometry {
        self.geometry.clone()
    }

    /// Replace the working geometry.
    pub fn replace_geometry(&mut self, geometry: MaterialGeometry) {
        self.geometry = geometry;
    }

    /// Clear the working geometry.
    pub fn reset_geometry(&mut self) {
        self.geometry = MaterialGeometry::empty();
    }

    /// Source quad for text deformation (errors unless four vertices).
    pub fn text_quad(&self) -> Result<[MaterialVertex; 4], crate::ClientError> {
        if self.geometry.vertices.len() < 4 {
            return Err(crate::ClientError::BadMaterial(
                "Text deformation requires a four-vertex source quad".to_string(),
            ));
        }
        Ok([
            self.geometry.vertices[0],
            self.geometry.vertices[1],
            self.geometry.vertices[2],
            self.geometry.vertices[3],
        ])
    }

    /// Append stamped geometry with rebiased indices.
    pub fn append_geometry(&mut self, geometry: &MaterialGeometry) {
        let offset = self.geometry.vertices.len() as u32;
        self.geometry.vertices.extend_from_slice(&geometry.vertices);
        self.geometry
            .indices
            .extend(geometry.indices.iter().map(|index| offset + index));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::math::{vec2, vec3};

    fn vertex() -> MaterialVertex {
        MaterialVertex::new(
            vec3(1.0, 2.0, 3.0),
            vec3(0.0, 0.0, 1.0),
            vec2(0.0, 0.0),
            vec2(0.0, 0.0),
            [255, 255, 255, 255],
        )
    }

    #[test]
    fn append_rebases_indices() {
        let mut state = MaterialDeformState::new(
            MaterialGeometry {
                vertices: vec![vertex()],
                indices: vec![0],
            },
            0.0,
            Vec::new(),
            None,
        );
        state.append_geometry(&MaterialGeometry {
            vertices: vec![vertex(), vertex()],
            indices: vec![0, 1, 0],
        });
        assert_eq!(state.snapshot_geometry().vertices.len(), 3);
        assert_eq!(state.snapshot_geometry().indices, vec![0, 1, 2, 1]);
    }

    #[test]
    fn text_quad_needs_four_vertices() {
        let state = MaterialDeformState::new(MaterialGeometry::empty(), 0.0, Vec::new(), None);
        assert!(state.text_quad().is_err());
    }
}
