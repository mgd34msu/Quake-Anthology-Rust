//! Quake III presentation: draw tools.
//!
//! Donor provenance: `src/content/q3/presentation/draw-tools.ts`.

use qa_core::math::{vec4, Vec4};
use std::rc::Rc;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::shared::definitions::*;
use crate::q3::presentation::hud::ClientMedia;
use crate::q3::presentation::hud::Shared;
use crate::q3::presentation::refdef::Refdef;
use crate::q3::presentation::retail_snapshot::SceneShader;
use crate::q3::presentation::state::*;

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
    if team == Team::TeamRed as i32 {
        vec4(1.0, 0.2, 0.2, 1.0)
    } else if team == Team::TeamBlue as i32 {
        vec4(0.2, 0.2, 1.0, 1.0)
    } else if team == Team::TeamSpectator as i32 {
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
    let schema = stat_schema(snapshot.player_state.product());
    get_color_for_health(
        snapshot.player_state.stats.get(schema.health()),
        snapshot.player_state.stats.get(schema.armor()),
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
        self.media
            .borrow()
            .resources
            .borrow()
            .picture(shader.as_ref())
            .map(|material| Picture { order: material.id })
            .unwrap_or(ZERO_PICTURE)
    }

    /// Legacy font pictures.
    fn legacy_fonts(&self) -> LegacyFonts {
        let media = self.media.borrow();
        let resources = media.resources.borrow();
        LegacyFonts {
            charset: resources
                .picture(media.graphics.charset_shader.as_ref())
                .map(|material| Picture { order: material.id })
                .unwrap_or(ZERO_PICTURE),
            proportional: resources
                .picture(media.graphics.charset_prop.as_ref())
                .map(|material| Picture { order: material.id })
                .unwrap_or(ZERO_PICTURE),
            glow: resources
                .picture(media.graphics.charset_prop_glow.as_ref())
                .map(|material| Picture { order: material.id })
                .unwrap_or(ZERO_PICTURE),
            banner: resources
                .picture(media.graphics.charset_prop_b.as_ref())
                .map(|material| Picture { order: material.id })
                .unwrap_or(ZERO_PICTURE),
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
            resources
                .picture(media.graphics.white_shader.as_ref())
                .map(|material| Picture { order: material.id })
                .unwrap_or(ZERO_PICTURE)
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
            resources
                .picture(media.graphics.white_shader.as_ref())
                .map(|material| Picture { order: material.id })
                .unwrap_or(ZERO_PICTURE)
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
            resources
                .picture(media.graphics.white_shader.as_ref())
                .map(|material| Picture { order: material.id })
                .unwrap_or(ZERO_PICTURE)
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
            resources
                .picture(media.graphics.charset_shader.as_ref())
                .map(|material| Picture { order: material.id })
                .unwrap_or(ZERO_PICTURE)
        };
        draw_char(&self.draw, charset, x, y, width, height, code);
    }

    /// Draw an extended string (`drawStringExt`).
    pub fn draw_string_ext(&self, options: &FixedTextOptions) {
        let charset = {
            let media = self.media.borrow();
            let resources = media.resources.borrow();
            resources
                .picture(media.graphics.charset_shader.as_ref())
                .map(|material| Picture { order: material.id })
                .unwrap_or(ZERO_PICTURE)
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
            resources
                .picture(media.graphics.back_tile_shader.as_ref())
                .map(|material| Picture { order: material.id })
                .unwrap_or(ZERO_PICTURE)
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

/// 2D rectangle (`Rect` / `UiRect`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect2d {
    /// X.
    pub x: f32,
    /// Y.
    pub y: f32,
    /// Width.
    pub width: f32,
    /// Height.
    pub height: f32,
}

/// Build a rectangle.
#[must_use]
pub fn rect2d(x: f32, y: f32, width: f32, height: f32) -> Rect2d {
    Rect2d { x, y, width, height }
}

/// UI rectangle (same shape as [`Rect2d`]).
pub type UiRect = Rect2d;

/// Texture coordinates (`TextureRect`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TextureRect {
    /// S.
    pub s: f32,
    /// T.
    pub t: f32,
    /// S end.
    pub s2: f32,
    /// T end.
    pub t2: f32,
}

/// Zero UV rectangle.
pub const ZERO_UV: TextureRect = TextureRect {
    s: 0.0,
    t: 0.0,
    s2: 0.0,
    t2: 0.0,
};

/// Full UV rectangle.
pub const FULL_UV: TextureRect = TextureRect {
    s: 0.0,
    t: 0.0,
    s2: 1.0,
    t2: 1.0,
};

/// Registered renderer picture (`PictureAsset`); `order` is the source handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Picture {
    /// Material order / handle.
    pub order: u32,
}

/// Zero (null-material) picture.
pub const ZERO_PICTURE: Picture = Picture { order: 0 };

/// Left text alignment (`UI_LEFT`).
pub const UI_LEFT: i32 = 0;

/// Centered text (`UI_CENTER`).
pub const UI_CENTER: i32 = 1;

/// Right-aligned text (`UI_RIGHT`).
pub const UI_RIGHT: i32 = 2;

/// Small font style (`UI_SMALLFONT`).
pub const UI_SMALLFONT: i32 = 0x10;

/// Giant font style (`UI_GIANTFONT`).
pub const UI_GIANTFONT: i32 = 0x40;

/// Drop shadow style (`UI_DROPSHADOW`).
pub const UI_DROPSHADOW: i32 = 0x800;

/// Blink style (`UI_BLINK`).
pub const UI_BLINK: i32 = 0x1000;

/// Inverse style (`UI_INVERSE`).
pub const UI_INVERSE: i32 = 0x2000;

/// Pulse style (`UI_PULSE`).
pub const UI_PULSE: i32 = 0x4000;

/// Source color escape table (`COLORS`).
pub(crate) const ESCAPE_COLORS: [Vec4; 8] = [
    Vec4 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        w: 1.0,
    },
    Vec4 {
        x: 1.0,
        y: 0.0,
        z: 0.0,
        w: 1.0,
    },
    Vec4 {
        x: 0.0,
        y: 1.0,
        z: 0.0,
        w: 1.0,
    },
    Vec4 {
        x: 1.0,
        y: 1.0,
        z: 0.0,
        w: 1.0,
    },
    Vec4 {
        x: 0.0,
        y: 0.0,
        z: 1.0,
        w: 1.0,
    },
    Vec4 {
        x: 0.0,
        y: 1.0,
        z: 1.0,
        w: 1.0,
    },
    Vec4 {
        x: 1.0,
        y: 0.0,
        z: 1.0,
        w: 1.0,
    },
    Vec4 {
        x: 1.0,
        y: 1.0,
        z: 1.0,
        w: 1.0,
    },
];

/// Validate 8-bit source text cut at NUL (`byteText`).
pub fn byte_text(text: &str) -> String {
    let cut = match text.find('\0') {
        Some(end) => &text[..end],
        None => text,
    };
    for ch in cut.chars() {
        if ch as u32 > 255 {
            panic!("Quake text requires an 8-bit source string");
        }
    }
    cut.to_string()
}

/// Whether a `^` escape starts at an index (`escapeAt`).
pub(crate) fn escape_at(units: &[char], index: usize) -> bool {
    units.get(index) == Some(&'^') && index + 1 < units.len() && units[index + 1] != '^'
}

/// Resolve an escape color (`escapeColor`).
pub(crate) fn escape_color(code: u32, alpha: f32) -> Vec4 {
    let color = ESCAPE_COLORS[((code.wrapping_sub(48)) & 7) as usize];
    Vec4 { w: alpha, ..color }
}

/// Black with an alpha (`black`).
pub(crate) fn black(alpha: f32) -> Vec4 {
    vec4(0.0, 0.0, 0.0, alpha)
}

/// Glyph metrics (`GlyphMetrics`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GlyphMetrics {
    /// Height.
    pub height: i32,
    /// Top.
    pub top: i32,
    /// Bottom.
    pub bottom: i32,
    /// Pitch.
    pub pitch: i32,
    /// Horizontal advance.
    pub x_skip: i32,
    /// Image width.
    pub image_width: i32,
    /// Image height.
    pub image_height: i32,
    /// S.
    pub s: f32,
    /// T.
    pub t: f32,
    /// S end.
    pub s2: f32,
    /// T end.
    pub t2: f32,
    /// Shader name.
    pub shader_name: String,
}

/// Registered glyph with its picture (`RegisteredGlyph`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RegisteredGlyph {
    /// Metrics.
    pub metrics: GlyphMetrics,
    /// Picture, when registered.
    pub picture: Option<Picture>,
}

/// Registered font (`RegisteredFont`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RegisteredFont {
    /// Name.
    pub name: String,
    /// Glyph scale.
    pub glyph_scale: f32,
    /// Glyphs.
    pub glyphs: Vec<RegisteredGlyph>,
}

/// Zero font with 256 empty glyphs (mission-hud `zeroFont`).
#[must_use]
pub fn zero_font() -> RegisteredFont {
    RegisteredFont {
        name: String::new(),
        glyph_scale: 0.0,
        glyphs: vec![RegisteredGlyph::default(); 256],
    }
}

/// Font profile (`"ui" | "cgame"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FontProfile {
    /// UI menus.
    #[default]
    Ui,
    /// Cgame HUD.
    Cgame,
}

/// Scalable font set (`FontSet`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FontSet {
    /// Small font.
    pub small: RegisteredFont,
    /// Normal font.
    pub normal: RegisteredFont,
    /// Big font.
    pub big: RegisteredFont,
    /// Profile.
    pub profile: FontProfile,
    /// Small threshold.
    pub small_threshold: f32,
    /// Big threshold.
    pub big_threshold: f32,
}

/// Zero cgame font set.
#[must_use]
pub fn zero_cgame_fonts() -> FontSet {
    FontSet {
        small: zero_font(),
        normal: zero_font(),
        big: zero_font(),
        profile: FontProfile::Cgame,
        small_threshold: 0.0,
        big_threshold: 0.0,
    }
}

/// Legacy fixed/atlas fonts (`LegacyFonts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyFonts {
    /// Charset picture.
    pub charset: Picture,
    /// Proportional picture.
    pub proportional: Picture,
    /// Glow picture.
    pub glow: Picture,
    /// Banner picture.
    pub banner: Picture,
}

/// Fixed text options (`FixedTextOptions`).
#[derive(Debug, Clone, PartialEq)]
pub struct FixedTextOptions {
    /// X.
    pub x: f32,
    /// Y.
    pub y: f32,
    /// Text.
    pub text: String,
    /// Color.
    pub color: Vec4,
    /// Character width.
    pub char_width: i32,
    /// Character height.
    pub char_height: i32,
    /// Maximum characters (`<= 0` means unbounded).
    pub max_chars: i32,
    /// Force the color through escapes.
    pub force_color: bool,
    /// Shadow pass.
    pub shadow: bool,
}

/// Proportional/banner text options (`UiTextOptions`).
#[derive(Debug, Clone, PartialEq)]
pub struct UiTextOptions {
    /// X.
    pub x: f32,
    /// Y.
    pub y: f32,
    /// Text.
    pub text: String,
    /// Color.
    pub color: Vec4,
    /// Style bits.
    pub style: i32,
    /// Time for pulse.
    pub time: i32,
}

/// Scalable text paint options (`TextPaintOptions`).
#[derive(Debug, Clone, PartialEq)]
pub struct TextPaintOptions {
    /// X.
    pub x: f32,
    /// Y.
    pub y: f32,
    /// Scale.
    pub scale: f32,
    /// Color.
    pub color: Vec4,
    /// Text.
    pub text: String,
    /// Extra advance.
    pub adjust: f32,
    /// Character limit (`<= 0` means unbounded).
    pub limit: i32,
    /// Style.
    pub style: i32,
}

/// Atlas glyph metric: x, y, advance (`AtlasMetric`).
pub type AtlasMetric = [i32; 3];

/// Invalid metric.
pub(crate) const INVALID_METRIC: AtlasMetric = [0, 0, -1];

/// Proportional ASCII atlas (`PROP_ASCII`).
pub(crate) const PROP_ASCII: [AtlasMetric; 65] = [
    [0, 0, 8],
    [11, 122, 7],
    [154, 181, 14],
    [55, 122, 17],
    [79, 122, 18],
    [101, 122, 23],
    [153, 122, 18],
    [9, 93, 7],
    [207, 122, 8],
    [230, 122, 9],
    [177, 122, 18],
    [30, 152, 18],
    [85, 181, 7],
    [34, 93, 11],
    [110, 181, 6],
    [130, 152, 14],
    [22, 64, 17],
    [41, 64, 12],
    [58, 64, 17],
    [78, 64, 18],
    [98, 64, 19],
    [120, 64, 18],
    [141, 64, 18],
    [204, 64, 16],
    [162, 64, 17],
    [182, 64, 18],
    [59, 181, 7],
    [35, 181, 7],
    [203, 152, 14],
    [56, 93, 14],
    [228, 152, 14],
    [177, 181, 18],
    [28, 122, 22],
    [5, 4, 18],
    [27, 4, 18],
    [48, 4, 18],
    [69, 4, 17],
    [90, 4, 13],
    [106, 4, 13],
    [121, 4, 18],
    [143, 4, 17],
    [164, 4, 8],
    [175, 4, 16],
    [195, 4, 18],
    [216, 4, 12],
    [230, 4, 23],
    [6, 34, 18],
    [27, 34, 18],
    [48, 34, 18],
    [68, 34, 18],
    [90, 34, 17],
    [110, 34, 18],
    [130, 34, 14],
    [146, 34, 18],
    [166, 34, 19],
    [185, 34, 29],
    [215, 34, 18],
    [234, 34, 18],
    [5, 64, 14],
    [60, 152, 7],
    [106, 151, 13],
    [83, 152, 7],
    [128, 122, 17],
    [4, 152, 21],
    [134, 181, 5],
];

/// Proportional atlas tail (`PROP_END`).
pub(crate) const PROP_END: [AtlasMetric; 4] = [[153, 152, 13], [11, 181, 5], [180, 152, 13], [79, 93, 17]];

/// Banner atlas (`BANNER`).
pub(crate) const BANNER: [AtlasMetric; 26] = [
    [11, 12, 33],
    [49, 12, 31],
    [85, 12, 31],
    [120, 12, 30],
    [156, 12, 21],
    [183, 12, 21],
    [207, 12, 32],
    [13, 55, 30],
    [49, 55, 13],
    [66, 55, 29],
    [101, 55, 31],
    [135, 55, 21],
    [158, 55, 40],
    [204, 55, 32],
    [12, 97, 31],
    [48, 97, 31],
    [82, 97, 30],
    [118, 97, 30],
    [153, 97, 30],
    [185, 97, 25],
    [213, 97, 30],
    [11, 139, 32],
    [42, 139, 51],
    [93, 139, 32],
    [126, 139, 31],
    [158, 139, 25],
];

/// Proportional glyph metric (`propMetric`).
#[must_use]
pub fn prop_metric(code: u32) -> AtlasMetric {
    let mut ch = code & 127;
    if ch < 32 || ch == 127 {
        return INVALID_METRIC;
    }
    if (97..=122).contains(&ch) {
        ch -= 32;
    }
    if ch >= 123 {
        PROP_END[(ch - 123) as usize]
    } else {
        PROP_ASCII[(ch - 32) as usize]
    }
}

/// Banner glyph metric (`bannerMetric`).
pub(crate) fn banner_metric(code: u32) -> AtlasMetric {
    if !(65..=90).contains(&code) {
        panic!("Invalid banner glyph index");
    }
    BANNER[(code - 65) as usize]
}

/// Proportional string width (`proportionalStringWidth`).
#[must_use]
pub fn proportional_string_width(input: &str) -> i32 {
    let text = byte_text(input);
    let mut width: i32 = 0;
    for ch in text.chars() {
        let metric = prop_metric(ch as u32);
        if metric[2] != -1 {
            width += metric[2] + 3;
        }
    }
    width - 3
}

/// Banner string width (`bannerStringWidth`).
#[must_use]
pub fn banner_string_width(input: &str) -> i32 {
    let text = byte_text(input);
    let mut width: i32 = 0;
    for ch in text.chars() {
        let code = ch as u32;
        if code == 32 {
            width += 12;
        } else if (65..=90).contains(&code) {
            width += banner_metric(code)[2] + 4;
        }
    }
    width - 4
}

/// Aligned X for a style (`alignedX`).
pub(crate) fn aligned_x(x: f32, width: i32, style: i32) -> i32 {
    x.trunc() as i32
        - if style & 7 == UI_CENTER {
            (width as f32 / 2.0).trunc() as i32
        } else if style & 7 == UI_RIGHT {
            width
        } else {
            0
        }
}

/// Pulse alpha (`pulse`).
pub(crate) fn pulse(time: i32) -> f32 {
    0.5 + 0.5 * ((time / 75) as f32).sin()
}

/// Select a font by scale (`selectFont`).
pub(crate) fn select_font(fonts: &FontSet, scale: f32) -> &RegisteredFont {
    if !scale.is_finite() {
        panic!("Non-finite text scale");
    }
    if scale <= fonts.small_threshold {
        &fonts.small
    } else if (fonts.profile == FontProfile::Ui && scale >= fonts.big_threshold)
        || (fonts.profile == FontProfile::Cgame && scale > fonts.big_threshold)
    {
        &fonts.big
    } else {
        &fonts.normal
    }
}

/// Fetch a glyph (`glyphAt`).
pub(crate) fn glyph_at(font: &RegisteredFont, code: u32) -> &RegisteredGlyph {
    font.glyphs
        .get(code as usize)
        .unwrap_or_else(|| panic!("Missing font glyph {code}"))
}

/// Text metric core (`textMetric`).
pub(crate) fn text_metric(fonts: &FontSet, input: &str, scale: f32, limit: i32, height: bool) -> i32 {
    let text = byte_text(input);
    let font = select_font(fonts, scale);
    let use_scale = scale * font.glyph_scale;
    let units: Vec<char> = text.chars().collect();
    let maximum = if limit > 0 {
        units.len().min(limit as usize)
    } else {
        units.len()
    };
    let mut value: f32 = 0.0;
    let mut count = 0usize;
    let mut index = 0usize;
    while index < units.len() && count < maximum {
        if escape_at(&units, index) {
            index += 2;
            continue;
        }
        let glyph = glyph_at(font, units[index] as u32);
        if height {
            value = value.max(glyph.metrics.height as f32);
        } else {
            value += glyph.metrics.x_skip as f32;
        }
        count += 1;
        index += 1;
    }
    (value * use_scale).trunc() as i32
}

/// Text width (`textWidth`).
#[must_use]
pub fn text_width(fonts: &FontSet, text: &str, scale: f32, limit: i32) -> i32 {
    text_metric(fonts, text, scale, limit, false)
}

/// Text height (`textHeight`).
#[must_use]
pub fn text_height(fonts: &FontSet, text: &str, scale: f32, limit: i32) -> i32 {
    text_metric(fonts, text, scale, limit, true)
}

/// Drawing coordinate space (`CoordinateSpace`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CoordinateSpace {
    /// Device pixels.
    Pixels,
    /// Cgame 640x480 stretch.
    #[default]
    Stretch640,
    /// Base UI 640.
    BaseUi640,
    /// Team UI 640.
    TeamUi640,
}

impl CoordinateSpace {
    /// Donor spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pixels => "pixels",
            Self::Stretch640 => "stretch-640",
            Self::BaseUi640 => "base-ui-640",
            Self::TeamUi640 => "team-ui-640",
        }
    }
}

/// Engine pixel command queue (`TextDrawSink` command surface).
pub trait HudDrawSink {
    /// Set the current color (None clears).
    fn set_color(&mut self, color: Option<Vec4>);
    /// Stretch/is blit pixels.
    fn stretch_pixels(&mut self, rect: Rect2d, uv: TextureRect, picture: Picture);
}

/// 2D drawing context (`Draw2D`) over a shared engine queue.
#[derive(Clone)]
pub struct Draw2D {
    /// Shared engine queue; identity is queue identity.
    pub sink: Shared<dyn HudDrawSink>,
    /// Coordinate space.
    pub space: CoordinateSpace,
    /// Target width.
    pub target_width: i32,
    /// Target height.
    pub target_height: i32,
}

impl Draw2D {
    /// Wrap a queue.
    pub fn new(sink: Shared<dyn HudDrawSink>, space: CoordinateSpace, target_width: i32, target_height: i32) -> Self {
        Self {
            sink,
            space,
            target_width,
            target_height,
        }
    }

    /// Whether two contexts share one engine queue.
    #[must_use]
    pub fn shares_queue(&self, other: &Draw2D) -> bool {
        Rc::ptr_eq(&self.sink, &other.sink)
    }

    /// Target width (`width`).
    #[must_use]
    pub fn width(&self) -> i32 {
        self.target_width
    }

    /// Target height (`height`).
    #[must_use]
    pub fn height(&self) -> i32 {
        self.target_height
    }

    /// Horizontal scale (`scaleX`).
    #[must_use]
    pub fn scale_x(&self) -> f32 {
        match self.space {
            CoordinateSpace::Pixels => 1.0,
            CoordinateSpace::Stretch640 => self.width() as f32 / 640.0,
            CoordinateSpace::BaseUi640 => self.height() as f32 * (1.0 / 480.0),
            CoordinateSpace::TeamUi640 => self.width() as f32 * (1.0 / 640.0),
        }
    }

    /// Vertical scale (`scaleY`).
    #[must_use]
    pub fn scale_y(&self) -> f32 {
        match self.space {
            CoordinateSpace::Pixels => 1.0,
            CoordinateSpace::Stretch640 => self.height() as f32 / 480.0,
            CoordinateSpace::BaseUi640 => self.scale_x(),
            CoordinateSpace::TeamUi640 => self.height() as f32 * (1.0 / 480.0),
        }
    }

    /// Horizontal bias (`biasX`).
    #[must_use]
    pub fn bias_x(&self) -> f32 {
        if self.space == CoordinateSpace::BaseUi640 && self.width().wrapping_mul(480) > self.height().wrapping_mul(640)
        {
            0.5 * (self.width() as f32 - self.height() as f32 * (640.0 / 480.0))
        } else {
            0.0
        }
    }

    /// Set the current color (`setColor`).
    pub fn set_color(&self, color: Option<Vec4>) {
        self.sink.borrow_mut().set_color(color);
    }

    /// Adjust to device pixels (`adjust`).
    #[must_use]
    pub fn adjust(&self, rect: Rect2d) -> Rect2d {
        let x = rect.x * self.scale_x();
        Rect2d {
            x: if self.space == CoordinateSpace::TeamUi640 {
                x
            } else {
                x + self.bias_x()
            },
            y: rect.y * self.scale_y(),
            width: rect.width * self.scale_x(),
            height: rect.height * self.scale_y(),
        }
    }

    /// Stretch a 640-space rect (`stretchPic`).
    pub fn stretch_pic(&self, rect: Rect2d, uv: TextureRect, picture: Picture) {
        let adjusted = self.adjust(rect);
        self.stretch_pixels(adjusted, uv, picture);
    }

    /// Stretch device pixels (`stretchPixels`).
    pub fn stretch_pixels(&self, rect: Rect2d, uv: TextureRect, picture: Picture) {
        self.sink.borrow_mut().stretch_pixels(rect, uv, picture);
    }

    /// Draw a full-UV picture (`drawPic`).
    pub fn draw_pic(&self, rect: Rect2d, picture: Picture) {
        self.stretch_pic(rect, FULL_UV, picture);
    }
}

/// Draw one charset glyph (`drawChar`).
pub fn draw_char(draw: &Draw2D, charset: Picture, x: f32, y: f32, width: f32, height: f32, code: u32) {
    let ch = code & 255;
    if ch == 32 {
        return;
    }
    let s = f64::from(ch & 15) / 16.0;
    let t = f64::from(ch >> 4) / 16.0;
    draw.stretch_pic(
        rect2d(x, y, width, height),
        TextureRect {
            s: s as f32,
            t: t as f32,
            s2: (s + 1.0 / 16.0) as f32,
            t2: (t + 1.0 / 16.0) as f32,
        },
        charset,
    );
}

/// Draw a fixed cgame string (`drawCgString`).
pub fn draw_cg_string(draw: &Draw2D, charset: Picture, options: &FixedTextOptions) {
    let text = byte_text(&options.text);
    let units: Vec<char> = text.chars().collect();
    let maximum = if options.max_chars <= 0 {
        32767usize
    } else {
        options.max_chars as usize
    };
    for shadow in [true, false] {
        if shadow && !options.shadow {
            continue;
        }
        let mut x = options.x.trunc() as i32;
        let mut count = 0usize;
        draw.set_color(Some(if shadow { black(options.color.w) } else { options.color }));
        let mut index = 0usize;
        while index < units.len() && count < maximum {
            if escape_at(&units, index) {
                if !shadow && !options.force_color {
                    draw.set_color(Some(escape_color(units[index + 1] as u32, options.color.w)));
                }
                index += 2;
                continue;
            }
            draw_char(
                draw,
                charset,
                (x + if shadow { 2 } else { 0 }) as f32,
                options.y.trunc() + if shadow { 2.0 } else { 0.0 },
                options.char_width as f32,
                options.char_height as f32,
                units[index] as u32,
            );
            x += options.char_width;
            count += 1;
            index += 1;
        }
    }
    draw.set_color(None);
}

/// One atlas text pass (`atlasPass`, cgame profile).
#[allow(clippy::too_many_arguments)]
pub(crate) fn atlas_pass_cgame(
    draw: &Draw2D,
    picture: Picture,
    text: &str,
    x: i32,
    y: i32,
    color: Vec4,
    size: f32,
    banner: bool,
) {
    draw.set_color(Some(color));
    let position = draw.adjust(rect2d(x as f32, y as f32, 0.0, 0.0));
    let vertical_scale = draw.scale_x();
    let top = y as f32 * draw.scale_x();
    let mut ax = position.x;
    let mut aw: f32;
    let gap = (if banner { 4.0 } else { 3.0 } * draw.scale_x()) * size;
    let height = (if banner { 36.0 } else { 27.0 } * vertical_scale) * size;
    let units: Vec<char> = text.chars().collect();
    for unit in units {
        let code = unit as u32 & 127;
        if banner && code != 32 && !(65..=90).contains(&code) {
            continue;
        }
        let metric = if banner && code != 32 {
            banner_metric(code)
        } else {
            prop_metric(code)
        };
        if code == 32 {
            aw = (if banner { 12.0 } else { 8.0 } * draw.scale_x()) * size;
        } else if metric[2] != -1 {
            aw = (metric[2] as f32 * draw.scale_x()) * size;
            draw.stretch_pixels(
                rect2d(ax, top, aw, height),
                TextureRect {
                    s: metric[0] as f32 / 256.0,
                    t: metric[1] as f32 / 256.0,
                    s2: (metric[0] + metric[2]) as f32 / 256.0,
                    t2: (metric[1] + if banner { 36 } else { 27 }) as f32 / 256.0,
                },
                picture,
            );
        } else {
            aw = 0.0;
        }
        ax += aw + gap;
    }
    draw.set_color(None);
}

/// Draw a proportional cgame string (`drawCgProportionalString`).
pub fn draw_cg_proportional_string(draw: &Draw2D, fonts: &LegacyFonts, options: &UiTextOptions) {
    let text = byte_text(&options.text);
    let size = if options.style & UI_SMALLFONT != 0 { 0.75 } else { 1.0 };
    let x = aligned_x(
        options.x,
        (proportional_string_width(&text) as f32 * size).trunc() as i32,
        options.style,
    );
    let y = options.y.trunc() as i32;
    if options.style & UI_DROPSHADOW != 0 {
        atlas_pass_cgame(
            draw,
            fonts.proportional,
            &text,
            x + 2,
            y + 2,
            black(options.color.w),
            size,
            false,
        );
    }
    if options.style & UI_INVERSE != 0 {
        let inverse = 0.8f32;
        atlas_pass_cgame(
            draw,
            fonts.proportional,
            &text,
            x,
            y,
            vec4(
                options.color.x * inverse,
                options.color.y * inverse,
                options.color.z * inverse,
                options.color.w,
            ),
            size,
            false,
        );
        return;
    }
    atlas_pass_cgame(draw, fonts.proportional, &text, x, y, options.color, size, false);
    if options.style & UI_PULSE != 0 {
        atlas_pass_cgame(
            draw,
            fonts.glow,
            &text,
            x,
            y,
            Vec4 {
                w: pulse(options.time),
                ..options.color
            },
            size,
            false,
        );
    }
}

/// Draw a banner cgame string (`drawCgBannerString`).
pub fn draw_cg_banner_string(draw: &Draw2D, fonts: &LegacyFonts, options: &UiTextOptions) {
    let text = byte_text(&options.text);
    let x = aligned_x(options.x, banner_string_width(&text), options.style);
    let y = options.y.trunc() as i32;
    if options.style & UI_DROPSHADOW != 0 {
        atlas_pass_cgame(
            draw,
            fonts.banner,
            &text,
            x + 2,
            y + 2,
            black(options.color.w),
            1.0,
            true,
        );
    }
    atlas_pass_cgame(draw, fonts.banner, &text, x, y, options.color, 1.0, true);
}

/// Paint one scalable glyph (`paintGlyph`).
pub(crate) fn paint_glyph(draw: &Draw2D, glyph: &RegisteredGlyph, x: f32, baseline: f32, scale: f32) {
    if let Some(picture) = glyph.picture {
        draw.stretch_pic(
            rect2d(
                x,
                baseline - scale * glyph.metrics.top as f32,
                glyph.metrics.image_width as f32 * scale,
                glyph.metrics.image_height as f32 * scale,
            ),
            TextureRect {
                s: glyph.metrics.s,
                t: glyph.metrics.t,
                s2: glyph.metrics.s2,
                t2: glyph.metrics.t2,
            },
            picture,
        );
    }
}

/// Paint scalable text (`textPaint`).
pub fn text_paint(draw: &Draw2D, fonts: &FontSet, options: &TextPaintOptions) {
    let text = byte_text(&options.text);
    let font = select_font(fonts, options.scale);
    let scale = options.scale * font.glyph_scale;
    let units: Vec<char> = text.chars().collect();
    let maximum = if options.limit > 0 {
        units.len().min(options.limit as usize)
    } else {
        units.len()
    };
    let mut x = options.x;
    let mut color = options.color;
    let mut count = 0usize;
    let mut index = 0usize;
    draw.set_color(Some(color));
    while index < units.len() && count < maximum {
        if escape_at(&units, index) {
            color = escape_color(units[index + 1] as u32, options.color.w);
            draw.set_color(Some(color));
            index += 2;
            continue;
        }
        let glyph = glyph_at(font, units[index] as u32);
        let baseline = options.y;
        if options.style == 3 || options.style == 6 {
            let offset = if options.style == 3 { 1.0 } else { 2.0 };
            if let Some(picture) = glyph.picture {
                draw.set_color(Some(black(color.w)));
                draw.stretch_pic(
                    rect2d(
                        x + offset,
                        baseline - scale * glyph.metrics.top as f32 + offset,
                        glyph.metrics.image_width as f32 * scale,
                        glyph.metrics.image_height as f32 * scale,
                    ),
                    TextureRect {
                        s: glyph.metrics.s,
                        t: glyph.metrics.t,
                        s2: glyph.metrics.s2,
                        t2: glyph.metrics.t2,
                    },
                    picture,
                );
                draw.set_color(Some(color));
            }
        }
        paint_glyph(draw, glyph, x, baseline, scale);
        x += glyph.metrics.x_skip as f32 * scale + options.adjust;
        count += 1;
        index += 1;
    }
    draw.set_color(None);
}

/// Paint scalable text clamped to `max_x` (`textPaintLimit`).
pub fn text_paint_limit(draw: &Draw2D, fonts: &FontSet, options: &TextPaintOptions, max_x: f32) -> f32 {
    let text = byte_text(&options.text);
    let cgame_fonts = FontSet {
        profile: FontProfile::Cgame,
        small: fonts.small.clone(),
        normal: fonts.normal.clone(),
        big: fonts.big.clone(),
        small_threshold: fonts.small_threshold,
        big_threshold: fonts.big_threshold,
    };
    let font = select_font(&cgame_fonts, options.scale);
    let scale = options.scale * font.glyph_scale;
    let units: Vec<char> = text.chars().collect();
    let maximum = if options.limit > 0 {
        units.len().min(options.limit as usize)
    } else {
        units.len()
    };
    let mut x = options.x;
    let mut result = max_x;
    let mut count = 0usize;
    let mut index = 0usize;
    draw.set_color(Some(options.color));
    while index < units.len() && count < maximum {
        if escape_at(&units, index) {
            draw.set_color(Some(escape_color(units[index + 1] as u32, options.color.w)));
            index += 2;
            continue;
        }
        let rest: String = units[index..].iter().collect();
        if text_width(fonts, &rest, scale, 1) as f32 + x > max_x {
            result = 0.0;
            break;
        }
        let glyph = glyph_at(font, units[index] as u32);
        paint_glyph(draw, glyph, x, options.y, scale);
        x += glyph.metrics.x_skip as f32 * scale + options.adjust;
        result = x;
        count += 1;
        index += 1;
    }
    draw.set_color(None);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::math::{angles_to_axis, vec3};

    #[test]
    fn qvm_axes_zero_angles() {
        let axis = angles_to_axis(vec3(0.0, 0.0, 0.0));
        assert_eq!(axis[0], vec3(1.0, 0.0, 0.0));
        assert_eq!(axis[1], vec3(-0.0, 1.0, 0.0));
        assert_eq!(axis[2], vec3(0.0, -0.0, 1.0));
    }

    #[test]
    fn draw_strlen_skips_escapes() {
        assert_eq!(draw_strlen("abc"), 3);
        assert_eq!(draw_strlen("^1ab"), 2);
        assert_eq!(draw_strlen("^^"), 2);
        assert_eq!(draw_strlen("ab\0cd"), 2);
        assert_eq!(proportional_size_scale(UI_SMALLFONT), 0.75);
        assert_eq!(proportional_size_scale(0), 1.0);
    }

    #[test]
    fn fade_color_windows() {
        assert_eq!(fade_color(100, 0, 3000.0), None);
        assert_eq!(fade_color(5000, 1000, 3000.0), None);
        assert_eq!(fade_color(1500, 1000, 3000.0), Some(vec4(1.0, 1.0, 1.0, 1.0)));
        assert_eq!(fade_color(3950, 1000, 3000.0).unwrap().w, 50.0 / 200.0);
    }

    #[test]
    fn team_and_health_colors() {
        assert_eq!(team_color(Team::TeamRed as i32), vec4(1.0, 0.2, 0.2, 1.0));
        assert_eq!(team_color(Team::TeamBlue as i32), vec4(0.2, 0.2, 1.0, 1.0));
        assert_eq!(team_color(Team::TeamSpectator as i32), vec4(0.7, 0.7, 0.7, 1.0));
        assert_eq!(team_color(99), vec4(1.0, 1.0, 1.0, 1.0));
        assert_eq!(get_color_for_health(0, 0), vec4(0.0, 0.0, 0.0, 1.0));
        assert_eq!(get_color_for_health(100, 100), vec4(1.0, 1.0, 1.0, 1.0));
        let mid = get_color_for_health(50, 0);
        assert_eq!(mid, vec4(1.0, 20.0 / 30.0, 0.0, 1.0));
    }

    #[test]
    fn atlas_widths() {
        assert_eq!(proportional_string_width("A"), 18);
        assert_eq!(banner_string_width("A"), 33);
        assert_eq!(proportional_string_width("a"), proportional_string_width("A"));
    }

    #[test]
    fn scalable_text_metrics() {
        let mut font = zero_font();
        font.glyph_scale = 1.0;
        for glyph in &mut font.glyphs {
            glyph.metrics.x_skip = 10;
            glyph.metrics.height = 12;
        }
        let fonts = FontSet {
            small: zero_font(),
            normal: font,
            big: zero_font(),
            profile: FontProfile::Cgame,
            small_threshold: 0.0,
            big_threshold: 99.0,
        };
        assert_eq!(text_width(&fonts, "AB", 1.0, 0), 20);
        assert_eq!(text_width(&fonts, "^1AB", 1.0, 0), 20);
        assert_eq!(text_width(&fonts, "ABC", 1.0, 2), 20);
        assert_eq!(text_height(&fonts, "AB", 1.0, 0), 12);
    }
}
