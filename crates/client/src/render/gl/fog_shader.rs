//! Q2 fog GLSL sources (donor `src/render/gl/fog-shader.ts`).

/// Fog pass selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FogPassKind {
    Global = 0,
    Height = 1,
    Sky = 2,
}

impl FogPassKind {
    pub const ALL: [FogPassKind; 3] = [FogPassKind::Global, FogPassKind::Height, FogPassKind::Sky];
}

/// Uniforms linked for `kind`.
#[must_use]
pub const fn fog_uniforms_for(kind: FogPassKind) -> &'static [&'static str] {
    match kind {
        FogPassKind::Sky => &["u_depth", "u_far_depth", "u_fog_color"],
        FogPassKind::Global => &["u_depth", "u_far_depth", "u_proj", "u_fog_color"],
        FogPassKind::Height => &[
            "u_depth",
            "u_far_depth",
            "u_proj",
            "u_vieworg",
            "u_forward",
            "u_right",
            "u_up",
            "u_tan",
            "u_hf_start",
            "u_hf_end",
            "u_hf_density",
            "u_hf_falloff",
        ],
    }
}

/// Vertex source for `kind`. The quad arrives in NDC directly, so `gl_Position`
/// is `gl_Vertex` with no matrix; the height pass interpolates the eye-space
/// ray exactly across the two triangles.
#[must_use]
pub fn build_fog_vertex_shader_source(kind: FogPassKind) -> String {
    let mut lines = vec!["#version 110".to_string(), "varying vec2 v_tc;".to_string()];
    if kind == FogPassKind::Height {
        lines.push("uniform vec3 u_forward;".to_string());
        lines.push("uniform vec3 u_right;".to_string());
        lines.push("uniform vec3 u_up;".to_string());
        lines.push("uniform vec4 u_tan;".to_string());
        lines.push("varying vec3 v_ray;".to_string());
    }
    lines.push("void main() {".to_string());
    lines.push("  v_tc = gl_MultiTexCoord0.st;".to_string());
    if kind == FogPassKind::Height {
        lines.push(
            "  v_ray = u_forward + u_right * (gl_Vertex.x * u_tan.x) + u_up * (gl_Vertex.y * u_tan.y);".to_string(),
        );
    }
    lines.push("  gl_Position = vec4(gl_Vertex.xy, 0.0, 1.0);".to_string());
    lines.push("}".to_string());
    lines.join("\n")
}

/// Fragment source for `kind`.
#[must_use]
pub fn build_fog_fragment_shader_source(kind: FogPassKind) -> String {
    let mut lines = vec![
        "#version 110".to_string(),
        "varying vec2 v_tc;".to_string(),
        "uniform sampler2D u_depth;".to_string(),
        "uniform float u_far_depth;".to_string(),
    ];
    if kind == FogPassKind::Sky {
        lines.push("uniform vec4 u_fog_color;".to_string());
        lines.push("void main() {".to_string());
        lines.push("  float d = texture2D(u_depth, v_tc).r;".to_string());
        lines.push("  if (d < u_far_depth) discard;".to_string());
        lines.push("  gl_FragColor = vec4(u_fog_color.rgb, u_fog_color.a);".to_string());
        lines.push("}".to_string());
        return lines.join("\n");
    }
    lines.push("uniform vec4 u_proj;".to_string());
    if kind == FogPassKind::Global {
        lines.push("uniform vec4 u_fog_color;".to_string());
    } else {
        lines.push("varying vec3 v_ray;".to_string());
        lines.push("uniform vec3 u_vieworg;".to_string());
        lines.push("uniform vec4 u_hf_start;".to_string());
        lines.push("uniform vec4 u_hf_end;".to_string());
        lines.push("uniform float u_hf_density;".to_string());
        lines.push("uniform float u_hf_falloff;".to_string());
    }
    lines.push("void main() {".to_string());
    lines.push("  float d = texture2D(u_depth, v_tc).r;".to_string());
    lines.push("  if (d >= u_far_depth) discard;".to_string());
    lines.push("  float ndc_z = 2.0 * d - 1.0;".to_string());
    lines.push("  float w = u_proj.y / (u_proj.x - ndc_z);".to_string());
    lines.push("  float frag_depth = d * w;".to_string());
    if kind == FogPassKind::Global {
        lines.push("  float dd = u_fog_color.a * frag_depth;".to_string());
        lines.push("  float fog = 1.0 - exp(-(dd * dd));".to_string());
        lines.push("  gl_FragColor = vec4(u_fog_color.rgb, fog);".to_string());
        lines.push("}".to_string());
        return lines.join("\n");
    }
    lines.push("  vec3 v_world_pos = u_vieworg + v_ray * w;".to_string());
    lines.push("  float dir_z = normalize(v_world_pos - u_vieworg).z;".to_string());
    lines.push("  float s = sign(dir_z);".to_string());
    lines.push("  dir_z += 0.00001 * (1.0 - s * s);".to_string());
    lines.push("  float eye = u_vieworg.z - u_hf_start.w;".to_string());
    lines.push("  float pos = v_world_pos.z - u_hf_start.w;".to_string());
    lines.push(
        "  float density = (exp(-u_hf_falloff * eye) - exp(-u_hf_falloff * pos)) / (u_hf_falloff * dir_z);".to_string(),
    );
    lines.push("  float extinction = 1.0 - clamp(exp(-density), 0.0, 1.0);".to_string());
    lines.push("  float fraction = clamp((pos - u_hf_start.w) / (u_hf_end.w - u_hf_start.w), 0.0, 1.0);".to_string());
    lines.push("  vec3 fog_color = mix(u_hf_start.rgb, u_hf_end.rgb, fraction) * extinction;".to_string());
    lines.push("  float fog = (1.0 - exp(-(u_hf_density * frag_depth))) * extinction;".to_string());
    lines.push("  gl_FragColor = vec4(fog_color, fog);".to_string());
    lines.push("}".to_string());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniforms_match_pass_kind() {
        assert_eq!(
            fog_uniforms_for(FogPassKind::Sky),
            &["u_depth", "u_far_depth", "u_fog_color"]
        );
        assert_eq!(fog_uniforms_for(FogPassKind::Global).len(), 4);
        assert_eq!(fog_uniforms_for(FogPassKind::Height).len(), 12);
        assert_eq!(FogPassKind::ALL.len(), 3);
    }

    #[test]
    fn height_pass_carries_ray_varying() {
        let vertex = build_fog_vertex_shader_source(FogPassKind::Height);
        assert!(vertex.contains("varying vec3 v_ray;"));
        assert!(vertex.contains("gl_Position = vec4(gl_Vertex.xy, 0.0, 1.0);"));
        let global = build_fog_vertex_shader_source(FogPassKind::Global);
        assert!(!global.contains("v_ray"));
    }

    #[test]
    fn fragment_passes_discard_opposite_depths() {
        let sky = build_fog_fragment_shader_source(FogPassKind::Sky);
        assert!(sky.contains("if (d < u_far_depth) discard;"));
        let global = build_fog_fragment_shader_source(FogPassKind::Global);
        assert!(global.contains("if (d >= u_far_depth) discard;"));
        assert!(global.contains("1.0 - exp(-(dd * dd))"));
        let height = build_fog_fragment_shader_source(FogPassKind::Height);
        assert!(height.contains("u_hf_falloff"));
        assert!(height.contains("mix(u_hf_start.rgb, u_hf_end.rgb, fraction)"));
    }
}
