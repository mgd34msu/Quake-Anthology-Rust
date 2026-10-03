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
//! records fills, images, and text bars as pixel quads, which become one
//! vertex-colored [`DrawBatch`] over a 1x1 uploaded white image (the same
//! overlay pattern as the Quake II damage blend). Text renders as measured
//! placeholder bars: glyph rasterization stays out of scope, but bar bounds
//! come from the real font metrics so rows and titles sit where the menu
//! places them. Menu art and fonts are synthetic (no catalog mounts), so the
//! menu entry is always available, with or without game content.

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
use qa_client::text::atlas::classic_charset;
use qa_client::text::atlas::TextFontSelection;
use qa_client::text::draw2d::Draw2D;
use qa_client::text::draw2d::ImagePicture;
use qa_client::text::draw2d::PictureAsset;
use qa_client::text::draw2d::Rect;
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

use super::startup_menu::StartupMenu;
use super::startup_menu::StartupMenuOptions;
use super::startup_saves::StartupSaveList;
use super::startup_selection::StartupSelectionModel;

/// View clear color behind the menu (dark blue charcoal, distinct from the
/// map view's black clear).
const MENU_CLEAR_COLOR: Vec4 = Vec4 {
    x: 0.03,
    y: 0.04,
    z: 0.08,
    w: 1.0,
};

/// Stand-in for the menu backdrop art, which has no uploaded image: deep
/// warm charcoal in the spirit of the Quake menu background.
const MENU_BACKDROP_COLOR: Vec4 = Vec4 {
    x: 0.10,
    y: 0.08,
    z: 0.07,
    w: 1.0,
};

/// Tag marking backdrop image pictures in captured emits.
const BACKDROP_IMAGE_TAG: u32 = u32::MAX;

/// Ordinal for the menu white image. Scene loaders allocate ordinals
/// upward from zero in their own registries, so a high ordinal cannot
/// collide with a world loaded later in the same backend.
const MENU_WHITE_ORDINAL: u32 = 0x7FFF_FF01;

/// Notice shown when a launch action cannot start a game in this build.
const MENU_LAUNCH_NOTICE: &str = "Launching from the menu is unavailable in this build";

/// Menu overlay over the ported startup menu (donor frontend menu).
pub(crate) struct WindowedMenu {
    menu: StartupMenu,
    seat: SeatId,
    client: ClientId,
    font: TextFontSelection,
    white: RendererImage,
    uploaded: bool,
    clock_ms: Rc<Cell<i64>>,
    launch_notice: Rc<Cell<bool>>,
}

impl WindowedMenu {
    /// Open the menu overlay over a selection model. `quit` is set when the
    /// menu's Quit button activates; launch actions (play, presets, saves)
    /// latch a status notice because this build has no game client to start.
    pub(crate) fn open(
        model: StartupSelectionModel,
        seat: SeatId,
        client: ClientId,
        owner: ResourceOwner,
        quit: Rc<Cell<bool>>,
    ) -> Result<Self, String> {
        let font = TextFontSelection::Classic {
            classic: classic_charset(7, 128, 128, "conchars", true).map_err(|error| error.to_string())?,
            unicode: None,
        };
        let art = {
            let authority = IdentityOwner::create("windowed-menu").map_err(|error| error.to_string())?;
            let mut images = SceneImageRegistry::new(ResourceOwner::new(11, authority.session().clone(), 0));
            let font_id = ResourceId::new("resource:windowed-menu:font").map_err(|error| error.to_string())?;
            let mut read = |path: &str| match path {
                "assets/ui/menu-background.png" => Ok(solid(1536, 1024)),
                "assets/ui/main-menu-background.png" => Ok(solid(1672, 941)),
                "assets/ui/menu-panel.png" => Ok(solid(1254, 1254)),
                "assets/ui/menu-focus.png" => Ok(solid(2172, 724)),
                other => Err(ClientError::BadUi(format!("missing asset: {other}"))),
            };
            load_native_ui_art(&font_id, &mut images, &mut read).map_err(|error| error.to_string())?
        };
        let clock_ms = Rc::new(Cell::new(0));
        let now = Rc::clone(&clock_ms);
        let launch_notice = Rc::new(Cell::new(false));
        let notice_play = Rc::clone(&launch_notice);
        let notice_preset = Rc::clone(&launch_notice);
        let notice_load = Rc::clone(&launch_notice);
        let menu = StartupMenu::new(StartupMenuOptions {
            lobby: None,
            sound: None,
            llm: None,
            clipboard: None,
            seat: seat.clone(),
            model: Rc::new(RefCell::new(model)),
            art,
            font: font.clone(),
            title_font: font.clone(),
            now: Rc::new(move || now.get()),
            play: Rc::new(move || notice_play.set(true)),
            play_preset: Some(Rc::new(move |_, _, _| notice_preset.set(true))),
            browser: None,
            connect: None,
            load: Rc::new(move |_| notice_load.set(true)),
            saves: Rc::new(StartupSaveList::default),
            refresh_saves: Rc::new(|| {}),
            quit: Rc::new(move || quit.set(true)),
            settings: Vec::new(),
            appearance: None,
            team_arena: None,
            libraries: None,
        });
        let white = RendererImage {
            owner,
            ordinal: MENU_WHITE_ORDINAL,
            source: ImageSource::Generated {
                name: "windowed-menu-white".to_string(),
            },
            width: 1,
            height: 1,
        };
        Ok(Self {
            menu,
            seat,
            client,
            font,
            white,
            uploaded: false,
            clock_ms,
            launch_notice,
        })
    }

    /// Borrow the ported startup menu.
    #[cfg(test)]
    pub(crate) fn menu(&self) -> &StartupMenu {
        &self.menu
    }

    /// Handle one UI input event; returns whether it was consumed.
    pub(crate) fn input(&self, event: &UiSeatInputEvent) -> bool {
        self.menu.input(event)
    }

    /// Ordered view for the menu plus the image uploads the backend must
    /// apply before executing it. Returns `None` when the live dimensions
    /// cannot host the menu, in which case the frame degrades to clear
    /// plus swap. The white upload is emitted exactly once; later frames
    /// carry no image operations.
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
        if self.launch_notice.take() {
            self.menu.set_status(MENU_LAUNCH_NOTICE, false);
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
        let mut capture = MenuCaptureServices::new(self.font.clone());
        let _ignored = self.menu.draw(&context, &mut capture);
        let batches = menu_batches(&capture.quads, width as f32, height as f32, &self.white);
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
        }
        Some((view, uploads))
    }

    /// Release the uploaded white image (no-op before the first frame).
    pub(crate) fn release_images(&self) -> Vec<ImageResourceOperation> {
        if self.uploaded {
            vec![ImageResourceOperation::ReleaseImage {
                image: self.white.clone(),
            }]
        } else {
            Vec::new()
        }
    }
}

/// Solid placeholder level for synthetic menu art.
fn solid(width: u32, height: u32) -> ImageLevel {
    ImageLevel {
        width,
        height,
        pixels: vec![9; (width * height * 4) as usize],
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

/// Capturing render services: every menu draw becomes a pixel quad.
struct MenuCaptureServices {
    quads: Vec<(Rect, Vec4)>,
    color: Vec4,
    font: TextFontSelection,
}

impl MenuCaptureServices {
    /// Capture over the menu body font (used to measure text bars).
    fn new(font: TextFontSelection) -> Self {
        Self {
            quads: Vec::new(),
            color: vec4(1.0, 1.0, 1.0, 1.0),
            font,
        }
    }

    /// Record one quad, skipping empty and fully transparent rects.
    fn push(&mut self, rect: Rect, color: Vec4) {
        if rect.width <= 0.0 || rect.height <= 0.0 || color.w <= 0.0 {
            return;
        }
        self.quads.push((rect, color));
    }

    /// Measure one text run through the real font metrics.
    fn measure(&self, text: &str, scale: f32) -> (f32, f32) {
        layout_text(&TextLayoutOptions {
            text,
            font: &self.font,
            scale,
            color: vec4(1.0, 1.0, 1.0, 1.0),
            color_codes: ColorCodes::Literal,
            force_color: false,
            alternate: false,
            max_width: None,
            align: LayoutAlign::Left,
            line_height: None,
            max_glyphs: None,
            tab_columns: 4,
        })
        .map_or((text.chars().count() as f32 * 8.0 * scale, 8.0 * scale), |layout| {
            (layout.width, layout.height)
        })
    }
}

impl UiRenderServices for MenuCaptureServices {
    fn draw_text(
        &mut self,
        _context: &UiDrawContext,
        command: &UiDrawCommand,
        _draw: &mut Draw2D,
    ) -> Result<(), ClientError> {
        if let UiDrawCommand::Text {
            origin,
            text,
            scale,
            color,
            align,
            ..
        } = command
        {
            let (width, height) = self.measure(text, *scale);
            let x = match align {
                TextAlign::Left => origin.x,
                TextAlign::Center => origin.x - width / 2.0,
                TextAlign::Right => origin.x - width,
            };
            self.push(
                Rect {
                    x,
                    y: origin.y,
                    width,
                    height,
                },
                *color,
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
        if resource.as_str().contains("background") {
            return Ok(PictureAsset::Image(ImagePicture {
                image: BACKDROP_IMAGE_TAG,
                width: 1,
                height: 1,
            }));
        }
        Ok(self.white())
    }

    fn emit(&mut self, command: UiEmitCommand) {
        match command {
            UiEmitCommand::SetColor(color) => self.color = color,
            UiEmitCommand::StretchPic { rect, image, .. } => {
                if image.image == BACKDROP_IMAGE_TAG {
                    self.push(rect, MENU_BACKDROP_COLOR);
                } else {
                    self.push(rect, self.color);
                }
            }
        }
    }

    fn material(&mut self, draw: UiMaterialDraw) {
        self.push(draw.rect, draw.color);
    }
}

/// Vertex-colored overlay batches for captured quads in NDC space (the
/// damage-blend overlay pattern: depth-always, no depth writes, blended).
fn menu_batches(quads: &[(Rect, Vec4)], width: f32, height: f32, white: &RendererImage) -> Vec<DrawBatch> {
    let mut vertices = Vec::with_capacity(quads.len() * 4);
    let mut indices = Vec::with_capacity(quads.len() * 6);
    for (rect, color) in quads {
        let base = vertices.len() as u32;
        let left = 2.0 * rect.x / width - 1.0;
        let right = 2.0 * (rect.x + rect.width) / width - 1.0;
        let top = 1.0 - 2.0 * rect.y / height;
        let bottom = 1.0 - 2.0 * (rect.y + rect.height) / height;
        for (x, y) in [(left, top), (right, top), (right, bottom), (left, bottom)] {
            vertices.push(RenderVertex {
                position: vec4(x, y, 0.0, 1.0),
                tex_coord: vec2(0.0, 0.0),
                color: *color,
            });
        }
        indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    if vertices.is_empty() {
        return Vec::new();
    }
    vec![DrawBatch {
        fog: None,
        luminance_alpha: false,
        indices,
        texture: TextureBinding::BindImage(white.clone()),
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
    }]
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
    use super::super::windowed::WindowedCollaborators;
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
            Box::new(WindowedCollaborators),
        )
        .unwrap();
        WindowedMenu::open(
            model,
            authority.seat(0),
            authority.client(0, 0),
            ResourceOwner::new(7, authority.session().clone(), 0),
            Rc::new(Cell::new(false)),
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
        assert_eq!(uploads.len(), 1);
        assert!(matches!(uploads[0], ImageResourceOperation::CreateImage { .. }));
        let batches = batches_of(&view);
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].lighting, BatchLighting::Vertex);
        let BatchVertices::Single(vertices) = &batches[0].vertices else {
            panic!("expected single-textured vertices");
        };
        assert!(
            vertices.len() >= 20,
            "menu has several quads, got {}",
            vertices.len() / 4
        );
        assert_eq!(view.state.clear.and_then(|clear| clear.color), Some(MENU_CLEAR_COLOR));
        let (repeat, reuploads) = menu.frame_view(960, 600, None, 32.0).expect("menu view");
        assert!(reuploads.is_empty());
        assert_eq!(batches_of(&repeat)[0].vertices, batches[0].vertices);
        let release = menu.release_images();
        assert_eq!(release.len(), 1);
        assert!(matches!(release[0], ImageResourceOperation::ReleaseImage { .. }));
    }

    #[test]
    fn menu_backdrop_covers_the_viewport() {
        let mut menu = menu();
        let (view, _) = menu.frame_view(640, 480, None, 0.0).expect("menu view");
        let BatchVertices::Single(vertices) = &batches_of(&view)[0].vertices else {
            panic!("expected single-textured vertices");
        };
        let backdrop = vertices.as_chunks::<4>().0.iter().find(|quad| {
            quad.iter().all(|vertex| {
                (vertex.color.x - MENU_BACKDROP_COLOR.x).abs() < f32::EPSILON
                    && (vertex.color.y - MENU_BACKDROP_COLOR.y).abs() < f32::EPSILON
                    && (vertex.color.z - MENU_BACKDROP_COLOR.z).abs() < f32::EPSILON
            })
        });
        let quad = backdrop.expect("a fullscreen backdrop quad");
        let xs: Vec<f32> = quad.iter().map(|vertex| vertex.position.x).collect();
        let ys: Vec<f32> = quad.iter().map(|vertex| vertex.position.y).collect();
        assert_eq!(xs, vec![-1.0, 1.0, 1.0, -1.0]);
        assert_eq!(ys, vec![1.0, 1.0, -1.0, -1.0]);
    }

    #[test]
    fn menu_navigation_changes_the_presented_quads() {
        let mut menu = menu();
        let seat = menu.seat.clone();
        let (before, _) = menu.frame_view(640, 480, Some(&seat), 0.0).expect("menu view");
        let before_batches = batches_of(&before)[0].clone();
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
        assert_ne!(batches_of(&after)[0].vertices, before_batches.vertices);
    }

    #[test]
    fn menu_rejects_unusable_sizes() {
        let mut menu = menu();
        assert!(menu.frame_view(0, 600, None, 0.0).is_none());
        assert!(menu.frame_view(960, 0, None, 0.0).is_none());
        assert!(menu.release_images().is_empty());
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
        let backdrop = MENU_BACKDROP_COLOR;
        let expected = [
            (backdrop.x * 255.0).round() as u8,
            (backdrop.y * 255.0).round() as u8,
            (backdrop.z * 255.0).round() as u8,
        ];
        let backdrop_pixels = menu_pixels
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|pixel| {
                pixel[0].abs_diff(expected[0]) <= 2
                    && pixel[1].abs_diff(expected[1]) <= 2
                    && pixel[2].abs_diff(expected[2]) <= 2
            })
            .count();
        let backdrop_ratio = backdrop_pixels as f64 / (480.0 * 300.0);
        assert!(
            backdrop_ratio > 0.10,
            "menu backdrop must be visible, got {backdrop_ratio:.3}"
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
