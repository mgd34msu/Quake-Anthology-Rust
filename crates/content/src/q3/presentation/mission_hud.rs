//! Quake III presentation: mission hud.
//!
//! Donor provenance: `src/content/q3/presentation/mission-hud.ts`.

use qa_core::cvar::CvarRegistry;
use qa_core::math::{vec4, Vec4};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::numeric::game_atof;
use crate::q3::base::game::numeric::GameRandom;
use crate::q3::base::shared::definitions::*;
use crate::q3::base::shared::items::{find_item_for_powerup, item_list};
use crate::q3::base::shared::player_state::*;
use crate::q3::presentation::audio::PcmSound;
use crate::q3::presentation::client_info::*;
use crate::q3::presentation::config::*;
use crate::q3::presentation::console::*;
use crate::q3::presentation::draw_icons::*;
use crate::q3::presentation::draw_tools::*;
use crate::q3::presentation::hud::ClientMedia;
use crate::q3::presentation::hud::{same, shared, Shared};
use crate::q3::presentation::mission_owner_draw::*;
use crate::q3::presentation::player_state::{ArsenalAmmoWarning, WeaponHudReader, WeaponHudStatus};
use crate::q3::presentation::resources::SoundAssetReader;
use crate::q3::presentation::retail_snapshot::{SceneModel, SceneShader};
use crate::q3::presentation::state::*;
use crate::q3::presentation::ui_adapters::*;

/// Mission HUD host services (`MissionHudHost`).
pub trait MissionHudHost {
    /// Weapon HUD report.
    fn weapon_hud(&self) -> Option<(Option<WeaponHudStatus>, ArsenalAmmoWarning)>;
    /// Sound assets.
    fn assets(&self) -> Shared<dyn SoundAssetReader>;
    /// Font registry.
    fn font_registry(&self) -> Shared<dyn FontRegistry>;
    /// Draw icons.
    fn icons(&self) -> Shared<ClientDrawIcons>;
    /// Configuration.
    fn configuration(&self) -> Shared<ClientConfiguration>;
    /// Cvar registry.
    fn cvars(&self) -> Shared<CvarRegistry>;
    /// Command buffer.
    fn commands(&self) -> Shared<dyn HudCommandBuffer>;
    /// Client store.
    fn clients(&self) -> Shared<dyn ClientInfoStore>;
    /// Random.
    fn random(&self) -> Shared<GameRandom>;
    /// Cinematics.
    fn cinematics(&self) -> Shared<dyn CinematicService>;
    /// Model painter.
    fn model_painter(&self) -> EngineUiModelPainter;
    /// Menu audio.
    fn audio(&self) -> Shared<dyn HudMenuAudio>;
    /// Configstring.
    fn config_string(&self, index: usize) -> String;
    /// Reset a player entity.
    fn reset_player_entity(&mut self, entity: &mut ClientEntity);
    /// Print.
    fn print(&mut self, text: &str);
    /// Milliseconds.
    fn milliseconds(&self) -> i32;
    /// Set the key catcher.
    fn set_key_catcher(&mut self, mask: i32);
    /// Initialize UI strings.
    fn initialize_ui_strings(&mut self);
    /// Load menu definitions.
    fn load_menu_definitions(
        &mut self,
        ctx: &mut dyn MenuLoadContext,
        set_path: &str,
        root: &MenuSource,
    ) -> MenuDefinitions;
    /// Create a menu runtime.
    fn create_menu_runtime(&mut self, seed: MenuRuntimeSeed) -> Box<dyn MenuRuntime>;
}

/// Team order (`TeamOrder`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TeamOrder {
    /// Personal voice.
    personal: &'static str,
    /// Team voice.
    team: &'static str,
    /// Button command.
    button: Option<&'static str>,
}

/// Team orders (`ORDERS`).
pub(crate) const ORDERS: [TeamOrder; 7] = [
    TeamOrder {
        personal: "onoffense",
        team: "offense",
        button: Some("+button7; wait; -button7"),
    },
    TeamOrder {
        personal: "ondefense",
        team: "defend",
        button: Some("+button8; wait; -button8"),
    },
    TeamOrder {
        personal: "onpatrol",
        team: "patrol",
        button: Some("+button9; wait; -button9"),
    },
    TeamOrder {
        personal: "onfollow",
        team: "followme",
        button: Some("+button10; wait; -button10"),
    },
    TeamOrder {
        personal: "ongetflag",
        team: "returnflag",
        button: None,
    },
    TeamOrder {
        personal: "onfollowcarrier",
        team: "followflagcarrier",
        button: None,
    },
    TeamOrder {
        personal: "oncamping",
        team: "camp",
        button: None,
    },
];

/// Mission score feeder (`MissionScoreFeeder`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum MissionScoreFeeder {
    /// Red.
    Red = 5,
    /// Blue.
    Blue = 6,
    /// Scoreboard.
    Scoreboard = 11,
}

/// Mouse 2 key (`KeyCode.Mouse2`).
pub(crate) const KEY_MOUSE2: i32 = 179;

/// Mission HUD source text (exclusive cap).
pub(crate) fn hud_source_text(value: &str, maximum: usize) -> String {
    let cut = match value.find('\0') {
        Some(end) => &value[..end],
        None => value,
    };
    if cut.chars().count() >= maximum {
        panic!("Mission HUD source string exceeds {} bytes", maximum - 1);
    }
    for ch in cut.chars() {
        if ch as u32 > 255 {
            panic!("Mission HUD text requires source byte characters");
        }
    }
    cut.to_string()
}

/// ASCII fold.
pub(crate) fn hud_fold(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_uppercase() {
                (ch as u8 + 32) as char
            } else {
                ch
            }
        })
        .collect()
}

/// Asset key (`assetKey`).
pub(crate) fn asset_key(path: Option<&str>) -> Option<String> {
    path.map(hud_fold)
}

/// Script source from bytes (`scriptSource`).
pub(crate) fn script_source(path: &str, bytes: &[u8]) -> MenuSource {
    let mut text = String::new();
    for byte in bytes {
        if *byte == 0 {
            break;
        }
        text.push(*byte as char);
    }
    MenuSource {
        path: path.to_string(),
        text,
    }
}

/// POSIX dirname.
pub(crate) fn posix_dirname(path: &str) -> &str {
    match path.rfind('/') {
        None => ".",
        Some(0) => "/",
        Some(position) => &path[..position],
    }
}

/// POSIX join + normalize.
pub(crate) fn posix_join(first: &str, second: &str) -> String {
    let absolute = second.starts_with('/') || first == "/";
    let mut parts: Vec<&str> = Vec::new();
    let combined = format!("{first}/{second}");
    for segment in combined.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            segment => parts.push(segment),
        }
    }
    let joined = parts.join("/");
    if absolute {
        format!("/{joined}")
    } else if joined.is_empty() {
        ".".to_string()
    } else {
        joined
    }
}

/// Loaded menus (`LoadedMenus`).
pub(crate) struct LoadedMenus {
    /// Definitions (retained for reloads; reads happen through the runtime).
    #[allow(dead_code)]
    definitions: MenuDefinitions,
    /// Runtime.
    runtime: Box<dyn MenuRuntime>,
}

/// Cached chat strings.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct MissionHudChat {
    /// System.
    system: String,
    /// Team 1.
    team1: String,
    /// Team 2.
    team2: String,
}

/// Team Arena HUD (`MissionHud`).
#[allow(clippy::type_complexity)]
pub struct MissionHud {
    /// Frame state.
    pub state: Shared<ClientGameState>,
    /// Static state.
    pub static_state: Shared<ClientGameStaticState>,
    /// Media.
    pub media: Shared<ClientMedia>,
    /// Host.
    pub host: Shared<dyn MissionHudHost>,
    /// Owner drawing.
    pub owner_draw: MissionOwnerDraw,
    /// Live fonts.
    pub fonts_handle: Shared<FontSet>,
    /// Chat.
    chat: Shared<MissionHudChat>,
    /// Loaded menus.
    loaded: RefCell<Option<LoadedMenus>>,
    /// Scoreboard.
    scoreboard: RefCell<Option<CapturedMenu>>,
    /// First scoreboard paint.
    scoreboard_first_time: Cell<bool>,
    /// Generation.
    generation: Cell<u64>,
    /// Load generation.
    load_generation: Cell<u64>,
    /// Closed.
    closed: Cell<bool>,
    /// Loading.
    loading: Cell<bool>,
    /// Captured menu.
    captured_menu: RefCell<Option<CapturedMenu>>,
    /// Widget assets.
    widget_assets: RefCell<UiWidgetAssets>,
    /// FX base.
    fx_base: RefCell<Option<SceneShader>>,
    /// FX colors.
    fx_colors: RefCell<[Option<SceneShader>; 7]>,
    /// Pictures by key.
    pictures: RefCell<HashMap<Option<String>, Option<Picture>>>,
    /// Sounds by key.
    sounds: RefCell<HashMap<Option<String>, Option<PcmSound>>>,
    /// Models by key.
    models: RefCell<HashMap<Option<String>, SceneModel>>,
    /// Registered fonts by path and size.
    registered_fonts: RefCell<HashMap<Option<String>, HashMap<i32, Option<RegisteredFont>>>>,
    /// Asset definitions.
    asset_definitions: RefCell<Option<MenuGlobalAssets>>,
}

impl MissionHud {
    /// Assemble a mission HUD.
    pub fn new(
        state: Shared<ClientGameState>,
        static_state: Shared<ClientGameStaticState>,
        media: Shared<ClientMedia>,
        host: Shared<dyn MissionHudHost>,
    ) -> Self {
        if state.borrow().product != Product::Missionpack
            || static_state.borrow().product != Product::Missionpack
            || !same(&media.borrow().static_state, &static_state)
            || !same(&host.borrow().icons().borrow().state, &state)
            || !same(&host.borrow().icons().borrow().tools.media, &media)
        {
            panic!("Mission HUD requires one Team Arena cgame state and media owner");
        }
        let fonts_handle = shared(zero_cgame_fonts());
        let chat = shared(MissionHudChat::default());
        let weapon_report = host.borrow().weapon_hud();
        let weapon_hud: Option<Shared<dyn WeaponHudReader>> = weapon_report.map(|report| {
            let reader: Shared<dyn WeaponHudReader> = shared(FixedWeaponHudReader { report });
            reader
        });
        let selected_state = state.clone();
        let selected_configuration = host.borrow().configuration();
        let selected_player = Rc::new(move || {
            let index = selected_configuration
                .borrow()
                .read_vm_cvar("cg_currentSelectedPlayer")
                .integer_value;
            if index < 0 || index >= selected_state.borrow().num_sorted_team_players {
                selected_configuration
                    .borrow()
                    .set_vm_integer(ClientVmCvarSymbol::CgCurrentSelectedPlayer, 0);
                return selected_configuration
                    .borrow()
                    .read_vm_cvar("cg_currentSelectedPlayer")
                    .integer_value;
            }
            index
        });
        let chat_reader = chat.clone();
        let chat_fn = Rc::new(move || {
            let chat = chat_reader.borrow();
            HudChatText {
                system: chat.system.clone(),
                team1: chat.team1.clone(),
                team2: chat.team2.clone(),
            }
        });
        let owner_draw = MissionOwnerDraw::new(
            state.clone(),
            static_state.clone(),
            media.clone(),
            MissionOwnerDrawHost {
                weapon_hud,
                icons: host.borrow().icons(),
                fonts: fonts_handle.clone(),
                configuration: host.borrow().configuration(),
                random: host.borrow().random(),
                strings: shared(MissionHudStrings(host.clone())),
                selected_player,
                chat: chat_fn,
            },
        );
        let zero = media
            .borrow()
            .resources
            .borrow()
            .picture(None)
            .map(|material| Picture { order: material.id })
            .unwrap_or(ZERO_PICTURE);
        let white = media
            .borrow()
            .resources
            .borrow()
            .picture(media.borrow().graphics.white_shader.as_ref())
            .map(|material| Picture { order: material.id })
            .unwrap_or(ZERO_PICTURE);
        Self {
            state,
            static_state,
            media,
            host,
            owner_draw,
            fonts_handle,
            chat,
            loaded: RefCell::new(None),
            scoreboard: RefCell::new(None),
            scoreboard_first_time: Cell::new(true),
            generation: Cell::new(0),
            load_generation: Cell::new(0),
            closed: Cell::new(false),
            loading: Cell::new(false),
            captured_menu: RefCell::new(None),
            widget_assets: RefCell::new(UiWidgetAssets {
                white_shader: white,
                gradient_bar: zero,
                scroll_bar: zero,
                scroll_bar_arrow_down: zero,
                scroll_bar_arrow_up: zero,
                scroll_bar_arrow_left: zero,
                scroll_bar_arrow_right: zero,
                scroll_bar_thumb: zero,
                slider_bar: zero,
                slider_thumb: zero,
            }),
            fx_base: RefCell::new(None),
            fx_colors: RefCell::new(std::array::from_fn(|_| None)),
            pictures: RefCell::new(HashMap::new()),
            sounds: RefCell::new(HashMap::new()),
            models: RefCell::new(HashMap::new()),
            registered_fonts: RefCell::new(HashMap::new()),
            asset_definitions: RefCell::new(None),
        }
    }

    /// Require an open HUD.
    fn open(&self) {
        if self.closed.get() {
            panic!("Mission HUD is disposed");
        }
    }

    /// Require a current registration generation.
    fn registration_current(&self, generation: u64) {
        self.open();
        if generation != self.generation.get() {
            panic!("Mission HUD media registration belongs to a retired lifecycle");
        }
    }

    /// Cached assets (`cachedAssets`).
    pub fn cached_assets(&self) -> (Option<SceneShader>, [Option<SceneShader>; 7], UiWidgetAssets) {
        (
            self.fx_base.borrow().clone(),
            self.fx_colors.borrow().clone(),
            *self.widget_assets.borrow(),
        )
    }

    /// Menu state (`menuState`).
    pub fn menu_state(&self) -> Option<MenuSnapshot> {
        self.loaded.borrow().as_ref().map(|loaded| loaded.runtime.snapshot())
    }

    /// Cache widget assets (`assetCache`).
    pub fn asset_cache(&self) {
        self.open();
        let generation = self.generation.get();
        let shader = |path: &str| {
            let registered = self
                .media
                .borrow()
                .resources
                .borrow_mut()
                .register_shader_no_mip(Some(path))
                .unwrap_or(None);
            self.open();
            if generation != self.generation.get() {
                panic!("Mission HUD asset registration belongs to a retired lifecycle");
            }
            registered
        };
        let gradient_bar = self
            .media
            .borrow()
            .resources
            .borrow()
            .picture(shader("ui/assets/gradientbar2.tga").as_ref())
            .map(|material| Picture { order: material.id })
            .unwrap_or(ZERO_PICTURE);
        *self.fx_base.borrow_mut() = shader("menu/art/fx_base");
        for (index, name) in ["red", "yel", "grn", "teal", "blue", "cyan", "white"]
            .iter()
            .enumerate()
        {
            self.fx_colors.borrow_mut()[index] = shader(&format!("menu/art/fx_{name}"));
        }
        let picture = |path: &str| {
            self.media
                .borrow()
                .resources
                .borrow()
                .picture(shader(path).as_ref())
                .map(|material| Picture { order: material.id })
                .unwrap_or(ZERO_PICTURE)
        };
        let white = self
            .media
            .borrow()
            .resources
            .borrow()
            .picture(self.media.borrow().graphics.white_shader.as_ref())
            .map(|material| Picture { order: material.id })
            .unwrap_or(ZERO_PICTURE);
        self.open();
        *self.widget_assets.borrow_mut() = UiWidgetAssets {
            white_shader: white,
            gradient_bar,
            scroll_bar: picture("ui/assets/scrollbar.tga"),
            scroll_bar_arrow_down: picture("ui/assets/scrollbar_arrow_dwn_a.tga"),
            scroll_bar_arrow_up: picture("ui/assets/scrollbar_arrow_up_a.tga"),
            scroll_bar_arrow_left: picture("ui/assets/scrollbar_arrow_left.tga"),
            scroll_bar_arrow_right: picture("ui/assets/scrollbar_arrow_right.tga"),
            scroll_bar_thumb: picture("ui/assets/scrollbar_thumb.tga"),
            slider_bar: picture("ui/assets/slider2.tga"),
            slider_thumb: picture("ui/assets/sliderbutt_1.tga"),
        };
    }

    /// Whether the other team has the flag.
    #[must_use]
    pub fn other_team_has_flag(&self) -> bool {
        self.owner_draw.other_team_has_flag()
    }

    /// Whether your team has the flag.
    #[must_use]
    pub fn your_team_has_flag(&self) -> bool {
        self.owner_draw.your_team_has_flag()
    }

    /// Register a picture.
    fn register_picture(&self, path: Option<&str>) -> Option<Picture> {
        self.open();
        let generation = self.generation.get();
        let shader = self
            .media
            .borrow()
            .resources
            .borrow_mut()
            .register_shader_no_mip(path)
            .unwrap_or(None);
        self.registration_current(generation);
        let picture = shader.map(|shader| {
            self.media
                .borrow()
                .resources
                .borrow()
                .picture(Some(&shader))
                .map(|material| Picture { order: material.id })
                .unwrap_or(ZERO_PICTURE)
        });
        self.pictures.borrow_mut().insert(asset_key(path), picture);
        picture
    }

    /// Register a sound.
    fn register_sound(&self, path: Option<&str>) -> Option<PcmSound> {
        self.open();
        let generation = self.generation.get();
        let sound = self.media.borrow().sound_bank.borrow_mut().register_sound(path, false);
        self.registration_current(generation);
        self.sounds.borrow_mut().insert(asset_key(path), sound.clone());
        sound
    }

    /// Register a model.
    fn register_model(&self, path: Option<&str>) -> SceneModel {
        self.open();
        let generation = self.generation.get();
        let model = self
            .media
            .borrow()
            .resources
            .borrow_mut()
            .register_model(path)
            .unwrap_or_default();
        self.registration_current(generation);
        self.models.borrow_mut().insert(asset_key(path), model.clone());
        model
    }

    /// Register a font.
    fn register_font(&self, path: Option<&str>, point_size: i32) {
        self.open();
        let generation = self.generation.get();
        let font = self
            .host
            .borrow()
            .font_registry()
            .borrow_mut()
            .register_font(path, point_size);
        self.registration_current(generation);
        self.registered_fonts
            .borrow_mut()
            .entry(asset_key(path))
            .or_default()
            .insert(point_size, font);
    }

    /// Fetch a registered font.
    fn font(&self, reference: &FontReference) -> Option<RegisteredFont> {
        match self
            .registered_fonts
            .borrow()
            .get(&asset_key(reference.path.as_deref()))
            .and_then(|sizes| sizes.get(&reference.point_size))
        {
            Some(font) => font.clone(),
            None => panic!("HUD font declaration has no completed registration"),
        }
    }

    /// Read a script source.
    fn source(&self, path: &str) -> Option<MenuSource> {
        if !self.host.borrow().assets().borrow().has_asset(path) {
            return None;
        }
        Some(script_source(
            path,
            &self
                .host
                .borrow()
                .assets()
                .borrow()
                .read_asset_sync(path)
                .unwrap_or_default(),
        ))
    }

    /// Read a menu buffer (`getMenuBuffer`).
    pub fn get_menu_buffer(&self, filename: &str) -> Option<String> {
        self.open();
        if !self.host.borrow().assets().borrow().has_asset(filename) {
            self.host
                .borrow_mut()
                .print(&format!("^1menu file not found: {filename}, using default\n"));
            return None;
        }
        let data = self
            .host
            .borrow()
            .assets()
            .borrow()
            .read_asset_sync(filename)
            .unwrap_or_default();
        if data.len() >= 32768 {
            self.host.borrow_mut().print(&format!(
                "^1menu file too large: {filename} is {}, max allowed is 32768",
                data.len()
            ));
            return None;
        }
        Some(script_source(filename, &data).text)
    }

    /// Reset strings (`resetStrings`).
    pub fn reset_strings(&self) {
        self.open();
        self.generation.set(self.generation.get() + 1);
        self.host.borrow_mut().initialize_ui_strings();
        if let Some(loaded) = self.loaded.borrow_mut().as_mut() {
            loaded.runtime.reset_definitions(MenuResetScope::Strings);
        }
    }

    /// Reset menus (`resetMenus`).
    pub fn reset_menus(&self) {
        self.open();
        self.generation.set(self.generation.get() + 1);
        if let Some(loaded) = self.loaded.borrow_mut().as_mut() {
            loaded.runtime.reset_definitions(MenuResetScope::Menus);
        }
    }

    /// Load the HUD menu (`loadHudMenu`).
    pub fn load_hud_menu(&self) {
        self.reset_menus();
        let requested: String = self
            .host
            .borrow()
            .cvars()
            .borrow()
            .get("cg_hudFiles")
            .map(|snapshot| snapshot.value.chars().take(1023).collect())
            .unwrap_or_default();
        self.load_menus(if requested.is_empty() { "ui/hud.txt" } else { &requested });
    }

    /// Load menus (`loadMenus`).
    pub fn load_menus(&self, path: &str) {
        self.open();
        if self.loading.get() {
            panic!("Mission HUD menu registration is already active");
        }
        let started = self.host.borrow().milliseconds();
        if !self.host.borrow().assets().borrow().has_asset(path) {
            panic!("^3menu file not found: {path}, using default\n");
        }
        struct LoadingGuard<'a> {
            hud: &'a MissionHud,
        }
        impl Drop for LoadingGuard<'_> {
            fn drop(&mut self) {
                self.hud.loading.set(false);
            }
        }
        let _guard = LoadingGuard { hud: self };
        let bytes = self
            .host
            .borrow()
            .assets()
            .borrow()
            .read_asset_sync(path)
            .unwrap_or_default();
        self.open();
        if bytes.len() >= 4096 {
            panic!("^1menu file too large: {path} is {}, max allowed is 4096", bytes.len());
        }
        self.loading.set(true);
        let root_source = script_source(path, &bytes);
        self.reset_menus();
        self.load_generation.set(self.generation.get());
        // Random draws, source resolution, registration, and asset publication
        // flow through this HUD as the load context.
        let definitions =
            self.host
                .borrow_mut()
                .load_menu_definitions(&mut MissionHudLoadContext { hud: self }, path, &root_source);
        self.open();
        if self.load_generation.get() != self.generation.get() {
            panic!("Mission HUD menu load belongs to a retired lifecycle");
        }
        *self.asset_definitions.borrow_mut() = Some(definitions.assets.clone());
        if let Some(gradient) = &definitions.assets.gradient_bar {
            let mut widgets = self.widget_assets.borrow_mut();
            widgets.gradient_bar = self
                .pictures
                .borrow()
                .get(&asset_key(gradient.path.as_deref()))
                .copied()
                .flatten()
                .unwrap_or_else(|| {
                    self.media
                        .borrow()
                        .resources
                        .borrow()
                        .picture(None)
                        .map(|material| Picture { order: material.id })
                        .unwrap_or(ZERO_PICTURE)
                });
        }
        if self.loaded.borrow().is_some() {
            {
                let mut loaded = self.loaded.borrow_mut();
                let loaded = loaded.as_mut().expect("Mission HUD menus are not loaded");
                loaded.runtime.reload_definitions(&definitions);
                loaded.definitions = definitions.clone();
            }
            self.open();
            if self.load_generation.get() != self.generation.get() {
                panic!("Mission HUD menu load belongs to a retired lifecycle");
            }
        } else {
            let seed = MenuRuntimeSeed {
                definitions: definitions.clone(),
                cvars: self.host.borrow().cvars(),
                widget_assets: *self.widget_assets.borrow(),
                zero_picture: self
                    .media
                    .borrow()
                    .resources
                    .borrow()
                    .picture(None)
                    .map(|material| Picture { order: material.id })
                    .unwrap_or(ZERO_PICTURE),
                fonts: self.fonts_handle.borrow().clone(),
            };
            let runtime = self.host.borrow_mut().create_menu_runtime(seed);
            self.open();
            if self.load_generation.get() != self.generation.get() {
                panic!("Mission HUD menu load belongs to a retired lifecycle");
            }
            *self.loaded.borrow_mut() = Some(LoadedMenus { definitions, runtime });
        }
        let elapsed = self.host.borrow().milliseconds().wrapping_sub(started);
        self.host
            .borrow_mut()
            .print(&format!("UI menu load time = {elapsed} milli seconds\n"));
    }

    /// Dispose (`dispose`).
    pub fn dispose(&self) {
        if self.closed.get() {
            return;
        }
        self.generation.set(self.generation.get() + 1);
        self.closed.set(true);
        if let Some(mut loaded) = self.loaded.borrow_mut().take() {
            loaded.runtime.retire();
        }
        *self.captured_menu.borrow_mut() = None;
    }

    /// Current player state.
    fn snapshot(&self) -> PlayerState {
        self.state
            .borrow()
            .snap
            .clone()
            .unwrap_or_else(|| {
                panic!("Mission HUD operation requires cg.snap");
            })
            .player_state
    }

    /// Selected index.
    fn selected(&self) -> i32 {
        self.host
            .borrow()
            .configuration()
            .borrow()
            .read_vm_cvar("cg_currentSelectedPlayer")
            .integer_value
    }

    /// Write the selected index.
    fn set_selected(&self, value: i32) {
        self.host
            .borrow()
            .configuration()
            .borrow()
            .set_vm_integer(ClientVmCvarSymbol::CgCurrentSelectedPlayer, value);
    }

    /// Set a cvar.
    fn set_cvar(&self, name: &str, value: &str) {
        let _ = self.host.borrow().cvars().borrow_mut().set(name, value, true);
    }

    /// Initialize team chat (`initTeamChat`).
    pub fn init_team_chat(&self) {
        *self.chat.borrow_mut() = MissionHudChat::default();
    }

    /// Set a print string (`setPrintString`).
    pub fn set_print_string(&self, kind: i32, value: &str) {
        let text = hud_source_text(value, 256);
        if kind == 0 {
            self.chat.borrow_mut().system = text;
        } else {
            let mut chat = self.chat.borrow_mut();
            chat.team2 = std::mem::take(&mut chat.team1);
            chat.team1 = text;
        }
    }

    /// Chat strings (`chat`).
    #[must_use]
    pub fn chat(&self) -> HudChatText {
        let chat = self.chat.borrow();
        HudChatText {
            system: chat.system.clone(),
            team1: chat.team1.clone(),
            team2: chat.team2.clone(),
        }
    }

    /// Append a command.
    fn append(&self, text: &str) {
        self.host.borrow().commands().borrow_mut().append(text);
    }

    /// Check a pending order (`checkOrderPending`).
    pub fn check_order_pending(&self) {
        let cgs = self.static_state.borrow();
        if (cgs.game_type as i32) < (GameType::GtCtf as i32) || !cgs.order_pending {
            return;
        }
        let current = cgs.current_order;
        drop(cgs);
        let order = ORDERS.get((current - 1) as usize);
        let selected = self.selected();
        if selected == self.state.borrow().num_sorted_team_players {
            let Some(order) = order else {
                panic!("CG_CheckOrderPending: Everyone order has no source voice command");
            };
            self.append(&format!("cmd vsay_team {}\n", order.team));
        } else {
            let client = self
                .state
                .borrow()
                .sorted_team_players
                .get(selected as usize)
                .copied()
                .unwrap_or_else(|| {
                    panic!("Mission HUD source index {selected} outside 8");
                });
            if client == self.snapshot().client_num {
                if let Some(order) = order {
                    self.append(&format!("teamtask {}\n", self.static_state.borrow().current_order));
                    self.append(&format!("cmd vsay_team {}\n", order.personal));
                }
            } else if let Some(order) = order {
                self.append(&format!("cmd vtell {client} {}\n", order.team));
            }
        }
        if let Some(order) = order {
            if let Some(button) = order.button {
                self.append(button);
            }
        }
        self.static_state.borrow_mut().order_pending = false;
    }

    /// Publish the selected player name.
    fn set_selected_player_name(&self) {
        let index = self.selected();
        if index >= 0 && index < self.state.borrow().num_sorted_team_players {
            let number = self.state.borrow().sorted_team_players[index as usize];
            let client = self.static_state.borrow().client_info[number as usize].clone();
            self.set_cvar("cg_selectedPlayerName", &client.name);
            self.set_cvar("cg_selectedPlayer", &format!("{number}"));
            self.static_state.borrow_mut().current_order = client.team_task;
        } else {
            self.set_cvar("cg_selectedPlayerName", "Everyone");
        }
    }

    /// Selected player (`getSelectedPlayer`).
    #[must_use]
    pub fn get_selected_player(&self) -> i32 {
        let index = self.selected();
        if index < 0 || index >= self.state.borrow().num_sorted_team_players {
            self.set_selected(0);
        }
        self.selected()
    }

    /// Select the next player (`selectNextPlayer`).
    pub fn select_next_player(&self) {
        self.check_order_pending();
        let index = self.selected();
        self.set_selected(if index >= 0 && index < self.state.borrow().num_sorted_team_players {
            index + 1
        } else {
            0
        });
        self.set_selected_player_name();
    }

    /// Select the previous player (`selectPreviousPlayer`).
    pub fn select_previous_player(&self) {
        self.check_order_pending();
        let index = self.selected();
        self.set_selected(if index > 0 && index < self.state.borrow().num_sorted_team_players {
            index - 1
        } else {
            self.state.borrow().num_sorted_team_players
        });
        self.set_selected_player_name();
    }

    /// Feeder count (`feederCount`).
    pub fn feeder_count(&self, feeder: i32) -> i32 {
        if feeder == MissionScoreFeeder::Scoreboard as i32 {
            return self.state.borrow().num_scores;
        }
        let team = if feeder == MissionScoreFeeder::Red as i32 {
            Team::TeamRed as i32
        } else if feeder == MissionScoreFeeder::Blue as i32 {
            Team::TeamBlue as i32
        } else {
            return 0;
        };
        let mut count = 0;
        for index in 0..self.state.borrow().num_scores {
            if self.state.borrow().scores[index as usize].team == team {
                count += 1;
            }
        }
        count
    }

    /// Score + info for a feeder row.
    fn info_from_score_index(&self, index: i32, team: i32) -> (ClientScore, ClientInfo) {
        let mut score_index = index;
        if (self.static_state.borrow().game_type as i32) >= (GameType::GtTeam as i32) {
            let mut count = 0;
            for position in 0..self.state.borrow().num_scores {
                if self.state.borrow().scores[position as usize].team != team {
                    continue;
                }
                if count == index {
                    score_index = position;
                    break;
                }
                count += 1;
            }
        }
        let score = self
            .state
            .borrow()
            .scores
            .get(score_index as usize)
            .copied()
            .unwrap_or_else(|| {
                panic!("Mission HUD source index {score_index} outside 64");
            });
        let info = self
            .static_state
            .borrow()
            .client_info
            .get(score.client as usize)
            .cloned()
            .unwrap_or_else(|| {
                panic!("Mission HUD source index {} outside 64", score.client);
            });
        (score, info)
    }

    /// Feeder item (`feederItem`).
    pub fn feeder_item(&self, feeder: i32, index: i32, column: i32) -> MenuFeederItem {
        let team = if feeder == MissionScoreFeeder::Red as i32 {
            Team::TeamRed as i32
        } else if feeder == MissionScoreFeeder::Blue as i32 {
            Team::TeamBlue as i32
        } else {
            -1
        };
        let (score, info_handle) = self.info_from_score_index(index, team);
        let info = info_handle;
        let mut text = String::new();
        let mut picture = None;
        if info.info_valid {
            match column {
                0 => {
                    let mut powerup = None;
                    if info.powerups & (1 << Powerup::PwNeutralflag as i32) != 0 {
                        powerup = Some(Powerup::PwNeutralflag);
                    } else if info.powerups & (1 << Powerup::PwRedflag as i32) != 0 {
                        powerup = Some(Powerup::PwRedflag);
                    } else if info.powerups & (1 << Powerup::PwBlueflag as i32) != 0 {
                        powerup = Some(Powerup::PwBlueflag);
                    }
                    if let Some(powerup) = powerup {
                        let item = find_item_for_powerup(self.state.borrow().product, powerup);
                        let Some(item) = item else {
                            panic!("Mission HUD flag has no source item");
                        };
                        let Some(at) = item_list(self.state.borrow().product)
                            .iter()
                            .position(|candidate| std::ptr::eq(candidate as *const _, item as *const _))
                        else {
                            panic!("Mission HUD flag item has no catalog index");
                        };
                        let visual = self.media.borrow().weapon_registry.borrow().item_visual(at);
                        picture = Some(
                            self.media
                                .borrow()
                                .resources
                                .borrow()
                                .picture(visual.icon.as_ref())
                                .map(|material| Picture { order: material.id })
                                .unwrap_or(ZERO_PICTURE),
                        );
                    } else if info.bot_skill > 0 && info.bot_skill <= 5 {
                        let shader = self
                            .media
                            .borrow()
                            .graphics
                            .bot_skill_shaders
                            .get((info.bot_skill - 1) as usize)
                            .cloned()
                            .unwrap_or_else(|| {
                                panic!("Mission HUD source index {} outside 5", info.bot_skill - 1);
                            });
                        picture = Some(
                            self.media
                                .borrow()
                                .resources
                                .borrow()
                                .picture(shader.as_ref())
                                .map(|material| Picture { order: material.id })
                                .unwrap_or(ZERO_PICTURE),
                        );
                    } else if info.handicap < 100 {
                        text = format!("{}", info.handicap);
                    }
                }
                1 => {
                    if team != -1 {
                        let shader = self.owner_draw.status_handle(info.team_task);
                        picture = Some(
                            self.media
                                .borrow()
                                .resources
                                .borrow()
                                .picture(shader.as_ref())
                                .map(|material| Picture { order: material.id })
                                .unwrap_or(ZERO_PICTURE),
                        );
                    }
                }
                2 => {
                    let schema = stat_schema(self.state.borrow().product);
                    if self.snapshot().stats.get(schema.clients_ready()) & (1 << score.client) != 0 {
                        text = "Ready".to_string();
                    } else if team == -1 {
                        if self.static_state.borrow().game_type == GameType::GtTournament {
                            text = format!("{}/{}", info.wins, info.losses);
                        } else if info.team == Team::TeamSpectator {
                            text = "Spectator".to_string();
                        }
                    } else if info.team_leader {
                        text = "Leader".to_string();
                    }
                }
                3 => text = info.name.clone(),
                4 => text = format!("{}", info.score),
                5 => text = format!("{:>4}", score.time),
                6 => {
                    text = if score.ping == -1 {
                        "connecting".to_string()
                    } else {
                        format!("{:>4}", score.ping)
                    };
                }
                _ => {}
            }
        }
        MenuFeederItem { text, picture }
    }

    /// Feeder selection (`feederSelection`).
    pub fn feeder_selection(&self, feeder: i32, index: i32) {
        if (self.static_state.borrow().game_type as i32) < (GameType::GtTeam as i32) {
            self.state.borrow_mut().selected_score = index;
            return;
        }
        let team = if feeder == MissionScoreFeeder::Red as i32 {
            Team::TeamRed as i32
        } else {
            Team::TeamBlue as i32
        };
        let mut count = 0;
        for position in 0..self.state.borrow().num_scores {
            if self.state.borrow().scores[position as usize].team != team {
                continue;
            }
            if index == count {
                self.state.borrow_mut().selected_score = position;
            }
            count += 1;
        }
    }

    /// Set the score selection (`setScoreSelection`).
    pub fn set_score_selection(&self, menu: Option<&CapturedMenu>) {
        let player = self.snapshot();
        let mut red = 0;
        let mut blue = 0;
        for position in 0..self.state.borrow().num_scores {
            let score = self.state.borrow().scores[position as usize];
            if score.team == Team::TeamRed as i32 {
                red += 1;
            } else if score.team == Team::TeamBlue as i32 {
                blue += 1;
            }
            if player.client_num == score.client {
                self.state.borrow_mut().selected_score = position;
            }
        }
        let Some(menu) = menu else {
            return;
        };
        if self.loaded.borrow().is_none() {
            return;
        }
        if (self.static_state.borrow().game_type as i32) >= (GameType::GtTeam as i32) {
            let selected = self.state.borrow().selected_score;
            let is_blue = self.state.borrow().scores[selected as usize].team == Team::TeamBlue as i32;
            self.loaded
                .borrow_mut()
                .as_mut()
                .expect("Mission HUD menus are not loaded")
                .runtime
                .set_captured_feeder_selection(
                    menu,
                    if is_blue {
                        MissionScoreFeeder::Blue as i32
                    } else {
                        MissionScoreFeeder::Red as i32
                    },
                    if is_blue { blue } else { red },
                );
        } else {
            let selected = self.state.borrow().selected_score;
            self.loaded
                .borrow_mut()
                .as_mut()
                .expect("Mission HUD menus are not loaded")
                .runtime
                .set_captured_feeder_selection(menu, MissionScoreFeeder::Scoreboard as i32, selected);
        }
    }

    /// Clear the scoreboard (`clearScoreboard`).
    pub fn clear_scoreboard(&self) {
        *self.scoreboard.borrow_mut() = None;
    }

    /// Menu scoreboard (`menuScoreboard`).
    #[must_use]
    pub fn menu_scoreboard(&self) -> Option<CapturedMenu> {
        self.scoreboard.borrow().clone()
    }

    /// Scroll a feeder (`scrollFeeder`).
    pub fn scroll_feeder(&self, menu: &CapturedMenu, feeder: i32, down: bool) {
        if self.scoreboard.borrow().as_ref() != Some(menu) {
            panic!("Score scrolling requires the current Mission HUD scoreboard");
        }
        self.open();
        if self.loaded.borrow().is_none() {
            panic!("Mission HUD menus are not loaded");
        }
        self.loaded
            .borrow_mut()
            .as_mut()
            .expect("Mission HUD menus are not loaded")
            .runtime
            .scroll_captured_feeder(menu, feeder, down);
    }

    /// Menu frame (time fields stay zero).
    fn frame(&self) -> MenuFrame {
        MenuFrame { time: 0, frame_time: 0 }
    }

    /// Paint all menus (`paintAll`).
    pub fn paint_all(&self) {
        self.open();
        if self.loaded.borrow().is_some() {
            let frame = self.frame();
            let mut draw = self.host.borrow().icons().borrow().tools.draw.clone();
            // The runtime only calls back into feeder/owner-draw queries.
            let mut callbacks = MissionHudCallbacks { hud: self };
            self.loaded
                .borrow_mut()
                .as_mut()
                .expect("Mission HUD menus are not loaded")
                .runtime
                .frame(&frame, &mut draw, &mut callbacks);
        }
    }

    /// Draw the scoreboard (`drawScoreboard`).
    pub fn draw_scoreboard(&self) -> bool {
        self.open();
        let state_pm = self.state.borrow().predicted_player_state.pm_type;
        if self.scoreboard.borrow().is_some() && self.loaded.borrow().is_some() {
            let menu = self.scoreboard.borrow().clone().expect("scoreboard");
            self.loaded
                .borrow_mut()
                .as_mut()
                .expect("Mission HUD menus are not loaded")
                .runtime
                .clear_captured_forced(&menu);
        }
        if self
            .host
            .borrow()
            .configuration()
            .borrow()
            .read_vm_cvar("cl_paused")
            .integer_value
            != 0
            || self.static_state.borrow().game_type == GameType::GtSinglePlayer
                && state_pm == MoveType::PmIntermission as i32
        {
            self.state.borrow_mut().deferred_player_loading = 0;
            self.scoreboard_first_time.set(true);
            return false;
        }
        let (warmup, show_scores, time, score_fade) = {
            let state = self.state.borrow();
            (state.warmup, state.show_scores, state.time, state.score_fade_time)
        };
        if warmup != 0 && !show_scores {
            return false;
        }
        if !show_scores
            && state_pm != MoveType::PmDead as i32
            && state_pm != MoveType::PmIntermission as i32
            && fade_color(time, score_fade, 200.0).is_none()
        {
            self.state.borrow_mut().deferred_player_loading = 0;
            self.state.borrow_mut().killer_name = String::new();
            self.scoreboard_first_time.set(true);
            return false;
        }
        if self.scoreboard.borrow().is_none() && self.loaded.borrow().is_some() {
            let name = if (self.static_state.borrow().game_type as i32) >= (GameType::GtTeam as i32) {
                "teamscore_menu"
            } else {
                "score_menu"
            };
            *self.scoreboard.borrow_mut() = self
                .loaded
                .borrow()
                .as_ref()
                .expect("Mission HUD menus are not loaded")
                .runtime
                .menu_handle(name);
        }
        if self.scoreboard.borrow().is_some() && self.loaded.borrow().is_some() {
            if self.scoreboard_first_time.get() {
                let menu = self.scoreboard.borrow().clone().expect("scoreboard");
                self.set_score_selection(Some(&menu));
                self.open();
                self.scoreboard_first_time.set(false);
            }
            let menu = self.scoreboard.borrow().clone().expect("scoreboard");
            let frame = self.frame();
            let mut draw = self.host.borrow().icons().borrow().tools.draw.clone();
            let mut callbacks = MissionHudCallbacks { hud: self };
            self.loaded
                .borrow_mut()
                .as_mut()
                .expect("Mission HUD menus are not loaded")
                .runtime
                .paint_captured(&menu, &frame, true, &mut draw, &mut callbacks);
        }
        self.state.borrow_mut().deferred_player_loading += 1;
        if self.state.borrow().deferred_player_loading > 10 {
            let host = self.host.clone();
            self.host
                .borrow()
                .clients()
                .borrow_mut()
                .load_deferred_players(&mut |entity| {
                    host.borrow_mut().reset_player_entity(entity);
                });
        }
        true
    }

    /// Close by name (`closeByName`).
    pub fn close_by_name(&self, name: &str) {
        self.open();
        if let Some(loaded) = self.loaded.borrow_mut().as_mut() {
            loaded.runtime.close(name);
        }
    }

    /// Show the response head (`showResponseHead`).
    pub fn show_response_head(&self) {
        self.open();
        if let Some(loaded) = self.loaded.borrow_mut().as_mut() {
            loaded.runtime.show("voiceMenu");
        }
        self.set_cvar("cl_conXOffset", "72");
        let time = self.state.borrow().time;
        self.state.borrow_mut().voice_time = time;
    }

    /// Draw timed menus (`drawTimedMenus`).
    pub fn draw_timed_menus(&self) {
        let (voice_time, time) = {
            let state = self.state.borrow();
            (state.voice_time, state.time)
        };
        if voice_time != 0 && time.wrapping_sub(voice_time) > 2500 {
            self.close_by_name("voiceMenu");
            self.set_cvar("cl_conXOffset", "0");
            self.state.borrow_mut().voice_time = 0;
        }
    }

    /// Client number from a name (`clientNumFromName`).
    #[must_use]
    pub fn client_num_from_name(&self, name: &str) -> i32 {
        let text = hud_fold(&hud_source_text(name, name.chars().count() + 1));
        for index in 0..self.static_state.borrow().maxclients {
            let client = self.static_state.borrow().client_info[index as usize].clone();
            if client.info_valid && hud_fold(&client.name) == text {
                return index;
            }
        }
        -1
    }

    /// Hide the team menu (`hideTeamMenu`).
    pub fn hide_team_menu(&self) {
        self.close_by_name("teamMenu");
        self.close_by_name("getMenu");
    }

    /// Show the team menu (`showTeamMenu`).
    pub fn show_team_menu(&self) {
        self.open();
        if let Some(loaded) = self.loaded.borrow_mut().as_mut() {
            loaded.runtime.show("teamMenu");
        }
    }

    /// Event handling (`eventHandling`).
    pub fn event_handling(&self, kind: i32) {
        self.static_state.borrow_mut().event_handling = kind;
        if kind == 0 {
            self.hide_team_menu();
        }
    }

    /// Mouse event (`mouseEvent`).
    pub fn mouse_event(&self, x: f32, y: f32) {
        // vmMain copies the old cgs cursor into cgDC before CG_MouseEvent applies its delta.
        if let Some(loaded) = self.loaded.borrow_mut().as_mut() {
            let (cursor_x, cursor_y) = {
                let cgs = self.static_state.borrow();
                (cgs.cursor_x, cgs.cursor_y)
            };
            loaded.runtime.set_display_cursor(cursor_x as f32, cursor_y as f32);
        }
        let pm_type = self.state.borrow().predicted_player_state.pm_type;
        if (pm_type == MoveType::PmNormal as i32 || pm_type == MoveType::PmSpectator as i32)
            && !self.state.borrow().show_scores
        {
            self.host.borrow_mut().set_key_catcher(0);
            return;
        }
        {
            let mut cgs = self.static_state.borrow_mut();
            cgs.cursor_x = (cgs.cursor_x as f32 + x).clamp(0.0, 640.0) as i32;
            cgs.cursor_y = (cgs.cursor_y as f32 + y).clamp(0.0, 480.0) as i32;
        }
        let (cursor_x, cursor_y) = {
            let cgs = self.static_state.borrow();
            (cgs.cursor_x, cgs.cursor_y)
        };
        let cursor = self
            .loaded
            .borrow()
            .as_ref()
            .map(|loaded| loaded.runtime.cursor_type(cursor_x as f32, cursor_y as f32))
            .unwrap_or(MenuCursorType::Arrow);
        self.static_state.borrow_mut().active_cursor = if cursor == MenuCursorType::Arrow {
            self.media.borrow().graphics.select_cursor.clone()
        } else {
            self.media.borrow().graphics.size_cursor.clone()
        };
        if self.loaded.borrow().is_none() {
            return;
        }
        if let Some(menu) = self.captured_menu.borrow().clone() {
            self.loaded
                .borrow_mut()
                .as_mut()
                .expect("Mission HUD menus are not loaded")
                .runtime
                .move_captured_menu(&menu, x, y);
        } else {
            self.loaded
                .borrow_mut()
                .as_mut()
                .expect("Mission HUD menus are not loaded")
                .runtime
                .pointer_move(cursor_x as f32, cursor_y as f32);
        }
    }

    /// Key event (`keyEvent`).
    pub fn key_event(&self, key: i32, down: bool) {
        if !down {
            return;
        }
        let pm_type = self.state.borrow().predicted_player_state.pm_type;
        if pm_type == MoveType::PmNormal as i32
            || pm_type == MoveType::PmSpectator as i32 && !self.state.borrow().show_scores
        {
            self.event_handling(0);
            self.host.borrow_mut().set_key_catcher(0);
            return;
        }
        if let Some(loaded) = self.loaded.borrow_mut().as_mut() {
            let (cursor_x, cursor_y) = {
                let cgs = self.static_state.borrow();
                (cgs.cursor_x, cgs.cursor_y)
            };
            loaded.runtime.handle_key(key, down, cursor_x as f32, cursor_y as f32);
        }
        if self.captured_menu.borrow().is_some() {
            *self.captured_menu.borrow_mut() = None;
        } else if key == KEY_MOUSE2 && self.loaded.borrow().is_some() {
            let (cursor_x, cursor_y) = {
                let cgs = self.static_state.borrow();
                (cgs.cursor_x, cgs.cursor_y)
            };
            *self.captured_menu.borrow_mut() = self
                .loaded
                .borrow_mut()
                .as_mut()
                .expect("Mission HUD menus are not loaded")
                .runtime
                .capture_menu(cursor_x as f32, cursor_y as f32);
        }
    }
}

/// Fixed weapon HUD reader.
pub(crate) struct FixedWeaponHudReader {
    /// Report.
    report: (Option<WeaponHudStatus>, ArsenalAmmoWarning),
}

impl WeaponHudReader for FixedWeaponHudReader {
    fn read(&mut self) -> (Option<WeaponHudStatus>, ArsenalAmmoWarning) {
        self.report.clone()
    }
}

/// Configstring view over a mission host.
pub(crate) struct MissionHudStrings(Shared<dyn MissionHudHost>);

impl HudConfigStrings for MissionHudStrings {
    fn config_string(&self, index: usize) -> String {
        self.0.borrow().config_string(index)
    }
}

/// Load context adapter.
pub(crate) struct MissionHudLoadContext<'a> {
    /// HUD.
    hud: &'a MissionHud,
}

impl MenuLoadContext for MissionHudLoadContext<'_> {
    fn random_next_int(&mut self) -> i32 {
        self.hud.host.borrow().random().borrow_mut().rand()
    }

    fn resolve_root(&mut self, requested: &str) -> Option<MenuSource> {
        self.hud.source(requested)
    }

    fn resolve(&mut self, from_path: &str, requested: &str) -> Option<MenuSource> {
        self.hud
            .source(&posix_join(posix_dirname(from_path), requested))
            .or_else(|| self.hud.source(requested))
    }

    fn register_font(&mut self, reference: &FontReference) {
        self.current();
        self.hud.register_font(reference.path.as_deref(), reference.point_size);
        self.current();
    }

    fn register_picture(&mut self, path: Option<&str>) -> u32 {
        self.current();
        let picture = self.hud.register_picture(path);
        self.current();
        picture.map(|picture| picture.order).unwrap_or(0)
    }

    fn register_sound(&mut self, path: Option<&str>) -> u32 {
        self.current();
        let sound = self.hud.register_sound(path);
        self.current();
        self.hud.media.borrow().sound_bank.borrow().index_for_sound(&sound) as u32
    }

    fn register_model(&mut self, path: Option<&str>) -> u32 {
        self.current();
        let model = self.hud.register_model(path);
        self.current();
        self.hud
            .media
            .borrow()
            .resources
            .borrow()
            .model_handle(&model)
            .unwrap_or(0) as u32
    }

    fn publish_asset_font(&mut self, field: &str, reference: &FontReference) {
        self.current();
        if field != "smallFont" && field != "textFont" && field != "bigFont" {
            return;
        }
        let Some(font) = self.hud.font(reference) else {
            return;
        };
        match field {
            "smallFont" => self.hud.fonts_handle.borrow_mut().small = font,
            "textFont" => self.hud.fonts_handle.borrow_mut().normal = font,
            _ => self.hud.fonts_handle.borrow_mut().big = font,
        }
    }

    fn initial_assets(&self) -> Option<MenuGlobalAssets> {
        self.hud.asset_definitions.borrow().clone()
    }
}

impl MissionHudLoadContext<'_> {
    /// Require a current load generation.
    fn current(&self) {
        self.hud.open();
        if self.hud.load_generation.get() != self.hud.generation.get() {
            panic!("Mission HUD menu load belongs to a retired lifecycle");
        }
    }
}

/// Paint callback adapter.
pub(crate) struct MissionHudCallbacks<'a> {
    /// HUD.
    hud: &'a MissionHud,
}

impl MenuPaintCallbacks for MissionHudCallbacks<'_> {
    fn feeder_count(&mut self, feeder: i32) -> i32 {
        self.hud.feeder_count(feeder)
    }

    fn feeder_item(&mut self, feeder: i32, index: i32, column: i32) -> MenuFeederItem {
        self.hud.feeder_item(feeder, index, column)
    }

    fn feeder_select(&mut self, feeder: i32, index: i32) {
        self.hud.feeder_selection(feeder, index);
    }

    fn owner_visible(&mut self, flags: i32) -> bool {
        self.hud.owner_draw.visible(flags)
    }

    fn owner_width(&mut self, id: i32, scale: f32) -> f32 {
        self.hud.owner_draw.width(id, scale)
    }

    fn owner_value(&mut self, id: i32) -> f32 {
        self.hud.owner_draw.value(id)
    }

    fn owner_paint(&mut self, request: &mut OwnerDrawPaintRequest) {
        self.hud.owner_draw.paint(request);
    }

    fn close_cinematic(&mut self, handle: i32) {
        self.hud.host.borrow().cinematics().borrow_mut().stop_slot(handle);
    }

    fn team_color(&mut self) -> Vec4 {
        match self.hud.snapshot().persistant.get(PersistentIndex::PersTeam as usize) {
            x if x == Team::TeamRed as i32 => vec4(1.0, 0.0, 0.0, 0.25),
            x if x == Team::TeamBlue as i32 => vec4(0.0, 0.0, 1.0, 0.25),
            _ => vec4(0.0, 0.17, 0.0, 0.25),
        }
    }

    fn paint_model(&mut self, request: &UiModelPaintRequest) {
        self.hud.host.borrow().model_painter().paint(request);
    }

    fn cvar_value(&mut self, name: &str) -> f64 {
        f64::from(
            game_atof(
                &self
                    .hud
                    .host
                    .borrow()
                    .cvars()
                    .borrow()
                    .get(name)
                    .map(|snapshot| snapshot.value.chars().take(127).collect::<String>())
                    .unwrap_or_default(),
            )
            .unwrap_or_default(),
        )
    }
}

impl ConsoleHud for MissionHud {
    fn reset_strings(&mut self) {
        MissionHud::reset_strings(self);
    }
    fn reset_menus(&mut self) {
        MissionHud::reset_menus(self);
    }
    fn load_menus(&mut self, path: &str) {
        MissionHud::load_menus(self, path);
    }
    fn clear_scoreboard(&mut self) {
        MissionHud::clear_scoreboard(self);
    }
    fn menu_scoreboard(&self) -> Option<CapturedMenu> {
        MissionHud::menu_scoreboard(self)
    }
    fn scroll_feeder(&mut self, menu: &CapturedMenu, feeder: i32, down: bool) {
        MissionHud::scroll_feeder(self, menu, feeder, down);
    }
}

impl ConsoleOrders for MissionHud {
    fn select_next_player(&mut self) {
        MissionHud::select_next_player(self);
    }
    fn select_previous_player(&mut self) {
        MissionHud::select_previous_player(self);
    }
    fn other_team_has_flag(&self) -> bool {
        MissionHud::other_team_has_flag(self)
    }
    fn your_team_has_flag(&self) -> bool {
        MissionHud::your_team_has_flag(self)
    }
}

/// Captured menu handle (`UiCapturedMenu`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CapturedMenu {
    /// Identity.
    pub id: u64,
    /// Name.
    pub name: String,
}

/// Menu snapshot (`UiRuntimeSnapshot`, used surface).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MenuSnapshot {
    /// Open menus.
    pub open_menus: Vec<String>,
}

/// Menu frame (`UiRuntimeFrame` time surface; draw travels separately).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MenuFrame {
    /// Time (always 0; the source never assigns `cgDC.realTime`).
    pub time: i32,
    /// Frame time (always 0).
    pub frame_time: i32,
}

/// Menu script source (`ScriptSource`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuSource {
    /// Path.
    pub path: String,
    /// Text.
    pub text: String,
}

/// Font reference (`UiFontReference`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontReference {
    /// Path.
    pub path: Option<String>,
    /// Point size.
    pub point_size: i32,
}

/// Asset reference with a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetReference {
    /// Path.
    pub path: Option<String>,
}

/// Global menu assets (`UiGlobalAssets`, used surface).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MenuGlobalAssets {
    /// Gradient bar.
    pub gradient_bar: Option<AssetReference>,
}

/// Menu definitions (`UiMenuDefinitions`, used surface).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MenuDefinitions {
    /// Assets.
    pub assets: MenuGlobalAssets,
}

/// Menu reset scope (`"strings" | "menus"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuResetScope {
    /// Strings.
    Strings,
    /// Menus.
    Menus,
}

/// Menu cursor type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuCursorType {
    /// Arrow.
    Arrow,
    /// Other (sized).
    Other,
}

/// Feeder item (`UiRuntimeFeederItem`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuFeederItem {
    /// Text.
    pub text: String,
    /// Picture.
    pub picture: Option<Picture>,
}

/// Widget assets (`UiWidgetAssets`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiWidgetAssets {
    /// White shader.
    pub white_shader: Picture,
    /// Gradient bar.
    pub gradient_bar: Picture,
    /// Scroll bar.
    pub scroll_bar: Picture,
    /// Scroll arrow down.
    pub scroll_bar_arrow_down: Picture,
    /// Scroll arrow up.
    pub scroll_bar_arrow_up: Picture,
    /// Scroll arrow left.
    pub scroll_bar_arrow_left: Picture,
    /// Scroll arrow right.
    pub scroll_bar_arrow_right: Picture,
    /// Scroll thumb.
    pub scroll_bar_thumb: Picture,
    /// Slider bar.
    pub slider_bar: Picture,
    /// Slider thumb.
    pub slider_thumb: Picture,
}

/// Menu audio (`UiRuntimeAudio`).
pub trait HudMenuAudio {
    /// Play a local sound.
    fn play_local(&mut self, sound: MenuAudioSound);
    /// Start background audio.
    fn start_background(&mut self, path: Option<String>);
    /// Stop background audio.
    fn stop_background(&mut self);
}

/// Menu audio sound (`PcmSound | number | undefined`).
#[derive(Debug, Clone, PartialEq)]
pub enum MenuAudioSound {
    /// Missing.
    Missing,
    /// PCM.
    Pcm(Option<PcmSound>),
    /// Handle.
    Handle(i32),
}

/// Cinematic asset (`UiCinematicAsset`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CinematicAsset {
    /// Path.
    pub path: String,
}

/// Cinematic instance (`UiCinematicInstance`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CinematicInstance {
    /// Asset.
    pub asset: CinematicAsset,
    /// Handle index.
    pub handle: i32,
}

/// Cinematics (`UiRuntimeCinematics` + `EngineUiCinematics.owner`).
pub trait CinematicService {
    /// Play a cinematic.
    fn play(&mut self, asset: &CinematicAsset, rect: Rect2d) -> Option<CinematicInstance>;
    /// Run a cinematic.
    fn run(&mut self, handle: i32, time: i32);
    /// Draw a cinematic.
    fn draw(&mut self, handle: i32, rect: Rect2d, draw: &Draw2D);
    /// Stop a cinematic.
    fn stop(&mut self, handle: i32);
    /// Prepare a cinematic.
    fn prepare(&mut self, path: &str) -> CinematicAsset;
    /// Stop a slot.
    fn stop_slot(&mut self, index: i32);
}

/// Font registry (`UiAssetRegistry.registerFont`).
pub trait FontRegistry {
    /// Register a font.
    fn register_font(&mut self, path: Option<&str>, point_size: i32) -> Option<RegisteredFont>;
}

/// Command buffer (`CommandBuffer.append`).
pub trait HudCommandBuffer {
    /// Append text.
    fn append(&mut self, text: &str);
}

/// Menu paint callbacks (feeder + owner-draw + team color + model paint).
pub trait MenuPaintCallbacks {
    /// Feeder count.
    fn feeder_count(&mut self, feeder: i32) -> i32;
    /// Feeder item.
    fn feeder_item(&mut self, feeder: i32, index: i32, column: i32) -> MenuFeederItem;
    /// Feeder selection.
    fn feeder_select(&mut self, feeder: i32, index: i32);
    /// Owner-draw visibility.
    fn owner_visible(&mut self, flags: i32) -> bool;
    /// Owner-draw width.
    fn owner_width(&mut self, id: i32, scale: f32) -> f32;
    /// Owner-draw value.
    fn owner_value(&mut self, id: i32) -> f32;
    /// Owner-draw paint.
    fn owner_paint(&mut self, request: &mut OwnerDrawPaintRequest);
    /// Close a cinematic.
    fn close_cinematic(&mut self, handle: i32);
    /// Team color.
    fn team_color(&mut self) -> Vec4;
    /// Paint a model.
    fn paint_model(&mut self, request: &UiModelPaintRequest);
    /// Cvar value.
    fn cvar_value(&mut self, name: &str) -> f64;
}

/// Menu runtime (`UiRuntime`, used surface).
pub trait MenuRuntime {
    /// Snapshot.
    fn snapshot(&self) -> MenuSnapshot;
    /// Reload definitions.
    fn reload_definitions(&mut self, definitions: &MenuDefinitions);
    /// Reset definitions.
    fn reset_definitions(&mut self, scope: MenuResetScope);
    /// Run a frame.
    fn frame(&mut self, frame: &MenuFrame, draw: &mut Draw2D, callbacks: &mut dyn MenuPaintCallbacks);
    /// Paint a captured menu.
    fn paint_captured(
        &mut self,
        menu: &CapturedMenu,
        frame: &MenuFrame,
        force: bool,
        draw: &mut Draw2D,
        callbacks: &mut dyn MenuPaintCallbacks,
    );
    /// Clear captured forced state.
    fn clear_captured_forced(&mut self, menu: &CapturedMenu);
    /// Menu handle by name.
    fn menu_handle(&self, name: &str) -> Option<CapturedMenu>;
    /// Set a captured feeder selection.
    fn set_captured_feeder_selection(&mut self, menu: &CapturedMenu, feeder: i32, index: i32);
    /// Scroll a captured feeder.
    fn scroll_captured_feeder(&mut self, menu: &CapturedMenu, feeder: i32, down: bool);
    /// Close by name.
    fn close(&mut self, name: &str);
    /// Show by name.
    fn show(&mut self, name: &str);
    /// Set the display cursor.
    fn set_display_cursor(&mut self, x: f32, y: f32);
    /// Cursor type at a point.
    fn cursor_type(&self, x: f32, y: f32) -> MenuCursorType;
    /// Move a captured menu.
    fn move_captured_menu(&mut self, menu: &CapturedMenu, dx: f32, dy: f32);
    /// Pointer move.
    fn pointer_move(&mut self, x: f32, y: f32);
    /// Handle a key.
    fn handle_key(&mut self, key: i32, down: bool, x: f32, y: f32);
    /// Capture the menu at a point.
    fn capture_menu(&mut self, x: f32, y: f32) -> Option<CapturedMenu>;
    /// Retire the runtime.
    fn retire(&mut self);
}

/// Menu load context (`loadMenuDefinitions` callbacks).
pub trait MenuLoadContext {
    /// Random int.
    fn random_next_int(&mut self) -> i32;
    /// Resolve the root source.
    fn resolve_root(&mut self, requested: &str) -> Option<MenuSource>;
    /// Resolve a nested source.
    fn resolve(&mut self, from_path: &str, requested: &str) -> Option<MenuSource>;
    /// Register a font.
    fn register_font(&mut self, reference: &FontReference);
    /// Register a picture, returning its handle.
    fn register_picture(&mut self, path: Option<&str>) -> u32;
    /// Register a sound, returning its handle.
    fn register_sound(&mut self, path: Option<&str>) -> u32;
    /// Register a model, returning its handle.
    fn register_model(&mut self, path: Option<&str>) -> u32;
    /// Publish an asset font field.
    fn publish_asset_font(&mut self, field: &str, reference: &FontReference);
    /// Initial assets.
    fn initial_assets(&self) -> Option<MenuGlobalAssets>;
}

/// Menu runtime seed for creation.
#[derive(Clone)]
pub struct MenuRuntimeSeed {
    /// Definitions.
    pub definitions: MenuDefinitions,
    /// Cvars.
    pub cvars: Shared<CvarRegistry>,
    /// Widget assets.
    pub widget_assets: UiWidgetAssets,
    /// Zero picture.
    pub zero_picture: Picture,
    /// Fonts.
    pub fonts: FontSet,
}
