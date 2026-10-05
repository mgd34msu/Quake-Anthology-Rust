//! Startup menu stack: main, native, session, hosting, browser, and load menus.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/startup-menu.ts`
//! (`StartupMenu`, `StartupMenuOptions`). The selection model and server
//! browser arrive as shared handles; menu registration, controls, settings,
//! and text reuse `qa_client`. Sync port: the donor's promise chains
//! (roster reads, map-rule checks, browser queries) run inline and latch
//! failures into the status row. Menu callbacks never borrow the shared
//! controller: navigation goes through a [`NavIntent`] queue drained after
//! each input event, so input drives cannot panic on reentrant borrows. The
//! arena-selection menu (`./base-arena-select-menu.ts`) is ported inline as a
//! private helper. `draw` and `caption_commands` render through the caller's
//! [`UiRenderServices`](qa_client::ui::common::draw::UiRenderServices); the
//! menu owns the [`UiTextRenderer`] and binds the body/title fonts per
//! appearance. `bind_input` takes the ported [`BindingStore`] surface rather
//! than the donor's `SeatInput`.

use std::cell::RefCell;
use std::rc::Rc;

use qa_client::input::KeyCode;
use qa_client::text::atlas::TextFontSelection;
use qa_client::text::captions::ActiveCaption;
use qa_client::text::draw2d::Rect;
use qa_client::text::layout::layout_text;
use qa_client::text::layout::ColorCodes;
use qa_client::text::layout::TextAlign as LayoutAlign;
use qa_client::text::layout::TextLayoutOptions;
use qa_client::text::ui_world::UiTextRenderer;
use qa_client::ui::common::accessibility::accessible_colors;
use qa_client::ui::common::assets::NativeUiArt;
use qa_client::ui::common::captions::caption_commands;
use qa_client::ui::common::controller::NativeUiController;
use qa_client::ui::common::controller::NativeUiOptions;
use qa_client::ui::common::controller::UiSound;
use qa_client::ui::common::draw::render_ui_commands;
use qa_client::ui::common::draw::UiRenderServices;
use qa_client::ui::common::layout::contains;
use qa_client::ui::common::layout::fit_ui;
use qa_client::ui::common::layout::menu_row;
use qa_client::ui::common::layout::transform_ui;
use qa_client::ui::common::layout::MenuRowOptions;
use qa_client::ui::common::menu_theme::menu_backdrop;
use qa_client::ui::common::menu_theme::menu_panel;
use qa_client::ui::common::menu_theme::menu_skin;
use qa_client::ui::common::menu_theme::menu_title_font;
use qa_client::ui::library::menu::register_library_menu;
use qa_client::ui::library::menu::LibraryMenuService;
use qa_client::ui::mods::menu::register_mod_menu;
use qa_client::ui::settings::bindings::register_binding_menus;
use qa_client::ui::settings::bindings::BindingResetHandle;
use qa_client::ui::settings::bindings::BindingSource;
use qa_client::ui::settings::bindings::BindingStore;
use qa_client::ui::settings::gyro::register_gyro_settings_menu;
use qa_client::ui::settings::gyro::GyroSettingsUi;
use qa_client::ui::settings::llm::register_llm_settings_menu;
use qa_client::ui::settings::llm::LlmSettingsUi;
use qa_client::ui::settings::local_lobby::register_local_lobby_menu;
use qa_client::ui::settings::local_lobby::HostSelection;
use qa_client::ui::settings::local_lobby::LobbySource;
use qa_client::ui::settings::register_settings_menus;
use qa_client::ui::settings::setting_control;
use qa_client::ui::settings::SettingBinding;
use qa_client::ui::settings::SettingCategory;
use qa_client::ui::types::ResourceId;
use qa_client::ui::types::SeatInputEvent;
use qa_client::ui::types::SeatInputEventKind;
use qa_client::ui::types::SeatUiController;
use qa_client::ui::types::SeatUiState;
use qa_client::ui::types::TextAlign;
use qa_client::ui::types::UiChoice;
use qa_client::ui::types::UiControl;
use qa_client::ui::types::UiControlId;
use qa_client::ui::types::UiControlKind;
use qa_client::ui::types::UiDrawCommand;
use qa_client::ui::types::UiDrawContext;
use qa_client::ui::types::UiListRow;
use qa_client::ui::types::UiMenu;
use qa_client::ui::types::UiMenuId;
use qa_client::ui::types::UiPreferenceValues;
use qa_client::ui::types::DEFAULT_UI_PREFERENCES;
use qa_content::contract::GameFamily;
use qa_core::identity::SeatId;
use qa_core::math::Vec4;
use qa_net::common::endpoint::address_key;
use qa_net::protocol::q1::PRFL_INT32COORD;
use qa_net::protocol::q1::PRFL_SHORTANGLE;
use qa_net::protocol::ProtocolIdentity;

use super::server_browser::browser_address;
use super::server_browser::BrowserConnection;
use super::server_browser::StartupServerBrowser;
use super::server_browser::BROWSER_SORT_ORDERS;
use super::startup_saves::StartupSaveList;
use super::startup_selection::MonsterRosterRow;
use super::startup_selection::StartupHosting;
use super::startup_selection::StartupHostingKind;
use super::startup_selection::StartupNativePreset;
use super::startup_selection::StartupSelectionField;
use super::startup_selection::StartupSelectionModel;
use super::startup_selection::StartupSelectionRow;
use super::startup_summary::startup_summary_layout;
use super::startup_summary::SummaryBounds;

fn menu_id(text: &str) -> UiMenuId {
    UiMenuId::new(text).unwrap_or_else(|_| panic!("static UI menu id is invalid: {text}"))
}

fn control_id(text: &str) -> UiControlId {
    UiControlId::new(text).unwrap_or_else(|_| panic!("static UI control id is invalid: {text}"))
}

fn library_menu_id() -> UiMenuId {
    menu_id("menu:startup:library")
}
fn main_menu_id() -> UiMenuId {
    menu_id("menu:startup:main")
}
fn browser_menu_id() -> UiMenuId {
    menu_id("menu:startup:servers")
}
fn browser_options_menu_id() -> UiMenuId {
    menu_id("menu:startup:server-filters")
}
fn browser_details_menu_id() -> UiMenuId {
    menu_id("menu:startup:server-details")
}
fn native_family_menu_id() -> UiMenuId {
    menu_id("menu:startup:native-family")
}
fn native_campaign_menu_id() -> UiMenuId {
    menu_id("menu:startup:native-campaign")
}
fn native_difficulty_menu_id() -> UiMenuId {
    menu_id("menu:startup:native-difficulty")
}
fn session_menu_id() -> UiMenuId {
    menu_id("menu:startup:session")
}
fn hosting_menu_id() -> UiMenuId {
    menu_id("menu:startup:hosting")
}
fn options_menu_id() -> UiMenuId {
    menu_id("menu:startup:options")
}
fn display_menu_id() -> UiMenuId {
    menu_id("menu:settings:display:0")
}
fn sound_menu_id() -> UiMenuId {
    menu_id("menu:startup:sound")
}
fn controls_menu_id() -> UiMenuId {
    menu_id("menu:settings:input:0")
}
fn accessibility_menu_id() -> UiMenuId {
    menu_id("menu:settings:accessibility:0")
}
fn select_menu_id() -> UiMenuId {
    menu_id("menu:startup:select")
}
fn roster_menu_id() -> UiMenuId {
    menu_id("menu:startup:roster")
}
fn category_menu_id() -> UiMenuId {
    menu_id("menu:startup:category")
}
fn load_menu_id() -> UiMenuId {
    menu_id("menu:startup:load")
}
fn arena_menu_id() -> UiMenuId {
    menu_id("menu:library:arena-selection")
}
fn settings_root_menu_id() -> UiMenuId {
    menu_id("menu:settings:root")
}

/// Session menu groups (donor `groups`).
const GROUPS: [(&str, &[StartupSelectionField]); 4] = [
    (
        "World",
        &[
            StartupSelectionField::Product,
            StartupSelectionField::MapProduct,
            StartupSelectionField::Map,
        ],
    ),
    (
        "Player",
        &[
            StartupSelectionField::Movement,
            StartupSelectionField::Character,
            StartupSelectionField::Model,
            StartupSelectionField::Seats,
        ],
    ),
    (
        "Combat",
        &[
            StartupSelectionField::Weapons,
            StartupSelectionField::Enemies,
            StartupSelectionField::Skill,
            StartupSelectionField::Mode,
            StartupSelectionField::Rules,
        ],
    ),
    (
        "Equipment",
        &[
            StartupSelectionField::Grapple,
            StartupSelectionField::GrappleStyle,
            StartupSelectionField::Grenades,
        ],
    ),
];

/// Local-lobby hooks (donor `options.lobby`).
#[derive(Clone)]
pub struct StartupLobbyHooks {
    /// Current lobby service, if any.
    pub current: LobbySource,
    /// Lobby creation selection.
    pub selection: Rc<dyn Fn() -> Result<HostSelection, String>>,
    /// Local seat count.
    pub seats: Rc<dyn Fn() -> u32>,
}

/// One Team Arena team choice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupTeamChoice {
    /// Team id.
    pub id: String,
    /// Team label.
    pub label: String,
}

/// Team Arena hooks (donor `options.teamArena`).
#[derive(Clone)]
pub struct StartupTeamArenaHooks {
    /// Team choices.
    pub choices: Rc<dyn Fn() -> Vec<StartupTeamChoice>>,
    /// Selected `(player, opponent)` teams.
    pub read: Rc<dyn Fn() -> (String, String)>,
    /// Select one side's team.
    pub write: Rc<dyn Fn(String, String)>,
}

/// Library menu services (donor `options.libraries`).
#[derive(Clone, Default)]
pub struct StartupLibraries {
    /// Add-ons service.
    pub addons: Option<Rc<RefCell<dyn LibraryMenuService>>>,
    /// Player-progress service.
    pub player_progress: Option<Rc<RefCell<dyn LibraryMenuService>>>,
    /// Demos service.
    pub demos: Option<Rc<RefCell<dyn LibraryMenuService>>>,
    /// Movies service.
    pub movies: Option<Rc<RefCell<dyn LibraryMenuService>>>,
    /// Server-profiles service.
    pub server_profiles: Option<Rc<RefCell<dyn LibraryMenuService>>>,
    /// Configurations service.
    pub configurations: Option<Rc<RefCell<dyn LibraryMenuService>>>,
}

/// Play-a-preset callback (donor `playPreset`).
pub type PlayPresetFn = Rc<dyn Fn(String, u8, Option<String>)>;

/// Startup menu options (donor `StartupMenuOptions`).
pub struct StartupMenuOptions {
    /// Local-lobby hooks.
    pub lobby: Option<StartupLobbyHooks>,
    /// Menu sound sink.
    pub sound: Option<Rc<dyn Fn(UiSound)>>,
    /// LLM settings service.
    pub llm: Option<Rc<RefCell<dyn LlmSettingsUi>>>,
    /// Clipboard reader.
    pub clipboard: Option<Rc<dyn Fn() -> Option<String>>>,
    /// Owning seat.
    pub seat: SeatId,
    /// Selection model.
    pub model: Rc<RefCell<StartupSelectionModel>>,
    /// Uploaded menu art.
    pub art: NativeUiArt,
    /// Body font.
    pub font: TextFontSelection,
    /// Title font.
    pub title_font: TextFontSelection,
    /// Current time in milliseconds.
    pub now: Rc<dyn Fn() -> i64>,
    /// Play the draft.
    pub play: Rc<dyn Fn()>,
    /// Play a native preset.
    pub play_preset: Option<PlayPresetFn>,
    /// Server browser.
    pub browser: Option<Rc<RefCell<StartupServerBrowser>>>,
    /// Connect to a browser connection.
    pub connect: Option<Rc<dyn Fn(BrowserConnection)>>,
    /// Load a save id.
    pub load: Rc<dyn Fn(String)>,
    /// Current save list.
    pub saves: Rc<dyn Fn() -> StartupSaveList>,
    /// Refresh the save list.
    pub refresh_saves: Rc<dyn Fn()>,
    /// Quit the application.
    pub quit: Rc<dyn Fn()>,
    /// Extra settings bindings.
    pub settings: Vec<SettingBinding>,
    /// UI preferences.
    pub appearance: Option<Rc<dyn Fn() -> UiPreferenceValues>>,
    /// Team Arena hooks.
    pub team_arena: Option<StartupTeamArenaHooks>,
    /// Library services.
    pub libraries: Option<StartupLibraries>,
}

/// Queued navigation (callbacks queue; [`StartupMenu::input`] drains).
#[derive(Debug, Clone, PartialEq, Eq)]
enum NavIntent {
    /// Open a menu.
    Open(UiMenuId),
    /// Close the top menu.
    Close,
}

/// Monster roster selection mode (donor `monsterField`).
#[derive(Debug, Clone, PartialEq, Eq)]
enum MonsterField {
    /// Draft field selection.
    None,
    /// Monster source selection.
    Source,
    /// One roster class.
    Class(Option<String>),
}

/// Mutable menu draft state.
struct StartupMenuState {
    status: String,
    busy: bool,
    group: usize,
    field: StartupSelectionField,
    page: usize,
    roster_page: usize,
    monster_field: MonsterField,
    save_page: usize,
    native_family: GameFamily,
    native_edition: String,
    native_preset: Option<StartupNativePreset>,
    native_skill: String,
    native_arena: Option<String>,
    host_draft: StartupHosting,
    host_port: String,
    gyro_menu: Option<UiMenuId>,
    binding_menu: Option<UiMenuId>,
    lobby_menu: Option<UiMenuId>,
}

/// Body font slot in the menu text renderer.
pub const MENU_BODY_FONT_SLOT: u32 = 0;
/// Title font slot in the menu text renderer.
pub const MENU_TITLE_FONT_SLOT: u32 = 1;

/// Map a draw-command font to a menu text slot.
#[must_use]
pub fn menu_font_slot(font: &ResourceId) -> u32 {
    if font == &menu_title_font() {
        MENU_TITLE_FONT_SLOT
    } else {
        MENU_BODY_FONT_SLOT
    }
}

/// Measure one text run (donor `measure`).
fn measure_run(text: &str, scale: f32, font: &TextFontSelection) -> f32 {
    layout_text(&TextLayoutOptions {
        text,
        font,
        scale,
        color: Vec4 {
            x: 1.0,
            y: 1.0,
            z: 1.0,
            w: 1.0,
        },
        color_codes: ColorCodes::Literal,
        force_color: false,
        alternate: false,
        max_width: None,
        align: LayoutAlign::Left,
        line_height: None,
        max_glyphs: None,
        tab_columns: 4,
    })
    .map_or(0.0, |layout| layout.width)
}

/// Truncate text with an ellipsis to a measured width (donor `fit`).
fn fit_text(text: &str, width: f32, scale: f32, font: &TextFontSelection) -> String {
    if measure_run(text, scale, font) <= width {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut end = chars.len();
    while end > 0 && measure_run(&format!("{}...", chars[..end].iter().collect::<String>()), scale, font) > width {
        end -= 1;
    }
    format!("{}...", chars[..end].iter().collect::<String>())
}

/// The startup menu stack (donor `StartupMenu`).
pub struct StartupMenu {
    controller: Rc<RefCell<NativeUiController>>,
    options: Rc<StartupMenuOptions>,
    state: Rc<RefCell<StartupMenuState>>,
    nav: Rc<RefCell<Vec<NavIntent>>>,
    text: RefCell<UiTextRenderer>,
    disposers: Vec<Box<dyn FnOnce()>>,
}

/// Shared menu handles captured by factories and callbacks.
#[derive(Clone)]
struct MenuRefs {
    controller: Rc<RefCell<NativeUiController>>,
    options: Rc<StartupMenuOptions>,
    state: Rc<RefCell<StartupMenuState>>,
    nav: Rc<RefCell<Vec<NavIntent>>>,
}

impl MenuRefs {
    fn set_status(&self, text: &str, busy: bool) {
        let mut state = self.state.borrow_mut();
        state.status = text.to_string();
        state.busy = busy;
    }

    fn open(&self, id: UiMenuId) {
        self.nav.borrow_mut().push(NavIntent::Open(id));
    }

    fn close(&self) {
        self.nav.borrow_mut().push(NavIntent::Close);
    }

    fn busy(&self) -> bool {
        self.state.borrow().busy
    }

    fn button(&self, id: &str, label: String, row: i32, activate: Rc<dyn Fn()>, wide: bool) -> UiControl {
        let enabled = !self.busy();
        UiControl {
            id: control_id(&format!("ui:startup:{id}")),
            label,
            rect: Rect {
                x: 64.0,
                y: 118.0 + row as f32 * 34.0,
                width: if wide { 512.0 } else { 224.0 },
                height: 30.0,
            },
            enabled,
            visible: true,
            kind: UiControlKind::Button {
                on_activate: Rc::new(move |_| activate()),
            },
        }
    }

    fn back(&self) -> UiControl {
        let refs = self.clone();
        self.button("back", "Back".to_string(), 9, Rc::new(move || refs.close()), true)
    }

    fn label(&self, id: &str, text: String, y: f32, enabled: bool) -> UiControl {
        UiControl {
            id: control_id(&format!("ui:startup:{id}")),
            label: text,
            rect: Rect {
                x: 64.0,
                y,
                width: 512.0,
                height: 30.0,
            },
            enabled,
            visible: true,
            kind: UiControlKind::Button {
                on_activate: Rc::new(|_| {}),
            },
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn choice(
        &self,
        id: &str,
        label: String,
        rect: Rect,
        enabled: bool,
        selected: Option<String>,
        choices: Vec<UiChoice>,
        on_select: Rc<dyn Fn(String)>,
    ) -> UiControl {
        UiControl {
            id: control_id(&format!("ui:startup:{id}")),
            label,
            rect,
            enabled,
            visible: true,
            kind: UiControlKind::Choice {
                choices,
                selected,
                on_select: Rc::new(move |_, value| on_select(value.to_string())),
            },
        }
    }

    fn toggle(
        &self,
        id: &str,
        label: String,
        rect: Rect,
        enabled: bool,
        checked: bool,
        on_change: Rc<dyn Fn(bool)>,
    ) -> UiControl {
        UiControl {
            id: control_id(&format!("ui:startup:{id}")),
            label,
            rect,
            enabled,
            visible: true,
            kind: UiControlKind::Toggle {
                checked,
                on_change: Rc::new(move |_, value| on_change(value)),
            },
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn field(
        &self,
        id: &str,
        label: String,
        text: String,
        maximum_length: usize,
        rect: Rect,
        enabled: bool,
        on_change: Rc<dyn Fn(String)>,
        on_submit: Rc<dyn Fn(String)>,
    ) -> UiControl {
        UiControl {
            id: control_id(&format!("ui:startup:{id}")),
            label,
            rect,
            enabled,
            visible: true,
            kind: UiControlKind::TextEntry {
                masked: false,
                text,
                maximum_length,
                on_change: Rc::new(move |_, value| on_change(value.to_string())),
                on_submit: Rc::new(move |_, value| on_submit(value.to_string())),
            },
        }
    }

    fn appearance(&self) -> UiPreferenceValues {
        self.options
            .appearance
            .as_ref()
            .map_or(DEFAULT_UI_PREFERENCES, |read| read())
    }

    fn measure_font(&self) -> TextFontSelection {
        if self.appearance().typeface == qa_client::ui::types::Typeface::Bold {
            self.options.title_font.clone()
        } else {
            self.options.font.clone()
        }
    }

    fn fit(&self, text: &str, width: f32, scale: f32) -> String {
        fit_text(text, width, scale, &self.measure_font())
    }

    fn model_rows(&self, fields: &[StartupSelectionField]) -> Vec<StartupSelectionRow> {
        let rows = self.options.model.borrow_mut().draft_rows().unwrap_or_default();
        fields
            .iter()
            .filter_map(|field| rows.iter().find(|row| row.id == *field).cloned())
            .collect()
    }
}

impl StartupMenu {
    /// Build the menu stack over shared handles.
    pub fn new(options: StartupMenuOptions) -> Self {
        let seat = options.seat.clone();
        let mut text = UiTextRenderer::new(seat.clone());
        text.bind(MENU_BODY_FONT_SLOT, options.font.clone());
        text.bind(MENU_TITLE_FONT_SLOT, options.title_font.clone());
        let skin = menu_skin(&options.art.skin.font);
        let options = Rc::new(options);
        let measure_options = Rc::clone(&options);
        let appearance_options = Rc::clone(&options);
        let sound_sink = options.sound.clone();
        let clipboard = options.clipboard.clone();
        let now = Rc::clone(&options.now);
        let controller = Rc::new(RefCell::new(NativeUiController::new(NativeUiOptions {
            seat,
            skin: Box::new(move || skin.clone()),
            now: Box::new(move || now()),
            bindings: Box::new(Vec::new),
            focus: Box::new(|_, _| {}),
            sound: Box::new(move |sound, _| {
                if let Some(sink) = &sound_sink {
                    sink(sound);
                }
            }),
            execute_script: Box::new(|_, _| panic!("Startup menu has no legacy scripts")),
            localize: Box::new(|text| text.to_string()),
            appearance: Box::new(move || {
                to_appearance(
                    &appearance_options
                        .appearance
                        .as_ref()
                        .map_or(DEFAULT_UI_PREFERENCES, |read| read()),
                )
            }),
            clipboard: Box::new(move || clipboard.as_ref().and_then(|read| read())),
            measure_text: Box::new(move |text, scale| {
                let bold = measure_options
                    .appearance
                    .as_ref()
                    .is_some_and(|read| read().typeface == qa_client::ui::types::Typeface::Bold);
                measure_run(
                    text,
                    scale,
                    if bold {
                        &measure_options.title_font
                    } else {
                        &measure_options.font
                    },
                )
            }),
        })));
        let state = Rc::new(RefCell::new(StartupMenuState {
            status: String::new(),
            busy: false,
            group: 0,
            field: StartupSelectionField::Product,
            page: 0,
            roster_page: 0,
            monster_field: MonsterField::None,
            save_page: 0,
            native_family: GameFamily::Q1,
            native_edition: "classic".to_string(),
            native_preset: None,
            native_skill: "1".to_string(),
            native_arena: None,
            host_draft: StartupHosting {
                kind: StartupHostingKind::Offline,
                port: 27910,
                q1_protocol: None,
            },
            host_port: "27910".to_string(),
            gyro_menu: None,
            binding_menu: None,
            lobby_menu: None,
        }));
        let refs = MenuRefs {
            controller: Rc::clone(&controller),
            options: Rc::clone(&options),
            state,
            nav: Rc::new(RefCell::new(Vec::new())),
        };
        let mut menu = Self {
            controller,
            options,
            state: Rc::clone(&refs.state),
            nav: Rc::clone(&refs.nav),
            text: RefCell::new(text),
            disposers: Vec::new(),
        };
        menu.register_main(&refs);
        let library_pages = menu.register_libraries(&refs);
        menu.register_library_index(&refs, &library_pages);
        menu.register_native_family(&refs);
        menu.register_arena_menu(&refs);
        menu.register_native_campaign(&refs);
        menu.register_native_difficulty(&refs);
        menu.register_browser(&refs);
        menu.register_browser_options(&refs);
        menu.register_browser_details(&refs);
        let lobby_root = menu.register_lobby(&refs);
        let mods_menus = register_mod_menu(
            &refs.controller,
            refs.options.model.clone() as Rc<RefCell<dyn qa_client::ui::mods::menu::ModMenuService>>,
        );
        let mods_root = mods_menus.root.clone();
        menu.disposers.push(Box::new(|| mods_menus.dispose()));
        menu.register_session(&refs, lobby_root, mods_root);
        menu.register_hosting(&refs);
        menu.register_category(&refs);
        let llm_root = menu.register_llm(&refs);
        menu.register_options(&refs, llm_root);
        menu.register_settings(&refs);
        menu.register_sound(&refs);
        menu.register_select(&refs);
        menu.register_roster(&refs);
        menu.register_load(&refs);
        menu.ensure_active_menu();
        menu
    }

    /// Shared controller.
    #[must_use]
    pub fn controller(&self) -> &Rc<RefCell<NativeUiController>> {
        &self.controller
    }

    /// Whether the menu is busy.
    #[must_use]
    pub fn is_busy(&self) -> bool {
        self.state.borrow().busy
    }

    /// Active menu id, if any.
    #[must_use]
    pub fn active_menu(&self) -> Option<UiMenuId> {
        self.controller.borrow().active_menu()
    }

    /// Current controller state.
    #[must_use]
    pub fn state(&self) -> SeatUiState {
        self.controller.borrow().state()
    }

    /// Owned text renderer for render-service adapters.
    #[must_use]
    pub fn text(&self) -> &RefCell<UiTextRenderer> {
        &self.text
    }

    fn register(&mut self, id: UiMenuId, refs: &MenuRefs, build: Rc<dyn Fn() -> Vec<UiControl>>) {
        let menu_id = id.clone();
        let register_controller = Rc::clone(&refs.controller);
        register_controller.borrow_mut().register(
            id.clone(),
            Rc::new(move || UiMenu {
                scroll: None,
                id: menu_id.clone(),
                title: String::new(),
                full_screen: false,
                controls: build(),
                on_open: Rc::new(|_| {}),
                on_close: Rc::new(|_| {}),
            }),
        );
        let controller = Rc::clone(&refs.controller);
        self.disposers
            .push(Box::new(move || controller.borrow_mut().unregister(&id)));
    }

    fn register_main(&mut self, refs: &MenuRefs) {
        let refs = refs.clone();
        let has_libraries = refs.options.libraries.is_some();
        self.register(
            main_menu_id(),
            &refs.clone(),
            Rc::new(move || {
                let open_family = refs.clone();
                let open_load = refs.clone();
                let open_options = refs.clone();
                let open_library = refs.clone();
                let quit = Rc::clone(&refs.options.quit);
                let refresh = Rc::clone(&refs.options.refresh_saves);
                let mut controls = vec![
                    refs.clone().button(
                        "native",
                        "Play a game".to_string(),
                        0,
                        Rc::new(move || {
                            open_family.state.borrow_mut().status.clear();
                            open_family.open(native_family_menu_id());
                        }),
                        false,
                    ),
                    refs.clone().button(
                        "load",
                        "Load Game".to_string(),
                        1,
                        Rc::new(move || {
                            open_load.state.borrow_mut().save_page = 0;
                            open_load.open(load_menu_id());
                            refresh();
                        }),
                        false,
                    ),
                    refs.clone().button(
                        "options",
                        "Options".to_string(),
                        2,
                        Rc::new(move || open_options.open(options_menu_id())),
                        false,
                    ),
                ];
                if has_libraries {
                    controls.push(refs.clone().button(
                        "library",
                        "Library".to_string(),
                        3,
                        Rc::new(move || open_library.open(library_menu_id())),
                        false,
                    ));
                }
                controls.push(refs.clone().button(
                    "quit",
                    "Quit".to_string(),
                    if has_libraries { 4 } else { 3 },
                    Rc::new(move || quit()),
                    false,
                ));
                controls
            }),
        );
    }

    fn register_libraries(&mut self, refs: &MenuRefs) -> Vec<(String, String, UiMenuId)> {
        let mut pages = Vec::new();
        let libraries = refs.options.libraries.clone().unwrap_or_default();
        for (name, label, service) in [
            ("addons", "Add-ons — Quaddicted", libraries.addons),
            ("demos", "Demos", libraries.demos),
            ("movies", "Movies", libraries.movies),
            ("configurations", "Configurations", libraries.configurations),
            ("server-profiles", "Server profiles", libraries.server_profiles),
            (
                "player-progress",
                "Achievements and progress",
                libraries.player_progress,
            ),
        ] {
            if let Some(service) = service {
                let page = register_library_menu(
                    &refs.controller,
                    &menu_id(&format!("menu:library:{name}")),
                    label,
                    service,
                );
                let root = page.root.clone();
                self.disposers.push(Box::new(|| page.dispose()));
                pages.push((name.to_string(), label.to_string(), root));
            }
        }
        pages
    }

    fn register_library_index(&mut self, refs: &MenuRefs, pages: &[(String, String, UiMenuId)]) {
        let refs = refs.clone();
        let pages = pages.to_vec();
        self.register(
            library_menu_id(),
            &refs.clone(),
            Rc::new(move || {
                let mut controls: Vec<UiControl> = pages
                    .iter()
                    .enumerate()
                    .map(|(index, (name, label, root))| {
                        let refs = refs.clone();
                        let root = root.clone();
                        refs.clone().button(
                            &format!("library:{name}"),
                            label.clone(),
                            index as i32,
                            Rc::new(move || refs.open(root.clone())),
                            true,
                        )
                    })
                    .collect();
                controls.push(refs.back());
                controls
            }),
        );
    }

    fn register_native_family(&mut self, refs: &MenuRefs) {
        let refs = refs.clone();
        self.register(
            native_family_menu_id(),
            &refs.clone(),
            Rc::new(move || {
                let mut controls: Vec<UiControl> = [
                    (GameFamily::Q1, "classic", "Quake classic"),
                    (GameFamily::Q1, "rerelease", "Quake rerelease"),
                    (GameFamily::Q2, "classic", "Quake II classic"),
                    (GameFamily::Q2, "rerelease", "Quake II rerelease"),
                    (GameFamily::Q3, "classic", "Quake III Arena"),
                ]
                .into_iter()
                .enumerate()
                .map(|(index, (family, edition, label))| {
                    let refs = refs.clone();
                    let available = refs
                        .options
                        .model
                        .borrow()
                        .presets()
                        .iter()
                        .any(|preset| preset.family == family && preset.edition == edition);
                    let busy = refs.busy();
                    let mut control = refs.clone().button(
                        &format!("game:{family}:{edition}"),
                        label.to_string(),
                        index as i32,
                        Rc::new(move || {
                            let mut state = refs.state.borrow_mut();
                            state.native_family = family;
                            state.native_edition = edition.to_string();
                            state.status.clear();
                            refs.open(native_campaign_menu_id());
                        }),
                        true,
                    );
                    control.enabled = !busy && available;
                    control
                })
                .collect();
                let refs_custom = refs.clone();
                controls.push(refs.clone().button(
                    "custom",
                    "Custom game".to_string(),
                    5,
                    Rc::new(move || {
                        refs_custom.state.borrow_mut().status.clear();
                        refs_custom.open(session_menu_id());
                    }),
                    true,
                ));
                controls.push(refs.back());
                controls
            }),
        );
    }

    /// Arena-selection menu, ported inline from
    /// `./base-arena-select-menu.ts` (`registerArenaSelectionMenu`).
    fn register_arena_menu(&mut self, refs: &MenuRefs) {
        let refs = refs.clone();
        let tier = Rc::new(RefCell::new(String::new()));
        let selected = Rc::new(RefCell::new(None::<String>));
        let id = arena_menu_id();
        let open_tier = Rc::clone(&tier);
        let open_selected = Rc::clone(&selected);
        let register_controller = Rc::clone(&refs.controller);
        let controller = Rc::clone(&refs.controller);
        register_controller.borrow_mut().register(
            id.clone(),
            Rc::new(move || {
                let snapshot = refs.options.model.borrow().base_arenas().cloned();
                if let Some(selection) = &snapshot {
                    if !selection.tiers.iter().any(|row| row.id == *tier.borrow()) {
                        let current = selection.current.clone();
                        *tier.borrow_mut() = selection
                            .rows
                            .iter()
                            .find(|row| Some(row.arena.map.clone()) == current)
                            .map_or_else(
                                || selection.tiers.first().map_or_else(String::new, |row| row.id.clone()),
                                |row| row.tier.clone(),
                            );
                    }
                }
                let rows: Vec<super::startup_selection::StartupArenaRow> =
                    snapshot.as_ref().map_or_else(Vec::new, |selection| {
                        selection
                            .rows
                            .iter()
                            .filter(|row| row.tier == *tier.borrow())
                            .cloned()
                            .collect()
                    });
                if !rows.iter().any(|row| Some(row.arena.map.clone()) == *selected.borrow()) {
                    let current = snapshot.as_ref().and_then(|selection| selection.current.clone());
                    *selected.borrow_mut() = rows
                        .iter()
                        .find(|row| Some(row.arena.map.clone()) == current)
                        .map(|row| row.arena.map.clone())
                        .or_else(|| rows.first().map(|row| row.arena.map.clone()));
                }
                let current = rows
                    .iter()
                    .find(|row| Some(row.arena.map.clone()) == *selected.borrow())
                    .cloned();
                let tiers: Vec<UiChoice> = snapshot.as_ref().map_or_else(Vec::new, |selection| {
                    selection
                        .tiers
                        .iter()
                        .map(|row| UiChoice {
                            id: row.id.clone(),
                            label: row.label.clone(),
                        })
                        .collect()
                });
                let select_tier = Rc::clone(&tier);
                let clear_selected = Rc::clone(&selected);
                let tier_control = UiControl {
                    id: control_id("ui:arena-select:tier"),
                    label: "Tier".to_string(),
                    rect: Rect {
                        x: 48.0,
                        y: 94.0,
                        width: 544.0,
                        height: 32.0,
                    },
                    enabled: snapshot.is_some(),
                    visible: true,
                    kind: UiControlKind::Choice {
                        choices: tiers,
                        selected: Some(tier.borrow().clone()),
                        on_select: Rc::new(move |_, value| {
                            *select_tier.borrow_mut() = value.to_string();
                            *clear_selected.borrow_mut() = None;
                        }),
                    },
                };
                let pick_selected = Rc::clone(&selected);
                let choose_refs = refs.clone();
                let choose_rows = rows.clone();
                let list_control = UiControl {
                    id: control_id("ui:arena-select:arenas"),
                    label: "Arenas".to_string(),
                    rect: Rect {
                        x: 48.0,
                        y: 144.0,
                        width: 544.0,
                        height: 156.0,
                    },
                    enabled: !rows.is_empty(),
                    visible: true,
                    kind: UiControlKind::List {
                        row_height: Some(39.0),
                        column_widths: None,
                        on_activate: Some(Rc::new(move |_, value| {
                            if choose_rows.iter().any(|row| row.arena.map == value && row.available) {
                                choose_refs.state.borrow_mut().native_arena = Some(value.to_string());
                                choose_refs.open(native_difficulty_menu_id());
                            }
                        })),
                        rows: rows
                            .iter()
                            .map(|row| UiListRow {
                                action: None,
                                id: row.arena.map.clone(),
                                cells: vec![
                                    row.arena.title.clone(),
                                    if row.available {
                                        row.record.clone()
                                    } else {
                                        "Locked".to_string()
                                    },
                                ],
                                image: None,
                                enabled: true,
                            })
                            .collect(),
                        selected: selected.borrow().clone(),
                        on_select: Rc::new(move |_, value| {
                            *pick_selected.borrow_mut() = Some(value.to_string());
                        }),
                    },
                };
                let opponents = current.as_ref().map_or_else(
                    || "No single-player arenas are available.".to_string(),
                    |row| {
                        format!(
                            "Opponents: {}",
                            if row.arena.bots.is_empty() {
                                "None".to_string()
                            } else {
                                row.arena.bots.join(", ")
                            }
                        )
                    },
                );
                let limits = current.as_ref().map_or_else(String::new, |row| {
                    [
                        if row.arena.frag_limit > 0 {
                            format!("{} frags", row.arena.frag_limit)
                        } else {
                            String::new()
                        },
                        if row.arena.time_limit > 0 {
                            format!("{} minutes", row.arena.time_limit)
                        } else {
                            String::new()
                        },
                    ]
                    .into_iter()
                    .filter(|part| !part.is_empty())
                    .collect::<Vec<_>>()
                    .join(" · ")
                });
                let play_refs = refs.clone();
                let play_map = current.as_ref().and_then(|row| {
                    if row.available {
                        Some(row.arena.map.clone())
                    } else {
                        None
                    }
                });
                let play_control = UiControl {
                    id: control_id("ui:arena-select:play"),
                    label: if current.as_ref().is_some_and(|row| !row.available) {
                        "Win the preceding tier to unlock".to_string()
                    } else {
                        "Choose difficulty".to_string()
                    },
                    rect: Rect {
                        x: 48.0,
                        y: 392.0,
                        width: 544.0,
                        height: 32.0,
                    },
                    enabled: play_map.is_some(),
                    visible: true,
                    kind: UiControlKind::Button {
                        on_activate: Rc::new(move |_| {
                            if let Some(map) = &play_map {
                                play_refs.state.borrow_mut().native_arena = Some(map.clone());
                                play_refs.open(native_difficulty_menu_id());
                            }
                        }),
                    },
                };
                let back_refs = refs.clone();
                let reset_tier = Rc::clone(&open_tier);
                let reset_selected = Rc::clone(&open_selected);
                UiMenu {
                    scroll: None,
                    id: arena_menu_id(),
                    title: "Choose an arena".to_string(),
                    full_screen: true,
                    controls: vec![
                        tier_control,
                        list_control,
                        UiControl {
                            id: control_id("ui:arena-select:opponents"),
                            label: opponents,
                            rect: Rect {
                                x: 48.0,
                                y: 318.0,
                                width: 544.0,
                                height: 30.0,
                            },
                            enabled: false,
                            visible: true,
                            kind: UiControlKind::Button {
                                on_activate: Rc::new(|_| {}),
                            },
                        },
                        UiControl {
                            id: control_id("ui:arena-select:limits"),
                            label: limits,
                            rect: Rect {
                                x: 48.0,
                                y: 350.0,
                                width: 544.0,
                                height: 30.0,
                            },
                            enabled: false,
                            visible: true,
                            kind: UiControlKind::Button {
                                on_activate: Rc::new(|_| {}),
                            },
                        },
                        play_control,
                        UiControl {
                            id: control_id("ui:arena-select:back"),
                            label: "Back".to_string(),
                            rect: Rect {
                                x: 48.0,
                                y: 436.0,
                                width: 544.0,
                                height: 32.0,
                            },
                            enabled: true,
                            visible: true,
                            kind: UiControlKind::Button {
                                on_activate: Rc::new(move |_| back_refs.close()),
                            },
                        },
                    ],
                    on_open: Rc::new(move |_| {
                        *reset_tier.borrow_mut() = String::new();
                        *reset_selected.borrow_mut() = None;
                    }),
                    on_close: Rc::new(|_| {}),
                }
            }),
        );
        let unregister = id.clone();
        self.disposers
            .push(Box::new(move || controller.borrow_mut().unregister(&unregister)));
    }

    fn native_campaigns(&self, family: GameFamily, edition: &str) -> Vec<StartupNativePreset> {
        self.options
            .model
            .borrow()
            .presets()
            .into_iter()
            .filter(|preset| preset.family == family && preset.edition == edition)
            .collect()
    }

    fn register_native_campaign(&mut self, refs: &MenuRefs) {
        let refs = refs.clone();
        self.register(
            native_campaign_menu_id(),
            &refs.clone(),
            Rc::new(move || {
                let state = refs.state.borrow();
                let family = state.native_family;
                let edition = state.native_edition.clone();
                drop(state);
                let mut controls: Vec<UiControl> = refs
                    .options
                    .model
                    .borrow()
                    .presets()
                    .into_iter()
                    .filter(|preset| preset.family == family && preset.edition == edition)
                    .enumerate()
                    .map(|(index, preset)| {
                        let refs = refs.clone();
                        let busy = refs.busy();
                        let mut control = refs.clone().button(
                            &format!("campaign:{}", preset.id),
                            preset.label.clone(),
                            index as i32,
                            Rc::new(move || {
                                let mut state = refs.state.borrow_mut();
                                state.native_preset = Some(preset.clone());
                                state.native_skill = preset.default_skill.clone();
                                state.native_arena = None;
                                state.status.clear();
                                drop(state);
                                if preset.id == "q3-baseq3" {
                                    refs.set_status("Loading arenas...", true);
                                    match refs.options.model.borrow_mut().refresh_base_arenas() {
                                        Ok(()) => {
                                            refs.set_status("", false);
                                            refs.open(arena_menu_id());
                                        }
                                        Err(error) => refs.set_status(&error.to_string(), false),
                                    }
                                } else {
                                    refs.open(native_difficulty_menu_id());
                                }
                            }),
                            true,
                        );
                        control.enabled = !busy;
                        control
                    })
                    .collect();
                controls.push(refs.back());
                controls
            }),
        );
    }

    fn register_native_difficulty(&mut self, refs: &MenuRefs) {
        let refs = refs.clone();
        self.register(
            native_difficulty_menu_id(),
            &refs.clone(),
            Rc::new(move || {
                let preset = refs.state.borrow().native_preset.clone();
                let Some(preset) = preset else { return vec![refs.back()] };
                let mut controls: Vec<UiControl> = preset
                    .difficulties
                    .iter()
                    .enumerate()
                    .map(|(index, difficulty)| {
                        let refs = refs.clone();
                        let id = difficulty.id.clone();
                        let busy = refs.busy();
                        let mut control = refs.clone().button(
                            &format!("difficulty:{id}"),
                            format!(
                                "{}{}",
                                if refs.state.borrow().native_skill == id {
                                    "> "
                                } else {
                                    ""
                                },
                                difficulty.label
                            ),
                            index as i32,
                            Rc::new(move || {
                                refs.state.borrow_mut().native_skill = id.clone();
                            }),
                            true,
                        );
                        control.enabled = !busy && difficulty.unavailable.is_none();
                        control
                    })
                    .collect();
                if preset.id == "q3-missionpack" {
                    if let Some(team_arena) = refs.options.team_arena.clone() {
                        for (side, index) in [("player", 5), ("opponent", 6)] {
                            let (player, opponent) = (team_arena.read)();
                            let selected = if side == "player" { player } else { opponent };
                            let write = Rc::clone(&team_arena.write);
                            controls.push(
                                refs.clone().choice(
                                    &format!("team:{side}"),
                                    if side == "player" {
                                        "Your team".to_string()
                                    } else {
                                        "Opponents".to_string()
                                    },
                                    Rect {
                                        x: 64.0,
                                        y: 118.0 + index as f32 * 34.0,
                                        width: 512.0,
                                        height: 30.0,
                                    },
                                    !refs.busy(),
                                    Some(selected),
                                    (team_arena.choices)()
                                        .into_iter()
                                        .map(|choice| UiChoice {
                                            id: choice.id,
                                            label: choice.label,
                                        })
                                        .collect(),
                                    Rc::new(move |value| write(side.to_string(), value)),
                                ),
                            );
                        }
                    }
                }
                let refs_play = refs.clone();
                let preset_id = preset.id.clone();
                controls.push(refs.clone().button(
                    "play-preset",
                    "Play".to_string(),
                    7,
                    Rc::new(move || {
                        if let Some(play) = refs_play.options.play_preset.clone() {
                            let state = refs_play.state.borrow();
                            let skill = state.native_skill.parse::<u8>().unwrap_or(0);
                            let arena = state.native_arena.clone();
                            drop(state);
                            play(preset_id.clone(), skill, arena);
                        }
                    }),
                    true,
                ));
                controls.push(refs.back());
                controls
            }),
        );
    }

    fn register_lobby(&mut self, refs: &MenuRefs) -> Option<UiMenuId> {
        let lobby = refs.options.lobby.clone()?;
        let menus = register_local_lobby_menu(&refs.controller, lobby.current, lobby.selection, lobby.seats);
        let root = menus.root.clone();
        refs.state.borrow_mut().lobby_menu = Some(root.clone());
        self.disposers.push(Box::new(|| menus.dispose()));
        Some(root)
    }

    fn register_session(&mut self, refs: &MenuRefs, lobby_root: Option<UiMenuId>, mods_root: UiMenuId) {
        let refs = refs.clone();
        self.register(
            session_menu_id(),
            &refs.clone(),
            Rc::new(move || {
                let mut controls = Vec::new();
                if refs.options.browser.is_some() {
                    let refs = refs.clone();
                    controls.push(refs.clone().button(
                        "browse",
                        "Find servers".to_string(),
                        8,
                        Rc::new(move || refs.open(browser_menu_id())),
                        false,
                    ));
                }
                if let Some(root) = &lobby_root {
                    let refs = refs.clone();
                    let root = root.clone();
                    controls.push(refs.clone().button(
                        "local-lobby",
                        "Local lobby".to_string(),
                        9,
                        Rc::new(move || refs.open(root.clone())),
                        false,
                    ));
                }
                for (index, (title, _)) in GROUPS.iter().enumerate() {
                    let refs = refs.clone();
                    controls.push(refs.clone().button(
                        &format!("group:{index}"),
                        (*title).to_string(),
                        index as i32,
                        Rc::new(move || {
                            refs.state.borrow_mut().group = index;
                            refs.open(category_menu_id());
                        }),
                        false,
                    ));
                }
                let hosting_kind = refs
                    .options
                    .model
                    .borrow_mut()
                    .hosting()
                    .map(|hosting| hosting.kind)
                    .unwrap_or(StartupHostingKind::Offline);
                let refs_hosting = refs.clone();
                controls.push(refs.clone().button(
                    "hosting",
                    if hosting_kind == StartupHostingKind::Offline {
                        "Network: local only".to_string()
                    } else {
                        "Network: hosting".to_string()
                    },
                    4,
                    Rc::new(move || {
                        let hosting = refs_hosting
                            .options
                            .model
                            .borrow_mut()
                            .hosting()
                            .unwrap_or(StartupHosting {
                                kind: StartupHostingKind::Offline,
                                port: 27910,
                                q1_protocol: None,
                            });
                        let mut state = refs_hosting.state.borrow_mut();
                        state.host_port = hosting.port.to_string();
                        state.host_draft = hosting;
                        state.status.clear();
                        drop(state);
                        refs_hosting.open(hosting_menu_id());
                    }),
                    false,
                ));
                let refs_mods = refs.clone();
                let root = mods_root.clone();
                controls.push(refs.clone().button(
                    "mods",
                    "Mods".to_string(),
                    5,
                    Rc::new(move || refs_mods.open(root.clone())),
                    false,
                ));
                let play = Rc::clone(&refs.options.play);
                controls.push(refs.clone().button(
                    "play",
                    if hosting_kind == StartupHostingKind::Offline {
                        "Play".to_string()
                    } else {
                        "Start server".to_string()
                    },
                    6,
                    Rc::new(move || play()),
                    false,
                ));
                controls.push(refs.clone().button(
                    "back",
                    "Back".to_string(),
                    7,
                    Rc::new({
                        let refs = refs.clone();
                        move || refs.close()
                    }),
                    false,
                ));
                controls
            }),
        );
    }

    fn register_hosting(&mut self, refs: &MenuRefs) {
        let refs = refs.clone();
        self.register(
            hosting_menu_id(),
            &refs.clone(),
            Rc::new(move || {
                let (draft, port, busy) = {
                    let state = refs.state.borrow();
                    (state.host_draft, state.host_port.clone(), state.busy)
                };
                let refs_kind = refs.clone();
                let mut controls = vec![refs.clone().choice(
                    "host-kind",
                    "Connections".to_string(),
                    menu_row(
                        0,
                        &MenuRowOptions {
                            x: None,
                            y: None,
                            width: None,
                            height: None,
                        },
                    ),
                    !busy,
                    Some(draft.kind.as_str().to_string()),
                    vec![
                        UiChoice {
                            id: "offline".to_string(),
                            label: "Local only".to_string(),
                        },
                        UiChoice {
                            id: "native-server".to_string(),
                            label: "Native game clients".to_string(),
                        },
                        UiChoice {
                            id: "unified-server".to_string(),
                            label: "This client: mixed games".to_string(),
                        },
                    ],
                    Rc::new(move |value| {
                        let kind = match value.as_str() {
                            "native-server" => StartupHostingKind::NativeServer,
                            "unified-server" => StartupHostingKind::UnifiedServer,
                            _ => StartupHostingKind::Offline,
                        };
                        refs_kind.state.borrow_mut().host_draft.kind = kind;
                    }),
                )];
                let refs_port = refs.clone();
                controls.push(refs.clone().field(
                    "host-port",
                    "Port".to_string(),
                    port,
                    5,
                    menu_row(
                        1,
                        &MenuRowOptions {
                            x: None,
                            y: None,
                            width: None,
                            height: None,
                        },
                    ),
                    !busy && draft.kind != StartupHostingKind::Offline,
                    Rc::new(move |value| {
                        refs_port.state.borrow_mut().host_port = value;
                    }),
                    Rc::new(|_| {}),
                ));
                if draft.kind == StartupHostingKind::NativeServer && draft.q1_protocol.is_some() {
                    let refs_protocol = refs.clone();
                    let selected = match draft.q1_protocol {
                        Some(ProtocolIdentity::Q1Fitzquake) => "666",
                        Some(ProtocolIdentity::Q1Rmq { .. }) => "999",
                        _ => "15",
                    }
                    .to_string();
                    let mut control = refs.clone().choice(
                        "host-q1-protocol",
                        "Quake protocol".to_string(),
                        menu_row(
                            2,
                            &MenuRowOptions {
                                x: None,
                                y: None,
                                width: None,
                                height: None,
                            },
                        ),
                        !busy,
                        Some(selected),
                        vec![
                            UiChoice {
                                id: "15".to_string(),
                                label: "NetQuake (15)".to_string(),
                            },
                            UiChoice {
                                id: "666".to_string(),
                                label: "FitzQuake (666)".to_string(),
                            },
                            UiChoice {
                                id: "999".to_string(),
                                label: "RMQ (999)".to_string(),
                            },
                        ],
                        Rc::new(move |value| {
                            let protocol = match value.as_str() {
                                "666" => ProtocolIdentity::Q1Fitzquake,
                                "999" => ProtocolIdentity::Q1Rmq {
                                    flags: PRFL_INT32COORD | PRFL_SHORTANGLE,
                                },
                                _ => ProtocolIdentity::Q1Netquake,
                            };
                            refs_protocol.state.borrow_mut().host_draft.q1_protocol = Some(protocol);
                        }),
                    );
                    control.visible = true;
                    controls.push(control);
                }
                let note_text = {
                    let model = refs.options.model.borrow_mut();
                    let single = model
                        .options()
                        .map(|options| options.mode == crate::options::GameMode::Singleplayer)
                        .unwrap_or(false);
                    let q3 = model
                        .catalog()
                        .product(&model.options().map(|options| options.product).unwrap_or_default())
                        .map(|product| product.expectation.family == GameFamily::Q3)
                        .unwrap_or(false);
                    if single && draft.kind != StartupHostingKind::Offline {
                        if q3 {
                            "Uses Deathmatch. Change rules in Combat.".to_string()
                        } else {
                            "Uses Co-op. Change rules in Combat.".to_string()
                        }
                    } else {
                        "Choose game rules and difficulty in Combat.".to_string()
                    }
                };
                let mut note = refs.label("host-mode", note_text, 118.0 + 3.0 * 34.0, false);
                note.enabled = false;
                controls.push(note);
                let refs_apply = refs.clone();
                controls.push(refs.clone().button(
                    "host-apply",
                    "Apply".to_string(),
                    4,
                    Rc::new(move || {
                        let (draft, port_text) = {
                            let state = refs_apply.state.borrow();
                            (state.host_draft, state.host_port.clone())
                        };
                        let port = if draft.kind == StartupHostingKind::Offline {
                            draft.port
                        } else {
                            match port_text.parse::<u32>() {
                                Ok(port) => port,
                                Err(_) => {
                                    refs_apply.set_status("Port must be a whole number from 1 to 65535.", false);
                                    return;
                                }
                            }
                        };
                        match refs_apply.options.model.borrow_mut().set_hosting(StartupHosting {
                            kind: draft.kind,
                            port,
                            q1_protocol: draft.q1_protocol,
                        }) {
                            Ok(()) => {
                                refs_apply.set_status("", false);
                                refs_apply.close();
                            }
                            Err(error) => refs_apply.set_status(&error.to_string(), false),
                        }
                    }),
                    true,
                ));
                controls.push(refs.back());
                controls
            }),
        );
    }
}

impl StartupMenu {
    fn register_category(&mut self, refs: &MenuRefs) {
        let refs = refs.clone();
        self.register(
            category_menu_id(),
            &refs.clone(),
            Rc::new(move || {
                let group = refs.state.borrow().group;
                let mut controls: Vec<UiControl> = refs
                    .model_rows(GROUPS[group].1)
                    .into_iter()
                    .enumerate()
                    .map(|(index, row)| row_control(&refs, &row, index))
                    .collect();
                controls.push(refs.back());
                controls
            }),
        );
    }

    fn register_llm(&mut self, refs: &MenuRefs) -> Option<UiMenuId> {
        let service = refs.options.llm.clone()?;
        let menus = register_llm_settings_menu(&refs.controller, service);
        let root = menus.root.clone();
        self.disposers.push(Box::new(|| menus.dispose()));
        Some(root)
    }

    fn register_options(&mut self, refs: &MenuRefs, llm_root: Option<UiMenuId>) {
        let refs = refs.clone();
        self.register(
            options_menu_id(),
            &refs.clone(),
            Rc::new(move || {
                let has_accessibility = refs
                    .options
                    .settings
                    .iter()
                    .any(|binding| binding.category == SettingCategory::Accessibility);
                let open_display = refs.clone();
                let open_sound = refs.clone();
                let open_controls = refs.clone();
                let open_accessibility = refs.clone();
                let open_all = refs.clone();
                let open_llm = refs.clone();
                let mut controls = vec![
                    refs.clone().button(
                        "display",
                        "Display".to_string(),
                        0,
                        Rc::new(move || open_display.open(display_menu_id())),
                        true,
                    ),
                    refs.clone().button(
                        "sound",
                        "Sound".to_string(),
                        1,
                        Rc::new(move || open_sound.open(sound_menu_id())),
                        true,
                    ),
                    refs.clone().button(
                        "controls",
                        "Controls".to_string(),
                        2,
                        Rc::new(move || open_controls.open(controls_menu_id())),
                        true,
                    ),
                ];
                if has_accessibility {
                    controls.push(refs.clone().button(
                        "accessibility",
                        "Accessibility".to_string(),
                        3,
                        Rc::new(move || open_accessibility.open(accessibility_menu_id())),
                        true,
                    ));
                }
                controls.push(refs.clone().button(
                    "all-options",
                    "All options".to_string(),
                    4,
                    Rc::new(move || open_all.open(settings_root_menu_id())),
                    true,
                ));
                if let Some(root) = &llm_root {
                    let root = root.clone();
                    controls.push(refs.clone().button(
                        "llm",
                        "LLM options".to_string(),
                        5,
                        Rc::new(move || open_llm.open(root.clone())),
                        true,
                    ));
                }
                controls.push(refs.back());
                controls
            }),
        );
    }

    fn register_settings(&mut self, refs: &MenuRefs) {
        let refs_bindings = refs.clone();
        let refs_gyro = refs.clone();
        let mut bindings = vec![SettingBinding {
            id: control_id("ui:startup:bindings"),
            label: "Bindings (Player 1)".to_string(),
            category: SettingCategory::Input,
            enabled: Rc::new(move || refs_bindings.state.borrow().binding_menu.is_some()),
            kind: qa_client::ui::settings::SettingBindingKind::Button {
                activate: Rc::new(move || {
                    if let Some(root) = refs_gyro.state.borrow().binding_menu.clone() {
                        refs_gyro.open(root);
                    }
                }),
            },
        }];
        bindings.extend(refs.options.settings.iter().cloned());
        let refs_gyro_button = refs.clone();
        let refs_gyro_enabled = refs.clone();
        bindings.push(SettingBinding {
            id: control_id("ui:startup:gyro"),
            label: "Gyro controls".to_string(),
            category: SettingCategory::Input,
            enabled: Rc::new(move || refs_gyro_enabled.state.borrow().gyro_menu.is_some()),
            kind: qa_client::ui::settings::SettingBindingKind::Button {
                activate: Rc::new(move || {
                    if let Some(root) = refs_gyro_button.state.borrow().gyro_menu.clone() {
                        refs_gyro_button.open(root);
                    }
                }),
            },
        });
        let menus = register_settings_menus(&refs.controller, &bindings, None, None);
        self.disposers.push(Box::new(|| menus.dispose()));
    }

    fn register_sound(&mut self, refs: &MenuRefs) {
        let refs = refs.clone();
        let seat = refs.options.seat.clone();
        let register_controller = Rc::clone(&refs.controller);
        let sound_controller = Rc::clone(&refs.controller);
        register_controller.borrow_mut().register(
            sound_menu_id(),
            Rc::new(move || {
                let audio: Vec<SettingBinding> = refs
                    .options
                    .settings
                    .iter()
                    .filter(|binding| binding.category == SettingCategory::Audio)
                    .cloned()
                    .collect();
                let rows = refs.model_rows(&[StartupSelectionField::Environment, StartupSelectionField::Doppler]);
                let mut controls: Vec<UiControl> = rows
                    .into_iter()
                    .enumerate()
                    .map(|(index, row)| {
                        let mut control = row_control(&refs, &row, index);
                        control.rect = Rect {
                            x: 64.0,
                            y: 118.0 + index as f32 * 38.0,
                            width: 496.0,
                            height: 34.0,
                        };
                        control
                    })
                    .collect();
                for (offset, binding) in audio.iter().enumerate() {
                    let index = controls.len() + offset;
                    controls.push(setting_control(
                        binding,
                        &Rect {
                            x: 64.0,
                            y: 118.0 + index as f32 * 38.0,
                            width: 496.0,
                            height: 34.0,
                        },
                        seat.clone(),
                    ));
                }
                let ids: Vec<UiControlId> = controls.iter().map(|control| control.id.clone()).collect();
                let height = controls.len() as f32 * 38.0;
                controls.push(refs.back());
                UiMenu {
                    scroll: Some(qa_client::ui::types::UiMenuScroll {
                        rect: Rect {
                            x: 64.0,
                            y: 118.0,
                            width: 512.0,
                            height: 298.0,
                        },
                        content_height: height,
                        controls: ids,
                    }),
                    id: sound_menu_id(),
                    title: String::new(),
                    full_screen: false,
                    controls,
                    on_open: Rc::new(|_| {}),
                    on_close: Rc::new(|_| {}),
                }
            }),
        );
        self.disposers.push(Box::new(move || {
            sound_controller.borrow_mut().unregister(&sound_menu_id())
        }));
    }

    fn register_select(&mut self, refs: &MenuRefs) {
        let refs = refs.clone();
        self.register(
            select_menu_id(),
            &refs.clone(),
            Rc::new(move || {
                let (choices, value, busy) = selection_choices(&refs);
                let pages = (choices.len().div_ceil(7)).max(1);
                {
                    let mut state = refs.state.borrow_mut();
                    state.page = state.page.min(pages - 1);
                }
                let page = refs.state.borrow().page;
                let mut controls: Vec<UiControl> = choices
                    .iter()
                    .skip(page * 7)
                    .take(7)
                    .enumerate()
                    .map(|(index, choice)| {
                        let refs = refs.clone();
                        let id = choice.id.clone();
                        let mut control = refs.clone().button(
                            &format!("choice:{id}"),
                            format!(
                                "{}{}",
                                if value == choice.id { "> " } else { "" },
                                refs.fit(
                                    &format!(
                                        "{}{}",
                                        choice.label,
                                        if choice.unavailable.is_some() {
                                            " (unavailable)"
                                        } else {
                                            ""
                                        }
                                    ),
                                    466.0,
                                    2.6
                                )
                            ),
                            index as i32,
                            Rc::new(move || {
                                let field = {
                                    let state = refs.state.borrow();
                                    (state.monster_field.clone(), state.field)
                                };
                                let result = match &field.0 {
                                    MonsterField::Class(classname) => refs
                                        .options
                                        .model
                                        .borrow_mut()
                                        .select_monster(classname.as_deref(), &id),
                                    MonsterField::Source => refs.options.model.borrow_mut().select_monster_source(&id),
                                    MonsterField::None => refs.options.model.borrow_mut().select(field.1, &id),
                                };
                                match result {
                                    Ok(()) => {
                                        refs.set_status("", false);
                                        refs.close();
                                        if field.0 == MonsterField::None
                                            && field.1 == StartupSelectionField::Enemies
                                            && id == "custom"
                                        {
                                            open_roster(&refs);
                                        }
                                    }
                                    Err(error) => refs.set_status(&error.to_string(), false),
                                }
                            }),
                            true,
                        );
                        control.enabled = choice.unavailable.is_none() && !busy;
                        control
                    })
                    .collect();
                if pages > 1 {
                    let refs_previous = refs.clone();
                    let refs_next = refs.clone();
                    controls.push(refs.clone().button(
                        "previous",
                        "Previous page".to_string(),
                        7,
                        Rc::new(move || prepare_choice_page(&refs_previous, (page + pages - 1) % pages)),
                        true,
                    ));
                    controls.push(refs.clone().button(
                        "next",
                        "Next page".to_string(),
                        8,
                        Rc::new(move || prepare_choice_page(&refs_next, (page + 1) % pages)),
                        true,
                    ));
                }
                controls.push(refs.back());
                controls
            }),
        );
    }

    fn register_roster(&mut self, refs: &MenuRefs) {
        let refs = refs.clone();
        self.register(
            roster_menu_id(),
            &refs.clone(),
            Rc::new(move || {
                let rows = refs
                    .options
                    .model
                    .borrow_mut()
                    .monster_roster_rows()
                    .unwrap_or_default();
                let pages = (rows.len().div_ceil(7)).max(1);
                {
                    let mut state = refs.state.borrow_mut();
                    state.roster_page = state.roster_page.min(pages - 1);
                }
                let roster_page = refs.state.borrow().roster_page;
                let source = refs
                    .options
                    .model
                    .borrow()
                    .monster_source_row()
                    .unwrap_or(MonsterSourceRowFallback::fallback());
                let source_label = source
                    .choices
                    .iter()
                    .find(|choice| choice.id == source.value)
                    .map_or_else(|| source.value.clone(), |choice| choice.label.clone());
                let mut controls: Vec<UiControl> = rows
                    .iter()
                    .skip(roster_page * 7)
                    .take(7)
                    .enumerate()
                    .map(|(index, row)| {
                        let refs = refs.clone();
                        let classname = row.classname.clone();
                        refs.clone().button(
                            &format!("monster:{}", row.classname.as_deref().unwrap_or("default")),
                            refs.fit(&format!("{}: {}", row.label, row.effective_label), 486.0, 2.6),
                            index as i32,
                            Rc::new(move || {
                                let mut state = refs.state.borrow_mut();
                                state.monster_field = MonsterField::Class(classname.clone());
                                state.page = 0;
                                drop(state);
                                refs.open(select_menu_id());
                            }),
                            true,
                        )
                    })
                    .collect();
                if pages > 1 {
                    let refs_previous = refs.clone();
                    let refs_next = refs.clone();
                    controls.push(refs.clone().button(
                        "previous",
                        "Previous page".to_string(),
                        7,
                        Rc::new(move || {
                            refs_previous.state.borrow_mut().roster_page = (roster_page + pages - 1) % pages;
                        }),
                        true,
                    ));
                    controls.push(refs.clone().button(
                        "next",
                        format!("Next page ({}/{pages})", roster_page + 1),
                        8,
                        Rc::new(move || {
                            refs_next.state.borrow_mut().roster_page = (roster_page + 1) % pages;
                        }),
                        true,
                    ));
                }
                let refs_source = refs.clone();
                let mut source_control = refs.clone().button(
                    "monster-source",
                    refs.fit(&format!("Monster source: {source_label}"), 486.0, 2.6),
                    0,
                    Rc::new(move || {
                        let mut state = refs_source.state.borrow_mut();
                        state.monster_field = MonsterField::Source;
                        state.page = 0;
                        drop(state);
                        refs_source.open(select_menu_id());
                    }),
                    true,
                );
                source_control.rect = Rect {
                    x: 64.0,
                    y: 78.0,
                    width: 512.0,
                    height: 26.0,
                };
                let mut all = vec![source_control];
                all.extend(controls);
                all.push(refs.back());
                all
            }),
        );
    }

    fn register_load(&mut self, refs: &MenuRefs) {
        let refs = refs.clone();
        self.register(
            load_menu_id(),
            &refs.clone(),
            Rc::new(move || {
                let saves = (refs.options.saves)();
                let pages = (saves.rows.len().div_ceil(5)).max(1);
                {
                    let mut state = refs.state.borrow_mut();
                    state.save_page = state.save_page.min(pages - 1);
                }
                let save_page = refs.state.borrow().save_page;
                let busy = refs.busy();
                let mut controls: Vec<UiControl> = saves
                    .rows
                    .iter()
                    .skip(save_page * 5)
                    .take(5)
                    .enumerate()
                    .map(|(index, save)| {
                        let refs = refs.clone();
                        let id = save.id.clone();
                        let mut control = refs.clone().button(
                            &format!("save:{}", save.id),
                            refs.fit(&format!("{}  -  {}", save.label, save.map), 486.0, 2.6),
                            index as i32,
                            Rc::new(move || (refs.options.load)(id.clone())),
                            true,
                        );
                        control.rect = Rect {
                            x: 64.0,
                            y: 118.0 + index as f32 * 46.0,
                            width: 512.0,
                            height: 42.0,
                        };
                        control.enabled = save.unavailable.is_none() && !busy;
                        control
                    })
                    .collect();
                if pages > 1 {
                    let refs_older = refs.clone();
                    controls.push(refs.clone().button(
                        "older-saves",
                        "Older saves".to_string(),
                        7,
                        Rc::new(move || {
                            refs_older.state.borrow_mut().save_page = (save_page + 1) % pages;
                        }),
                        true,
                    ));
                }
                let refs_refresh = refs.clone();
                controls.push(refs.clone().button(
                    "refresh-saves",
                    "Refresh".to_string(),
                    8,
                    Rc::new(move || (refs_refresh.options.refresh_saves)()),
                    true,
                ));
                controls.push(refs.back());
                controls
            }),
        );
    }
}

/// Fallback monster-source row when the model cannot build one.
struct MonsterSourceRowFallback;

impl MonsterSourceRowFallback {
    fn fallback() -> super::startup_selection::MonsterSourceRow {
        super::startup_selection::MonsterSourceRow {
            label: "Monster source".to_string(),
            value: "native".to_string(),
            choices: Vec::new(),
        }
    }
}

/// Selection-row choices for the select menu (donor `selectionRow`).
fn selection_choices(refs: &MenuRefs) -> (Vec<super::startup_selection::StartupSelectionChoice>, String, bool) {
    let (field, busy) = {
        let state = refs.state.borrow();
        (state.monster_field.clone(), state.busy)
    };
    match field {
        MonsterField::Source => {
            let row = refs
                .options
                .model
                .borrow()
                .monster_source_row()
                .unwrap_or_else(|_| MonsterSourceRowFallback::fallback());
            (row.choices, row.value, busy)
        }
        MonsterField::Class(classname) => {
            let row: Option<MonsterRosterRow> = refs
                .options
                .model
                .borrow_mut()
                .monster_roster_rows()
                .unwrap_or_default()
                .into_iter()
                .find(|row| row.classname == classname);
            row.map_or_else(
                || (Vec::new(), String::new(), busy),
                |row| (row.choices, row.value, busy),
            )
        }
        MonsterField::None => {
            let field = refs.state.borrow().field;
            let row = refs
                .options
                .model
                .borrow_mut()
                .draft_rows()
                .unwrap_or_default()
                .into_iter()
                .find(|row| row.id == field);
            row.map_or_else(
                || (Vec::new(), String::new(), busy),
                |row| (row.choices, row.value, busy),
            )
        }
    }
}

/// Read the selected map's roster, then open the roster menu (donor `openRoster`).
fn open_roster(refs: &MenuRefs) {
    refs.set_status("Reading this map's monster roster...", true);
    match refs.options.model.borrow_mut().prepare_monster_roster() {
        Ok(()) => {
            refs.state.borrow_mut().roster_page = 0;
            refs.set_status("", false);
            refs.open(roster_menu_id());
        }
        Err(error) => refs.set_status(&error.to_string(), false),
    }
}

/// Turn the select-menu page, checking map rules on the map field (donor `prepareChoicePage`).
fn prepare_choice_page(refs: &MenuRefs, page: usize) {
    refs.state.borrow_mut().page = page;
    if refs.state.borrow().monster_field != MonsterField::None
        || refs.state.borrow().field != StartupSelectionField::Map
    {
        return;
    }
    match refs.options.model.borrow_mut().prepare_map_choices(page * 7, 7) {
        Ok(false) => {}
        Ok(true) => refs.set_status("", false),
        Err(error) => refs.set_status(&error.to_string(), false),
    }
}

/// Category/sound row control (donor `row`).
fn row_control(refs: &MenuRefs, row: &StartupSelectionRow, index: usize) -> UiControl {
    let rect = Rect {
        x: 64.0,
        y: 118.0 + index as f32 * 34.0,
        width: 512.0,
        height: 30.0,
    };
    let busy = refs.busy();
    if row.id == StartupSelectionField::Grapple {
        let refs = refs.clone();
        return refs.clone().choice(
            "grapple",
            row.label.clone(),
            rect,
            !busy,
            Some(row.value.clone()),
            row.choices
                .iter()
                .filter(|choice| choice.unavailable.is_none())
                .map(|choice| UiChoice {
                    id: choice.id.clone(),
                    label: choice.label.clone(),
                })
                .collect(),
            Rc::new(move |value| {
                if let Err(error) = refs
                    .options
                    .model
                    .borrow_mut()
                    .select(StartupSelectionField::Grapple, &value)
                {
                    refs.set_status(&error.to_string(), false);
                }
            }),
        );
    }
    if row.id == StartupSelectionField::Grenades {
        let refs = refs.clone();
        let enabled = !busy
            && (row.value == "enabled"
                || row
                    .choices
                    .iter()
                    .any(|choice| choice.id == "enabled" && choice.unavailable.is_none()));
        return refs.clone().toggle(
            "grenades",
            row.label.clone(),
            rect,
            enabled,
            row.value == "enabled",
            Rc::new(move |on| {
                if let Err(error) = refs
                    .options
                    .model
                    .borrow_mut()
                    .select(StartupSelectionField::Grenades, if on { "enabled" } else { "disabled" })
                {
                    refs.set_status(&error.to_string(), false);
                }
            }),
        );
    }
    let selected = row
        .choices
        .iter()
        .find(|choice| choice.id == row.value)
        .map_or_else(|| row.value.clone(), |choice| choice.label.clone());
    let label = refs.fit(&format!("{}: {selected}", row.label), 486.0, 2.6);
    let field = row.id;
    let refs = refs.clone();
    refs.clone().button(
        &field
            .as_str()
            .replace("mapProduct", "map-product")
            .replace("grappleStyle", "grapple-style"),
        label,
        index as i32,
        Rc::new(move || {
            let mut state = refs.state.borrow_mut();
            state.monster_field = MonsterField::None;
            state.field = field;
            drop(state);
            refs.open(select_menu_id());
            prepare_choice_page(&refs, 0);
        }),
        true,
    )
}

impl StartupMenu {
    fn register_browser(&mut self, refs: &MenuRefs) {
        let refs = refs.clone();
        self.register(
            browser_menu_id(),
            &refs.clone(),
            Rc::new(move || {
                let Some(browser) = refs.options.browser.clone() else {
                    return vec![refs.back()];
                };
                let entries = match browser.borrow().rows() {
                    Ok(entries) => entries,
                    Err(error) => {
                        refs.set_status(&error.to_string(), false);
                        Vec::new()
                    }
                };
                let pages = (entries.len().div_ceil(3)).max(1);
                {
                    let mut state = refs.state.borrow_mut();
                    state.page = state.page.min(pages - 1);
                }
                let page = refs.state.borrow().page;
                let busy = refs.busy();
                let mut controls = vec![{
                    let browser = Rc::clone(&browser);
                    let refs = refs.clone();
                    let protocol = browser.borrow().protocol.as_str().to_string();
                    refs.clone().choice(
                        "server-protocol",
                        "Game".to_string(),
                        Rect {
                            x: 64.0,
                            y: 108.0,
                            width: 512.0,
                            height: 30.0,
                        },
                        true,
                        Some(protocol),
                        vec![
                            UiChoice {
                                id: "q1".to_string(),
                                label: "Quake".to_string(),
                            },
                            UiChoice {
                                id: "qw".to_string(),
                                label: "QuakeWorld".to_string(),
                            },
                            UiChoice {
                                id: "q2".to_string(),
                                label: "Quake II".to_string(),
                            },
                            UiChoice {
                                id: "q3".to_string(),
                                label: "Quake III Arena".to_string(),
                            },
                        ],
                        Rc::new(move |value| {
                            if let Err(error) = browser.borrow_mut().choose(&value) {
                                refs.set_status(&error.to_string(), false);
                            } else {
                                refs.state.borrow_mut().page = 0;
                            }
                        }),
                    )
                }];
                {
                    let browser = Rc::clone(&browser);
                    let refs = refs.clone();
                    let address = browser.borrow().address();
                    let change_browser = Rc::clone(&browser);
                    let query_browser = Rc::clone(&browser);
                    controls.push(refs.clone().field(
                        "server-address",
                        "Address".to_string(),
                        address,
                        255,
                        Rect {
                            x: 64.0,
                            y: 144.0,
                            width: 512.0,
                            height: 30.0,
                        },
                        !busy,
                        Rc::new(move |value| change_browser.borrow_mut().set_address(value)),
                        Rc::new(move |_| {
                            if let Err(error) = query_browser.borrow_mut().query() {
                                query_browser.borrow_mut().status = error.to_string();
                            }
                        }),
                    ));
                }
                let query_browser = Rc::clone(&browser);
                controls.push(fixed_button(
                    &refs.clone(),
                    "server-query",
                    "Query",
                    64.0,
                    180.0,
                    160.0,
                    Rc::new(move || {
                        if let Err(error) = query_browser.borrow_mut().query() {
                            query_browser.borrow_mut().status = error.to_string();
                        }
                    }),
                ));
                let lan_browser = Rc::clone(&browser);
                let lan_refs = refs.clone();
                controls.push(fixed_button(
                    &refs.clone(),
                    "server-lan",
                    "Find LAN",
                    240.0,
                    180.0,
                    160.0,
                    Rc::new(move || {
                        if let Err(error) = lan_browser.borrow_mut().scan() {
                            lan_refs.set_status(&error.to_string(), false);
                        }
                    }),
                ));
                let favorite_browser = Rc::clone(&browser);
                controls.push(fixed_button(
                    &refs.clone(),
                    "server-favorite",
                    "Favorite",
                    416.0,
                    180.0,
                    160.0,
                    Rc::new(move || {
                        if let Err(error) = favorite_browser.borrow_mut().favorite() {
                            favorite_browser.borrow_mut().status = error.to_string();
                        }
                    }),
                ));
                {
                    let browser = Rc::clone(&browser);
                    let refs = refs.clone();
                    let filter = browser.borrow().filter.clone();
                    controls.push(refs.clone().field(
                        "server-filter",
                        "Filter".to_string(),
                        filter,
                        255,
                        Rect {
                            x: 64.0,
                            y: 216.0,
                            width: 320.0,
                            height: 30.0,
                        },
                        !busy,
                        Rc::new(move |value| {
                            browser.borrow_mut().filter = value;
                            refs.state.borrow_mut().page = 0;
                        }),
                        Rc::new(|_| {}),
                    ));
                }
                let filters_label = {
                    let browser = browser.borrow();
                    if browser.hide_empty || browser.hide_full {
                        "Filters on".to_string()
                    } else {
                        "Sort/filter".to_string()
                    }
                };
                let refs_filters = refs.clone();
                controls.push(fixed_button(
                    &refs.clone(),
                    "server-sort-filter",
                    &filters_label,
                    400.0,
                    216.0,
                    176.0,
                    Rc::new(move || refs_filters.open(browser_options_menu_id())),
                ));
                for (index, entry) in entries.iter().skip(page * 3).take(3).enumerate() {
                    let key = address_key(&entry.address, true);
                    let favorite = entry
                        .sources
                        .contains(&qa_net::services::discovery::DiscoverySource::Favorite);
                    let name = entry.status.as_ref().map_or_else(
                        || browser_address(&entry.address).unwrap_or_else(|_| key.clone()),
                        |status| status.name.clone(),
                    );
                    let counts = entry.status.as_ref().map_or_else(
                        || "?".to_string(),
                        |status| format!("{}/{}", status.players, status.max_players),
                    );
                    let ping = entry
                        .ping_milliseconds
                        .map_or_else(String::new, |ping| format!("{}ms", ping.round() as i64));
                    let label = refs.fit(
                        &format!(
                            "{}{}  {counts}  {ping}",
                            if favorite { "* " } else { "" },
                            name.trim_end()
                        ),
                        490.0,
                        2.6,
                    );
                    let browser = Rc::clone(&browser);
                    let refs = refs.clone();
                    controls.push(fixed_button(
                        &refs.clone(),
                        &format!("server:{key}"),
                        &label,
                        64.0,
                        252.0 + index as f32 * 34.0,
                        512.0,
                        Rc::new(move || {
                            if let Err(error) = browser.borrow_mut().select(&key) {
                                refs.set_status(&error.to_string(), false);
                            }
                        }),
                    ));
                }
                let refs_page = refs.clone();
                controls.push(fixed_button(
                    &refs.clone(),
                    "server-page",
                    &format!("Page {}/{}", page + 1, pages),
                    64.0,
                    358.0,
                    240.0,
                    Rc::new(move || {
                        refs_page.state.borrow_mut().page = (page + 1) % pages;
                    }),
                ));
                let favorites_label = if browser.borrow().favorites_only {
                    "Favorites only"
                } else {
                    "All servers"
                }
                .to_string();
                let favorites_browser = Rc::clone(&browser);
                let refs_favorites = refs.clone();
                controls.push(fixed_button(
                    &refs.clone(),
                    "server-favorites",
                    &favorites_label,
                    320.0,
                    358.0,
                    256.0,
                    Rc::new(move || {
                        let mut browser = favorites_browser.borrow_mut();
                        browser.favorites_only = !browser.favorites_only;
                        drop(browser);
                        refs_favorites.state.borrow_mut().page = 0;
                    }),
                ));
                let connect_browser = Rc::clone(&browser);
                let connect = refs.options.connect.clone();
                controls.push(fixed_button(
                    &refs.clone(),
                    "server-connect",
                    "Connect",
                    64.0,
                    396.0,
                    160.0,
                    Rc::new(move || match connect_browser.borrow_mut().connection() {
                        Ok(connection) => {
                            if let Some(connect) = &connect {
                                connect(connection);
                            }
                        }
                        Err(error) => connect_browser.borrow_mut().status = error.to_string(),
                    }),
                ));
                let refs_details = refs.clone();
                controls.push(fixed_button(
                    &refs.clone(),
                    "server-details",
                    "Details",
                    240.0,
                    396.0,
                    160.0,
                    Rc::new(move || {
                        refs_details.state.borrow_mut().page = 0;
                        refs_details.open(browser_details_menu_id());
                    }),
                ));
                controls.push(fixed_button(
                    &refs.clone(),
                    "server-back",
                    "Back",
                    416.0,
                    396.0,
                    160.0,
                    Rc::new({
                        let refs = refs.clone();
                        move || refs.close()
                    }),
                ));
                controls
            }),
        );
    }

    fn register_browser_options(&mut self, refs: &MenuRefs) {
        let refs = refs.clone();
        self.register(
            browser_options_menu_id(),
            &refs.clone(),
            Rc::new(move || {
                let Some(browser) = refs.options.browser.clone() else {
                    return vec![refs.back()];
                };
                let refs_sort = refs.clone();
                let sort_browser = Rc::clone(&browser);
                let sort_order = sort_browser.borrow().sort_order.id().to_string();
                let mut controls = vec![refs.clone().choice(
                    "server-sort",
                    "Sort".to_string(),
                    Rect {
                        x: 64.0,
                        y: 118.0,
                        width: 512.0,
                        height: 30.0,
                    },
                    true,
                    Some(sort_order),
                    BROWSER_SORT_ORDERS
                        .iter()
                        .map(|order| UiChoice {
                            id: order.id.id().to_string(),
                            label: order.label.to_string(),
                        })
                        .collect(),
                    Rc::new(move |value| {
                        if let Err(error) = sort_browser.borrow_mut().choose_sort(&value) {
                            refs_sort.set_status(&error.to_string(), false);
                        } else {
                            refs_sort.state.borrow_mut().page = 0;
                        }
                    }),
                )];
                for (id, label, y) in [
                    ("server-hide-empty", "Hide empty", 152.0),
                    ("server-hide-full", "Hide full", 186.0),
                ] {
                    let refs = refs.clone();
                    let browser = Rc::clone(&browser);
                    let checked = if id == "server-hide-empty" {
                        browser.borrow().hide_empty
                    } else {
                        browser.borrow().hide_full
                    };
                    controls.push(refs.clone().toggle(
                        id,
                        label.to_string(),
                        Rect {
                            x: 64.0,
                            y,
                            width: 512.0,
                            height: 30.0,
                        },
                        true,
                        checked,
                        Rc::new(move |value| {
                            if id == "server-hide-empty" {
                                browser.borrow_mut().hide_empty = value;
                            } else {
                                browser.borrow_mut().hide_full = value;
                            }
                            refs.state.borrow_mut().page = 0;
                        }),
                    ));
                }
                let master_browser = Rc::clone(&browser);
                let master_address = master_browser.borrow().master_address();
                controls.push(refs.clone().field(
                    "server-master",
                    "Master address or HTTP list".to_string(),
                    master_address,
                    2048,
                    Rect {
                        x: 64.0,
                        y: 226.0,
                        width: 512.0,
                        height: 30.0,
                    },
                    true,
                    Rc::new(move |value| master_browser.borrow_mut().set_master_address(value)),
                    Rc::new(|_| {}),
                ));
                let discover_browser = Rc::clone(&browser);
                controls.push(refs.clone().button(
                    "server-discover",
                    "Find Internet servers".to_string(),
                    5,
                    Rc::new(move || {
                        if let Err(error) = discover_browser.borrow_mut().discover() {
                            discover_browser.borrow_mut().status = error.to_string();
                        }
                    }),
                    true,
                ));
                controls.push(refs.clone().button(
                    "server-filters-back",
                    "Back".to_string(),
                    7,
                    Rc::new({
                        let refs = refs.clone();
                        move || refs.close()
                    }),
                    true,
                ));
                controls
            }),
        );
    }

    fn register_browser_details(&mut self, refs: &MenuRefs) {
        let refs = refs.clone();
        self.register(
            browser_details_menu_id(),
            &refs.clone(),
            Rc::new(move || {
                let Some(browser) = refs.options.browser.clone() else {
                    return vec![refs.back()];
                };
                let lines: Vec<String> = match browser.borrow().details() {
                    Ok(lines) => lines
                        .into_iter()
                        .flat_map(|line| {
                            if line.len() <= 64 {
                                vec![line]
                            } else {
                                line.chars()
                                    .collect::<Vec<_>>()
                                    .chunks(64)
                                    .map(|chunk| chunk.iter().collect())
                                    .collect()
                            }
                        })
                        .collect(),
                    Err(error) => {
                        refs.set_status(&error.to_string(), false);
                        Vec::new()
                    }
                };
                let pages = (lines.len().div_ceil(8)).max(1);
                {
                    let mut state = refs.state.borrow_mut();
                    state.page = state.page.min(pages - 1);
                }
                let page = refs.state.borrow().page;
                let mut controls: Vec<UiControl> = lines
                    .iter()
                    .skip(page * 8)
                    .take(8)
                    .enumerate()
                    .map(|(index, line)| {
                        let mut control = refs.clone().button(
                            &format!("server-detail:{index}"),
                            line.clone(),
                            index as i32,
                            Rc::new(|| {}),
                            true,
                        );
                        control.enabled = false;
                        control
                    })
                    .collect();
                let refs_page = refs.clone();
                controls.push(refs.clone().button(
                    "server-detail-page",
                    format!("Page {}/{}", page + 1, pages),
                    8,
                    Rc::new(move || {
                        refs_page.state.borrow_mut().page = (page + 1) % pages;
                    }),
                    true,
                ));
                controls.push(refs.back());
                controls
            }),
        );
    }
}

/// Fixed-rectangle button (donor browser `button`).
fn fixed_button(
    refs: &MenuRefs,
    id: &str,
    label: &str,
    x: f32,
    y: f32,
    width: f32,
    activate: Rc<dyn Fn()>,
) -> UiControl {
    UiControl {
        id: control_id(&format!("ui:startup:{id}")),
        label: label.to_string(),
        rect: Rect {
            x,
            y,
            width,
            height: 30.0,
        },
        enabled: !refs.busy(),
        visible: true,
        kind: UiControlKind::Button {
            on_activate: Rc::new(move |_| activate()),
        },
    }
}

/// Project preferences to a controller appearance.
fn to_appearance(prefs: &UiPreferenceValues) -> qa_client::ui::types::UiAppearance {
    qa_client::ui::types::UiAppearance {
        menu_scale: prefs.menu_scale,
        text_scale: prefs.text_scale,
        high_contrast: prefs.high_contrast,
        color_mode: prefs.color_mode,
    }
}

impl StartupMenu {
    /// Open the local-lobby menu, if bound.
    pub fn open_local_lobby(&self) {
        if let Some(root) = self.state.borrow().lobby_menu.clone() {
            let _ = self.controller.borrow_mut().open_menu(&root);
        }
    }

    /// Reopen the server browser.
    pub fn resume_server_browser(&self) {
        if self.options.browser.is_some() {
            let _ = self.controller.borrow_mut().open_menu(&browser_menu_id());
        }
    }

    /// Reopen display options through the options menu.
    pub fn resume_display_options(&self) {
        let _ = self.controller.borrow_mut().open_menu(&options_menu_id());
        let _ = self.controller.borrow_mut().open_menu(&display_menu_id());
    }

    /// Latch a status message.
    pub fn set_status(&self, text: &str, busy: bool) {
        let mut state = self.state.borrow_mut();
        state.status = text.to_string();
        state.busy = busy;
    }

    /// Ensure a menu is active, opening the main menu otherwise.
    pub fn ensure_active_menu(&self) -> UiMenuId {
        if let Some(active) = self.controller.borrow().active_menu() {
            return active;
        }
        let _ = self.controller.borrow_mut().open_menu(&main_menu_id());
        main_menu_id()
    }

    fn drain_nav(&self) {
        for intent in self.nav.borrow_mut().drain(..).collect::<Vec<_>>() {
            match intent {
                NavIntent::Open(id) => {
                    let _ = self.controller.borrow_mut().open_menu(&id);
                }
                NavIntent::Close => self.controller.borrow_mut().close_menu(),
            }
        }
    }

    /// Handle one input event; returns whether it was consumed.
    pub fn input(&self, event: &SeatInputEvent) -> bool {
        if self.state.borrow().busy {
            return true;
        }
        if let SeatInputEventKind::MouseWheel { delta } = &event.kind {
            if delta.y != 0.0 {
                let active = self.controller.borrow().active_menu();
                let step = if delta.y < 0.0 { 1 } else { -1 };
                if active == Some(select_menu_id()) {
                    let choices = {
                        let refs = MenuRefs {
                            controller: Rc::clone(&self.controller),
                            options: Rc::clone(&self.options),
                            state: Rc::clone(&self.state),
                            nav: Rc::clone(&self.nav),
                        };
                        selection_choices(&refs).0.len()
                    };
                    let pages = (choices.div_ceil(7)).max(1);
                    let page = self.state.borrow().page;
                    let next = (page as i32 + step).clamp(0, pages as i32 - 1) as usize;
                    let refs = MenuRefs {
                        controller: Rc::clone(&self.controller),
                        options: Rc::clone(&self.options),
                        state: Rc::clone(&self.state),
                        nav: Rc::clone(&self.nav),
                    };
                    prepare_choice_page(&refs, next);
                    return true;
                }
                if active == Some(roster_menu_id()) {
                    let count = self
                        .options
                        .model
                        .borrow_mut()
                        .monster_roster_rows()
                        .unwrap_or_default()
                        .len();
                    let pages = (count.div_ceil(7)).max(1);
                    let page = self.state.borrow().roster_page;
                    self.state.borrow_mut().roster_page = (page as i32 + step).clamp(0, pages as i32 - 1) as usize;
                    return true;
                }
                if active == Some(load_menu_id()) {
                    let count = (self.options.saves)().rows.len();
                    let pages = (count.div_ceil(5)).max(1);
                    let page = self.state.borrow().save_page;
                    self.state.borrow_mut().save_page = (page as i32 + step).clamp(0, pages as i32 - 1) as usize;
                    return true;
                }
            }
        }
        if self.controller.borrow().active_menu() == Some(main_menu_id())
            && matches!(&event.kind, SeatInputEventKind::Key { code, .. } if *code == KeyCode::Escape as i32)
        {
            return true;
        }
        let handled = match self.controller.borrow_mut().input(event) {
            Ok(handled) => handled,
            Err(error) => {
                self.set_status(&error.to_string(), false);
                return true;
            }
        };
        self.drain_nav();
        self.ensure_active_menu();
        handled
    }

    /// Draw the menu stack through render services.
    pub fn draw(
        &self,
        context: &UiDrawContext,
        services: &mut dyn UiRenderServices,
    ) -> Result<(), qa_client::ClientError> {
        let appearance = self
            .options
            .appearance
            .as_ref()
            .map_or(DEFAULT_UI_PREFERENCES, |read| read());
        self.text.borrow_mut().bind(
            MENU_BODY_FONT_SLOT,
            if appearance.typeface == qa_client::ui::types::Typeface::Bold {
                self.options.title_font.clone()
            } else {
                self.options.font.clone()
            },
        );
        let colors = accessible_colors(
            &menu_skin(&self.options.art.skin.font).colors,
            &to_appearance(&appearance),
        );
        let active = self.controller.borrow().active_menu();
        let transform = fit_ui(&context.binding.safe_area, appearance.menu_scale)?;
        let mut sink = TextSink {
            commands: Vec::new(),
            overlay_commands: Vec::new(),
            overlay: false,
            body_font: self.options.art.skin.font.clone(),
            title_font: menu_title_font(),
            text: colors.text,
            accent: colors.accent,
        };
        let backdrop = menu_backdrop(context);
        let original_panel = menu_panel(context, active == Some(main_menu_id()));
        let panel = match &original_panel {
            UiDrawCommand::Fill { rect, .. } if appearance.high_contrast => UiDrawCommand::Fill {
                rect: *rect,
                color: colors.panel,
            },
            _ => original_panel,
        };
        let state = self.state.borrow();
        let title = if active == Some(main_menu_id()) {
            "QUAKE".to_string()
        } else if active == Some(session_menu_id()) {
            "Custom game".to_string()
        } else if active == Some(hosting_menu_id()) {
            "Host a game".to_string()
        } else if active == Some(library_menu_id()) {
            "Library".to_string()
        } else if active == Some(native_family_menu_id()) {
            "Play a game".to_string()
        } else if active == Some(native_campaign_menu_id()) {
            let mut title = family_label(state.native_family);
            if state.native_family != GameFamily::Q3 {
                title.push_str(if state.native_edition == "classic" {
                    " classic"
                } else {
                    " rerelease"
                });
            }
            title
        } else if active == Some(native_difficulty_menu_id()) {
            "Difficulty".to_string()
        } else if active == Some(category_menu_id()) {
            GROUPS[state.group].0.to_string()
        } else if active == Some(roster_menu_id()) {
            "Custom roster".to_string()
        } else if active == Some(select_menu_id()) {
            "Choose".to_string()
        } else if active.clone() == state.gyro_menu {
            "Gyro controls".to_string()
        } else if active == Some(browser_menu_id()) {
            "Find servers".to_string()
        } else if active == Some(browser_details_menu_id()) {
            "Server details".to_string()
        } else if active == Some(browser_options_menu_id()) {
            "Server filters".to_string()
        } else if active == Some(options_menu_id()) {
            "Options".to_string()
        } else if active == Some(display_menu_id()) {
            "Display".to_string()
        } else if active == Some(sound_menu_id()) {
            "Sound".to_string()
        } else if active == Some(controls_menu_id()) {
            "Controls".to_string()
        } else {
            "Load Game".to_string()
        };
        let status = state.status.clone();
        drop(state);
        let active_text = active.as_ref().map(UiMenuId::as_str).unwrap_or("");
        if !active_text.starts_with("menu:settings:")
            && !active_text.starts_with("menu:library:")
            && !active_text.starts_with("menu:bindings:")
            && !active_text.starts_with("menu:mods:")
        {
            let scale = if active == Some(main_menu_id()) { 6.0 } else { 4.0 };
            sink.push(title, 64.0, 44.0, scale, true, true);
        }
        if active == self.state.borrow().binding_menu {
            let product = self
                .options
                .model
                .borrow_mut()
                .options()
                .map(|options| options.product)
                .unwrap_or_default();
            let title = self
                .options
                .model
                .borrow()
                .catalog()
                .product(&product)
                .map(|product| product.expectation.title.clone())
                .unwrap_or_default();
            let refs = MenuRefs {
                controller: Rc::clone(&self.controller),
                options: Rc::clone(&self.options),
                state: Rc::clone(&self.state),
                nav: Rc::clone(&self.nav),
            };
            sink.push(
                refs.fit(&format!("Player 1 - {title}"), 512.0, 1.4),
                64.0,
                458.0,
                1.4,
                false,
                false,
            );
        }
        if active == Some(native_difficulty_menu_id()) {
            if let Some(preset) = self.state.borrow().native_preset.clone() {
                let refs = self.refs();
                sink.push(refs.fit(&preset.label, 512.0, 1.6), 64.0, 86.0, 1.6, false, false);
            }
        }
        if active == Some(native_campaign_menu_id()) {
            let state = self.state.borrow();
            let unavailable = self
                .native_campaigns(state.native_family, &state.native_edition)
                .into_iter()
                .find(|preset| preset.unavailable.is_some());
            drop(state);
            if let Some(preset) = unavailable {
                if let Some(reason) = preset.unavailable {
                    let refs = self.refs();
                    sink.push(refs.fit(&reason, 512.0, 1.8), 64.0, 395.0, 1.8, true, false);
                }
            }
        }
        if active == Some(select_menu_id()) && status.is_empty() {
            let refs = self.refs();
            let (choices, _, _) = selection_choices(&refs);
            let page = refs.state.borrow().page;
            let cursor = self.controller.borrow().state().cursor;
            let hovered = choices.iter().skip(page * 7).take(7).enumerate().find(|(index, _)| {
                contains(
                    &menu_row(
                        *index as i32,
                        &MenuRowOptions {
                            x: None,
                            y: None,
                            width: None,
                            height: None,
                        },
                    ),
                    cursor,
                )
            });
            if let Some((_, choice)) = hovered {
                if let Some(reason) = &choice.unavailable {
                    sink.push(refs.fit(reason, 512.0, 1.5), 64.0, 458.0, 1.5, true, false);
                }
            }
        }
        if active == Some(roster_menu_id()) {
            sink.push(
                "Map counts shown. * Custom override.".to_string(),
                64.0,
                460.0,
                1.5,
                false,
                false,
            );
        }
        sink.commands.push(UiDrawCommand::Fill {
            rect: Rect {
                x: 64.0,
                y: 104.0,
                width: if active == Some(main_menu_id()) { 224.0 } else { 512.0 },
                height: 1.0,
            },
            color: Vec4 {
                x: 0.6,
                y: 0.39,
                z: 0.18,
                w: 0.65,
            },
        });
        if active == Some(session_menu_id()) {
            let refs = self.refs();
            sink.push("Your game".to_string(), 316.0, 118.0, 2.0, true, false);
            let fields = [
                StartupSelectionField::Product,
                StartupSelectionField::MapProduct,
                StartupSelectionField::Map,
                StartupSelectionField::Movement,
                StartupSelectionField::Character,
                StartupSelectionField::Model,
                StartupSelectionField::Weapons,
                StartupSelectionField::Enemies,
                StartupSelectionField::Grapple,
                StartupSelectionField::GrappleStyle,
                StartupSelectionField::Grenades,
                StartupSelectionField::Mode,
            ];
            let rows = refs.model_rows(&fields);
            let raw = startup_summary_layout(
                rows.len() as f64,
                &SummaryBounds {
                    x: 316.0,
                    y: 142.0,
                    width: 260.0,
                    height: 300.0,
                },
            );
            let layout = StartupSummaryF32 {
                x: raw.x as f32,
                y: raw.y as f32,
                width: raw.width as f32,
                row_height: raw.row_height as f32,
                label_scale: raw.label_scale as f32,
                value_scale: raw.value_scale as f32,
                value_offset: raw.value_offset as f32,
            };
            for (index, row) in rows.iter().enumerate() {
                let value = row
                    .choices
                    .iter()
                    .find(|choice| choice.id == row.value)
                    .map_or_else(|| row.value.clone(), |choice| choice.label.clone());
                let y = layout.y + index as f32 * layout.row_height;
                sink.push(
                    refs.fit(&row.label, layout.width, layout.label_scale),
                    layout.x,
                    y,
                    layout.label_scale,
                    true,
                    false,
                );
                sink.push(
                    refs.fit(
                        &value
                            .replace(" (campaign default)", "")
                            .replace(" authored monsters", " monsters"),
                        layout.width,
                        layout.value_scale,
                    ),
                    layout.x,
                    y + layout.value_offset,
                    layout.value_scale,
                    false,
                    false,
                );
            }
        }
        if active == Some(browser_menu_id()) {
            if let Some(browser) = self.options.browser.clone() {
                if status.is_empty() {
                    let refs = self.refs();
                    let browser_ref = browser.borrow();
                    let entries = browser_ref.rows().unwrap_or_default();
                    if entries.is_empty() {
                        sink.push(browser_ref.empty_message(), 64.0, 430.0, 1.5, false, false);
                    }
                    let selected = entries.iter().find(|entry| {
                        address_key(&entry.address, true) == browser_ref.selected.clone().unwrap_or_default()
                    });
                    let line = match selected.and_then(|entry| entry.status.clone()) {
                        None => browser_ref.status.clone(),
                        Some(status) => format!("{} — {}", status.map, browser_ref.status),
                    };
                    sink.push(refs.fit(&line, 512.0, 1.8), 64.0, 450.0, 1.8, false, false);
                }
            }
        }
        if active == Some(load_menu_id()) {
            let saves = (self.options.saves)();
            if saves.rows.is_empty() {
                sink.push("No saved games".to_string(), 64.0, 138.0, 2.5, false, false);
                sink.push(
                    "Your saved games will appear here.".to_string(),
                    64.0,
                    174.0,
                    1.8,
                    false,
                    false,
                );
            }
            sink.overlay = true;
            let save_page = self.state.borrow().save_page;
            let cursor = self.controller.borrow().state().cursor;
            let refs = self.refs();
            for (index, save) in saves.rows.iter().skip(save_page * 5).take(5).enumerate() {
                let stamp = save.saved_at_milliseconds;
                sink.push(
                    refs.fit(&format!("{}  -  {stamp}", save.game), 486.0, 1.6),
                    74.0,
                    144.0 + index as f32 * 46.0,
                    1.6,
                    false,
                    false,
                );
                if save.unavailable.is_some()
                    && contains(
                        &Rect {
                            x: 64.0,
                            y: 118.0 + index as f32 * 46.0,
                            width: 512.0,
                            height: 42.0,
                        },
                        cursor,
                    )
                {
                    if let Some(reason) = &save.unavailable {
                        sink.push(refs.fit(reason, 512.0, 1.6), 64.0, 360.0, 1.6, true, false);
                    }
                }
            }
            if let Some(error) = saves.error {
                sink.push(refs.fit(&error, 512.0, 1.5), 64.0, 390.0, 1.5, false, false);
            }
        }
        sink.overlay = false;
        if !status.is_empty() {
            let refs = self.refs();
            sink.push(refs.fit(&status, 512.0, 1.5), 64.0, 450.0, 1.5, true, false);
        }
        let main_commands: Vec<UiDrawCommand> = sink
            .commands
            .into_iter()
            .map(|command| transform_ui(&command, &transform))
            .collect();
        let mut head = vec![backdrop, panel];
        head.extend(main_commands);
        render_ui_commands(context, &head, services)?;
        let menu_commands = self.controller.borrow_mut().draw(context)?;
        render_ui_commands(context, &menu_commands, services)?;
        let overlay_commands: Vec<UiDrawCommand> = sink
            .overlay_commands
            .into_iter()
            .map(|command| transform_ui(&command, &transform))
            .collect();
        render_ui_commands(context, &overlay_commands, services)?;
        Ok(())
    }

    /// Caption draws for active media captions.
    pub fn caption_commands(
        &self,
        captions: &[ActiveCaption],
        context: &UiDrawContext,
        services: &mut dyn UiRenderServices,
    ) -> Result<(), qa_client::ClientError> {
        let appearance = self
            .options
            .appearance
            .as_ref()
            .map_or(DEFAULT_UI_PREFERENCES, |read| read());
        if !appearance.captions {
            return Ok(());
        }
        self.text.borrow_mut().bind(
            MENU_BODY_FONT_SLOT,
            if appearance.typeface == qa_client::ui::types::Typeface::Bold {
                self.options.title_font.clone()
            } else {
                self.options.font.clone()
            },
        );
        let Some(cap_ink) = self.options.art.skin.cap_ink else {
            return Ok(());
        };
        let area = context.binding.safe_area;
        let scale = appearance.text_scale * (area.width / 640.0).min(area.height / 480.0);
        let refs = self.refs();
        let draws = caption_commands(
            captions,
            &Rect {
                x: area.x + 8.0,
                y: area.y + area.height * 0.65,
                width: area.width - 16.0,
                height: area.height * 0.3,
            },
            &self.options.art.skin.font,
            scale,
            &|text, scale| measure_run(text, scale, &refs.measure_font()),
            &cap_ink,
        );
        render_ui_commands(context, &draws, services)?;
        Ok(())
    }

    /// Bind gyro settings, opening its menu.
    pub fn bind_gyro(&mut self, settings: GyroSettingsUi) {
        let menus = register_gyro_settings_menu(&self.controller, settings, Some(""));
        self.state.borrow_mut().gyro_menu = Some(menus.root.clone());
        self.disposers.push(Box::new(|| menus.dispose()));
    }

    /// Bind input bindings, opening their menu.
    pub fn bind_input(
        &mut self,
        input: Rc<RefCell<dyn BindingStore>>,
        actions: BindingSource,
        reset: Option<BindingResetHandle>,
    ) {
        let menus = register_binding_menus(&self.controller, input, actions, reset);
        self.state.borrow_mut().binding_menu = Some(menus.root.clone());
        self.disposers.push(Box::new(|| menus.dispose()));
    }

    /// Close the menu stack and release text fonts.
    pub fn close(&mut self) {
        self.controller.borrow_mut().close_all();
        for dispose in self.disposers.drain(..) {
            dispose();
        }
        self.text.borrow_mut().clear();
    }

    fn refs(&self) -> MenuRefs {
        MenuRefs {
            controller: Rc::clone(&self.controller),
            options: Rc::clone(&self.options),
            state: Rc::clone(&self.state),
            nav: Rc::clone(&self.nav),
        }
    }
}

/// f32 view of the summary layout.
struct StartupSummaryF32 {
    x: f32,
    y: f32,
    width: f32,
    row_height: f32,
    label_scale: f32,
    value_scale: f32,
    value_offset: f32,
}

/// Draw-text sink splitting base and overlay commands.
struct TextSink {
    commands: Vec<UiDrawCommand>,
    overlay_commands: Vec<UiDrawCommand>,
    overlay: bool,
    body_font: ResourceId,
    title_font: ResourceId,
    text: Vec4,
    accent: Vec4,
}

impl TextSink {
    fn push(&mut self, value: String, x: f32, y: f32, scale: f32, accent: bool, heading: bool) {
        let command = UiDrawCommand::Text {
            origin: qa_core::math::vec2(x, y),
            text: value,
            font: if heading {
                self.title_font.clone()
            } else {
                self.body_font.clone()
            },
            scale,
            color: if accent { self.accent } else { self.text },
            align: TextAlign::Left,
            shadow: true,
        };
        if self.overlay {
            self.overlay_commands.push(command);
        } else {
            self.commands.push(command);
        }
    }
}

/// Family label (donor `familyLabel`).
fn family_label(family: GameFamily) -> String {
    match family {
        GameFamily::Q1 => "Quake".to_string(),
        GameFamily::Q2 => "Quake II".to_string(),
        GameFamily::Q3 => "Quake III Arena".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use qa_client::render::scene::resources::SceneImageRegistry;
    use qa_client::render::types::ImageLevel;
    use qa_client::render::types::ResourceOwner;
    use qa_client::text::atlas::classic_charset;
    use qa_client::text::draw2d::Draw2D;
    use qa_client::text::draw2d::ImagePicture;
    use qa_client::text::draw2d::PictureAsset;
    use qa_client::ui::common::assets::load_native_ui_art;
    use qa_client::ui::common::draw::UiEmitCommand;
    use qa_client::ui::common::draw::UiMaterialDraw;
    use qa_client::ui::types::ContentId as UiContentId;
    use qa_client::ui::types::DopplerSelection as UiDoppler;
    use qa_client::ui::types::EnvironmentSelection as UiEnvironment;
    use qa_client::ui::types::PresentationSelection as UiPresentation;
    use qa_client::ui::types::ProviderRef as UiProviderRef;
    use qa_client::ui::types::SeatPresentationBinding;
    use qa_client::ClientError;
    use qa_content::catalog::CatalogArchive;
    use qa_content::catalog::CatalogProduct;
    use qa_content::catalog::InstalledCatalog;
    use qa_content::catalog::LaunchPreset;
    use qa_content::catalog::LaunchQvmCompatibility;
    use qa_content::catalog::LaunchWeaponSources;
    use qa_content::catalog::ProductAvailability;
    use qa_content::catalog::ProductExpectation;
    use qa_content::contract::ContentId;
    use qa_content::contract::ExecutableRecipe;
    use qa_content::contract::ModDescription;
    use qa_content::contract::ModSelection;
    use qa_content::contract::ProviderReference;
    use qa_content::mounts::MountPreparationScope;
    use qa_core::identity::IdentityOwner;

    use super::super::startup_selection::PreparedQ3Catalog;
    use super::super::startup_selection::PreparedTeamArena;
    use super::super::startup_selection::QvmGrappleStyle;
    use super::super::startup_selection::StartupArenaSelection;
    use super::super::startup_selection::StartupPlayerProducts;
    use super::super::startup_selection::StartupSelectionCollaborators;
    use super::*;
    use crate::options::ApplicationOptions;
    use crate::options::Network;

    struct FakeCollaborators;

    impl StartupSelectionCollaborators for FakeCollaborators {
        fn duplicate(&self) -> Box<dyn StartupSelectionCollaborators> {
            Box::new(FakeCollaborators)
        }
        fn mod_choices(&self, _catalog: &InstalledCatalog) -> Result<Vec<ModDescription>, String> {
            Ok(Vec::new())
        }
        fn apply_mods(
            &self,
            recipe: ExecutableRecipe,
            _choices: &[ModDescription],
            _mods: &[ModSelection],
        ) -> Result<ExecutableRecipe, String> {
            Ok(recipe)
        }
        fn read_arena_selection(
            &self,
            _catalog: &InstalledCatalog,
            _options: &ApplicationOptions,
        ) -> Result<StartupArenaSelection, String> {
            Ok(StartupArenaSelection::default())
        }
        fn prepare_q3_product(
            &self,
            catalog: &InstalledCatalog,
            _product_id: &str,
            _initial: &ApplicationOptions,
        ) -> Result<PreparedQ3Catalog, String> {
            Ok(PreparedQ3Catalog {
                catalog: catalog.clone(),
                q3_product: None,
            })
        }
        fn load_team_arena(
            &self,
            _catalog: &InstalledCatalog,
            _initial: &ApplicationOptions,
        ) -> Result<Option<PreparedTeamArena>, String> {
            Ok(None)
        }
        fn qvm_grapple_selection(
            &self,
            _catalog: &InstalledCatalog,
            _product_id: &str,
            _mounts: &MountPreparationScope,
        ) -> Result<Option<QvmGrappleStyle>, String> {
            Ok(None)
        }
        fn player_products(
            &self,
            _catalog: &InstalledCatalog,
            product: &str,
            _movement: GameFamily,
            _movement_product: Option<&str>,
            _character: GameFamily,
            _network: &Network,
        ) -> Result<StartupPlayerProducts, String> {
            Ok(StartupPlayerProducts {
                movement: product.to_string(),
                character: product.to_string(),
            })
        }
        fn application_preset(
            &self,
            _catalog: &InstalledCatalog,
            _options: &ApplicationOptions,
            _movement: Option<&ProviderReference>,
            _character: Option<&ProviderReference>,
        ) -> Result<LaunchPreset, String> {
            Err("no preset in tests".to_string())
        }
        fn launch_weapons(&self) -> Box<dyn LaunchWeaponSources> {
            Box::new(FakeWeapons)
        }
        fn launch_compat(&self) -> Box<dyn LaunchQvmCompatibility> {
            Box::new(FakeCompat)
        }
    }

    struct FakeWeapons;

    impl LaunchWeaponSources for FakeWeapons {
        fn canonical_weapon_source(
            &self,
            _map: &ProviderReference,
            weapon: &ProviderReference,
            _catalog: &InstalledCatalog,
        ) -> Result<ProviderReference, qa_content::catalog::CatalogError> {
            Ok(weapon.clone())
        }
        fn selected_weapon_resources(
            &self,
            _map: &ProviderReference,
            _weapons: &[ProviderReference],
            _catalog: &InstalledCatalog,
        ) -> Result<Vec<qa_content::contract::ResourceRequest>, qa_content::catalog::CatalogError> {
            Ok(Vec::new())
        }
        fn selected_weapon_timing(
            &self,
            _map: &ProviderReference,
            _weapons: &[ProviderReference],
            _catalog: &InstalledCatalog,
        ) -> Result<Vec<qa_content::contract::ProviderTiming>, qa_content::catalog::CatalogError> {
            Ok(Vec::new())
        }
        fn admit_weapon_timing(
            &self,
            _timing: &mut Vec<qa_content::contract::ProviderTiming>,
            _weapon: &qa_content::contract::ProviderTiming,
        ) -> Result<(), qa_content::catalog::CatalogError> {
            Ok(())
        }
        fn weapon_provider_ids(&self) -> Vec<qa_core::identity::ProviderId> {
            Vec::new()
        }
    }

    struct FakeCompat;

    impl LaunchQvmCompatibility for FakeCompat {
        fn read_qvm_compatibility(
            &self,
            _mounts: &dyn qa_content::catalog::BehaviorMounts,
            _artifact_path: &str,
            _digest: &qa_content::contract::ContentDigest,
            _role: qa_content::catalog::QvmCompatRole,
        ) -> Result<qa_content::contract::QvmAbiProfile, qa_content::catalog::CatalogError> {
            Err(qa_content::catalog::CatalogError::Invalid(
                "no qvm in tests".to_string(),
            ))
        }
    }

    fn product(id: &str, family: GameFamily, edition: &str, campaign: &str) -> CatalogProduct {
        CatalogProduct {
            id: ContentId(id.to_string()),
            expectation: ProductExpectation {
                id: id.to_string(),
                family,
                edition: edition.to_string(),
                campaign: campaign.to_string(),
                title: id.to_string(),
                content_directory: campaign.to_string(),
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
        InstalledCatalog::new(
            "/tmp/qa-startup-menu-test".to_string(),
            vec![
                product("q1-classic-id1", GameFamily::Q1, "classic", "id1"),
                product("q1-quakeworld", GameFamily::Q1, "quakeworld", "qw"),
                product("q2-rerelease-baseq2", GameFamily::Q2, "rerelease", "baseq2"),
            ],
            Vec::<CatalogArchive>::new(),
            0,
            None,
        )
        .unwrap()
    }

    fn solid(width: u32, height: u32) -> ImageLevel {
        ImageLevel {
            width,
            height,
            pixels: vec![9; (width * height * 4) as usize],
        }
    }

    fn art() -> NativeUiArt {
        let authority = IdentityOwner::create("startup-menu-art-test").unwrap();
        let mut images = SceneImageRegistry::new(ResourceOwner::new(1, authority.session().clone(), 0));
        let font = ResourceId::new("resource:test:font").unwrap();
        let mut read = |path: &str| match path {
            "assets/ui/menu-background.png" => Ok(solid(1536, 1024)),
            "assets/ui/main-menu-background.png" => Ok(solid(1672, 941)),
            "assets/ui/menu-panel.png" => Ok(solid(1254, 1254)),
            "assets/ui/menu-focus.png" => Ok(solid(2172, 724)),
            other => Err(ClientError::BadUi(format!("missing asset: {other}"))),
        };
        load_native_ui_art(&font, &mut images, &mut read).unwrap()
    }

    fn font() -> TextFontSelection {
        TextFontSelection::Classic {
            classic: classic_charset(7, 128, 128, "conchars", true).unwrap(),
            unicode: None,
        }
    }

    fn menu() -> StartupMenu {
        let authority = IdentityOwner::create("startup-menu-test").unwrap();
        let options = ApplicationOptions {
            product: "q1-classic-id1".to_string(),
            map: "maps/start.bsp".to_string(),
            character_model: "player".to_string(),
            ..ApplicationOptions::default()
        };
        let model = StartupSelectionModel::new(catalog(), options, Box::new(FakeCollaborators)).unwrap();
        StartupMenu::new(StartupMenuOptions {
            lobby: None,
            sound: None,
            llm: None,
            clipboard: None,
            seat: authority.seat(0),
            model: Rc::new(RefCell::new(model)),
            art: art(),
            font: font(),
            title_font: font(),
            now: Rc::new(|| 0),
            play: Rc::new(|| {}),
            play_preset: None,
            browser: None,
            connect: None,
            load: Rc::new(|_| {}),
            saves: Rc::new(StartupSaveList::default),
            refresh_saves: Rc::new(|| {}),
            quit: Rc::new(|| {}),
            settings: Vec::new(),
            appearance: None,
            team_arena: None,
            libraries: None,
        })
    }

    fn key(seat: SeatId, code: i32, down: bool) -> SeatInputEvent {
        SeatInputEvent {
            seat,
            time_ms: 0,
            kind: SeatInputEventKind::Key {
                code,
                down,
                repeat: false,
            },
        }
    }

    struct FakeServices {
        texts: Vec<String>,
        emits: Vec<UiEmitCommand>,
    }

    impl UiRenderServices for FakeServices {
        fn draw_text(
            &mut self,
            _context: &UiDrawContext,
            command: &UiDrawCommand,
            _draw: &mut Draw2D,
        ) -> Result<(), ClientError> {
            if let UiDrawCommand::Text { text, .. } = command {
                self.texts.push(text.clone());
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
        fn picture(&self, _resource: &ResourceId) -> Result<PictureAsset, ClientError> {
            Ok(self.white())
        }
        fn emit(&mut self, command: UiEmitCommand) {
            self.emits.push(command);
        }
        fn material(&mut self, _draw: UiMaterialDraw) {}
    }

    fn draw_context(menu: &StartupMenu) -> UiDrawContext {
        let authority = IdentityOwner::create("startup-menu-draw-test").unwrap();
        let viewport = Rect {
            x: 0.0,
            y: 0.0,
            width: 640.0,
            height: 480.0,
        };
        let _ = menu;
        UiDrawContext {
            binding: SeatPresentationBinding {
                seat: menu.controller.borrow().seat(),
                client: authority.client(0, 0),
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
            time_ms: 0,
        }
    }

    #[test]
    fn opens_main_and_navigates_to_family() {
        let menu = menu();
        assert_eq!(menu.active_menu(), Some(main_menu_id()));
        let seat = menu.controller.borrow().seat();
        assert!(menu.input(&key(seat.clone(), KeyCode::Enter as i32, true)));
        assert_eq!(menu.active_menu(), Some(native_family_menu_id()));
    }

    #[test]
    fn escape_on_main_is_consumed() {
        let menu = menu();
        let seat = menu.controller.borrow().seat();
        assert!(menu.input(&key(seat, KeyCode::Escape as i32, true)));
        assert_eq!(menu.active_menu(), Some(main_menu_id()));
    }

    #[test]
    fn busy_consumes_input() {
        let menu = menu();
        menu.set_status("working", true);
        assert!(menu.is_busy());
        let seat = menu.controller.borrow().seat();
        assert!(menu.input(&key(seat, KeyCode::Enter as i32, true)));
    }

    #[test]
    fn draw_emits_menu_commands() {
        let menu = menu();
        let context = draw_context(&menu);
        let mut services = FakeServices {
            texts: Vec::new(),
            emits: Vec::new(),
        };
        menu.draw(&context, &mut services).unwrap();
        assert!(services.texts.iter().any(|text| text == "QUAKE"));
        assert!(!services.emits.is_empty());
    }

    #[test]
    fn captions_stay_home_when_disabled() {
        let menu = menu();
        let context = draw_context(&menu);
        let mut services = FakeServices {
            texts: Vec::new(),
            emits: Vec::new(),
        };
        menu.caption_commands(&[], &context, &mut services).unwrap();
        assert!(services.texts.is_empty());
    }

    #[test]
    fn close_clears_the_stack() {
        let mut menu = menu();
        menu.close();
        assert_eq!(menu.active_menu(), None);
    }

    /// wu-16: keyboard and gamepad menu interaction. The main menu holds
    /// four buttons (native, load, options, quit); these tests drive
    /// [`StartupMenu::input`] the way the windowed seat router would and
    /// assert focus and selection state changes.
    mod interaction {
        use std::cell::Cell;

        use qa_client::input::ControllerAxis;
        use qa_client::ui::types::SeatInputFocus;

        use super::*;

        fn focus_id(menu: &StartupMenu) -> Option<String> {
            match menu.state().focus {
                SeatInputFocus::Menu { control, .. } => control.map(|id| id.as_str().to_string()),
                _ => None,
            }
        }

        fn pad_button(seat: SeatId, button: u8, down: bool) -> SeatInputEvent {
            SeatInputEvent {
                seat,
                time_ms: 0,
                kind: SeatInputEventKind::ControllerButton {
                    device: 0,
                    button: i32::from(button),
                    down,
                },
            }
        }

        fn pad_axis(seat: SeatId, axis: ControllerAxis, value: f32) -> SeatInputEvent {
            SeatInputEvent {
                seat,
                time_ms: 0,
                kind: SeatInputEventKind::ControllerAxis { device: 0, axis, value },
            }
        }

        fn menu_with_quit() -> (StartupMenu, Rc<Cell<bool>>) {
            let flag = Rc::new(Cell::new(false));
            let quit = Rc::clone(&flag);
            let authority = IdentityOwner::create("startup-menu-quit-test").unwrap();
            let options = ApplicationOptions {
                product: "q1-classic-id1".to_string(),
                map: "maps/start.bsp".to_string(),
                character_model: "player".to_string(),
                ..ApplicationOptions::default()
            };
            let model = StartupSelectionModel::new(catalog(), options, Box::new(FakeCollaborators)).unwrap();
            let menu = StartupMenu::new(StartupMenuOptions {
                lobby: None,
                sound: None,
                llm: None,
                clipboard: None,
                seat: authority.seat(0),
                model: Rc::new(RefCell::new(model)),
                art: art(),
                font: font(),
                title_font: font(),
                now: Rc::new(|| 0),
                play: Rc::new(|| {}),
                play_preset: None,
                browser: None,
                connect: None,
                load: Rc::new(|_| {}),
                saves: Rc::new(StartupSaveList::default),
                refresh_saves: Rc::new(|| {}),
                quit: Rc::new(move || quit.set(true)),
                settings: Vec::new(),
                appearance: None,
                team_arena: None,
                libraries: None,
            });
            (menu, flag)
        }

        #[test]
        fn arrow_keys_walk_focus_through_main_buttons() {
            let menu = menu();
            let seat = menu.controller.borrow().seat();
            assert_eq!(focus_id(&menu).as_deref(), Some("ui:startup:native"));
            assert!(menu.input(&key(seat.clone(), KeyCode::Down as i32, true)));
            assert_eq!(focus_id(&menu).as_deref(), Some("ui:startup:load"));
            assert!(menu.input(&key(seat.clone(), KeyCode::Down as i32, true)));
            assert_eq!(focus_id(&menu).as_deref(), Some("ui:startup:options"));
            assert!(menu.input(&key(seat.clone(), KeyCode::Down as i32, true)));
            assert_eq!(focus_id(&menu).as_deref(), Some("ui:startup:quit"));
            assert!(menu.input(&key(seat.clone(), KeyCode::Up as i32, true)));
            assert_eq!(focus_id(&menu).as_deref(), Some("ui:startup:options"));
        }

        #[test]
        fn focus_wraps_past_both_ends() {
            let menu = menu();
            let seat = menu.controller.borrow().seat();
            assert!(menu.input(&key(seat.clone(), KeyCode::Up as i32, true)));
            assert_eq!(focus_id(&menu).as_deref(), Some("ui:startup:quit"));
            assert!(menu.input(&key(seat.clone(), KeyCode::Down as i32, true)));
            assert_eq!(focus_id(&menu).as_deref(), Some("ui:startup:native"));
        }

        #[test]
        fn gamepad_buttons_walk_focus_and_activate() {
            let menu = menu();
            let seat = menu.controller.borrow().seat();
            assert!(menu.input(&pad_button(seat.clone(), 12, true)));
            assert_eq!(focus_id(&menu).as_deref(), Some("ui:startup:load"));
            assert!(menu.input(&pad_button(seat.clone(), 11, true)));
            assert_eq!(focus_id(&menu).as_deref(), Some("ui:startup:native"));
            assert!(menu.input(&pad_button(seat.clone(), 0, true)));
            assert_eq!(menu.active_menu(), Some(native_family_menu_id()));
        }

        #[test]
        fn gamepad_stick_deflection_moves_focus() {
            let menu = menu();
            let seat = menu.controller.borrow().seat();
            assert!(menu.input(&pad_axis(seat.clone(), ControllerAxis::LeftY, 0.8)));
            assert_eq!(focus_id(&menu).as_deref(), Some("ui:startup:load"));
            assert!(menu.input(&pad_axis(seat.clone(), ControllerAxis::LeftY, -0.8)));
            assert_eq!(focus_id(&menu).as_deref(), Some("ui:startup:native"));
        }

        #[test]
        fn activating_quit_runs_the_quit_callback() {
            let (menu, quit) = menu_with_quit();
            let seat = menu.controller.borrow().seat();
            for _ in 0..3 {
                assert!(menu.input(&key(seat.clone(), KeyCode::Down as i32, true)));
            }
            assert_eq!(focus_id(&menu).as_deref(), Some("ui:startup:quit"));
            assert!(!quit.get());
            assert!(menu.input(&key(seat.clone(), KeyCode::Enter as i32, true)));
            assert!(quit.get());
        }

        #[test]
        fn activating_load_opens_the_load_menu() {
            let menu = menu();
            let seat = menu.controller.borrow().seat();
            assert!(menu.input(&key(seat.clone(), KeyCode::Down as i32, true)));
            assert!(menu.input(&key(seat.clone(), KeyCode::Enter as i32, true)));
            assert_eq!(menu.active_menu(), Some(load_menu_id()));
            assert!(menu.input(&key(seat.clone(), KeyCode::Escape as i32, true)));
            assert_eq!(menu.active_menu(), Some(main_menu_id()));
        }
    }
}
