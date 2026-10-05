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
//! donor theme), then one textured batch binding the console charset
//! atlas from [`super::windowed_menu_text`] so every glyph draws with
//! real UVs. The atlas loads the real `conchars` through installed
//! content mounts when game data is present and falls back to the
//! synthetic atlas otherwise, so the menu entry is always available,
//! with or without game content.

use std::cell::Cell;
use std::cell::RefCell;
use std::rc::Rc;

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
use qa_core::identity::ClientId;
use qa_core::identity::IdentityOwner;
use qa_core::identity::SeatId;
use qa_core::math::vec2;
use qa_core::math::vec4;
use qa_core::math::Vec4;

use super::startup_menu::menu_font_slot;
use super::startup_menu::StartupMenu;
use super::startup_menu::StartupMenuOptions;
use super::startup_menu::MENU_TITLE_FONT_SLOT;
use super::startup_saves::StartupSaveList;
use super::startup_selection::StartupSelectionModel;
use super::windowed_menu_text::conchars_rgba;
use super::windowed_menu_text::font_upload_sized;
use super::windowed_menu_text::glyph_batches;
use super::windowed_menu_text::menu_font_selection;
use super::windowed_menu_text::menu_font_selection_for;
use super::windowed_menu_text::resolve_menu_charset;
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
/// First ordinal for the four uploaded menu art images.
const MENU_ART_ORDINAL_BASE: u32 = 0x7FFF_FF10;

/// One uploaded menu art image.
struct MenuArtUpload {
    /// Backend image handle.
    image: RendererImage,
    /// Decoded level.
    level: ImageLevel,
}

/// Menu overlay over the ported startup menu (donor frontend menu).
pub(crate) struct WindowedMenu {
    menu: StartupMenu,
    seat: SeatId,
    client: ClientId,
    font: TextFontSelection,
    white: RendererImage,
    font_image: RendererImage,
    font_width: u32,
    font_height: u32,
    font_pixels: Vec<u8>,
    art: [MenuArtUpload; 4],
    uploaded: bool,
    clock_ms: Rc<Cell<i64>>,
    model: Rc<RefCell<StartupSelectionModel>>,
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
        let (font, font_width, font_height, font_pixels) = {
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
        let shared = Rc::new(RefCell::new(model));
        let launch_play = launch.clone();
        let launch_preset = launch.clone();
        let launch_load = launch.clone();
        let menu = StartupMenu::new(StartupMenuOptions {
            lobby: None,
            sound: None,
            llm: None,
            clipboard: None,
            seat: seat.clone(),
            model: Rc::clone(&shared),
            art,
            font: font.clone(),
            title_font: font.clone(),
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
        Ok(Self {
            menu,
            seat,
            client,
            font,
            white,
            font_image,
            font_width,
            font_height,
            font_pixels,
            art: art_uploads,
            uploaded: false,
            clock_ms,
            model: shared,
        })
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
    /// plus swap. The white and font-atlas uploads are emitted exactly
    /// once; later frames carry no image operations.
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
        let mut capture = MenuCaptureServices::new(self.font.clone(), self.font.clone());
        let _ignored = self.menu.draw(&context, &mut capture);
        let mut batches = menu_batches(&capture.runs, width as f32, height as f32, &self.white, &self.art);
        batches.extend(glyph_batches(
            &capture.glyphs,
            width as f32,
            height as f32,
            &self.font_image,
        ));
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
}

impl MenuCaptureServices {
    /// Capture over the menu body and title fonts.
    fn new(body_font: TextFontSelection, title_font: TextFontSelection) -> Self {
        Self {
            runs: Vec::new(),
            glyphs: Vec::new(),
            color: vec4(1.0, 1.0, 1.0, 1.0),
            body_font,
            title_font,
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
    fn push_glyph(&mut self, rect: Rect, uv: TextureRect, color: Vec4) {
        if rect.width <= 0.0 || rect.height <= 0.0 || color.w <= 0.0 {
            return;
        }
        self.glyphs.push(GlyphQuad { rect, uv, color });
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
            // atlas cell through the 2D context.
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
                if image.image == FONT_PICTURE_HANDLE {
                    self.push_glyph(rect, uv, self.color);
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

    fn catalog() -> InstalledCatalog {
        InstalledCatalog::new(
            "/tmp/qa-windowed-menu-test".to_string(),
            vec![CatalogProduct {
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
            }],
            Vec::new(),
            0,
            None,
        )
        .unwrap()
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
