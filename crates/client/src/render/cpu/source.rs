//! Source renderer compatibility shims.
//!
//! Donor provenance: `src/render/cpu/source.ts` in full — translated from
//! id Software's `renderer/tr_shade.c` `R_DrawElements`/`R_DrawStripElements`.
//! Copyright (C) 1999-2005 Id Software, Inc.

use qa_core::math::{Vec2, Vec4};

use super::super::types::{DrawBatch, TextureBinding};

/// Source primitive submission mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourcePrimitiveMode {
    /// Indexed elements.
    Elements,
    /// Client-array strips.
    ArrayStrips,
    /// Discrete strips.
    DiscreteStrips,
    /// No primitives.
    None,
}

/// Map the requested source primitive mode.
#[must_use]
pub const fn source_primitive_mode(requested: u32, compiled_arrays: bool) -> SourcePrimitiveMode {
    match requested {
        0 => {
            if compiled_arrays {
                SourcePrimitiveMode::Elements
            } else {
                SourcePrimitiveMode::ArrayStrips
            }
        }
        1 => SourcePrimitiveMode::ArrayStrips,
        2 => SourcePrimitiveMode::Elements,
        3 => SourcePrimitiveMode::DiscreteStrips,
        _ => SourcePrimitiveMode::None,
    }
}

/// Receiver for one emitted triangle strip.
pub trait SourceStripEmitter {
    /// Begin a strip.
    fn begin(&mut self);
    /// Emit one strip element.
    fn element(&mut self, index: u32);
    /// End a strip.
    fn end(&mut self);
}

/// Emit index triples as source triangle strips.
pub fn emit_source_triangle_strips(indices: &[u32], emit: &mut impl SourceStripEmitter) {
    if indices.is_empty() {
        return;
    }
    if !indices.len().is_multiple_of(3) {
        panic!("source triangle strips require complete index triples");
    }
    let (mut last_a, mut last_b, mut last_c) = (indices[0], indices[1], indices[2]);
    emit.begin();
    emit.element(last_a);
    emit.element(last_b);
    emit.element(last_c);
    let mut even = false;
    for triple in indices[3..].as_chunks::<3>().0 {
        let (a, b, c) = (triple[0], triple[1], triple[2]);
        let continues = if even {
            a == last_a && b == last_c
        } else {
            a == last_c && b == last_b
        };
        if continues {
            emit.element(c);
            even = !even;
        } else {
            emit.end();
            emit.begin();
            emit.element(a);
            emit.element(b);
            emit.element(c);
            even = false;
        }
        (last_a, last_b, last_c) = (a, b, c);
    }
    emit.end();
}

/// Source tess arrays at one draw call, indexed like batch vertices.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceStageCell {
    /// Stage color.
    pub color: Vec4,
    /// Stage coordinates.
    pub tex_coord: Vec2,
    /// Secondary stage coordinates.
    pub tex_coord2: Vec2,
    /// Raw coordinates.
    pub raw_tex_coord: Vec2,
    /// Raw secondary coordinates.
    pub raw_tex_coord2: Vec2,
}

/// Source stage kind selecting coordinate sourcing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceStageKind {
    /// Generic single-textured stage.
    GenericSingle,
    /// Vertex-lit stage.
    VertexLit,
    /// Dynamic-light stage.
    Dlight,
    /// Fog stage.
    Fog,
    /// Generic paired stage.
    GenericPair,
    /// Lightmapped paired stage.
    LightmappedPair,
}

/// Source stage data: state bits, batch, and scratch tess arrays.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceStageData {
    /// Source state bits.
    pub state_bits: u32,
    /// Stage kind.
    pub kind: SourceStageKind,
    /// Published batch.
    pub batch: DrawBatch,
    /// Scratch tess cells matching published vertices.
    pub scratch: Vec<SourceStageCell>,
}

/// Prepared source draw: begin, bind, draw, release.
pub trait PreparedSourceDraw {
    /// Begin the draw.
    fn begin(&mut self);
    /// Prepare a texture slot.
    fn prepare_texture(&mut self, unit: u32);
    /// Bind a texture slot.
    fn apply_texture(&mut self, unit: u32, operation: &TextureBinding);
    /// Finish texture binding.
    fn finish_textures(&mut self);
    /// Issue the draw for one primitive mode request.
    fn draw(&mut self, primitives: u32);
    /// Release transient state.
    fn cleanup(&mut self);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primitive_modes_match_source_requests() {
        assert_eq!(source_primitive_mode(0, true), SourcePrimitiveMode::Elements);
        assert_eq!(source_primitive_mode(0, false), SourcePrimitiveMode::ArrayStrips);
        assert_eq!(source_primitive_mode(1, true), SourcePrimitiveMode::ArrayStrips);
        assert_eq!(source_primitive_mode(2, false), SourcePrimitiveMode::Elements);
        assert_eq!(source_primitive_mode(3, false), SourcePrimitiveMode::DiscreteStrips);
        assert_eq!(source_primitive_mode(9, true), SourcePrimitiveMode::None);
    }

    #[test]
    fn continuing_triples_extend_one_strip() {
        struct Recorder {
            strips: Vec<Vec<u32>>,
        }
        impl SourceStripEmitter for Recorder {
            fn begin(&mut self) {
                self.strips.push(Vec::new());
            }
            fn element(&mut self, index: u32) {
                self.strips.last_mut().expect("strip begun").push(index);
            }
            fn end(&mut self) {}
        }
        let mut recorder = Recorder { strips: Vec::new() };
        // Second triple continues (c, b) order; third restarts the strip.
        emit_source_triangle_strips(&[0, 1, 2, 2, 1, 3, 7, 8, 9], &mut recorder);
        assert_eq!(recorder.strips, vec![vec![0, 1, 2, 3], vec![7, 8, 9]]);
    }

    #[test]
    fn empty_indices_emit_nothing() {
        struct Recorder {
            begun: bool,
        }
        impl SourceStripEmitter for Recorder {
            fn begin(&mut self) {
                self.begun = true;
            }
            fn element(&mut self, _index: u32) {}
            fn end(&mut self) {}
        }
        let mut recorder = Recorder { begun: false };
        emit_source_triangle_strips(&[], &mut recorder);
        assert!(!recorder.begun);
    }
}
