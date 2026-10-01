//! Menu console fonts and proportional menu typography.
//!
//! Donor provenance: `src/app/bootstrap/menu-font.ts` (`loadMenuFont`,
//! `loadMenuTypography`). Mount reads, WAD/PCX/palette decoding, charset
//! construction, codepoint coverage, and product selection are a direct
//! port over workspace siblings. Charset/texture registration and mounted
//! TrueType selection stay behind [`MenuCharsetImages`] and
//! [`MountedMenuFonts`] (those registry surfaces drifted).

use qa_client::text::atlas::{classic_charset, text_codepoints, AtlasGlyph, AtlasKind, TextAtlas, TextFontSelection};
use qa_client::text::draw2d::ImagePicture;
use qa_content::contract::{ContentMount, GameFamily, MountPlanId, ResolvedMountPlan};
use qa_content::images::indexed::{decode_pcx, IndexedImage};
use qa_content::images::palette::{decode_palette, indexed_render_image, PaletteTransparency};
use qa_content::images::wad::decode_wad;
use qa_content::mounts::{open_mount_plan, MountedContent, OpenMountOptions};
use qa_content::q3::presentation::draw_tools::prop_metric;
use std::collections::{BTreeMap, HashSet};
use thiserror::Error;

/// Failure of menu font loading.
#[derive(Debug, Error)]
pub enum MenuFontError {
    /// Font loading failure.
    #[error("{0}")]
    Font(String),
    /// Mount failure.
    #[error(transparent)]
    Mount(#[from] qa_content::mounts::MountError),
    /// Content decode failure.
    #[error(transparent)]
    Binary(#[from] qa_core::binary::BinaryError),
    /// Catalog failure.
    #[error(transparent)]
    Catalog(#[from] qa_content::catalog::CatalogError),
    /// Client failure.
    #[error(transparent)]
    Client(#[from] qa_client::ClientError),
}

/// Loaded menu texture: name plus its picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuTexture {
    /// Texture name.
    pub name: &'static str,
    /// Picture handle.
    pub picture: ImagePicture,
}

/// Charset image registration and texture loading.
pub trait MenuCharsetImages {
    /// Load a menu texture, returning it when present.
    fn load_texture(
        &mut self,
        path: &str,
        family: GameFamily,
        palette: Option<&[u8]>,
    ) -> Result<Option<MenuTexture>, MenuFontError>;
    /// Register an indexed charset image, returning its handle.
    fn register_indexed(
        &mut self,
        name: &str,
        width: u32,
        height: u32,
        pixels: Vec<u8>,
        palette: Vec<u8>,
    ) -> Result<u32, MenuFontError>;
    /// Release an image handle.
    fn release(&mut self, image: u32);
}

/// Mounted font selection (kfont/TrueType).
pub trait MountedMenuFonts {
    /// Select a kfont with a classic fallback.
    fn select_kfont(&mut self, path: &str, classic: TextAtlas) -> Result<TextFontSelection, MenuFontError>;
    /// Load a TrueType atlas, returning it when present.
    fn load_true_type(&mut self, path: &str, size: u32, codepoints: &[u32])
        -> Result<Option<TextAtlas>, MenuFontError>;
    /// Load TrueType fallback pages.
    fn load_true_type_pages(
        &mut self,
        path: &str,
        size: u32,
        codepoints: &[u32],
    ) -> Result<Vec<TextAtlas>, MenuFontError>;
    /// Close the font registry.
    fn close(&mut self);
}

/// Loaded menu font with its image handle.
pub struct LoadedMenuFont {
    /// Font selection.
    pub font: TextFontSelection,
    image: u32,
}

impl LoadedMenuFont {
    /// Close fonts and release the image.
    pub fn close(self, images: &mut dyn MenuCharsetImages, fonts: &mut dyn MountedMenuFonts) {
        fonts.close();
        images.release(self.image);
    }
}

/// Load the console charset font for a family.
pub fn load_menu_font(
    mounts: &MountedContent,
    family: GameFamily,
    rerelease: bool,
    images: &mut dyn MenuCharsetImages,
    fonts: &mut dyn MountedMenuFonts,
) -> Result<LoadedMenuFont, MenuFontError> {
    let (image, width, height) = if family == GameFamily::Q1 {
        let wad = mounts.open("gfx.wad", |_| true)?;
        let palette = mounts.open("gfx/palette.lmp", |_| true)?;
        let (Some(wad), Some(palette)) = (wad, palette) else {
            return Err(MenuFontError::Font(
                "Quake console requires gfx.wad and its palette".to_owned(),
            ));
        };
        let archive = decode_wad(&wad.bytes, "gfx.wad")?;
        let lump = archive.lumps.iter().find(|value| value.name == "conchars");
        let Some(lump) = lump else {
            return Err(MenuFontError::Font("Quake conchars is missing or malformed".to_owned()));
        };
        if lump.compression != 0 || lump.bytes.len() != 128 * 128 {
            return Err(MenuFontError::Font("Quake conchars is missing or malformed".to_owned()));
        }
        let palette = decode_palette(&palette.bytes, "gfx/palette.lmp")?;
        indexed_render_image(
            vec![IndexedImage {
                width: 128,
                height: 128,
                indices: lump.bytes.clone(),
            }],
            palette.clone(),
            PaletteTransparency::Index(0),
            None,
            None,
        )?;
        (
            images.register_indexed("conchars", 128, 128, lump.bytes.clone(), palette.colors)?,
            128,
            128,
        )
    } else {
        let palette_asset = if family == GameFamily::Q2 {
            mounts.open("pics/colormap.pcx", |_| true)?
        } else {
            None
        };
        let colors = match palette_asset.as_ref() {
            Some(asset) => decode_pcx(&asset.bytes, "pics/colormap.pcx")?.palette,
            None => None,
        };
        let texture = images.load_texture(
            if family == GameFamily::Q2 {
                "pics/conchars.pcx"
            } else {
                "gfx/2d/bigchars"
            },
            family,
            colors.as_deref(),
        )?;
        let Some(texture) = texture else {
            return Err(MenuFontError::Font("Quake III console charset is missing".to_owned()));
        };
        (texture.picture.image, texture.picture.width, texture.picture.height)
    };
    let classic = classic_charset(image, width, height, "conchars", family != GameFamily::Q3)?;
    let font = if family == GameFamily::Q2 && rerelease {
        match fonts.select_kfont("fonts/qconfont.kfont", classic) {
            Ok(font) => font,
            Err(error) => {
                fonts.close();
                images.release(image);
                return Err(error);
            }
        }
    } else {
        TextFontSelection::Classic { classic, unicode: None }
    };
    Ok(LoadedMenuFont { font, image })
}

/// Menu typography: body plus title selections.
pub struct MenuTypography<F> {
    /// Body selection.
    pub body: TextFontSelection,
    /// Title selection.
    pub title: TextFontSelection,
    /// Open mounts, released when typography drops.
    pub mounted: Option<MountedContent>,
    /// Closer for fonts and mounts.
    pub closer: F,
}

/// Opened typography mounts plus their closer.
pub type OpenedTypographyMounts = (MountedContent, Box<dyn FnOnce()>);

/// Mount opener for typography products.
pub trait TypographyMounts {
    /// Open mounts for `mounts`, returning a closer.
    fn open(&mut self, mounts: Vec<ContentMount>) -> Result<OpenedTypographyMounts, MenuFontError>;
}

/// Proportional glyph atlas from Q3 metrics.
#[must_use]
pub fn proportional_glyphs() -> BTreeMap<u32, AtlasGlyph> {
    let mut glyphs = BTreeMap::new();
    for code in 32..127u32 {
        let [x, y, width] = prop_metric(code);
        glyphs.insert(
            code,
            AtlasGlyph {
                x: x as u32,
                y: y as u32,
                width: width as u32,
                height: 27,
                advance: (width + 3) as u32,
                color: false,
            },
        );
    }
    glyphs
}

/// Load proportional menu typography: rerelease TrueType, Q3 metrics, or classic fallback.
pub fn load_menu_typography(
    catalog: &qa_content::catalog::InstalledCatalog,
    classic: TextAtlas,
    images: &mut dyn MenuCharsetImages,
    fonts: &mut dyn MountedMenuFonts,
    mounts: &mut dyn TypographyMounts,
) -> Result<MenuTypography<Box<dyn FnOnce()>>, MenuFontError> {
    let product = catalog.products.iter().find(|product| {
        product.availability == qa_content::catalog::ProductAvailability::Installed
            && product.expectation.edition == "rerelease"
            && (product.expectation.family == GameFamily::Q1 || product.expectation.family == GameFamily::Q2)
    });
    let Some(product) = product else {
        let q3 = catalog.products.iter().find(|product| {
            product.availability == qa_content::catalog::ProductAvailability::Installed
                && product.expectation.family == GameFamily::Q3
        });
        let Some(q3) = q3 else {
            let font = TextFontSelection::Classic { classic, unicode: None };
            return Ok(MenuTypography {
                body: font.clone(),
                title: font,
                mounted: None,
                closer: Box::new(|| {}),
            });
        };
        let plan_mounts = catalog.mounts_for(q3.id.as_str())?;
        let (mounted, close_mounted) = mounts.open(plan_mounts)?;
        let texture = images.load_texture("menu/art/font1_prop.tga", GameFamily::Q3, None)?;
        let Some(texture) = texture else {
            close_mounted();
            return Err(MenuFontError::Font(
                "Quake III proportional menu font is missing".to_owned(),
            ));
        };
        let font = TextFontSelection::Atlas {
            classic,
            font: TextAtlas {
                kind: AtlasKind::Kfont,
                name: "Q3 proportional".to_owned(),
                picture: texture.picture,
                line_height: 27,
                cap_ink: None,
                glyphs: proportional_glyphs(),
            },
            fallbacks: Vec::new(),
        };
        return Ok(MenuTypography {
            body: font.clone(),
            title: font,
            mounted: Some(mounted),
            closer: close_mounted,
        });
    };
    let plan_mounts = catalog.mounts_for(product.id.as_str())?;
    let (mounted, close_mounted) = mounts.open(plan_mounts)?;
    let outcome: Result<(TextFontSelection, TextFontSelection), MenuFontError> = (|| {
        let mut coverage: HashSet<u32> = (32..256).chain(0x2000..0x2070).collect();
        for name in mounted.list_files("localization", ".txt")? {
            if let Some(resource) = mounted.open(&format!("localization/{name}"), |_| true)? {
                let text = String::from_utf8(resource.bytes)
                    .map_err(|_| MenuFontError::Font("Invalid localization text".to_owned()))?;
                for point in text_codepoints(&text) {
                    if point >= 32 {
                        coverage.insert(point);
                    }
                }
            }
        }
        let authored: Vec<u32> = coverage.iter().copied().collect();
        let codepoints: Vec<u32> = (32..256).chain(0x2000..0x2070).collect();
        let body = fonts.load_true_type("fonts/Montserrat-Regular.ttf", 48, &codepoints)?;
        let title = fonts.load_true_type("fonts/NotoSans-Bold.ttf", 72, &codepoints)?;
        let (Some(body), Some(title)) = (body, title) else {
            return Err(MenuFontError::Font(
                "Installed proportional menu fonts are missing".to_owned(),
            ));
        };
        let mut fallbacks = vec![title.clone()];
        for path in [
            "fonts/NotoSans-Bold.ttf",
            "fonts/NotoSansJP-Regular.otf",
            "fonts/NotoSansKR-Regular.otf",
        ] {
            let missing: Vec<u32> = authored
                .iter()
                .copied()
                .filter(|point| {
                    !body.glyphs.contains_key(point)
                        && !fallbacks.iter().any(|font: &TextAtlas| font.glyphs.contains_key(point))
                })
                .collect();
            if missing.is_empty() {
                break;
            }
            fallbacks.extend(fonts.load_true_type_pages(path, 48, &missing)?);
        }
        Ok((
            TextFontSelection::Atlas {
                font: body,
                classic: classic.clone(),
                fallbacks: fallbacks.clone(),
            },
            TextFontSelection::Atlas {
                font: title,
                classic,
                fallbacks: fallbacks[1..].to_vec(),
            },
        ))
    })();
    match outcome {
        Ok((body, title)) => Ok(MenuTypography {
            body,
            title,
            mounted: Some(mounted),
            closer: close_mounted,
        }),
        Err(error) => {
            fonts.close();
            close_mounted();
            Err(error)
        }
    }
}

/// Open a typography mount plan over `mounts`.
pub fn open_typography_mounts(mounts: Vec<ContentMount>) -> Result<MountedContent, MenuFontError> {
    let plan = ResolvedMountPlan {
        id: MountPlanId("mount-plan:menu:typography".to_owned()),
        mounts: mounts.clone(),
        default_order: mounts.iter().map(|mount| mount.identity().id.clone()).collect(),
        prefix_orders: Vec::new(),
    };
    Ok(open_mount_plan(
        &plan,
        OpenMountOptions {
            pure: None,
            q3_restriction: None,
            links: Vec::new(),
            loose_comparison: None,
        },
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_content::catalog::{CatalogProduct, InstalledCatalog, ProductAvailability, ProductExpectation};
    use qa_content::contract::ContentId;
    use std::collections::HashMap;

    struct FakeImages {
        textures: HashMap<String, MenuTexture>,
        registered: Vec<u32>,
        released: Vec<u32>,
        next: u32,
    }

    impl MenuCharsetImages for FakeImages {
        fn load_texture(
            &mut self,
            path: &str,
            _family: GameFamily,
            _palette: Option<&[u8]>,
        ) -> Result<Option<MenuTexture>, MenuFontError> {
            Ok(self.textures.get(path).copied())
        }

        fn register_indexed(
            &mut self,
            _name: &str,
            _width: u32,
            _height: u32,
            _pixels: Vec<u8>,
            _palette: Vec<u8>,
        ) -> Result<u32, MenuFontError> {
            self.next += 1;
            self.registered.push(self.next);
            Ok(self.next)
        }

        fn release(&mut self, image: u32) {
            self.released.push(image);
        }
    }

    struct FakeFonts;

    impl MountedMenuFonts for FakeFonts {
        fn select_kfont(&mut self, _path: &str, classic: TextAtlas) -> Result<TextFontSelection, MenuFontError> {
            Ok(TextFontSelection::Classic { classic, unicode: None })
        }

        fn load_true_type(
            &mut self,
            _path: &str,
            _size: u32,
            _codepoints: &[u32],
        ) -> Result<Option<TextAtlas>, MenuFontError> {
            Ok(None)
        }

        fn load_true_type_pages(
            &mut self,
            _path: &str,
            _size: u32,
            _codepoints: &[u32],
        ) -> Result<Vec<TextAtlas>, MenuFontError> {
            Ok(Vec::new())
        }

        fn close(&mut self) {}
    }

    struct FakeMountsOpener;

    impl TypographyMounts for FakeMountsOpener {
        fn open(&mut self, mounts: Vec<ContentMount>) -> Result<OpenedTypographyMounts, MenuFontError> {
            Ok((open_typography_mounts(mounts)?, Box::new(|| {})))
        }
    }

    fn catalog() -> InstalledCatalog {
        InstalledCatalog::new(
            "/corpus".to_owned(),
            vec![CatalogProduct {
                id: ContentId("q3".to_owned()),
                expectation: ProductExpectation {
                    id: "q3".to_owned(),
                    family: GameFamily::Q3,
                    edition: "classic".to_owned(),
                    campaign: "baseq3".to_owned(),
                    title: "Q3".to_owned(),
                    content_directory: "baseq3".to_owned(),
                    base_product: None,
                    required_content_archives: Vec::new(),
                    required_programs: Vec::new(),
                    map_witness: None,
                    unresolved_reason: None,
                },
                availability: ProductAvailability::Installed,
                archives: Vec::new(),
                loose_root: None,
                user_content: None,
                maps: Vec::new(),
                diagnostics: Vec::new(),
            }],
            Vec::new(),
            1,
            None,
        )
        .expect("catalog")
    }

    fn classic() -> TextAtlas {
        classic_charset(1, 128, 128, "conchars", true).expect("classic")
    }

    #[test]
    fn proportional_glyphs_cover_printable_ascii() {
        let glyphs = proportional_glyphs();
        assert_eq!(glyphs.len(), 95);
        assert_eq!(glyphs.get(&32).expect("space").height, 27);
        assert_eq!(
            glyphs.get(&65).expect("A").advance,
            glyphs.get(&65).expect("A").width + 3
        );
    }

    #[test]
    fn q3_typography_loads_proportional_font() {
        let catalog = catalog();
        let mut images = FakeImages {
            textures: HashMap::from([(
                "menu/art/font1_prop.tga".to_owned(),
                MenuTexture {
                    name: "font1_prop",
                    picture: ImagePicture {
                        image: 9,
                        width: 256,
                        height: 256,
                    },
                },
            )]),
            registered: Vec::new(),
            released: Vec::new(),
            next: 0,
        };
        let mut fonts = FakeFonts;
        let mut opener = FakeMountsOpener;
        let typography =
            load_menu_typography(&catalog, classic(), &mut images, &mut fonts, &mut opener).expect("typography");
        assert!(matches!(typography.body, TextFontSelection::Atlas { .. }));
        (typography.closer)();
    }

    #[test]
    fn missing_proportional_font_errors() {
        let catalog = catalog();
        let mut images = FakeImages {
            textures: HashMap::new(),
            registered: Vec::new(),
            released: Vec::new(),
            next: 0,
        };
        let mut fonts = FakeFonts;
        let mut opener = FakeMountsOpener;
        let err = match load_menu_typography(&catalog, classic(), &mut images, &mut fonts, &mut opener) {
            Ok(_) => panic!("missing font"),
            Err(err) => err,
        };
        assert_eq!(err.to_string(), "Quake III proportional menu font is missing");
    }

    #[test]
    fn classic_fallback_without_products() {
        let catalog = InstalledCatalog::new("/corpus".to_owned(), Vec::new(), Vec::new(), 1, None).expect("catalog");
        let mut images = FakeImages {
            textures: HashMap::new(),
            registered: Vec::new(),
            released: Vec::new(),
            next: 0,
        };
        let mut fonts = FakeFonts;
        let mut opener = FakeMountsOpener;
        let typography =
            load_menu_typography(&catalog, classic(), &mut images, &mut fonts, &mut opener).expect("typography");
        assert!(matches!(typography.body, TextFontSelection::Classic { .. }));
        (typography.closer)();
    }
}
