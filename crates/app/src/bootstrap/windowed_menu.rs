//! Windowed menu entry: the startup menu as windowed GL frames.
//!
//! Donor provenance: `src/app/bootstrap/startup.ts`
//! (`StartupApplication.open` with `entry === "menu"`, and the `draw`
//! closure that paints `StartupMenu` when no game client is active). The
//! donor opens graphics for both entries but only queues the initial world
//! launch for `"run"`; `"menu"` stays on the frontend menu. This module is
//! the `"menu"` side of that split: [`WindowedMenu`] owns a ported
//! [`StartupMenu`](super::startup_menu::StartupMenu) and presents each frame
//! as one ordered 2D view, reusing the real menu layout (backdrop, panel,
//! buttons, titles) through the menu's own `draw` path. A capturing
//! [`UiRenderServices`](qa_client::ui::common::draw::UiRenderServices)
//! records fills, images, and text runs as pixel quads, which become
//! vertex-colored [`DrawBatch`] values (the same overlay pattern as the
//! Quake II damage blend): fills over a 1x1 uploaded white image grouped
//! into emit-order runs with the textured donor backdrop quad (the panel
//! is a flat fill and the skin leaves panel/focus art unset, exactly the
//! donor theme), then textured glyph batches binding the menu font
//! atlases from [`super::windowed_menu_text`] so every glyph draws with
//! real UVs. Menu text follows the donor cascade through
//! [`load_menu_typography`](super::menu_font::load_menu_typography):
//! rerelease TrueType body/title atlases, else the Q3 proportional atlas,
//! else the classic console charset (the real `conchars` through installed
//! content mounts when game data is present, the synthetic atlas
//! otherwise), so the menu entry is always available, with or without
//! game content.

use std::cell::Cell;
use std::cell::RefCell;
use std::rc::Rc;

use qa_client::audio::engine::UnifiedAudio;
use qa_client::render::scene::resources::SceneImageRegistry;
use qa_client::render::types::AlphaTest;
use qa_client::render::types::BatchLighting;
use qa_client::render::types::BatchPrimitive;
use qa_client::render::types::BatchVertices;
use qa_client::render::types::BlendFactor;
use qa_client::render::types::CullFace;
use qa_client::render::types::DepthTest;
use qa_client::render::types::DrawBatch;
use qa_client::render::types::ImageLevel;
use qa_client::render::types::ImageResourceOperation;
use qa_client::render::types::ImageSource;
use qa_client::render::types::Rect as ClientRect;
use qa_client::render::types::RenderImage;
use qa_client::render::types::RenderOperation;
use qa_client::render::types::RenderState;
use qa_client::render::types::RenderVertex;
use qa_client::render::types::RenderView as ClientRenderView;
use qa_client::render::types::RenderViewState;
use qa_client::render::types::RendererImage;
use qa_client::render::types::ResourceOwner;
use qa_client::render::types::SourceTime;
use qa_client::render::types::TextureBinding;
use qa_client::render::types::TextureFilter;
use qa_client::render::types::TextureSampling;
use qa_client::render::types::ViewClear;
use qa_client::render::types::ViewTarget;
use qa_client::text::atlas::FontImageServices;
use qa_client::text::atlas::TextAtlas;
use qa_client::text::atlas::TextFontRegistry;
use qa_client::text::atlas::TextFontRequest;
use qa_client::text::atlas::TextFontSelection;
use qa_client::text::draw2d::Draw2D;
use qa_client::text::draw2d::ImagePicture;
use qa_client::text::draw2d::PictureAsset;
use qa_client::text::draw2d::Rect;
use qa_client::text::draw2d::TextureRect;
use qa_client::text::layout::draw_text_layout;
use qa_client::text::layout::layout_text;
use qa_client::text::layout::ColorCodes;
use qa_client::text::layout::TextAlign as LayoutAlign;
use qa_client::text::layout::TextLayoutOptions;
use qa_client::ui::common::assets::load_native_ui_art;
use qa_client::ui::common::draw::UiEmitCommand;
use qa_client::ui::common::draw::UiMaterialDraw;
use qa_client::ui::common::draw::UiRenderServices;
use qa_client::ui::types::ContentId as UiContentId;
use qa_client::ui::types::DopplerSelection as UiDoppler;
use qa_client::ui::types::EnvironmentSelection as UiEnvironment;
use qa_client::ui::types::PresentationSelection as UiPresentation;
use qa_client::ui::types::ProviderRef as UiProviderRef;
use qa_client::ui::types::ResourceId;
use qa_client::ui::types::SeatInputEvent as UiSeatInputEvent;
use qa_client::ui::types::SeatInputEventKind as UiSeatInputEventKind;
use qa_client::ui::types::SeatPresentationBinding;
use qa_client::ui::types::TextAlign;
use qa_client::ui::types::UiDrawCommand;
use qa_client::ui::types::UiDrawContext;
use qa_client::ClientError;
use qa_content::contract::ContentMount;
use qa_content::contract::GameFamily;
use qa_content::images::expand_indexed_image;
use qa_content::images::indexed_render_image;
use qa_content::images::IndexedImage;
use qa_content::images::Palette;
use qa_content::images::PaletteLayer;
use qa_content::images::PaletteTransparency;
use qa_content::mounts::MountedContent;
use qa_core::identity::ClientId;
use qa_core::identity::IdentityOwner;
use qa_core::identity::SeatId;
use qa_core::math::vec2;
use qa_core::math::vec4;
use qa_core::math::Vec4;

use super::menu_font::load_menu_typography;
use super::menu_font::open_typography_mounts;
use super::menu_font::MenuCharsetImages;
use super::menu_font::MenuFontError;
use super::menu_font::MenuTexture;
use super::menu_font::MenuTypography;
use super::menu_font::MountedMenuFonts;
use super::menu_font::OpenedTypographyMounts;
use super::menu_font::TypographyMounts;
use super::startup_menu::menu_font_slot;
use super::startup_menu::StartupMenu;
use super::startup_menu::StartupMenuOptions;
use super::startup_menu::MENU_TITLE_FONT_SLOT;
use super::startup_saves::StartupSaveList;
use super::startup_selection::StartupSelectionModel;
use super::windowed_menu_text::conchars_rgba;
use super::windowed_menu_text::decode_menu_texture;
use super::windowed_menu_text::font_upload_linear;
use super::windowed_menu_text::font_upload_sized;
use super::windowed_menu_text::glyph_batches;
use super::windowed_menu_text::menu_font_selection;
use super::windowed_menu_text::menu_font_selection_for;
use super::windowed_menu_text::resolve_menu_charset;
use super::windowed_menu_text::FontAtlasImage;
use super::windowed_menu_text::GlyphQuad;
use super::windowed_menu_text::CONCHARS_HEIGHT;
use super::windowed_menu_text::CONCHARS_WIDTH;
use super::windowed_menu_text::FONT_PICTURE_HANDLE;

/// View clear color behind the menu (dark blue charcoal, distinct from the
/// map view's black clear).
const MENU_CLEAR_COLOR: Vec4 = Vec4 {
    x: 0.03,
    y: 0.04,
    z: 0.08,
    w: 1.0,
};

/// Embedded donor menu artwork (see `crates/app/assets/ui`): the same PNGs
/// the donor reads through `loadMenuArtImage`, decoded at open and
/// validated against the art manifest dimensions.
const MENU_BACKGROUND_PNG: &[u8] = include_bytes!("../../assets/ui/menu-background.png");
/// Embedded menu panel nine-slice art.
const MENU_PANEL_PNG: &[u8] = include_bytes!("../../assets/ui/menu-panel.png");
/// Embedded focus glow nine-slice art.
const MENU_FOCUS_PNG: &[u8] = include_bytes!("../../assets/ui/menu-focus.png");
/// Embedded main-menu backdrop art.
const MAIN_MENU_BACKGROUND_PNG: &[u8] = include_bytes!("../../assets/ui/main-menu-background.png");

/// Tag marking menu art pictures in captured emits.
const ART_BACKGROUND_TAG: u32 = u32::MAX - 3;
/// Tag marking the main-menu backdrop picture in captured emits.
const ART_MAIN_BACKGROUND_TAG: u32 = u32::MAX - 2;
/// Tag marking the menu panel picture in captured emits.
const ART_PANEL_TAG: u32 = u32::MAX - 1;
/// Tag marking the menu focus picture in captured emits.
const ART_FOCUS_TAG: u32 = u32::MAX;

/// Ordinal for the menu white image. Scene loaders allocate ordinals
/// upward from zero in their own registries, so a high ordinal cannot
/// collide with a world loaded later in the same backend.
const MENU_WHITE_ORDINAL: u32 = 0x7FFF_FF01;
/// Ordinal for the menu font atlas, uploaded beside the white image.
const MENU_FONT_ORDINAL: u32 = 0x7FFF_FF02;
/// First ordinal for proportional/TrueType atlas uploads (below the white
/// and classic ordinals so a long fallback chain cannot reach the art
/// ordinals).
const MENU_EXTRA_FONT_ORDINAL_BASE: u32 = 0x7FFF_FE00;
/// First ordinal for the four uploaded menu art images.
const MENU_ART_ORDINAL_BASE: u32 = 0x7FFF_FF10;

/// One uploaded menu art image.
struct MenuArtUpload {
    /// Backend image handle.
    image: RendererImage,
    /// Decoded level.
    level: ImageLevel,
}

/// One proportional/TrueType atlas upload behind the menu fonts.
struct ExtraFontAtlas {
    /// Headless picture handle carried by laid-out glyphs.
    handle: u32,
    /// Backend image handle.
    image: RendererImage,
    /// Atlas width in pixels.
    width: u32,
    /// Atlas height in pixels.
    height: u32,
    /// Top-down RGBA texels.
    pixels: Vec<u8>,
}

/// Menu overlay over the ported startup menu (donor frontend menu).
pub(crate) struct WindowedMenu {
    menu: StartupMenu,
    seat: SeatId,
    client: ClientId,
    font: TextFontSelection,
    title_font: TextFontSelection,
    white: RendererImage,
    font_image: RendererImage,
    font_width: u32,
    font_height: u32,
    font_pixels: Vec<u8>,
    extra_fonts: Vec<ExtraFontAtlas>,
    art: [MenuArtUpload; 4],
    uploaded: bool,
    clock_ms: Rc<Cell<i64>>,
    model: Rc<RefCell<StartupSelectionModel>>,
    menu_audio: Option<super::menu_audio::MenuAudio>,
}

impl WindowedMenu {
    /// Open the menu overlay over a selection model. `quit` is set when the
    /// menu's Quit button activates; launch actions (play, presets, saves)
    /// queue [`StartupAction`](super::startup::StartupAction) values on
    /// `launch` for the backend to drain (donor `pending`).
    pub(crate) fn open(
        model: StartupSelectionModel,
        seat: SeatId,
        client: ClientId,
        owner: ResourceOwner,
        quit: Rc<Cell<bool>>,
        launch: super::windowed_menu_launch::MenuLaunchQueue,
    ) -> Result<Self, String> {
        let (classic_selection, font_width, font_height, font_pixels) = {
            let preferred = model.options().ok().map(|options| options.product);
            match resolve_menu_charset(model.catalog(), preferred.as_deref()) {
                Some(charset) => {
                    let selection = menu_font_selection_for(charset.width, charset.height, charset.baked)
                        .map_err(|error| error.to_string())?;
                    (selection, charset.width, charset.height, charset.pixels)
                }
                None => (
                    menu_font_selection().map_err(|error| error.to_string())?,
                    CONCHARS_WIDTH,
                    CONCHARS_HEIGHT,
                    conchars_rgba(),
                ),
            }
        };
        // Donor `startup.ts`: the menu binds the typography body/title
        // selections. A typography failure (installed product but missing
        // font assets) keeps the classic charset so the menu still opens.
        let (font, title_font, stored_atlases) = {
            let store = Rc::new(RefCell::new(SharedFontStore::new()));
            let mut images = WindowedMenuImages(Rc::clone(&store));
            let mut fonts = WindowedMenuFonts(Rc::clone(&store));
            let mut mounts = WindowedMenuMounts(Rc::clone(&store));
            match load_menu_typography(
                model.catalog(),
                classic_selection.classic().clone(),
                &mut images,
                &mut fonts,
                &mut mounts,
            ) {
                Ok(typography) => {
                    let MenuTypography {
                        body, title, closer, ..
                    } = typography;
                    closer();
                    (body, title, store.borrow().atlases.clone())
                }
                Err(_) => (classic_selection.clone(), classic_selection, Vec::new()),
            }
        };
        let levels = [
            decode_embedded_art("assets/ui/menu-background.png", MENU_BACKGROUND_PNG)?,
            decode_embedded_art("assets/ui/main-menu-background.png", MAIN_MENU_BACKGROUND_PNG)?,
            decode_embedded_art("assets/ui/menu-panel.png", MENU_PANEL_PNG)?,
            decode_embedded_art("assets/ui/menu-focus.png", MENU_FOCUS_PNG)?,
        ];
        let art = {
            let authority = IdentityOwner::create("windowed-menu").map_err(|error| error.to_string())?;
            let mut images = SceneImageRegistry::new(ResourceOwner::new(11, authority.session().clone(), 0));
            let font_id = ResourceId::new("resource:windowed-menu:font").map_err(|error| error.to_string())?;
            let mut read = |path: &str| {
                levels
                    .iter()
                    .find(|level| level.0 == path)
                    .map(|level| level.1.clone())
                    .ok_or_else(|| ClientError::BadUi(format!("missing asset: {path}")))
            };
            load_native_ui_art(&font_id, &mut images, &mut read).map_err(|error| error.to_string())?
        };
        let clock_ms = Rc::new(Cell::new(0));
        let now = Rc::clone(&clock_ms);
        // Donor `startup.ts`: the menu clicks through the frontend audio.
        // Without menu mounts or click sounds the sink stays unset and the
        // menu stays silent.
        let preferred_product = model.options().ok().map(|options| options.product);
        let menu_audio =
            super::menu_audio::MenuAudio::open(model.catalog(), preferred_product.as_deref(), seat.clone());
        let shared = Rc::new(RefCell::new(model));
        let launch_play = launch.clone();
        let launch_preset = launch.clone();
        let launch_load = launch.clone();
        let menu = StartupMenu::new(StartupMenuOptions {
            lobby: None,
            sound: menu_audio.as_ref().map(|audio| audio.sink()),
            llm: None,
            clipboard: None,
            seat: seat.clone(),
            model: Rc::clone(&shared),
            art,
            font: font.clone(),
            title_font: title_font.clone(),
            now: Rc::new(move || now.get()),
            play: Rc::new(move || launch_play.push(super::startup::StartupAction::Play)),
            play_preset: Some(Rc::new(move |id, skill, arena_map| {
                launch_preset.push(super::startup::StartupAction::Preset { id, skill, arena_map });
            })),
            browser: None,
            connect: None,
            load: Rc::new(move |path| {
                launch_load.push(super::startup::StartupAction::Load {
                    path,
                    source_product: None,
                });
            }),
            saves: Rc::new(StartupSaveList::default),
            refresh_saves: Rc::new(|| {}),
            quit: Rc::new(move || quit.set(true)),
            settings: Vec::new(),
            appearance: None,
            team_arena: None,
            libraries: None,
        });
        let white = RendererImage {
            owner: owner.clone(),
            ordinal: MENU_WHITE_ORDINAL,
            source: ImageSource::Generated {
                name: "windowed-menu-white".to_string(),
            },
            width: 1,
            height: 1,
        };
        let font_image = RendererImage {
            owner: owner.clone(),
            ordinal: MENU_FONT_ORDINAL,
            source: ImageSource::Generated {
                name: "windowed-menu-font".to_string(),
            },
            width: font_width,
            height: font_height,
        };
        let art_uploads = levels.map(|(path, level)| {
            let ordinal = MENU_ART_ORDINAL_BASE + art_ordinal(path);
            MenuArtUpload {
                image: RendererImage {
                    owner: owner.clone(),
                    ordinal,
                    source: ImageSource::Generated {
                        name: format!("windowed-menu-art:{path}"),
                    },
                    width: level.width,
                    height: level.height,
                },
                level,
            }
        });
        let extra_fonts = stored_atlases
            .into_iter()
            .enumerate()
            .map(|(index, atlas)| ExtraFontAtlas {
                handle: atlas.handle,
                image: RendererImage {
                    owner: owner.clone(),
                    ordinal: MENU_EXTRA_FONT_ORDINAL_BASE + index as u32,
                    source: ImageSource::Generated {
                        name: format!("windowed-menu-font:{}", atlas.name),
                    },
                    width: atlas.width,
                    height: atlas.height,
                },
                width: atlas.width,
                height: atlas.height,
                pixels: atlas.pixels,
            })
            .collect();
        Ok(Self {
            menu,
            seat,
            client,
            font,
            title_font,
            white,
            font_image,
            font_width,
            font_height,
            font_pixels,
            extra_fonts,
            art: art_uploads,
            uploaded: false,
            clock_ms,
            model: shared,
            menu_audio,
        })
    }

    /// Drain queued menu clicks into the windowed engine (once per frame,
    /// after draw). No menu audio keeps the menu silent.
    pub(crate) fn drain_menu_audio(&mut self, audio: &mut Option<UnifiedAudio>) {
        let (Some(menu_audio), Some(engine)) = (self.menu_audio.as_mut(), audio.as_mut()) else {
            return;
        };
        menu_audio.drain(engine);
    }

    /// Borrow the ported startup menu.
    #[cfg(test)]
    pub(crate) fn menu(&self) -> &StartupMenu {
        &self.menu
    }

    /// Atlas size behind the font image.
    #[cfg(test)]
    pub(crate) fn font_atlas_size(&self) -> (u32, u32) {
        (self.font_width, self.font_height)
    }

    /// Atlas texels behind the font image.
    #[cfg(test)]
    pub(crate) fn font_atlas_pixels(&self) -> &[u8] {
        &self.font_pixels
    }

    /// Body font selection.
    #[cfg(test)]
    pub(crate) fn body_font(&self) -> &TextFontSelection {
        &self.font
    }

    /// Title font selection.
    #[cfg(test)]
    pub(crate) fn title_font_selection(&self) -> &TextFontSelection {
        &self.title_font
    }

    /// Extra atlas uploads as `(handle, width, height)` in upload order.
    #[cfg(test)]
    pub(crate) fn extra_font_atlases(&self) -> Vec<(u32, u32, u32)> {
        self.extra_fonts
            .iter()
            .map(|extra| (extra.handle, extra.width, extra.height))
            .collect()
    }

    /// Shared selection model behind the menu.
    pub(crate) fn model(&self) -> &Rc<RefCell<StartupSelectionModel>> {
        &self.model
    }

    /// Active menu id, if any.
    pub(crate) fn active_menu(&self) -> Option<qa_client::ui::types::UiMenuId> {
        self.menu.active_menu()
    }

    /// Focused control id on the active menu, if any.
    pub(crate) fn focus_control(&self) -> Option<qa_client::ui::types::UiControlId> {
        match self.menu.state().focus {
            qa_client::ui::types::SeatInputFocus::Menu { control, .. } => control,
            _ => None,
        }
    }

    /// Latch a status message on the menu.
    pub(crate) fn set_status(&self, text: &str) {
        self.menu.set_status(text, false);
    }

    /// Handle one UI input event; returns whether it was consumed.
    pub(crate) fn input(&self, event: &UiSeatInputEvent) -> bool {
        self.menu.input(event)
    }

    /// Ordered view for the menu plus the image uploads the backend must
    /// apply before executing it. Returns `None` when the live dimensions
    /// cannot host the menu, in which case the frame degrades to clear
    /// plus swap. The white and font-atlas uploads (classic plus any
    /// proportional/TrueType atlases) are emitted exactly once; later
    /// frames carry no image operations.
    pub(crate) fn frame_view(
        &mut self,
        width: i32,
        height: i32,
        seat: Option<&SeatId>,
        time_ms: f64,
    ) -> Option<(ClientRenderView, Vec<ImageResourceOperation>)> {
        if width <= 0 || height <= 0 {
            return None;
        }
        self.clock_ms.set(time_ms as i64);
        let viewport = Rect {
            x: 0.0,
            y: 0.0,
            width: width as f32,
            height: height as f32,
        };
        let context = UiDrawContext {
            binding: SeatPresentationBinding {
                seat: seat.cloned().unwrap_or_else(|| self.seat.clone()),
                client: self.client.clone(),
                viewport,
                safe_area: viewport,
                hud_scale: 1.0,
                presentation: UiPresentation {
                    doppler: UiDoppler::Disabled,
                    environment: UiEnvironment::Disabled,
                    assets: UiContentId::new("assets"),
                    hud: UiProviderRef {
                        provider: "hud".to_string(),
                        content: UiContentId::new("hud"),
                    },
                    effects: UiProviderRef {
                        provider: "fx".to_string(),
                        content: UiContentId::new("fx"),
                    },
                    audio: UiProviderRef {
                        provider: "audio".to_string(),
                        content: UiContentId::new("audio"),
                    },
                },
            },
            time_ms: time_ms as i64,
        };
        let mut handles = Vec::with_capacity(1 + self.extra_fonts.len());
        handles.push(FONT_PICTURE_HANDLE);
        handles.extend(self.extra_fonts.iter().map(|extra| extra.handle));
        let mut capture = MenuCaptureServices::new(self.font.clone(), self.title_font.clone(), handles);
        let _ignored = self.menu.draw(&context, &mut capture);
        let mut batches = menu_batches(&capture.runs, width as f32, height as f32, &self.white, &self.art);
        let mut fonts = Vec::with_capacity(1 + self.extra_fonts.len());
        fonts.push(FontAtlasImage {
            handle: FONT_PICTURE_HANDLE,
            image: self.font_image.clone(),
        });
        fonts.extend(self.extra_fonts.iter().map(|extra| FontAtlasImage {
            handle: extra.handle,
            image: extra.image.clone(),
        }));
        batches.extend(glyph_batches(&capture.glyphs, width as f32, height as f32, &fonts));
        let operations = if batches.is_empty() {
            Vec::new()
        } else {
            vec![RenderOperation::Draw(batches)]
        };
        let target = match seat {
            Some(seat) => ViewTarget::Seat((*seat).clone()),
            None => ViewTarget::Preview("windowed-menu".to_string()),
        };
        let view = ClientRenderView {
            state: RenderViewState {
                viewport: ClientRect {
                    x: 0.0,
                    y: 0.0,
                    width: viewport.width,
                    height: viewport.height,
                },
                clear: Some(ViewClear {
                    depth: 1.0,
                    color: Some(MENU_CLEAR_COLOR),
                    stencil: false,
                }),
                clip_plane: None,
            },
            target,
            time: SourceTime::Milliseconds(time_ms),
            before_view: Vec::new(),
            operations,
        };
        let mut uploads = Vec::new();
        if !self.uploaded {
            self.uploaded = true;
            uploads.push(white_upload(&self.white));
            uploads.push(font_upload_sized(
                &self.font_image,
                self.font_width,
                self.font_height,
                self.font_pixels.clone(),
            ));
            uploads.extend(
                self.extra_fonts
                    .iter()
                    .map(|extra| font_upload_linear(&extra.image, extra.width, extra.height, extra.pixels.clone())),
            );
            uploads.extend(self.art.iter().map(art_upload));
        }
        Some((view, uploads))
    }

    /// Release the uploaded white, font-atlas, and art images (no-op before
    /// the first frame).
    pub(crate) fn release_images(&self) -> Vec<ImageResourceOperation> {
        if self.uploaded {
            let mut release = vec![
                ImageResourceOperation::ReleaseImage {
                    image: self.white.clone(),
                },
                ImageResourceOperation::ReleaseImage {
                    image: self.font_image.clone(),
                },
            ];
            release.extend(
                self.extra_fonts
                    .iter()
                    .map(|extra| ImageResourceOperation::ReleaseImage {
                        image: extra.image.clone(),
                    }),
            );
            release.extend(self.art.iter().map(|upload| ImageResourceOperation::ReleaseImage {
                image: upload.image.clone(),
            }));
            release
        } else {
            Vec::new()
        }
    }
}

/// Decode one embedded menu PNG, rejecting unknown paths like the donor.
fn decode_embedded_art(path: &'static str, bytes: &[u8]) -> Result<(&'static str, ImageLevel), String> {
    let image = super::menu_art::load_menu_art_image(path, bytes).map_err(|error| error.to_string())?;
    Ok((
        path,
        ImageLevel {
            width: image.width,
            height: image.height,
            pixels: image.pixels,
        },
    ))
}

/// Ordinal slot for one art path (matches the manifest load order).
fn art_ordinal(path: &str) -> u32 {
    match path {
        "assets/ui/menu-background.png" => 0,
        "assets/ui/main-menu-background.png" => 1,
        "assets/ui/menu-panel.png" => 2,
        _ => 3,
    }
}

/// Upload operation for one menu art image (donor `loadNativeUiArt`
/// sampling: clamp with linear filtering).
fn art_upload(upload: &MenuArtUpload) -> ImageResourceOperation {
    ImageResourceOperation::CreateImage {
        image: upload.image.clone(),
        content: RenderImage::Rgba8 {
            levels: vec![upload.level.clone()],
            border_color: vec4(0.0, 0.0, 0.0, 0.0),
        },
        sampling: TextureSampling {
            repeat: false,
            filter: TextureFilter::Linear,
        },
    }
}

/// Upload operation for the menu white image (the scene loader's white
/// content and sampling).
fn white_upload(white: &RendererImage) -> ImageResourceOperation {
    ImageResourceOperation::CreateImage {
        image: white.clone(),
        content: RenderImage::Rgba8 {
            levels: vec![ImageLevel {
                width: 1,
                height: 1,
                pixels: vec![255, 255, 255, 255],
            }],
            border_color: vec4(1.0, 1.0, 1.0, 1.0),
        },
        sampling: TextureSampling {
            repeat: true,
            filter: TextureFilter::Nearest,
        },
    }
}

/// One proportional/TrueType atlas stored during typography load.
#[derive(Debug, Clone)]
struct StoredFontAtlas {
    /// Headless picture handle carried by laid-out glyphs.
    handle: u32,
    /// Atlas name for the upload label.
    name: String,
    /// Atlas width in pixels.
    width: u32,
    /// Atlas height in pixels.
    height: u32,
    /// Top-down RGBA texels.
    pixels: Vec<u8>,
}

/// Shared typography load state behind the three `menu_font` hosts:
/// the typography product mounts plus every atlas registered while
/// `load_menu_typography` runs.
struct SharedFontStore {
    /// Typography product mounts, refreshed by each mount open.
    mounts: Vec<ContentMount>,
    /// Stored atlases in registration order.
    atlases: Vec<StoredFontAtlas>,
    /// Next picture handle (the classic charset keeps
    /// [`FONT_PICTURE_HANDLE`]).
    next_handle: u32,
}

impl SharedFontStore {
    /// Empty store over no mounts.
    fn new() -> Self {
        Self {
            mounts: Vec::new(),
            atlases: Vec::new(),
            next_handle: FONT_PICTURE_HANDLE + 1,
        }
    }

    /// Store one atlas, returning its picture handle.
    fn store(&mut self, name: &str, width: u32, height: u32, pixels: Vec<u8>) -> u32 {
        let handle = self.next_handle;
        self.next_handle += 1;
        self.atlases.push(StoredFontAtlas {
            handle,
            name: name.to_string(),
            width,
            height,
            pixels,
        });
        handle
    }

    /// Forget one stored atlas.
    fn release(&mut self, image: u32) {
        self.atlases.retain(|atlas| atlas.handle != image);
    }

    /// Reopen the typography product mounts for one read.
    fn mounted(&self) -> Result<MountedContent, MenuFontError> {
        open_typography_mounts(self.mounts.clone())
    }
}

/// `MenuCharsetImages` over installed content mounts: texture reads decode
/// through the mount plan and register into the shared atlas store.
struct WindowedMenuImages(Rc<RefCell<SharedFontStore>>);

/// Static texture name for one font texture path.
fn menu_texture_name(path: &str) -> &'static str {
    match path {
        "menu/art/font1_prop.tga" => "font1_prop",
        _ => "menu-font",
    }
}

impl MenuCharsetImages for WindowedMenuImages {
    fn load_texture(
        &mut self,
        path: &str,
        _family: GameFamily,
        palette: Option<&[u8]>,
    ) -> Result<Option<MenuTexture>, MenuFontError> {
        let mounted = self.0.borrow().mounted()?;
        let asset = mounted.open(path, |_| true)?;
        let Some(asset) = asset else {
            return Ok(None);
        };
        let Some(decoded) = decode_menu_texture(&asset.bytes, path, palette) else {
            return Err(MenuFontError::Font(format!(
                "Menu font texture {path} is missing or malformed"
            )));
        };
        let picture = ImagePicture {
            image: 0,
            width: decoded.width,
            height: decoded.height,
        };
        let handle = self
            .0
            .borrow_mut()
            .store(path, decoded.width, decoded.height, decoded.pixels);
        Ok(Some(MenuTexture {
            name: menu_texture_name(path),
            picture: ImagePicture {
                image: handle,
                ..picture
            },
        }))
    }

    fn register_indexed(
        &mut self,
        name: &str,
        width: u32,
        height: u32,
        pixels: Vec<u8>,
        palette: Vec<u8>,
    ) -> Result<u32, MenuFontError> {
        // Donor Q1 `loadMenuFont` branch: indexed texels through the
        // palette with index 0 transparent.
        let render = indexed_render_image(
            vec![IndexedImage {
                width,
                height,
                indices: pixels,
            }],
            Palette {
                colors: palette,
                source: name.to_string(),
            },
            PaletteTransparency::Index(0),
            None,
            None,
        )
        .map_err(|error| MenuFontError::Font(error.to_string()))?;
        let level = expand_indexed_image(&render, 0, PaletteLayer::Combined)
            .map_err(|error| MenuFontError::Font(error.to_string()))?;
        Ok(self.0.borrow_mut().store(name, level.width, level.height, level.pixels))
    }

    fn release(&mut self, image: u32) {
        self.0.borrow_mut().release(image);
    }
}

/// `FontImageServices` over reopened typography mounts plus the shared
/// atlas store.
struct HostFontServices<'a> {
    /// Reopened typography mounts for font file reads.
    mounted: &'a MountedContent,
    /// Shared atlas store for registrations.
    store: &'a mut SharedFontStore,
}

impl FontImageServices for HostFontServices<'_> {
    fn read(&mut self, path: &str) -> Option<Vec<u8>> {
        self.mounted
            .open(path, |_| true)
            .ok()
            .flatten()
            .map(|asset| asset.bytes)
    }

    fn register_image(&mut self, name: &str, width: u32, height: u32, rgba: Vec<u8>) -> u32 {
        self.store.store(name, width, height, rgba)
    }

    fn release_image(&mut self, image: u32) {
        self.store.release(image);
    }
}

/// `MountedMenuFonts` over installed content mounts: each call runs one
/// [`TextFontRegistry`] over the typography mounts and the shared atlas
/// store (donor `createMountedTextFonts`).
struct WindowedMenuFonts(Rc<RefCell<SharedFontStore>>);

impl WindowedMenuFonts {
    /// Run one font-registry call over reopened typography mounts.
    fn with_registry<T>(
        &mut self,
        run: impl FnOnce(&mut TextFontRegistry<'_>) -> Result<T, MenuFontError>,
    ) -> Result<T, MenuFontError> {
        let mounted = self.0.borrow().mounted()?;
        let mut store = self.0.borrow_mut();
        let mut services = HostFontServices {
            mounted: &mounted,
            store: &mut store,
        };
        let mut registry = TextFontRegistry::new(&mut services);
        run(&mut registry)
    }
}

impl MountedMenuFonts for WindowedMenuFonts {
    fn select_kfont(&mut self, path: &str, classic: TextAtlas) -> Result<TextFontSelection, MenuFontError> {
        self.with_registry(
            |registry| Ok(registry.select(&TextFontRequest::Kfont { path: path.to_string() }, &classic)?),
        )
    }

    fn load_true_type(
        &mut self,
        path: &str,
        size: u32,
        codepoints: &[u32],
    ) -> Result<Option<TextAtlas>, MenuFontError> {
        self.with_registry(|registry| Ok(registry.load_truetype(path, size, codepoints)?))
    }

    fn load_true_type_pages(
        &mut self,
        path: &str,
        size: u32,
        codepoints: &[u32],
    ) -> Result<Vec<TextAtlas>, MenuFontError> {
        self.with_registry(|registry| Ok(registry.load_truetype_pages(path, size, codepoints)?))
    }

    fn close(&mut self) {}
}

/// `TypographyMounts` over installed content mounts: opens the plan and
/// remembers it so texture and font reads can reopen it.
struct WindowedMenuMounts(Rc<RefCell<SharedFontStore>>);

impl TypographyMounts for WindowedMenuMounts {
    fn open(&mut self, mounts: Vec<ContentMount>) -> Result<OpenedTypographyMounts, MenuFontError> {
        self.0.borrow_mut().mounts = mounts.clone();
        Ok((open_typography_mounts(mounts)?, Box::new(|| {})))
    }
}

/// Convert one router seat event into menu input.
pub(crate) fn convert_router_event(event: &qa_client::input::router::SeatInputEvent) -> UiSeatInputEvent {
    use qa_client::input::router::SeatInputEvent as RouterEvent;
    let (seat, time_ms, kind) = match event {
        RouterEvent::Focus { seat, time_ms, focused } => (
            seat.clone(),
            *time_ms,
            UiSeatInputEventKind::Focus { focused: *focused },
        ),
        RouterEvent::Key {
            seat,
            time_ms,
            code,
            down,
        } => (
            seat.clone(),
            *time_ms,
            UiSeatInputEventKind::Key {
                code: *code,
                down: *down,
                repeat: false,
            },
        ),
        RouterEvent::Text { seat, time_ms, text } => (
            seat.clone(),
            *time_ms,
            UiSeatInputEventKind::Text { text: text.clone() },
        ),
        RouterEvent::MouseMotion {
            seat,
            time_ms,
            position,
            delta,
        } => (
            seat.clone(),
            *time_ms,
            UiSeatInputEventKind::MouseMotion {
                position: vec2(position.0 as f32, position.1 as f32),
                delta: vec2(delta.0 as f32, delta.1 as f32),
            },
        ),
        RouterEvent::MouseButton {
            seat,
            time_ms,
            button,
            down,
        } => (
            seat.clone(),
            *time_ms,
            UiSeatInputEventKind::MouseButton {
                button: i32::from(*button),
                down: *down,
            },
        ),
        RouterEvent::MouseWheel { seat, time_ms, delta } => (
            seat.clone(),
            *time_ms,
            UiSeatInputEventKind::MouseWheel {
                delta: vec2(delta.0 as f32, delta.1 as f32),
            },
        ),
        RouterEvent::ControllerButton {
            seat,
            time_ms,
            device,
            button,
            down,
        } => (
            seat.clone(),
            *time_ms,
            UiSeatInputEventKind::ControllerButton {
                device: *device,
                button: i32::from(*button),
                down: *down,
            },
        ),
        RouterEvent::ControllerAxis {
            seat,
            time_ms,
            device,
            axis,
            value,
        } => (
            seat.clone(),
            *time_ms,
            UiSeatInputEventKind::ControllerAxis {
                device: *device,
                axis: *axis,
                value: *value as f32,
            },
        ),
    };
    UiSeatInputEvent {
        seat,
        time_ms: time_ms as i64,
        kind,
    }
}

/// One textured menu quad: fills carry degenerate UVs over the white
/// image while art quads (backdrop, nine-slice panel/focus) carry real UVs
/// over their uploaded image.
struct MenuQuad {
    /// Destination rectangle in drawable pixels.
    rect: Rect,
    /// Source coordinates.
    uv: TextureRect,
    /// Quad color.
    color: Vec4,
}

/// One emit-order run of quads over a single image: consecutive fills and
/// art quads group into runs so painter order survives batching.
struct MenuQuadRun {
    /// Art slot, or `None` for the white image.
    art: Option<usize>,
    /// Run quads in emit order.
    quads: Vec<MenuQuad>,
}

/// Capturing render services: fills and images become textured pixel quads
/// grouped into emit-order runs while text runs lay out into per-glyph
/// quads with atlas UVs.
struct MenuCaptureServices {
    runs: Vec<MenuQuadRun>,
    glyphs: Vec<GlyphQuad>,
    color: Vec4,
    body_font: TextFontSelection,
    title_font: TextFontSelection,
    fonts: Vec<u32>,
}

impl MenuCaptureServices {
    /// Capture over the menu body and title fonts. `fonts` lists every
    /// font picture handle (classic charset plus proportional/TrueType
    /// atlases); stretch quads over those handles become glyph quads.
    fn new(body_font: TextFontSelection, title_font: TextFontSelection, fonts: Vec<u32>) -> Self {
        Self {
            runs: Vec::new(),
            glyphs: Vec::new(),
            color: vec4(1.0, 1.0, 1.0, 1.0),
            body_font,
            title_font,
            fonts,
        }
    }

    /// Record one quad, skipping empty and fully transparent rects.
    fn push(&mut self, art: Option<usize>, rect: Rect, uv: TextureRect, color: Vec4) {
        if rect.width <= 0.0 || rect.height <= 0.0 || color.w <= 0.0 {
            return;
        }
        let extend = self.runs.last().is_some_and(|run| run.art == art);
        if !extend {
            self.runs.push(MenuQuadRun { art, quads: Vec::new() });
        }
        if let Some(run) = self.runs.last_mut() {
            run.quads.push(MenuQuad { rect, uv, color });
        }
    }

    /// Record one glyph quad, skipping empty and fully transparent rects.
    fn push_glyph(&mut self, font: u32, rect: Rect, uv: TextureRect, color: Vec4) {
        if rect.width <= 0.0 || rect.height <= 0.0 || color.w <= 0.0 {
            return;
        }
        self.glyphs.push(GlyphQuad { font, rect, uv, color });
    }
}

impl UiRenderServices for MenuCaptureServices {
    fn draw_text(
        &mut self,
        _context: &UiDrawContext,
        command: &UiDrawCommand,
        draw: &mut Draw2D,
    ) -> Result<(), ClientError> {
        if let UiDrawCommand::Text {
            origin,
            text,
            font,
            scale,
            color,
            align,
            shadow,
        } = command
        {
            // Donor `UiTextRenderer.draw`: one layout supplies measurement
            // and draw positions, then each visible glyph stretches its
            // atlas cell through the 2D context. The layout already
            // resolves Atlas selections per codepoint (proportional or
            // TrueType cell, else a fallback, else the classic cell), so
            // body and title text share this path for both font kinds.
            let selection = if menu_font_slot(font) == MENU_TITLE_FONT_SLOT {
                &self.title_font
            } else {
                &self.body_font
            };
            let layout = layout_text(&TextLayoutOptions {
                text,
                font: selection,
                scale: *scale,
                color: *color,
                color_codes: ColorCodes::Literal,
                force_color: false,
                alternate: false,
                max_width: None,
                align: LayoutAlign::Left,
                line_height: None,
                max_glyphs: None,
                tab_columns: 4,
            })?;
            let offset = match align {
                TextAlign::Left => 0.0,
                TextAlign::Center => layout.width / 2.0,
                TextAlign::Right => layout.width,
            };
            draw_text_layout(
                draw,
                &layout,
                vec2(origin.x - offset, origin.y),
                if *shadow { 1.0 } else { 0.0 },
            );
        }
        Ok(())
    }

    fn white(&self) -> PictureAsset {
        PictureAsset::Image(ImagePicture {
            image: 0,
            width: 1,
            height: 1,
        })
    }

    fn picture(&self, resource: &ResourceId) -> Result<PictureAsset, ClientError> {
        let tag = match resource.as_str() {
            "resource:engine-menu:background" => ART_BACKGROUND_TAG,
            "resource:engine-menu:main-background" => ART_MAIN_BACKGROUND_TAG,
            "resource:engine-menu:panel" => ART_PANEL_TAG,
            "resource:engine-menu:focus" => ART_FOCUS_TAG,
            _ => return Ok(self.white()),
        };
        Ok(PictureAsset::Image(ImagePicture {
            image: tag,
            width: 1,
            height: 1,
        }))
    }

    fn emit(&mut self, command: UiEmitCommand) {
        match command {
            UiEmitCommand::SetColor(color) => self.color = color,
            UiEmitCommand::StretchPic { rect, uv, image } => {
                if self.fonts.contains(&image.image) {
                    self.push_glyph(image.image, rect, uv, self.color);
                } else {
                    self.push(tag_art_slot(image.image), rect, uv, self.color);
                }
            }
        }
    }

    fn material(&mut self, draw: UiMaterialDraw) {
        self.push(
            None,
            draw.rect,
            TextureRect {
                s: 0.0,
                t: 0.0,
                s2: 0.0,
                t2: 0.0,
            },
            draw.color,
        );
    }
}

/// Art upload slot for one tagged picture (`None` is the white image).
fn tag_art_slot(tag: u32) -> Option<usize> {
    match tag {
        ART_BACKGROUND_TAG => Some(0),
        ART_MAIN_BACKGROUND_TAG => Some(1),
        ART_PANEL_TAG => Some(2),
        ART_FOCUS_TAG => Some(3),
        _ => None,
    }
}

/// Vertex-colored overlay batches for captured runs in NDC space (the
/// damage-blend overlay pattern: depth-always, no depth writes, blended).
/// Each run binds its own image (white for fills, one art image for the
/// backdrop) so painter order survives batching.
fn menu_batches(
    runs: &[MenuQuadRun],
    width: f32,
    height: f32,
    white: &RendererImage,
    art: &[MenuArtUpload; 4],
) -> Vec<DrawBatch> {
    let mut batches = Vec::with_capacity(runs.len());
    for run in runs {
        let mut vertices = Vec::with_capacity(run.quads.len() * 4);
        let mut indices = Vec::with_capacity(run.quads.len() * 6);
        for quad in &run.quads {
            let base = vertices.len() as u32;
            let left = 2.0 * quad.rect.x / width - 1.0;
            let right = 2.0 * (quad.rect.x + quad.rect.width) / width - 1.0;
            let top = 1.0 - 2.0 * quad.rect.y / height;
            let bottom = 1.0 - 2.0 * (quad.rect.y + quad.rect.height) / height;
            for (x, y, s, t) in [
                (left, top, quad.uv.s, quad.uv.t),
                (right, top, quad.uv.s2, quad.uv.t),
                (right, bottom, quad.uv.s2, quad.uv.t2),
                (left, bottom, quad.uv.s, quad.uv.t2),
            ] {
                vertices.push(RenderVertex {
                    position: vec4(x, y, 0.0, 1.0),
                    tex_coord: vec2(s, t),
                    color: quad.color,
                });
            }
            indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        if vertices.is_empty() {
            continue;
        }
        let texture = match run.art {
            Some(slot) => TextureBinding::BindImage(art[slot].image.clone()),
            None => TextureBinding::BindImage(white.clone()),
        };
        batches.push(DrawBatch {
            fog: None,
            luminance_alpha: false,
            indices,
            texture,
            state: RenderState {
                blend: (BlendFactor::SrcAlpha, BlendFactor::OneMinusSrcAlpha),
                depth_test: DepthTest::Always,
                depth_write: false,
                alpha_test: AlphaTest::None,
                cull: CullFace::None,
                depth_range: [0.0, 1.0],
                polygon_offset: None,
            },
            lighting: BatchLighting::Vertex,
            primitive: BatchPrimitive::Triangles,
            vertices: BatchVertices::Single(vertices),
        });
    }
    batches
}

#[cfg(test)]
mod tests {
    use qa_client::input::router::SeatInputEvent as RouterEvent;
    use qa_client::input::KeyCode;
    use qa_content::catalog::CatalogProduct;
    use qa_content::catalog::InstalledCatalog;
    use qa_content::catalog::ProductAvailability;
    use qa_content::catalog::ProductExpectation;
    use qa_content::contract::ContentId;
    use qa_content::contract::GameFamily;

    use super::super::startup::StartupEntry;
    use super::super::windowed::open_windowed_application;
    use super::super::windowed_preset::WindowedPresetCollaborators;
    use super::*;
    use crate::options::ApplicationOptions;

    fn base_product() -> CatalogProduct {
        CatalogProduct {
            id: ContentId("q2-classic-baseq2".to_string()),
            expectation: ProductExpectation {
                id: "q2-classic-baseq2".to_string(),
                family: GameFamily::Q2,
                edition: "classic".to_string(),
                campaign: "baseq2".to_string(),
                title: "Quake II".to_string(),
                content_directory: "baseq2".to_string(),
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
        }
    }

    fn catalog() -> InstalledCatalog {
        catalog_with(Vec::new())
    }

    /// Test catalog with the default product plus extras (the selection
    /// model requires `q2-classic-baseq2`).
    fn catalog_with(extra: Vec<CatalogProduct>) -> InstalledCatalog {
        let mut products = vec![base_product()];
        products.extend(extra);
        InstalledCatalog::new("/tmp/qa-windowed-menu-test".to_string(), products, Vec::new(), 0, None).unwrap()
    }

    fn menu() -> WindowedMenu {
        let authority = IdentityOwner::create("windowed-menu-test").unwrap();
        let model = StartupSelectionModel::new(
            catalog(),
            ApplicationOptions::default(),
            Box::new(WindowedPresetCollaborators),
        )
        .unwrap();
        WindowedMenu::open(
            model,
            authority.seat(0),
            authority.client(0, 0),
            ResourceOwner::new(7, authority.session().clone(), 0),
            Rc::new(Cell::new(false)),
            super::super::windowed_menu_launch::MenuLaunchQueue::new(),
        )
        .unwrap()
    }

    fn batches_of(view: &ClientRenderView) -> &[DrawBatch] {
        assert_eq!(view.operations.len(), 1);
        let RenderOperation::Draw(batches) = &view.operations[0] else {
            panic!("expected a draw operation");
        };
        batches
    }

    #[test]
    fn menu_opens_on_main_with_backdrop_and_upload() {
        let mut menu = menu();
        let expected = qa_client::ui::types::UiMenuId::new("menu:startup:main").unwrap();
        assert_eq!(menu.menu().active_menu(), Some(expected));
        assert!(menu.release_images().is_empty());
        let (view, uploads) = menu.frame_view(960, 600, None, 16.0).expect("menu view");
        assert_eq!(uploads.len(), 6);
        for upload in &uploads {
            assert!(matches!(upload, ImageResourceOperation::CreateImage { .. }));
        }
        let batches = batches_of(&view);
        // Donor draw order: backdrop art, one flat-fill run (panel,
        // divider, control fills), then the glyph batch.
        assert_eq!(batches.len(), 3);
        for batch in batches {
            assert_eq!(batch.lighting, BatchLighting::Vertex);
        }
        // Backdrop first, text last.
        let TextureBinding::BindImage(backdrop) = &batches[0].texture else {
            panic!("first batch must bind a menu image");
        };
        assert_eq!(backdrop.ordinal, MENU_ART_ORDINAL_BASE + 1);
        let BatchVertices::Single(text) = &batches.last().expect("text batch").vertices else {
            panic!("expected single-textured text vertices");
        };
        assert!(!text.is_empty() && text.len() % 4 == 0, "text draws whole glyph quads");
        assert_eq!(view.state.clear.and_then(|clear| clear.color), Some(MENU_CLEAR_COLOR));
        let (repeat, reuploads) = menu.frame_view(960, 600, None, 32.0).expect("menu view");
        assert!(reuploads.is_empty());
        assert_eq!(batches_of(&repeat), batches);
        let release = menu.release_images();
        assert_eq!(release.len(), 6);
        for release in &release {
            assert!(matches!(release, ImageResourceOperation::ReleaseImage { .. }));
        }
    }

    #[test]
    fn menu_text_batch_binds_the_font_atlas_with_glyph_uvs() {
        let mut menu = menu();
        let (view, uploads) = menu.frame_view(640, 480, None, 0.0).expect("menu view");
        assert_eq!(uploads.len(), 6);
        let batches = batches_of(&view);
        let text_batch = batches.last().expect("text batch");
        let TextureBinding::BindImage(font) = &text_batch.texture else {
            panic!("text batch must bind the font atlas image");
        };
        assert_eq!(font.ordinal, MENU_FONT_ORDINAL);
        assert_eq!((font.width, font.height), (128, 128));
        let BatchVertices::Single(vertices) = &text_batch.vertices else {
            panic!("expected single-textured text vertices");
        };
        assert!(
            vertices.len() >= 40,
            "title plus buttons emit glyphs, got {}",
            vertices.len() / 4
        );
        for quad in vertices.as_chunks::<4>().0 {
            let (s, t) = (quad[0].tex_coord.x, quad[0].tex_coord.y);
            let (s2, t2) = (quad[2].tex_coord.x, quad[2].tex_coord.y);
            assert!(s2 > s && t2 > t, "glyph UVs span an atlas cell");
            assert!(
                (0.0..=1.0).contains(&s) && (0.0..=1.0).contains(&t2),
                "glyph UVs stay in the atlas"
            );
            assert!(quad.iter().all(|vertex| vertex.color.w > 0.0), "glyphs stay opaque");
        }
    }

    #[test]
    fn menu_backdrop_covers_the_viewport() {
        let mut menu = menu();
        let (view, _) = menu.frame_view(640, 480, None, 0.0).expect("menu view");
        let batches = batches_of(&view);
        let TextureBinding::BindImage(backdrop_image) = &batches[0].texture else {
            panic!("first batch must bind a menu image");
        };
        assert_eq!(backdrop_image.ordinal, MENU_ART_ORDINAL_BASE + 1);
        assert_eq!((backdrop_image.width, backdrop_image.height), (1672, 941));
        let BatchVertices::Single(vertices) = &batches[0].vertices else {
            panic!("expected single-textured vertices");
        };
        assert_eq!(vertices.len(), 4, "backdrop is one fullscreen quad");
        let quad = vertices.as_chunks::<4>().0.first().expect("backdrop quad");
        let xs: Vec<f32> = quad.iter().map(|vertex| vertex.position.x).collect();
        let ys: Vec<f32> = quad.iter().map(|vertex| vertex.position.y).collect();
        assert_eq!(xs, vec![-1.0, 1.0, 1.0, -1.0]);
        assert_eq!(ys, vec![1.0, 1.0, -1.0, -1.0]);
        assert!(
            quad.iter().all(|vertex| vertex.color == vec4(1.0, 1.0, 1.0, 1.0)),
            "backdrop art draws untinted"
        );
        let (s, t) = (quad[0].tex_coord.x, quad[0].tex_coord.y);
        let (s2, t2) = (quad[2].tex_coord.x, quad[2].tex_coord.y);
        assert!(s2 > s && t2 > t, "backdrop UVs span the art");
    }

    #[test]
    fn menu_panel_is_a_flat_fill_without_nine_slice_art() {
        // Donor `menuPanel` is a flat fill and the menu skin leaves panel
        // and focus art unset; no batch may bind the nine-slice images.
        let mut menu = menu();
        let (view, _) = menu.frame_view(960, 600, None, 0.0).expect("menu view");
        let batches = batches_of(&view);
        let ordinals: Vec<u32> = batches
            .iter()
            .filter_map(|batch| match &batch.texture {
                TextureBinding::BindImage(image) => Some(image.ordinal),
                _ => None,
            })
            .collect();
        let panel = MENU_ART_ORDINAL_BASE + 2;
        let focus = MENU_ART_ORDINAL_BASE + 3;
        assert!(!ordinals.contains(&panel), "no panel art batch: {ordinals:?}");
        assert!(!ordinals.contains(&focus), "no focus art batch: {ordinals:?}");
        let TextureBinding::BindImage(backdrop) = &batches[0].texture else {
            panic!("first batch must bind a menu image");
        };
        assert_eq!(backdrop.ordinal, MENU_ART_ORDINAL_BASE + 1);
        let white_batches = batches
            .iter()
            .filter(|batch| {
                matches!(&batch.texture, TextureBinding::BindImage(image) if image.ordinal == MENU_WHITE_ORDINAL)
            })
            .count();
        assert!(white_batches >= 1, "panel and controls draw as flat fills");
        let BatchVertices::Single(text) = &batches.last().expect("text batch").vertices else {
            panic!("expected single-textured text vertices");
        };
        assert!(!text.is_empty() && text.len() % 4 == 0, "text draws whole glyph quads");
    }

    #[test]
    fn menu_navigation_changes_the_presented_quads() {
        let mut menu = menu();
        let seat = menu.seat.clone();
        let (before, _) = menu.frame_view(640, 480, Some(&seat), 0.0).expect("menu view");
        let before_batches = batches_of(&before).to_vec();
        let enter = UiSeatInputEvent {
            seat,
            time_ms: 0,
            kind: UiSeatInputEventKind::Key {
                code: KeyCode::Enter as i32,
                down: true,
                repeat: false,
            },
        };
        assert!(menu.input(&enter));
        let (after, _) = menu.frame_view(640, 480, None, 0.0).expect("menu view");
        assert_ne!(batches_of(&after), before_batches.as_slice());
    }

    #[test]
    fn menu_rejects_unusable_sizes() {
        let mut menu = menu();
        assert!(menu.frame_view(0, 600, None, 0.0).is_none());
        assert!(menu.frame_view(960, 0, None, 0.0).is_none());
        assert!(menu.release_images().is_empty());
    }

    #[test]
    fn menu_falls_back_to_synthetic_without_charset() {
        let menu = menu();
        assert_eq!(menu.font_atlas_size(), (128, 128));
        assert_eq!(
            menu.font_atlas_pixels(),
            super::super::windowed_menu_text::conchars_rgba().as_slice()
        );
    }

    fn scratch_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("qa-menu-typo-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        dir
    }

    fn loose_product(id: &str, family: GameFamily, edition: &str, loose_root: Option<String>) -> CatalogProduct {
        CatalogProduct {
            id: ContentId(id.to_string()),
            expectation: ProductExpectation {
                id: id.to_string(),
                family,
                edition: edition.to_string(),
                campaign: "test".to_string(),
                title: "Test".to_string(),
                content_directory: "test".to_string(),
                base_product: None,
                required_content_archives: Vec::new(),
                required_programs: Vec::new(),
                map_witness: None,
                unresolved_reason: None,
            },
            availability: ProductAvailability::Installed,
            archives: Vec::new(),
            loose_root,
            user_content: None,
            maps: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    fn open_menu(catalog: InstalledCatalog) -> WindowedMenu {
        let authority = IdentityOwner::create("windowed-menu-typo-test").unwrap();
        let model = StartupSelectionModel::new(
            catalog,
            ApplicationOptions::default(),
            Box::new(WindowedPresetCollaborators),
        )
        .unwrap();
        WindowedMenu::open(
            model,
            authority.seat(0),
            authority.client(0, 0),
            ResourceOwner::new(7, authority.session().clone(), 0),
            Rc::new(Cell::new(false)),
            super::super::windowed_menu_launch::MenuLaunchQueue::new(),
        )
        .unwrap()
    }

    fn font_batch_ordinals(view: &ClientRenderView) -> Vec<u32> {
        batches_of(view)
            .iter()
            .filter_map(|batch| match &batch.texture {
                TextureBinding::BindImage(image)
                    if image.ordinal == MENU_FONT_ORDINAL
                        || (image.ordinal >= MENU_EXTRA_FONT_ORDINAL_BASE && image.ordinal < MENU_WHITE_ORDINAL) =>
                {
                    Some(image.ordinal)
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn menu_loads_q3_proportional_typography() {
        use qa_content::images::encode_tga;
        use qa_content::images::ImageLevel as ContentLevel;
        let dir = scratch_dir("q3");
        // Real `font1_prop.tga` dims: the prop metrics address rows of a
        // 256 by 256 atlas.
        let bytes = encode_tga(&ContentLevel {
            width: 256,
            height: 256,
            pixels: vec![255u8; 256 * 256 * 4],
        });
        std::fs::create_dir_all(dir.join("menu/art")).expect("art dir");
        std::fs::write(dir.join("menu/art/font1_prop.tga"), &bytes).expect("prop font");
        let catalog = catalog_with(vec![loose_product(
            "q3-test-base",
            GameFamily::Q3,
            "classic",
            Some(dir.to_string_lossy().into_owned()),
        )]);
        let mut menu = open_menu(catalog);
        for selection in [menu.body_font(), menu.title_font_selection()] {
            let TextFontSelection::Atlas { font, fallbacks, .. } = selection else {
                panic!("Q3 menu text must use the proportional atlas");
            };
            assert_eq!(font.name, "Q3 proportional");
            assert_eq!(font.line_height, 27);
            assert!(fallbacks.is_empty());
        }
        assert_eq!(menu.extra_font_atlases(), vec![(FONT_PICTURE_HANDLE + 1, 256, 256)]);
        let (view, uploads) = menu.frame_view(640, 480, None, 0.0).expect("menu view");
        // White, classic, proportional, plus the four art uploads.
        assert_eq!(uploads.len(), 7);
        let ImageResourceOperation::CreateImage { sampling, .. } = &uploads[2] else {
            panic!("proportional upload must create its atlas image");
        };
        assert_eq!(sampling.filter, TextureFilter::Linear);
        let batches = batches_of(&view);
        assert!(batches.len() >= 3, "fills plus proportional text batches");
        let ordinals = font_batch_ordinals(&view);
        assert!(
            ordinals.contains(&MENU_EXTRA_FONT_ORDINAL_BASE),
            "text binds the proportional upload, got {ordinals:?}"
        );
        for batch in batches {
            let TextureBinding::BindImage(image) = &batch.texture else {
                continue;
            };
            if image.ordinal != MENU_EXTRA_FONT_ORDINAL_BASE {
                continue;
            }
            let BatchVertices::Single(vertices) = &batch.vertices else {
                panic!("proportional batch must be single-textured");
            };
            assert!(!vertices.is_empty() && vertices.len() % 4 == 0);
            for quad in vertices.as_chunks::<4>().0 {
                let (s, t) = (quad[0].tex_coord.x, quad[0].tex_coord.y);
                let (s2, t2) = (quad[2].tex_coord.x, quad[2].tex_coord.y);
                assert!(s2 > s && t2 > t, "proportional UVs span a glyph cell");
                assert!(
                    (0.0..=1.0).contains(&s)
                        && (0.0..=1.0).contains(&t)
                        && (0.0..=1.0).contains(&s2)
                        && (0.0..=1.0).contains(&t2),
                    "proportional UVs stay in the atlas"
                );
            }
        }
        assert_eq!(menu.release_images().len(), 7);
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn menu_falls_back_to_classic_when_proportional_font_missing() {
        let dir = scratch_dir("q3-missing");
        let catalog = catalog_with(vec![loose_product(
            "q3-test-base",
            GameFamily::Q3,
            "classic",
            Some(dir.to_string_lossy().into_owned()),
        )]);
        let mut menu = open_menu(catalog);
        assert!(matches!(menu.body_font(), TextFontSelection::Classic { .. }));
        assert!(matches!(menu.title_font_selection(), TextFontSelection::Classic { .. }));
        assert!(menu.extra_font_atlases().is_empty());
        let (view, uploads) = menu.frame_view(640, 480, None, 0.0).expect("menu view");
        assert_eq!(uploads.len(), 6);
        assert_eq!(batches_of(&view).len(), 3);
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    fn put_u16(out: &mut Vec<u8>, value: u16) {
        out.extend_from_slice(&value.to_be_bytes());
    }

    fn put_i16(out: &mut Vec<u8>, value: i16) {
        out.extend_from_slice(&value.to_be_bytes());
    }

    fn put_u32(out: &mut Vec<u8>, value: u32) {
        out.extend_from_slice(&value.to_be_bytes());
    }

    fn assemble_font(tables: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut font = Vec::new();
        put_u32(&mut font, 0x0001_0000);
        put_u16(&mut font, tables.len() as u16);
        put_u16(&mut font, 0);
        put_u16(&mut font, 0);
        put_u16(&mut font, 0);
        let mut offset = 12 + tables.len() * 16;
        for (tag, data) in tables {
            let bytes = tag.as_bytes();
            font.extend_from_slice(&[bytes[0], bytes[1], bytes[2], bytes[3]]);
            put_u32(&mut font, 0);
            put_u32(&mut font, offset as u32);
            put_u32(&mut font, data.len() as u32);
            offset += data.len();
            while !offset.is_multiple_of(4) {
                offset += 1;
            }
        }
        for (_, data) in tables {
            font.extend_from_slice(data);
            while !font.len().is_multiple_of(4) {
                font.push(0);
            }
        }
        font
    }

    /// Minimal TrueType fixture with outlines for `?` and `H` (same shape
    /// as the client atlas fixture: 1000 units per em, one rectangular
    /// glyph shared by both codepoints).
    fn fixture_ttf() -> Vec<u8> {
        let mut head = Vec::new();
        put_u32(&mut head, 0x0001_0000);
        put_u32(&mut head, 0);
        put_u32(&mut head, 0);
        put_u32(&mut head, 0x5f0f_3cf5);
        put_u16(&mut head, 0);
        put_u16(&mut head, 1000);
        head.extend_from_slice(&[0u8; 16]);
        for _ in 0..4 {
            put_i16(&mut head, 0);
        }
        put_u16(&mut head, 0);
        put_u16(&mut head, 0);
        put_i16(&mut head, 0);
        put_i16(&mut head, 1);
        put_i16(&mut head, 0);
        let mut hhea = Vec::new();
        put_u32(&mut hhea, 0x0001_0000);
        put_i16(&mut hhea, 800);
        put_i16(&mut hhea, -200);
        put_i16(&mut hhea, 0);
        put_u16(&mut hhea, 600);
        for _ in 0..6 {
            put_i16(&mut hhea, 0);
        }
        hhea.extend_from_slice(&[0u8; 8]);
        put_i16(&mut hhea, 0);
        put_u16(&mut hhea, 2);
        let mut maxp = Vec::new();
        put_u32(&mut maxp, 0x0001_0000);
        put_u16(&mut maxp, 2);
        let mut hmtx = Vec::new();
        for _ in 0..2 {
            put_u16(&mut hmtx, 600);
            put_i16(&mut hmtx, 0);
        }
        let mut subtable = Vec::new();
        put_u16(&mut subtable, 4);
        put_u16(&mut subtable, 40);
        put_u16(&mut subtable, 0);
        put_u16(&mut subtable, 6);
        put_u16(&mut subtable, 4);
        put_u16(&mut subtable, 1);
        put_u16(&mut subtable, 2);
        for end in [63u16, 72, 0xffff] {
            put_u16(&mut subtable, end);
        }
        put_u16(&mut subtable, 0);
        for start in [63u16, 72, 0xffff] {
            put_u16(&mut subtable, start);
        }
        put_i16(&mut subtable, -62);
        put_i16(&mut subtable, -71);
        put_i16(&mut subtable, 1);
        for _ in 0..3 {
            put_u16(&mut subtable, 0);
        }
        let mut cmap = Vec::new();
        put_u16(&mut cmap, 0);
        put_u16(&mut cmap, 1);
        put_u16(&mut cmap, 3);
        put_u16(&mut cmap, 1);
        put_u32(&mut cmap, 12);
        cmap.extend_from_slice(&subtable);
        let mut glyph = Vec::new();
        put_i16(&mut glyph, 1);
        put_i16(&mut glyph, 0);
        put_i16(&mut glyph, 0);
        put_i16(&mut glyph, 500);
        put_i16(&mut glyph, 700);
        put_u16(&mut glyph, 3);
        put_u16(&mut glyph, 0);
        glyph.extend_from_slice(&[1, 1, 1, 1]);
        for delta in [0i16, 500, 0, -500] {
            put_i16(&mut glyph, delta);
        }
        for delta in [0i16, 0, 700, 0] {
            put_i16(&mut glyph, delta);
        }
        let mut loca = Vec::new();
        put_u32(&mut loca, 0);
        put_u32(&mut loca, 0);
        put_u32(&mut loca, glyph.len() as u32);
        assemble_font(&[
            ("head", head),
            ("hhea", hhea),
            ("maxp", maxp),
            ("hmtx", hmtx),
            ("cmap", cmap),
            ("loca", loca),
            ("glyf", glyph),
        ])
    }

    #[test]
    fn menu_loads_rerelease_truetype_typography() {
        let dir = scratch_dir("rerelease");
        let ttf = fixture_ttf();
        std::fs::create_dir_all(dir.join("fonts")).expect("fonts dir");
        std::fs::write(dir.join("fonts/Montserrat-Regular.ttf"), &ttf).expect("body font");
        std::fs::write(dir.join("fonts/NotoSans-Bold.ttf"), &ttf).expect("title font");
        let catalog = catalog_with(vec![loose_product(
            "q2-rerelease-test",
            GameFamily::Q2,
            "rerelease",
            Some(dir.to_string_lossy().into_owned()),
        )]);
        let mut menu = open_menu(catalog);
        // Separate body (48 px) and title (72 px) rasterizations over
        // separate uploads.
        let TextFontSelection::Atlas { font: body, .. } = menu.body_font() else {
            panic!("rerelease body text must use a TrueType atlas");
        };
        let TextFontSelection::Atlas { font: title, .. } = menu.title_font_selection() else {
            panic!("rerelease title text must use a TrueType atlas");
        };
        assert!(body.glyphs.contains_key(&72));
        assert!(title.glyphs.contains_key(&72));
        assert_ne!(body.picture.image, title.picture.image);
        assert_ne!((body.picture.width, body.picture.height), (0, 0));
        let atlases = menu.extra_font_atlases();
        assert_eq!(atlases.len(), 2, "body plus title uploads, got {atlases:?}");
        assert_eq!(atlases[0].0, body.picture.image);
        assert_eq!(atlases[1].0, title.picture.image);
        let (view, uploads) = menu.frame_view(640, 480, None, 0.0).expect("menu view");
        assert_eq!(uploads.len(), 8);
        let batches = batches_of(&view);
        assert!(batches.len() >= 3, "fills plus TrueType text batches");
        // The fixture only covers `?` and `H`, so every other glyph falls
        // back to the classic charset: both uploads bind.
        let ordinals = font_batch_ordinals(&view);
        assert!(
            ordinals.contains(&MENU_FONT_ORDINAL),
            "classic fallback binds, got {ordinals:?}"
        );
        let BatchVertices::Single(text) = &batches.last().expect("text batch").vertices else {
            panic!("expected single-textured text vertices");
        };
        assert!(!text.is_empty() && text.len() % 4 == 0, "text draws whole glyph quads");
        assert_eq!(menu.release_images().len(), 8);
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn menu_font_host_registers_indexed_charsets() {
        let store = Rc::new(RefCell::new(SharedFontStore::new()));
        let mut images = WindowedMenuImages(Rc::clone(&store));
        let mut palette = vec![0u8; 768];
        palette[3..6].copy_from_slice(&[255, 255, 255]);
        let handle = images
            .register_indexed("conchars", 16, 16, vec![1u8; 16 * 16], palette)
            .expect("register");
        assert_eq!(handle, FONT_PICTURE_HANDLE + 1);
        let stored = store.borrow().atlases.clone();
        assert_eq!(stored.len(), 1);
        assert_eq!((stored[0].width, stored[0].height), (16, 16));
        assert_eq!(stored[0].pixels.len(), 16 * 16 * 4);
        assert_eq!(&stored[0].pixels[0..4], &[255, 255, 255, 255]);
        images.release(handle);
        assert!(store.borrow().atlases.is_empty());
    }

    #[test]
    fn router_events_convert_to_menu_input() {
        let authority = IdentityOwner::create("windowed-menu-convert-test").unwrap();
        let seat = authority.seat(0);
        let key = convert_router_event(&RouterEvent::Key {
            seat: seat.clone(),
            time_ms: 12.0,
            code: 27,
            down: true,
        });
        assert_eq!(key.seat, seat);
        assert_eq!(key.time_ms, 12);
        assert_eq!(
            key.kind,
            UiSeatInputEventKind::Key {
                code: 27,
                down: true,
                repeat: false,
            }
        );
        let motion = convert_router_event(&RouterEvent::MouseMotion {
            seat: seat.clone(),
            time_ms: 13.0,
            position: (3.0, 4.0),
            delta: (1.0, 0.0),
        });
        assert_eq!(
            motion.kind,
            UiSeatInputEventKind::MouseMotion {
                position: vec2(3.0, 4.0),
                delta: vec2(1.0, 0.0),
            }
        );
        let wheel = convert_router_event(&RouterEvent::MouseWheel {
            seat,
            time_ms: 14.0,
            delta: (0.0, -1.0),
        });
        assert_eq!(wheel.kind, UiSeatInputEventKind::MouseWheel { delta: vec2(0.0, -1.0) });
    }

    fn live_options() -> ApplicationOptions {
        ApplicationOptions {
            windowed: true,
            width: 480,
            height: 300,
            frame_limit: Some(3),
            corpus_root: steel_corpus_root(),
            ..ApplicationOptions::default()
        }
    }

    fn steel_corpus_root() -> String {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target")
            .to_string_lossy()
            .into_owned()
    }

    fn write_ppm(path: &std::path::Path, width: u32, height: u32, pixels: &[u8]) {
        let mut rgb = Vec::with_capacity((width * height * 3) as usize);
        for pixel in pixels.as_chunks::<4>().0 {
            rgb.extend_from_slice(&pixel[0..3]);
        }
        let mut file = std::fs::File::create(path).unwrap();
        use std::io::Write;
        file.write_all(format!("P6\n{width} {height}\n255\n").as_bytes())
            .unwrap();
        file.write_all(&rgb).unwrap();
    }

    fn capture_entry(entry: StartupEntry) -> Option<(Vec<u8>, bool)> {
        let options = live_options();
        let mut composed = match open_windowed_application(&options, entry) {
            Ok(composed) => composed,
            Err(error) => {
                assert!(!error.is_empty(), "honest open failure");
                return None;
            }
        };
        composed.app.step().expect("menu step works");
        let pixels = composed.app.capture_next_frame().expect("capture works");
        let active = composed.app.active_game();
        composed.app.close().expect("close works");
        Some((pixels, active))
    }

    #[test]
    fn live_menu_entry_presents_menu_ui() {
        let _gl_guard = super::super::windowed::WINDOWED_GL_TEST_LOCK.lock().unwrap();
        let Some((menu_pixels, menu_active)) = capture_entry(StartupEntry::Menu) else {
            return;
        };
        assert!(!menu_active, "menu entry has no active game");
        assert_eq!(menu_pixels.len(), 480 * 300 * 4);
        let Some((run_pixels, _)) = capture_entry(StartupEntry::Run) else {
            return;
        };
        assert_eq!(run_pixels.len(), menu_pixels.len());
        let differing = menu_pixels
            .iter()
            .zip(run_pixels.iter())
            .filter(|(menu, run)| menu != run)
            .count();
        let ratio = differing as f64 / menu_pixels.len() as f64;
        assert!(
            ratio > 0.25,
            "menu capture must differ from the run capture, got {ratio:.3}"
        );
        let samples = [
            (0, 0),
            (479, 0),
            (0, 299),
            (479, 299),
            (240, 0),
            (240, 299),
            (0, 150),
            (479, 150),
        ];
        let mut distinct = std::collections::BTreeSet::new();
        for (x, y) in samples {
            let at = (y * 480 + x) * 4;
            distinct.insert((menu_pixels[at], menu_pixels[at + 1], menu_pixels[at + 2]));
        }
        assert!(
            distinct.len() >= 3,
            "menu backdrop art varies across the viewport, got {distinct:?}"
        );
        let temporary = std::env::temp_dir();
        let menu_path = temporary.join("qa-wu13-menu.ppm");
        let run_path = temporary.join("qa-wu13-run.ppm");
        write_ppm(&menu_path, 480, 300, &menu_pixels);
        write_ppm(&run_path, 480, 300, &run_pixels);
        eprintln!("menu screenshot: {}", menu_path.display());
        eprintln!("run screenshot: {}", run_path.display());
    }
}
