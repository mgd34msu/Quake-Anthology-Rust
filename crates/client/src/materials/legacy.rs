//! Q1/Q2 classic materials: animation, liquids, sky, lightmap passes.
//!
//! Donor provenance: `src/materials/legacy.ts` (from Q1/Q2
//! `gl_rsurf.c`/`gl_warp.c`).

use std::collections::BTreeMap;

use qa_core::math::{Vec2, Vec3, Vec4};

use super::evaluate::{BatchLighting, BatchVertex, MaterialBatch, TextureRef, Texturing};
use super::geometry::MaterialGeometry;
use super::lighting::Q1LightmapEncoding;
use super::state::{AlphaTest, Blend, CullFace, DepthTest, RenderState};
use super::turbulence::{source_turbulence, TurbulenceProfile};
use crate::ClientError;

/// Q1 surface kind (`Q1Material["surface"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1Surface {
    /// Ordinary.
    Ordinary,
    /// Sky.
    Sky,
    /// Fence (alpha-masked).
    Fence,
    /// Water.
    Water,
    /// Slime.
    Slime,
    /// Lava.
    Lava,
    /// Teleport.
    Teleport,
}

/// Q1 material (`Q1Material`, image handles are `u32`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1Material {
    /// Name.
    pub name: String,
    /// Base texture handle.
    pub texture: u32,
    /// Lightmap handle (or `None`).
    pub lightmap: Option<u32>,
    /// Vertex-lit pass.
    pub vertex_lit: bool,
    /// Surface kind.
    pub surface: Q1Surface,
    /// Opacity.
    pub alpha: f32,
    /// Primary animation frames.
    pub animation: Vec<Q1AnimFrame>,
    /// Alternate animation frames.
    pub alternate_animation: Vec<Q1AnimFrame>,
}

/// One Q1 animation frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q1AnimFrame {
    /// Image handle.
    pub image: u32,
    /// Start in tenths.
    pub start_tenths: i32,
    /// End in tenths.
    pub end_tenths: i32,
}

/// Q2 material (`Q2Material`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2Material {
    /// Frame handles (non-empty).
    pub frames: Q2Frames,
    /// Lightmap handle (or `None`).
    pub lightmap: Option<u32>,
    /// Vertex-lit pass.
    pub vertex_lit: bool,
    /// Surface flags.
    pub surface_flags: u32,
    /// Flowing scroll.
    pub flowing: bool,
    /// Warp (liquid).
    pub warp: bool,
    /// Opacity.
    pub alpha: f32,
}

/// Q2 frame list with a static cap for `Copy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q2Frames {
    /// Frames.
    pub frames: [u32; 8],
    /// Frame count.
    pub count: usize,
}

/// Classify a Q1 surface (`q1SurfaceKind`).
#[must_use]
pub const fn q1_surface_kind(name: &str) -> Q1Surface {
    let bytes = name.as_bytes();
    if starts_with_bytes(bytes, "sky") {
        Q1Surface::Sky
    } else if starts_with_bytes(bytes, "{") {
        Q1Surface::Fence
    } else if !starts_with_bytes(bytes, "*") {
        Q1Surface::Ordinary
    } else if starts_with_bytes(bytes, "*lava") {
        Q1Surface::Lava
    } else if starts_with_bytes(bytes, "*slime") {
        Q1Surface::Slime
    } else if starts_with_bytes(bytes, "*tele") {
        Q1Surface::Teleport
    } else {
        Q1Surface::Water
    }
}

const fn starts_with_bytes(bytes: &[u8], prefix: &str) -> bool {
    let prefix = prefix.as_bytes();
    if bytes.len() < prefix.len() {
        return false;
    }
    let mut index = 0;
    while index < prefix.len() {
        if bytes[index] != prefix[index] {
            return false;
        }
        index += 1;
    }
    true
}

/// Create a Q1 material (`createQ1Material`).
#[must_use]
pub fn create_q1_material(
    name: &str,
    texture: u32,
    lightmap: Option<u32>,
    vertex_lit: bool,
    alpha: f32,
    animation: Vec<Q1AnimFrame>,
    alternate_animation: Vec<Q1AnimFrame>,
) -> Q1Material {
    Q1Material {
        name: name.to_string(),
        texture,
        lightmap,
        vertex_lit,
        surface: q1_surface_kind(name),
        alpha,
        animation,
        alternate_animation,
    }
}

/// Q1 texture animations (`q1TextureAnimations`, `+0..9`/`+A..J` loops).
pub fn q1_texture_animations(
    name: &str,
    textures: &[(&str, u32)],
) -> Result<(Vec<Q1AnimFrame>, Vec<Q1AnimFrame>), ClientError> {
    if !name.starts_with('+') {
        return Ok((Vec::new(), Vec::new()));
    }
    let mut primary: BTreeMap<u8, u32> = BTreeMap::new();
    let mut alternate: BTreeMap<u8, u32> = BTreeMap::new();
    for (texture_name, image) in textures {
        if !texture_name.starts_with('+') || texture_name.get(2..) != name.get(2..) {
            continue;
        }
        let code = texture_name.as_bytes()[1].to_ascii_uppercase();
        if (48..=57).contains(&code) {
            primary.insert(code - 48, *image);
        } else if (65..=74).contains(&code) {
            alternate.insert(code - 65, *image);
        } else {
            return Err(ClientError::BadMaterial(format!(
                "Bad Q1 animated texture frame {texture_name}"
            )));
        }
    }
    let sequence = |frames: &BTreeMap<u8, u32>| -> Result<Vec<Q1AnimFrame>, ClientError> {
        let count = frames.keys().next_back().map_or(0, |last| usize::from(*last) + 1);
        let mut result = Vec::with_capacity(count);
        for frame in 0..count {
            let image = frames
                .get(&(frame as u8))
                .copied()
                .ok_or_else(|| ClientError::BadMaterial(format!("Missing Q1 animation frame {frame} in {name}")))?;
            result.push(Q1AnimFrame {
                image,
                start_tenths: frame as i32 * 2,
                end_tenths: (frame + 1) as i32 * 2,
            });
        }
        Ok(result)
    };
    let animation = sequence(&primary)?;
    let alternate_animation = sequence(&alternate)?;
    if name.as_bytes()[1].to_ascii_uppercase() >= 65 {
        Ok((alternate_animation, animation))
    } else {
        Ok((animation, alternate_animation))
    }
}

/// Wrap a frame counter into `0..count` (`((value % count) + count) % count`).
///
/// Q1 tenth-phases and Q2 frame indices share this wrap; the pickers below
/// stay split because Q1 selects by tenth ranges while Q2 indexes directly.
fn wrap_animation_frame(value: i32, count: i32) -> i32 {
    (value % count + count) % count
}

/// Pick a Q1 animated texture (`q1AnimatedTexture`).
pub fn q1_animated_texture(material: &Q1Material, time: f32, alternate: bool) -> Result<u32, ClientError> {
    let frames = if alternate && !material.alternate_animation.is_empty() {
        &material.alternate_animation
    } else {
        &material.animation
    };
    let Some(last) = frames.last() else {
        return Ok(material.texture);
    };
    let phase = wrap_animation_frame((time * 10.0).trunc() as i32, last.end_tenths);
    frames
        .iter()
        .find(|frame| phase >= frame.start_tenths && phase < frame.end_tenths)
        .map(|frame| frame.image)
        .ok_or_else(|| ClientError::BadMaterial(format!("Broken animation cycle in {}", material.name)))
}

/// Create a Q2 material (`createQ2Material`).
pub fn create_q2_material(
    frames: &[u32],
    lightmap: Option<u32>,
    vertex_lit: bool,
    surface_flags: u32,
) -> Result<Q2Material, ClientError> {
    if frames.is_empty() || frames.len() > 8 {
        return Err(ClientError::BadMaterial(
            "Q2 material requires one to eight texture frames".to_string(),
        ));
    }
    let mut list = [0u32; 8];
    list[..frames.len()].copy_from_slice(frames);
    Ok(Q2Material {
        frames: Q2Frames {
            frames: list,
            count: frames.len(),
        },
        lightmap,
        vertex_lit,
        surface_flags,
        flowing: surface_flags & 64 != 0,
        warp: surface_flags & 8 != 0,
        alpha: if surface_flags & 16 != 0 {
            0.33
        } else if surface_flags & 32 != 0 {
            0.66
        } else {
            1.0
        },
    })
}

/// Pick a Q2 animated texture (`q2AnimatedTexture`).
pub fn q2_animated_texture(material: &Q2Material, frame: f32, name: &str) -> Result<u32, ClientError> {
    let count = material.frames.count as i32;
    let index = wrap_animation_frame(frame.trunc() as i32, count);
    material
        .frames
        .frames
        .get(index as usize)
        .copied()
        .ok_or_else(|| ClientError::BadMaterial(format!("Broken Q2 animation cycle in {name}")))
}

/// Liquid texture coordinates (`liquidTexCoords`, pre-tile UVs).
#[must_use]
pub fn liquid_tex_coords(uv: Vec2, time: f32, q2: bool, flowing: bool) -> Vec2 {
    let profile = if q2 {
        TurbulenceProfile::Q2
    } else {
        TurbulenceProfile::Q1
    };
    let scroll = if q2 && flowing {
        -64.0 * (time * 0.5 - (time * 0.5).trunc())
    } else {
        0.0
    };
    qa_core::math::vec2(
        (uv.x + source_turbulence(uv.y * 0.125 + time, profile) + scroll) / 64.0,
        (uv.y + source_turbulence(uv.x * 0.125 + time, profile)) / 64.0,
    )
}

/// Q2 flowing texture coordinates (`q2FlowingTexCoords`).
#[must_use]
pub fn q2_flowing_tex_coords(uv: Vec2, time: f32) -> Vec2 {
    let scroll = -64.0 * (time / 40.0 - (time / 40.0).trunc());
    qa_core::math::vec2(uv.x + if scroll == 0.0 { -64.0 } else { scroll }, uv.y)
}

/// Q1 sky layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1SkyLayer {
    /// Solid.
    Solid,
    /// Overlay.
    Overlay,
}

/// Q1 sky texture coordinates (`q1SkyTexCoords`).
#[must_use]
pub fn q1_sky_tex_coords(position: Vec3, view_origin: Vec3, time: f32, layer: Q1SkyLayer) -> Vec2 {
    let x = position.x - view_origin.x;
    let y = position.y - view_origin.y;
    let z = (position.z - view_origin.z) * 3.0;
    let scale = 378.0 / (x * x + y * y + z * z).sqrt();
    let mut scroll = time * if layer == Q1SkyLayer::Solid { 8.0 } else { 16.0 };
    scroll -= (scroll.trunc() as i32 & !127) as f32;
    qa_core::math::vec2((scroll + x * scale) / 128.0, (scroll + y * scale) / 128.0)
}

/// Legacy material draw context (`LegacyMaterialDrawContext`).
pub struct LegacyMaterialDrawContext<'a> {
    /// Entity RGBA tint as bytes-as-floats (0..255 per channel, donor
    /// `entityRGBA` scale); `None` means opaque white.
    pub entity_rgba: Option<Vec4>,
    /// Time.
    pub time: f32,
    /// Animation frame (Q2).
    pub animation_frame: f32,
    /// Alternate animation (Q1).
    pub alternate_animation: bool,
    /// Fullbright image.
    pub fullbright: Option<u32>,
    /// Q1 fog active.
    pub q1_fog_active: bool,
    /// Q1 lightmap encoding.
    pub q1_lightmap_encoding: Q1LightmapEncoding,
    /// Uploaded direct lightmap for translucent passes.
    pub translucent_lightmap: Option<u32>,
    /// Cull face.
    pub cull: CullFace,
    /// Depth range.
    pub depth_range: [f32; 2],
    /// Project callback.
    pub project: &'a dyn Fn(Vec3) -> Vec4,
}

/// A legacy material reference.
#[derive(Debug, Clone, PartialEq)]
pub enum LegacyMaterial {
    /// Q1.
    Q1(Q1Material),
    /// Q2 (with name for diagnostics).
    Q2 {
        /// Name.
        name: String,
        /// Material.
        material: Q2Material,
    },
}

/// Prepare legacy material batches (`prepareLegacyMaterialBatches`).
pub fn prepare_legacy_material_batches(
    material: &LegacyMaterial,
    geometry: &MaterialGeometry,
    context: &LegacyMaterialDrawContext,
) -> Result<Vec<MaterialBatch>, ClientError> {
    let is_sky = match material {
        LegacyMaterial::Q1(material) => material.surface == Q1Surface::Sky,
        LegacyMaterial::Q2 { material, .. } => material.surface_flags & 4 != 0,
    };
    if is_sky {
        return Err(ClientError::BadMaterial(
            "Sky surfaces require the sky geometry and layer path".to_string(),
        ));
    }
    let (image, alpha, vertex_lit, lightmap) = match material {
        LegacyMaterial::Q1(material) => (
            q1_animated_texture(material, context.time, context.alternate_animation)?,
            material.alpha,
            material.vertex_lit,
            material.lightmap,
        ),
        LegacyMaterial::Q2 { name, material } => (
            q2_animated_texture(material, context.animation_frame, name)?,
            material.alpha,
            material.vertex_lit,
            material.lightmap,
        ),
    };
    let tint = context
        .entity_rgba
        .unwrap_or(qa_core::math::vec4(255.0, 255.0, 255.0, 255.0));
    let alpha = alpha * tint.w / 255.0;
    let blended = alpha < 1.0;
    let fence = matches!(material, LegacyMaterial::Q1(material) if material.surface == Q1Surface::Fence);
    let state = RenderState {
        blend: if blended {
            Blend {
                source: super::state::BlendFactor::SrcAlpha,
                destination: super::state::BlendFactor::OneMinusSrcAlpha,
            }
        } else {
            super::state::OPAQUE_BLEND
        },
        depth_test: DepthTest::LessEqual,
        depth_write: !blended,
        alpha_test: if fence { AlphaTest::Gt0 } else { AlphaTest::None },
        cull: context.cull,
        depth_range: context.depth_range,
        polygon_offset: None,
    };
    let vertices: Vec<BatchVertex> = geometry
        .vertices
        .iter()
        .map(|vertex| {
            let mut uv = vertex.tex_coord;
            match material {
                LegacyMaterial::Q1(material) => match material.surface {
                    Q1Surface::Water | Q1Surface::Slime | Q1Surface::Lava | Q1Surface::Teleport => {
                        uv = liquid_tex_coords(uv, context.time, false, false);
                    }
                    _ => {}
                },
                LegacyMaterial::Q2 { material, .. } => {
                    if material.warp {
                        uv = liquid_tex_coords(uv, context.time, true, material.flowing);
                    } else if material.flowing {
                        uv = q2_flowing_tex_coords(uv, context.time);
                    }
                }
            }
            let unlit_brush = matches!(material, LegacyMaterial::Q2 { material, .. }
                if !material.vertex_lit && (material.warp || blended));
            let intensity = if unlit_brush { 0.5 } else { 1.0 };
            let color = if vertex_lit {
                qa_core::math::vec4(
                    f32::from(vertex.color[0]) / 255.0,
                    f32::from(vertex.color[1]) / 255.0,
                    f32::from(vertex.color[2]) / 255.0,
                    alpha,
                )
            } else {
                qa_core::math::vec4(intensity, intensity, intensity, alpha)
            };
            BatchVertex {
                position: (context.project)(vertex.position),
                tex_coord: uv,
                tex_coord2: None,
                color: qa_core::math::vec4(
                    color.x * tint.x / 255.0,
                    color.y * tint.y / 255.0,
                    color.z * tint.z / 255.0,
                    color.w,
                ),
            }
        })
        .collect();
    let mut batches = vec![MaterialBatch {
        lighting: BatchLighting::Vertex,
        fog: None,
        texturing: Texturing::Single,
        state,
        texture: TextureRef::BindImage(image),
        second_texture: None,
        indices: geometry.indices.clone(),
        vertices,
    }];
    if let Some(lightmap) = lightmap {
        if blended || context.q1_fog_active {
            let combined = context.translucent_lightmap.ok_or_else(|| {
                ClientError::BadMaterial(
                    "Translucent lightmapped surfaces require an uploaded directLightmapPixels image".to_string(),
                )
            })?;
            let paired: Vec<BatchVertex> = batches[0]
                .vertices
                .iter()
                .zip(geometry.vertices.iter())
                .map(|(vertex, source)| BatchVertex {
                    tex_coord2: Some(source.lightmap_coord),
                    ..*vertex
                })
                .collect();
            batches[0] = MaterialBatch {
                lighting: BatchLighting::Vertex,
                fog: None,
                texturing: Texturing::Pair,
                state: batches[0].state,
                texture: TextureRef::BindImage(image),
                second_texture: Some((TextureRef::BindImage(combined), super::evaluate::PairEnv::Modulate)),
                indices: geometry.indices.clone(),
                vertices: paired,
            };
        } else {
            let blend = match material {
                LegacyMaterial::Q1(_) if context.q1_lightmap_encoding != Q1LightmapEncoding::Rgb => Blend {
                    source: super::state::BlendFactor::Zero,
                    destination: if context.q1_lightmap_encoding == Q1LightmapEncoding::InvertedAlpha {
                        super::state::BlendFactor::OneMinusSrcAlpha
                    } else {
                        super::state::BlendFactor::OneMinusSrcColor
                    },
                },
                _ => Blend {
                    source: super::state::BlendFactor::DstColor,
                    destination: super::state::BlendFactor::Zero,
                },
            };
            let light_vertices: Vec<BatchVertex> = batches[0]
                .vertices
                .iter()
                .zip(geometry.vertices.iter())
                .map(|(vertex, source)| BatchVertex {
                    position: vertex.position,
                    tex_coord: source.lightmap_coord,
                    tex_coord2: None,
                    color: qa_core::math::vec4(1.0, 1.0, 1.0, 1.0),
                })
                .collect();
            batches.push(MaterialBatch {
                lighting: BatchLighting::Vertex,
                fog: None,
                texturing: Texturing::Single,
                texture: TextureRef::BindImage(lightmap),
                state: RenderState {
                    blend,
                    depth_test: DepthTest::Equal,
                    depth_write: false,
                    alpha_test: AlphaTest::None,
                    ..state
                },
                second_texture: None,
                indices: geometry.indices.clone(),
                vertices: light_vertices,
            });
        }
    }
    if let Some(fullbright) = context.fullbright {
        let bright: Vec<BatchVertex> = batches[0]
            .vertices
            .iter()
            .map(|vertex| BatchVertex {
                color: qa_core::math::vec4(tint.x / 255.0, tint.y / 255.0, tint.z / 255.0, alpha),
                ..*vertex
            })
            .collect();
        batches.push(MaterialBatch {
            lighting: BatchLighting::Vertex,
            fog: None,
            texturing: Texturing::Single,
            texture: TextureRef::BindImage(fullbright),
            state: RenderState {
                blend: Blend {
                    source: super::state::BlendFactor::SrcAlpha,
                    destination: super::state::BlendFactor::OneMinusSrcAlpha,
                },
                depth_test: if blended {
                    DepthTest::LessEqual
                } else {
                    DepthTest::Equal
                },
                depth_write: false,
                alpha_test: AlphaTest::Gt0,
                ..state
            },
            second_texture: None,
            indices: geometry.indices.clone(),
            vertices: bright,
        });
    }
    Ok(batches)
}

/// An indexed source image for sky splitting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedImage {
    /// Width.
    pub width: usize,
    /// Height.
    pub height: usize,
    /// Palette indices.
    pub pixels: Vec<u8>,
    /// RGB palette (768 bytes).
    pub palette: Vec<u8>,
}

/// Split RGBA layers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkyLayers {
    /// Solid layer RGBA.
    pub solid: Vec<u8>,
    /// Solid dimensions.
    pub solid_size: (usize, usize),
    /// Overlay layer RGBA.
    pub overlay: Vec<u8>,
    /// Overlay dimensions.
    pub overlay_size: (usize, usize),
}

/// Split a classic Q1 sky texture (`splitQ1SkyTexture`).
pub fn split_q1_sky_texture(image: &IndexedImage) -> Result<SkyLayers, ClientError> {
    if image.width != 256 || image.height != 128 {
        return Err(ClientError::BadMaterial(
            "Classic Q1 sky requires a 256x128 indexed texture".to_string(),
        ));
    }
    let color = |index: u8| -> Result<(u8, u8, u8), ClientError> {
        let offset = usize::from(index) * 3;
        match image.palette.get(offset..offset + 3) {
            Some(color) => Ok((color[0], color[1], color[2])),
            None => Err(ClientError::BadMaterial("Q1 sky palette is incomplete".to_string())),
        }
    };
    let mut solid = vec![0u8; 128 * 128 * 4];
    let mut overlay = vec![0u8; 128 * 128 * 4];
    let (mut red, mut green, mut blue) = (0u32, 0u32, 0u32);
    for y in 0..128 {
        for x in 0..128 {
            let index = image.pixels[y * 256 + x + 128];
            let (r, g, b) = color(index)?;
            solid[(y * 128 + x) * 4..(y * 128 + x) * 4 + 4].copy_from_slice(&[r, g, b, 255]);
            red += u32::from(r);
            green += u32::from(g);
            blue += u32::from(b);
        }
    }
    let average = [(red / 16384) as u8, (green / 16384) as u8, (blue / 16384) as u8, 0];
    for y in 0..128 {
        for x in 0..128 {
            let index = image.pixels[y * 256 + x];
            let slot = (y * 128 + x) * 4;
            if index == 0 {
                overlay[slot..slot + 4].copy_from_slice(&average);
            } else {
                let (r, g, b) = color(index)?;
                overlay[slot..slot + 4].copy_from_slice(&[r, g, b, 255]);
            }
        }
    }
    Ok(SkyLayers {
        solid,
        solid_size: (128, 128),
        overlay,
        overlay_size: (128, 128),
    })
}

/// Split a Quake64 sky texture (`splitQ64SkyTexture`).
pub fn split_q64_sky_texture(image: &IndexedImage) -> Result<SkyLayers, ClientError> {
    if image.height < 2 || !image.height.is_multiple_of(2) {
        return Err(ClientError::BadMaterial(
            "Quake64 sky requires two vertically stacked layers".to_string(),
        ));
    }
    let width = image.width;
    let height = image.height / 2;
    let count = width * height;
    let mut solid = vec![0u8; count * 4];
    let mut overlay = vec![0u8; count * 4];
    for index in 0..count {
        let (front, back) = (image.pixels[index], image.pixels[count + index]);
        for channel in 0..3 {
            let a = image.palette.get(usize::from(front) * 3 + channel).copied();
            let b = image.palette.get(usize::from(back) * 3 + channel).copied();
            match (a, b) {
                (Some(a), Some(b)) => {
                    overlay[index * 4 + channel] = a;
                    solid[index * 4 + channel] = b;
                }
                _ => {
                    return Err(ClientError::BadMaterial(
                        "Quake64 sky palette is incomplete".to_string(),
                    ));
                }
            }
        }
        overlay[index * 4 + 3] = 128;
        solid[index * 4 + 3] = 255;
    }
    Ok(SkyLayers {
        solid,
        solid_size: (width, height),
        overlay,
        overlay_size: (width, height),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surface_kinds() {
        assert_eq!(q1_surface_kind("sky1"), Q1Surface::Sky);
        assert_eq!(q1_surface_kind("{fence"), Q1Surface::Fence);
        assert_eq!(q1_surface_kind("*lava1"), Q1Surface::Lava);
        assert_eq!(q1_surface_kind("*slime"), Q1Surface::Slime);
        assert_eq!(q1_surface_kind("*tele"), Q1Surface::Teleport);
        assert_eq!(q1_surface_kind("*water"), Q1Surface::Water);
        assert_eq!(q1_surface_kind("rock"), Q1Surface::Ordinary);
    }

    #[test]
    fn animations_form_tenth_loops() {
        let (animation, alternate) =
            q1_texture_animations("+0lava", &[("+0lava", 1), ("+1lava", 2), ("+Alava", 3)]).unwrap();
        assert_eq!(animation.len(), 2);
        assert_eq!(animation[1].end_tenths, 4);
        assert_eq!(alternate.len(), 1);
    }

    #[test]
    fn missing_frame_is_an_error() {
        assert!(q1_texture_animations("+0lava", &[("+0lava", 1), ("+2lava", 2)]).is_err());
    }

    #[test]
    fn animation_wrap_matches_both_pickers() {
        assert_eq!(wrap_animation_frame(7, 4), 3);
        assert_eq!(wrap_animation_frame(-1, 4), 3);
        assert_eq!(wrap_animation_frame(-9, 4), 3);
        let q1 = create_q1_material(
            "+0lava",
            9,
            None,
            false,
            1.0,
            vec![
                Q1AnimFrame {
                    image: 1,
                    start_tenths: 0,
                    end_tenths: 2,
                },
                Q1AnimFrame {
                    image: 2,
                    start_tenths: 2,
                    end_tenths: 4,
                },
            ],
            Vec::new(),
        );
        assert_eq!(q1_animated_texture(&q1, -0.1, false).unwrap(), 2);
        assert_eq!(q1_animated_texture(&q1, 0.25, false).unwrap(), 2);
        let q2 = create_q2_material(&[10, 11, 12], None, false, 0).unwrap();
        assert_eq!(q2_animated_texture(&q2, -1.0, "lava").unwrap(), 12);
        assert_eq!(q2_animated_texture(&q2, 4.0, "lava").unwrap(), 11);
    }

    #[test]
    fn q2_alpha_from_flags() {
        assert_eq!(create_q2_material(&[1], None, false, 16).unwrap().alpha, 0.33);
        assert_eq!(create_q2_material(&[1], None, false, 0).unwrap().alpha, 1.0);
        assert!(create_q2_material(&[], None, false, 0).is_err());
    }

    #[test]
    fn q1_sky_splits() {
        let image = IndexedImage {
            width: 256,
            height: 128,
            pixels: vec![1; 256 * 128],
            palette: vec![7; 768],
        };
        let layers = split_q1_sky_texture(&image).unwrap();
        assert_eq!(layers.solid.len(), 128 * 128 * 4);
        assert_eq!(layers.solid[0..3], [7, 7, 7]);
    }

    #[test]
    fn legacy_batches_cover_lightmap() {
        let material = create_q1_material("rock", 1, Some(2), false, 1.0, Vec::new(), Vec::new());
        let project = |position: Vec3| qa_core::math::vec4(position.x, position.y, position.z, 1.0);
        let context = LegacyMaterialDrawContext {
            entity_rgba: None,
            time: 0.0,
            animation_frame: 0.0,
            alternate_animation: false,
            fullbright: None,
            q1_fog_active: false,
            q1_lightmap_encoding: Q1LightmapEncoding::Rgb,
            translucent_lightmap: None,
            cull: CullFace::Front,
            depth_range: [0.0, 1.0],
            project: &project,
        };
        let geometry = MaterialGeometry {
            vertices: vec![super::super::geometry::MaterialVertex::new(
                qa_core::math::vec3(0.0, 0.0, 0.0),
                qa_core::math::vec3(0.0, 0.0, 1.0),
                qa_core::math::vec2(0.0, 0.0),
                qa_core::math::vec2(0.0, 0.0),
                [255, 255, 255, 255],
            )],
            indices: vec![0],
        };
        let batches = prepare_legacy_material_batches(&LegacyMaterial::Q1(material), &geometry, &context).unwrap();
        assert_eq!(batches.len(), 2);
    }
}
