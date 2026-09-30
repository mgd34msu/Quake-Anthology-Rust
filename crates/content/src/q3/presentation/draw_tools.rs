//! Quake III presentation: draw tools.
//!
//! Donor provenance: `src/content/q3/presentation/draw-tools.ts`.

use qa_core::math::{vec4, Vec4};

// Intra-group imports: sibling modules split from the same flat port.
pub use super::mirrors_present_hud::{banner_string_width, proportional_string_width};
use crate::q3::presentation::mirrors_present_hud::*;

/// Visible length without color escapes (`drawStrlen`).
#[must_use]
pub fn draw_strlen(text: &str) -> i32 {
    let units: Vec<char> = text.chars().collect();
    let mut end = units.len();
    for (index, unit) in units.iter().enumerate() {
        if *unit == '\0' {
            end = index;
            break;
        }
        if *unit as u32 > 255 {
            panic!("Cgame text requires byte characters");
        }
    }
    let mut count = 0i32;
    let mut index = 0usize;
    while index < end {
        if units[index] == '^' && index + 1 < units.len() && units[index + 1] != '\0' && units[index + 1] != '^' {
            index += 1;
        } else {
            count += 1;
        }
        index += 1;
    }
    count
}

/// Proportional size scale (`proportionalSizeScale`).
#[must_use]
pub fn proportional_size_scale(style: i32) -> f32 {
    if style & UI_SMALLFONT != 0 {
        0.75
    } else {
        1.0
    }
}

/// White with fade alpha (`fadeColor`).
#[must_use]
pub fn fade_color(time: i32, start_msec: i32, total_msec: f32) -> Option<Vec4> {
    let total = total_msec as i32;
    let elapsed = time.wrapping_sub(start_msec);
    if start_msec == 0 || elapsed >= total {
        return None;
    }
    let remaining = total.wrapping_sub(elapsed);
    Some(vec4(
        1.0,
        1.0,
        1.0,
        if remaining < 200 { remaining as f32 / 200.0 } else { 1.0 },
    ))
}

/// Team color (`teamColor`).
#[must_use]
pub fn team_color(team: i32) -> Vec4 {
    if team == Team::Red as i32 {
        vec4(1.0, 0.2, 0.2, 1.0)
    } else if team == Team::Blue as i32 {
        vec4(0.2, 0.2, 1.0, 1.0)
    } else if team == Team::Spectator as i32 {
        vec4(0.7, 0.7, 0.7, 1.0)
    } else {
        vec4(1.0, 1.0, 1.0, 1.0)
    }
}

/// Health color (`getColorForHealth`).
#[must_use]
pub fn get_color_for_health(health: i32, armor: i32) -> Vec4 {
    if health <= 0 {
        return vec4(0.0, 0.0, 0.0, 1.0);
    }
    let maximum = (f64::from(health) * ARMOR_PROTECTION / (1.0 - ARMOR_PROTECTION)) as i32;
    let health = health.wrapping_add(armor.min(maximum));
    vec4(
        1.0,
        if health > 60 {
            1.0
        } else if health < 30 {
            0.0
        } else {
            (health - 30) as f32 / 30.0
        },
        if health >= 100 {
            1.0
        } else if health < 66 {
            0.0
        } else {
            (health - 66) as f32 / 33.0
        },
        1.0,
    )
}

/// Snapshot health color (`colorForHealth`).
#[must_use]
pub fn color_for_health(state: &Shared<ClientGameState>) -> Vec4 {
    let snapshot = state.borrow().snap.clone();
    let Some(snapshot) = snapshot else {
        panic!("CG_ColorForHealth requires a current snapshot");
    };
    let schema = stat_schema(snapshot.player_state.product);
    get_color_for_health(
        snapshot.player_state.stats.get(schema.health),
        snapshot.player_state.stats.get(schema.armor),
    )
}

/// Cgame draw recorder (`ClientDrawTools`).
#[derive(Clone)]
pub struct ClientDrawTools {
    /// Drawing context.
    pub draw: Draw2D,
    /// Media.
    pub media: Shared<ClientMedia>,
}

impl ClientDrawTools {
    /// Assemble tools, requiring stretch-640 coordinates.
    pub fn new(draw: Draw2D, media: Shared<ClientMedia>) -> Self {
        if draw.space != CoordinateSpace::Stretch640 {
            panic!("Cgame drawing requires stretch-640 coordinates");
        }
        Self { draw, media }
    }

    /// Picture for a shader.
    #[must_use]
    pub fn picture(&self, shader: &Option<SceneShader>) -> Picture {
        self.media.borrow().resources.borrow().picture(shader)
    }

    /// Legacy font pictures.
    fn legacy_fonts(&self) -> LegacyFonts {
        let media = self.media.borrow();
        let resources = media.resources.borrow();
        LegacyFonts {
            charset: resources.picture(&media.graphics.charset_shader),
            proportional: resources.picture(&media.graphics.charset_prop),
            glow: resources.picture(&media.graphics.charset_prop_glow),
            banner: resources.picture(&media.graphics.charset_prop_b),
        }
    }

    /// Draw a proportional string (`drawProportionalString`).
    pub fn draw_proportional_string(&self, options: &UiTextOptions) {
        draw_cg_proportional_string(&self.draw, &self.legacy_fonts(), options);
    }

    /// Draw a banner string (`drawBannerString`).
    pub fn draw_banner_string(&self, options: &UiTextOptions) {
        draw_cg_banner_string(&self.draw, &self.legacy_fonts(), options);
    }

    /// Adjust from 640 space (`adjustFrom640`).
    #[must_use]
    pub fn adjust_from_640(&self, rect: Rect2d) -> Rect2d {
        self.draw.adjust(rect)
    }

    /// Fill a rectangle (`fillRect`).
    pub fn fill_rect(&self, rect: Rect2d, color: Option<Vec4>) {
        let picture = {
            let media = self.media.borrow();
            let resources = media.resources.borrow();
            resources.picture(&media.graphics.white_shader)
        };
        self.draw.set_color(color);
        self.draw.stretch_pic(rect, ZERO_UV, picture);
        self.draw.set_color(None);
    }

    /// Draw side borders (`drawSides`).
    pub fn draw_sides(&self, rect: Rect2d, size: f32) {
        let adjusted = self.adjust_from_640(rect);
        let width = size * self.draw.scale_x();
        let picture = {
            let media = self.media.borrow();
            let resources = media.resources.borrow();
            resources.picture(&media.graphics.white_shader)
        };
        self.draw.stretch_pixels(Rect2d { width, ..adjusted }, ZERO_UV, picture);
        self.draw.stretch_pixels(
            Rect2d {
                x: adjusted.x + adjusted.width - width,
                width,
                ..adjusted
            },
            ZERO_UV,
            picture,
        );
    }

    /// Draw top/bottom borders (`drawTopBottom`).
    pub fn draw_top_bottom(&self, rect: Rect2d, size: f32) {
        let adjusted = self.adjust_from_640(rect);
        let height = size * self.draw.scale_y();
        let picture = {
            let media = self.media.borrow();
            let resources = media.resources.borrow();
            resources.picture(&media.graphics.white_shader)
        };
        self.draw
            .stretch_pixels(Rect2d { height, ..adjusted }, ZERO_UV, picture);
        self.draw.stretch_pixels(
            Rect2d {
                y: adjusted.y + adjusted.height - height,
                height,
                ..adjusted
            },
            ZERO_UV,
            picture,
        );
    }

    /// Draw a rectangle border (`drawRect`).
    pub fn draw_rect(&self, rect: Rect2d, size: f32, color: Option<Vec4>) {
        self.draw.set_color(color);
        self.draw_top_bottom(rect, size);
        self.draw_sides(rect, size);
        self.draw.set_color(None);
    }

    /// Draw a picture (`drawPic`).
    pub fn draw_pic(&self, rect: Rect2d, shader: &Option<SceneShader>) {
        let picture = self.picture(shader);
        self.draw.draw_pic(rect, picture);
    }

    /// Draw a character (`drawChar`).
    pub fn draw_char(&self, x: f32, y: f32, width: f32, height: f32, code: u32) {
        if code & 255 == 32 {
            return;
        }
        let charset = {
            let media = self.media.borrow();
            let resources = media.resources.borrow();
            resources.picture(&media.graphics.charset_shader)
        };
        draw_char(&self.draw, charset, x, y, width, height, code);
    }

    /// Draw an extended string (`drawStringExt`).
    pub fn draw_string_ext(&self, options: &FixedTextOptions) {
        let charset = {
            let media = self.media.borrow();
            let resources = media.resources.borrow();
            resources.picture(&media.graphics.charset_shader)
        };
        draw_cg_string(&self.draw, charset, options);
    }

    /// Draw a big string (`drawBigString`).
    pub fn draw_big_string(&self, x: i32, y: i32, text: &str, alpha: f32) {
        self.draw_string_ext(&FixedTextOptions {
            x: x as f32,
            y: y as f32,
            text: text.to_string(),
            color: vec4(1.0, 1.0, 1.0, alpha),
            force_color: false,
            shadow: true,
            char_width: 16,
            char_height: 16,
            max_chars: 0,
        });
    }

    /// Draw a big colored string (`drawBigStringColor`).
    pub fn draw_big_string_color(&self, x: i32, y: i32, text: &str, color: Vec4) {
        self.draw_string_ext(&FixedTextOptions {
            x: x as f32,
            y: y as f32,
            text: text.to_string(),
            color,
            force_color: true,
            shadow: true,
            char_width: 16,
            char_height: 16,
            max_chars: 0,
        });
    }

    /// Draw a small string (`drawSmallString`).
    pub fn draw_small_string(&self, x: i32, y: i32, text: &str, alpha: f32) {
        self.draw_string_ext(&FixedTextOptions {
            x: x as f32,
            y: y as f32,
            text: text.to_string(),
            color: vec4(1.0, 1.0, 1.0, alpha),
            force_color: false,
            shadow: false,
            char_width: 8,
            char_height: 16,
            max_chars: 0,
        });
    }

    /// Draw a small colored string (`drawSmallStringColor`).
    pub fn draw_small_string_color(&self, x: i32, y: i32, text: &str, color: Vec4) {
        self.draw_string_ext(&FixedTextOptions {
            x: x as f32,
            y: y as f32,
            text: text.to_string(),
            color,
            force_color: true,
            shadow: false,
            char_width: 8,
            char_height: 16,
            max_chars: 0,
        });
    }

    /// Tile one clear box (`tileClearBox`).
    fn tile_clear_box(&self, x: f32, y: f32, width: f32, height: f32) {
        let picture = {
            let media = self.media.borrow();
            let resources = media.resources.borrow();
            resources.picture(&media.graphics.back_tile_shader)
        };
        self.draw.stretch_pixels(
            rect2d(x, y, width, height),
            TextureRect {
                s: x / 64.0,
                t: y / 64.0,
                s2: (x + width) / 64.0,
                t2: (y + height) / 64.0,
            },
            picture,
        );
    }

    /// Tile-clear around a viewport (`tileClear`).
    pub fn tile_clear(&self, refdef: &Refdef) {
        let width = self.draw.width();
        let height = self.draw.height();
        if refdef.x == 0 && refdef.y == 0 && refdef.width == width && refdef.height == height {
            return;
        }
        let top = refdef.y;
        let bottom = top + refdef.height - 1;
        let left = refdef.x;
        let right = left + refdef.width - 1;
        self.tile_clear_box(0.0, 0.0, width as f32, top as f32);
        self.tile_clear_box(0.0, bottom as f32, width as f32, (height - bottom) as f32);
        self.tile_clear_box(0.0, top as f32, left as f32, (bottom - top + 1) as f32);
        self.tile_clear_box(
            right as f32,
            top as f32,
            (width - right) as f32,
            (bottom - top + 1) as f32,
        );
    }
}
