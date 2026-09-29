//! Legacy UI seat input adapter (`LegacyUiSeat`).
//!
//! Donor provenance: `src/ui/common/legacy/input.ts`. One adapter and source
//! runtime belong to one local seat and loaded UI module. The donor is
//! `async` so source actions finish in SDL delivery order; this port is
//! synchronous (`Promise -> sync` on [`LegacyUiSeat::drain`] and
//! [`LegacyUiSeat::input`]) and preserves delivery order by draining the
//! accepted batch before its frame.
//!
//! Held-key tracking uses a hash map, so multi-key focus-loss release order
//! is unspecified where the donor used insertion order; all held keys are
//! still released. Wheel iteration counts match the donor's float loop bound
//! (`ceil(abs(delta))`); non-finite deltas deliver nothing instead of hanging
//! the donor's unbounded loop, and iteration clamps at 1024.

use std::collections::HashMap;

use qa_core::identity::SeatId;
use qa_core::math::Vec2;

use super::runtime::{UiKeyEvent, UiRuntime};
use crate::input::{ControllerAxis, KeyCode};
use crate::text::draw2d::Draw2D;
use crate::ui::types::{SeatInputEvent, SeatInputEventKind};
use crate::ClientError;

/// Maximum wheel clicks delivered per event (donor hangs past finiteness).
const MAX_WHEEL_CLICKS: i32 = 1024;

/// Axis deflection threshold.
const AXIS_THRESHOLD: f32 = 0.5;

/// Map a physical mouse button to a key (`mouseKey`).
fn mouse_key(button: i32) -> Option<i32> {
    match button {
        1 => Some(KeyCode::Mouse1 as i32),
        2 => Some(KeyCode::Mouse3 as i32),
        3 => Some(KeyCode::Mouse2 as i32),
        4 => Some(KeyCode::Mouse4 as i32),
        5 => Some(KeyCode::Mouse5 as i32),
        _ => None,
    }
}

/// Map a controller button to a key (`controllerKey`).
fn controller_key(button: i32) -> Option<i32> {
    match button {
        0 => Some(KeyCode::Enter as i32),
        1 | 6 => Some(KeyCode::Escape as i32),
        11 => Some(KeyCode::Up as i32),
        12 => Some(KeyCode::Down as i32),
        13 => Some(KeyCode::Left as i32),
        14 => Some(KeyCode::Right as i32),
        _ if (0..32).contains(&button) => Some(KeyCode::Joy1 as i32 + button),
        _ => None,
    }
}

/// Axis name for held-key identity (`left-x` / `left-y`).
fn axis_name(axis: ControllerAxis) -> Option<&'static str> {
    match axis {
        ControllerAxis::LeftX => Some("left-x"),
        ControllerAxis::LeftY => Some("left-y"),
        _ => None,
    }
}

/// One seat's legacy UI adapter (`LegacyUiSeat`).
pub struct LegacyUiSeat {
    /// Owning seat.
    pub seat: SeatId,
    /// Source runtime.
    pub runtime: UiRuntime,
    /// Pointer in 640x480 units.
    pointer: Vec2,
    /// Held physical inputs to key codes.
    held: HashMap<String, i32>,
    /// Accepted batch awaiting drain.
    pending: Vec<SeatInputEvent>,
    /// Window focus.
    focused: bool,
    /// Closed flag.
    closed: bool,
}

impl LegacyUiSeat {
    /// Build an adapter for a seat and runtime.
    #[must_use]
    pub fn new(seat: SeatId, runtime: UiRuntime) -> Self {
        Self {
            seat,
            runtime,
            pointer: Vec2 { x: 320.0, y: 240.0 },
            held: HashMap::new(),
            pending: Vec::new(),
            focused: true,
            closed: false,
        }
    }

    /// Current pointer (`cursor`).
    #[must_use]
    pub fn cursor(&self) -> Vec2 {
        self.pointer
    }

    /// Fail when closed.
    fn opened(&self) -> Result<(), ClientError> {
        if self.closed {
            return Err(ClientError::BadUi("Legacy UI seat is closed".to_string()));
        }
        Ok(())
    }

    /// Accept an event into the pending batch (`route`).
    ///
    /// Returns whether the event continues past focus routing (focus events
    /// return `false`, like the donor).
    pub fn route(&mut self, event: SeatInputEvent) -> Result<bool, ClientError> {
        self.opened()?;
        if event.seat != self.seat {
            return Ok(false);
        }
        let continued = !matches!(event.kind, SeatInputEventKind::Focus { .. });
        self.pending.push(event);
        Ok(continued)
    }

    /// Drain the accepted batch in order (`drain`).
    ///
    /// `Promise -> sync`.
    pub fn drain(&mut self, draw: &mut Draw2D) -> Result<(), ClientError> {
        self.opened()?;
        let pending = std::mem::take(&mut self.pending);
        for event in &pending {
            self.input(event, draw)?;
        }
        Ok(())
    }

    /// Deliver a key with held tracking (`key`).
    fn key(&mut self, physical: &str, code: Option<i32>, down: bool) -> Result<bool, ClientError> {
        let Some(code) = code else {
            return Ok(false);
        };
        if down {
            self.held.insert(physical.to_string(), code);
        } else {
            self.held.remove(physical);
        }
        self.runtime
            .handle_key(UiKeyEvent::Key { code, down }, self.pointer.x, self.pointer.y)
    }

    /// Deliver one event (`input`).
    ///
    /// `Promise -> sync`.
    pub fn input(&mut self, event: &SeatInputEvent, draw: &mut Draw2D) -> Result<bool, ClientError> {
        self.opened()?;
        if event.seat != self.seat {
            return Ok(false);
        }
        if draw.commands.seat() != &self.seat {
            return Err(ClientError::BadUi(
                "Legacy UI drawing belongs to another seat".to_string(),
            ));
        }
        self.runtime.set_display_time(event.time_ms as i32)?;
        if let SeatInputEventKind::Focus { focused } = event.kind {
            self.focused = focused;
            if !focused {
                let held: Vec<i32> = self.held.values().copied().collect();
                for code in held {
                    self.runtime
                        .handle_key(UiKeyEvent::Key { code, down: false }, self.pointer.x, self.pointer.y)?;
                }
                self.held.clear();
            }
            return Ok(false);
        }
        if !self.focused {
            return Ok(false);
        }
        match &event.kind {
            SeatInputEventKind::Key { code, down, .. } => self.key(&format!("key:{code}"), Some(*code), *down),
            SeatInputEventKind::Text { text } => {
                let mut consumed = false;
                for character in text.chars() {
                    let code = character as u32;
                    if code <= 255 {
                        consumed = self.runtime.handle_key(
                            UiKeyEvent::Character { code: code as i32 },
                            self.pointer.x,
                            self.pointer.y,
                        )? || consumed;
                    }
                }
                Ok(consumed)
            }
            SeatInputEventKind::MouseMotion { position, .. } => {
                let target = draw.commands.target();
                // NaN passes through to set_display_cursor validation (donor throws there).
                let raw_x = (position.x - target.x - draw.bias_x()) / draw.scale_x();
                let x = if raw_x.is_nan() { raw_x } else { raw_x.clamp(0.0, 640.0) };
                let raw_y = (position.y - target.y) / draw.scale_y();
                let y = if raw_y.is_nan() { raw_y } else { raw_y.clamp(0.0, 480.0) };
                self.pointer = Vec2 { x, y };
                self.runtime.set_display_cursor(x, y)?;
                self.runtime.pointer_move(x, y)
            }
            SeatInputEventKind::MouseButton { button, down } => {
                self.key(&format!("mouse:{button}"), mouse_key(*button), *down)
            }
            SeatInputEventKind::MouseWheel { delta } => {
                let mut consumed = false;
                let code = if delta.y > 0.0 {
                    KeyCode::MouseWheelUp as i32
                } else {
                    KeyCode::MouseWheelDown as i32
                };
                let clicks = if delta.y.is_finite() {
                    delta.y.abs().clamp(0.0, MAX_WHEEL_CLICKS as f32).ceil() as i32
                } else {
                    0
                };
                for _ in 0..clicks {
                    consumed = self.key("wheel", Some(code), true)? || consumed;
                    self.key("wheel", Some(code), false)?;
                }
                Ok(consumed)
            }
            SeatInputEventKind::ControllerButton { device, button, down } => {
                self.key(&format!("controller:{device}:{button}"), controller_key(*button), *down)
            }
            SeatInputEventKind::ControllerAxis { device, axis, value } => {
                let Some(name) = axis_name(*axis) else {
                    return Ok(false);
                };
                let physical = format!("axis:{device}:{name}");
                let previous = self.held.get(&physical).copied();
                let current = if value.abs() < AXIS_THRESHOLD {
                    None
                } else if *axis == ControllerAxis::LeftX {
                    Some(if *value < 0.0 {
                        KeyCode::Left as i32
                    } else {
                        KeyCode::Right as i32
                    })
                } else if *value < 0.0 {
                    Some(KeyCode::Up as i32)
                } else {
                    Some(KeyCode::Down as i32)
                };
                if current == previous {
                    return Ok(current.is_some());
                }
                let mut consumed = false;
                if let Some(previous) = previous {
                    consumed = self.key(&physical, Some(previous), false)?;
                }
                if let Some(current) = current {
                    consumed = self.key(&physical, Some(current), true)? || consumed;
                }
                Ok(consumed)
            }
            SeatInputEventKind::Focus { .. } => Ok(false),
        }
    }

    /// Close the seat and dispose its runtime (`close`).
    pub fn close(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        self.pending.clear();
        self.held.clear();
        self.runtime.dispose();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::draw2d::{CoordinateSpace, ImagePicture, PictureAsset, Rect, TextCommandSink};
    use crate::text::q3_font::{FontProfile, FontSet, GlyphMetrics, RegisteredFont, RegisteredGlyph};
    use qa_core::identity::IdentityOwner;
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::super::runtime::{
        CommandContext, PcmSound, SceneModel, SourceLocation, UiCinematicAsset, UiCinematicInstance, UiCommandBuffer,
        UiCommandOrigin, UiCvarRegistry, UiCvarValue, UiExternalScriptContext, UiExternalScriptHost, UiHandleKind,
        UiLocalSound, UiMenuDefinition, UiMenuDefinitions, UiOwnerDrawKeyResult, UiOwnerDrawPaintRequest, UiRect,
        UiRuntimeAudio, UiRuntimeBindings, UiRuntimeCinematics, UiRuntimeContext, UiRuntimeFeeder, UiRuntimeFeederItem,
        UiRuntimeOptions, UiRuntimeOwnerDraw, UiRuntimeResources, UiScriptCursor, UiWidgetAssets, UiWindowDefinition,
        UiWindowFlag,
    };

    /// Shared handle.
    type Shared<T> = Rc<RefCell<T>>;

    /// Image picture.
    fn pic(id: u32) -> PictureAsset {
        PictureAsset::Image(ImagePicture {
            image: id,
            width: 8,
            height: 8,
        })
    }

    /// Minimal cvars.
    #[derive(Debug)]
    struct Cvars {
        /// Values.
        map: Shared<HashMap<String, UiCvarValue>>,
    }

    impl UiCvarRegistry for Cvars {
        fn get(&self, name: &str) -> Option<UiCvarValue> {
            self.map.borrow().get(name).cloned()
        }

        fn set(&mut self, name: &str, value: &str, _force: bool) {
            let numeric = value.parse::<f32>().unwrap_or(0.0);
            self.map.borrow_mut().insert(
                name.to_string(),
                UiCvarValue {
                    value: value.to_string(),
                    numeric_value: numeric,
                },
            );
        }

        fn reset(&mut self, name: &str, _force: bool) {
            self.map.borrow_mut().remove(name);
        }
    }

    /// Null commands.
    #[derive(Debug)]
    struct Commands;

    impl UiCommandBuffer for Commands {
        fn append(&mut self, _text: &str, _context: &CommandContext) {}
    }

    /// Null resources.
    #[derive(Debug)]
    struct Resources;

    impl UiRuntimeResources for Resources {
        fn handle_kind(&self) -> UiHandleKind {
            UiHandleKind::Diagnostic
        }

        fn picture_handle(&mut self, _picture: Option<PictureAsset>) -> Result<i32, ClientError> {
            Err(ClientError::BadUi("diagnostic".to_string()))
        }

        fn picture_for_handle(&self, _handle: i32) -> Result<Option<PictureAsset>, ClientError> {
            Err(ClientError::BadUi("diagnostic".to_string()))
        }

        fn model_for_handle(&self, _handle: i32) -> Result<SceneModel, ClientError> {
            Err(ClientError::BadUi("diagnostic".to_string()))
        }

        fn register_font(&mut self, _path: Option<&str>, _point_size: i32) {}

        fn register_picture(&mut self, _path: Option<&str>) -> Option<PictureAsset> {
            None
        }

        fn registered_picture(&self, _path: Option<&str>) -> Option<PictureAsset> {
            None
        }

        fn register_sound(&mut self, _path: Option<&str>) -> Option<PcmSound> {
            None
        }

        fn registered_sound(&self, _path: Option<&str>) -> Option<PcmSound> {
            None
        }

        fn register_model(&mut self, _path: Option<&str>) -> SceneModel {
            SceneModel { path: None, handle: 0 }
        }

        fn registered_model(&self, _path: Option<&str>) -> Option<SceneModel> {
            None
        }

        fn prepare_cinematic(&mut self, path: &str) -> UiCinematicAsset {
            UiCinematicAsset { path: path.to_string() }
        }
    }

    /// Null audio.
    #[derive(Debug)]
    struct Audio;

    impl UiRuntimeAudio for Audio {
        fn play_local(&mut self, _sound: Option<UiLocalSound>) {}

        fn start_background(&mut self, _path: Option<&str>) {}

        fn stop_background(&mut self) {}
    }

    /// Null cinematics.
    #[derive(Debug)]
    struct Cinematics;

    impl UiRuntimeCinematics for Cinematics {
        fn play(&mut self, _asset: &UiCinematicAsset, _rect: &UiRect) -> Option<UiCinematicInstance> {
            None
        }

        fn run(&mut self, _handle: i32, _time: i32) {}

        fn draw(&mut self, _handle: i32, _rect: &UiRect, _draw: &mut Draw2D) {}

        fn stop(&mut self, _handle: i32) {}
    }

    /// Null feeder.
    #[derive(Debug)]
    struct Feeder;

    impl UiRuntimeFeeder for Feeder {
        fn count(&mut self, _feeder: f32) -> i32 {
            0
        }

        fn item(&mut self, _feeder: f32, _index: i32, _column: i32) -> Option<UiRuntimeFeederItem> {
            None
        }

        fn image(&mut self, _feeder: f32, _index: i32) -> Option<PictureAsset> {
            None
        }

        fn select(&mut self, _feeder: f32, _index: i32) {}
    }

    /// Null owner-draw.
    #[derive(Debug)]
    struct OwnerDraw;

    impl UiRuntimeOwnerDraw for OwnerDraw {
        fn visible(&mut self, _flags: i32) -> bool {
            true
        }

        fn width(&mut self, _owner_draw: i32, _scale: f32) -> i32 {
            0
        }

        fn value(&mut self, _owner_draw: i32) -> f32 {
            0.0
        }

        fn handle_key(&mut self, _owner_draw: i32, _flags: i32, special: f32, _key: i32) -> UiOwnerDrawKeyResult {
            UiOwnerDrawKeyResult {
                handled: false,
                special,
            }
        }

        fn paint(&mut self, _request: UiOwnerDrawPaintRequest<'_, '_>) {}

        fn close_cinematic(&mut self, _owner_draw: i32) {}
    }

    /// Null external scripts.
    #[derive(Debug)]
    struct External;

    impl UiExternalScriptHost for External {
        fn run(&mut self, _cursor: &mut dyn UiScriptCursor, _context: &UiExternalScriptContext) {}
    }

    /// Null bindings.
    #[derive(Debug)]
    struct Bindings {
        /// Overstrike.
        overstrike: bool,
    }

    impl UiRuntimeBindings for Bindings {
        fn key_name(&mut self, key: i32) -> String {
            format!("key{key}")
        }

        fn get_binding(&mut self, _key: i32) -> String {
            String::new()
        }

        fn set_binding(&mut self, _key: i32, _command: &str) {}

        fn get_overstrike(&mut self) -> bool {
            self.overstrike
        }

        fn set_overstrike(&mut self, enabled: bool) {
            self.overstrike = enabled;
        }
    }

    /// Glyph with no picture.
    fn glyph() -> RegisteredGlyph {
        RegisteredGlyph {
            metrics: GlyphMetrics {
                height: 8,
                top: 8,
                bottom: 0,
                pitch: 8,
                x_skip: 8,
                image_width: 8,
                image_height: 8,
                s: 0.0,
                t: 0.0,
                s2: 1.0,
                t2: 1.0,
                shader_name: String::new(),
            },
            picture: None,
        }
    }

    /// Font set with full coverage.
    fn fonts() -> FontSet {
        let font = |name: &str| RegisteredFont {
            name: name.to_string(),
            glyph_scale: 1.0,
            glyphs: vec![glyph(); 256],
        };
        FontSet {
            small: font("small"),
            normal: font("normal"),
            big: font("big"),
            profile: FontProfile::Ui,
            small_threshold: 0.5,
            big_threshold: 2.0,
        }
    }

    /// Widget pictures.
    fn widgets() -> UiWidgetAssets {
        UiWidgetAssets {
            white_shader: pic(1),
            gradient_bar: pic(2),
            scroll_bar: pic(3),
            scroll_bar_arrow_down: pic(4),
            scroll_bar_arrow_up: pic(5),
            scroll_bar_arrow_left: pic(6),
            scroll_bar_arrow_right: pic(7),
            scroll_bar_thumb: pic(8),
            slider_bar: pic(9),
            slider_thumb: pic(10),
        }
    }

    /// Seat fixture: owner seat plus a runtime with one menu.
    fn seat_fixture() -> (LegacyUiSeat, Shared<HashMap<String, UiCvarValue>>, SeatId) {
        let owner = IdentityOwner::create("seat-test").unwrap();
        let seat = owner.seat(0);
        let cvars: Shared<HashMap<String, UiCvarValue>> = Rc::new(RefCell::new(HashMap::new()));
        let mut definitions = UiMenuDefinitions::empty();
        definitions.menus = vec![UiMenuDefinition {
            location: SourceLocation {
                path: "test".to_string(),
                line: 1,
                column: 1,
            },
            source_index: 0,
            window: UiWindowDefinition {
                rect: UiRect {
                    x: 0.0,
                    y: 0.0,
                    width: 640.0,
                    height: 480.0,
                },
                client_rect: UiRect {
                    x: 0.0,
                    y: 0.0,
                    width: 640.0,
                    height: 480.0,
                },
                rect_effects: UiRect {
                    x: 0.0,
                    y: 0.0,
                    width: 0.0,
                    height: 0.0,
                },
                rect_effects2: UiRect {
                    x: 0.0,
                    y: 0.0,
                    width: 0.0,
                    height: 0.0,
                },
                name: Some("main".to_string()),
                group: None,
                cinematic: None,
                style: 0,
                border: 0,
                owner_draw: 0,
                owner_draw_flags: 0,
                border_size: 1.0,
                flags: UiWindowFlag::VISIBLE,
                next_time: 0,
                offset_time: 0,
                cinematic_handle: -1,
                fore_color: qa_core::math::Vec4 {
                    x: 1.0,
                    y: 1.0,
                    z: 1.0,
                    w: 1.0,
                },
                back_color: qa_core::math::Vec4 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                    w: 1.0,
                },
                border_color: qa_core::math::Vec4 {
                    x: 1.0,
                    y: 1.0,
                    z: 1.0,
                    w: 1.0,
                },
                outline_color: qa_core::math::Vec4 {
                    x: 0.5,
                    y: 0.5,
                    z: 0.5,
                    w: 1.0,
                },
                background: None,
                background_handle: None,
            },
            font: None,
            full_screen: 0,
            cursor_item: -1,
            font_index: 0,
            fade_cycle: 0,
            fade_clamp: 0.0,
            fade_amount: 0.0,
            on_open: None,
            on_close: None,
            on_escape: None,
            sound_loop: None,
            focus_color: qa_core::math::Vec4 {
                x: 1.0,
                y: 1.0,
                z: 0.0,
                w: 1.0,
            },
            disable_color: qa_core::math::Vec4 {
                x: 0.5,
                y: 0.5,
                z: 0.5,
                w: 1.0,
            },
            items: Vec::new(),
        }];
        let options = UiRuntimeOptions {
            definitions,
            source_parser: None,
            print: None,
            cvar_value: None,
            cvars: Box::new(Cvars { map: cvars.clone() }),
            commands: Box::new(Commands),
            command_context: CommandContext {
                session: owner.session().clone(),
                origin: UiCommandOrigin::LocalConsole,
            },
            resources: Box::new(Resources),
            fonts: fonts(),
            widget_assets: widgets(),
            zero_picture: pic(0),
            audio: Box::new(Audio),
            cinematics: Box::new(Cinematics),
            paint_model: Box::new(|_request| {}),
            context: UiRuntimeContext::Ui {
                bindings: Box::new(Bindings { overstrike: false }),
                pause: Box::new(|_paused| {}),
            },
            feeder: Box::new(Feeder),
            owner_draw: Box::new(OwnerDraw),
            external_script: Box::new(External),
            get_team_color: Box::new(|| qa_core::math::Vec4 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
                w: 1.0,
            }),
        };
        let mut runtime = UiRuntime::create(options).unwrap();
        runtime.show("main").unwrap();
        (LegacyUiSeat::new(seat.clone(), runtime), cvars, seat)
    }

    /// Draw sink bound to a seat.
    fn sink_for(seat: &SeatId) -> TextCommandSink {
        TextCommandSink::new(
            seat.clone(),
            Rect {
                x: 0.0,
                y: 0.0,
                width: 640.0,
                height: 480.0,
            },
        )
    }

    /// Input event builder.
    fn event(seat: &SeatId, kind: SeatInputEventKind) -> SeatInputEvent {
        SeatInputEvent {
            seat: seat.clone(),
            time_ms: 100,
            kind,
        }
    }

    #[test]
    fn route_accepts_own_seat() {
        let (mut seat, _cvars, id) = seat_fixture();
        let other = IdentityOwner::create("other").unwrap().seat(0);
        assert!(seat
            .route(event(
                &id,
                SeatInputEventKind::Key {
                    code: 13,
                    down: true,
                    repeat: false,
                }
            ))
            .unwrap());
        assert!(!seat
            .route(event(
                &other,
                SeatInputEventKind::Key {
                    code: 13,
                    down: true,
                    repeat: false,
                }
            ))
            .unwrap());
        assert!(!seat
            .route(event(&id, SeatInputEventKind::Focus { focused: true }))
            .unwrap());
    }

    #[test]
    fn mouse_motion_clamps_pointer() {
        let (mut seat, _cvars, id) = seat_fixture();
        let mut sink = sink_for(&id);
        let mut draw = Draw2D::new(&mut sink, CoordinateSpace::Stretch640);
        seat.input(
            &event(
                &id,
                SeatInputEventKind::MouseMotion {
                    position: Vec2 { x: 9999.0, y: -50.0 },
                    delta: Vec2 { x: 0.0, y: 0.0 },
                },
            ),
            &mut draw,
        )
        .unwrap();
        assert_eq!(seat.cursor(), Vec2 { x: 640.0, y: 0.0 });
        seat.input(
            &event(
                &id,
                SeatInputEventKind::MouseMotion {
                    position: Vec2 { x: 100.0, y: 120.0 },
                    delta: Vec2 { x: 0.0, y: 0.0 },
                },
            ),
            &mut draw,
        )
        .unwrap();
        assert_eq!(seat.cursor(), Vec2 { x: 100.0, y: 120.0 });
    }

    #[test]
    fn focus_loss_releases_and_blocks() {
        let (mut seat, _cvars, id) = seat_fixture();
        let mut sink = sink_for(&id);
        let mut draw = Draw2D::new(&mut sink, CoordinateSpace::Stretch640);
        seat.input(
            &event(
                &id,
                SeatInputEventKind::Key {
                    code: 65,
                    down: true,
                    repeat: false,
                },
            ),
            &mut draw,
        )
        .unwrap();
        assert!(!seat
            .input(&event(&id, SeatInputEventKind::Focus { focused: false }), &mut draw)
            .unwrap());
        assert!(!seat
            .input(
                &event(
                    &id,
                    SeatInputEventKind::Key {
                        code: 66,
                        down: true,
                        repeat: false,
                    },
                ),
                &mut draw,
            )
            .unwrap());
        seat.input(&event(&id, SeatInputEventKind::Focus { focused: true }), &mut draw)
            .unwrap();
        assert!(seat.held.is_empty());
    }

    #[test]
    fn wheel_delivers_click_pairs() {
        let (mut seat, _cvars, id) = seat_fixture();
        let mut sink = sink_for(&id);
        let mut draw = Draw2D::new(&mut sink, CoordinateSpace::Stretch640);
        seat.input(
            &event(
                &id,
                SeatInputEventKind::MouseWheel {
                    delta: Vec2 { x: 0.0, y: 3.0 },
                },
            ),
            &mut draw,
        )
        .unwrap();
        assert!(!seat.held.contains_key("wheel"));
        seat.input(
            &event(
                &id,
                SeatInputEventKind::MouseWheel {
                    delta: Vec2 { x: 0.0, y: 0.0 },
                },
            ),
            &mut draw,
        )
        .unwrap();
    }

    #[test]
    fn axis_threshold_routes_stick() {
        let (mut seat, _cvars, id) = seat_fixture();
        let mut sink = sink_for(&id);
        let mut draw = Draw2D::new(&mut sink, CoordinateSpace::Stretch640);
        let axis = |value: f32| {
            event(
                &id,
                SeatInputEventKind::ControllerAxis {
                    device: 0,
                    axis: ControllerAxis::LeftX,
                    value,
                },
            )
        };
        assert!(!seat.input(&axis(0.1), &mut draw).unwrap());
        // Delivered but unconsumed by the empty menu; held tracks the press.
        assert!(!seat.input(&axis(0.9), &mut draw).unwrap());
        assert!(seat.held.contains_key("axis:0:left-x"));
        assert!(seat.input(&axis(0.95), &mut draw).unwrap());
        assert!(!seat.input(&axis(0.0), &mut draw).unwrap());
        assert!(!seat.held.contains_key("axis:0:left-x"));
        assert!(!seat
            .input(
                &event(
                    &id,
                    SeatInputEventKind::ControllerAxis {
                        device: 0,
                        axis: ControllerAxis::RightX,
                        value: 1.0,
                    },
                ),
                &mut draw,
            )
            .unwrap());
    }

    #[test]
    fn drain_rejects_foreign_sink_and_closed_seat() {
        let (mut seat, _cvars, id) = seat_fixture();
        let foreign = IdentityOwner::create("foreign").unwrap().seat(0);
        let mut sink = sink_for(&foreign);
        let mut draw = Draw2D::new(&mut sink, CoordinateSpace::Stretch640);
        assert!(seat
            .input(
                &event(
                    &id,
                    SeatInputEventKind::Key {
                        code: 13,
                        down: true,
                        repeat: false,
                    },
                ),
                &mut draw,
            )
            .is_err());
        seat.close();
        assert!(seat
            .route(event(&id, SeatInputEventKind::Focus { focused: true }))
            .is_err());
        let mut sink = sink_for(&id);
        let mut draw = Draw2D::new(&mut sink, CoordinateSpace::Stretch640);
        assert!(seat.drain(&mut draw).is_err());
        seat.close();
    }

    #[test]
    fn button_tables_map() {
        assert_eq!(mouse_key(1), Some(KeyCode::Mouse1 as i32));
        assert_eq!(mouse_key(2), Some(KeyCode::Mouse3 as i32));
        assert_eq!(mouse_key(3), Some(KeyCode::Mouse2 as i32));
        assert_eq!(mouse_key(9), None);
        assert_eq!(controller_key(0), Some(KeyCode::Enter as i32));
        assert_eq!(controller_key(6), Some(KeyCode::Escape as i32));
        assert_eq!(controller_key(11), Some(KeyCode::Up as i32));
        assert_eq!(controller_key(2), Some(KeyCode::Joy1 as i32 + 2));
        assert_eq!(controller_key(33), None);
    }
}
