//! Startup game-selection model: draft choices plus launch resolution.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/startup-selection.ts`
//! (`StartupSelectionModel`, `createStartupSelection`, and the row/choice types).
//! The draft stores UI choices; catalog queries reuse `qa_content`, binding
//! items reuse `qa_client`, and Team Arena planning reuses
//! [`super::team_arena_skirmish`]. Sync port: the donor's async discovery,
//! map scans, and launch resolution become sync calls over the sync
//! `qa_content` APIs and `std::fs`. Unported siblings arrive through the one
//! [`StartupSelectionCollaborators`] seam: mod discovery/application
//! (`./mod-selection.ts`), arena selection (`./base-arena-selection.ts`), Q3
//! catalog preparation (`./q3-product.ts`), QVM grapple selection
//! (`./qvm-grapple-selection.ts`), and the launch preset/player products
//! (`./content.ts`, `applicationPreset`/`applicationPlayerProducts`). The tiny
//! match-rules donor (`./match-modes.ts`) is ported inline. `ApplicationOptions`
//! on this lane has no `serverProfile`/`q3Product`/`teamArenaSkirmish`/
//! `remoteContent` fields, so the profile arrives via
//! [`StartupSelectionModel::select_server_profile`], and the Q3 product plus
//! skirmish ride on [`StartupLaunch`].

use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;

use qa_client::input::weapons::base_weapon_binding_items;
use qa_client::input::weapons::WeaponBindingItem;
use qa_client::input::weapons::WeaponFamily;
use qa_client::ui::mods::menu::ModMenuRow;
use qa_client::ui::mods::menu::ModMenuService;
use qa_client::ui::settings::action_catalog::BindingCapabilities;
use qa_client::ui::settings::action_catalog::ScoreCommand;
use qa_content::archive::open_archive;
use qa_content::archive::read_entry;
use qa_content::archive::ArchiveEntry;
use qa_content::archive::EntryRef;
use qa_content::archive::FileSource;
use qa_content::bsp::parse_q1_entities;
use qa_content::bsp::q1_entity_value;
use qa_content::bsp3::parse_q3_entities;
use qa_content::catalog::canonical_weapon_source;
use qa_content::catalog::default_monster_roster;
use qa_content::catalog::disabled_equipment;
use qa_content::catalog::discover_installed_content;
use qa_content::catalog::equipment_providers;
use qa_content::catalog::expected_products;
use qa_content::catalog::grapple_styles;
use qa_content::catalog::native_equipment;
use qa_content::catalog::native_provider_timing;
use qa_content::catalog::offhand_grenade_source;
use qa_content::catalog::preset_choice;
use qa_content::catalog::resolve_launch;
use qa_content::catalog::source_program_product;
use qa_content::catalog::supports_selected_weapon_product;
use qa_content::catalog::CatalogError;
use qa_content::catalog::CatalogProduct;
use qa_content::catalog::DiscoverContentOptions;
use qa_content::catalog::GrappleStyle;
use qa_content::catalog::InstalledCatalog;
use qa_content::catalog::LaunchPreset;
use qa_content::catalog::LaunchQvmCompatibility;
use qa_content::catalog::LaunchWeaponSources;
use qa_content::catalog::ProductAvailability;
use qa_content::catalog::ResolveLaunchOptions;
use qa_content::classify_bsp;
use qa_content::contract::mod_selection_key;
use qa_content::contract::read_mod_selection;
use qa_content::contract::CampaignSelection;
use qa_content::contract::CharacterSelection;
use qa_content::contract::ContentId;
use qa_content::contract::ContractError;
use qa_content::contract::DopplerSelection;
use qa_content::contract::EnemySelection;
use qa_content::contract::EnvironmentSelection;
use qa_content::contract::EquipmentSelection;
use qa_content::contract::ExecutableRecipe;
use qa_content::contract::GameFamily;
use qa_content::contract::GrappleBinding;
use qa_content::contract::GrappleMechanicDetail;
use qa_content::contract::GrappleSelection;
use qa_content::contract::HandGrenadeSelection;
use qa_content::contract::LaunchSelection;
use qa_content::contract::MapSelection;
use qa_content::contract::ModAvailability;
use qa_content::contract::ModDescription;
use qa_content::contract::ModPurpose;
use qa_content::contract::ModSelection;
use qa_content::contract::MonsterDefinitionReference;
use qa_content::contract::MonsterSelectionTarget;
use qa_content::contract::ProviderReference;
use qa_content::contract::ResourceRequest;
use qa_content::contract::SourceEdition;
use qa_content::mods::ModSelectionSet;
use qa_content::mods::ModsError;
use qa_content::monsters::campaign_monster_slots;
use qa_content::monsters::monster_sources;
use qa_content::monsters::provider_text;
use qa_content::monsters::MonsterError;
use qa_content::monsters::MonsterFamily;
use qa_content::mounts::MountPreparationScope;
use qa_content::q2::foundation::fields::parse_q2_entities;
use qa_content::q2::foundation::host::Q2Edition;
use qa_content::q3::foundation::animation_config::CommonParseCursor;
use qa_content::q3::foundation::animation_config::CommonParseState;
use qa_content::user_data::default_user_content_root;
use qa_content::BspKind;
use qa_core::identity::ProviderId;
use qa_core::time::ClockProfile;
use qa_net::protocol::ProtocolIdentity;
use thiserror::Error;

use super::content::content_family;
use super::startup_source::StartupQ3Product;
use super::team_arena_skirmish::plan_team_arena_skirmish;
use super::team_arena_skirmish::TeamArenaCampaign;
use super::team_arena_skirmish::TeamArenaSkirmish;
use super::team_arena_skirmish::TeamArenaSkirmishError;
use super::team_arena_skirmish::TeamArenaTeams;
use super::team_arena_skirmish::DEFAULT_SKIRMISH_CURSOR;
use crate::options::ApplicationOptions;
use crate::options::GameFamily as OptionsFamily;
use crate::options::GameMode;
use crate::options::MatchRules;
use crate::options::Network;
use crate::options::Renderer;
use crate::persistence::mods::ModSelection as AppModSelection;

/// Startup selection failure (donor `Error`/`RangeError` texts preserved).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StartupSelectionError {
    /// Donor failure text.
    #[error("{0}")]
    Failed(String),
    /// Invalid display settings.
    #[error("Invalid display settings")]
    InvalidDisplay,
    /// Invalid hosting port.
    #[error("Port must be a whole number from 1 to 65535.")]
    InvalidPort,
    /// Invalid startup settings.
    #[error("Invalid startup settings")]
    InvalidSettings,
}

impl From<CatalogError> for StartupSelectionError {
    fn from(error: CatalogError) -> Self {
        Self::Failed(error.to_string())
    }
}

impl From<ContractError> for StartupSelectionError {
    fn from(error: ContractError) -> Self {
        Self::Failed(error.to_string())
    }
}

impl From<ModsError> for StartupSelectionError {
    fn from(error: ModsError) -> Self {
        Self::Failed(error.to_string())
    }
}

impl From<MonsterError> for StartupSelectionError {
    fn from(error: MonsterError) -> Self {
        Self::Failed(error.to_string())
    }
}

impl From<TeamArenaSkirmishError> for StartupSelectionError {
    fn from(error: TeamArenaSkirmishError) -> Self {
        Self::Failed(error.to_string())
    }
}

impl From<std::io::Error> for StartupSelectionError {
    fn from(error: std::io::Error) -> Self {
        Self::Failed(error.to_string())
    }
}

/// Draft field (donor `StartupSelectionField`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StartupSelectionField {
    /// Doppler selection.
    Doppler,
    /// Environment selection.
    Environment,
    /// Game/mod product.
    Product,
    /// Map content product.
    MapProduct,
    /// Starting map.
    Map,
    /// Movement product.
    Movement,
    /// Character product.
    Character,
    /// Character model.
    Model,
    /// Weapon source product.
    Weapons,
    /// Monster roster.
    Enemies,
    /// Hook placement.
    Grapple,
    /// Hook style.
    GrappleStyle,
    /// Offhand grenades.
    Grenades,
    /// Game mode.
    Mode,
    /// Match rules.
    Rules,
    /// Difficulty.
    Skill,
    /// Local seats.
    Seats,
    /// Renderer.
    Renderer,
}

impl StartupSelectionField {
    /// Donor field id.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Doppler => "doppler",
            Self::Environment => "environment",
            Self::Product => "product",
            Self::MapProduct => "mapProduct",
            Self::Map => "map",
            Self::Movement => "movement",
            Self::Character => "character",
            Self::Model => "model",
            Self::Weapons => "weapons",
            Self::Enemies => "enemies",
            Self::Grapple => "grapple",
            Self::GrappleStyle => "grappleStyle",
            Self::Grenades => "grenades",
            Self::Mode => "mode",
            Self::Rules => "rules",
            Self::Skill => "skill",
            Self::Seats => "seats",
            Self::Renderer => "renderer",
        }
    }
}

/// One selectable choice (donor `StartupSelectionChoice`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupSelectionChoice {
    /// Choice id.
    pub id: String,
    /// Display label.
    pub label: String,
    /// Unavailability reason, if any.
    pub unavailable: Option<String>,
}

/// One draft row (donor `StartupSelectionRow`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupSelectionRow {
    /// Draft field.
    pub id: StartupSelectionField,
    /// Display label.
    pub label: String,
    /// Selected choice id.
    pub value: String,
    /// Available choices.
    pub choices: Vec<StartupSelectionChoice>,
}

/// One monster roster row (donor `MonsterRosterRow`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonsterRosterRow {
    /// Entity classname (`None` selects the unmatched default).
    pub classname: Option<String>,
    /// Display label.
    pub label: String,
    /// Selected choice id.
    pub value: String,
    /// Effective replacement label.
    pub effective_label: String,
    /// Available choices.
    pub choices: Vec<StartupSelectionChoice>,
}

/// Native campaign preset (donor `StartupNativePreset`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupNativePreset {
    /// Product id.
    pub id: String,
    /// Display label.
    pub label: String,
    /// Unavailability reason, if any.
    pub unavailable: Option<String>,
    /// Game family.
    pub family: GameFamily,
    /// Edition.
    pub edition: String,
    /// Difficulty choices.
    pub difficulties: Vec<StartupSelectionChoice>,
    /// Default difficulty id.
    pub default_skill: String,
}

/// Resolved launch (donor `StartupLaunch`, plus the Q3 product and skirmish
/// that have no [`ApplicationOptions`] fields on this lane).
#[derive(Debug, Clone, PartialEq)]
pub struct StartupLaunch {
    /// Resolved options.
    pub options: ApplicationOptions,
    /// Resolved recipe.
    pub recipe: ExecutableRecipe,
    /// Q3 product policy (donor `options.q3Product`).
    pub q3_product: Option<StartupQ3Product>,
    /// Planned Team Arena skirmish (donor `options.teamArenaSkirmish`).
    pub team_arena_skirmish: Option<TeamArenaSkirmish>,
}

/// Hosting kind (donor `StartupHosting["kind"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupHostingKind {
    /// Local only.
    Offline,
    /// Native game clients.
    NativeServer,
    /// Mixed-game clients.
    UnifiedServer,
}

impl StartupHostingKind {
    /// Donor spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Offline => "offline",
            Self::NativeServer => "native-server",
            Self::UnifiedServer => "unified-server",
        }
    }
}

/// Hosting selection (donor `StartupHosting`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartupHosting {
    /// Hosting kind.
    pub kind: StartupHostingKind,
    /// Bind port.
    pub port: u32,
    /// NetQuake host protocol, when the game is classic Quake.
    pub q1_protocol: Option<ProtocolIdentity>,
}

/// One base arena (donor `BaseArena` in `./base-arena-catalog.ts`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseArenaInfo {
    /// Arena number.
    pub number: u32,
    /// Arena map path.
    pub map: String,
    /// Arena title.
    pub title: String,
    /// Opponent bot names.
    pub bots: Vec<String>,
    /// Special marker (`training`/`final`/empty).
    pub special: String,
    /// Selection order.
    pub selection: u32,
    /// Frag limit.
    pub frag_limit: u32,
    /// Time limit in minutes.
    pub time_limit: u32,
}

/// One arena tier (donor `ArenaSelection["tiers"]` row).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArenaTier {
    /// Tier id.
    pub id: String,
    /// Tier label.
    pub label: String,
}

/// One base-arena row (donor `ArenaSelectionRow`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupArenaRow {
    /// Arena.
    pub arena: BaseArenaInfo,
    /// Tier id.
    pub tier: String,
    /// Whether the arena is unlocked.
    pub available: bool,
    /// Best-run record text.
    pub record: String,
}

/// Base-arena selection (donor `ArenaSelection`, via `./base-arena-selection.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StartupArenaSelection {
    /// Tiers.
    pub tiers: Vec<ArenaTier>,
    /// Arena rows.
    pub rows: Vec<StartupArenaRow>,
    /// Current arena map, if any.
    pub current: Option<String>,
}

/// Default movement/character products (donor `applicationPlayerProducts`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupPlayerProducts {
    /// Movement product id.
    pub movement: String,
    /// Character product id.
    pub character: String,
}

/// Prepared Q3 catalog (donor `prepareQ3ApplicationProduct` result).
#[derive(Debug, Clone)]
pub struct PreparedQ3Catalog {
    /// Catalog after Q3 preparation.
    pub catalog: InstalledCatalog,
    /// Q3 product policy, if any.
    pub q3_product: Option<StartupQ3Product>,
}

/// Prepared Team Arena metadata (donor `loadTeamArenaCampaign` result).
#[derive(Debug, Clone)]
pub struct PreparedTeamArena {
    /// Loaded campaign.
    pub campaign: TeamArenaCampaign,
    /// Catalog after preparation.
    pub catalog: InstalledCatalog,
    /// Q3 product policy.
    pub q3_product: Option<StartupQ3Product>,
}

/// QVM grapple style (donor `applicationQvmGrappleSelection` result).
#[derive(Debug, Clone, PartialEq)]
pub struct QvmGrappleStyle {
    /// Style id.
    pub id: String,
    /// Style title.
    pub title: String,
    /// Enabled grapple selection.
    pub selection: GrappleSelection,
}

/// Unported startup-selection collaborators (one seam: `./mod-selection.ts`,
/// `./base-arena-selection.ts`, `./q3-product.ts`,
/// `./qvm-grapple-selection.ts`, and `./content.ts`).
pub trait StartupSelectionCollaborators {
    /// Duplicate the collaborators for a refreshed model.
    fn duplicate(&self) -> Box<dyn StartupSelectionCollaborators>;
    /// Mod descriptions (donor `applicationModChoices`).
    fn mod_choices(&self, catalog: &InstalledCatalog) -> Result<Vec<ModDescription>, String>;
    /// Apply enabled mods to a recipe (donor `applyApplicationMods`).
    fn apply_mods(
        &self,
        recipe: ExecutableRecipe,
        choices: &[ModDescription],
        mods: &[ModSelection],
    ) -> Result<ExecutableRecipe, String>;
    /// Base-arena selection (donor `readArenaSelection`).
    fn read_arena_selection(
        &self,
        catalog: &InstalledCatalog,
        options: &ApplicationOptions,
    ) -> Result<StartupArenaSelection, String>;
    /// Prepare the Q3 catalog (donor `prepareQ3ApplicationProduct`).
    fn prepare_q3_product(
        &self,
        catalog: &InstalledCatalog,
        product_id: &str,
        initial: &ApplicationOptions,
    ) -> Result<PreparedQ3Catalog, String>;
    /// Load Team Arena metadata (donor `loadTeamArenaCampaign`, via the real
    /// [`super::team_arena_skirmish`] loader).
    fn load_team_arena(
        &self,
        catalog: &InstalledCatalog,
        initial: &ApplicationOptions,
    ) -> Result<Option<PreparedTeamArena>, String>;
    /// QVM grapple selection for an installed Q3 product (donor
    /// `applicationQvmGrappleSelection`).
    fn qvm_grapple_selection(
        &self,
        catalog: &InstalledCatalog,
        product_id: &str,
        mounts: &MountPreparationScope,
    ) -> Result<Option<QvmGrappleStyle>, String>;
    /// Default movement/character products (donor `applicationPlayerProducts`).
    fn player_products(
        &self,
        catalog: &InstalledCatalog,
        product: &str,
        movement: GameFamily,
        movement_product: Option<&str>,
        character: GameFamily,
        network: &Network,
    ) -> Result<StartupPlayerProducts, String>;
    /// Launch preset (donor `applicationPreset`).
    fn application_preset(
        &self,
        catalog: &InstalledCatalog,
        options: &ApplicationOptions,
        movement: Option<&ProviderReference>,
        character: Option<&ProviderReference>,
    ) -> Result<LaunchPreset, String>;
    /// Launch weapon sources for [`resolve_launch`](qa_content::catalog::resolve_launch).
    fn launch_weapons(&self) -> Box<dyn LaunchWeaponSources>;
    /// Launch QVM compatibility for [`resolve_launch`](qa_content::catalog::resolve_launch).
    fn launch_compat(&self) -> Box<dyn LaunchQvmCompatibility>;
}

fn choice(id: &str, label: &str, unavailable: Option<String>) -> StartupSelectionChoice {
    StartupSelectionChoice {
        id: id.to_string(),
        label: label.to_string(),
        unavailable,
    }
}

fn plain_choice(id: &str) -> StartupSelectionChoice {
    choice(id, id, None)
}

fn unavailable(product: &CatalogProduct) -> Option<String> {
    match &product.availability {
        ProductAvailability::Installed => None,
        ProductAvailability::Unresolved { reason } => Some(reason.clone()),
        ProductAvailability::Missing { requirements } => Some(format!("Missing: {}", requirements.join(", "))),
    }
}

fn product_choice(product: &CatalogProduct) -> StartupSelectionChoice {
    choice(
        &product.expectation.id,
        &format!("{} ({})", product.expectation.title, product.expectation.edition),
        unavailable(product),
    )
}

fn monster_label(classname: &str) -> String {
    match classname {
        "monster_army" => "Grunt".to_string(),
        "monster_demon1" => "Fiend".to_string(),
        "monster_wizard" => "Scrag".to_string(),
        "monster_shalrath" => "Vore".to_string(),
        "monster_tarbaby" => "Spawn".to_string(),
        _ => classname
            .strip_prefix("monster_")
            .unwrap_or(classname)
            .split('_')
            .map(|word| {
                let mut chars = word.chars();
                match chars.next() {
                    None => String::new(),
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
}

fn monster_program_suffix(program: &str) -> String {
    if program == "id1" || program == "baseq2" {
        String::new()
    } else {
        format!(" {program}")
    }
}

fn monster_source_title(family: &str, program: &str, title: Option<&str>) -> String {
    if program == "id1" || program == "baseq2" {
        if family == "q1" {
            "Quake".to_string()
        } else {
            "Quake II".to_string()
        }
    } else {
        title.unwrap_or(program).to_string()
    }
}

fn edition_str(edition: SourceEdition) -> &'static str {
    match edition {
        SourceEdition::Classic => "classic",
        SourceEdition::Rerelease => "rerelease",
    }
}

fn options_family(family: GameFamily) -> OptionsFamily {
    match family {
        GameFamily::Q1 => OptionsFamily::Q1,
        GameFamily::Q2 => OptionsFamily::Q2,
        GameFamily::Q3 => OptionsFamily::Q3,
    }
}

fn contract_family(family: OptionsFamily) -> GameFamily {
    match family {
        OptionsFamily::Q1 => GameFamily::Q1,
        OptionsFamily::Q2 => GameFamily::Q2,
        OptionsFamily::Q3 => GameFamily::Q3,
    }
}

fn weapon_family(family: GameFamily) -> WeaponFamily {
    match family {
        GameFamily::Q1 => WeaponFamily::Q1,
        GameFamily::Q2 => WeaponFamily::Q2,
        GameFamily::Q3 => WeaponFamily::Q3,
    }
}

fn provider_id(text: &str) -> ProviderId {
    match text.split_once(':') {
        Some((namespace, name)) => ProviderId::new(namespace, name),
        None => ProviderId::new("", text),
    }
}

fn to_contract(selection: &AppModSelection) -> ModSelection {
    ModSelection {
        product: selection.product.clone(),
        id: selection.id.clone(),
    }
}

fn to_app(selection: &ModSelection) -> AppModSelection {
    AppModSelection {
        product: selection.product.clone(),
        id: selection.id.clone(),
    }
}

fn provider_reference(provider: &str, content: &str) -> ProviderReference {
    ProviderReference {
        provider: provider_id(provider),
        content: ContentId(content.to_string()),
    }
}

/// Match-mode selection (donor `MatchModeSelection` in `./match-modes.ts`).
struct MatchModeSelection<'a> {
    family: &'a str,
    edition: &'a str,
    campaign: &'a str,
    mode: &'a str,
    rules: &'a str,
}

/// Match-rules availability, ported inline from `./match-modes.ts`
/// (`matchModeUnavailable`).
fn match_mode_unavailable(selection: &MatchModeSelection) -> Option<String> {
    let MatchModeSelection {
        family,
        edition,
        campaign,
        mode,
        rules,
    } = *selection;
    if rules == "standard" {
        return None;
    }
    if rules == "horde" {
        if family != "q1" || edition != "rerelease" || (campaign != "mg1" && campaign != "dopa") {
            return Some(
                "Horde requires Quake rerelease Dimension of the Machine or Dimension of the Past".to_string(),
            );
        }
        return if mode == "deathmatch" {
            Some("Horde requires single player or cooperative mode".to_string())
        } else {
            None
        };
    }
    if family != "q2" {
        return Some("These match rules require a Quake II source game".to_string());
    }
    if mode != "deathmatch" {
        return Some("These match rules require deathmatch mode".to_string());
    }
    if rules == "ctf" || rules == "lmctf" {
        return if edition != "classic" {
            Some("This CTF ruleset requires classic Quake II".to_string())
        } else {
            None
        };
    }
    if edition != "rerelease" && campaign != "rogue" {
        return Some("Tag and DeathBall require Ground Zero or Quake II rerelease".to_string());
    }
    None
}

/// Map/rules availability, ported inline from `./match-modes.ts`
/// (`matchMapUnavailable`).
fn match_map_unavailable(selection: &MatchModeSelection, classnames: &[String]) -> Option<String> {
    if let Some(reason) = match_mode_unavailable(selection) {
        return Some(reason);
    }
    let classes: HashSet<&str> = classnames.iter().map(String::as_str).collect();
    if selection.rules == "deathball" {
        let missing: Vec<&str> = [
            "dm_dball_ball",
            "dm_dball_ball_start",
            "dm_dball_goal",
            "dm_dball_team1_start",
            "dm_dball_team2_start",
        ]
        .into_iter()
        .filter(|name| !classes.contains(name))
        .collect();
        if !missing.is_empty() {
            return Some(format!("DeathBall map is missing: {}", missing.join(", ")));
        }
    }
    if selection.rules == "horde"
        && (!classes.contains("horde_manager") || !classes.iter().any(|name| name.starts_with("info_monster_start")))
    {
        return Some("Horde requires an authored horde_manager and monster spawn points".to_string());
    }
    if selection.rules == "ctf" || selection.rules == "lmctf" {
        let missing: Vec<&str> = ["item_flag_team1", "item_flag_team2"]
            .into_iter()
            .filter(|name| !classes.contains(name))
            .collect();
        if !missing.is_empty() {
            return Some(format!("CTF map is missing: {}", missing.join(", ")));
        }
    }
    if selection.mode == "deathmatch"
        && selection.rules != "deathball"
        && !classes.contains("info_player_deathmatch")
        && !(selection.family == "q3"
            && classes.contains("team_CTF_redplayer")
            && classes.contains("team_CTF_blueplayer"))
    {
        return Some("Deathmatch requires an authored player spawn".to_string());
    }
    None
}

fn mode_str(mode: GameMode) -> &'static str {
    match mode {
        GameMode::Singleplayer => "singleplayer",
        GameMode::Coop => "coop",
        GameMode::Deathmatch => "deathmatch",
    }
}

fn rules_str(rules: MatchRules) -> &'static str {
    match rules {
        MatchRules::Standard => "standard",
        MatchRules::Ctf => "ctf",
        MatchRules::Lmctf => "lmctf",
        MatchRules::Tag => "tag",
        MatchRules::Deathball => "deathball",
        MatchRules::Horde => "horde",
    }
}

fn renderer_str(renderer: Renderer) -> &'static str {
    match renderer {
        Renderer::Gl => "gl",
        Renderer::Cpu => "cpu",
    }
}

/// Per-product monster roster draft.
#[derive(Debug, Clone, Default)]
struct MonsterRoster {
    source: String,
    default: String,
    by_classname: HashMap<String, String>,
}

/// Display draft (donor `display`).
#[derive(Debug, Clone, Copy)]
struct DisplayDraft {
    width: u32,
    height: u32,
    gamma: f64,
}

/// The draft stores UI choices; the launch resolver remains the recipe
/// authority (donor `StartupSelectionModel`). Sync port: every donor `async`
/// method is sync here.
pub struct StartupSelectionModel {
    collaborators: Box<dyn StartupSelectionCollaborators>,
    current_catalog: InstalledCatalog,
    initial: ApplicationOptions,
    q3_product: Option<StartupQ3Product>,
    arena_selection: Option<StartupArenaSelection>,
    team_arena_campaign: Option<TeamArenaCampaign>,
    selected_teams: TeamArenaTeams,
    selected_server_profile: Option<Option<String>>,
    values: HashMap<StartupSelectionField, String>,
    mod_choices: Vec<ModDescription>,
    qvm_hook_styles: Vec<GrappleStyle>,
    selected_mods: Option<ModSelectionSet>,
    mod_status: String,
    refreshing_mods: bool,
    display: DisplayDraft,
    display_overrides_consumed: bool,
    playable_maps: HashMap<String, Vec<StartupSelectionChoice>>,
    authored_default_maps: HashMap<String, String>,
    loose_models: HashMap<String, Vec<String>>,
    eligible_maps: HashMap<String, Vec<StartupSelectionChoice>>,
    map_classnames: HashMap<String, Vec<String>>,
    monster_classes: HashMap<String, HashMap<String, u32>>,
    rosters: HashMap<String, MonsterRoster>,
    selected_models: HashMap<String, String>,
    model_choices: HashMap<String, Vec<StartupSelectionChoice>>,
}

impl StartupSelectionModel {
    /// Build a model over a catalog and initial options.
    pub fn new(
        current_catalog: InstalledCatalog,
        initial: ApplicationOptions,
        collaborators: Box<dyn StartupSelectionCollaborators>,
    ) -> Result<Self, StartupSelectionError> {
        let product = current_catalog.product(&initial.product)?;
        let campaign = product.expectation.campaign.clone();
        let family = product.expectation.family;
        let player_products = collaborators
            .player_products(
                &current_catalog,
                &initial.product,
                content_family(initial.movement),
                initial.movement_product.as_deref(),
                content_family(initial.character),
                &initial.network,
            )
            .map_err(StartupSelectionError::Failed)?;
        let rules = match initial.rules {
            Some(rules) => rules_str(rules).to_string(),
            None if family == GameFamily::Q2
                && product.expectation.edition == "classic"
                && (campaign == "ctf" || campaign == "lmctf") =>
            {
                campaign
            }
            None => "standard".to_string(),
        };
        let mut values = HashMap::new();
        values.insert(StartupSelectionField::Product, initial.product.clone());
        values.insert(
            StartupSelectionField::MapProduct,
            initial.map_product.clone().unwrap_or_else(|| initial.product.clone()),
        );
        values.insert(StartupSelectionField::Map, initial.map.clone());
        values.insert(StartupSelectionField::Movement, player_products.movement);
        values.insert(StartupSelectionField::Character, player_products.character);
        values.insert(StartupSelectionField::Model, initial.character_model.clone());
        values.insert(StartupSelectionField::Doppler, "source".to_string());
        values.insert(StartupSelectionField::Environment, "audio-content".to_string());
        values.insert(StartupSelectionField::Weapons, "native".to_string());
        values.insert(StartupSelectionField::Enemies, "native".to_string());
        values.insert(StartupSelectionField::Grapple, "native".to_string());
        values.insert(StartupSelectionField::GrappleStyle, "native".to_string());
        values.insert(StartupSelectionField::Grenades, "native".to_string());
        values.insert(StartupSelectionField::Mode, mode_str(initial.mode).to_string());
        values.insert(StartupSelectionField::Rules, rules);
        values.insert(StartupSelectionField::Skill, initial.skill.to_string());
        values.insert(StartupSelectionField::Seats, initial.seats.to_string());
        values.insert(
            StartupSelectionField::Renderer,
            renderer_str(initial.renderer).to_string(),
        );
        let mut selected_models = HashMap::new();
        selected_models.insert(
            values[&StartupSelectionField::Character].clone(),
            initial.character_model.clone(),
        );
        Ok(Self {
            collaborators,
            current_catalog,
            display: DisplayDraft {
                width: initial.width,
                height: initial.height,
                gamma: initial.gamma,
            },
            initial,
            q3_product: None,
            arena_selection: None,
            team_arena_campaign: None,
            selected_teams: TeamArenaTeams {
                player: "pagans".to_string(),
                opponent: "stroggs".to_string(),
            },
            selected_server_profile: None,
            values,
            mod_choices: Vec::new(),
            qvm_hook_styles: Vec::new(),
            selected_mods: None,
            mod_status: String::new(),
            refreshing_mods: false,
            display_overrides_consumed: false,
            playable_maps: HashMap::new(),
            authored_default_maps: HashMap::new(),
            loose_models: HashMap::new(),
            eligible_maps: HashMap::new(),
            map_classnames: HashMap::new(),
            monster_classes: HashMap::new(),
            rosters: HashMap::new(),
            selected_models,
            model_choices: HashMap::new(),
        })
    }

    fn value(&self, field: StartupSelectionField) -> &str {
        self.values.get(&field).map_or("", String::as_str)
    }

    /// Current catalog.
    #[must_use]
    pub fn catalog(&self) -> &InstalledCatalog {
        &self.current_catalog
    }

    /// Current base-arena selection, if refreshed.
    #[must_use]
    pub fn base_arenas(&self) -> Option<&StartupArenaSelection> {
        self.arena_selection.as_ref()
    }

    /// Refresh the base-arena selection from the `q3-baseq3` preset.
    pub fn refresh_base_arenas(&mut self) -> Result<(), StartupSelectionError> {
        let launch = self.resolve_preset("q3-baseq3", None, None)?;
        let catalog = self.current_catalog.clone();
        self.arena_selection = Some(
            self.collaborators
                .read_arena_selection(&catalog, &launch.options)
                .map_err(StartupSelectionError::Failed)?,
        );
        Ok(())
    }

    /// Authored Team Arena team choices (donor `teamArena.choices`).
    #[must_use]
    pub fn team_arena_choices(&self) -> Vec<StartupSelectionChoice> {
        match &self.team_arena_campaign {
            None => Vec::new(),
            Some(campaign) => campaign
                .teams
                .iter()
                .map(|team| {
                    let mut chars = team.name.chars();
                    let label = match chars.next() {
                        None => String::new(),
                        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                    };
                    choice(&team.name, &label, None)
                })
                .collect(),
        }
    }

    /// Selected Team Arena teams (donor `teamArena.read`).
    #[must_use]
    pub fn team_arena_read(&self) -> &TeamArenaTeams {
        &self.selected_teams
    }

    /// Select a Team Arena team (donor `teamArena.write`).
    pub fn team_arena_write(&mut self, side: &str, team: &str) -> Result<(), StartupSelectionError> {
        let name = team.to_lowercase();
        let known = self
            .team_arena_campaign
            .as_ref()
            .is_some_and(|campaign| campaign.teams.iter().any(|row| row.name == name));
        if !known {
            return Err(StartupSelectionError::Failed(
                "Unknown authored Team Arena team".to_string(),
            ));
        }
        if side == "player" {
            self.selected_teams.player = name;
        } else {
            self.selected_teams.opponent = name;
        }
        Ok(())
    }

    /// Select a server profile path (`None` clears the selection).
    pub fn select_server_profile(&mut self, path: Option<String>) {
        self.selected_server_profile = Some(path);
    }

    /// Current hosting selection.
    pub fn hosting(&self) -> Result<StartupHosting, StartupSelectionError> {
        let product = self.product(StartupSelectionField::Product)?.expectation.clone();
        let q1_protocol = if product.family == GameFamily::Q1 && product.edition != "quakeworld" {
            Some(self.initial.q1_protocol.unwrap_or(ProtocolIdentity::Q1Netquake))
        } else {
            None
        };
        match &self.initial.network {
            Network::NativeServer { port, .. } | Network::Q2Server { port, .. } => Ok(StartupHosting {
                kind: StartupHostingKind::NativeServer,
                port: u32::from(*port),
                q1_protocol,
            }),
            Network::UnifiedServer { port, .. } => Ok(StartupHosting {
                kind: StartupHostingKind::UnifiedServer,
                port: u32::from(*port),
                q1_protocol,
            }),
            _ => {
                let port = if product.family == GameFamily::Q3 {
                    27960
                } else if product.family == GameFamily::Q2 {
                    if crate::options::live_q2_protocol(&self.initial, product.edition == "rerelease")
                        == ProtocolIdentity::Q2Kex
                    {
                        5069
                    } else {
                        27910
                    }
                } else if product.edition == "quakeworld" {
                    27500
                } else {
                    26000
                };
                Ok(StartupHosting {
                    kind: StartupHostingKind::Offline,
                    port,
                    q1_protocol,
                })
            }
        }
    }

    /// Apply a hosting selection.
    pub fn set_hosting(&mut self, value: StartupHosting) -> Result<(), StartupSelectionError> {
        if value.port < 1 || value.port > 65535 {
            return Err(StartupSelectionError::InvalidPort);
        }
        if value.kind != StartupHostingKind::Offline && self.value(StartupSelectionField::Mode) == "singleplayer" {
            let family = self.product(StartupSelectionField::Product)?.expectation.family;
            self.select(
                StartupSelectionField::Mode,
                if family == GameFamily::Q3 { "deathmatch" } else { "coop" },
            )?;
        }
        let host = match &self.initial.network {
            Network::NativeServer { host, .. }
            | Network::Q2Server { host, .. }
            | Network::UnifiedServer { host, .. } => host.clone(),
            _ => "0.0.0.0".to_string(),
        };
        let keep_transport = self.initial.network_transport.clone();
        let keep_q1 = value.kind == StartupHostingKind::NativeServer
            && self.hosting()?.q1_protocol.is_some()
            && value.q1_protocol.is_some();
        let port = value.port as u16;
        self.initial.network = match value.kind {
            StartupHostingKind::Offline => Network::Offline,
            StartupHostingKind::NativeServer => Network::NativeServer { host, port },
            StartupHostingKind::UnifiedServer => Network::UnifiedServer { host, port },
        };
        self.initial.q1_protocol = if keep_q1 { value.q1_protocol } else { None };
        self.initial.network_transport = if value.kind == StartupHostingKind::NativeServer {
            keep_transport
        } else {
            None
        };
        Ok(())
    }

    fn apply_selected_server_profile(&self, options: &mut ApplicationOptions) {
        if let Some(path) = &self.selected_server_profile {
            options.server_profile_path = path.clone();
        }
    }

    fn prepare_q3_catalog(&mut self) -> Result<(), StartupSelectionError> {
        let preferred = self.current_catalog.products.iter().find(|product| {
            product.expectation.id == self.initial.product && product.expectation.family == GameFamily::Q3
        });
        let fallback = self.current_catalog.products.iter().find(|product| {
            product.expectation.family == GameFamily::Q3 && product.availability == ProductAvailability::Installed
        });
        let Some(product) = preferred.or(fallback) else {
            return Ok(());
        };
        let id = product.expectation.id.clone();
        let catalog = self.current_catalog.clone();
        let prepared = self
            .collaborators
            .prepare_q3_product(&catalog, &id, &self.initial)
            .map_err(StartupSelectionError::Failed)?;
        self.current_catalog = prepared.catalog;
        if prepared.q3_product.is_some() {
            self.q3_product = prepared.q3_product;
        }
        Ok(())
    }

    fn prepare_team_arena(&mut self) -> Result<(), StartupSelectionError> {
        if self.team_arena_campaign.is_some() {
            return Ok(());
        }
        let installed = self
            .current_catalog
            .products
            .iter()
            .find(|product| product.expectation.id == "q3-missionpack")
            .is_some_and(|product| product.availability == ProductAvailability::Installed);
        if !installed {
            return Ok(());
        }
        let catalog = self.current_catalog.clone();
        let prepared = self
            .collaborators
            .load_team_arena(&catalog, &self.initial)
            .map_err(StartupSelectionError::Failed)?;
        if let Some(prepared) = prepared {
            self.team_arena_campaign = Some(prepared.campaign);
            self.current_catalog = prepared.catalog;
            if prepared.q3_product.is_some() {
                self.q3_product = prepared.q3_product;
            }
        }
        Ok(())
    }

    fn hook_styles(&self) -> Vec<GrappleStyle> {
        let product = self.product(StartupSelectionField::Product);
        let mut styles = match product {
            Ok(product) => grapple_styles(&self.current_catalog, product),
            Err(_) => Vec::new(),
        };
        styles.extend(self.qvm_hook_styles.iter().cloned());
        styles
    }

    fn prepare_hook_styles(&mut self, catalog: &InstalledCatalog) -> Result<(), StartupSelectionError> {
        let mounts = MountPreparationScope::new();
        let mut styles: HashMap<String, GrappleStyle> = HashMap::new();
        for product in catalog.products.iter().filter(|product| {
            product.expectation.family == GameFamily::Q3 && product.availability == ProductAvailability::Installed
        }) {
            let selection = self
                .collaborators
                .qvm_grapple_selection(catalog, &product.expectation.id, &mounts)
                .map_err(StartupSelectionError::Failed)?;
            if let Some(selection) = selection {
                if !styles.contains_key(&selection.id) {
                    styles.insert(
                        selection.id.clone(),
                        GrappleStyle {
                            id: selection.id,
                            title: selection.title,
                            selection: selection.selection,
                            unavailable: None,
                        },
                    );
                }
            }
        }
        self.qvm_hook_styles = styles.into_values().collect();
        Ok(())
    }

    fn prepare_mods(&mut self, catalog: &InstalledCatalog) -> Result<(), StartupSelectionError> {
        let choices = self
            .collaborators
            .mod_choices(catalog)
            .map_err(StartupSelectionError::Failed)?;
        let descriptions = choices.clone();
        if let Some(selected) = &mut self.selected_mods {
            selected.refresh(descriptions)?;
        } else {
            let mut initial: Vec<ModSelection> = self.initial.mods.iter().map(to_contract).collect();
            if let Some(behavior) = &self.initial.weapon_behavior {
                initial.push(ModSelection {
                    product: behavior.product.clone(),
                    id: behavior.id.clone(),
                });
            }
            self.selected_mods = Some(ModSelectionSet::new(descriptions, initial)?);
        }
        self.mod_choices = choices;
        Ok(())
    }

    /// Re-discover installed content and rebuild the draft caches.
    pub fn refresh_catalog(&mut self) -> Result<(), StartupSelectionError> {
        let user_root = self.current_catalog.user_content_root.as_ref().map(PathBuf::from);
        let catalog = discover_installed_content(&DiscoverContentOptions {
            corpus_root: PathBuf::from(&self.current_catalog.corpus_root),
            user_content_root: user_root,
            products: None,
            generation: self.current_catalog.generation + 1,
            discover_mods: true,
            remote_content: None,
        })?;
        let initial_product = catalog
            .products
            .iter()
            .find(|product| {
                product.expectation.id == self.initial.product && product.availability == ProductAvailability::Installed
            })
            .or_else(|| {
                catalog
                    .products
                    .iter()
                    .find(|product| product.availability == ProductAvailability::Installed)
            })
            .ok_or_else(|| StartupSelectionError::Failed("No installed game content remains".to_string()))?;
        let product_id = initial_product.expectation.id.clone();
        let mut initial = self.initial.clone();
        initial.product = product_id;
        let mut candidate = StartupSelectionModel::new(catalog, initial, self.collaborators.duplicate())?;
        candidate.prepare_maps()?;
        self.current_catalog = candidate.current_catalog;
        self.initial = candidate.initial;
        self.q3_product = candidate.q3_product;
        self.team_arena_campaign = candidate.team_arena_campaign;
        self.mod_choices = candidate.mod_choices;
        self.qvm_hook_styles = candidate.qvm_hook_styles;
        if let Some(selected) = &mut self.selected_mods {
            selected.refresh(self.mod_choices.clone())?;
        }
        self.playable_maps = candidate.playable_maps;
        self.authored_default_maps = candidate.authored_default_maps;
        self.loose_models = candidate.loose_models;
        self.eligible_maps.clear();
        self.map_classnames = candidate.map_classnames;
        self.monster_classes = candidate.monster_classes;
        self.model_choices.clear();
        if !self
            .current_catalog
            .products
            .iter()
            .any(|product| product.expectation.id == self.value(StartupSelectionField::MapProduct))
        {
            let fallback = self.initial.product.clone();
            self.values.insert(StartupSelectionField::MapProduct, fallback);
        }
        if !self
            .current_catalog
            .products
            .iter()
            .any(|product| product.expectation.id == self.value(StartupSelectionField::Product))
        {
            let fallback = self.initial.product.clone();
            self.values.insert(StartupSelectionField::Product, fallback.clone());
            self.values.insert(StartupSelectionField::MapProduct, fallback);
            let map = self.default_map()?;
            self.values.insert(StartupSelectionField::Map, map);
        }
        Ok(())
    }

    /// Scan installed maps, models, mods, and hook styles.
    pub fn prepare_maps(&mut self) -> Result<(), StartupSelectionError> {
        self.eligible_maps.clear();
        self.prepare_q3_catalog()?;
        self.prepare_team_arena()?;
        let catalog = self.current_catalog.clone();
        self.prepare_mods(&catalog)?;
        self.prepare_hook_styles(&catalog)?;
        let mut files: HashMap<String, FileSource> = HashMap::new();
        let mut playable: HashMap<String, bool> = HashMap::new();
        let products = self.current_catalog.products.clone();
        for product in &products {
            if unavailable(product).is_some() {
                continue;
            }
            if product.expectation.family == GameFamily::Q2 {
                if let Some(loose) = &product.loose_root {
                    let directory = Path::new(loose).join("players");
                    let mut models = Vec::new();
                    if directory.is_dir() {
                        for entry in std::fs::read_dir(&directory)? {
                            let entry = entry?;
                            if entry.file_type()?.is_dir()
                                && directory.join(entry.file_name()).join("tris.md2").is_file()
                            {
                                models.push(format!("players/{}/tris.md2", entry.file_name().to_string_lossy()));
                            }
                        }
                    }
                    self.loose_models.insert(product.expectation.id.clone(), models);
                }
            }
            let mut choices: Vec<StartupSelectionChoice> = Vec::new();
            for map in self.current_catalog.maps_for(&product.expectation.id)? {
                let key = map_key(&map.source, map.member_index);
                let mut accepted = playable.get(&key).copied();
                if accepted.is_none() && product.expectation.family == GameFamily::Q1 {
                    let scanned = self.scan_q1_map(&map.source, map.member_index, &map.path, &mut files, &key);
                    match scanned {
                        Ok(value) => {
                            accepted = Some(value);
                            playable.insert(key.clone(), value);
                        }
                        Err(error) => {
                            choices.push(choice(&map.path, &map.path, Some(error.to_string())));
                            continue;
                        }
                    }
                }
                if accepted != Some(false) {
                    choices.push(plain_choice(&map.path));
                }
            }
            let authored = self.current_catalog.authored_starts_for(&product.expectation.id)?;
            let mut starts: Vec<StartupSelectionChoice> = Vec::new();
            for start in authored.map_or_else(Vec::new, |catalog| catalog.starts) {
                if starts
                    .iter()
                    .any(|choice: &StartupSelectionChoice| choice.id.to_lowercase() == start.path.to_lowercase())
                {
                    continue;
                }
                let installed = choices
                    .iter()
                    .find(|choice| choice.id.to_lowercase() == start.path.to_lowercase());
                let title = if start.title.is_empty() {
                    start.path.clone()
                } else {
                    start.title.clone()
                };
                let start_id = installed.map_or_else(|| start.path.clone(), |found| found.id.clone());
                starts.push(choice(
                    &start_id,
                    &title,
                    match installed {
                        None => Some(format!("Missing authored start map: {}", start.path)),
                        Some(found) => found.unavailable.clone(),
                    },
                ));
            }
            if let Some(first) = starts.first() {
                self.authored_default_maps
                    .insert(product.expectation.id.clone(), first.id.clone());
            }
            let authored_paths: HashSet<String> = starts.iter().map(|start| start.id.to_lowercase()).collect();
            let mut rest: Vec<StartupSelectionChoice> = choices
                .into_iter()
                .filter(|map| !authored_paths.contains(&map.id.to_lowercase()))
                .collect();
            rest.sort_by(|left, right| left.id.cmp(&right.id));
            starts.extend(rest);
            self.playable_maps.insert(product.expectation.id.clone(), starts);
        }
        for file in files.values() {
            file.close();
        }
        Ok(())
    }

    /// Scan one Quake map for player starts, caching its monster classes.
    fn scan_q1_map(
        &mut self,
        source: &str,
        member_index: Option<u64>,
        path: &str,
        files: &mut HashMap<String, FileSource>,
        key: &str,
    ) -> Result<bool, StartupSelectionError> {
        let (bytes, length) = read_map_member(source, member_index, files)?;
        let header = bytes.get(0..12.min(bytes.len())).unwrap_or(&[]);
        if header.len() < 12 {
            return Err(StartupSelectionError::Failed(format!("Truncated map header: {path}")));
        }
        let _ = length;
        if classify_bsp(header, path).map_err(|error| StartupSelectionError::Failed(error.to_string()))? != BspKind::Q1
        {
            return Ok(true);
        }
        let start = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
        let size = u32::from_le_bytes([header[8], header[9], header[10], header[11]]) as usize;
        if start > bytes.len() || size > bytes.len() - start {
            return Err(StartupSelectionError::Failed(format!(
                "Invalid map entity lump: {path}"
            )));
        }
        let text = String::from_utf8_lossy(&bytes[start..start + size]);
        let entities =
            parse_q1_entities(&text, path).map_err(|error| StartupSelectionError::Failed(error.to_string()))?;
        let classnames: Vec<String> = entities
            .iter()
            .map(|entity| q1_entity_value(entity, "classname").unwrap_or("").to_string())
            .collect();
        self.cache_monster_classes(key, &classnames);
        Ok(entities.iter().any(|entity| {
            matches!(
                q1_entity_value(entity, "classname"),
                Some("info_player_start" | "info_player_deathmatch" | "info_player_coop" | "info_player_start2")
            )
        }))
    }

    /// Training map for a Q3 product, if its arena files name one.
    fn q3_training_map(&self, product: &CatalogProduct) -> Result<Option<String>, StartupSelectionError> {
        for path in self.files(product)?.iter().filter(|path| {
            path.as_str() == "scripts/arenas.txt"
                || (path.starts_with("scripts/") && path.ends_with(".arena") && !path["scripts/".len()..].contains('/'))
        }) {
            let bytes = self.current_catalog.read(&product.expectation.id, path)?;
            let text = String::from_utf8_lossy(&bytes).into_owned();
            let mut cursor =
                CommonParseCursor::new(&text).map_err(|error| StartupSelectionError::Failed(error.to_string()))?;
            let mut parser = CommonParseState::new();
            loop {
                let token = parser
                    .parse(&mut cursor)
                    .map_err(|error| StartupSelectionError::Failed(error.to_string()))?;
                if token != "{" {
                    break;
                }
                let mut fields = HashMap::new();
                loop {
                    let key = parser
                        .parse(&mut cursor)
                        .map_err(|error| StartupSelectionError::Failed(error.to_string()))?;
                    if key == "}" || key.is_empty() {
                        break;
                    }
                    let value = parser
                        .parse(&mut cursor)
                        .map_err(|error| StartupSelectionError::Failed(error.to_string()))?;
                    fields.insert(key, value);
                }
                if fields
                    .get("special")
                    .is_some_and(|special| special.to_lowercase() == "training")
                {
                    if let Some(map) = fields.get("map") {
                        return Ok(Some(format!("maps/{map}.bsp")));
                    }
                }
            }
        }
        Ok(None)
    }

    /// Installed official campaign presets.
    pub fn presets(&self) -> Vec<StartupNativePreset> {
        let expected_list = expected_products();
        let expected: HashSet<&str> = expected_list.iter().map(|product| product.id.as_str()).collect();
        self.current_catalog
            .products
            .iter()
            .filter(|product| {
                product.availability == ProductAvailability::Installed
                    && expected.contains(product.expectation.id.as_str())
                    && (product.expectation.edition == "classic" || product.expectation.edition == "rerelease")
                    && product.expectation.campaign != "ctf"
                    && product.expectation.campaign != "lmctf"
            })
            .map(|product| {
                let family = product.expectation.family;
                let difficulties = if family == GameFamily::Q3 {
                    vec![
                        choice("1", "I Can Win", None),
                        choice("2", "Bring It On", None),
                        choice("3", "Hurt Me Plenty", None),
                        choice("4", "Hardcore", None),
                        choice("5", "Nightmare", None),
                    ]
                } else {
                    vec![
                        choice("0", "Easy", None),
                        choice("1", "Normal", None),
                        choice("2", "Hard", None),
                        choice("3", "Nightmare", None),
                    ]
                };
                let base = product_choice(product);
                StartupNativePreset {
                    id: base.id,
                    label: base.label,
                    unavailable: None,
                    family,
                    edition: product.expectation.edition.clone(),
                    difficulties,
                    default_skill: if family == GameFamily::Q3 {
                        "2".to_string()
                    } else {
                        "1".to_string()
                    },
                }
            })
            .collect()
    }

    /// Resolve a native campaign preset to launch options and a recipe.
    pub fn resolve_preset(
        &mut self,
        id: &str,
        difficulty: Option<u8>,
        arena_map: Option<&str>,
    ) -> Result<StartupLaunch, StartupSelectionError> {
        let selected = self
            .presets()
            .into_iter()
            .find(|preset| preset.id == id)
            .ok_or_else(|| {
                StartupSelectionError::Failed(format!("Installed official campaign preset unavailable: {id}"))
            })?;
        if let Some(reason) = selected.unavailable {
            return Err(StartupSelectionError::Failed(reason));
        }
        let level = difficulty.unwrap_or_else(|| selected.default_skill.parse().unwrap_or(1));
        if !selected
            .difficulties
            .iter()
            .any(|choice| choice.id.parse::<u8>().ok() == Some(level))
        {
            return Err(StartupSelectionError::Failed("Invalid preset difficulty".to_string()));
        }
        let product = self.current_catalog.require(id)?;
        let family = product.expectation.family;
        let team_arena_skirmish = if product.expectation.campaign == "missionpack" && (1..=5).contains(&level) {
            let campaign = self
                .team_arena_campaign
                .as_ref()
                .ok_or_else(|| StartupSelectionError::Failed("Team Arena metadata is not prepared".to_string()))?;
            Some(plan_team_arena_skirmish(
                campaign,
                level,
                DEFAULT_SKIRMISH_CURSOR,
                &self.selected_teams,
            )?)
        } else {
            None
        };
        let selected_arena = arena_map.and_then(|map| {
            self.arena_selection
                .as_ref()?
                .rows
                .iter()
                .find(|row| row.arena.map == map)
        });
        if arena_map.is_some() && (id != "q3-baseq3" || selected_arena.is_none_or(|row| !row.available)) {
            return Err(StartupSelectionError::Failed(
                "That arena is not unlocked in this profile.".to_string(),
            ));
        }
        let product = self.current_catalog.require(id)?;
        let preferred = selected_arena
            .map(|row| row.arena.map.clone())
            .or_else(|| team_arena_skirmish.as_ref().map(|skirmish| skirmish.map.clone()))
            .or_else(|| {
                if family == GameFamily::Q3 {
                    self.q3_training_map(product).unwrap_or(None)
                } else {
                    None
                }
            });
        let preferred = preferred.or_else(|| {
            if family == GameFamily::Q3 {
                None
            } else {
                self.authored_default_maps
                    .get(id)
                    .cloned()
                    .or_else(|| product.expectation.map_witness.clone())
                    .or_else(|| {
                        if family == GameFamily::Q1 {
                            Some("maps/start.bsp".to_string())
                        } else {
                            None
                        }
                    })
            }
        });
        let playable = self.playable_maps.get(id).cloned().unwrap_or_default();
        let map = preferred.as_ref().and_then(|path| {
            playable
                .iter()
                .find(|choice| choice.id.to_lowercase() == path.to_lowercase())
        });
        match map {
            Some(found) if found.unavailable.is_none() => {}
            found => {
                let reason = found
                    .and_then(|choice| choice.unavailable.clone())
                    .unwrap_or_else(|| "authored campaign start map is unavailable".to_string());
                return Err(StartupSelectionError::Failed(format!("{}: {reason}", selected.label)));
            }
        }
        let map = map.map_or_else(String::new, |found| found.id.clone());
        let character_model = team_arena_skirmish.as_ref().map_or_else(
            || match family {
                GameFamily::Q1 => "player".to_string(),
                GameFamily::Q2 => "male".to_string(),
                GameFamily::Q3 => "sarge".to_string(),
            },
            |skirmish| skirmish.player_model.clone(),
        );
        let product = self.current_catalog.require(id)?;
        let model = self
            .models_for(product)
            .iter()
            .find(|model| model.id == character_model)
            .cloned();
        match model {
            Some(found) if found.unavailable.is_none() => {}
            _ => {
                return Err(StartupSelectionError::Failed(format!(
                    "{}: native {character_model} model is unavailable",
                    selected.label
                )));
            }
        }
        let skill = if family == GameFamily::Q3 { 1 } else { level };
        if skill > 3 {
            return Err(StartupSelectionError::Failed("Invalid campaign difficulty".to_string()));
        }
        let renderer = match self.value(StartupSelectionField::Renderer) {
            "gl" => Renderer::Gl,
            "cpu" => Renderer::Cpu,
            _ => return Err(StartupSelectionError::Failed("Invalid renderer selection".to_string())),
        };
        let mut options = self.initial.clone();
        options.server_profile_path = None;
        options.map_product = None;
        options.quake_c_program = None;
        options.weapon_behavior = None;
        options.mods = Vec::new();
        options.q1_protocol = None;
        options.q2_protocol = None;
        options.width = self.display.width;
        options.height = self.display.height;
        options.gamma = self.display.gamma;
        if self.display_overrides_consumed {
            options.display_overrides = crate::options::DisplayOverrides::default();
        }
        options.renderer = renderer;
        options.product = id.to_string();
        options.map = map;
        options.movement = options_family(family);
        options.movement_product = Some(id.to_string());
        options.character = options_family(family);
        options.character_model = character_model;
        options.skill = skill;
        options.bot_skill = if family == GameFamily::Q3 && (1..=5).contains(&level) {
            Some(level)
        } else {
            None
        };
        options.mode = GameMode::Singleplayer;
        options.rules = Some(MatchRules::Standard);
        options.seats = 1;
        options.dedicated = false;
        options.network = Network::Offline;
        self.apply_selected_server_profile(&mut options);
        let movement = provider_reference(&format!("{family}:movement"), id);
        let character = provider_reference(&format!("{family}:character"), id);
        let catalog = self.current_catalog.clone();
        let preset = self
            .collaborators
            .application_preset(&catalog, &options, Some(&movement), Some(&character))
            .map_err(StartupSelectionError::Failed)?;
        let choice = preset_choice(preset.id.clone());
        let weapons = self.collaborators.launch_weapons();
        let compat = self.collaborators.launch_compat();
        let recipe = resolve_launch(
            &ResolveLaunchOptions {
                choice: &choice,
                preset: &preset,
                catalog: &catalog,
                id: None,
                mounts: None,
            },
            &*weapons,
            &*compat,
        )?;
        options.explicit_rules.skill = true;
        options.explicit_rules.mode = true;
        options.explicit_rules.capacity = true;
        Ok(StartupLaunch {
            options,
            recipe,
            q3_product: self.q3_product,
            team_arena_skirmish,
        })
    }

    fn maps(&mut self) -> Result<Vec<StartupSelectionChoice>, StartupSelectionError> {
        let product_id = self.value(StartupSelectionField::MapProduct).to_string();
        let product = self.current_catalog.require(&product_id)?.clone();
        let base = self
            .playable_maps
            .get(&product.expectation.id)
            .cloned()
            .unwrap_or_default();
        let options = self.options()?;
        if options.mode != GameMode::Deathmatch && options.rules.unwrap_or(MatchRules::Standard) == MatchRules::Standard
        {
            return Ok(base);
        }
        let source_id = self.value(StartupSelectionField::Product).to_string();
        let source = source_program_product(&self.current_catalog, &source_id)?.clone();
        let key = format!(
            "{}:{}:{}:{}",
            source.expectation.id,
            product.expectation.id,
            mode_str(options.mode),
            options.rules.map_or("standard", rules_str)
        );
        if let Some(cached) = self.eligible_maps.get(&key) {
            return Ok(cached.clone());
        }
        let metadata: HashMap<String, (String, Option<u64>)> = self
            .current_catalog
            .maps_for(&product.expectation.id)?
            .into_iter()
            .map(|map| (map.path.to_lowercase(), (map.source, map.member_index)))
            .collect();
        let result: Vec<StartupSelectionChoice> = base
            .into_iter()
            .map(|option| {
                if option.unavailable.is_some() {
                    return option;
                }
                let key = metadata.get(&option.id.to_lowercase());
                let classnames = key.and_then(|(source, member)| self.map_classnames.get(&map_key(source, *member)));
                match classnames {
                    None => option,
                    Some(names) => {
                        let family = source.expectation.family.to_string();
                        let selection = MatchModeSelection {
                            family: &family,
                            edition: &source.expectation.edition,
                            campaign: &source.expectation.campaign,
                            mode: mode_str(options.mode),
                            rules: options.rules.map_or("standard", rules_str),
                        };
                        StartupSelectionChoice {
                            unavailable: match_map_unavailable(&selection, names),
                            ..option
                        }
                    }
                }
            })
            .collect();
        self.eligible_maps.insert(key, result.clone());
        Ok(result)
    }

    fn default_map(&mut self) -> Result<String, StartupSelectionError> {
        let product_id = self.value(StartupSelectionField::MapProduct).to_string();
        let product = self.current_catalog.require(&product_id)?.clone();
        let maps = self.maps()?;
        let preferred = self
            .authored_default_maps
            .get(&product.expectation.id)
            .cloned()
            .or_else(|| product.expectation.map_witness.clone())
            .or_else(|| match product.expectation.family {
                GameFamily::Q1 => Some("maps/start.bsp".to_string()),
                GameFamily::Q2 => Some("maps/base1.bsp".to_string()),
                GameFamily::Q3 => Some("maps/q3dm0.bsp".to_string()),
            });
        Ok(preferred
            .and_then(|path| maps.iter().find(|map| map.id == path).map(|map| map.id.clone()))
            .unwrap_or_default())
    }

    fn product(&self, field: StartupSelectionField) -> Result<&CatalogProduct, StartupSelectionError> {
        let id = self.value(field).to_string();
        Ok(self.current_catalog.product(&id)?)
    }

    fn geometry(&self) -> Result<&CatalogProduct, StartupSelectionError> {
        let id = self.value(StartupSelectionField::MapProduct).to_string();
        Ok(self.current_catalog.require(&id)?)
    }

    fn files(&self, product: &CatalogProduct) -> Result<HashSet<String>, StartupSelectionError> {
        let mut paths: HashSet<String> = product
            .archives
            .iter()
            .flat_map(|archive| archive.entries.iter().map(|entry| entry.path.to_lowercase()))
            .chain(
                self.loose_models
                    .get(&product.expectation.id)
                    .cloned()
                    .unwrap_or_default(),
            )
            .collect();
        if let Some(base) = &product.expectation.base_product {
            for path in self.files(self.current_catalog.product(base)?)? {
                paths.insert(path);
            }
        }
        Ok(paths)
    }

    fn models(&mut self) -> Result<Vec<StartupSelectionChoice>, StartupSelectionError> {
        let id = self.value(StartupSelectionField::Character).to_string();
        let product = self.current_catalog.product(&id)?.clone();
        if let Some(existing) = self.model_choices.get(&product.expectation.id) {
            return Ok(existing.clone());
        }
        let result = self.models_for(&product);
        self.model_choices
            .insert(product.expectation.id.clone(), result.clone());
        Ok(result)
    }

    fn models_for(&self, product: &CatalogProduct) -> Vec<StartupSelectionChoice> {
        let paths = self.files(product).unwrap_or_default();
        if product.expectation.family == GameFamily::Q1 {
            return vec![choice(
                "player",
                "Quake player",
                if paths.contains("progs/player.mdl") {
                    None
                } else {
                    Some("Player model not installed".to_string())
                },
            )];
        }
        let mut models = HashSet::new();
        for path in &paths {
            let name = if product.expectation.family == GameFamily::Q3 {
                let rest = path.strip_prefix("models/players/").unwrap_or("");
                let rest = rest.strip_prefix("characters/").unwrap_or(rest);
                match rest.rsplit_once('/') {
                    Some((name, "lower.md3")) if !name.contains('/') => Some(name.to_string()),
                    _ => None,
                }
                .filter(|name| {
                    let lower = name.to_lowercase();
                    (paths.contains(&format!("models/players/{lower}/upper.md3"))
                        || paths.contains(&format!("models/players/characters/{lower}/upper.md3")))
                        && (paths.contains(&format!("models/players/{lower}/head.md3"))
                            || paths.contains(&format!("models/players/heads/{lower}/{lower}.md3")))
                })
            } else {
                path.strip_prefix("players/")
                    .and_then(|rest| rest.strip_suffix("/tris.md2"))
                    .filter(|name| !name.contains('/'))
                    .map(str::to_string)
            };
            if let Some(name) = name {
                models.insert(name);
            }
        }
        let mut names: Vec<String> = models.into_iter().collect();
        names.sort();
        names.into_iter().map(|name| plain_choice(&name)).collect()
    }

    fn roster(&mut self) -> &mut MonsterRoster {
        let product = self.value(StartupSelectionField::Product).to_string();
        self.rosters.entry(product).or_insert_with(|| MonsterRoster {
            source: "native".to_string(),
            default: "native".to_string(),
            by_classname: HashMap::new(),
        })
    }

    fn cache_monster_classes(&mut self, key: &str, classnames: &[String]) {
        self.map_classnames.insert(key.to_string(), classnames.to_vec());
        self.eligible_maps.clear();
        let mut counts: HashMap<String, u32> = HashMap::new();
        for classname in classnames {
            if classname.starts_with("monster_") {
                *counts.entry(classname.clone()).or_insert(0) += 1;
            }
        }
        self.monster_classes.insert(key.to_string(), counts);
    }

    /// Read the selected map's monster roster.
    pub fn prepare_monster_roster(&mut self) -> Result<(), StartupSelectionError> {
        if self.geometry()?.expectation.family == GameFamily::Q3 {
            return Err(StartupSelectionError::Failed(
                "This map has no supported authored monster roster".to_string(),
            ));
        }
        self.prepare_map_classnames(None)
    }

    /// Prepare one page of map choices; returns whether work was done.
    pub fn prepare_map_choices(&mut self, offset: usize, count: usize) -> Result<bool, StartupSelectionError> {
        let product = self.geometry()?.clone();
        if self.value(StartupSelectionField::Mode) != "deathmatch"
            && self.value(StartupSelectionField::Rules) == "standard"
        {
            return Ok(false);
        }
        let maps = self.current_catalog.maps_for(&product.expectation.id)?;
        let choices: Vec<StartupSelectionChoice> = self
            .maps()?
            .into_iter()
            .skip(offset)
            .take(count)
            .filter(|choice| {
                if choice.unavailable.is_some() {
                    return false;
                }
                let map = maps
                    .iter()
                    .find(|map| map.path.to_lowercase() == choice.id.to_lowercase());
                map.is_none_or(|map| {
                    !self
                        .map_classnames
                        .contains_key(&map_key(&map.source, map.member_index))
                })
            })
            .collect();
        if choices.is_empty() {
            return Ok(false);
        }
        self.prepare_map_page(&product.expectation.id, &choices)?;
        Ok(true)
    }

    fn prepare_map_page(
        &mut self,
        product_id: &str,
        choices: &[StartupSelectionChoice],
    ) -> Result<(), StartupSelectionError> {
        for map in choices {
            if let Err(error) = self.prepare_map_classnames(Some(map.id.clone())) {
                let reason = error.to_string();
                let current = self.playable_maps.get(product_id).cloned().unwrap_or_default();
                self.playable_maps.insert(
                    product_id.to_string(),
                    current
                        .into_iter()
                        .map(|value| {
                            if value.id == map.id {
                                StartupSelectionChoice {
                                    unavailable: Some(reason.clone()),
                                    ..value
                                }
                            } else {
                                value
                            }
                        })
                        .collect(),
                );
                self.eligible_maps.clear();
            }
        }
        Ok(())
    }

    fn prepare_map_classnames(&mut self, path: Option<String>) -> Result<(), StartupSelectionError> {
        let path = path.unwrap_or_else(|| self.value(StartupSelectionField::Map).to_string());
        let product = self.geometry()?.clone();
        let map = self
            .current_catalog
            .maps_for(&product.expectation.id)?
            .into_iter()
            .find(|map| map.path.to_lowercase() == path.to_lowercase())
            .ok_or_else(|| StartupSelectionError::Failed("Selected map is unavailable".to_string()))?;
        let key = map_key(&map.source, map.member_index);
        if self.map_classnames.contains_key(&key) {
            return Ok(());
        }
        let mut files = HashMap::new();
        let (bytes, length) = read_map_member(&map.source, map.member_index, &mut files)?;
        for file in files.values() {
            file.close();
        }
        let header = bytes.get(0..16.min(bytes.len())).unwrap_or(&[]);
        if header.len() < 16 {
            return Err(StartupSelectionError::Failed("Truncated map header".to_string()));
        }
        let codec =
            classify_bsp(header, &map.path).map_err(|error| StartupSelectionError::Failed(error.to_string()))?;
        let q1 = codec == BspKind::Q1;
        let at = |index: usize| {
            u32::from_le_bytes([header[index], header[index + 1], header[index + 2], header[index + 3]]) as usize
        };
        let (start, declared) = if q1 { (at(4), at(8)) } else { (at(8), at(12)) };
        let size = if codec == BspKind::Q2 && (start as u64) < length {
            declared.min(length as usize - start)
        } else {
            declared
        };
        if (start as u64) > length || (size as u64) > length - start as u64 {
            return Err(StartupSelectionError::Failed("Invalid map entity lump".to_string()));
        }
        let text = String::from_utf8_lossy(bytes.get(start..start + size).unwrap_or(&[]))
            .trim_end_matches('\0')
            .to_string();
        let classnames: Vec<String> = match codec {
            BspKind::Q2 => parse_q2_entities(
                &text,
                if product.expectation.edition == "rerelease" {
                    Q2Edition::Rerelease
                } else {
                    Q2Edition::Classic
                },
            )
            .iter()
            .map(|entity| entity.classname.clone())
            .collect(),
            BspKind::Q3 => parse_q3_entities(&text, &map.path)
                .map_err(|error| StartupSelectionError::Failed(error.to_string()))?
                .iter()
                .map(|entity| entity.get("classname").unwrap_or("").to_string())
                .collect(),
            BspKind::Q1 => parse_q1_entities(&text, &map.path)
                .map_err(|error| StartupSelectionError::Failed(error.to_string()))?
                .iter()
                .map(|entity| q1_entity_value(entity, "classname").unwrap_or("").to_string())
                .collect(),
        };
        self.cache_monster_classes(&key, &classnames);
        Ok(())
    }

    fn monster_choices(&self) -> Result<Vec<StartupSelectionChoice>, StartupSelectionError> {
        let mut choices = vec![choice("native", "Keep native", None)];
        for source in monster_sources() {
            let family = source.family.as_str();
            let edition = edition_str(source.edition);
            let program = source.program.as_str();
            let product = self.current_catalog.products.iter().find(|product| {
                product.expectation.family.to_string() == family
                    && product.expectation.edition == edition
                    && product.expectation.campaign == program
            });
            let paths = match product {
                Some(product) => self.files(product)?,
                None => HashSet::new(),
            };
            let mut creatures: Vec<(&String, &qa_content::monsters::MonsterCreature)> =
                source.creatures.iter().collect();
            creatures.sort_by(|left, right| left.0.cmp(right.0));
            for (classname, creature) in creatures {
                let reason = match product {
                    None => Some("Source content unavailable".to_string()),
                    Some(product) => unavailable(product).or_else(|| {
                        if creature
                            .resources
                            .iter()
                            .all(|path| paths.contains(&path.to_lowercase()))
                        {
                            None
                        } else {
                            Some("Creature resources are not installed".to_string())
                        }
                    }),
                };
                choices.push(choice(
                    &format!("{}/{classname}", provider_text(&source.provider)),
                    &format!(
                        "{} ({}{}, {edition})",
                        monster_label(classname),
                        family.to_uppercase(),
                        monster_program_suffix(program)
                    ),
                    reason,
                ));
            }
        }
        Ok(choices)
    }

    /// Monster source row for the roster menu.
    pub fn monster_source_row(&self) -> Result<MonsterSourceRow, StartupSelectionError> {
        let mut choices = vec![choice("native", "Authored campaign monsters", None)];
        for source in monster_sources() {
            let family = source.family.as_str();
            let edition = edition_str(source.edition);
            let program = source.program.as_str();
            let product = self.current_catalog.products.iter().find(|product| {
                product.expectation.family.to_string() == family
                    && product.expectation.edition == edition
                    && product.expectation.campaign == program
            });
            choices.push(choice(
                &provider_text(&source.provider),
                &format!(
                    "{} ({edition})",
                    monster_source_title(
                        family,
                        program,
                        product.map(|product| product.expectation.title.as_str())
                    )
                ),
                match product {
                    None => Some("Source content unavailable".to_string()),
                    Some(product) => unavailable(product),
                },
            ));
        }
        let product = self.value(StartupSelectionField::Product).to_string();
        let value = self
            .rosters
            .get(&product)
            .map_or("native", |roster| roster.source.as_str())
            .to_string();
        Ok(MonsterSourceRow {
            label: "Monster source".to_string(),
            value,
            choices,
        })
    }

    /// Select the monster source for a custom roster.
    pub fn select_monster_source(&mut self, id: &str) -> Result<(), StartupSelectionError> {
        if self.geometry()?.expectation.family == GameFamily::Q3 {
            return Err(StartupSelectionError::Failed(
                "This map has no supported authored monster roster".to_string(),
            ));
        }
        let selected = self
            .monster_source_row()?
            .choices
            .into_iter()
            .find(|choice| choice.id == id)
            .ok_or_else(|| StartupSelectionError::Failed("Unknown monster source".to_string()))?;
        if let Some(reason) = selected.unavailable {
            return Err(StartupSelectionError::Failed(reason));
        }
        self.roster().source = id.to_string();
        self.values.insert(StartupSelectionField::Enemies, "custom".to_string());
        Ok(())
    }

    fn effective_monster_roster(&mut self, include_overrides: bool) -> Result<EnemySelection, StartupSelectionError> {
        let product = self.value(StartupSelectionField::Product).to_string();
        let roster = self.rosters.get(&product).cloned().unwrap_or(MonsterRoster {
            source: "native".to_string(),
            default: "native".to_string(),
            by_classname: HashMap::new(),
        });
        let family = self.geometry()?.expectation.family;
        let mut by_classname: HashMap<String, MonsterSelectionTarget> = HashMap::new();
        if include_overrides {
            for (classname, id) in &roster.by_classname {
                by_classname.insert(classname.clone(), self.monster_target(id)?);
            }
        }
        let fallback = self.monster_target(&roster.default)?;
        if roster.source == "native" || family == GameFamily::Q3 {
            return Ok(EnemySelection::Replace {
                default: fallback,
                by_classname,
            });
        }
        let source = monster_sources()
            .into_iter()
            .find(|source| provider_text(&source.provider) == roster.source)
            .ok_or_else(|| StartupSelectionError::Failed("Unknown monster source".to_string()))?;
        let edition = edition_str(source.edition);
        let program = source.program.as_str();
        let product = self
            .current_catalog
            .products
            .iter()
            .find(|product| {
                product.expectation.family.to_string() == source.family.as_str()
                    && product.expectation.edition == edition
                    && product.expectation.campaign == program
            })
            .ok_or_else(|| StartupSelectionError::Failed("Monster source content unavailable".to_string()))?;
        let authored = match family {
            GameFamily::Q1 => MonsterFamily::Q1,
            GameFamily::Q2 => MonsterFamily::Q2,
            GameFamily::Q3 => {
                return Ok(EnemySelection::Replace {
                    default: fallback,
                    by_classname,
                })
            }
        };
        let target = ProviderReference {
            provider: source.provider.clone(),
            content: product.id.clone(),
        };
        match default_monster_roster(authored, &target, &by_classname)? {
            EnemySelection::Replace { by_classname, .. } => Ok(EnemySelection::Replace {
                default: fallback,
                by_classname,
            }),
            other => Ok(other),
        }
    }

    /// Custom monster roster rows.
    pub fn monster_roster_rows(&mut self) -> Result<Vec<MonsterRosterRow>, StartupSelectionError> {
        let geometry_id = self.geometry()?.expectation.id.clone();
        let map = self
            .current_catalog
            .maps_for(&geometry_id)?
            .into_iter()
            .find(|map| map.path == self.value(StartupSelectionField::Map));
        let counts = map.as_ref().and_then(|map| {
            self.monster_classes
                .get(&map_key(&map.source, map.member_index))
                .cloned()
        });
        let choices = self.monster_choices()?;
        let defaults = self.effective_monster_roster(false)?;
        let effective = self.effective_monster_roster(true)?;
        let family = self.geometry()?.expectation.family;
        let product = self.value(StartupSelectionField::Product).to_string();
        let roster = self.rosters.get(&product).cloned().unwrap_or(MonsterRoster {
            source: "native".to_string(),
            default: "native".to_string(),
            by_classname: HashMap::new(),
        });
        let mut slots: HashSet<String> = campaign_monster_slots(family)
            .iter()
            .map(|slot| slot.classname.clone())
            .collect();
        if let Some(counts) = &counts {
            slots.extend(counts.keys().cloned());
        }
        slots.extend(roster.by_classname.keys().cloned());
        let target_label = |target: &MonsterSelectionTarget| match target {
            MonsterSelectionTarget::MapDefined => "Keep native".to_string(),
            MonsterSelectionTarget::Defined(reference) => {
                let source = monster_sources()
                    .into_iter()
                    .find(|source| source.provider == reference.source.provider);
                match source {
                    None => format!(
                        "{} ({})",
                        monster_label(&reference.classname),
                        provider_text(&reference.source.provider)
                    ),
                    Some(source) => format!(
                        "{} ({}{} {})",
                        monster_label(&reference.classname),
                        source.family.as_str().to_uppercase(),
                        monster_program_suffix(source.program.as_str()),
                        edition_str(source.edition)
                    ),
                }
            }
        };
        let mut rows = vec![MonsterRosterRow {
            classname: None,
            label: "Unmatched classes".to_string(),
            value: roster.default.clone(),
            effective_label: match &defaults {
                EnemySelection::Replace { default, .. } => target_label(default),
                EnemySelection::MapDefined => "Keep native".to_string(),
            },
            choices: choices.clone(),
        }];
        let mut slots: Vec<String> = slots.into_iter().collect();
        slots.sort_by(|left, right| {
            let left_live = counts
                .as_ref()
                .is_some_and(|counts| counts.get(left).is_some_and(|count| *count > 0));
            let right_live = counts
                .as_ref()
                .is_some_and(|counts| counts.get(right).is_some_and(|count| *count > 0));
            right_live.cmp(&left_live).then_with(|| left.cmp(right))
        });
        for classname in slots {
            let value = roster
                .by_classname
                .get(&classname)
                .cloned()
                .unwrap_or_else(|| "default".to_string());
            let target = match &effective {
                EnemySelection::Replace { default, by_classname } => {
                    by_classname.get(&classname).unwrap_or(default).clone()
                }
                EnemySelection::MapDefined => MonsterSelectionTarget::MapDefined,
            };
            let default_target = match &defaults {
                EnemySelection::Replace { default, by_classname } => {
                    by_classname.get(&classname).unwrap_or(default).clone()
                }
                EnemySelection::MapDefined => MonsterSelectionTarget::MapDefined,
            };
            let label = target_label(&target);
            rows.push(MonsterRosterRow {
                classname: Some(classname.clone()),
                label: format!(
                    "{} ({})",
                    monster_label(&classname),
                    counts
                        .as_ref()
                        .and_then(|counts| counts.get(&classname))
                        .copied()
                        .unwrap_or(0)
                ),
                value: value.clone(),
                effective_label: format!("{label}{}", if value == "default" { "" } else { " *" }),
                choices: vec![choice(
                    "default",
                    &format!("Use default: {}", target_label(&default_target)),
                    None,
                )]
                .into_iter()
                .chain(choices.clone())
                .collect(),
            });
        }
        Ok(rows)
    }

    /// Select one monster roster choice.
    pub fn select_monster(&mut self, classname: Option<&str>, id: &str) -> Result<(), StartupSelectionError> {
        let row = self
            .monster_roster_rows()?
            .into_iter()
            .find(|row| row.classname.as_deref() == classname);
        let selected = row
            .as_ref()
            .and_then(|row| row.choices.iter().find(|choice| choice.id == id))
            .ok_or_else(|| StartupSelectionError::Failed("Unknown monster roster choice".to_string()))?;
        if let Some(reason) = &selected.unavailable {
            return Err(StartupSelectionError::Failed(reason.clone()));
        }
        match classname {
            None => self.roster().default = id.to_string(),
            Some(classname) if id == "default" => {
                self.roster().by_classname.remove(classname);
            }
            Some(classname) => {
                self.roster().by_classname.insert(classname.to_string(), id.to_string());
            }
        }
        Ok(())
    }

    fn monster_target(&self, id: &str) -> Result<MonsterSelectionTarget, StartupSelectionError> {
        if id == "native" {
            return Ok(MonsterSelectionTarget::MapDefined);
        }
        let source = monster_sources()
            .into_iter()
            .find(|source| id.starts_with(&format!("{}/", provider_text(&source.provider))))
            .ok_or_else(|| StartupSelectionError::Failed("Unknown monster source".to_string()))?;
        let edition = edition_str(source.edition);
        let program = source.program.as_str();
        let product = self
            .current_catalog
            .products
            .iter()
            .find(|product| {
                product.expectation.family.to_string() == source.family.as_str()
                    && product.expectation.edition == edition
                    && product.expectation.campaign == program
            })
            .ok_or_else(|| StartupSelectionError::Failed("Monster source content unavailable".to_string()))?;
        Ok(MonsterSelectionTarget::Defined(MonsterDefinitionReference {
            source: ProviderReference {
                provider: source.provider.clone(),
                content: product.id.clone(),
            },
            classname: id[provider_text(&source.provider).len() + 1..].to_string(),
        }))
    }

    fn base_choices(&self) -> Vec<StartupSelectionChoice> {
        self.current_catalog
            .products
            .iter()
            .filter(|product| {
                (product.expectation.edition == "classic" || product.expectation.edition == "rerelease")
                    && ((product.expectation.family == GameFamily::Q1 && product.expectation.campaign == "id1")
                        || (product.expectation.family == GameFamily::Q2 && product.expectation.campaign == "baseq2"))
                    || product.expectation.id == "q3-baseq3"
            })
            .map(product_choice)
            .collect()
    }

    /// All draft rows (donor `rows`; renamed: `ModMenuService::rows` owns the short name).
    pub fn draft_rows(&mut self) -> Result<Vec<StartupSelectionRow>, StartupSelectionError> {
        let current = self.product(StartupSelectionField::Product)?.clone();
        let current_label = product_choice(&current).label;
        let catalog = self.current_catalog.clone();
        let defaults_player = self
            .collaborators
            .player_products(
                &catalog,
                self.value(StartupSelectionField::Product),
                current.expectation.family,
                None,
                current.expectation.family,
                &self.initial.network,
            )
            .map_err(StartupSelectionError::Failed)?;
        let mut movement_choices = self.base_choices();
        movement_choices.push(product_choice(self.current_catalog.product("q1-quakeworld")?));
        let mut character_choices = self.base_choices();
        for id in [
            &defaults_player.movement,
            &self.value(StartupSelectionField::Movement).to_string(),
        ] {
            if !movement_choices.iter().any(|choice| &choice.id == id) {
                movement_choices.push(product_choice(self.current_catalog.product(id)?));
            }
        }
        for id in [
            &defaults_player.character,
            &self.value(StartupSelectionField::Character).to_string(),
        ] {
            if !character_choices.iter().any(|choice| &choice.id == id) {
                character_choices.push(product_choice(self.current_catalog.product(id)?));
            }
        }
        let product = self.value(StartupSelectionField::Product).to_string();
        let roster = self.rosters.get(&product).cloned().unwrap_or(MonsterRoster {
            source: "native".to_string(),
            default: "native".to_string(),
            by_classname: HashMap::new(),
        });
        let monster_source = self.monster_source_row()?;
        let source_label = monster_source
            .choices
            .iter()
            .find(|source| source.id == roster.source)
            .map_or("Authored campaign monsters", |source| source.label.as_str())
            .to_string();
        let customized = roster.default != "native" || !roster.by_classname.is_empty();
        let monster_value =
            if self.value(StartupSelectionField::Enemies) == "custom" && roster.source != "native" && !customized {
                roster.source.clone()
            } else {
                self.value(StartupSelectionField::Enemies).to_string()
            };
        let provider = provider_reference(
            &format!("{}:official", current.expectation.family),
            &current.expectation.id,
        );
        let rules_product = if self.value(StartupSelectionField::Rules) == "ctf"
            || self.value(StartupSelectionField::Rules) == "lmctf"
        {
            self.current_catalog
                .product(&format!("q2-classic-{}", self.value(StartupSelectionField::Rules)))?
                .clone()
        } else {
            current.clone()
        };
        let defaults = if unavailable(&current).is_some() || unavailable(&rules_product).is_some() {
            disabled_equipment()
        } else if self.value(StartupSelectionField::Rules) == "standard" {
            native_equipment(&self.current_catalog, &provider, &provider)?
        } else {
            let rules_ref = provider_reference(
                &format!(
                    "{}:{}",
                    current.expectation.family,
                    self.value(StartupSelectionField::Rules)
                ),
                &rules_product.expectation.id,
            );
            native_equipment(&self.current_catalog, &provider, &rules_ref)?
        };
        let native_weapons = choice("native", &format!("{} weapons", current_label), None);
        let native_monsters = choice("native", &format!("{} authored monsters", current_label), None);
        let styles = self.hook_styles();
        let placement = if self.value(StartupSelectionField::Grapple) == "native" {
            match &defaults.grapple {
                GrappleSelection::Enabled { binding, .. } => match binding {
                    GrappleBinding::Slot => "slot",
                    GrappleBinding::Offhand => "offhand",
                },
                GrappleSelection::Disabled => "disabled",
            }
            .to_string()
        } else {
            self.value(StartupSelectionField::Grapple).to_string()
        };
        let default_style = match &defaults.grapple {
            GrappleSelection::Enabled { mechanic, .. } => match mechanic {
                GrappleMechanicDetail::Q3Qvm { profile } => profile.id.clone(),
                GrappleMechanicDetail::Q1Threewave { .. } => "q1-threewave".to_string(),
                GrappleMechanicDetail::Q2Ctf { .. } => "q2-ctf".to_string(),
                GrappleMechanicDetail::Q2Lmctf => "q2-lmctf".to_string(),
            },
            GrappleSelection::Disabled => styles.iter().find(|style| style.unavailable.is_none()).map_or_else(
                || styles.first().map_or_else(String::new, |style| style.id.clone()),
                |style| style.id.clone(),
            ),
        };
        let style = if self.value(StartupSelectionField::GrappleStyle) == "native" {
            default_style
        } else {
            self.value(StartupSelectionField::GrappleStyle).to_string()
        };
        let hook_unavailable = if styles.iter().any(|style| style.unavailable.is_none()) {
            None
        } else {
            Some("Install a game or mod that provides a hook".to_string())
        };
        let has_grenade_source = offhand_grenade_source(&self.current_catalog, &current).is_some();
        let grenades = if self.value(StartupSelectionField::Grenades) == "native" {
            match &defaults.hand_grenades {
                HandGrenadeSelection::Enabled { .. } => "enabled",
                HandGrenadeSelection::Disabled => "disabled",
            }
            .to_string()
        } else {
            self.value(StartupSelectionField::Grenades).to_string()
        };
        let environment_product = self.current_catalog.product("q2-rerelease-baseq2")?.clone();
        let mut rows = vec![
            StartupSelectionRow {
                id: StartupSelectionField::Environment,
                label: "Environment".to_string(),
                value: self.value(StartupSelectionField::Environment).to_string(),
                choices: vec![
                    choice("disabled", "Off", None),
                    choice("audio-content", "Game default", None),
                    choice(
                        "q2-rerelease-baseq2",
                        "Quake II environments",
                        if unavailable(&environment_product).is_none() {
                            None
                        } else {
                            Some("Requires Quake II rerelease data".to_string())
                        },
                    ),
                ],
            },
            StartupSelectionRow {
                id: StartupSelectionField::Doppler,
                label: "Doppler".to_string(),
                value: self.value(StartupSelectionField::Doppler).to_string(),
                choices: vec![choice("source", "Game default", None), choice("disabled", "Off", None)],
            },
            StartupSelectionRow {
                id: StartupSelectionField::Product,
                label: "Game / mod".to_string(),
                value: self.value(StartupSelectionField::Product).to_string(),
                choices: self.current_catalog.products.iter().map(product_choice).collect(),
            },
            StartupSelectionRow {
                id: StartupSelectionField::MapProduct,
                label: "Map content".to_string(),
                value: self.value(StartupSelectionField::MapProduct).to_string(),
                choices: self.current_catalog.products.iter().map(product_choice).collect(),
            },
            StartupSelectionRow {
                id: StartupSelectionField::Map,
                label: "Starting map".to_string(),
                value: self.value(StartupSelectionField::Map).to_string(),
                choices: self.maps()?,
            },
            StartupSelectionRow {
                id: StartupSelectionField::Movement,
                label: "Movement".to_string(),
                value: self.value(StartupSelectionField::Movement).to_string(),
                choices: movement_choices,
            },
            StartupSelectionRow {
                id: StartupSelectionField::Character,
                label: "Character source".to_string(),
                value: self.value(StartupSelectionField::Character).to_string(),
                choices: character_choices,
            },
            StartupSelectionRow {
                id: StartupSelectionField::Model,
                label: "Character model".to_string(),
                value: self.value(StartupSelectionField::Model).to_string(),
                choices: self.models()?,
            },
            StartupSelectionRow {
                id: StartupSelectionField::Weapons,
                label: "Weapons".to_string(),
                value: self.value(StartupSelectionField::Weapons).to_string(),
                choices: vec![native_weapons]
                    .into_iter()
                    .chain(
                        self.current_catalog
                            .products
                            .iter()
                            .filter(|product| supports_selected_weapon_product(&product.expectation))
                            .map(product_choice),
                    )
                    .collect(),
            },
        ];
        let enemies_value = monster_value;
        rows.push(StartupSelectionRow {
            id: StartupSelectionField::Enemies,
            label: "Monsters".to_string(),
            value: enemies_value,
            choices: vec![
                native_monsters,
                choice(
                    "custom",
                    &if self.value(StartupSelectionField::Enemies) == "custom" {
                        format!("{source_label} (custom)")
                    } else {
                        "Custom roster".to_string()
                    },
                    if current.expectation.family == GameFamily::Q3 {
                        Some("This map has no supported authored monster roster".to_string())
                    } else {
                        None
                    },
                ),
            ]
            .into_iter()
            .chain(
                monster_source
                    .choices
                    .into_iter()
                    .filter(|source| source.id != "native")
                    .map(|source| StartupSelectionChoice {
                        unavailable: if current.expectation.family == GameFamily::Q3 {
                            Some("This map has no supported authored monster roster".to_string())
                        } else {
                            source.unavailable
                        },
                        ..source
                    }),
            )
            .collect(),
        });
        rows.push(StartupSelectionRow {
            id: StartupSelectionField::Grapple,
            label: "Hook".to_string(),
            value: placement.clone(),
            choices: vec![
                choice("disabled", "Off", None),
                choice("slot", "Weapon slot", hook_unavailable.clone()),
                choice("offhand", "Offhand", hook_unavailable),
            ],
        });
        if placement != "disabled" {
            rows.push(StartupSelectionRow {
                id: StartupSelectionField::GrappleStyle,
                label: "Hook style".to_string(),
                value: style,
                choices: styles
                    .into_iter()
                    .map(|style| choice(&style.id, &style.title, style.unavailable))
                    .collect(),
            });
        }
        rows.push(StartupSelectionRow {
            id: StartupSelectionField::Grenades,
            label: "Offhand grenades".to_string(),
            value: grenades,
            choices: vec![
                choice("disabled", "Off", None),
                choice(
                    "enabled",
                    "On",
                    if has_grenade_source {
                        None
                    } else {
                        Some("Requires Quake II grenade assets".to_string())
                    },
                ),
            ],
        });
        rows.push(StartupSelectionRow {
            id: StartupSelectionField::Mode,
            label: "Game mode".to_string(),
            value: self.value(StartupSelectionField::Mode).to_string(),
            choices: vec![
                choice("singleplayer", "Single player", None),
                choice("coop", "Cooperative", None),
                choice("deathmatch", "Deathmatch", None),
            ],
        });
        let program = source_program_product(&self.current_catalog, &current.expectation.id)?.clone();
        let mode = match self.value(StartupSelectionField::Mode) {
            "deathmatch" => "deathmatch",
            "coop" => "coop",
            _ => "singleplayer",
        };
        let mut rule_choices = vec![choice("standard", "Standard", None)];
        for (rule, label) in [("tag", "Tag"), ("deathball", "DeathBall"), ("horde", "Horde")] {
            let family = program.expectation.family.to_string();
            rule_choices.push(choice(
                rule,
                label,
                match_mode_unavailable(&MatchModeSelection {
                    family: &family,
                    edition: &program.expectation.edition,
                    campaign: &program.expectation.campaign,
                    mode,
                    rules: rule,
                }),
            ));
        }
        for (rule, label) in [("ctf", "Q2 Capture the Flag"), ("lmctf", "Loki's Minions CTF")] {
            let reason = if current.expectation.family != GameFamily::Q2 || current.expectation.edition != "classic" {
                Some("Requires a classic Quake II campaign".to_string())
            } else if self.value(StartupSelectionField::Mode) != "deathmatch" {
                Some("Requires deathmatch mode".to_string())
            } else {
                unavailable(self.current_catalog.product(&format!("q2-classic-{rule}"))?)
            };
            rule_choices.push(choice(rule, label, reason));
        }
        rows.push(StartupSelectionRow {
            id: StartupSelectionField::Rules,
            label: "Match rules".to_string(),
            value: self.value(StartupSelectionField::Rules).to_string(),
            choices: rule_choices,
        });
        rows.push(StartupSelectionRow {
            id: StartupSelectionField::Skill,
            label: "Difficulty".to_string(),
            value: self.value(StartupSelectionField::Skill).to_string(),
            choices: vec![
                choice("0", "Easy", None),
                choice("1", "Normal", None),
                choice("2", "Hard", None),
                choice("3", "Nightmare", None),
            ],
        });
        rows.push(StartupSelectionRow {
            id: StartupSelectionField::Seats,
            label: "Local players".to_string(),
            value: self.value(StartupSelectionField::Seats).to_string(),
            choices: ["1", "2", "3", "4"].into_iter().map(plain_choice).collect(),
        });
        rows.push(StartupSelectionRow {
            id: StartupSelectionField::Renderer,
            label: "Renderer".to_string(),
            value: self.value(StartupSelectionField::Renderer).to_string(),
            choices: vec![choice("gl", "OpenGL", None), choice("cpu", "Software", None)],
        });
        Ok(rows)
    }

    /// Select one draft choice.
    pub fn select(&mut self, field: StartupSelectionField, id: &str) -> Result<(), StartupSelectionError> {
        let selected = self
            .draft_rows()?
            .into_iter()
            .find(|row| row.id == field)
            .and_then(|row| row.choices.into_iter().find(|option| option.id == id))
            .ok_or_else(|| StartupSelectionError::Failed(format!("Unknown {} selection: {id}", field.as_str())))?;
        if let Some(reason) = selected.unavailable {
            return Err(StartupSelectionError::Failed(reason));
        }
        if field == StartupSelectionField::Enemies && id != "native" && id != "custom" {
            self.select_monster_source(id)?;
            return Ok(());
        }
        self.values.insert(field, id.to_string());
        if field == StartupSelectionField::Product {
            let expectation = self.product(StartupSelectionField::Product)?.expectation.clone();
            if expectation.family != GameFamily::Q1 || expectation.edition == "quakeworld" {
                self.initial.q1_protocol = None;
            }
            self.values.insert(StartupSelectionField::MapProduct, id.to_string());
            let map = self.default_map()?;
            self.values.insert(StartupSelectionField::Map, map);
            if expectation.family == GameFamily::Q3 {
                self.values.insert(StartupSelectionField::Enemies, "native".to_string());
            }
        }
        if field == StartupSelectionField::MapProduct {
            let map = self.default_map()?;
            self.values.insert(StartupSelectionField::Map, map);
        }
        if field == StartupSelectionField::Mode || field == StartupSelectionField::Product {
            let product_id = self.value(StartupSelectionField::Product).to_string();
            let product = source_program_product(&self.current_catalog, &product_id)?.clone();
            let rules = self.value(StartupSelectionField::Rules).to_string();
            if ["standard", "ctf", "lmctf", "tag", "deathball", "horde"].contains(&rules.as_str()) {
                let mode = match self.value(StartupSelectionField::Mode) {
                    "deathmatch" => "deathmatch",
                    "coop" => "coop",
                    _ => "singleplayer",
                };
                let family = product.expectation.family.to_string();
                if match_mode_unavailable(&MatchModeSelection {
                    family: &family,
                    edition: &product.expectation.edition,
                    campaign: &product.expectation.campaign,
                    mode,
                    rules: &rules,
                })
                .is_some()
                {
                    self.values.insert(StartupSelectionField::Rules, "standard".to_string());
                }
            }
        }
        if field == StartupSelectionField::Model {
            let character = self.value(StartupSelectionField::Character).to_string();
            self.selected_models.insert(character, id.to_string());
        }
        if field == StartupSelectionField::Character {
            let family = self.product(StartupSelectionField::Character)?.expectation.family;
            let preferred = self.selected_models.get(id).cloned().unwrap_or_else(|| match family {
                GameFamily::Q1 => "player".to_string(),
                GameFamily::Q2 => "male".to_string(),
                GameFamily::Q3 => "sarge".to_string(),
            });
            let models = self.models()?;
            let model = models.iter().find(|model| model.id == preferred).map_or_else(
                || models.first().map_or_else(String::new, |model| model.id.clone()),
                |model| model.id.clone(),
            );
            self.values.insert(StartupSelectionField::Model, model.clone());
            self.selected_models.insert(id.to_string(), model);
        }
        Ok(())
    }

    /// Override the display draft.
    pub fn set_display(&mut self, width: u32, height: u32, gamma: f64) -> Result<(), StartupSelectionError> {
        if !(1..=16384).contains(&width)
            || !(1..=16384).contains(&height)
            || !gamma.is_finite()
            || !(0.5..=3.0).contains(&gamma)
        {
            return Err(StartupSelectionError::InvalidDisplay);
        }
        self.display = DisplayDraft { width, height, gamma };
        self.display_overrides_consumed = true;
        Ok(())
    }

    /// Draft options.
    pub fn options(&self) -> Result<ApplicationOptions, StartupSelectionError> {
        let mode = match self.value(StartupSelectionField::Mode) {
            "singleplayer" => GameMode::Singleplayer,
            "coop" => GameMode::Coop,
            "deathmatch" => GameMode::Deathmatch,
            _ => return Err(StartupSelectionError::InvalidSettings),
        };
        let renderer = match self.value(StartupSelectionField::Renderer) {
            "gl" => Renderer::Gl,
            "cpu" => Renderer::Cpu,
            _ => return Err(StartupSelectionError::InvalidSettings),
        };
        let rules = match self.value(StartupSelectionField::Rules) {
            "standard" => MatchRules::Standard,
            "ctf" => MatchRules::Ctf,
            "lmctf" => MatchRules::Lmctf,
            "tag" => MatchRules::Tag,
            "deathball" => MatchRules::Deathball,
            "horde" => MatchRules::Horde,
            _ => return Err(StartupSelectionError::InvalidSettings),
        };
        let skill: u8 = self.value(StartupSelectionField::Skill).parse().unwrap_or(99);
        if skill > 3 {
            return Err(StartupSelectionError::InvalidSettings);
        }
        let mut options = self.initial.clone();
        options.weapon_behavior = None;
        let keep_q1 = matches!(options.network, Network::NativeServer { .. })
            && self.hosting().is_ok_and(|hosting| hosting.q1_protocol.is_some())
            && options.q1_protocol.is_some();
        if !keep_q1 {
            options.q1_protocol = None;
        }
        let mut mods: Vec<AppModSelection> = self.initial.mods.clone();
        if let Some(behavior) = &self.initial.weapon_behavior {
            mods.push(AppModSelection {
                product: behavior.product.clone(),
                id: behavior.id.clone(),
            });
        }
        if let Some(selected) = &self.selected_mods {
            mods = selected.enabled().iter().map(to_app).collect();
        }
        options.mods = mods;
        self.apply_selected_server_profile(&mut options);
        options.product = self.value(StartupSelectionField::Product).to_string();
        options.map_product = Some(self.value(StartupSelectionField::MapProduct).to_string());
        options.map = self.value(StartupSelectionField::Map).to_string();
        options.movement = options_family(self.product(StartupSelectionField::Movement)?.expectation.family);
        options.movement_product = Some(self.value(StartupSelectionField::Movement).to_string());
        options.character = options_family(self.product(StartupSelectionField::Character)?.expectation.family);
        options.character_model = self.value(StartupSelectionField::Model).to_string();
        options.mode = mode;
        options.rules = Some(rules);
        options.skill = skill;
        options.seats = self.value(StartupSelectionField::Seats).parse().unwrap_or(1);
        options.renderer = renderer;
        options.width = self.display.width;
        options.height = self.display.height;
        options.gamma = self.display.gamma;
        if self.display_overrides_consumed {
            options.display_overrides = crate::options::DisplayOverrides::default();
        }
        Ok(options)
    }

    /// Human-readable draft summary.
    pub fn summary(&mut self) -> Result<Vec<String>, StartupSelectionError> {
        let mut lines: Vec<String> = self
            .draft_rows()?
            .into_iter()
            .filter(|row| row.id != StartupSelectionField::Renderer)
            .map(|row| {
                format!(
                    "{}: {}",
                    row.label,
                    row.choices
                        .iter()
                        .find(|choice| choice.id == row.value)
                        .map_or_else(|| row.value.clone(), |choice| choice.label.clone())
                )
            })
            .collect();
        lines.push("Pickups: authored map items supply the selected arsenal through its admitted mappings. Independent pickup replacement is not implemented.".to_string());
        Ok(lines)
    }

    /// Weapon binding items for the selected arsenal.
    pub fn binding_items(&self) -> Result<Vec<WeaponBindingItem>, StartupSelectionError> {
        let product = if self.value(StartupSelectionField::Weapons) == "native" {
            self.product(StartupSelectionField::Product)?.clone()
        } else {
            let id = self.value(StartupSelectionField::Weapons).to_string();
            self.current_catalog.require(&id)?.clone()
        };
        Ok(base_weapon_binding_items(
            weapon_family(product.expectation.family),
            &product.expectation.campaign,
            &product.expectation.edition,
        ))
    }

    /// Binding capabilities for the resolved equipment.
    pub fn binding_capabilities(&mut self) -> Result<BindingCapabilities, StartupSelectionError> {
        let options = self.options()?;
        let catalog = self.current_catalog.clone();
        let base = self
            .collaborators
            .application_preset(&catalog, &options, None, None)
            .map_err(StartupSelectionError::Failed)?;
        let equipment = self.selected_equipment(base.equipment.clone())?;
        let family = self
            .current_catalog
            .require(base.engine_behavior.content.as_str())?
            .expectation
            .family;
        Ok(BindingCapabilities {
            chat: family != GameFamily::Q1,
            score_command: match family {
                GameFamily::Q2 => Some(ScoreCommand::Score),
                GameFamily::Q3 => Some(ScoreCommand::Scores),
                GameFamily::Q1 => None,
            },
            offhand_grapple: matches!(
                &equipment.grapple,
                GrappleSelection::Enabled {
                    binding: GrappleBinding::Offhand,
                    ..
                }
            ),
            offhand_grenades: matches!(&equipment.hand_grenades, HandGrenadeSelection::Enabled { .. }),
        })
    }

    fn selected_equipment(&self, baseline: EquipmentSelection) -> Result<EquipmentSelection, StartupSelectionError> {
        let mut equipment = baseline;
        if self.value(StartupSelectionField::Grapple) == "disabled" {
            equipment.grapple = disabled_equipment().grapple;
        } else {
            let binding = if self.value(StartupSelectionField::Grapple) == "native" {
                match &equipment.grapple {
                    GrappleSelection::Enabled {
                        binding: GrappleBinding::Slot,
                        ..
                    } => "slot",
                    GrappleSelection::Enabled {
                        binding: GrappleBinding::Offhand,
                        ..
                    } => "offhand",
                    GrappleSelection::Disabled => "disabled",
                }
            } else {
                self.value(StartupSelectionField::Grapple)
            };
            if binding != "disabled" {
                let slot = match binding {
                    "slot" => GrappleBinding::Slot,
                    "offhand" => GrappleBinding::Offhand,
                    _ => return Err(StartupSelectionError::Failed("Invalid hook placement".to_string())),
                };
                let styles = self.hook_styles();
                let id = if self.value(StartupSelectionField::GrappleStyle) == "native" {
                    match &equipment.grapple {
                        GrappleSelection::Enabled { mechanic, .. } => match mechanic {
                            GrappleMechanicDetail::Q3Qvm { profile } => profile.id.clone(),
                            GrappleMechanicDetail::Q1Threewave { .. } => "q1-threewave".to_string(),
                            GrappleMechanicDetail::Q2Ctf { .. } => "q2-ctf".to_string(),
                            GrappleMechanicDetail::Q2Lmctf => "q2-lmctf".to_string(),
                        },
                        GrappleSelection::Disabled => styles
                            .iter()
                            .find(|style| style.unavailable.is_none())
                            .map_or_else(String::new, |style| style.id.clone()),
                    }
                } else {
                    self.value(StartupSelectionField::GrappleStyle).to_string()
                };
                let style = styles.iter().find(|style| style.id == id);
                match style {
                    Some(style) if style.unavailable.is_none() => match &style.selection {
                        GrappleSelection::Enabled { source, mechanic, .. } => {
                            equipment.grapple = GrappleSelection::Enabled {
                                source: source.clone(),
                                binding: slot,
                                mechanic: mechanic.clone(),
                            };
                        }
                        GrappleSelection::Disabled => {
                            return Err(StartupSelectionError::Failed(
                                "Select an installed hook style".to_string(),
                            ));
                        }
                    },
                    Some(style) => {
                        return Err(StartupSelectionError::Failed(
                            style
                                .unavailable
                                .clone()
                                .unwrap_or_else(|| "Select an installed hook style".to_string()),
                        ));
                    }
                    None => {
                        return Err(StartupSelectionError::Failed(
                            "Select an installed hook style".to_string(),
                        ))
                    }
                }
            }
        }
        if self.value(StartupSelectionField::Grenades) == "disabled" {
            equipment.hand_grenades = disabled_equipment().hand_grenades;
        } else if self.value(StartupSelectionField::Grenades) == "enabled" {
            let product = offhand_grenade_source(&self.current_catalog, self.product(StartupSelectionField::Product)?)
                .ok_or_else(|| {
                    StartupSelectionError::Failed("Offhand grenades require Quake II grenade assets".to_string())
                })?
                .clone();
            let edition = match product.expectation.edition.as_str() {
                "classic" => SourceEdition::Classic,
                "rerelease" => SourceEdition::Rerelease,
                _ => {
                    return Err(StartupSelectionError::Failed(
                        "Invalid grenade asset source".to_string(),
                    ))
                }
            };
            equipment.hand_grenades = HandGrenadeSelection::Enabled {
                source: ProviderReference {
                    provider: equipment_providers().hand_grenades,
                    content: product.id.clone(),
                },
                edition,
                initial_ammo: 5.0,
                capacity: 50.0,
            };
        }
        Ok(equipment)
    }

    /// Resolve the draft to launch options and a recipe.
    pub fn resolve(&mut self) -> Result<StartupLaunch, StartupSelectionError> {
        if self.value(StartupSelectionField::Mode) == "deathmatch"
            || self.value(StartupSelectionField::Rules) != "standard"
        {
            self.prepare_map_classnames(None)?;
        }
        for row in self.draft_rows()? {
            let selected = row.choices.iter().find(|choice| choice.id == row.value);
            match selected {
                Some(found) if found.unavailable.is_none() => {}
                found => {
                    let reason = found
                        .and_then(|choice| choice.unavailable.clone())
                        .unwrap_or_else(|| "choose an installed option".to_string());
                    return Err(StartupSelectionError::Failed(format!("{}: {reason}", row.label)));
                }
            }
        }
        let options = self.options()?;
        let catalog = self.current_catalog.clone();
        let base = self
            .collaborators
            .application_preset(&catalog, &options, None, None)
            .map_err(StartupSelectionError::Failed)?;
        let map_content = self.geometry()?.clone();
        let movement_product = self.product(StartupSelectionField::Movement)?.clone();
        let character_product = self.product(StartupSelectionField::Character)?.clone();
        let movement = provider_reference(
            &format!("{}:movement", movement_product.expectation.family),
            &movement_product.expectation.id,
        );
        let character = provider_reference(
            &format!("{}:character", character_product.expectation.family),
            &character_product.expectation.id,
        );
        let movement_timing = native_provider_timing(
            &movement,
            movement_product.expectation.family,
            movement_product.expectation.edition == "rerelease",
        );
        let character_timing = native_provider_timing(
            &character,
            character_product.expectation.family,
            character_product.expectation.edition == "rerelease",
        );
        let timing: Vec<qa_content::contract::ProviderTiming> = base
            .timing
            .iter()
            .map(|profile| {
                if profile.provider == movement.provider {
                    if movement_product.expectation.edition == "quakeworld" {
                        qa_content::contract::ProviderTiming {
                            clock: ClockProfile::Q1Quakeworld {
                                maximum_command_milliseconds: 255.0,
                            },
                            ..movement_timing.clone()
                        }
                    } else {
                        movement_timing.clone()
                    }
                } else if profile.provider == character.provider {
                    if character_product.expectation.edition == "quakeworld" {
                        qa_content::contract::ProviderTiming {
                            clock: ClockProfile::Q1Quakeworld {
                                maximum_command_milliseconds: 50.0,
                            },
                            ..character_timing.clone()
                        }
                    } else {
                        character_timing.clone()
                    }
                } else {
                    profile.clone()
                }
            })
            .collect();
        let environment = match self.value(StartupSelectionField::Environment) {
            "audio-content" => EnvironmentSelection::AudioContent,
            "disabled" => EnvironmentSelection::Disabled,
            selected => EnvironmentSelection::Selected {
                resource: ResourceRequest {
                    content: ContentId(self.current_catalog.require(selected)?.expectation.id.clone()),
                    path: "sound/default.environments".to_string(),
                },
            },
        };
        let doppler = match self.value(StartupSelectionField::Doppler) {
            "source" => DopplerSelection::Source,
            "disabled" => DopplerSelection::Disabled,
            _ => return Err(StartupSelectionError::Failed("Invalid Doppler selection".to_string())),
        };
        let mut preset = base.clone();
        preset.timing = timing;
        preset.presentation.doppler = doppler;
        preset.presentation.environment = environment;
        let character_selection = CharacterSelection {
            definition: character.clone(),
            appearance: provider_reference(
                &format!(
                    "{}:model/{}",
                    contract_family(options.character),
                    options.character_model
                ),
                character.content.as_str(),
            ),
        };
        let mut selections = preset_choice(preset.id.clone());
        selections.map = LaunchSelection::Selected(MapSelection {
            geometry: ResourceRequest {
                content: ContentId(map_content.expectation.id.clone()),
                path: options.map.clone(),
            },
            entities: base.map.entities.clone(),
        });
        selections.movement = LaunchSelection::Selected(movement);
        selections.character = LaunchSelection::Selected(character_selection);
        selections.campaign = LaunchSelection::Selected(if options.mode == GameMode::Deathmatch {
            CampaignSelection::None
        } else {
            base.campaign.clone()
        });
        if self.value(StartupSelectionField::Weapons) != "native" {
            let product = self
                .current_catalog
                .require(self.value(StartupSelectionField::Weapons))?
                .clone();
            selections.weapons = LaunchSelection::Selected(vec![canonical_weapon_source(
                &base.map.entities,
                &provider_reference(
                    &format!("{}:official", product.expectation.family),
                    &product.expectation.id,
                ),
                &catalog,
            )?]);
        }
        if self.value(StartupSelectionField::Enemies) == "custom" {
            selections.enemies = LaunchSelection::Selected(self.effective_monster_roster(true)?);
        }
        selections.equipment = LaunchSelection::Selected(self.selected_equipment(base.equipment.clone())?);
        let weapons = self.collaborators.launch_weapons();
        let compat = self.collaborators.launch_compat();
        let recipe = resolve_launch(
            &ResolveLaunchOptions {
                choice: &selections,
                preset: &preset,
                catalog: &catalog,
                id: None,
                mounts: None,
            },
            &*weapons,
            &*compat,
        )?;
        let launch_mods: Vec<ModSelection> = options.mods.iter().map(to_contract).collect();
        let recipe = self
            .collaborators
            .apply_mods(recipe, &self.mod_choices, &launch_mods)
            .map_err(StartupSelectionError::Failed)?;
        let mut options = options;
        options.explicit_rules.skill = true;
        options.explicit_rules.mode = true;
        options.explicit_rules.capacity = true;
        Ok(StartupLaunch {
            options,
            recipe,
            q3_product: self.q3_product,
            team_arena_skirmish: None,
        })
    }
}

/// Monster source row (donor `monsterSourceRow` return).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonsterSourceRow {
    /// Display label.
    pub label: String,
    /// Selected source id.
    pub value: String,
    /// Available sources.
    pub choices: Vec<StartupSelectionChoice>,
}

/// Map scan cache key (donor `` `${map.source}:${map.memberIndex}` ``).
fn map_key(source: &str, member_index: Option<u64>) -> String {
    match member_index {
        Some(index) => format!("{source}:{index}"),
        None => format!("{source}:null"),
    }
}

/// Read a map member's bytes: loose files read directly, PACK members slice
/// the container, and other archives decode the entry.
fn read_map_member(
    source: &str,
    member_index: Option<u64>,
    files: &mut HashMap<String, FileSource>,
) -> Result<(Vec<u8>, u64), StartupSelectionError> {
    let path = Path::new(source);
    let file = match files.get(source) {
        Some(file) => file,
        None => {
            files.insert(
                source.to_string(),
                FileSource::new(path).map_err(|error| StartupSelectionError::Failed(error.to_string()))?,
            );
            &files[source]
        }
    };
    let Some(member) = member_index else {
        let length = file.byte_length();
        let bytes = file
            .read(0, length)
            .map_err(|error| StartupSelectionError::Failed(error.to_string()))?;
        return Ok((bytes, length));
    };
    let archive = open_archive(path, None).map_err(|error| StartupSelectionError::Failed(error.to_string()))?;
    let entry = archive
        .entries
        .get(member as usize)
        .ok_or_else(|| StartupSelectionError::Failed(format!("Missing map archive entry: {source}")))?;
    match entry {
        ArchiveEntry::Pak(entry) => {
            let bytes = file
                .read(entry.data_offset, entry.byte_length)
                .map_err(|error| StartupSelectionError::Failed(error.to_string()))?;
            Ok((bytes, entry.byte_length))
        }
        ArchiveEntry::Zip(_) => {
            let bytes = read_entry(&archive, EntryRef::Ordinal(member as usize))
                .map_err(|error| StartupSelectionError::Failed(error.to_string()))?;
            let length = bytes.len() as u64;
            Ok((bytes, length))
        }
    }
}

impl ModMenuService for StartupSelectionModel {
    fn rows(&self) -> Vec<ModMenuRow> {
        self.selected_mods.as_ref().map_or_else(Vec::new, |selected| {
            selected
                .entries()
                .into_iter()
                .filter(|entry| entry.purpose == ModPurpose::Addition)
                .map(|entry| ModMenuRow {
                    id: mod_selection_key(&entry.selection).unwrap_or_default(),
                    title: entry.title.clone(),
                    source: entry.source_title.clone(),
                    enabled: selected.has(&entry.selection).unwrap_or(false),
                    unavailable: match &entry.availability {
                        ModAvailability::Available => None,
                        ModAvailability::Unavailable { reason } => Some(reason.clone()),
                    },
                })
                .collect()
        })
    }

    fn set_enabled(&mut self, id: &str, enabled: bool) {
        let result = (|| -> Result<(), StartupSelectionError> {
            let selected = self
                .selected_mods
                .as_mut()
                .ok_or_else(|| StartupSelectionError::Failed("Mod discovery has not finished".to_string()))?;
            selected.set_enabled(&read_mod_selection(id)?, enabled)?;
            Ok(())
        })();
        self.mod_status = result.err().map_or_else(String::new, |error| error.to_string());
    }

    fn refresh(&mut self) {
        if self.refreshing_mods {
            return;
        }
        self.refreshing_mods = true;
        self.mod_status = "Reading installed mods...".to_string();
        let result = (|| -> Result<(), StartupSelectionError> {
            let user_root = self.current_catalog.user_content_root.as_ref().map(PathBuf::from);
            let catalog = discover_installed_content(&DiscoverContentOptions {
                corpus_root: PathBuf::from(&self.current_catalog.corpus_root),
                user_content_root: user_root,
                products: None,
                generation: self.current_catalog.generation + 1,
                discover_mods: true,
                remote_content: None,
            })?;
            self.prepare_mods(&catalog)?;
            self.prepare_hook_styles(&catalog)?;
            self.current_catalog = catalog;
            Ok(())
        })();
        self.mod_status = result.err().map_or_else(String::new, |error| error.to_string());
        self.refreshing_mods = false;
    }

    fn status(&self) -> String {
        self.mod_status.clone()
    }
}

/// Build a startup selection model over discovered content (donor
/// `createStartupSelection`). Sync port: discovery and map scans are sync.
pub fn create_startup_selection(
    options: ApplicationOptions,
    collaborators: Box<dyn StartupSelectionCollaborators>,
) -> Result<StartupSelectionModel, StartupSelectionError> {
    let user_root = options
        .user_content_root
        .clone()
        .map(PathBuf::from)
        .unwrap_or_else(default_user_content_root);
    let catalog = discover_installed_content(&DiscoverContentOptions {
        corpus_root: PathBuf::from(&options.corpus_root),
        user_content_root: Some(user_root),
        products: None,
        generation: 0,
        discover_mods: true,
        remote_content: None,
    })?;
    let mut model = StartupSelectionModel::new(catalog, options, collaborators)?;
    model.prepare_maps()?;
    Ok(model)
}

#[cfg(test)]
mod tests {
    use qa_content::catalog::BehaviorMounts;
    use qa_content::catalog::CatalogArchive;
    use qa_content::catalog::ProductExpectation;
    use qa_content::catalog::QvmCompatRole;
    use qa_content::contract::ContentDigest;
    use qa_content::contract::ProviderTiming;
    use qa_content::contract::QvmAbiProfile;
    use qa_content::contract::ResourceRequest as ContractResourceRequest;

    use super::*;

    struct FakeWeapons;

    impl LaunchWeaponSources for FakeWeapons {
        fn canonical_weapon_source(
            &self,
            _map: &ProviderReference,
            weapon: &ProviderReference,
            _catalog: &InstalledCatalog,
        ) -> Result<ProviderReference, CatalogError> {
            Ok(weapon.clone())
        }

        fn selected_weapon_resources(
            &self,
            _map: &ProviderReference,
            _weapons: &[ProviderReference],
            _catalog: &InstalledCatalog,
        ) -> Result<Vec<ContractResourceRequest>, CatalogError> {
            Ok(Vec::new())
        }

        fn selected_weapon_timing(
            &self,
            _map: &ProviderReference,
            _weapons: &[ProviderReference],
            _catalog: &InstalledCatalog,
        ) -> Result<Vec<ProviderTiming>, CatalogError> {
            Ok(Vec::new())
        }

        fn admit_weapon_timing(
            &self,
            _timing: &mut Vec<ProviderTiming>,
            _weapon: &ProviderTiming,
        ) -> Result<(), CatalogError> {
            Ok(())
        }

        fn weapon_provider_ids(&self) -> Vec<ProviderId> {
            Vec::new()
        }
    }

    struct FakeCompat;

    impl LaunchQvmCompatibility for FakeCompat {
        fn read_qvm_compatibility(
            &self,
            _mounts: &dyn BehaviorMounts,
            _artifact_path: &str,
            _digest: &ContentDigest,
            _role: QvmCompatRole,
        ) -> Result<QvmAbiProfile, CatalogError> {
            Err(CatalogError::Invalid("no qvm in tests".to_string()))
        }
    }

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

    fn expectation(id: &str, family: GameFamily, edition: &str, campaign: &str) -> ProductExpectation {
        ProductExpectation {
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
        }
    }

    fn product(id: &str, family: GameFamily, edition: &str, campaign: &str, installed: bool) -> CatalogProduct {
        CatalogProduct {
            id: ContentId(id.to_string()),
            expectation: expectation(id, family, edition, campaign),
            availability: if installed {
                ProductAvailability::Installed
            } else {
                ProductAvailability::Missing {
                    requirements: vec!["game data".to_string()],
                }
            },
            archives: Vec::new(),
            loose_root: None,
            user_content: None,
            maps: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    fn catalog() -> InstalledCatalog {
        InstalledCatalog::new(
            "/tmp/qa-startup-selection-test".to_string(),
            vec![
                product("q1-classic-id1", GameFamily::Q1, "classic", "id1", true),
                product("q1-quakeworld", GameFamily::Q1, "quakeworld", "qw", true),
                product("q2-classic-baseq2", GameFamily::Q2, "classic", "baseq2", true),
                product("q2-classic-ctf", GameFamily::Q2, "classic", "ctf", true),
                product("q2-classic-lmctf", GameFamily::Q2, "classic", "lmctf", false),
                product("q2-rerelease-baseq2", GameFamily::Q2, "rerelease", "baseq2", false),
                product("q3-baseq3", GameFamily::Q3, "classic", "baseq3", true),
            ],
            Vec::<CatalogArchive>::new(),
            0,
            None,
        )
        .unwrap()
    }

    fn options() -> ApplicationOptions {
        ApplicationOptions {
            product: "q1-classic-id1".to_string(),
            map: "maps/start.bsp".to_string(),
            character_model: "player".to_string(),
            ..ApplicationOptions::default()
        }
    }

    fn model() -> StartupSelectionModel {
        let mut model = StartupSelectionModel::new(catalog(), options(), Box::new(FakeCollaborators)).unwrap();
        model.prepare_maps().unwrap();
        model
    }

    #[test]
    fn menu_defaults_honor_launch_movement_override() {
        let mut initial = options();
        initial.movement_product = Some("q1-quakeworld".to_string());
        let model = StartupSelectionModel::new(
            catalog(),
            initial,
            Box::new(crate::bootstrap::windowed_preset::WindowedPresetCollaborators),
        )
        .unwrap();
        assert_eq!(model.value(StartupSelectionField::Movement), "q1-quakeworld");
    }

    #[test]
    fn builds_draft_values() {
        let model = model();
        assert_eq!(model.value(StartupSelectionField::Product), "q1-classic-id1");
        assert_eq!(model.value(StartupSelectionField::MapProduct), "q1-classic-id1");
        assert_eq!(model.value(StartupSelectionField::Movement), "q1-classic-id1");
        assert_eq!(model.value(StartupSelectionField::Skill), "1");
        assert_eq!(model.value(StartupSelectionField::Renderer), "gl");
    }

    #[test]
    fn hosting_reports_offline_ports() {
        let model = model();
        let hosting = model.hosting().unwrap();
        assert_eq!((hosting.kind, hosting.port), (StartupHostingKind::Offline, 26000));
        assert_eq!(hosting.q1_protocol, Some(ProtocolIdentity::Q1Netquake));
    }

    #[test]
    fn set_hosting_validates_ports_and_flips_mode() {
        let mut model = model();
        assert_eq!(
            model.set_hosting(StartupHosting {
                kind: StartupHostingKind::NativeServer,
                port: 0,
                q1_protocol: None
            }),
            Err(StartupSelectionError::InvalidPort)
        );
        model
            .set_hosting(StartupHosting {
                kind: StartupHostingKind::NativeServer,
                port: 26000,
                q1_protocol: Some(ProtocolIdentity::Q1Netquake),
            })
            .unwrap();
        assert_eq!(model.value(StartupSelectionField::Mode), "coop");
        assert_eq!(model.hosting().unwrap().kind, StartupHostingKind::NativeServer);
    }

    #[test]
    fn select_validates_and_cascades() {
        let mut model = model();
        model.select(StartupSelectionField::Skill, "3").unwrap();
        assert_eq!(model.value(StartupSelectionField::Skill), "3");
        assert!(model.select(StartupSelectionField::Skill, "9").is_err());
        model.select(StartupSelectionField::Product, "q3-baseq3").unwrap();
        assert_eq!(model.value(StartupSelectionField::MapProduct), "q3-baseq3");
        assert_eq!(model.value(StartupSelectionField::Enemies), "native");
    }

    #[test]
    fn set_display_validates_ranges() {
        let mut model = model();
        assert_eq!(
            model.set_display(0, 600, 1.0),
            Err(StartupSelectionError::InvalidDisplay)
        );
        assert_eq!(
            model.set_display(960, 600, 9.0),
            Err(StartupSelectionError::InvalidDisplay)
        );
        model.set_display(1280, 720, 1.2).unwrap();
        assert_eq!(model.options().unwrap().width, 1280);
    }

    #[test]
    fn rows_cover_all_fields() {
        let mut model = model();
        let rows = model.draft_rows().unwrap();
        let ids: Vec<StartupSelectionField> = rows.iter().map(|row| row.id).collect();
        assert!(ids.contains(&StartupSelectionField::Product));
        assert!(ids.contains(&StartupSelectionField::Rules));
        assert!(ids.contains(&StartupSelectionField::Renderer));
        let rules = rows.iter().find(|row| row.id == StartupSelectionField::Rules).unwrap();
        let horde = rules.choices.iter().find(|choice| choice.id == "horde").unwrap();
        assert!(horde.unavailable.is_some());
    }

    #[test]
    fn summary_lists_rows_and_pickups() {
        let mut model = model();
        let summary = model.summary().unwrap();
        assert!(summary.iter().any(|line| line.starts_with("Game / mod:")));
        assert!(summary.last().unwrap().starts_with("Pickups:"));
    }

    #[test]
    fn presets_filter_expected_installed() {
        let model = model();
        let presets = model.presets();
        assert!(presets.iter().any(|preset| preset.id == "q1-classic-id1"));
        assert!(!presets.iter().any(|preset| preset.id == "q2-rerelease-baseq2"));
        let q1 = presets.iter().find(|preset| preset.id == "q1-classic-id1").unwrap();
        assert_eq!(q1.default_skill, "1");
    }

    #[test]
    fn resolve_preset_rejects_unknown() {
        let mut model = model();
        assert_eq!(
            model.resolve_preset("q9-elsewhere", None, None),
            Err(StartupSelectionError::Failed(
                "Installed official campaign preset unavailable: q9-elsewhere".to_string()
            ))
        );
        assert_eq!(
            model.resolve_preset("q1-classic-id1", Some(9), None),
            Err(StartupSelectionError::Failed("Invalid preset difficulty".to_string()))
        );
    }

    #[test]
    fn resolve_requires_an_installed_map() {
        let mut model = model();
        let error = model.resolve().err().unwrap();
        assert_eq!(
            error,
            StartupSelectionError::Failed("Starting map: choose an installed option".to_string())
        );
    }

    #[test]
    fn monster_rows_offer_native_default() {
        let mut model = model();
        let rows = model.monster_roster_rows().unwrap();
        assert_eq!(rows[0].label, "Unmatched classes");
        assert_eq!(rows[0].value, "native");
        model.select_monster(None, "native").unwrap();
    }

    #[test]
    fn mods_menu_reports_before_discovery() {
        let model = StartupSelectionModel::new(catalog(), options(), Box::new(FakeCollaborators)).unwrap();
        assert!(ModMenuService::rows(&model).is_empty());
        assert_eq!(ModMenuService::status(&model), "");
    }
}
