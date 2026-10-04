//! Quake rerelease `kfont` parsing.
//!
//! Donor provenance: `src/text/kfont.ts` (from `quakespasm` `kfont`).

/// First ASCII code in a `kfont`.
pub const KFONT_ASCII_MIN: u32 = 32;
/// Last ASCII code in a `kfont`.
pub const KFONT_ASCII_MAX: u32 = 126;
/// ASCII coverage count.
pub const KFONT_NUM_CHARS: usize = (KFONT_ASCII_MAX - KFONT_ASCII_MIN + 1) as usize;

/// A `kfont` glyph (`KfontCharT`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KfontChar {
    /// X offset.
    pub x: i32,
    /// Y offset.
    pub y: i32,
    /// Width.
    pub w: i32,
    /// Height.
    pub h: i32,
    /// Baked color.
    pub color: bool,
}

/// A parsed `kfont` (`ParsedKfontT`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedKfont {
    /// Texture token.
    pub texture_token: String,
    /// ASCII glyphs.
    pub chars: Vec<Option<KfontChar>>,
    /// All glyphs by codepoint.
    pub glyphs: Vec<(u32, KfontChar)>,
    /// Line height.
    pub line_height: i32,
}

/// Minimal Q2 token state (`LegacyParseState` subset).
struct TokenState<'a> {
    bytes: &'a [u8],
    index: usize,
}

fn parse_q2_token(state: &mut TokenState) -> String {
    crate::render::scene::image_policy::next_q2_token(state.bytes, &mut state.index)
}

/// Parse a `kfont` (`ParseKfont`).
#[must_use]
pub fn parse_kfont(text: &str) -> Option<ParsedKfont> {
    let mut state = TokenState {
        bytes: text.as_bytes(),
        index: 0,
    };
    let mut texture_token: Option<String> = None;
    let mut chars: Vec<Option<KfontChar>> = vec![None; KFONT_NUM_CHARS];
    let mut glyphs: Vec<(u32, KfontChar)> = Vec::new();
    let mut line_height = 0i32;
    loop {
        let token = parse_q2_token(&mut state);
        if token.is_empty() {
            break;
        }
        if token == "texture" {
            texture_token = Some(parse_q2_token(&mut state));
        } else if token == "unicode" {
            // No payload; entries arrive as mapchar blocks.
        } else if token == "mapchar" {
            parse_q2_token(&mut state);
            loop {
                let entry = parse_q2_token(&mut state);
                if entry == "}" || entry.is_empty() {
                    break;
                }
                let codepoint = entry.parse::<i32>().ok()?;
                let x = parse_q2_token(&mut state).parse::<i32>().ok()?;
                let y = parse_q2_token(&mut state).parse::<i32>().ok()?;
                let w = parse_q2_token(&mut state).parse::<i32>().ok()?;
                let h = parse_q2_token(&mut state).parse::<i32>().ok()?;
                parse_q2_token(&mut state);
                let glyph = KfontChar {
                    x,
                    y,
                    w,
                    h,
                    color: false,
                };
                if let Ok(code) = u32::try_from(codepoint) {
                    glyphs.push((code, glyph));
                    if h > line_height {
                        line_height = h;
                    }
                    if let Ok(offset) = usize::try_from(codepoint - KFONT_ASCII_MIN as i32) {
                        if offset < KFONT_NUM_CHARS {
                            chars[offset] = Some(glyph);
                        }
                    }
                }
            }
        }
    }
    Some(ParsedKfont {
        texture_token: texture_token?,
        chars,
        glyphs,
        line_height,
    })
}

/// A retained `kfont` (`KfontT`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kfont {
    /// Picture name.
    pub pic: String,
    /// ASCII glyphs.
    pub chars: Vec<Option<KfontChar>>,
    /// All glyphs by codepoint.
    pub glyphs: Vec<(u32, KfontChar)>,
    /// Line height.
    pub line_height: i32,
}

impl From<ParsedKfont> for Kfont {
    fn from(parsed: ParsedKfont) -> Self {
        Self {
            pic: parsed.texture_token.clone(),
            chars: parsed.chars,
            glyphs: parsed.glyphs,
            line_height: parsed.line_height,
        }
    }
}

/// Look up an ASCII glyph (`SCR_KFontLookup`).
#[must_use]
pub fn kfont_lookup(font: &Kfont, codepoint: u32) -> Option<KfontChar> {
    if !(KFONT_ASCII_MIN..=KFONT_ASCII_MAX).contains(&codepoint) {
        return None;
    }
    let glyph = font.chars[(codepoint - KFONT_ASCII_MIN) as usize]?;
    if glyph.w == 0 {
        None
    } else {
        Some(glyph)
    }
}

/// Look up any glyph (`kfontGlyph`).
#[must_use]
pub fn kfont_glyph(font: &Kfont, codepoint: u32) -> Option<KfontChar> {
    let glyph = font
        .glyphs
        .iter()
        .find(|(code, _)| *code == codepoint)
        .map(|(_, glyph)| *glyph)?;
    if glyph.w == 0 {
        None
    } else {
        Some(glyph)
    }
}

/// Whether a glyph exists (`kfontHasGlyph`).
#[must_use]
pub fn kfont_has_glyph(font: &Kfont, codepoint: u32) -> bool {
    kfont_glyph(font, codepoint).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "texture gfx/font.tga\nmapchar font\n65 0 0 8 8 0\n";

    #[test]
    fn parses_mapchar() {
        let parsed = parse_kfont(SAMPLE).unwrap();
        assert_eq!(parsed.texture_token, "gfx/font.tga");
        assert_eq!(parsed.line_height, 8);
        let font = Kfont::from(parsed);
        assert_eq!(kfont_lookup(&font, 65).unwrap().w, 8);
        assert!(kfont_lookup(&font, 66).is_none());
        assert!(kfont_has_glyph(&font, 65));
    }

    #[test]
    fn missing_texture_is_none() {
        assert!(parse_kfont("mapchar font\n65 0 0 8 8 0\n").is_none());
    }

    #[test]
    fn braces_are_rejected_like_donor() {
        assert!(parse_kfont("texture gfx/font.tga\nmapchar font {\n65 0 0 8 8 0\n}\n").is_none());
    }

    #[test]
    fn out_of_range_is_none() {
        let font = Kfont::from(parse_kfont(SAMPLE).unwrap());
        assert!(kfont_lookup(&font, 31).is_none());
        assert!(kfont_lookup(&font, 127).is_none());
    }

    #[test]
    fn shared_tokenizer_matches_q2_parse_edges() {
        fn tokens(text: &str) -> Vec<String> {
            let mut state = TokenState {
                bytes: text.as_bytes(),
                index: 0,
            };
            let mut out = Vec::new();
            loop {
                let token = parse_q2_token(&mut state);
                if token.is_empty() {
                    break;
                }
                out.push(token);
            }
            out
        }
        assert_eq!(tokens("a  b\tc"), vec!["a", "b", "c"]);
        assert_eq!(tokens("a // skip\nb"), vec!["a", "b"]);
        assert_eq!(tokens("a // trailing"), vec!["a"]);
        assert_eq!(tokens("a \"b c\" d"), vec!["a", "b c", "d"]);
        assert_eq!(tokens("\"unterminated"), vec!["unterminated"]);
        assert_eq!(tokens(""), Vec::<String>::new());
        assert_eq!(tokens("   // only"), Vec::<String>::new());
        assert_eq!(tokens("a/ /b"), vec!["a/", "/b"]);
        let quoted = parse_kfont("texture \"gfx/my font.tga\"\nmapchar font\n65 0 0 8 8 0\n").unwrap();
        assert_eq!(quoted.texture_token, "gfx/my font.tga");
    }
}
