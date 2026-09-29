//! Shadow receiver sampling (donor `src/render/gl/shadow-shader.ts`).

/// Which receiver the shadow factor snippet shades.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadowReceiver {
    World,
    Model,
}

/// GLSL lines computing `lit` (1.0 unoccluded) for light `i`, using the cone
/// PCF and 3x2 cube-face atlas layout with the donor's depth biases.
#[must_use]
pub fn shadow_factor_lines(receiver: ShadowReceiver) -> Vec<String> {
    let cone_bias = match receiver {
        ShadowReceiver::World => "0.0005",
        ShadowReceiver::Model => "0.0025",
    };
    let (base_bias, axial_scale) = match receiver {
        ShadowReceiver::World => ("1.0", "2.0"),
        ShadowReceiver::Model => ("5.0", "6.0"),
    };
    vec![
      "      float lit = 1.0;".to_string(),
      "      vec2 rect_lo = u_light_atlas[i].xy;".to_string(),
      "      vec2 rect_size = u_light_atlas[i].zw;".to_string(),
      "      if (u_light_shadow[i] < 1.5) {".to_string(),
      "        vec4 lpos = u_light_matrix[i] * vec4(worldPosition, 1.0);".to_string(),
      "        if (lpos.w > 0.0) {".to_string(),
      "          vec3 lproj = lpos.xyz / lpos.w;".to_string(),
      "          if (lproj.x >= 0.0 && lproj.x <= 1.0 && lproj.y >= 0.0 && lproj.y <= 1.0 && lproj.z <= 1.0) {".to_string(),
      "            vec2 base = lproj.xy * rect_size + rect_lo;".to_string(),
      "            vec2 tap_lo = rect_lo + u_shadow_texel;".to_string(),
      "            vec2 tap_hi = rect_lo + rect_size - u_shadow_texel;".to_string(),
      "            lit = 0.0;".to_string(),
      "            for (int sy = 0; sy < 2; sy++) {".to_string(),
      "              for (int sx = 0; sx < 2; sx++) {".to_string(),
      "                vec2 off = (vec2(float(sx), float(sy)) - 0.5) * u_shadow_texel;".to_string(),
      "                float d = texture2D(u_shadow_map, clamp(base + off, tap_lo, tap_hi)).r;".to_string(),
      format!("                lit += (lproj.z - {cone_bias}) > d ? 0.0 : 1.0;"),
      "              }".to_string(),
      "            }".to_string(),
      "            lit *= 0.25;".to_string(),
      "          }".to_string(),
      "        }".to_string(),
      "      } else {".to_string(),
      "        vec3 lvec = worldPosition - u_light_pos[i];".to_string(),
      "        vec3 lmag = abs(lvec);".to_string(),
      "        vec3 face_f;".to_string(),
      "        vec3 face_r;".to_string(),
      "        vec3 face_u;".to_string(),
      "        float face;".to_string(),
      "        if (lmag.x >= lmag.y && lmag.x >= lmag.z) {".to_string(),
      "          if (lvec.x >= 0.0) { face_f = vec3(1.0, 0.0, 0.0); face_r = vec3(0.0, -1.0, 0.0); face_u = vec3(0.0, 0.0, 1.0); face = 0.0; }".to_string(),
      "          else { face_f = vec3(-1.0, 0.0, 0.0); face_r = vec3(0.0, 1.0, 0.0); face_u = vec3(0.0, 0.0, 1.0); face = 1.0; }".to_string(),
      "        } else if (lmag.y >= lmag.z) {".to_string(),
      "          if (lvec.y >= 0.0) { face_f = vec3(0.0, 1.0, 0.0); face_r = vec3(1.0, 0.0, 0.0); face_u = vec3(0.0, 0.0, 1.0); face = 2.0; }".to_string(),
      "          else { face_f = vec3(0.0, -1.0, 0.0); face_r = vec3(-1.0, 0.0, 0.0); face_u = vec3(0.0, 0.0, 1.0); face = 3.0; }".to_string(),
      "        } else {".to_string(),
      "          if (lvec.z >= 0.0) { face_f = vec3(0.0, 0.0, 1.0); face_r = vec3(0.0, 1.0, 0.0); face_u = vec3(1.0, 0.0, 0.0); face = 4.0; }".to_string(),
      "          else { face_f = vec3(0.0, 0.0, -1.0); face_r = vec3(0.0, -1.0, 0.0); face_u = vec3(1.0, 0.0, 0.0); face = 5.0; }".to_string(),
      "        }".to_string(),
      "        float axial = dot(lvec, face_f);".to_string(),
      "        if (axial > SHADOW_NEAR) {".to_string(),
      "          float zfar = max(u_light_radius[i], SHADOW_NEAR * 2.0);".to_string(),
      "          float pa = (zfar + SHADOW_NEAR) / (SHADOW_NEAR - zfar);".to_string(),
      "          float pb = (2.0 * zfar * SHADOW_NEAR) / (SHADOW_NEAR - zfar);".to_string(),
      "          vec2 cell_size = rect_size / vec2(SHADOW_CUBE_COLS, SHADOW_CUBE_ROWS);".to_string(),
      "          vec2 cell_lo = rect_lo + vec2(mod(face, SHADOW_CUBE_COLS), floor(face / SHADOW_CUBE_COLS)) * cell_size;".to_string(),
      "          vec2 face_uv = vec2(dot(lvec, face_r), dot(lvec, face_u)) / axial * 0.5 + 0.5;".to_string(),
      "          vec2 base = cell_lo + face_uv * cell_size;".to_string(),
      "          vec2 tap_lo = cell_lo + u_shadow_texel;".to_string(),
      "          vec2 tap_hi = cell_lo + cell_size - u_shadow_texel;".to_string(),
      "          float face_texels = cell_size.x / u_shadow_texel;".to_string(),
      format!("          float bias = {base_bias} + axial * (2.0 / face_texels) * {axial_scale};"),
      "          lit = 0.0;".to_string(),
      "          for (int sy = 0; sy < 2; sy++) {".to_string(),
      "            for (int sx = 0; sx < 2; sx++) {".to_string(),
      "              vec2 off = (vec2(float(sx), float(sy)) - 0.5) * u_shadow_texel;".to_string(),
      "              float d = texture2D(u_shadow_map, clamp(base + off, tap_lo, tap_hi)).r;".to_string(),
      "              float stored = pb / ((2.0 * d - 1.0) + pa);".to_string(),
      "              lit += (axial - bias) > stored ? 0.0 : 1.0;".to_string(),
      "            }".to_string(),
      "          }".to_string(),
      "          lit *= 0.25;".to_string(),
      "        }".to_string(),
      "      }".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receivers_differ_only_in_biases() {
        let world = shadow_factor_lines(ShadowReceiver::World);
        let model = shadow_factor_lines(ShadowReceiver::Model);
        assert_eq!(world.len(), model.len());
        assert!(world.len() > 60);
        let differing: Vec<usize> = world
            .iter()
            .zip(model.iter())
            .enumerate()
            .filter_map(|(index, (a, b))| (a != b).then_some(index))
            .collect();
        assert_eq!(differing.len(), 2);
        assert!(world.join("\n").contains("lproj.z - 0.0005"));
        assert!(model.join("\n").contains("lproj.z - 0.0025"));
        assert!(model.join("\n").contains("float bias = 5.0"));
    }

    #[test]
    fn snippet_computes_lit_for_cone_and_cube() {
        let source = shadow_factor_lines(ShadowReceiver::World).join("\n");
        assert!(source.contains("float lit = 1.0;"));
        assert!(source.contains("u_light_matrix[i]"));
        assert!(source.contains("SHADOW_CUBE_COLS"));
        assert!(source.contains("lit *= 0.25;"));
    }
}
