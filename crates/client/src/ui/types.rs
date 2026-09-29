//! UI contracts: seat-owned menus, controls, draw commands, and HUD frames.
//!
//! Ported from the TypeScript donor's `src/contracts/ui.ts`. Every handle is
//! seat-owned: there is no default seat, and input, notification, and menu
//! state always name their seat. Layout math uses the crate's `f32` vectors
//! and [`Rect`](crate::text::draw2d::Rect); time is integer milliseconds.
//!
//! Payloads owned by other subsystems (network snapshots, movement state,
//! full playerstates) stay generic or family-tagged here so this crate never
//! duplicates their contracts; the application binds concrete types.

use std::rc::Rc;

use qa_core::identity::{ClientId, ProviderId, SeatId};
use qa_core::math::{Vec2, Vec3, Vec4};
use qa_core::time::SourceTime;

use crate::error::ClientError;
use crate::input::{ControllerAxis, InputBinding};
use crate::text::draw2d::Rect;

/// UI control identity (`ui:<menu>:<control>`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UiControlId(String);

impl UiControlId {
    /// Build an id, requiring the `ui:` namespace and a name part.
    pub fn new(id: &str) -> Result<Self, ClientError> {
        let mut parts = id.split(':');
        let namespace = parts.next().unwrap_or("");
        let name = parts.next().unwrap_or("");
        if namespace != "ui" || name.is_empty() || parts.next().is_none() {
            return Err(ClientError::BadUi(format!("invalid UI control id: {id}")));
        }
        Ok(Self(id.to_string()))
    }

    /// Borrow the id text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for UiControlId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// UI menu identity (`menu:<scope>:<name>`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UiMenuId(String);

impl UiMenuId {
    /// Build an id, requiring the `menu:` namespace and a name part.
    pub fn new(id: &str) -> Result<Self, ClientError> {
        let mut parts = id.split(':');
        let namespace = parts.next().unwrap_or("");
        let scope = parts.next().unwrap_or("");
        if namespace != "menu" || scope.is_empty() || parts.next().is_none() {
            return Err(ClientError::BadUi(format!("invalid UI menu id: {id}")));
        }
        Ok(Self(id.to_string()))
    }

    /// Borrow the id text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for UiMenuId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Content resource identity (`resource:...`), carried opaquely by UI code.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResourceId(String);

impl ResourceId {
    /// Build an id, requiring the `resource:` namespace.
    pub fn new(id: &str) -> Result<Self, ClientError> {
        if !id.starts_with("resource:") || id.len() <= "resource:".len() {
            return Err(ClientError::BadUi(format!("invalid resource id: {id}")));
        }
        Ok(Self(id.to_string()))
    }

    /// Borrow the id text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ResourceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Content identity, carried opaquely by UI code.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ContentId(String);

impl ContentId {
    /// Build an id.
    #[must_use]
    pub fn new(id: &str) -> Self {
        Self(id.to_string())
    }

    /// Borrow the id text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Gameplay item identity, carried opaquely by UI code.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ItemId(String);

impl ItemId {
    /// Build an id.
    #[must_use]
    pub fn new(id: &str) -> Self {
        Self(id.to_string())
    }

    /// Borrow the id text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Provider plus content reference.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProviderRef {
    /// Provider name (`namespace:name` text).
    pub provider: String,
    /// Content identity.
    pub content: ContentId,
}

/// Content plus path reference.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResourceRequest {
    /// Content identity.
    pub content: ContentId,
    /// Path within the content.
    pub path: String,
}

/// Doppler presentation selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DopplerSelection {
    /// Source audio.
    Source,
    /// Disabled.
    Disabled,
}

/// Environment presentation selection.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EnvironmentSelection {
    /// Audio content.
    AudioContent,
    /// Disabled.
    Disabled,
    /// Selected resource.
    Selected {
        /// Resource.
        resource: ResourceRequest,
    },
}

/// Seat presentation selection.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PresentationSelection {
    /// Doppler selection.
    pub doppler: DopplerSelection,
    /// Environment selection.
    pub environment: EnvironmentSelection,
    /// Asset content.
    pub assets: ContentId,
    /// HUD provider.
    pub hud: ProviderRef,
    /// Effects provider.
    pub effects: ProviderRef,
    /// Audio provider.
    pub audio: ProviderRef,
}

/// Command dialect selecting per-family UI syntax.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommandDialect {
    /// Q1 NetQuake.
    Q1Netquake,
    /// Q1 QuakeWorld.
    Q1Quakeworld,
    /// Q2 classic.
    Q2Classic,
    /// Q2 rerelease.
    Q2Rerelease,
    /// Q3.
    Q3,
}

impl CommandDialect {
    /// Donor dialect name.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            CommandDialect::Q1Netquake => "q1-netquake",
            CommandDialect::Q1Quakeworld => "q1-quakeworld",
            CommandDialect::Q2Classic => "q2-classic",
            CommandDialect::Q2Rerelease => "q2-rerelease",
            CommandDialect::Q3 => "q3",
        }
    }

    /// Whether this is a Q1 dialect.
    #[must_use]
    pub fn is_q1(&self) -> bool {
        matches!(self, CommandDialect::Q1Netquake | CommandDialect::Q1Quakeworld)
    }

    /// Whether this is a Q2 dialect.
    #[must_use]
    pub fn is_q2(&self) -> bool {
        matches!(self, CommandDialect::Q2Classic | CommandDialect::Q2Rerelease)
    }

    /// Parse a donor dialect name.
    pub fn parse(text: &str) -> Result<Self, ClientError> {
        match text {
            "q1-netquake" => Ok(CommandDialect::Q1Netquake),
            "q1-quakeworld" => Ok(CommandDialect::Q1Quakeworld),
            "q2-classic" => Ok(CommandDialect::Q2Classic),
            "q2-rerelease" => Ok(CommandDialect::Q2Rerelease),
            "q3" => Ok(CommandDialect::Q3),
            _ => Err(ClientError::BadUi(format!("unknown command dialect: {text}"))),
        }
    }
}

/// Q2 protocol family selecting HUD layout grammar and configstring tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q2ProtocolFamily {
    /// Classic (34).
    Classic,
    /// R1Q2 (35).
    R1Q2,
    /// Q2Pro (36).
    Rerelease,
    /// KEX (2023).
    Kex,
    /// KEX demo (2022).
    KexDemo,
    /// Private classic (4038, classic game layout).
    PrivateClassic,
}

impl Q2ProtocolFamily {
    /// Whether the rerelease HUD grammar applies.
    #[must_use]
    pub fn is_rerelease(&self) -> bool {
        matches!(
            self,
            Q2ProtocolFamily::Rerelease | Q2ProtocolFamily::Kex | Q2ProtocolFamily::KexDemo
        )
    }

    /// Configstring table layout (`q2ApplicationLayout`).
    #[must_use]
    pub fn layout(&self) -> Q2ConfigLayout {
        if self.is_rerelease() {
            Q2ConfigLayout {
                models: 62,
                sounds: 8254,
                images: 10302,
                lights: 10814,
                items: 11326,
                player_skins: 11582,
                max_models: 8192,
                max_sounds: 2048,
                max_images: 512,
                max_config_strings: 12448,
                map_checksum: 61,
                max_clients: 60,
                air_accelerate: 59,
                n64_physics: Some(12103),
            }
        } else {
            Q2ConfigLayout {
                models: 32,
                sounds: 288,
                images: 544,
                lights: 800,
                items: 1056,
                player_skins: 1312,
                max_models: 256,
                max_sounds: 256,
                max_images: 256,
                max_config_strings: 2080,
                map_checksum: 31,
                max_clients: 30,
                air_accelerate: 29,
                n64_physics: None,
            }
        }
    }
}

/// Q2 configstring table layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Q2ConfigLayout {
    /// Models base.
    pub models: i32,
    /// Sounds base.
    pub sounds: i32,
    /// Images base.
    pub images: i32,
    /// Lights base.
    pub lights: i32,
    /// Items base.
    pub items: i32,
    /// Player skins base.
    pub player_skins: i32,
    /// Model slots.
    pub max_models: i32,
    /// Sound slots.
    pub max_sounds: i32,
    /// Image slots.
    pub max_images: i32,
    /// Configstring slots.
    pub max_config_strings: i32,
    /// Map checksum index.
    pub map_checksum: i32,
    /// Max clients index.
    pub max_clients: i32,
    /// Air accelerate index.
    pub air_accelerate: i32,
    /// N64 physics index, rerelease only.
    pub n64_physics: Option<i32>,
}

/// One seat input event.
#[derive(Debug, Clone, PartialEq)]
pub struct SeatInputEvent {
    /// Owning seat.
    pub seat: SeatId,
    /// Event time in milliseconds.
    pub time_ms: i64,
    /// Event payload.
    pub kind: SeatInputEventKind,
}

/// Seat input event payload.
#[derive(Debug, Clone, PartialEq)]
pub enum SeatInputEventKind {
    /// Key press or release.
    Key {
        /// Key code.
        code: i32,
        /// Press (`true`) or release (`false`).
        down: bool,
        /// Auto-repeat.
        repeat: bool,
    },
    /// Text input.
    Text {
        /// Text.
        text: String,
    },
    /// Mouse motion.
    MouseMotion {
        /// Position in drawable pixels.
        position: Vec2,
        /// Delta in drawable pixels.
        delta: Vec2,
    },
    /// Mouse button.
    MouseButton {
        /// Physical button.
        button: i32,
        /// Press (`true`) or release (`false`).
        down: bool,
    },
    /// Mouse wheel.
    MouseWheel {
        /// Delta.
        delta: Vec2,
    },
    /// Controller button.
    ControllerButton {
        /// Device index.
        device: i32,
        /// Button index.
        button: i32,
        /// Press (`true`) or release (`false`).
        down: bool,
    },
    /// Controller axis.
    ControllerAxis {
        /// Device index.
        device: i32,
        /// Axis.
        axis: ControllerAxis,
        /// Deflection in `[-1, 1]`.
        value: f32,
    },
    /// Window focus.
    Focus {
        /// Focused.
        focused: bool,
    },
}

/// Where a seat's input currently goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeatInputFocus {
    /// In-game input.
    Game,
    /// Menu input.
    Menu {
        /// Menu id.
        menu: UiMenuId,
        /// Focused control, if any.
        control: Option<UiControlId>,
    },
    /// Console input.
    Console,
    /// Chat input.
    Chat {
        /// Team chat.
        team: bool,
        /// Current text.
        text: String,
    },
}

/// A notification line on one seat.
#[derive(Debug, Clone, PartialEq)]
pub struct UiNotification {
    /// Seat sequence number.
    pub sequence: u64,
    /// Text.
    pub text: String,
    /// Chat (`true`) or notice (`false`).
    pub chat: bool,
    /// Start time.
    pub starts: SourceTime,
    /// Duration.
    pub duration: SourceTime,
}

/// Center-print state on one seat.
#[derive(Debug, Clone, PartialEq)]
pub struct CenterPrintState {
    /// Text.
    pub text: String,
    /// Start time.
    pub starts: SourceTime,
    /// Duration.
    pub duration: SourceTime,
    /// Instant (`true`) or typewriter (`false`).
    pub instant: bool,
    /// Milliseconds per character for typewriter prints.
    pub character_ms: Option<f64>,
}

/// Full seat UI state.
#[derive(Debug, Clone, PartialEq)]
pub struct SeatUiState {
    /// Owning seat.
    pub seat: SeatId,
    /// Input focus.
    pub focus: SeatInputFocus,
    /// Cursor in UI units.
    pub cursor: Vec2,
    /// Seat bindings.
    pub bindings: Vec<InputBinding>,
    /// Notifications.
    pub notifications: Vec<UiNotification>,
    /// Center print, if any.
    pub center_print: Option<CenterPrintState>,
    /// Scoreboard visible.
    pub show_scores: bool,
}

/// Seat presentation binding: viewport, safe area, scale, and selection.
#[derive(Debug, Clone, PartialEq)]
pub struct SeatPresentationBinding {
    /// Owning seat.
    pub seat: SeatId,
    /// Client.
    pub client: ClientId,
    /// Viewport in drawable pixels.
    pub viewport: Rect,
    /// Safe area in drawable pixels.
    pub safe_area: Rect,
    /// Seat HUD scale.
    pub hud_scale: f32,
    /// Presentation selection.
    pub presentation: PresentationSelection,
}

/// Text alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextAlign {
    /// Left.
    Left,
    /// Center.
    Center,
    /// Right.
    Right,
}

/// One UI draw command in absolute drawable pixels.
#[derive(Debug, Clone, PartialEq)]
pub enum UiDrawCommand {
    /// Filled rectangle.
    Fill {
        /// Rectangle.
        rect: Rect,
        /// Color.
        color: Vec4,
    },
    /// Image rectangle.
    Image {
        /// Rectangle.
        rect: Rect,
        /// Resource.
        resource: ResourceId,
        /// Texture coordinates.
        tex_coords: [Vec2; 2],
        /// Color.
        color: Vec4,
    },
    /// Text run.
    Text {
        /// Origin.
        origin: Vec2,
        /// Text.
        text: String,
        /// Font resource.
        font: ResourceId,
        /// Scale.
        scale: f32,
        /// Color.
        color: Vec4,
        /// Alignment.
        align: TextAlign,
        /// Drop shadow.
        shadow: bool,
    },
    /// Scissor clip (`None` clears).
    Clip {
        /// Clip rectangle.
        rect: Option<Rect>,
    },
}

/// Context for one UI draw call.
#[derive(Debug, Clone, PartialEq)]
pub struct UiDrawContext {
    /// Seat binding.
    pub binding: SeatPresentationBinding,
    /// Draw time in milliseconds.
    pub time_ms: i64,
}

/// Choice-list entry.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UiChoice {
    /// Choice id.
    pub id: String,
    /// Label.
    pub label: String,
}

/// Callback receiving only its seat.
pub type SeatCallback = Rc<dyn Fn(SeatId)>;
/// Callback receiving a seat and a toggle state.
pub type SeatCheckedCallback = Rc<dyn Fn(SeatId, bool)>;
/// Callback receiving a seat and a slider value.
pub type SeatValueCallback = Rc<dyn Fn(SeatId, f32)>;
/// Callback receiving a seat and text.
pub type SeatTextCallback = Rc<dyn Fn(SeatId, &str)>;
/// Callback receiving a seat and a selected id.
pub type SeatChoiceCallback = Rc<dyn Fn(SeatId, &str)>;
/// Owner-draw callback producing draw commands.
pub type OwnerDrawCallback = Rc<dyn Fn(&UiDrawContext) -> Vec<UiDrawCommand>>;
/// Owner-draw key callback; returns whether the key was consumed.
pub type OwnerKeyCallback = Rc<dyn Fn(SeatId, i32, bool) -> bool>;

/// Per-row action button in a list.
#[derive(Clone)]
pub struct ListRowAction {
    /// Label.
    pub label: String,
    /// Activation callback.
    pub on_activate: SeatCallback,
}

impl std::fmt::Debug for ListRowAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListRowAction").field("label", &self.label).finish_non_exhaustive()
    }
}

/// One list row.
#[derive(Clone)]
pub struct UiListRow {
    /// Row action, if any.
    pub action: Option<ListRowAction>,
    /// Row id.
    pub id: String,
    /// Cell texts.
    pub cells: Vec<String>,
    /// Row image, if any.
    pub image: Option<ResourceId>,
    /// Enabled.
    pub enabled: bool,
}

impl std::fmt::Debug for UiListRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UiListRow")
            .field("id", &self.id)
            .field("cells", &self.cells)
            .field("enabled", &self.enabled)
            .finish_non_exhaustive()
    }
}

/// UI control payload.
#[derive(Clone)]
pub enum UiControlKind {
    /// Button.
    Button {
        /// Activation callback.
        on_activate: SeatCallback,
    },
    /// Toggle.
    Toggle {
        /// Checked.
        checked: bool,
        /// Change callback.
        on_change: SeatCheckedCallback,
    },
    /// Slider.
    Slider {
        /// Minimum.
        minimum: f32,
        /// Maximum.
        maximum: f32,
        /// Step.
        step: f32,
        /// Value.
        value: f32,
        /// Value label override.
        value_label: Option<String>,
        /// Change callback.
        on_change: SeatValueCallback,
    },
    /// Text entry.
    TextEntry {
        /// Masked (password).
        masked: bool,
        /// Text.
        text: String,
        /// Maximum length in characters.
        maximum_length: usize,
        /// Change callback.
        on_change: SeatTextCallback,
        /// Submit callback.
        on_submit: SeatTextCallback,
    },
    /// Choice list.
    Choice {
        /// Choices.
        choices: Vec<UiChoice>,
        /// Selected id, if any.
        selected: Option<String>,
        /// Select callback.
        on_select: SeatChoiceCallback,
    },
    /// Row list.
    List {
        /// Row height override.
        row_height: Option<f32>,
        /// Column widths override.
        column_widths: Option<Vec<f32>>,
        /// Row activation callback.
        on_activate: Option<SeatChoiceCallback>,
        /// Rows.
        rows: Vec<UiListRow>,
        /// Selected row id, if any.
        selected: Option<String>,
        /// Select callback.
        on_select: SeatChoiceCallback,
    },
    /// Owner-drawn control; the source module draws and handles keys.
    OwnerDraw {
        /// Owning provider.
        owner: ProviderId,
        /// Source id.
        source_id: i32,
        /// Draw callback.
        on_draw: OwnerDrawCallback,
        /// Key callback.
        on_key: OwnerKeyCallback,
    },
}

impl std::fmt::Debug for UiControlKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UiControlKind::Button { .. } => write!(f, "Button"),
            UiControlKind::Toggle { checked, .. } => {
                f.debug_struct("Toggle").field("checked", checked).finish_non_exhaustive()
            }
            UiControlKind::Slider { value, .. } => {
                f.debug_struct("Slider").field("value", value).finish_non_exhaustive()
            }
            UiControlKind::TextEntry { text, .. } => {
                f.debug_struct("TextEntry").field("text", text).finish_non_exhaustive()
            }
            UiControlKind::Choice { selected, .. } => {
                f.debug_struct("Choice").field("selected", selected).finish_non_exhaustive()
            }
            UiControlKind::List { selected, .. } => {
                f.debug_struct("List").field("selected", selected).finish_non_exhaustive()
            }
            UiControlKind::OwnerDraw { source_id, .. } => {
                f.debug_struct("OwnerDraw").field("source_id", source_id).finish_non_exhaustive()
            }
        }
    }
}

/// One menu control.
#[derive(Clone)]
pub struct UiControl {
    /// Control id.
    pub id: UiControlId,
    /// Label.
    pub label: String,
    /// Rectangle in UI units.
    pub rect: Rect,
    /// Enabled.
    pub enabled: bool,
    /// Visible.
    pub visible: bool,
    /// Payload.
    pub kind: UiControlKind,
}

impl std::fmt::Debug for UiControl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UiControl")
            .field("id", &self.id)
            .field("label", &self.label)
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

/// Scrollable control region within a menu.
#[derive(Debug, Clone, PartialEq)]
pub struct UiMenuScroll {
    /// Visible rectangle in UI units.
    pub rect: Rect,
    /// Content height in UI units.
    pub content_height: f32,
    /// Scrolled control ids.
    pub controls: Vec<UiControlId>,
}

/// One menu.
#[derive(Clone)]
pub struct UiMenu {
    /// Scroll region, if any.
    pub scroll: Option<UiMenuScroll>,
    /// Menu id.
    pub id: UiMenuId,
    /// Title.
    pub title: String,
    /// Full screen.
    pub full_screen: bool,
    /// Controls.
    pub controls: Vec<UiControl>,
    /// Open callback.
    pub on_open: SeatCallback,
    /// Close callback.
    pub on_close: SeatCallback,
}

impl std::fmt::Debug for UiMenu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UiMenu")
            .field("id", &self.id)
            .field("title", &self.title)
            .finish_non_exhaustive()
    }
}

/// Legacy Q3 menu script executed through the source module.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LegacyUiScript {
    /// Owning provider module.
    pub module: ProviderId,
    /// Script source.
    pub source: String,
}

/// Legacy feeder backing a native list control.
#[derive(Clone)]
pub struct LegacyUiFeeder {
    /// Owning provider module.
    pub module: ProviderId,
    /// Source id.
    pub source_id: i32,
    /// Row count.
    pub count: Rc<dyn Fn(SeatId) -> usize>,
    /// Row lookup.
    pub row: Rc<dyn Fn(SeatId, usize) -> Option<UiListRow>>,
    /// Row selection.
    pub select: Rc<dyn Fn(SeatId, usize)>,
}

impl std::fmt::Debug for LegacyUiFeeder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LegacyUiFeeder").field("source_id", &self.source_id).finish_non_exhaustive()
    }
}

/// Seat UI controller surface.
pub trait SeatUiController {
    /// Owning seat.
    fn seat(&self) -> SeatId;
    /// Current state.
    fn state(&self) -> SeatUiState;
    /// Handle one input event; returns whether it was consumed.
    fn input(&mut self, event: &SeatInputEvent) -> Result<bool, ClientError>;
    /// Open a menu.
    fn open_menu(&mut self, menu: &UiMenuId) -> Result<(), ClientError>;
    /// Close the top menu.
    fn close_menu(&mut self);
    /// Execute a legacy script.
    fn execute_script(&mut self, script: LegacyUiScript);
    /// Draw the active menu.
    fn draw(&mut self, context: &UiDrawContext) -> Result<Vec<UiDrawCommand>, ClientError>;
}

/// Network snapshot family carried by a HUD frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SnapshotFamily {
    /// Q1.
    Q1,
    /// QuakeWorld.
    Quakeworld,
    /// Q2 classic.
    Q2,
    /// Q2 rerelease.
    Q2Rerelease,
    /// Q3.
    Q3,
}

/// Network snapshot carried by a HUD frame; contents stay net-owned.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NetworkSnapshot {
    /// Snapshot family.
    pub family: SnapshotFamily,
    /// Server time in milliseconds.
    pub server_time_ms: i64,
}

/// One HUD frame.
#[derive(Debug, Clone, PartialEq)]
pub struct HudFrame<Snapshot = NetworkSnapshot> {
    /// Seat binding.
    pub binding: SeatPresentationBinding,
    /// Frame time.
    pub time: SourceTime,
    /// Network snapshot.
    pub snapshot: Snapshot,
    /// Seat UI state.
    pub ui: SeatUiState,
}

/// HUD provider drawing one frame of commands.
pub trait HudProvider<Snapshot = NetworkSnapshot> {
    /// Provider id.
    fn id(&self) -> &ProviderId;
    /// Draw one frame.
    fn draw(&self, frame: &HudFrame<Snapshot>) -> Vec<UiDrawCommand>;
}

/// Q2 server data for one HUD draw: layout string plus inventory counts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2HudServerData {
    /// Layout string.
    pub layout: String,
    /// Inventory counts.
    pub inventory: Vec<i32>,
}

/// Q2 HUD draw frame. Draw imports bind the selected seat before invoking
/// the source module; the player type stays net-owned.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2HudDraw<Player> {
    /// Owning seat.
    pub seat: SeatId,
    /// Server data.
    pub server_data: Q2HudServerData,
    /// Viewport in drawable pixels.
    pub viewport: Rect,
    /// Safe area in drawable pixels.
    pub safe_area: Rect,
    /// Scale.
    pub scale: f32,
    /// Zero-based player number.
    pub player_number: i32,
    /// Player state.
    pub player: Player,
}

/// Q2 rerelease cgame API identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Q2CgameApiIdentity {
    /// API version (2022).
    pub version: u32,
}

/// Q2 rerelease cgame API version.
pub const Q2_CGAME_API_VERSION: u32 = 2022;

impl Q2CgameApiIdentity {
    /// Current identity.
    #[must_use]
    pub fn current() -> Self {
        Self {
            version: Q2_CGAME_API_VERSION,
        }
    }
}

/// Q2 cgame exports. Movement and player types stay owned by the movement
/// and network ports; the application binds them.
pub trait Q2CgameExports<Player = (), MovementInput = (), MovementResult = ()> {
    /// API identity.
    fn api(&self) -> Q2CgameApiIdentity;
    /// Initialize.
    fn init(&mut self);
    /// Shut down.
    fn shutdown(&mut self);
    /// Draw the HUD.
    fn draw_hud(&mut self, frame: &Q2HudDraw<Player>);
    /// Touch pictures.
    fn touch_pictures(&mut self);
    /// Layout flags for a player.
    fn layout_flags(&self, player: &Player) -> u32;
    /// Active weapon-wheel weapon.
    fn active_weapon_wheel_weapon(&self, player: &Player) -> i32;
    /// Owned weapon-wheel weapons bitmask.
    fn owned_weapon_wheel_weapons(&self, player: &Player) -> i32;
    /// Weapon-wheel ammo count.
    fn weapon_wheel_ammo_count(&self, player: &Player, ammo_id: i32) -> i32;
    /// Powerup-wheel count.
    fn powerup_wheel_count(&self, player: &Player, powerup_id: i32) -> i32;
    /// Hit-marker damage.
    fn hit_marker_damage(&self, player: &Player) -> i32;
    /// Predict movement.
    fn pmove(&mut self, input: MovementInput) -> MovementResult;
    /// Parse a configstring.
    fn parse_config_string(&mut self, index: i32, value: &str);
    /// Parse a center print.
    fn parse_center_print(&mut self, seat: SeatId, text: &str, instant: bool);
    /// Clear notifications.
    fn clear_notify(&mut self, seat: SeatId);
    /// Clear center print.
    fn clear_center_print(&mut self, seat: SeatId);
    /// Deliver a notify message.
    fn notify_message(&mut self, seat: SeatId, text: &str, chat: bool);
    /// Monster muzzle-flash offset.
    fn monster_flash_offset(&self, flash_id: i32) -> Vec3;
}

/// Stereo view for a Q3 cgame frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StereoView {
    /// Center.
    Center,
    /// Left.
    Left,
    /// Right.
    Right,
}

/// Q3 cgame event-handling mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q3CgameEventHandling {
    /// None.
    None,
    /// Team menu.
    TeamMenu,
    /// Scoreboard.
    Scoreboard,
    /// Edit HUD.
    EditHud,
}

/// Q3 cgame API version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q3CgameVersion {
    /// Version 3.
    V3,
    /// Version 4.
    V4,
}

/// Q3 cgame exports. The donor's promise-returning entries are synchronous
/// here; the application adapter blocks or polls its guest runtime.
pub trait Q3CgameExports {
    /// API version.
    fn api_version(&self) -> Q3CgameVersion;
    /// Owning seat.
    fn seat(&self) -> SeatId;
    /// Initialize.
    fn init(&mut self, server_message_number: i32, server_command_sequence: i32, client_number: i32);
    /// Shut down.
    fn shutdown(&mut self);
    /// Handle a console command; returns whether it was consumed.
    fn console_command(&mut self, args: &[String]) -> bool;
    /// Draw the active frame.
    fn draw_active_frame(&mut self, server_time_ms: i32, stereo_view: StereoView, demo_playback: bool);
    /// Crosshair player, if any.
    fn crosshair_player(&mut self) -> Option<i32>;
    /// Last attacker, if any.
    fn last_attacker(&mut self) -> Option<i32>;
    /// Handle a key event.
    fn key_event(&mut self, key: i32, down: bool);
    /// Handle a mouse event.
    fn mouse_event(&mut self, dx: i32, dy: i32);
    /// Set event-handling mode.
    fn event_handling(&mut self, mode: Q3CgameEventHandling);
}

/// Q3 menu command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q3MenuCommand {
    /// None.
    None,
    /// Main menu.
    Main,
    /// In-game menu.
    Ingame,
    /// Need CD.
    NeedCd,
    /// Bad CD key.
    BadCdKey,
    /// Team menu.
    Team,
    /// Postgame.
    Postgame,
}

/// Q3 UI API version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q3UiVersion {
    /// Version 4.
    V4,
    /// Version 6.
    V6,
}

/// Q3 UI exports. The donor's promise-returning entries are synchronous
/// here; the application adapter blocks or polls its guest runtime.
pub trait Q3UiExports {
    /// API version.
    fn api_version(&self) -> Q3UiVersion;
    /// Owning seat.
    fn seat(&self) -> SeatId;
    /// Initialize.
    fn init(&mut self, connecting: bool);
    /// Shut down.
    fn shutdown(&mut self);
    /// Handle a key event.
    fn key_event(&mut self, key: i32, down: bool);
    /// Handle a mouse event.
    fn mouse_event(&mut self, dx: i32, dy: i32);
    /// Refresh for a realtime instant.
    fn refresh(&mut self, real_time_ms: i32);
    /// Whether a fullscreen menu is active.
    fn is_fullscreen(&mut self) -> bool;
    /// Set the active menu.
    fn set_active_menu(&mut self, menu: Q3MenuCommand);
    /// Handle a console command; returns whether it was consumed.
    fn console_command(&mut self, real_time_ms: i32, args: &[String]) -> bool;
    /// Draw the connect screen.
    fn draw_connect_screen(&mut self, overlay: bool);
    /// Whether a unique CD key is present.
    fn has_unique_cd_key(&mut self) -> bool;
}

/// Arsenal ammo warning level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArsenalAmmoWarning {
    /// None.
    None,
    /// Low.
    Low,
    /// Empty.
    Empty,
}

/// Weapon ammo status.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum WeaponAmmo {
    /// Unmetered.
    Unmetered,
    /// Finite ammo.
    Finite {
        /// Ammo item.
        item: ItemId,
        /// Count.
        count: i32,
        /// Enough ammo to start firing.
        has_ammo_to_start: bool,
        /// Low.
        low: bool,
    },
}

/// Weapon HUD status.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WeaponHudStatus {
    /// Source provider.
    pub source: ProviderRef,
    /// Weapon item.
    pub item: ItemId,
    /// Label.
    pub label: String,
    /// Ammo.
    pub ammo: WeaponAmmo,
}

/// Weapon HUD icon.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum WeaponHudIcon {
    /// WAD picture.
    WadPicture {
        /// Resource request.
        resource: ResourceRequest,
        /// Lump name.
        lump: String,
    },
    /// Image.
    Image {
        /// Resource request.
        resource: ResourceRequest,
    },
    /// Shader.
    Shader {
        /// Content.
        content: ContentId,
        /// Shader name.
        name: String,
    },
}

/// Typeface preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Typeface {
    /// Standard.
    Standard,
    /// Bold.
    Bold,
}

/// Interface color-mode preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InterfaceColorMode {
    /// Standard.
    Standard,
    /// Blue and yellow.
    BlueYellow,
    /// Monochrome.
    Monochrome,
}

/// Per-seat UI preference values consumed by menu and HUD draws.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiPreferenceValues {
    /// HUD scale.
    pub hud_scale: f32,
    /// Text scale.
    pub text_scale: f32,
    /// Menu scale.
    pub menu_scale: f32,
    /// High contrast.
    pub high_contrast: bool,
    /// Reduced flashes.
    pub reduced_flashes: bool,
    /// Captions.
    pub captions: bool,
    /// Crosshair.
    pub crosshair: bool,
    /// Crosshair size in UI units.
    pub crosshair_size: f32,
    /// Typeface.
    pub typeface: Typeface,
    /// Color mode.
    pub color_mode: InterfaceColorMode,
}

/// Default UI preference values.
pub const DEFAULT_UI_PREFERENCES: UiPreferenceValues = UiPreferenceValues {
    hud_scale: 1.0,
    text_scale: 1.0,
    menu_scale: 1.0,
    high_contrast: false,
    reduced_flashes: false,
    captions: true,
    crosshair: true,
    crosshair_size: 8.0,
    typeface: Typeface::Standard,
    color_mode: InterfaceColorMode::Standard,
};

impl Default for UiPreferenceValues {
    fn default() -> Self {
        DEFAULT_UI_PREFERENCES
    }
}

/// Menu appearance sampled once per draw.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiAppearance {
    /// Menu scale.
    pub menu_scale: f32,
    /// Text scale.
    pub text_scale: f32,
    /// High contrast.
    pub high_contrast: bool,
    /// Color mode.
    pub color_mode: InterfaceColorMode,
}

impl Default for UiAppearance {
    fn default() -> Self {
        UiAppearance {
            menu_scale: 1.0,
            text_scale: 1.0,
            high_contrast: false,
            color_mode: InterfaceColorMode::Standard,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_and_menu_ids_validate_namespaces() {
        assert!(UiControlId::new("ui:match:join").is_ok());
        assert!(UiControlId::new("menu:match:join").is_err());
        assert!(UiControlId::new("ui:only").is_err());
        assert!(UiMenuId::new("menu:settings:root").is_ok());
        assert!(UiMenuId::new("ui:settings:root").is_err());
        assert!(UiMenuId::new("menu:short").is_err());
        assert!(ResourceId::new("resource:engine-menu:background").is_ok());
        assert!(ResourceId::new("engine-menu:background").is_err());
    }

    #[test]
    fn q2_layouts_match_donor_tables() {
        let classic = Q2ProtocolFamily::Classic.layout();
        assert_eq!(classic.images, 544);
        assert_eq!(classic.max_config_strings, 2080);
        assert_eq!(classic.n64_physics, None);
        assert!(!Q2ProtocolFamily::PrivateClassic.is_rerelease());
        let rerelease = Q2ProtocolFamily::Rerelease.layout();
        assert_eq!(rerelease.images, 10302);
        assert_eq!(rerelease.max_config_strings, 12448);
        assert_eq!(rerelease.n64_physics, Some(12103));
        assert!(Q2ProtocolFamily::KexDemo.is_rerelease());
    }

    #[test]
    fn dialects_round_trip() {
        for (text, dialect) in [
            ("q1-netquake", CommandDialect::Q1Netquake),
            ("q1-quakeworld", CommandDialect::Q1Quakeworld),
            ("q2-classic", CommandDialect::Q2Classic),
            ("q2-rerelease", CommandDialect::Q2Rerelease),
            ("q3", CommandDialect::Q3),
        ] {
            assert_eq!(CommandDialect::parse(text).unwrap(), dialect);
            assert_eq!(dialect.as_str(), text);
        }
        assert!(CommandDialect::parse("q4").is_err());
        assert!(CommandDialect::Q1Netquake.is_q1());
        assert!(CommandDialect::Q2Classic.is_q2());
        assert!(!CommandDialect::Q3.is_q1());
    }
}
