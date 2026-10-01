//! Per-seat game UI: HUD, pause menu, captions, wheel, and settings.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/ui.ts`
//! (`ApplicationSeatUi`). Menu registration, HUD drawing, captions, the
//! weapon wheel, and settings reuse `qa_client`; sound captions, source HUD,
//! weapon assets, Team Arena results, and rerelease presentation reuse the
//! lane modules. Sync port: the donor's async preparation, localization, and
//! refresh become sync calls. Unported siblings arrive through boxed traits:
//! simulation access (`./simulation/types.ts`), game prompt
//! (`./game-prompt.ts`), player death (`./player-death.ts`), Q2 match UI
//! (`./q2-match-ui.ts`), native Q2 HUD (`./q2-native-hud.ts`), the authored
//! wheel (`./q1-wheel.ts`), base-arena menus (`./base-arena-menu.ts`), and
//! the audio/input/asset surfaces (`./audio.ts`, `./input.ts`, `./assets.ts`).
//! Pause-menu navigation queues intents drained after input (same reentrancy
//! rule as [`super::startup_menu`]). `draw` and `caption_commands` render
//! through the caller's [`UiRenderServices`](qa_client::ui::common::draw::UiRenderServices).
//! The donor's `ApplicationInputUi` conformance is structural here: the
//! methods exist, and the `implements` clause returns with `./input.ts`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use super::remote_seat_source::RemoteActorId;
use qa_client::input::KeyCode;
use qa_client::text::atlas::TextFontSelection;
use qa_client::text::captions::ActiveCaption;
use qa_client::text::captions::CaptionCue;
use qa_client::text::captions::CaptionKind;
use qa_client::text::draw2d::Draw2D;
use qa_client::text::draw2d::ImagePicture;
use qa_client::text::draw2d::PictureAsset;
use qa_client::text::draw2d::Rect;
use qa_client::text::draw2d::RectOrigin;
use qa_client::text::layout::layout_text;
use qa_client::text::layout::ColorCodes;
use qa_client::text::layout::TextAlign as LayoutAlign;
use qa_client::text::layout::TextLayoutOptions;
use qa_client::text::ui_world::UiTextRenderer;
use qa_client::ui::common::assets::NativeUiArt;
use qa_client::ui::common::captions::caption_commands;
use qa_client::ui::common::controller::NativeUiController;
use qa_client::ui::common::controller::NativeUiOptions;
use qa_client::ui::common::controller::UiSound;
use qa_client::ui::common::draw::render_ui_commands;
use qa_client::ui::common::draw::UiEmitCommand;
use qa_client::ui::common::draw::UiMaterialDraw;
use qa_client::ui::common::draw::UiRenderServices;
use qa_client::ui::common::layout::menu_row;
use qa_client::ui::common::layout::MenuRowOptions;
use qa_client::ui::common::menu_theme::menu_panel;
use qa_client::ui::common::menu_theme::menu_skin;
use qa_client::ui::common::menu_theme::menu_title_font;
use qa_client::ui::common::skin::hud_skin_font;
use qa_client::ui::hud::draw_common_hud;
use qa_client::ui::hud::empty_hud_data;
use qa_client::ui::hud::hud_vital_occupied_rects;
use qa_client::ui::hud::powerups::PowerupTimerView;
use qa_client::ui::hud::q2_native::AmmunitionView;
use qa_client::ui::hud::q2_native::ArsenalIcon;
use qa_client::ui::hud::q2_native::NativeInventoryItem;
use qa_client::ui::hud::q2_native::NativeInventoryReadout;
use qa_client::ui::hud::q2_native::NativeQ2HudArsenal;
use qa_client::ui::hud::q2_native::NativeQ2HudFrame;
use qa_client::ui::hud::q2_native::SelectedItemView;
use qa_client::ui::hud::q2_rerelease_layout::NativeQ2HudEnvironment;
use qa_client::ui::hud::weapon::CommonWeaponHud;
use qa_client::ui::hud::wheel::SeatWeaponWheel;
use qa_client::ui::hud::wheel::WeaponWheelServices;
use qa_client::ui::hud::wheel::WheelItem;
use qa_client::ui::hud::wheel::WheelMode;
use qa_client::ui::hud::wheel::DEFAULT_WHEEL_OPTIONS;
use qa_client::ui::hud::CommonHudData;
use qa_client::ui::hud::CommonHudDrawOptions;
use qa_client::ui::hud::HudCrosshair;
use qa_client::ui::hud::HudInventoryItem;
use qa_client::ui::hud::HudPrompt;
use qa_client::ui::hud::HudValue;
use qa_client::ui::hud::SeatHudMessages;
use qa_client::ui::library::inventory::register_inventory_menu;
use qa_client::ui::library::inventory::CommandFn;
use qa_client::ui::library::match_menu::register_match_menu;
use qa_client::ui::saves::menu::register_saved_game_menus;
use qa_client::ui::saves::menu::SavedGameMenuService;
use qa_client::ui::settings::action_catalog::shared_binding_actions;
use qa_client::ui::settings::action_catalog::BindableItem;
use qa_client::ui::settings::action_catalog::BindingCapabilities;
use qa_client::ui::settings::bind_audio_geometry_settings;
use qa_client::ui::settings::bind_audio_settings;
use qa_client::ui::settings::bind_input_settings;
use qa_client::ui::settings::bind_music_playlist_settings;
use qa_client::ui::settings::bindings::register_binding_menus;
use qa_client::ui::settings::bindings::BindingResetHandle;
use qa_client::ui::settings::bindings::BindingSource;
use qa_client::ui::settings::bindings::BindingStore;
use qa_client::ui::settings::console::bind_console_settings;
use qa_client::ui::settings::gameplay::bind_gameplay_settings;
use qa_client::ui::settings::gameplay::reset_gameplay_settings;
use qa_client::ui::settings::gameplay::GameplaySettingsSource;
use qa_client::ui::settings::gyro::register_gyro_settings_menu;
use qa_client::ui::settings::gyro::GyroSettingsUi;
use qa_client::ui::settings::images::bind_image_settings;
use qa_client::ui::settings::images::bind_model_settings;
use qa_client::ui::settings::input_routing::bind_input_routing_settings;
use qa_client::ui::settings::input_routing::DeviceView;
use qa_client::ui::settings::input_routing::ReRouter;
use qa_client::ui::settings::language::read_seat_language;
use qa_client::ui::settings::llm::LlmSettingsUi;
use qa_client::ui::settings::ranking_account::register_ranking_account_menu;
use qa_client::ui::settings::ranking_account::RankingAccountSource;
use qa_client::ui::settings::register_settings_menus;
use qa_client::ui::settings::server::register_server_settings_menu;
use qa_client::ui::settings::server::HostServerSettingsUi;
use qa_client::ui::settings::services::bind_native_video_settings;
use qa_client::ui::settings::services::bind_renderer_settings;
use qa_client::ui::settings::services::RendererBackend;
use qa_client::ui::settings::services::RendererOptions;
use qa_client::ui::settings::services::WindowView;
use qa_client::ui::settings::services::WorkerToggle;
use qa_client::ui::settings::AudioOutputSettings;
use qa_client::ui::settings::AudioSettings;
use qa_client::ui::settings::InputTuningHost;
use qa_client::ui::settings::SettingBinding;
use qa_client::ui::settings::SettingBindingKind;
use qa_client::ui::settings::SettingCategory;
use qa_client::ui::settings::SettingCvars;
use qa_client::ui::settings::SettingReset;
use qa_client::ui::settings::SettingsValueService;
use qa_client::ui::settings::VibrationService;
use qa_client::ui::types::ArsenalAmmoWarning;
use qa_client::ui::types::CommandDialect;
use qa_client::ui::types::ContentId as UiContentId;
use qa_client::ui::types::ItemId;
use qa_client::ui::types::ProviderRef;
use qa_client::ui::types::ResourceId;
use qa_client::ui::types::SeatInputEvent;
use qa_client::ui::types::SeatInputEventKind;
use qa_client::ui::types::SeatInputFocus;
use qa_client::ui::types::SeatUiController;
use qa_client::ui::types::Typeface;
use qa_client::ui::types::UiControl;
use qa_client::ui::types::UiControlId;
use qa_client::ui::types::UiControlKind;
use qa_client::ui::types::UiDrawCommand;
use qa_client::ui::types::UiDrawContext;
use qa_client::ui::types::UiMenu;
use qa_client::ui::types::UiMenuId;
use qa_client::ui::types::WeaponAmmo;
use qa_client::ui::types::WeaponHudStatus;
use qa_client::view::SceneCamera;
use qa_client::ClientError;
use qa_content::contract::ContentId;
use qa_content::contract::ProviderReference;
use qa_core::cmd::tokenize_command;
use qa_core::cmd::Dialect as CmdDialect;
use qa_core::cmd::TextMode;
use qa_core::cvar::CvarRegistry;
use qa_core::identity::ActorId;
use qa_core::identity::ClientId;
use qa_core::identity::SeatId;
use qa_core::math::Vec4;
use qa_core::time::SourceTime;
use thiserror::Error;

use super::media::rerelease_presentation::ApplicationRereleasePresentation;
use super::media::rerelease_presentation::RereleasePresentationProvider;
use super::media::rerelease_presentation::RereleasePresentationSeat;
use super::seat_hud_state::SeatHudEvent;
use super::seat_hud_state::SeatHudIcons;
use super::seat_hud_state::SeatHudSourceEvent;
use super::seat_hud_state::SeatSourceHud;
use super::sound_captions::SeatSoundCaptions;
use super::sound_captions::SoundCaptionPreferences;
use super::sound_captions::SoundCaptionReference;
use super::sound_captions::SoundCaptionVoiceStart;
use super::sound_captions::SoundMediaCaptions;
use super::sound_captions::VoiceClockSample;
use super::team_arena_results::TeamArenaResultService;
use super::team_arena_results::TeamArenaResults;
use super::weapon_hud::ApplicationWeaponHudAssets;
use super::weapon_hud::HudIcon;
use super::weapon_hud::HudIconResolver;
use super::weapon_hud::HudStatusRef;
use super::weapon_hud::WeaponHudBackend;
use super::weapon_hud::WeaponHudError;

/// Game-prompt menu id (donor `gamePromptMenu` in `./game-prompt.ts`).
const GAME_PROMPT_MENU: &str = "menu:application:prompt";
/// Player-death menu id (donor `playerDeathMenu` in `./player-death.ts`).
const PLAYER_DEATH_MENU: &str = "menu:application:death";
/// Pause menu id (donor `menu:application:game`).
const PAUSE_MENU: &str = "menu:application:game";

/// Seat UI failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum UiSeatError {
    /// Menu construction failed.
    #[error("Seat UI failed: {0}")]
    Failed(String),
}

impl From<ClientError> for UiSeatError {
    fn from(error: ClientError) -> Self {
        Self::Failed(error.to_string())
    }
}

impl From<WeaponHudError> for UiSeatError {
    fn from(error: WeaponHudError) -> Self {
        Self::Failed(error.to_string())
    }
}

/// Native HUD localizer factory (donor `nativeHudLocalizer`).
pub type NativeHudLocalizer = Rc<dyn Fn(ContentId) -> Box<dyn Fn(String, Vec<String>) -> String>>;
/// Source text localizer (donor `localize`).
pub type SeatTextLocalizer = Rc<dyn Fn(ContentId, String, Vec<String>) -> String>;
/// Weapon picture resolver (load key to GPU asset).
pub type WeaponPictureResolver = Rc<dyn Fn(&str) -> Option<PictureAsset>>;
/// Draw-command picture resolver.
pub type DrawPictureResolver = Rc<dyn Fn(&ResourceId) -> Option<PictureAsset>>;
/// Boxed source-HUD localizer.
type BoxedSourceLocalizer = Box<dyn FnMut(&UiContentId, &str, &[String]) -> String>;

/// Menu typography (donor `MenuTypography` in `./menu-font.ts`).
#[derive(Debug, Clone)]
pub struct MenuTypography {
    /// Body font.
    pub body: TextFontSelection,
    /// Title font.
    pub title: TextFontSelection,
}

/// Seat armor readout.
#[derive(Debug, Clone, PartialEq)]
pub struct SeatArmor {
    /// Regular armor kind (`none` when absent).
    pub regular_kind: String,
    /// Regular armor points.
    pub regular_points: i32,
    /// Powered armor kind (`none` when absent).
    pub powered_kind: String,
    /// Powered armor cells.
    pub powered_cells: i32,
}

/// Seat weapon status (donor `PlayerUi["weaponStatus"]`).
#[derive(Debug, Clone, PartialEq)]
pub struct SeatWeaponStatus {
    /// Source provider id (`namespace:name`).
    pub source_provider: String,
    /// Source content id.
    pub source_content: String,
    /// Weapon item id.
    pub item: String,
    /// Display label.
    pub label: String,
    /// Ammo state.
    pub ammo: WeaponAmmo,
}

/// Seat HUD inventory item (donor `PlayerUi["items"]` row).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatHudItem {
    /// Item kind (`weapon`/`powerup`/other).
    pub kind: String,
    /// Item id.
    pub id: String,
    /// Source ordering.
    pub source_ordinal: i32,
    /// Display label.
    pub label: String,
    /// Whether the seat owns this item.
    pub owned: bool,
}

/// Native inventory presentation (donor `nativeInventory.presentation`).
#[derive(Debug, Clone, PartialEq)]
pub struct SeatNativePresentation {
    /// Presentation kind (`item`/`weapon`/other).
    pub kind: String,
    /// Provider reference.
    pub source: ProviderReference,
    /// Weapon item for icon lookup.
    pub weapon: String,
    /// Item icon, when the kind is `item`.
    pub icon: Option<HudIcon>,
}

/// Native inventory readout (donor `PlayerUi["nativeInventory"]`).
#[derive(Debug, Clone, PartialEq)]
pub struct SeatNativeInventory {
    /// Selected item (`None` when nothing is selected).
    pub selected: Option<String>,
    /// Presentation, if any.
    pub presentation: Option<SeatNativePresentation>,
    /// Inventory rows.
    pub items: Vec<SeatNativeInventoryItem>,
}

/// One native inventory row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatNativeInventoryItem {
    /// Item id.
    pub item: String,
    /// Display label.
    pub label: String,
    /// Stack count.
    pub count: i32,
}

/// Seat player UI snapshot (donor `PlayerUi` subset read by this module).
#[derive(Debug, Clone, PartialEq)]
pub struct SeatPlayerUi {
    /// Health.
    pub health: i32,
    /// Armor.
    pub armor: SeatArmor,
    /// Active powerups.
    pub powerups: Vec<PowerupTimerView>,
    /// Inventory items.
    pub items: Vec<SeatHudItem>,
    /// Active weapon id, if any.
    pub active_weapon: Option<String>,
    /// Weapon status, if any.
    pub weapon_status: Option<SeatWeaponStatus>,
    /// Arsenal-level ammo warning.
    pub arsenal_warning: ArsenalAmmoWarning,
    /// Whether the selected arsenal replaces the native status.
    pub selected_arsenal: bool,
    /// Native inventory, if any.
    pub native_inventory: Option<SeatNativeInventory>,
    /// Ammo count, if any.
    pub ammo_count: Option<i32>,
}

/// CTF/LMCTF composition event (donor `Q2CompositionEvent` ctf/lmctf).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatCtfEvent {
    /// Rules (`ctf`/`lmctf`).
    pub rules: String,
    /// Red score.
    pub red_score: i32,
    /// Blue score.
    pub blue_score: i32,
    /// Red flag carrier, if any.
    pub red_carrier: Option<ActorId>,
    /// Blue flag carrier, if any.
    pub blue_carrier: Option<ActorId>,
}

/// Seat presentation payload (donor `SimulationPresentationEvent` subset).
#[derive(Debug, Clone, PartialEq)]
pub enum SeatPresentationPayload {
    /// Source-HUD event, forwarded to [`SeatSourceHud`].
    Hud(SeatHudEvent),
    /// Q2 player inventory visibility.
    Q2PlayerInventory {
        /// Player actor.
        actor: ActorId,
        /// Whether the inventory opened.
        visible: bool,
    },
    /// Q2 player userinfo name.
    Q2PlayerUserinfo {
        /// Player actor.
        actor: ActorId,
        /// Player name.
        name: String,
    },
    /// Q2 player chat print.
    Q2PlayerPrint {
        /// Target actor (`None` broadcasts).
        target: Option<ActorId>,
        /// Text.
        text: String,
    },
    /// Q2 CTF/LMCTF composition event.
    Q2Composition {
        /// Event.
        event: SeatCtfEvent,
    },
    /// Q1 player message.
    Q1Message {
        /// Player actor.
        player: ActorId,
        /// Center print instead of notify.
        center: bool,
        /// Text.
        text: String,
    },
    /// Q2 center print.
    Q2Centerprint {
        /// Player actor.
        actor: ActorId,
        /// Text.
        text: String,
        /// Duration in seconds (`None` keeps the default).
        duration_seconds: Option<f64>,
        /// Instant print (`None` keeps the default).
        instant: Option<bool>,
    },
    /// Q2 chat print.
    Q2Print {
        /// Player actor (`None` broadcasts).
        actor: Option<ActorId>,
        /// Text.
        text: String,
    },
    /// Q3 server command.
    Q3ServerCommand {
        /// Target client (`-1` broadcasts).
        client: i32,
        /// Command text.
        text: String,
    },
}

/// One seat presentation event.
#[derive(Debug, Clone, PartialEq)]
pub struct SeatPresentationEvent {
    /// Source content.
    pub content: ContentId,
    /// Source time in seconds.
    pub seconds: f64,
    /// Payload.
    pub event: SeatPresentationPayload,
}

/// Simulation access (donor `Pick<SimulationPresentationAccess, "playerUi">`).
pub trait SeatSimulationUi {
    /// Player UI snapshot for an actor.
    fn player_ui(&self, actor: &ActorId) -> SeatPlayerUi;
}

/// Game prompt (donor `SeatGamePrompt` in `./game-prompt.ts`).
pub trait SeatPromptUi {
    /// Prepare the prompt menu.
    fn prepare(&mut self, focus: Rc<dyn Fn() -> SeatInputFocus>);
    /// Clear the pending prompt.
    fn clear(&mut self);
    /// Handle one input event.
    fn input(&mut self, event: &SeatInputEvent) -> bool;
    /// Receive presentation events.
    fn receive(&mut self, events: &[SeatPresentationEvent]);
    /// Unregister the prompt menu.
    fn close(&mut self);
}

/// Player death screen (donor `SeatPlayerDeath` in `./player-death.ts`).
pub trait SeatDeathUi {
    /// Observe current health.
    fn observe(&mut self, health: i32);
    /// Handle one input event.
    fn input(&mut self, event: &SeatInputEvent) -> bool;
    /// Whether the death screen is active.
    fn active(&self) -> bool;
    /// Unregister the death menu.
    fn close(&mut self);
}

/// Q2 match UI (donor `Q2MatchUi` in `./q2-match-ui.ts`).
pub trait SeatMatchUi {
    /// Record a player name.
    fn match_name(&mut self, actor: &ActorId, name: &str);
    /// Receive a CTF/LMCTF event.
    fn receive(&mut self, event: &SeatCtfEvent);
    /// Current action prompts.
    fn prompts(&self) -> Vec<HudPrompt>;
    /// Unregister match menus.
    fn close(&mut self);
}

/// Native Q2 HUD mode (donor `"layout-overlay" | "replace-status"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2HudMode {
    /// Draw the layout only.
    LayoutOverlay,
    /// Draw statusbar, layout, and inventory.
    ReplaceStatus,
}

/// Native Q2 HUD renderer (donor `ApplicationQ2NativeHud` in
/// `./q2-native-hud.ts`). Sync port: preparation has no `assertCurrent`
/// guards.
pub trait SeatQ2NativeHud {
    /// Prepare frame operations.
    #[allow(clippy::too_many_arguments)]
    fn prepare(
        &mut self,
        content: &ContentId,
        frame: &NativeQ2HudFrame,
        context: &UiDrawContext,
        scale: f32,
        mode: Option<Q2HudMode>,
        arsenal: Option<&NativeQ2HudArsenal>,
        environment: Option<&NativeQ2HudEnvironment>,
    );
    /// Draw commands for a frame.
    fn commands(
        &self,
        frame: &NativeQ2HudFrame,
        context: &UiDrawContext,
        scale: f32,
        binding: Option<&dyn Fn(&str) -> String>,
        mode: Option<Q2HudMode>,
        arsenal: Option<&NativeQ2HudArsenal>,
    ) -> Vec<UiDrawCommand>;
    /// Resolve a prepared picture.
    fn picture(&self, resource: &ResourceId) -> Option<qa_client::text::draw2d::PictureAsset>;
    /// Release prepared state.
    fn clear(&mut self);
}

/// Authored Q1 wheel (donor `ApplicationQ1Wheel` in `./q1-wheel.ts`).
pub trait SeatAuthoredWheel {
    /// Prepare wheel icons.
    fn prepare(&mut self, resolver: &mut dyn HudIconResolver, status: Option<&HudStatusRef>, player: &SeatPlayerUi);
    /// Authored wheel items, if the wheel overrides the player arsenal.
    fn items(&self, player: &SeatPlayerUi) -> Option<Vec<SeatHudItem>>;
    /// Q1 slot switch selection, if any.
    fn switch_weapon(&self, player: &SeatPlayerUi, first: i32, second: i32) -> Option<String>;
}

/// Base-arena menus (donor `BaseArenaMenus` in `./base-arena-menu.ts`).
pub trait SeatBaseArenaUi {
    /// Refresh arena progress menus.
    fn update(&mut self, controller: &mut NativeUiController);
    /// Unregister arena menus.
    fn close(&mut self, controller: &mut NativeUiController);
}

/// Team Arena results screen (donor `TeamArenaResults`).
pub trait SeatTeamArenaUi {
    /// Whether the results screen has activated.
    fn active(&self) -> bool;
    /// Show the results menu once a result is available.
    fn update(&mut self, controller: &mut NativeUiController) -> Result<(), String>;
    /// Unregister the results menu.
    fn close(&self, controller: &mut NativeUiController);
}

impl<S: TeamArenaResultService + 'static> SeatTeamArenaUi for TeamArenaResults<S> {
    fn active(&self) -> bool {
        self.active()
    }

    fn update(&mut self, controller: &mut NativeUiController) -> Result<(), String> {
        TeamArenaResults::update(self, controller).map_err(|error| error.to_string())
    }

    fn close(&self, controller: &mut NativeUiController) {
        TeamArenaResults::close(self, controller);
    }
}

/// Seat audio surface (donor `ApplicationAudio` in `./audio.ts`).
pub trait SeatHudAudio {
    /// Play a UI sound for an owner seat.
    fn ui_sound(&self, sound: UiSound, owner: &SeatId);
    /// Effects volume.
    fn effects_volume(&self) -> f32;
    /// Set effects volume.
    fn set_effects_volume(&self, value: f32);
    /// Music volume.
    fn music_volume(&self) -> f32;
    /// Set music volume.
    fn set_music_volume(&self, value: f32);
    /// Output format.
    fn output_format(&self) -> qa_client::ui::settings::AudioOutputFormat;
    /// Select an output format.
    fn select_output_format(&self, format: &qa_client::ui::settings::AudioOutputFormat);
    /// Selected output device, if any.
    fn selected_output(&self) -> Option<String>;
    /// Output device names.
    fn output_device_names(&self) -> Vec<String>;
    /// Select an output device.
    fn select_output(&self, name: Option<&str>);
}

struct AudioSettingsAdapter {
    audio: Rc<dyn SeatHudAudio>,
}

impl SettingsValueService<AudioSettings> for AudioSettingsAdapter {
    fn read(&self) -> AudioSettings {
        AudioSettings {
            effects_volume: self.audio.effects_volume(),
            music_volume: self.audio.music_volume(),
        }
    }

    fn write(&self, update: &dyn Fn(&mut AudioSettings)) {
        let mut values = self.read();
        update(&mut values);
        self.audio.set_effects_volume(values.effects_volume);
        self.audio.set_music_volume(values.music_volume);
    }
}

struct AudioOutputAdapter {
    audio: Rc<dyn SeatHudAudio>,
    report: Rc<dyn Fn(String)>,
}

impl AudioOutputSettings for AudioOutputAdapter {
    fn selected(&self) -> Option<String> {
        self.audio.selected_output()
    }

    fn devices(&self) -> Vec<String> {
        self.audio.output_device_names()
    }

    fn select(&self, name: Option<&str>) {
        self.audio.select_output(name);
    }

    fn report(&self, message: &str) {
        (self.report)(message.to_string());
    }

    fn format(&self) -> Option<Rc<dyn qa_client::ui::settings::AudioOutputFormatControl>> {
        Some(Rc::new(AudioFormatAdapter {
            audio: Rc::clone(&self.audio),
        }) as Rc<dyn qa_client::ui::settings::AudioOutputFormatControl>)
    }
}

struct AudioFormatAdapter {
    audio: Rc<dyn SeatHudAudio>,
}

impl qa_client::ui::settings::AudioOutputFormatControl for AudioFormatAdapter {
    fn read(&self) -> qa_client::ui::settings::AudioOutputFormat {
        self.audio.output_format()
    }

    fn select(&self, format: &qa_client::ui::settings::AudioOutputFormat) {
        self.audio.select_output_format(format);
    }
}

/// Seat haptics surface (donor `LocalInput["haptics"]`).
pub trait SeatHaptics {
    /// Whether vibration is enabled.
    fn enabled(&self) -> bool;
    /// Set vibration enabled.
    fn set_enabled(&self, enabled: bool);
    /// Vibration strength.
    fn strength(&self) -> f32;
    /// Set vibration strength.
    fn set_strength(&self, strength: f32);
    /// Set rumble active.
    fn set_active(&self, active: bool);
}

/// Seat console surface (donor `LocalInput["console"]`).
pub trait SeatConsole {
    /// Print console text.
    fn print(&self, text: &str);
    /// Toggle the console.
    fn toggle(&self);
}

/// Seat focus and bindings surface (donor `LocalInput["input"]`).
pub trait SeatFocus {
    /// Current bindings.
    fn bindings(&self) -> Vec<qa_client::input::InputBinding>;
    /// Current focus.
    fn focus(&self) -> SeatInputFocus;
    /// Whether game input is focused.
    fn focused(&self) -> bool;
    /// Set focus.
    fn set_focus(&self, focus: SeatInputFocus, time_ms: i64);
    /// Set the impulse value.
    fn set_impulse(&self, value: i32);
}

/// Seat asset access for preparation (donor `ApplicationAssets`).
pub trait SeatPrepareAssets<Catalog> {
    /// Weapon HUD backend plus icon resolver (one call so the caller can
    /// borrow both without splitting the asset owner).
    fn weapon_services(&mut self) -> (&mut dyn WeaponHudBackend, &mut dyn HudIconResolver);
    /// Caption catalog loader.
    fn caption_loader(&mut self) -> &mut dyn FnMut(&SoundCaptionReference, &str) -> Catalog;
    /// Rerelease provider.
    fn rerelease_provider(&self) -> Rc<dyn RereleasePresentationProvider>;
    /// Catalog product expectation.
    fn catalog_product(&self, id: &str) -> Result<qa_content::catalog::ProductExpectation, String>;
}

/// Seat input sample (donor `SeatInputSample`).
#[derive(Debug, Clone, PartialEq)]
pub struct SeatInputSample {
    /// Sample time in milliseconds.
    pub now_milliseconds: i64,
    /// Sampled buttons.
    pub buttons: Vec<SeatSampleButton>,
}

/// One sampled button.
#[derive(Debug, Clone, PartialEq)]
pub struct SeatSampleButton {
    /// Button action.
    pub action: String,
    /// Whether the button is active.
    pub active: bool,
    /// Whether the button was pressed this sample.
    pub pressed: bool,
    /// Analog fraction.
    pub fraction: f32,
}

/// Queued pause-menu navigation.
#[derive(Debug, Clone, PartialEq, Eq)]
enum SeatNavIntent {
    /// Open a menu.
    Open(UiMenuId),
    /// Close all menus.
    CloseAll,
}

/// Seat UI options (donor constructor parameters).
pub struct UiSeatOptions<Catalog> {
    /// Player actor.
    pub actor: ActorId,
    /// Owning seat.
    pub seat: SeatId,
    /// Seat client.
    pub client: ClientId,
    /// Seat index.
    pub seat_index: u32,
    /// Menu art.
    pub art: NativeUiArt,
    /// HUD font.
    pub font: TextFontSelection,
    /// Menu typography.
    pub typography: MenuTypography,
    /// Simulation access.
    pub simulation: Rc<dyn SeatSimulationUi>,
    /// Console command sink.
    pub command: Rc<dyn Fn(String, Vec<String>)>,
    /// Quit callback.
    pub quit: Rc<dyn Fn()>,
    /// Audio surface.
    pub audio: Rc<dyn SeatHudAudio>,
    /// Focus surface.
    pub focus: Rc<dyn SeatFocus>,
    /// Haptics surface.
    pub haptics: Rc<dyn SeatHaptics>,
    /// Console surface.
    pub console: Rc<dyn SeatConsole>,
    /// Builder dialect.
    pub dialect: CommandDialect,
    /// Current time in milliseconds.
    pub now: Rc<dyn Fn() -> i64>,
    /// Shared settings, if any.
    pub shared_settings: Option<Rc<dyn SettingCvars>>,
    /// Input router.
    pub router: Rc<RefCell<dyn ReRouter>>,
    /// Router devices.
    pub router_devices: Rc<dyn Fn() -> Vec<DeviceView>>,
    /// Router capture refresh.
    pub update_capture: Rc<dyn Fn()>,
    /// Binding store.
    pub binding_store: Rc<RefCell<dyn BindingStore>>,
    /// Binding capabilities.
    pub binding_capabilities: Rc<dyn Fn() -> BindingCapabilities>,
    /// Whether bindings can reset for a seat.
    pub can_reset_bindings: Rc<dyn Fn(SeatId) -> bool>,
    /// Reset bindings for a seat.
    pub reset_bindings: Rc<dyn Fn(SeatId)>,
    /// Native window.
    pub window: Rc<RefCell<dyn WindowView>>,
    /// Live renderer backend.
    pub renderer_backend: Rc<dyn Fn() -> RendererBackend>,
    /// Renderer worker toggle, if exposed.
    pub renderer_worker: Option<WorkerToggle>,
    /// Queue a renderer backend change.
    pub apply_renderer: Rc<dyn Fn(RendererBackend)>,
    /// Console command buffer.
    pub commands: Rc<RefCell<qa_core::cmd_buffer::CommandBuffer>>,
    /// Command context for renderer changes.
    pub command_context: Rc<dyn Fn() -> qa_core::cmd_buffer::CommandContext>,
    /// Gyro settings.
    pub gyro: GyroSettingsUi,
    /// Input-device bindings.
    pub device_bindings: Vec<SettingBinding>,
    /// Input tuning host.
    pub tuning_host: Rc<RefCell<dyn InputTuningHost>>,
    /// Vibration rows enabled (donor always binds haptics; `false` skips).
    pub vibration: bool,
    /// Local player count.
    pub local_count: Rc<dyn Fn() -> usize>,
    /// Local player capacity.
    pub local_capacity: usize,
    /// Whether a seat's local player can be removed.
    pub can_remove_local: Rc<dyn Fn(SeatId) -> bool>,
    /// Attach-UI disposer factory (donor `input.attachUi`).
    pub attach: Rc<dyn Fn(SeatId) -> Box<dyn FnOnce()>>,
    /// HUD cvar registry.
    pub hud_cvars: Rc<RefCell<CvarRegistry>>,
    /// HUD caption clock.
    pub caption_clock: Rc<dyn Fn() -> VoiceClockSample>,
    /// Source-HUD icons.
    pub icons: Rc<RefCell<dyn SeatHudIcons>>,
    /// Game prompt.
    pub prompt: Box<dyn SeatPromptUi>,
    /// Player death screen.
    pub death: Box<dyn SeatDeathUi>,
    /// Q2 match UI.
    pub match_ui: Box<dyn SeatMatchUi>,
    /// Native Q2 HUD.
    pub native_q2_hud: Box<dyn SeatQ2NativeHud>,
    /// Authored wheel.
    pub authored_wheel: Rc<RefCell<dyn SeatAuthoredWheel>>,
    /// Base-arena menus, if any.
    pub base_arena: Option<Box<dyn SeatBaseArenaUi>>,
    /// Team Arena results, if any.
    pub team_arena: Option<Box<dyn SeatTeamArenaUi>>,
    /// Host server settings, if any.
    pub host_settings: Option<HostServerSettingsUi>,
    /// Language binding, if any.
    pub language: Option<SettingBinding>,
    /// Save-menu service, if any.
    pub saves: Option<Rc<RefCell<dyn SavedGameMenuService>>>,
    /// View binding, if any.
    pub view_setting: Option<SettingBinding>,
    /// LLM settings, if any.
    pub llm: Option<Rc<RefCell<dyn LlmSettingsUi>>>,
    /// Guest UI mode (donor `guestUi`).
    pub guest_ui: bool,
    /// Gameplay settings source, if any.
    pub gameplay: Option<Rc<GameplaySettingsSource>>,
    /// Return-to-lobby callback, if any.
    pub lobby: Option<Rc<dyn Fn()>>,
    /// Ranking account hooks, if any.
    pub rankings: Option<UiSeatRankings>,
    /// Native HUD localizer override, if any.
    pub native_hud_localizer: Option<NativeHudLocalizer>,
    /// Source localizer override, if any.
    pub localize: Option<SeatTextLocalizer>,
    /// Source-HUD actor (donor `local.player.actor` as a remote id).
    pub source_actor: RemoteActorId,
    /// Weapon picture resolver (load key to GPU asset; the renderer lane
    /// owns uploads, so weapon icons resolve here before menu art).
    pub pictures: Option<WeaponPictureResolver>,
    /// Caption catalog marker.
    pub catalog: std::marker::PhantomData<Catalog>,
}

/// Ranking account hooks (donor `options.rankings`).
#[derive(Clone)]
pub struct UiSeatRankings {
    /// Current account actions, if any.
    pub current: RankingAccountSource,
    /// Take a pending menu request.
    pub take_menu_request: Rc<dyn Fn() -> bool>,
}

/// Wheel services over seat collaborators.
pub struct SeatWheelServices {
    seat: SeatId,
    simulation: Rc<dyn SeatSimulationUi>,
    authored_wheel: Rc<RefCell<dyn SeatAuthoredWheel>>,
    actor: ActorId,
    guest_ui: bool,
    command: Rc<dyn Fn(String, Vec<String>)>,
    audio: Rc<dyn SeatHudAudio>,
    wheel_icons: Rc<RefCell<HashMap<String, ResourceId>>>,
    now: Rc<dyn Fn() -> i64>,
}

impl WeaponWheelServices for SeatWheelServices {
    fn seat(&self) -> SeatId {
        self.seat.clone()
    }

    fn items(&self, mode: WheelMode) -> Vec<WheelItem> {
        let player = self.simulation.player_ui(&self.actor);
        let authored = if mode == WheelMode::Weapons && !self.guest_ui {
            self.authored_wheel.borrow().items(&player)
        } else {
            None
        };
        let items = authored.unwrap_or_else(|| {
            if self.guest_ui {
                Vec::new()
            } else {
                player.items.clone()
            }
        });
        let want = if mode == WheelMode::Weapons {
            "weapon"
        } else {
            "powerup"
        };
        items
            .into_iter()
            .filter(|item| item.kind == want)
            .map(|item| {
                let icon = self.wheel_icons.borrow().get(&item.id).cloned();
                WheelItem {
                    id: item.id.clone(),
                    source_ordinal: item.source_ordinal,
                    sort_order: item.source_ordinal,
                    label: item.label.clone(),
                    owned: item.owned,
                    has_ammo: true,
                    count: None,
                    warning_count: 0,
                    icon: icon.clone(),
                    selected_icon: icon,
                }
            })
            .collect()
    }

    fn active_item(&self) -> Option<String> {
        if self.guest_ui {
            None
        } else {
            self.simulation.player_ui(&self.actor).active_weapon.clone()
        }
    }

    fn select(&mut self, id: &str, _mode: WheelMode, _seat: SeatId) {
        (self.command)("use".to_string(), vec![id.to_string()]);
    }

    fn now(&self) -> i64 {
        (self.now)()
    }

    fn changed(&mut self, seat: SeatId) {
        self.audio.ui_sound(UiSound::Move, &seat);
    }
}

impl RereleasePresentationProvider for Rc<dyn RereleasePresentationProvider> {
    fn family(&self, content: &ContentId) -> Option<qa_content::contract::GameFamily> {
        (**self).family(content)
    }

    fn list_localization_files(&self, content: &ContentId) -> Vec<String> {
        (**self).list_localization_files(content)
    }

    fn open_localization(&self, content: &ContentId, path: &str) -> Option<Vec<u8>> {
        (**self).open_localization(content, path)
    }

    fn load_sky_face(&self, content: &ContentId, path: &str) -> qa_client::render::types::RendererImage {
        (**self).load_sky_face(content, path)
    }
}

/// Prepared native inventory item (donor `nativeInventoryItem`).
#[derive(Debug, Clone, PartialEq)]
struct PreparedNativeItem {
    item: String,
    source: ProviderReference,
    label: String,
    localized_label: String,
    icon: Option<ArsenalIcon>,
}

/// The per-seat game UI (donor `ApplicationSeatUi`).
pub struct ApplicationSeatUi<Catalog> {
    controller: Rc<RefCell<NativeUiController>>,
    art: NativeUiArt,
    actor: ActorId,
    seat: SeatId,
    client: ClientId,
    simulation: Rc<dyn SeatSimulationUi>,
    command: Rc<dyn Fn(String, Vec<String>)>,
    focus: Rc<dyn SeatFocus>,
    dialect: CommandDialect,
    now: Rc<dyn Fn() -> i64>,
    shared_settings: Option<Rc<dyn SettingCvars>>,
    hud_cvars: Rc<RefCell<CvarRegistry>>,
    caption_clock: Rc<dyn Fn() -> VoiceClockSample>,
    icons: Rc<RefCell<dyn SeatHudIcons>>,
    prompt: Box<dyn SeatPromptUi>,
    death: Box<dyn SeatDeathUi>,
    match_ui: Box<dyn SeatMatchUi>,
    native_q2_hud: Box<dyn SeatQ2NativeHud>,
    authored_wheel: Rc<RefCell<dyn SeatAuthoredWheel>>,
    base_arena: Option<Box<dyn SeatBaseArenaUi>>,
    team_arena: Option<Box<dyn SeatTeamArenaUi>>,
    guest_ui: bool,
    native_hud_localizer: Option<NativeHudLocalizer>,
    native_hud_localizations: Option<ApplicationRereleasePresentation<Rc<dyn RereleasePresentationProvider>>>,
    manual_pause: bool,
    nav: Rc<RefCell<Vec<SeatNavIntent>>>,
    font: TextFontSelection,
    typography: MenuTypography,
    text: UiTextRenderer,
    menu_text: UiTextRenderer,
    preferences: Rc<qa_client::ui::settings::SeatUiPreferences>,
    messages: SeatHudMessages,
    weapon_wheel: SeatWeaponWheel<SeatWheelServices>,
    wheel_icons: Rc<RefCell<HashMap<String, ResourceId>>>,
    weapon_icons: SeatWeaponIcons,
    weapon_assets: ApplicationWeaponHudAssets,
    pictures: Option<WeaponPictureResolver>,
    inventory_root: UiMenuId,
    ranking_root: Option<UiMenuId>,
    take_ranking_request: Option<Rc<dyn Fn() -> bool>>,
    native_inventory_item: Option<PreparedNativeItem>,
    source_hud: SeatSourceHud<BoxedSourceLocalizer>,
    sound_captions: SeatSoundCaptions<Catalog>,
    disposers: Vec<Box<dyn FnOnce()>>,
}

/// Prepared weapon/ammo icons.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct SeatWeaponIcons {
    weapon: Option<ResourceId>,
    ammo: Option<ResourceId>,
}

impl<Catalog: SoundMediaCaptions + Clone + 'static> ApplicationSeatUi<Catalog> {
    /// Build the seat UI over shared handles.
    #[allow(clippy::too_many_lines)]
    pub fn new(options: UiSeatOptions<Catalog>) -> Result<Self, UiSeatError> {
        let seat = options.seat.clone();
        let mut text = UiTextRenderer::new(seat.clone());
        text.bind(super::startup_menu::MENU_BODY_FONT_SLOT, options.font.clone());
        let mut menu_text = UiTextRenderer::new(seat.clone());
        menu_text.bind(
            super::startup_menu::MENU_BODY_FONT_SLOT,
            options.typography.body.clone(),
        );
        menu_text.bind(
            super::startup_menu::MENU_TITLE_FONT_SLOT,
            options.typography.title.clone(),
        );
        let preferences = Rc::new(qa_client::ui::settings::SeatUiPreferences::new(
            seat.clone(),
            options.shared_settings.clone(),
        ));
        let controller_preferences = options.shared_settings.clone();
        let controller_seat = seat.clone();
        let controller_focus = Rc::clone(&options.focus);
        let controller_haptics = Rc::clone(&options.haptics);
        let controller_update_capture = Rc::clone(&options.update_capture);
        let controller_audio = Rc::clone(&options.audio);
        let controller_now = Rc::clone(&options.now);
        let controller_font = options.font.clone();
        let controller_typography = options.typography.clone();
        let controller_art_font = options.art.skin.font.clone();
        let controller = Rc::new(RefCell::new(NativeUiController::new(NativeUiOptions {
            seat: seat.clone(),
            skin: Box::new(move || menu_skin(&controller_art_font)),
            now: Box::new(move || controller_now()),
            bindings: Box::new({
                let focus = Rc::clone(&controller_focus);
                move || focus.bindings()
            }),
            focus: Box::new(move |focus, time| {
                controller_focus.set_focus(focus.clone(), time);
                controller_haptics.set_active(controller_focus.focused() && matches!(focus, SeatInputFocus::Game));
                controller_update_capture();
            }),
            sound: Box::new(move |sound, owner| controller_audio.ui_sound(sound, &owner)),
            execute_script: Box::new(|script, _| {
                panic!(
                    "Legacy UI module {:?} is not attached to this native menu",
                    script.module
                )
            }),
            localize: Box::new(|text| text.to_string()),
            appearance: Box::new(move || {
                let values = qa_client::ui::settings::SeatUiPreferences::new(
                    controller_seat.clone(),
                    controller_preferences.clone(),
                )
                .values();
                qa_client::ui::types::UiAppearance {
                    menu_scale: values.menu_scale,
                    text_scale: values.text_scale,
                    high_contrast: values.high_contrast,
                    color_mode: values.color_mode,
                }
            }),
            clipboard: Box::new(|| {
                qa_platform::sdl::read_sdl_clipboard()
                    .ok()
                    .flatten()
                    .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            }),
            measure_text: Box::new(move |text, scale| {
                layout_text(&TextLayoutOptions {
                    text,
                    font: &controller_typography.body,
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
            }),
        })));
        let _ = &controller_font;
        let mut disposers: Vec<Box<dyn FnOnce()>> = Vec::new();
        let source_localize: Option<BoxedSourceLocalizer> = options.localize.clone().map(|localize| {
            Box::new(move |content: &UiContentId, text: &str, args: &[String]| {
                localize(ContentId(content.as_str().to_string()), text.to_string(), args.to_vec())
            }) as BoxedSourceLocalizer
        });
        let source_hud = SeatSourceHud::with_localize(options.source_actor, source_localize);
        let sound_captions = SeatSoundCaptions::new();
        let command = Rc::clone(&options.command);
        let inventory_command: CommandFn = {
            let command = Rc::clone(&command);
            Rc::new(move |name, args| command(name.to_string(), args.to_vec()))
        };
        let source_items = Rc::new(RefCell::new(Vec::<HudInventoryItem>::new()));
        let _ = &source_items;
        let inventory = register_inventory_menu(&controller, Rc::new(|| None), inventory_command);
        let inventory_root = inventory.root.clone();
        disposers.push(Box::new(|| inventory.dispose()));
        let match_menus = if options.dialect == CommandDialect::Q3 {
            let command = Rc::clone(&command);
            let menus = register_match_menu(
                &controller,
                Rc::new(move |name, args| command(name.to_string(), args.to_vec())),
            );
            Some(menus)
        } else {
            None
        };
        if let Some(menus) = match_menus {
            disposers.push(Box::new(|| menus.dispose()));
        }
        let wheel_icons: Rc<RefCell<HashMap<String, ResourceId>>> = Rc::new(RefCell::new(HashMap::new()));
        let authored_wheel = Rc::clone(&options.authored_wheel);
        let wheel = SeatWeaponWheel::new(
            SeatWheelServices {
                seat: seat.clone(),
                simulation: Rc::clone(&options.simulation),
                authored_wheel: Rc::clone(&authored_wheel) as Rc<RefCell<dyn SeatAuthoredWheel>>,
                actor: options.actor.clone(),
                guest_ui: options.guest_ui,
                command: Rc::clone(&command),
                audio: Rc::clone(&options.audio),
                wheel_icons: Rc::clone(&wheel_icons),
                now: Rc::clone(&options.now),
            },
            DEFAULT_WHEEL_OPTIONS,
        )?;
        let binding_source = BindingSource::Live(Rc::new({
            let simulation = Rc::clone(&options.simulation);
            let actor = options.actor.clone();
            let guest_ui = options.guest_ui;
            let dialect = options.dialect;
            let capabilities = Rc::clone(&options.binding_capabilities);
            move || {
                let items: Vec<BindableItem> = if guest_ui {
                    Vec::new()
                } else {
                    simulation
                        .player_ui(&actor)
                        .items
                        .iter()
                        .filter(|item| item.kind == "weapon" || item.kind == "powerup")
                        .map(|item| BindableItem {
                            id: item.id.clone(),
                            label: item.label.clone(),
                            kind: if item.kind == "weapon" {
                                qa_client::ui::settings::action_catalog::BindableItemKind::Weapon
                            } else {
                                qa_client::ui::settings::action_catalog::BindableItemKind::Powerup
                            },
                        })
                        .collect()
                };
                shared_binding_actions(dialect, &items, &capabilities())
            }
        }));
        let reset_seat = seat.clone();
        let can_reset = Rc::clone(&options.can_reset_bindings);
        let do_reset = Rc::clone(&options.reset_bindings);
        let bindings = register_binding_menus(
            &controller,
            Rc::clone(&options.binding_store),
            binding_source,
            Some(BindingResetHandle {
                available: Rc::new(move || can_reset(reset_seat.clone())),
                reset: {
                    let seat = seat.clone();
                    let do_reset = Rc::clone(&do_reset);
                    Rc::new(move || do_reset(seat.clone()))
                },
            }),
        );
        let bindings_root = bindings.root.clone();
        disposers.push(Box::new(|| bindings.dispose()));
        let audio_service: Rc<dyn SettingsValueService<AudioSettings>> = Rc::new(AudioSettingsAdapter {
            audio: Rc::clone(&options.audio),
        });
        let console_print = Rc::clone(&options.console);
        let audio_output: Rc<dyn AudioOutputSettings> = Rc::new(AudioOutputAdapter {
            audio: Rc::clone(&options.audio),
            report: Rc::new(move |text| console_print.print(&format!("{text}\n"))),
        });
        let volumes = bind_audio_settings(audio_service, Some(audio_output));
        let report_console = Rc::clone(&options.console);
        let report: Rc<dyn Fn(&str)> = Rc::new(move |message| report_console.print(&format!("{message}\n")));
        let apply_commands = Rc::clone(&options.commands);
        let apply_context = Rc::clone(&options.command_context);
        let apply_renderer = Rc::clone(&options.apply_renderer);
        let display = [
            bind_renderer_settings(RendererOptions {
                current: Rc::clone(&options.renderer_backend),
                worker: options.renderer_worker.clone(),
                apply: Rc::new(move |backend| {
                    let text = format!("vid_restart {}\n", backend.as_str());
                    apply_renderer(backend);
                    let context = apply_context();
                    let _ = apply_commands.borrow_mut().append(&text, Some(&context), None);
                }),
                report: Rc::clone(&report),
                enabled: Rc::new(|| true),
            }),
            bind_native_video_settings(
                Rc::clone(&options.window),
                options.shared_settings.clone(),
                Rc::clone(&report),
            ),
        ]
        .concat();
        let images: Vec<SettingBinding> = match &options.shared_settings {
            None => Vec::new(),
            Some(shared) => bind_image_settings(shared)?
                .into_iter()
                .chain(bind_model_settings(shared)?)
                .chain(bind_console_settings(shared)?)
                .collect(),
        };
        let ranking_menus = options
            .rankings
            .as_ref()
            .map(|rankings| register_ranking_account_menu(&controller, Rc::clone(&rankings.current)));
        let ranking_root = ranking_menus.as_ref().map(|menus| menus.root.clone());
        let take_ranking_request = options
            .rankings
            .as_ref()
            .map(|rankings| Rc::clone(&rankings.take_menu_request));
        let ranking_settings: Vec<SettingBinding> = match (&ranking_menus, &options.rankings) {
            (Some(menus), Some(rankings)) => {
                let root = menus.root.clone();
                let current = Rc::clone(&rankings.current);
                let open = Rc::clone(&controller);
                vec![SettingBinding {
                    id: UiControlId::new("ui:network:rankings")?,
                    label: "Ranking account".to_string(),
                    category: SettingCategory::Network,
                    enabled: Rc::new(move || current().is_some()),
                    kind: SettingBindingKind::Button {
                        activate: Rc::new(move || {
                            let _ = open.borrow_mut().open_menu(&root);
                        }),
                    },
                }]
            }
            _ => Vec::new(),
        };
        let server_menus = match options.host_settings.clone() {
            None => None,
            Some(host) => {
                let bindings_count = (host.bindings)().len();
                let menus = register_server_settings_menu(&controller, host);
                let root = menus.root.clone();
                let open = Rc::clone(&controller);
                let _ = bindings_count;
                Some((
                    menus,
                    SettingBinding {
                        id: UiControlId::new("ui:network:server-settings")?,
                        label: "Server settings".to_string(),
                        category: SettingCategory::Network,
                        enabled: Rc::new(move || true),
                        kind: SettingBindingKind::Button {
                            activate: Rc::new(move || {
                                let _ = open.borrow_mut().open_menu(&root);
                            }),
                        },
                    },
                ))
            }
        };
        let gyro_menus = register_gyro_settings_menu(&controller, options.gyro.clone(), Some(""));
        let gyro_root = gyro_menus.root.clone();
        let local_count = Rc::clone(&options.local_count);
        let local_capacity = options.local_capacity;
        let can_remove = Rc::clone(&options.can_remove_local);
        let remove_seat = seat.clone();
        let join_command = Rc::clone(&command);
        let drop_command = Rc::clone(&command);
        let drop_seat = seat_index_for_command(options.seat_index);
        let vibration: Option<VibrationService> = options.vibration.then(|| {
            Rc::new(VibrationAdapter {
                haptics: Rc::clone(&options.haptics),
            }) as VibrationService
        });
        let gameplay_reset = options.gameplay.clone().map(|source| {
            let label = "Reset player preferences...".to_string();
            SettingReset {
                label,
                apply: Rc::new(move || reset_gameplay_settings(&source)),
            }
        });
        let mut all_settings = display;
        if let Some(view) = options.view_setting.clone() {
            all_settings.push(view);
        }
        all_settings.extend(images);
        if let Some(language) = options.language.clone() {
            all_settings.push(language);
        }
        all_settings.push(SettingBinding {
            id: UiControlId::new("ui:input:bindings")?,
            label: "Key and controller bindings".to_string(),
            category: SettingCategory::Input,
            enabled: Rc::new(|| true),
            kind: {
                let open = Rc::clone(&controller);
                let root = bindings_root.clone();
                SettingBindingKind::Button {
                    activate: Rc::new(move || {
                        let _ = open.borrow_mut().open_menu(&root);
                    }),
                }
            },
        });
        all_settings.push(SettingBinding {
            id: UiControlId::new("ui:settings:gyro")?,
            label: "Gyro controls".to_string(),
            category: SettingCategory::Input,
            enabled: Rc::new(|| true),
            kind: {
                let open = Rc::clone(&controller);
                SettingBindingKind::Button {
                    activate: Rc::new(move || {
                        let _ = open.borrow_mut().open_menu(&gyro_root);
                    }),
                }
            },
        });
        if let Some((_, binding)) = &server_menus {
            all_settings.push(binding.clone());
        }
        all_settings.extend(ranking_settings);
        all_settings.extend(options.device_bindings.clone());
        all_settings.extend(bind_input_routing_settings(
            Rc::clone(&options.router),
            Rc::clone(&options.router_devices),
        ));
        all_settings.push(SettingBinding {
            id: UiControlId::new("ui:input:local-join")?,
            label: "Add local player".to_string(),
            category: SettingCategory::Input,
            enabled: Rc::new(move || local_count() < local_capacity.min(4)),
            kind: SettingBindingKind::Button {
                activate: Rc::new(move || join_command("local_join".to_string(), Vec::new())),
            },
        });
        all_settings.push(SettingBinding {
            id: UiControlId::new("ui:input:local-drop")?,
            label: "Remove this player".to_string(),
            category: SettingCategory::Input,
            enabled: Rc::new(move || can_remove(remove_seat.clone())),
            kind: SettingBindingKind::Button {
                activate: Rc::new(move || drop_command("local_drop".to_string(), vec![drop_seat.clone()])),
            },
        });
        all_settings.extend(bind_input_settings(&options.tuning_host, vibration));
        all_settings.extend(volumes);
        all_settings.extend(bind_audio_geometry_settings(options.shared_settings.as_ref()));
        all_settings.extend(bind_music_playlist_settings(options.shared_settings.as_ref(), None));
        all_settings.extend(preferences.bindings());
        if let Some(gameplay) = options.gameplay.as_ref() {
            all_settings.extend(bind_gameplay_settings(gameplay));
        }
        let settings_menus = register_settings_menus(&controller, &all_settings, options.llm.clone(), gameplay_reset);
        let settings_root = settings_menus.root.clone();
        disposers.push(Box::new(|| settings_menus.dispose()));
        disposers.push(Box::new(|| gyro_menus.dispose()));
        if let Some((menus, _)) = server_menus {
            disposers.push(Box::new(|| menus.dispose()));
        }
        if let Some(menus) = ranking_menus {
            disposers.push(Box::new(|| menus.dispose()));
        }
        let saves_menus = register_saved_game_menus(&controller, options.saves.clone());
        let saves_save = saves_menus.save.clone();
        let saves_load = saves_menus.load.clone();
        disposers.push(Box::new(|| saves_menus.dispose()));
        let pause_id = UiMenuId::new(PAUSE_MENU).map_err(|error| UiSeatError::Failed(error.to_string()))?;
        let nav: Rc<RefCell<Vec<SeatNavIntent>>> = Rc::new(RefCell::new(Vec::new()));
        let pause_nav = Rc::clone(&nav);
        let has_arena = options.base_arena.is_some();
        let has_match = options.dialect == CommandDialect::Q3;
        let has_lobby = options.lobby.clone();
        let pause_console = Rc::clone(&options.console);
        let pause_quit = Rc::clone(&options.quit);
        controller.borrow_mut().register(
            pause_id.clone(),
            Rc::new(move || {
                let mut controls = vec![
                    pause_button(&pause_nav, "resume", "Resume game", 1, SeatNavIntent::CloseAll),
                    pause_button(
                        &pause_nav,
                        "save",
                        "Save game",
                        2,
                        SeatNavIntent::Open(saves_save.clone()),
                    ),
                    pause_button(
                        &pause_nav,
                        "load",
                        "Load game",
                        3,
                        SeatNavIntent::Open(saves_load.clone()),
                    ),
                    pause_button(
                        &pause_nav,
                        "settings",
                        "Options",
                        4,
                        SeatNavIntent::Open(settings_root.clone()),
                    ),
                ];
                let console_nav = Rc::clone(&pause_nav);
                let toggle_console = Rc::clone(&pause_console);
                controls.push(UiControl {
                    id: UiControlId::new("ui:application:console")
                        .unwrap_or_else(|_| panic!("static UI control id is invalid")),
                    label: "Console".to_string(),
                    rect: menu_row(
                        5,
                        &MenuRowOptions {
                            x: None,
                            y: None,
                            width: None,
                            height: None,
                        },
                    ),
                    enabled: true,
                    visible: true,
                    kind: UiControlKind::Button {
                        on_activate: Rc::new(move |_| {
                            console_nav.borrow_mut().push(SeatNavIntent::CloseAll);
                            toggle_console.toggle();
                        }),
                    },
                });
                if has_arena {
                    controls.push(pause_button(
                        &pause_nav,
                        "progress",
                        "Arena progress",
                        6,
                        SeatNavIntent::Open(arena_progress_id()),
                    ));
                }
                if has_match {
                    controls.push(pause_button(
                        &pause_nav,
                        "match",
                        "Match controls",
                        7,
                        SeatNavIntent::Open(match_menu_id()),
                    ));
                }
                if let Some(lobby) = &has_lobby {
                    let lobby = Rc::clone(lobby);
                    controls.push(UiControl {
                        id: UiControlId::new("ui:application:lobby")
                            .unwrap_or_else(|_| panic!("static UI control id is invalid")),
                        label: "Return to lobby".to_string(),
                        rect: menu_row(
                            8,
                            &MenuRowOptions {
                                x: None,
                                y: None,
                                width: None,
                                height: None,
                            },
                        ),
                        enabled: true,
                        visible: true,
                        kind: UiControlKind::Button {
                            on_activate: Rc::new(move |_| lobby()),
                        },
                    });
                }
                let quit = Rc::clone(&pause_quit);
                controls.push(UiControl {
                    id: UiControlId::new("ui:application:quit")
                        .unwrap_or_else(|_| panic!("static UI control id is invalid")),
                    label: "End game".to_string(),
                    rect: menu_row(
                        if has_lobby.is_some() { 9 } else { 8 },
                        &MenuRowOptions {
                            x: None,
                            y: None,
                            width: None,
                            height: None,
                        },
                    ),
                    enabled: true,
                    visible: true,
                    kind: UiControlKind::Button {
                        on_activate: Rc::new(move |_| quit()),
                    },
                });
                UiMenu {
                    scroll: None,
                    id: UiMenuId::new(PAUSE_MENU).unwrap_or_else(|_| panic!("static UI menu id is invalid")),
                    title: "Paused".to_string(),
                    full_screen: true,
                    controls,
                    on_open: Rc::new(|_| {}),
                    on_close: Rc::new(|_| {}),
                }
            }),
        );
        let unregister_id = pause_id.clone();
        let unregister_controller = Rc::clone(&controller);
        disposers.push(Box::new(move || {
            unregister_controller.borrow_mut().unregister(&unregister_id)
        }));
        let dispose_input = (options.attach)(seat.clone());
        disposers.push(dispose_input);
        Ok(Self {
            controller,
            art: options.art,
            actor: options.actor,
            seat: seat.clone(),
            client: options.client,
            simulation: options.simulation,
            command,
            focus: options.focus,
            dialect: options.dialect,
            now: options.now,
            shared_settings: options.shared_settings,
            hud_cvars: options.hud_cvars,
            caption_clock: options.caption_clock,
            icons: options.icons,
            prompt: options.prompt,
            death: options.death,
            match_ui: options.match_ui,
            native_q2_hud: options.native_q2_hud,
            authored_wheel,
            base_arena: options.base_arena,
            team_arena: options.team_arena,
            guest_ui: options.guest_ui,
            native_hud_localizer: options.native_hud_localizer,
            native_hud_localizations: None,
            manual_pause: false,
            nav,
            font: options.font,
            typography: options.typography,
            text,
            menu_text,
            preferences,
            messages: SeatHudMessages::new(seat.clone()),
            weapon_wheel: wheel,
            wheel_icons,
            weapon_icons: SeatWeaponIcons::default(),
            weapon_assets: ApplicationWeaponHudAssets::new(),
            pictures: options.pictures,
            inventory_root,
            ranking_root,
            take_ranking_request,
            native_inventory_item: None,
            source_hud,
            sound_captions,
            disposers,
        })
    }
}

/// Haptics-backed vibration settings.
struct VibrationAdapter {
    haptics: Rc<dyn SeatHaptics>,
}

impl SettingsValueService<qa_client::ui::settings::ControllerVibrationSettings> for VibrationAdapter {
    fn read(&self) -> qa_client::ui::settings::ControllerVibrationSettings {
        qa_client::ui::settings::ControllerVibrationSettings {
            controller_vibration: self.haptics.enabled(),
            controller_vibration_strength: self.haptics.strength(),
        }
    }

    fn write(&self, update: &dyn Fn(&mut qa_client::ui::settings::ControllerVibrationSettings)) {
        let mut values = self.read();
        update(&mut values);
        self.haptics.set_strength(values.controller_vibration_strength);
        self.haptics.set_enabled(values.controller_vibration);
    }
}

/// Seat index for the `local_drop` command (donor `String(seat.index + 1)`).
fn seat_index_for_command(seat_index: u32) -> String {
    (seat_index + 1).to_string()
}

/// One pause-menu button queuing navigation (donor `button`).
fn pause_button(
    nav: &Rc<RefCell<Vec<SeatNavIntent>>>,
    id: &str,
    label: &str,
    row: i32,
    intent: SeatNavIntent,
) -> UiControl {
    let nav = Rc::clone(nav);
    UiControl {
        id: UiControlId::new(&format!("ui:application:{id}"))
            .unwrap_or_else(|_| panic!("static UI control id is invalid")),
        label: label.to_string(),
        rect: menu_row(row, &MenuRowOptions::default()),
        enabled: true,
        visible: true,
        kind: UiControlKind::Button {
            on_activate: Rc::new(move |_| {
                nav.borrow_mut().push(intent.clone());
            }),
        },
    }
}

/// Arena progress menu id (donor `menu:application:arena-progress`).
fn arena_progress_id() -> UiMenuId {
    UiMenuId::new("menu:application:arena-progress").unwrap_or_else(|_| panic!("static UI menu id is invalid"))
}

/// Match menu id (donor `menu:application:match`).
fn match_menu_id() -> UiMenuId {
    UiMenuId::new(qa_client::ui::library::match_menu::MATCH_MENU_ID)
        .unwrap_or_else(|_| panic!("static UI menu id is invalid"))
}

/// Shared source-HUD icons behind one `Rc` (donor `HudAssets`).
struct SharedHudIcons(Rc<RefCell<dyn SeatHudIcons>>);

impl SeatHudIcons for SharedHudIcons {
    fn load_image(&mut self, content: &UiContentId, path: &str) -> ResourceId {
        self.0.borrow_mut().load_image(content, path)
    }

    fn picture_size(&self, image: &ResourceId) -> Option<(f32, f32)> {
        self.0.borrow().picture_size(image)
    }

    fn palette(&mut self, content: &UiContentId) -> Option<Vec<u8>> {
        self.0.borrow_mut().palette(content)
    }
}

/// Prompt menu id (donor `gamePromptMenu`).
fn prompt_menu_id() -> UiMenuId {
    UiMenuId::new(GAME_PROMPT_MENU).unwrap_or_else(|_| panic!("static UI menu id is invalid"))
}

/// Death menu id (donor `playerDeathMenu`).
fn death_menu_id() -> UiMenuId {
    UiMenuId::new(PLAYER_DEATH_MENU).unwrap_or_else(|_| panic!("static UI menu id is invalid"))
}

/// Pause menu id (donor `menu:application:game`).
fn pause_menu_id() -> UiMenuId {
    UiMenuId::new(PAUSE_MENU).unwrap_or_else(|_| panic!("static UI menu id is invalid"))
}

/// Weapon status reference for icon preparation.
fn hud_status_ref(status: &SeatWeaponStatus) -> HudStatusRef {
    HudStatusRef {
        source_provider: status.source_provider.clone(),
        source_content: status.source_content.clone(),
        item: status.item.clone(),
    }
}

/// Weapon load key as a namespaced resource id.
fn weapon_resource_id(key: &str) -> Option<ResourceId> {
    ResourceId::new(&format!("resource:{key}")).ok()
}

/// Load key behind a weapon resource id.
fn weapon_load_key(id: &ResourceId) -> &str {
    id.as_str().strip_prefix("resource:").unwrap_or(id.as_str())
}

impl<Catalog: SoundMediaCaptions + Clone + 'static> ApplicationSeatUi<Catalog> {
    /// Borrow the menu controller.
    #[must_use]
    pub fn controller(&self) -> &Rc<RefCell<NativeUiController>> {
        &self.controller
    }

    /// Whether the pause menu (or a modal result/death screen) is open.
    #[must_use]
    pub fn pause_menu_open(&self) -> bool {
        self.manual_pause
            || self.team_arena.as_ref().is_some_and(|arena| arena.active())
            || self.death.active() && self.controller.borrow().active_menu().as_ref() != Some(&death_menu_id())
    }

    /// Borrow the seat preferences.
    #[must_use]
    pub fn preferences(&self) -> &Rc<qa_client::ui::settings::SeatUiPreferences> {
        &self.preferences
    }

    /// Borrow the HUD messages.
    #[must_use]
    pub fn messages(&self) -> &SeatHudMessages {
        &self.messages
    }

    /// Borrow the weapon wheel.
    #[must_use]
    pub fn weapon_wheel(&self) -> &SeatWeaponWheel<SeatWheelServices> {
        &self.weapon_wheel
    }

    /// Borrow the HUD text renderer.
    #[must_use]
    pub fn text(&self) -> &UiTextRenderer {
        &self.text
    }

    /// HUD font for the current typeface.
    fn hud_font(&self) -> TextFontSelection {
        if self.preferences.values().typeface == Typeface::Bold {
            self.typography.title.clone()
        } else {
            self.font.clone()
        }
    }

    /// Menu font for the current typeface.
    fn menu_font(&self) -> TextFontSelection {
        if self.preferences.values().typeface == Typeface::Bold {
            self.typography.title.clone()
        } else {
            self.typography.body.clone()
        }
    }

    /// Rebind fonts after an image refresh (donor `refreshImages`).
    pub fn refresh_images(&mut self, font: TextFontSelection, typography: MenuTypography) {
        self.font = font.clone();
        self.typography = typography.clone();
        self.text.bind(super::startup_menu::MENU_BODY_FONT_SLOT, font);
        self.menu_text
            .bind(super::startup_menu::MENU_BODY_FONT_SLOT, typography.body);
        self.menu_text
            .bind(super::startup_menu::MENU_TITLE_FONT_SLOT, typography.title);
    }

    /// Reload weapon pictures, returning the deferred swap (donor
    /// `prepareImageRefresh`; the caller commits via
    /// [`Self::commit_image_refresh`]).
    pub fn prepare_image_refresh(
        &mut self,
        assets: &mut dyn SeatPrepareAssets<Catalog>,
    ) -> Result<super::weapon_hud::WeaponHudImageRefresh, UiSeatError> {
        let (backend, _) = assets.weapon_services();
        Ok(self.weapon_assets.prepare_image_refresh(backend)?)
    }

    /// Commit a refresh from [`Self::prepare_image_refresh`] and release native
    /// HUD state.
    pub fn commit_image_refresh(&mut self, refresh: super::weapon_hud::WeaponHudImageRefresh) {
        refresh.commit(&mut self.weapon_assets);
        self.native_q2_hud.clear();
    }

    /// Prepare one frame (donor `prepare`).
    pub fn prepare(&mut self, assets: &mut dyn SeatPrepareAssets<Catalog>) -> Result<(), UiSeatError> {
        let player = self.simulation.player_ui(&self.actor);
        if self.guest_ui {
            let (backend, resolver) = assets.weapon_services();
            let status = player.weapon_status.as_ref().map(hud_status_ref);
            let icons = self.weapon_assets.prepare(backend, resolver, status.as_ref())?;
            self.weapon_icons = SeatWeaponIcons {
                weapon: icons.weapon.as_deref().and_then(weapon_resource_id),
                ammo: icons.ammo.as_deref().and_then(weapon_resource_id),
            };
            return Ok(());
        }
        if self.take_ranking_request.as_ref().is_some_and(|take| take()) {
            if let Some(root) = self.ranking_root.clone() {
                let _ = self.controller.borrow_mut().open_menu(&root);
            }
        }
        {
            let Self {
                team_arena,
                base_arena,
                controller,
                ..
            } = self;
            if let Some(arena) = team_arena.as_mut() {
                let _ = arena.update(&mut controller.borrow_mut());
            }
            if let Some(arena) = base_arena.as_mut() {
                arena.update(&mut controller.borrow_mut());
            }
        }
        if self.team_arena.is_none() {
            self.death.observe(player.health);
        }
        if self.death.active() {
            self.weapon_wheel.close(false);
        }
        let focus = Rc::clone(&self.focus);
        self.prompt.prepare(Rc::new(move || focus.focus()));
        let status = player.weapon_status.as_ref().map(hud_status_ref);
        let (backend, resolver) = assets.weapon_services();
        let icons = self.weapon_assets.prepare(backend, resolver, status.as_ref())?;
        self.weapon_icons = SeatWeaponIcons {
            weapon: icons.weapon.as_deref().and_then(weapon_resource_id),
            ammo: icons.ammo.as_deref().and_then(weapon_resource_id),
        };
        let (backend, resolver) = assets.weapon_services();
        let _ = backend;
        self.authored_wheel
            .borrow_mut()
            .prepare(resolver, status.as_ref(), &player);
        self.source_hud.prepare(&mut SharedHudIcons(Rc::clone(&self.icons)));
        for message in self.source_hud.drain_objective_prints() {
            let _ = self.messages.center_print(
                &self.seat,
                &message.text,
                SourceTime::Seconds(message.seconds as f32),
                SourceTime::Seconds(5.0),
                false,
                40.0,
            );
        }
        let language = match self.shared_settings.as_ref() {
            None => "english".to_string(),
            Some(shared) => read_seat_language(shared.as_ref(), self.seat.index()),
        };
        let clock = (self.caption_clock)();
        self.sound_captions.prepare(clock, &language, assets.caption_loader());
        if player.weapon_status.is_some() {
            for item in player.items.iter().filter(|item| item.kind == "weapon") {
                let item_status = player.weapon_status.as_ref().map(|status| HudStatusRef {
                    source_provider: status.source_provider.clone(),
                    source_content: status.source_content.clone(),
                    item: item.id.clone(),
                });
                let (backend, resolver) = assets.weapon_services();
                if let Ok(prepared) = self.weapon_assets.prepare(backend, resolver, item_status.as_ref()) {
                    if let Some(weapon) = prepared.weapon {
                        if let Some(id) = weapon_resource_id(&weapon) {
                            self.wheel_icons.borrow_mut().insert(item.id.clone(), id);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Drain queued pause-menu navigation.
    fn drain_nav(&mut self) {
        let intents = std::mem::take(&mut *self.nav.borrow_mut());
        for intent in intents {
            match intent {
                SeatNavIntent::Open(id) => {
                    let _ = self.controller.borrow_mut().open_menu(&id);
                }
                SeatNavIntent::CloseAll => self.controller.borrow_mut().close_all(),
            }
        }
        self.manual_pause = self.controller.borrow().active_menu().as_ref() == Some(&pause_menu_id());
    }

    /// Handle one input event (donor `input`).
    pub fn input(&mut self, event: &SeatInputEvent, focus: &SeatInputFocus) -> bool {
        if self.death.input(event) {
            return true;
        }
        if matches!(focus, SeatInputFocus::Console | SeatInputFocus::Chat { .. }) {
            return false;
        }
        if self.prompt.input(event) {
            return true;
        }
        let escape = KeyCode::Escape as i32;
        let prompt_escape = match &event.kind {
            SeatInputEventKind::Key { code, down, repeat, .. } => *code == escape && *down && !*repeat,
            SeatInputEventKind::ControllerButton { button, down, .. } => (*button == 1 || *button == 6) && *down,
            SeatInputEventKind::MouseButton { button, down, .. } => *button == 3 && *down,
            _ => false,
        };
        if self.controller.borrow().active_menu().as_ref() == Some(&prompt_menu_id()) && prompt_escape {
            let _ = self.controller.borrow_mut().open_menu(&pause_menu_id());
            self.manual_pause = true;
            return true;
        }
        if self.controller.borrow().active_menu().is_some() {
            let consumed = self.controller.borrow_mut().input(event).unwrap_or(false);
            self.drain_nav();
            return consumed;
        }
        let menu = match &event.kind {
            SeatInputEventKind::Key { code, down, repeat, .. } => *code == escape && *down && !*repeat,
            SeatInputEventKind::ControllerButton { button, down, .. } => *button == 6 && *down,
            _ => false,
        };
        if menu {
            self.weapon_wheel.close(false);
            let _ = self.controller.borrow_mut().open_menu(&pause_menu_id());
            self.manual_pause = true;
            return true;
        }
        self.weapon_wheel.input(event).unwrap_or(false)
    }

    /// Sample input through the wheel (donor `sample`).
    #[must_use]
    pub fn sample(&mut self, input: SeatInputSample) -> SeatInputSample {
        self.weapon_wheel.update();
        let scores = input
            .buttons
            .iter()
            .any(|button| button.action == "scores" && button.active);
        self.source_hud.scores(scores);
        let attack = input
            .buttons
            .iter()
            .any(|button| button.action == "attack" && (button.active || button.pressed));
        let wheel = self.weapon_wheel.command(attack, input.now_milliseconds);
        if !wheel.holster && !wheel.consume_attack && input.now_milliseconds >= self.weapon_wheel.weapon_lock_until() {
            return input;
        }
        SeatInputSample {
            now_milliseconds: input.now_milliseconds,
            buttons: input
                .buttons
                .into_iter()
                .map(|button| {
                    if button.action == "attack" {
                        SeatSampleButton {
                            active: false,
                            pressed: false,
                            fraction: 0.0,
                            ..button
                        }
                    } else {
                        button
                    }
                })
                .collect(),
        }
    }

    /// Open or close the wheel (donor `wheel`).
    pub fn wheel(&mut self, mode: WheelMode, down: bool) {
        if down {
            self.weapon_wheel.open(mode);
        } else {
            self.weapon_wheel.close(true);
        }
    }

    /// Switch the Q1 weapon by slots (donor `switchWeapon`).
    pub fn switch_weapon(&mut self, first: i32, second: i32) -> bool {
        if self.guest_ui {
            return false;
        }
        let player = self.simulation.player_ui(&self.actor);
        let active = player.active_weapon.as_deref().unwrap_or_default();
        if !active.starts_with("q1:") {
            return false;
        }
        if let Some(selected) = self.authored_wheel.borrow().switch_weapon(&player, first, second) {
            (self.command)("use".to_string(), vec![selected]);
        }
        true
    }

    /// Cycle the rerelease wheel (donor `cycleWeapon`).
    pub fn cycle_weapon(&mut self, direction: i32) -> bool {
        if self.guest_ui || self.dialect != CommandDialect::Q2Rerelease {
            return false;
        }
        self.weapon_wheel.cycle(direction);
        true
    }

    /// Close the wheel and all menus (donor `closeMenus`).
    pub fn close_menus(&mut self) {
        self.weapon_wheel.close(false);
        self.controller.borrow_mut().close_all();
        self.manual_pause = false;
    }

    /// Clear the pending prompt (donor `clearPrompt`).
    pub fn clear_prompt(&mut self) {
        self.prompt.clear();
    }

    /// Record a started mixer voice for captions (donor audio observer).
    pub fn note_voice_start(&mut self, event: SoundCaptionVoiceStart) {
        self.sound_captions.note_voice_start(event);
    }

    /// Record a stopped mixer voice for captions (donor audio observer).
    pub fn note_voice_stop(&mut self, voice_id: u64, output_sample: u64) {
        self.sound_captions.note_voice_stop(voice_id, output_sample);
    }

    /// Show center print (donor `centerPrint`).
    pub fn center_print(&mut self, text: &str, time_milliseconds: f64, duration_milliseconds: f64) {
        let _ = self.messages.center_print(
            &self.seat,
            text,
            SourceTime::Milliseconds(time_milliseconds as i32),
            SourceTime::Milliseconds(duration_milliseconds as i32),
            true,
            125.0,
        );
    }

    /// Receive presentation events (donor `receive`).
    pub fn receive(&mut self, events: &[SeatPresentationEvent]) {
        self.prompt.receive(events);
        for source in events {
            match &source.event {
                SeatPresentationPayload::Hud(event) => {
                    self.source_hud.receive(SeatHudSourceEvent {
                        content: UiContentId::new(source.content.as_str()),
                        seconds: source.seconds,
                        event: event.clone(),
                    });
                }
                SeatPresentationPayload::Q2PlayerInventory { actor, visible } => {
                    if actor != &self.actor {
                        continue;
                    }
                    let active = self.controller.borrow().active_menu();
                    if *visible && active.as_ref() != Some(&self.inventory_root) {
                        let root = self.inventory_root.clone();
                        let _ = self.controller.borrow_mut().open_menu(&root);
                    } else if !visible && active.as_ref() == Some(&self.inventory_root) {
                        self.controller.borrow_mut().close_menu();
                    }
                }
                SeatPresentationPayload::Q2PlayerUserinfo { actor, name } => {
                    self.match_ui.match_name(actor, name);
                }
                SeatPresentationPayload::Q2Composition { event } => {
                    self.match_ui.receive(event);
                }
                SeatPresentationPayload::Q1Message { player, center, text } => {
                    if player != &self.actor {
                        continue;
                    }
                    let starts = SourceTime::Seconds(source.seconds as f32);
                    let duration = SourceTime::Seconds(3.0);
                    if *center {
                        let _ = self
                            .messages
                            .center_print(&self.seat, text, starts, duration, true, 125.0);
                    } else {
                        let _ = self.messages.notify(&self.seat, text, false, starts, duration);
                    }
                }
                SeatPresentationPayload::Q2Centerprint {
                    actor,
                    text,
                    duration_seconds,
                    instant,
                } => {
                    if actor != &self.actor {
                        continue;
                    }
                    let _ = self.messages.center_print(
                        &self.seat,
                        text,
                        SourceTime::Seconds(source.seconds as f32),
                        SourceTime::Seconds(duration_seconds.unwrap_or(5.0) as f32),
                        instant.unwrap_or(true),
                        40.0,
                    );
                }
                SeatPresentationPayload::Q3ServerCommand { client, text } => {
                    if *client >= 0 && *client as u32 != self.client.slot() {
                        continue;
                    }
                    let Ok(tokens) = tokenize_command(text, CmdDialect::Q3, TextMode::Source) else {
                        continue;
                    };
                    let mut argv = tokens.argv.into_iter();
                    let (Some(command), Some(body)) = (argv.next(), argv.next()) else {
                        continue;
                    };
                    let starts = SourceTime::Seconds(source.seconds as f32);
                    let duration = SourceTime::Seconds(3.0);
                    if command == "cp" {
                        let _ = self
                            .messages
                            .center_print(&self.seat, &body, starts, duration, true, 125.0);
                    } else if command == "chat" || command == "tchat" {
                        let _ = self.messages.notify(&self.seat, &body, true, starts, duration);
                    }
                }
                SeatPresentationPayload::Q2PlayerPrint { target, text } => {
                    if target.as_ref().is_some_and(|target| target != &self.actor) {
                        continue;
                    }
                    let _ = self.messages.notify(
                        &self.seat,
                        text,
                        true,
                        SourceTime::Seconds(source.seconds as f32),
                        SourceTime::Seconds(3.0),
                    );
                }
                SeatPresentationPayload::Q2Print { actor, text } => {
                    if actor.as_ref().is_some_and(|actor| actor != &self.actor) {
                        continue;
                    }
                    let _ = self.messages.notify(
                        &self.seat,
                        text,
                        true,
                        SourceTime::Seconds(source.seconds as f32),
                        SourceTime::Seconds(3.0),
                    );
                }
            }
        }
    }
}

/// Source-status override for [`ApplicationSeatUi::draw`] (donor
/// `sourceStatus`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UiSeatDrawSource {
    /// Explicit vitals.
    Vitals {
        /// Health.
        health: i32,
        /// Armor.
        armor: i32,
    },
    /// Native status rendering.
    Native,
}

/// Draw options for [`ApplicationSeatUi::draw`] (donor trailing
/// parameters).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiSeatDrawOptions {
    /// Game view visible.
    pub game_visible: bool,
    /// Crosshair visible.
    pub crosshair_visible: bool,
    /// Native status rendering.
    pub native_status: bool,
    /// Show the aggregate arsenal warning.
    pub show_aggregate_warning: bool,
    /// Native crosshair (`None` follows `native_status`).
    pub native_crosshair: Option<bool>,
    /// Source-status override.
    pub source_status: Option<UiSeatDrawSource>,
}

impl Default for UiSeatDrawOptions {
    fn default() -> Self {
        Self {
            game_visible: true,
            crosshair_visible: true,
            native_status: false,
            show_aggregate_warning: true,
            native_crosshair: None,
            source_status: None,
        }
    }
}

/// Component Q2 renderer override (donor `component`).
pub struct Q2HudComponent<'a> {
    /// Component renderer.
    pub renderer: &'a mut dyn SeatQ2NativeHud,
    /// Inventory presentation mode.
    pub mode: Q2HudMode,
}

/// Render services over seat text, art, and caller sinks.
struct SeatRenderServices<'a> {
    text: &'a UiTextRenderer,
    white: ImagePicture,
    pictures: Option<WeaponPictureResolver>,
    extra: Option<DrawPictureResolver>,
    art: &'a NativeUiArt,
    emit: &'a mut dyn FnMut(UiEmitCommand),
    material: &'a mut dyn FnMut(UiMaterialDraw),
}

impl UiRenderServices for SeatRenderServices<'_> {
    fn draw_text(
        &mut self,
        context: &UiDrawContext,
        command: &UiDrawCommand,
        draw: &mut Draw2D,
    ) -> Result<(), ClientError> {
        let UiDrawCommand::Text {
            origin,
            text,
            font,
            scale,
            color,
            align,
            shadow,
        } = command
        else {
            return Ok(());
        };
        let slot = if font == &menu_title_font() {
            super::startup_menu::MENU_TITLE_FONT_SLOT
        } else {
            super::startup_menu::MENU_BODY_FONT_SLOT
        };
        let mapped = match align {
            qa_client::ui::types::TextAlign::Left => LayoutAlign::Left,
            qa_client::ui::types::TextAlign::Center => LayoutAlign::Center,
            qa_client::ui::types::TextAlign::Right => LayoutAlign::Right,
        };
        let text_command = qa_client::text::ui_world::UiTextCommand {
            font: slot,
            text: text.clone(),
            scale: *scale,
            color: *color,
            origin: RectOrigin {
                x: origin.x,
                y: origin.y,
            },
            align: mapped,
            shadow: *shadow,
        };
        self.text.draw(&context.binding.seat, &text_command, draw)?;
        Ok(())
    }

    fn white(&self) -> PictureAsset {
        PictureAsset::Image(self.white)
    }

    fn picture(&self, resource: &ResourceId) -> Result<PictureAsset, ClientError> {
        if let Some(extra) = self.extra.as_ref() {
            if let Some(asset) = extra(resource) {
                return Ok(asset);
            }
        }
        if let Some(pictures) = self.pictures.as_ref() {
            if let Some(asset) = pictures(weapon_load_key(resource)) {
                return Ok(asset);
            }
        }
        Ok(self
            .art
            .picture(resource)
            .map(PictureAsset::Image)
            .unwrap_or_else(|_| self.white()))
    }

    fn emit(&mut self, command: UiEmitCommand) {
        (self.emit)(command);
    }

    fn material(&mut self, draw: UiMaterialDraw) {
        (self.material)(draw);
    }
}

impl<Catalog: SoundMediaCaptions + Clone + 'static> ApplicationSeatUi<Catalog> {
    /// Weapon-status rectangles occluding the game view (donor
    /// `weaponOcclusion`).
    pub fn weapon_occlusion(&self, context: &UiDrawContext, game_visible: bool) -> Result<Vec<Rect>, UiSeatError> {
        if self.guest_ui || !game_visible || !matches!(self.focus.focus(), SeatInputFocus::Game) {
            return Ok(Vec::new());
        }
        let player = self.simulation.player_ui(&self.actor);
        let values = self.preferences.values();
        let skin = hud_skin_font(&self.art.skin, &self.hud_font());
        let cap_height = skin.cap_ink.map_or(8.0, |ink| ink.height);
        Ok(hud_vital_occupied_rects(
            context,
            if player.weapon_status.is_none() { 2 } else { 3 },
            values.hud_scale,
            self.art.skin.font_scale * values.text_scale,
            cap_height,
        )?)
    }

    /// Text measurer for the HUD font.
    fn hud_measure(&self) -> qa_client::ui::hud::weapon::MeasureText {
        let font = self.hud_font();
        Rc::new(move |text, scale| {
            layout_text(&TextLayoutOptions {
                text,
                font: &font,
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
        })
    }

    /// Draw the seat UI (donor `draw`).
    #[allow(clippy::too_many_lines)]
    pub fn draw(
        &mut self,
        context: &UiDrawContext,
        camera: SceneCamera,
        emit: &mut dyn FnMut(UiEmitCommand),
        material: &mut dyn FnMut(UiMaterialDraw),
        options: &UiSeatDrawOptions,
    ) -> Result<(), UiSeatError> {
        let native_crosshair = options.native_crosshair.unwrap_or(options.native_status);
        let source_vitals = match options.source_status {
            Some(UiSeatDrawSource::Vitals { health, armor }) => Some((health, armor)),
            _ => None,
        };
        self.text
            .bind(super::startup_menu::MENU_BODY_FONT_SLOT, self.hud_font());
        self.menu_text
            .bind(super::startup_menu::MENU_BODY_FONT_SLOT, self.menu_font());
        let in_game = matches!(self.focus.focus(), SeatInputFocus::Game);
        if self.guest_ui
            && !matches!(options.source_status, Some(UiSeatDrawSource::Native))
            && options.game_visible
            && in_game
        {
            let status = self.simulation.player_ui(&self.actor).weapon_status;
            if let Some(status) = status {
                let weapon = self.weapon_hud(&status, ArsenalAmmoWarning::None, true);
                let base = empty_hud_data(self.seat.clone());
                let hud = CommonHudData {
                    visible: true,
                    crosshair: HudCrosshair {
                        visible: false,
                        ..base.crosshair
                    },
                    weapon: Some(weapon),
                    ..base
                };
                let skin = hud_skin_font(&self.art.skin, &self.hud_font());
                let draw_options = CommonHudDrawOptions {
                    skin,
                    measure_text: Some(self.hud_measure()),
                    preferences: self.preferences.values(),
                    camera: Some(camera),
                    localize: Rc::new(|text| text.to_string()),
                };
                let commands = draw_common_hud(context, &hud, &draw_options, &mut self.messages)?;
                let mut services = SeatRenderServices {
                    text: &self.text,
                    white: self.art.white,
                    pictures: self.pictures.clone(),
                    extra: None,
                    art: &self.art,
                    emit: &mut *emit,
                    material: &mut *material,
                };
                render_ui_commands(context, &commands, &mut services)?;
            }
        }
        if !self.guest_ui || source_vitals.is_some() {
            let player = self.simulation.player_ui(&self.actor);
            let _ = self.messages.set_source_points(&self.seat, self.source_hud.points());
            let frame = self.source_hud.presentation(context.time_ms as f64);
            let armor = if player.armor.regular_kind == "none" {
                0
            } else {
                player.armor.regular_points
            };
            let clock = (self.caption_clock)();
            let captions = self
                .sound_captions
                .active(
                    clock,
                    &SoundCaptionPreferences {
                        enabled: self.preferences.values().captions,
                    },
                )
                .into_iter()
                .map(|caption| ActiveCaption {
                    cue: CaptionCue {
                        id: caption.source.clone(),
                        kind: CaptionKind::Caption,
                        start_ms: 0.0,
                        duration_ms: 0.0,
                        text: caption.text.clone(),
                        speaker: None,
                        arguments: Vec::new(),
                    },
                    localized_text: caption.text,
                    localized_speaker: None,
                })
                .collect::<Vec<_>>();
            let mut prompts = self.match_ui.prompts();
            prompts.extend(frame.prompts.clone());
            if player.armor.powered_kind != "none" {
                prompts.push(HudPrompt {
                    action: format!("Power {} {}", player.armor.powered_kind, player.armor.powered_cells),
                    binding: String::new(),
                    icon: None,
                });
            }
            let wheel = self.weapon_wheel.draw_state();
            let native_weapon = player.weapon_status.is_none()
                || matches!(options.source_status, Some(UiSeatDrawSource::Native))
                || self.guest_ui
                || options.native_status && player.selected_arsenal;
            let weapon = if native_weapon {
                None
            } else {
                player.weapon_status.as_ref().map(|status| {
                    let warning = if options.show_aggregate_warning {
                        player.arsenal_warning
                    } else {
                        ArsenalAmmoWarning::None
                    };
                    self.weapon_hud(status, warning, options.native_status)
                })
            };
            let vitals = if options.native_status && source_vitals.is_none() {
                Vec::new()
            } else {
                let health = source_vitals.map_or(player.health, |(health, _)| health);
                let armor_value = source_vitals.map_or(armor, |(_, armor)| armor);
                vec![
                    HudValue {
                        label: "Health".to_string(),
                        value: health as f32,
                        icon: None,
                        warning: health <= 25,
                    },
                    HudValue {
                        label: "Armor".to_string(),
                        value: armor_value as f32,
                        icon: None,
                        warning: false,
                    },
                ]
            };
            let base = empty_hud_data(self.seat.clone());
            let hud = CommonHudData {
                powerups: player.powerups.clone(),
                weapon,
                visible: options.game_visible && in_game,
                vitals,
                inventory: frame.inventory.clone(),
                prompts,
                health_bars: frame.health_bars.clone(),
                help: frame.help.clone(),
                captions,
                wheel: wheel.wheel,
                carousel: wheel.carousel,
                crosshair: HudCrosshair {
                    visible: options.crosshair_visible && !native_crosshair,
                    ..base.crosshair
                },
                help_path: frame.help_path,
                damage_indicators: frame.damage_indicators.clone(),
                pickup: frame.pickup.clone(),
                ..base
            };
            let skin = hud_skin_font(&self.art.skin, &self.hud_font());
            let draw_options = CommonHudDrawOptions {
                skin,
                measure_text: Some(self.hud_measure()),
                preferences: self.preferences.values(),
                camera: Some(camera),
                localize: Rc::new(|text| text.to_string()),
            };
            let commands = draw_common_hud(context, &hud, &draw_options, &mut self.messages)?;
            let mut services = SeatRenderServices {
                text: &self.text,
                white: self.art.white,
                pictures: self.pictures.clone(),
                extra: None,
                art: &self.art,
                emit: &mut *emit,
                material: &mut *material,
            };
            render_ui_commands(context, &commands, &mut services)?;
        }
        let panel = menu_panel(context, false);
        let active = self.controller.borrow().active_menu();
        let backdrop = match (&panel, active.as_ref()) {
            (UiDrawCommand::Fill { rect, color }, Some(menu)) if menu == &death_menu_id() => UiDrawCommand::Fill {
                rect: *rect,
                color: Vec4 { w: 0.45, ..*color },
            },
            _ => panel,
        };
        let drawn = if active.is_none() {
            Vec::new()
        } else {
            let mut menu_context = context.clone();
            menu_context.time_ms = (self.now)();
            let mut commands = vec![backdrop];
            commands.extend(self.controller.borrow_mut().draw(&menu_context)?);
            commands
        };
        let mut services = SeatRenderServices {
            text: &self.menu_text,
            white: self.art.white,
            pictures: None,
            extra: None,
            art: &self.art,
            emit: &mut *emit,
            material: &mut *material,
        };
        render_ui_commands(context, &drawn, &mut services)?;
        Ok(())
    }

    /// Weapon panel for a status (donor weapon object literal).
    fn weapon_hud(
        &self,
        status: &SeatWeaponStatus,
        warning: ArsenalAmmoWarning,
        native_status: bool,
    ) -> CommonWeaponHud {
        let icon_key = self
            .weapon_icons
            .weapon
            .as_ref()
            .or(self.weapon_icons.ammo.as_ref())
            .map(weapon_load_key);
        let ammo_key = self.weapon_icons.ammo.as_ref().map(weapon_load_key);
        CommonWeaponHud {
            status: WeaponHudStatus {
                source: ProviderRef {
                    provider: status.source_provider.clone(),
                    content: UiContentId::new(&status.source_content),
                },
                item: ItemId::new(&status.item),
                label: status.label.clone(),
                ammo: status.ammo.clone(),
            },
            warning,
            weapon_icon: self.weapon_icons.weapon.clone(),
            ammo_icon: self.weapon_icons.ammo.clone(),
            icon_aspect: self.weapon_assets.aspect(icon_key) as f32,
            ammo_aspect: self.weapon_assets.aspect(ammo_key) as f32,
            native_status,
            measure_text: Some(self.hud_measure()),
        }
    }

    /// Provider-resolved native arsenal (donor `nativeQ2Arsenal`).
    fn native_q2_arsenal(&self) -> Option<NativeQ2HudArsenal> {
        let player = self.simulation.player_ui(&self.actor);
        let selected = player.native_inventory.as_ref()?;
        if !player.selected_arsenal && player.native_inventory.is_none() {
            return None;
        }
        let mut arsenal = NativeQ2HudArsenal::default();
        if let (Some(prepared), Some(presentation)) =
            (self.native_inventory_item.as_ref(), selected.presentation.as_ref())
        {
            let matches = prepared.item == selected.selected.clone().unwrap_or_default()
                && prepared.source.content == presentation.source.content
                && prepared.source.provider == presentation.source.provider;
            if matches {
                arsenal.selected_item = Some(SelectedItemView {
                    label: prepared.label.clone(),
                    localized_label: prepared.localized_label.clone(),
                    icon: prepared.icon.clone(),
                });
            }
        }
        let rows = selected
            .items
            .iter()
            .enumerate()
            .map(|(index, item)| NativeInventoryItem {
                label: item.label.clone(),
                count: item.count,
                item: index as i32,
            })
            .collect::<Vec<_>>();
        let position = selected
            .selected
            .as_ref()
            .and_then(|id| selected.items.iter().position(|item| &item.item == id))
            .map_or(-1, |index| index as i32);
        arsenal.inventory = Some(NativeInventoryReadout {
            items: rows,
            selected: position,
        });
        if player.selected_arsenal {
            let icon = self.weapon_icons.ammo.as_ref().map(|resource| {
                let key = weapon_load_key(resource);
                ArsenalIcon {
                    resource: resource.clone(),
                    aspect: self.weapon_assets.aspect(Some(key)) as f32,
                }
            });
            arsenal.ammunition = Some(AmmunitionView {
                count: player.ammo_count,
                icon,
            });
        }
        Some(arsenal)
    }

    /// Localizer for native source text (donor `nativeSourceLocalizer`).
    fn native_source_localizer(
        &mut self,
        assets: &mut dyn SeatPrepareAssets<Catalog>,
        content: &ContentId,
    ) -> Result<super::media::rerelease_presentation::RereleaseLocalizer, UiSeatError> {
        if let Some(localizer) = self.native_hud_localizer.clone() {
            let resolved = localizer(content.clone());
            return Ok(Box::new(move |text: &str, args: &[String]| {
                resolved(text.to_string(), args.to_vec())
            }));
        }
        if self.native_hud_localizations.is_none() {
            let provider = assets.rerelease_provider();
            self.native_hud_localizations = Some(ApplicationRereleasePresentation::new(
                provider,
                vec![RereleasePresentationSeat {
                    seat: self.seat.clone(),
                    actor: self.actor.clone(),
                    language: None,
                }],
                self.shared_settings.clone(),
            ));
        }
        let presentation = self
            .native_hud_localizations
            .as_mut()
            .unwrap_or_else(|| panic!("rerelease presentation is initialized above"));
        presentation
            .source_localizer(&self.seat, content)
            .map_err(|error| UiSeatError::Failed(error.to_string()))
    }

    /// Prepare the selected native inventory item (donor
    /// `prepareNativeInventory`).
    fn prepare_native_inventory(&mut self, assets: &mut dyn SeatPrepareAssets<Catalog>) -> Result<(), UiSeatError> {
        self.native_inventory_item = None;
        let player = self.simulation.player_ui(&self.actor);
        let Some(inventory) = player.native_inventory.as_ref() else {
            return Ok(());
        };
        let Some(presentation) = inventory.presentation.as_ref() else {
            return Ok(());
        };
        let Some(selected) = inventory.selected.as_ref() else {
            return Ok(());
        };
        let Some(item) = inventory.items.iter().find(|item| &item.item == selected) else {
            return Ok(());
        };
        let product = assets
            .catalog_product(presentation.source.content.as_str())
            .map_err(UiSeatError::Failed)?;
        let provider = format!(
            "{}:{}",
            presentation.source.provider.namespace, presentation.source.provider.name
        );
        let resolved = if presentation.kind == "item" {
            None
        } else {
            let (_, resolver) = assets.weapon_services();
            resolver.resolve(&provider, presentation.source.content.as_str(), &presentation.weapon)
        };
        let icon = if presentation.kind == "item" {
            presentation.icon.clone()
        } else if presentation.kind == "weapon" {
            resolved
                .as_ref()
                .and_then(|icons| icons.selected_weapon.clone().or_else(|| icons.weapon.clone()))
        } else {
            resolved.as_ref().and_then(|icons| icons.ammo.clone())
        };
        let key = match icon.as_ref() {
            None => None,
            Some(icon) => {
                let (backend, _) = assets.weapon_services();
                Some(self.weapon_assets.load(backend, icon)?)
            }
        };
        let localized_label = if product.edition == "rerelease" {
            self.native_source_localizer(assets, &presentation.source.content)?(&item.label, &[])
        } else {
            item.label.clone()
        };
        self.native_inventory_item = Some(PreparedNativeItem {
            item: item.item.clone(),
            source: presentation.source.clone(),
            label: item.label.clone(),
            localized_label,
            icon: key.as_deref().and_then(|key| {
                weapon_resource_id(key).map(|resource| ArsenalIcon {
                    resource,
                    aspect: self.weapon_assets.aspect(Some(key)) as f32,
                })
            }),
        });
        Ok(())
    }

    /// Prepare native Q2 HUD operations (donor `prepareNativeQ2Hud`).
    pub fn prepare_native_q2_hud(
        &mut self,
        frame: &NativeQ2HudFrame,
        content: &ContentId,
        assets: &mut dyn SeatPrepareAssets<Catalog>,
        context: &UiDrawContext,
        component: Option<Q2HudComponent<'_>>,
    ) -> Result<(), UiSeatError> {
        if component.is_none() {
            self.prepare_native_inventory(assets)?;
        }
        let mut environment = None;
        if frame.protocol.is_rerelease() {
            let use_font = {
                let mut registry = self.hud_cvars.borrow_mut();
                match registry.get("scr_usekfont") {
                    Some(snapshot) => Some(snapshot),
                    None => registry
                        .register("scr_usekfont", "1", 0)
                        .map_err(|error| UiSeatError::Failed(error.to_string()))?,
                }
            };
            let Some(use_font) = use_font else {
                return Err(UiSeatError::Failed(
                    "Could not register rerelease HUD font setting".to_string(),
                ));
            };
            let localize = self.native_source_localizer(assets, content)?;
            let font = self.hud_font();
            let font_scale = 1.25_f32;
            environment = Some(NativeQ2HudEnvironment {
                table: None,
                use_font: use_font.integer_value != 0,
                font_line_height: 8.0 * font_scale,
                measure: Rc::new(move |text| {
                    layout_text(&TextLayoutOptions {
                        text,
                        font: &font,
                        scale: font_scale,
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
                    .map_or((0.0, 0.0), |layout| (layout.width, layout.height))
                }),
                localize: Rc::new(move |text, args| localize(text, args)),
            });
        }
        let scale = self.preferences.values().hud_scale * context.binding.hud_scale;
        let arsenal = if component.is_none() {
            self.native_q2_arsenal()
        } else {
            None
        };
        match component {
            Some(component) => component.renderer.prepare(
                content,
                frame,
                context,
                scale,
                Some(component.mode),
                arsenal.as_ref(),
                environment.as_ref(),
            ),
            None => self.native_q2_hud.prepare(
                content,
                frame,
                context,
                scale,
                None,
                arsenal.as_ref(),
                environment.as_ref(),
            ),
        }
        Ok(())
    }

    /// Draw the native Q2 HUD (donor `drawNativeQ2Hud`).
    pub fn draw_native_q2_hud(
        &mut self,
        frame: &NativeQ2HudFrame,
        context: &UiDrawContext,
        emit: &mut dyn FnMut(UiEmitCommand),
        material: &mut dyn FnMut(UiMaterialDraw),
        binding: Option<&dyn Fn(&str) -> String>,
        component: Option<Q2HudComponent<'_>>,
    ) -> Result<(), UiSeatError> {
        let scale = self.preferences.values().hud_scale * context.binding.hud_scale;
        self.text
            .bind(super::startup_menu::MENU_BODY_FONT_SLOT, self.hud_font());
        let arsenal = if component.is_none() {
            self.native_q2_arsenal()
        } else {
            None
        };
        let commands = match component {
            Some(component) => {
                let mode = component.mode;
                let commands =
                    component
                        .renderer
                        .commands(frame, context, scale, binding, Some(mode), arsenal.as_ref());
                let mut resolved = HashMap::new();
                for command in &commands {
                    if let UiDrawCommand::Image { resource, .. } = command {
                        if let Some(asset) = component.renderer.picture(resource) {
                            resolved.insert(resource.clone(), asset);
                        }
                    }
                }
                (commands, resolved)
            }
            None => {
                let commands = self
                    .native_q2_hud
                    .commands(frame, context, scale, binding, None, arsenal.as_ref());
                let mut resolved = HashMap::new();
                for command in &commands {
                    if let UiDrawCommand::Image { resource, .. } = command {
                        if let Some(asset) = self.native_q2_hud.picture(resource) {
                            resolved.insert(resource.clone(), asset);
                        }
                    }
                }
                (commands, resolved)
            }
        };
        let (commands, resolved) = commands;
        let fallback: DrawPictureResolver = Rc::new(move |resource| resolved.get(resource).copied());
        let mut services = SeatRenderServices {
            text: &self.text,
            white: self.art.white,
            pictures: self.pictures.clone(),
            extra: Some(fallback),
            art: &self.art,
            emit: &mut *emit,
            material: &mut *material,
        };
        render_ui_commands(context, &commands, &mut services)?;
        Ok(())
    }

    /// Caption draw commands (donor `captionCommands`).
    pub fn caption_commands(
        &mut self,
        captions: &[ActiveCaption],
        context: &UiDrawContext,
    ) -> Result<Vec<UiEmitCommand>, UiSeatError> {
        if !self.preferences.values().captions {
            return Ok(Vec::new());
        }
        let mut commands = Vec::new();
        self.menu_text
            .bind(super::startup_menu::MENU_BODY_FONT_SLOT, self.menu_font());
        let area = context.binding.safe_area;
        let rect = Rect {
            x: area.x + 8.0,
            y: area.y + area.height * 0.65,
            width: area.width - 16.0,
            height: area.height * 0.3,
        };
        let values = self.preferences.values();
        let scale = values.text_scale * (area.width / 640.0).min(area.height / 480.0);
        let font = self.menu_font();
        let skin = hud_skin_font(&self.art.skin, &self.menu_font());
        let fallback = qa_client::text::atlas::CapInk { top: 0.0, height: 8.0 };
        let ink = skin.cap_ink.as_ref().unwrap_or(&fallback);
        let draws = caption_commands(
            captions,
            &rect,
            &self.art.skin.font,
            scale,
            &|text, scale| {
                layout_text(&TextLayoutOptions {
                    text,
                    font: &font,
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
            },
            ink,
        );
        let mut services = SeatRenderServices {
            text: &self.menu_text,
            white: self.art.white,
            pictures: None,
            extra: None,
            art: &self.art,
            emit: &mut |command| commands.push(command),
            material: &mut |_| panic!("Caption text requested a material draw"),
        };
        render_ui_commands(context, &draws, &mut services)?;
        Ok(commands)
    }

    /// Release every menu and sink (donor `close`).
    pub fn close(&mut self) {
        self.sound_captions.close();
        {
            let Self {
                base_arena,
                team_arena,
                controller,
                ..
            } = self;
            if let Some(arena) = base_arena.as_mut() {
                arena.close(&mut controller.borrow_mut());
            }
            if let Some(arena) = team_arena.as_mut() {
                arena.close(&mut controller.borrow_mut());
            }
        }
        self.death.close();
        self.prompt.close();
        self.match_ui.close();
        for dispose in self.disposers.drain(..) {
            dispose();
        }
        self.controller.borrow_mut().close_all();
        self.manual_pause = false;
        self.text.clear();
        self.menu_text.clear();
        self.messages.clear();
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use qa_client::input::InputBinding;
    use qa_client::render::scene::resources::SceneImageRegistry;
    use qa_client::render::types::{ImageLevel, ImageSource, ResourceOwner};
    use qa_client::text::atlas::classic_charset;
    use qa_client::ui::common::assets::load_native_ui_art;
    use qa_client::ui::settings::bindings::SeatBindingStore;
    use qa_client::ui::settings::gyro::{GyroCalibrationView, GyroRouteResult, GyroRouter};
    use qa_client::ui::settings::input_routing::{ControllerSelectionView, ReRouter};
    use qa_client::ui::settings::services::RendererBackend;
    use qa_client::ui::settings::{
        GamepadPreview, GamepadTuningView, GyroTuningView, GyroYawAxis, MouseTuningView, StickPreview,
        DEFAULT_GAMEPAD_TUNING,
    };
    use qa_client::ui::types::ContentId as UiContentIdTest;
    use qa_client::ui::types::DopplerSelection as UiDoppler;
    use qa_client::ui::types::EnvironmentSelection as UiEnvironment;
    use qa_client::ui::types::PresentationSelection as UiPresentation;
    use qa_client::ui::types::ProviderRef as UiProviderRef;
    use qa_client::ui::types::SeatPresentationBinding;
    use qa_core::cmd::Dialect;
    use qa_core::identity::IdentityOwner;
    use qa_core::math::Vec2;
    use qa_core::math::{identity_mat4, vec3};

    use super::super::sound_captions::SoundActiveCaption;
    use super::super::weapon_hud::{HudIconSet, WeaponHudMount, WeaponHudPicture};

    use super::*;

    #[derive(Debug, Clone, Default)]
    struct TestCatalog {
        captions: Vec<SoundActiveCaption>,
    }

    impl SoundMediaCaptions for TestCatalog {
        fn active(
            &self,
            _source: &str,
            _source_time_milliseconds: f64,
            _paused: bool,
            _preferences: &SoundCaptionPreferences,
        ) -> Vec<SoundActiveCaption> {
            self.captions.clone()
        }

        fn clear(&mut self) {
            self.captions.clear();
        }
    }

    struct FakeSimulation {
        player: Rc<RefCell<SeatPlayerUi>>,
    }

    impl SeatSimulationUi for FakeSimulation {
        fn player_ui(&self, _actor: &ActorId) -> SeatPlayerUi {
            self.player.borrow().clone()
        }
    }

    struct FakeAudio {
        effects: Cell<f32>,
        music: Cell<f32>,
        outputs: RefCell<Vec<String>>,
    }

    impl SeatHudAudio for FakeAudio {
        fn ui_sound(&self, _sound: UiSound, _owner: &SeatId) {}
        fn effects_volume(&self) -> f32 {
            self.effects.get()
        }
        fn set_effects_volume(&self, value: f32) {
            self.effects.set(value);
        }
        fn music_volume(&self) -> f32 {
            self.music.get()
        }
        fn set_music_volume(&self, value: f32) {
            self.music.set(value);
        }
        fn output_format(&self) -> qa_client::ui::settings::AudioOutputFormat {
            qa_client::ui::settings::AudioOutputFormat {
                sample_rate: 44100,
                sample_bits: 16,
                channels: 2,
            }
        }
        fn select_output_format(&self, _format: &qa_client::ui::settings::AudioOutputFormat) {}
        fn selected_output(&self) -> Option<String> {
            None
        }
        fn output_device_names(&self) -> Vec<String> {
            self.outputs.borrow().clone()
        }
        fn select_output(&self, _name: Option<&str>) {}
    }

    struct FakeFocus {
        focus: RefCell<SeatInputFocus>,
    }

    impl SeatFocus for FakeFocus {
        fn bindings(&self) -> Vec<InputBinding> {
            Vec::new()
        }
        fn focus(&self) -> SeatInputFocus {
            self.focus.borrow().clone()
        }
        fn focused(&self) -> bool {
            matches!(*self.focus.borrow(), SeatInputFocus::Game)
        }
        fn set_focus(&self, focus: SeatInputFocus, _time_ms: i64) {
            *self.focus.borrow_mut() = focus;
        }
        fn set_impulse(&self, _value: i32) {}
    }

    struct FakeHaptics {
        enabled: Cell<bool>,
        strength: Cell<f32>,
    }

    impl SeatHaptics for FakeHaptics {
        fn enabled(&self) -> bool {
            self.enabled.get()
        }
        fn set_enabled(&self, enabled: bool) {
            self.enabled.set(enabled);
        }
        fn strength(&self) -> f32 {
            self.strength.get()
        }
        fn set_strength(&self, strength: f32) {
            self.strength.set(strength);
        }
        fn set_active(&self, _active: bool) {}
    }

    struct FakeConsole {
        prints: RefCell<Vec<String>>,
    }

    impl SeatConsole for FakeConsole {
        fn print(&self, text: &str) {
            self.prints.borrow_mut().push(text.to_string());
        }
        fn toggle(&self) {}
    }

    struct FakePrompt {
        received: Rc<RefCell<usize>>,
    }

    impl SeatPromptUi for FakePrompt {
        fn prepare(&mut self, _focus: Rc<dyn Fn() -> SeatInputFocus>) {}
        fn clear(&mut self) {}
        fn input(&mut self, _event: &SeatInputEvent) -> bool {
            false
        }
        fn receive(&mut self, events: &[SeatPresentationEvent]) {
            *self.received.borrow_mut() += events.len();
        }
        fn close(&mut self) {}
    }

    struct FakeDeath {
        health: Rc<Cell<i32>>,
        active: bool,
    }

    impl SeatDeathUi for FakeDeath {
        fn observe(&mut self, health: i32) {
            self.health.set(health);
        }

        fn input(&mut self, _event: &SeatInputEvent) -> bool {
            false
        }
        fn active(&self) -> bool {
            self.active
        }
        fn close(&mut self) {}
    }

    struct FakeMatch {
        names: Rc<RefCell<Vec<String>>>,
    }

    impl SeatMatchUi for FakeMatch {
        fn match_name(&mut self, _actor: &ActorId, name: &str) {
            self.names.borrow_mut().push(name.to_string());
        }
        fn receive(&mut self, _event: &SeatCtfEvent) {}
        fn prompts(&self) -> Vec<HudPrompt> {
            Vec::new()
        }
        fn close(&mut self) {}
    }

    struct FakeNativeQ2;

    impl SeatQ2NativeHud for FakeNativeQ2 {
        fn prepare(
            &mut self,
            _content: &ContentId,
            _frame: &NativeQ2HudFrame,
            _context: &UiDrawContext,
            _scale: f32,
            _mode: Option<Q2HudMode>,
            _arsenal: Option<&NativeQ2HudArsenal>,
            _environment: Option<&NativeQ2HudEnvironment>,
        ) {
        }
        fn commands(
            &self,
            _frame: &NativeQ2HudFrame,
            _context: &UiDrawContext,
            _scale: f32,
            _binding: Option<&dyn Fn(&str) -> String>,
            _mode: Option<Q2HudMode>,
            _arsenal: Option<&NativeQ2HudArsenal>,
        ) -> Vec<UiDrawCommand> {
            Vec::new()
        }
        fn picture(&self, _resource: &ResourceId) -> Option<PictureAsset> {
            None
        }
        fn clear(&mut self) {}
    }

    struct FakeAuthored;

    impl SeatAuthoredWheel for FakeAuthored {
        fn prepare(
            &mut self,
            _resolver: &mut dyn HudIconResolver,
            _status: Option<&HudStatusRef>,
            _player: &SeatPlayerUi,
        ) {
        }
        fn items(&self, _player: &SeatPlayerUi) -> Option<Vec<SeatHudItem>> {
            None
        }
        fn switch_weapon(&self, _player: &SeatPlayerUi, _first: i32, _second: i32) -> Option<String> {
            None
        }
    }

    struct FakeIcons;

    impl SeatHudIcons for FakeIcons {
        fn load_image(&mut self, _content: &UiContentId, _path: &str) -> ResourceId {
            ResourceId::new("resource:test:image").unwrap()
        }
        fn picture_size(&self, _image: &ResourceId) -> Option<(f32, f32)> {
            Some((8.0, 8.0))
        }
        fn palette(&mut self, _content: &UiContentId) -> Option<Vec<u8>> {
            None
        }
    }

    struct FakeRouter {
        seat: SeatId,
    }

    impl ReRouter for FakeRouter {
        fn inputs(&self) -> Vec<SeatId> {
            vec![self.seat.clone()]
        }
        fn keyboard_seat(&self) -> Option<SeatId> {
            Some(self.seat.clone())
        }
        fn set_keyboard_seat(&mut self, _seat: Option<SeatId>) {}
        fn update_capture(&mut self) {}
        fn controller_selection(&self, _seat: SeatId) -> ControllerSelectionView {
            ControllerSelectionView::Automatic
        }
        fn set_controller_selection(&mut self, _seat: SeatId, _selection: ControllerSelectionView) {}
        fn controller_for(&self, _seat: SeatId) -> Option<i32> {
            None
        }
    }

    struct FakeWindow;

    impl WindowView for FakeWindow {
        fn logical_size(&self) -> (u32, u32) {
            (640, 480)
        }
        fn display_modes(&self) -> Vec<(u32, u32)> {
            vec![(640, 480)]
        }
        fn fullscreen(&self) -> bool {
            false
        }
        fn is_gl(&self) -> bool {
            false
        }
        fn set_size(&mut self, _width: u32, _height: u32) -> Result<(), String> {
            Ok(())
        }
        fn set_fullscreen(&mut self, _fullscreen: bool) -> Result<(), String> {
            Ok(())
        }
    }

    struct FakeTuning;

    impl InputTuningHost for FakeTuning {
        fn mouse_tuning(&self) -> MouseTuningView {
            MouseTuningView {
                sensitivity: 3.0,
                acceleration: 0.0,
                filter: false,
                yaw: 1.0,
                pitch: 1.0,
                side: 1.0,
                forward: 1.0,
                free_look: true,
                look_spring: false,
                look_strafe: false,
                invert_pitch: false,
            }
        }
        fn set_mouse_tuning(&self, _tuning: &MouseTuningView) {}
        fn always_run(&self) -> bool {
            false
        }
        fn set_always_run(&self, _value: bool) {}
        fn builder_dialect(&self) -> CommandDialect {
            CommandDialect::Q1Netquake
        }
        fn gamepad_tuning(&self) -> GamepadTuningView {
            DEFAULT_GAMEPAD_TUNING
        }
        fn set_gamepad_tuning(&self, _tuning: &GamepadTuningView) {}
        fn gamepad_preview(&self) -> GamepadPreview {
            let zero = StickPreview {
                raw: Vec2 { x: 0.0, y: 0.0 },
                curved: Vec2 { x: 0.0, y: 0.0 },
            };
            GamepadPreview {
                move_stick: zero,
                look_stick: zero,
            }
        }
    }

    struct FakeGyro;

    impl GyroRouter for FakeGyro {
        fn gyro_tuning(&self, _seat: SeatId) -> GyroTuningView {
            GyroTuningView {
                enabled: false,
                yaw_sensitivity: 1.0,
                pitch_sensitivity: 1.0,
                yaw_axis: GyroYawAxis::Y,
            }
        }
        fn set_gyro_tuning(&mut self, _seat: SeatId, _tuning: GyroTuningView) {}
        fn set_gyro_enabled(&mut self, _seat: SeatId, _enabled: bool) -> GyroRouteResult {
            GyroRouteResult::Accepted
        }
        fn calibration(&self, _seat: SeatId) -> GyroCalibrationView {
            GyroCalibrationView::Idle
        }
        fn begin_calibration(&mut self, _seat: SeatId) -> GyroRouteResult {
            GyroRouteResult::Accepted
        }
        fn cancel_calibration(&mut self, _seat: SeatId) {}
        fn reset_calibration(&mut self, _seat: SeatId) {}
    }

    struct FakeBackend;

    impl WeaponHudBackend for FakeBackend {
        fn family(&mut self, _content: &str) -> qa_content::contract::GameFamily {
            qa_content::contract::GameFamily::Q1
        }
        fn palette(&mut self, _content: &str) -> Option<Vec<u8>> {
            None
        }
        fn open_mount(&mut self, _content: &str, _path: &str) -> Result<Option<WeaponHudMount>, WeaponHudError> {
            Ok(None)
        }
        fn register_shader_picture(&mut self, _content: &str, _name: &str) -> Result<WeaponHudPicture, WeaponHudError> {
            Ok(WeaponHudPicture::Image {
                name: "shader".to_string(),
                width: 8,
                height: 8,
            })
        }
        fn load_texture(
            &mut self,
            _content: &str,
            _path: &str,
            _request: super::super::weapon_hud::HudTextureRequest,
        ) -> Result<Option<super::super::weapon_hud::WeaponHudTexture>, WeaponHudError> {
            Ok(None)
        }
        fn register_indexed_image(
            &mut self,
            _key: &str,
            _image: &qa_content::images::palette::IndexedRenderImage,
            _resource: &str,
        ) -> Result<(), WeaponHudError> {
            Ok(())
        }
    }

    struct FakeResolver;

    impl HudIconResolver for FakeResolver {
        fn resolve(&mut self, _source_provider: &str, _source_content: &str, _item: &str) -> Option<HudIconSet> {
            None
        }
    }

    struct FakeProvider;

    impl RereleasePresentationProvider for FakeProvider {
        fn family(&self, _content: &ContentId) -> Option<qa_content::contract::GameFamily> {
            None
        }
        fn list_localization_files(&self, _content: &ContentId) -> Vec<String> {
            Vec::new()
        }
        fn open_localization(&self, _content: &ContentId, _path: &str) -> Option<Vec<u8>> {
            None
        }
        fn load_sky_face(&self, _content: &ContentId, _path: &str) -> qa_client::render::types::RendererImage {
            let authority = IdentityOwner::create("ui-test-sky").unwrap();
            qa_client::render::types::RendererImage {
                owner: ResourceOwner::new(1, authority.session().clone(), 0),
                ordinal: 0,
                source: ImageSource::Generated {
                    name: "sky".to_string(),
                },
                width: 8,
                height: 8,
            }
        }
    }

    type TestCaptionLoader = Box<dyn FnMut(&SoundCaptionReference, &str) -> TestCatalog>;

    struct FakeAssets {
        backend: FakeBackend,
        resolver: FakeResolver,
        load: TestCaptionLoader,
    }

    impl SeatPrepareAssets<TestCatalog> for FakeAssets {
        fn weapon_services(&mut self) -> (&mut dyn WeaponHudBackend, &mut dyn HudIconResolver) {
            (&mut self.backend, &mut self.resolver)
        }
        fn caption_loader(&mut self) -> &mut dyn FnMut(&SoundCaptionReference, &str) -> TestCatalog {
            &mut self.load
        }
        fn rerelease_provider(&self) -> Rc<dyn RereleasePresentationProvider> {
            Rc::new(FakeProvider)
        }
        fn catalog_product(&self, _id: &str) -> Result<qa_content::catalog::ProductExpectation, String> {
            Ok(qa_content::catalog::ProductExpectation {
                id: "test".to_string(),
                family: qa_content::contract::GameFamily::Q2,
                edition: "classic".to_string(),
                campaign: "baseq2".to_string(),
                title: "Test".to_string(),
                content_directory: "baseq2".to_string(),
                base_product: None,
                required_content_archives: Vec::new(),
                required_programs: Vec::new(),
                map_witness: None,
                unresolved_reason: None,
            })
        }
    }

    fn player() -> SeatPlayerUi {
        SeatPlayerUi {
            health: 100,
            armor: SeatArmor {
                regular_kind: "none".to_string(),
                regular_points: 0,
                powered_kind: "none".to_string(),
                powered_cells: 0,
            },
            powerups: Vec::new(),
            items: Vec::new(),
            active_weapon: None,
            weapon_status: None,
            arsenal_warning: ArsenalAmmoWarning::None,
            selected_arsenal: false,
            native_inventory: None,
            ammo_count: None,
        }
    }

    fn art() -> NativeUiArt {
        let authority = IdentityOwner::create("ui-art-test").unwrap();
        let mut images = SceneImageRegistry::new(ResourceOwner::new(1, authority.session().clone(), 0));
        let font = ResourceId::new("resource:test:font").unwrap();
        let solid = |width: u32, height: u32| ImageLevel {
            width,
            height,
            pixels: vec![9; (width * height * 4) as usize],
        };
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

    struct TestHandles {
        actor: ActorId,
        seat: SeatId,
        client: ClientId,
        player: Rc<RefCell<SeatPlayerUi>>,
        prompt_received: Rc<RefCell<usize>>,
        match_names: Rc<RefCell<Vec<String>>>,
        death_health: Rc<Cell<i32>>,
    }

    fn seat_ui(dialect: CommandDialect, guest_ui: bool) -> (ApplicationSeatUi<TestCatalog>, TestHandles) {
        let authority = IdentityOwner::create("ui-seat-test").unwrap();
        let actor = authority.actor(0, 0);
        let seat = authority.seat(0);
        let client = authority.client(0, 0);
        let player = Rc::new(RefCell::new(player()));
        let prompt_received = Rc::new(RefCell::new(0_usize));
        let match_names = Rc::new(RefCell::new(Vec::new()));
        let death_health = Rc::new(Cell::new(-1));
        let context_session = authority.session().clone();
        let context_seat = seat.clone();
        let context_client = client.clone();
        let ui = ApplicationSeatUi::new(UiSeatOptions {
            actor: actor.clone(),
            seat: seat.clone(),
            client: client.clone(),
            seat_index: 0,
            art: art(),
            font: font(),
            typography: MenuTypography {
                body: font(),
                title: font(),
            },
            simulation: Rc::new(FakeSimulation {
                player: Rc::clone(&player),
            }),
            command: Rc::new(|_, _| {}),
            quit: Rc::new(|| {}),
            audio: Rc::new(FakeAudio {
                effects: Cell::new(0.8),
                music: Cell::new(0.7),
                outputs: RefCell::new(Vec::new()),
            }),
            focus: Rc::new(FakeFocus {
                focus: RefCell::new(SeatInputFocus::Game),
            }),
            haptics: Rc::new(FakeHaptics {
                enabled: Cell::new(true),
                strength: Cell::new(1.0),
            }),
            console: Rc::new(FakeConsole {
                prints: RefCell::new(Vec::new()),
            }),
            dialect,
            now: Rc::new(|| 0),
            shared_settings: None,
            router: Rc::new(RefCell::new(FakeRouter { seat: seat.clone() })),
            router_devices: Rc::new(Vec::new),
            update_capture: Rc::new(|| {}),
            binding_store: Rc::new(RefCell::new(SeatBindingStore::new(seat.clone()))),
            binding_capabilities: Rc::new(|| BindingCapabilities {
                chat: false,
                score_command: None,
                offhand_grapple: false,
                offhand_grenades: false,
            }),
            can_reset_bindings: Rc::new(|_| false),
            reset_bindings: Rc::new(|_| {}),
            window: Rc::new(RefCell::new(FakeWindow)),
            renderer_backend: Rc::new(|| RendererBackend::Cpu),
            renderer_worker: None,
            apply_renderer: Rc::new(|_| {}),
            commands: Rc::new(RefCell::new(
                qa_core::cmd_buffer::CommandBuffer::new(
                    Dialect::Q3,
                    qa_core::cmd_buffer::CommandContext::new(
                        context_session.clone(),
                        qa_core::cmd_buffer::CommandOrigin::LocalConsole,
                    ),
                    qa_core::cmd_buffer::BufferOptions::new(),
                )
                .unwrap(),
            )),
            command_context: Rc::new(move || {
                qa_core::cmd_buffer::CommandContext::new(
                    context_session.clone(),
                    qa_core::cmd_buffer::CommandOrigin::LocalSeat {
                        seat: context_seat.clone(),
                        client: context_client.clone(),
                    },
                )
            }),
            gyro: GyroSettingsUi {
                router: Rc::new(RefCell::new(FakeGyro)),
                device: Rc::new(|| None),
                busy: Rc::new(|| false),
                message: Rc::new(String::new),
                save: Rc::new(|| Ok(())),
            },
            device_bindings: Vec::new(),
            tuning_host: Rc::new(RefCell::new(FakeTuning)),
            vibration: true,
            local_count: Rc::new(|| 1),
            local_capacity: 4,
            can_remove_local: Rc::new(|_| false),
            attach: Rc::new(|_| Box::new(|| {})),
            hud_cvars: Rc::new(RefCell::new(CvarRegistry::new(Dialect::Q2Classic))),
            caption_clock: Rc::new(|| VoiceClockSample {
                output_sample: 0,
                paused: false,
            }),
            icons: Rc::new(RefCell::new(FakeIcons)),
            prompt: Box::new(FakePrompt {
                received: Rc::clone(&prompt_received),
            }),
            death: Box::new(FakeDeath {
                health: Rc::clone(&death_health),
                active: false,
            }),
            match_ui: Box::new(FakeMatch {
                names: Rc::clone(&match_names),
            }),
            native_q2_hud: Box::new(FakeNativeQ2),
            authored_wheel: Rc::new(RefCell::new(FakeAuthored)),
            base_arena: None,
            team_arena: None,
            host_settings: None,
            language: None,
            saves: None,
            view_setting: None,
            llm: None,
            guest_ui,
            gameplay: None,
            lobby: None,
            rankings: None,
            native_hud_localizer: None,
            localize: None,
            source_actor: RemoteActorId {
                session: 1,
                slot: 0,
                generation: 0,
            },
            pictures: None,
            catalog: std::marker::PhantomData,
        })
        .unwrap();
        (
            ui,
            TestHandles {
                actor,
                seat,
                client,
                player,
                prompt_received,
                match_names,
                death_health,
            },
        )
    }

    fn assets() -> FakeAssets {
        FakeAssets {
            backend: FakeBackend,
            resolver: FakeResolver,
            load: Box::new(|_, _| TestCatalog::default()),
        }
    }

    fn draw_context(seat: &SeatId, client: &ClientId) -> UiDrawContext {
        let viewport = Rect {
            x: 0.0,
            y: 0.0,
            width: 640.0,
            height: 480.0,
        };
        UiDrawContext {
            binding: SeatPresentationBinding {
                seat: seat.clone(),
                client: client.clone(),
                viewport,
                safe_area: viewport,
                hud_scale: 1.0,
                presentation: UiPresentation {
                    doppler: UiDoppler::Disabled,
                    environment: UiEnvironment::Disabled,
                    assets: UiContentIdTest::new("assets"),
                    hud: UiProviderRef {
                        provider: "hud".to_string(),
                        content: UiContentIdTest::new("hud"),
                    },
                    effects: UiProviderRef {
                        provider: "fx".to_string(),
                        content: UiContentIdTest::new("fx"),
                    },
                    audio: UiProviderRef {
                        provider: "audio".to_string(),
                        content: UiContentIdTest::new("audio"),
                    },
                },
            },
            time_ms: 0,
        }
    }

    fn camera() -> SceneCamera {
        SceneCamera {
            origin: vec3(0.0, 0.0, 0.0),
            axis: [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)],
            projection: identity_mat4(),
            viewport: qa_client::view::Rect {
                x: 0,
                y: 0,
                width: 640,
                height: 480,
            },
            clip: qa_client::view::CameraClip::None,
        }
    }

    fn escape(seat: &SeatId) -> SeatInputEvent {
        SeatInputEvent {
            seat: seat.clone(),
            time_ms: 0,
            kind: SeatInputEventKind::Key {
                code: KeyCode::Escape as i32,
                down: true,
                repeat: false,
            },
        }
    }

    #[test]
    fn builds_and_registers_pause_menu() {
        let (ui, _) = seat_ui(CommandDialect::Q3, false);
        assert!(ui.controller.borrow().is_registered(&pause_menu_id()));
        assert!(ui
            .controller
            .borrow()
            .is_registered(&UiMenuId::new("menu:settings:root").unwrap()));
        assert!(!ui.pause_menu_open());
    }

    #[test]
    fn escape_opens_pause_menu_and_close_menus_clears() {
        let (mut ui, handles) = seat_ui(CommandDialect::Q3, false);
        assert!(ui.input(&escape(&handles.seat), &SeatInputFocus::Game));
        assert_eq!(ui.controller.borrow().active_menu().as_ref(), Some(&pause_menu_id()));
        assert!(ui.pause_menu_open());
        ui.close_menus();
        assert_eq!(ui.controller.borrow().active_menu(), None);
        assert!(!ui.pause_menu_open());
    }

    #[test]
    fn wheel_gates_on_items() {
        let (mut ui, handles) = seat_ui(CommandDialect::Q3, false);
        ui.wheel(WheelMode::Weapons, true);
        assert!(!ui.weapon_wheel().is_open());
        handles.player.borrow_mut().items.push(SeatHudItem {
            kind: "weapon".to_string(),
            id: "q1:axe".to_string(),
            source_ordinal: 0,
            label: "Axe".to_string(),
            owned: true,
        });
        ui.wheel(WheelMode::Weapons, true);
        assert!(ui.weapon_wheel().is_open());
        ui.wheel(WheelMode::Weapons, false);
        assert!(!ui.weapon_wheel().is_open());
    }

    #[test]
    fn q1_message_and_q3_cp_routing() {
        let (mut ui, handles) = seat_ui(CommandDialect::Q3, false);
        let content = ContentId("test:content".to_string());
        ui.receive(&[
            SeatPresentationEvent {
                content: content.clone(),
                seconds: 1.0,
                event: SeatPresentationPayload::Q1Message {
                    player: handles.actor.clone(),
                    center: false,
                    text: "hello".to_string(),
                },
            },
            SeatPresentationEvent {
                content,
                seconds: 2.0,
                event: SeatPresentationPayload::Q3ServerCommand {
                    client: -1,
                    text: "cp \"center\"".to_string(),
                },
            },
        ]);
        assert_eq!(*handles.prompt_received.borrow(), 2);
        let state = ui.messages.active(2_500);
        assert!(state.notifications.iter().any(|notice| notice.text == "hello"));
        assert_eq!(
            state.center_print.as_ref().map(|print| print.text.clone()),
            Some("center".to_string())
        );
    }

    #[test]
    fn q2_inventory_toggles_menu() {
        let (mut ui, handles) = seat_ui(CommandDialect::Q3, false);
        let root = ui.inventory_root.clone();
        ui.receive(&[SeatPresentationEvent {
            content: ContentId("test:content".to_string()),
            seconds: 0.0,
            event: SeatPresentationPayload::Q2PlayerInventory {
                actor: handles.actor.clone(),
                visible: true,
            },
        }]);
        assert_eq!(ui.controller.borrow().active_menu().as_ref(), Some(&root));
        ui.receive(&[SeatPresentationEvent {
            content: ContentId("test:content".to_string()),
            seconds: 0.0,
            event: SeatPresentationPayload::Q2PlayerInventory {
                actor: handles.actor.clone(),
                visible: false,
            },
        }]);
        assert_eq!(ui.controller.borrow().active_menu(), None);
    }

    #[test]
    fn userinfo_records_name() {
        let (mut ui, handles) = seat_ui(CommandDialect::Q3, false);
        ui.receive(&[SeatPresentationEvent {
            content: ContentId("test:content".to_string()),
            seconds: 0.0,
            event: SeatPresentationPayload::Q2PlayerUserinfo {
                actor: handles.actor.clone(),
                name: "Ranger".to_string(),
            },
        }]);
        assert_eq!(*handles.match_names.borrow(), vec!["Ranger".to_string()]);
    }

    #[test]
    fn sample_passes_through_when_closed() {
        let (mut ui, _) = seat_ui(CommandDialect::Q3, false);
        let input = SeatInputSample {
            now_milliseconds: 100,
            buttons: vec![SeatSampleButton {
                action: "attack".to_string(),
                active: true,
                pressed: true,
                fraction: 1.0,
            }],
        };
        let sampled = ui.sample(input);
        assert!(sampled.buttons[0].active);
    }

    #[test]
    fn cycle_weapon_gates_on_dialect() {
        let (mut ui, _) = seat_ui(CommandDialect::Q3, false);
        assert!(!ui.cycle_weapon(1));
        let (mut ui, _) = seat_ui(CommandDialect::Q2Rerelease, false);
        assert!(ui.cycle_weapon(1));
        let (mut ui, _) = seat_ui(CommandDialect::Q2Rerelease, true);
        assert!(!ui.cycle_weapon(1));
    }

    #[test]
    fn switch_weapon_gates() {
        let (mut ui, _) = seat_ui(CommandDialect::Q3, true);
        assert!(!ui.switch_weapon(1, 2));
        let (mut ui, handles) = seat_ui(CommandDialect::Q3, false);
        assert!(!ui.switch_weapon(1, 2));
        handles.player.borrow_mut().active_weapon = Some("q1:axe".to_string());
        assert!(ui.switch_weapon(1, 2));
    }

    #[test]
    fn prepare_observes_death_and_runs() {
        let (mut ui, handles) = seat_ui(CommandDialect::Q3, false);
        handles.player.borrow_mut().health = 42;
        ui.prepare(&mut assets()).unwrap();
        assert_eq!(handles.death_health.get(), 42);
        let (mut ui, _) = seat_ui(CommandDialect::Q3, true);
        ui.prepare(&mut assets()).unwrap();
    }

    #[test]
    fn draw_and_captions_smoke() {
        let (mut ui, handles) = seat_ui(CommandDialect::Q3, false);
        let context = draw_context(&handles.seat, &handles.client);
        let mut emits = Vec::new();
        let mut materials = Vec::new();
        ui.draw(
            &context,
            camera(),
            &mut |command| emits.push(command),
            &mut |draw| materials.push(draw),
            &UiSeatDrawOptions::default(),
        )
        .unwrap();
        let captions = ui.caption_commands(&[], &context).unwrap();
        assert!(captions
            .iter()
            .all(|command| matches!(command, UiEmitCommand::SetColor(_))));
        let cue = ActiveCaption {
            cue: CaptionCue {
                id: "cue".to_string(),
                kind: CaptionKind::Caption,
                start_ms: 0.0,
                duration_ms: 1000.0,
                text: "caption".to_string(),
                speaker: None,
                arguments: Vec::new(),
            },
            localized_text: "caption".to_string(),
            localized_speaker: None,
        };
        let drawn = ui.caption_commands(&[cue], &context).unwrap();
        assert!(drawn
            .iter()
            .any(|command| matches!(command, UiEmitCommand::StretchPic { .. })));
        let occlusion = ui.weapon_occlusion(&context, true).unwrap();
        assert!(!occlusion.is_empty());
    }

    #[test]
    fn close_unregisters_and_clears() {
        let (mut ui, handles) = seat_ui(CommandDialect::Q3, false);
        ui.center_print("bye", 0.0, 1000.0);
        ui.close();
        assert!(!ui.controller.borrow().is_registered(&pause_menu_id()));
        assert!(ui.messages.active(0).notifications.is_empty());
        assert!(ui.messages.active(0).center_print.is_none());
        let _ = handles;
    }
}
