//! Server settings: definitions, cvar owners, profiles, collections.
//!
//! Donor provenance: `src/settings/server/{types,common,cvars,profile,q1,
//! q2,q3,rotation,selection,library,lmctf,lmctf-cvars,q2-owner}.ts`. Same
//! setting ids, labels, ranges, defaults, apply timing, profile documents,
//! map-name rules, and Q2/LMCTF cvar tables. Configuration files, console
//! bridges, and UI writes share [`parse_server_setting`], as in the donor.
//!
//! Live rule bindings (`bindQ2ServerCvars`, `bindQ2PlayerCvars`, LMCTF rule
//! mirrors) and `restoreQ2ServerCvars` live in
//! [`super::q2_owner`]; registration here uses the same
//! name/default/flag tables.

use qa_core::cvar::q2_flags;
use qa_core::cvar::CvarRegistry;

use super::config::ConfigStore;
use super::json::{parse_json, stringify_pretty, Json};
use super::SettingsError;

/// Setting id, always `server:`-prefixed (donor `ServerSettingId`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ServerSettingId(String);

impl ServerSettingId {
    /// Validate a `server:`-prefixed id.
    pub fn new(id: &str) -> Result<Self, SettingsError> {
        if id.starts_with("server:") && id.len() > "server:".len() {
            Ok(Self(id.to_string()))
        } else {
            Err(SettingsError::BadValue(format!("Invalid server setting id {id:?}")))
        }
    }

    /// The id text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// When a write takes effect (donor `ServerApplyAt`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerApplyAt {
    /// Applies immediately.
    Live,
    /// Applies at the next match.
    NextMatch,
    /// Applies at the next map.
    NextMap,
    /// Applies at the next restart.
    Restart,
}

/// Cvar write target (donor `ServerSettingTarget`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerSettingTarget {
    /// Whole cvar value.
    Value {
        /// Cvar name.
        name: String,
    },
    /// One bit of a flag cvar.
    Bit {
        /// Cvar name.
        name: String,
        /// Flag mask.
        mask: u32,
        /// Whether the set bit means disabled.
        inverted: bool,
    },
}

impl ServerSettingTarget {
    #[must_use]
    fn name(&self) -> &str {
        match self {
            Self::Value { name } | Self::Bit { name, .. } => name,
        }
    }
}

/// Setting control widget (donor `CvarSettingSpec` kinds).
#[derive(Debug, Clone, PartialEq)]
pub enum SettingControl {
    /// Boolean toggle.
    Toggle,
    /// Numeric slider.
    Slider {
        /// Minimum value.
        minimum: f64,
        /// Maximum value.
        maximum: f64,
        /// Step.
        step: f64,
        /// Whether only integers validate.
        integer: bool,
    },
    /// Fixed choice list.
    Choice {
        /// Available choices.
        choices: Vec<SettingChoice>,
    },
    /// Free text entry.
    TextEntry {
        /// Maximum length in characters.
        maximum_length: usize,
    },
}

/// One setting choice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingChoice {
    /// Choice id.
    pub id: String,
    /// Choice label.
    pub label: String,
}

/// One server setting definition.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerSettingDefinition {
    /// Setting id.
    pub id: ServerSettingId,
    /// Display label.
    pub label: String,
    /// Description.
    pub description: String,
    /// When writes take effect.
    pub apply_at: ServerApplyAt,
    /// Default value.
    pub default_value: String,
    /// Cvar target.
    pub target: ServerSettingTarget,
    /// Control widget.
    pub control: SettingControl,
}

/// Numeric-limit setting (donor `serverLimit`).
#[must_use]
pub fn server_limit(
    id: &str,
    name: &str,
    label: &str,
    default_value: f64,
    description: &str,
    apply_at: ServerApplyAt,
    integer: bool,
) -> ServerSettingDefinition {
    ServerSettingDefinition {
        id: ServerSettingId::new(id).expect("server setting id"),
        label: label.to_string(),
        description: description.to_string(),
        default_value: number_text(default_value),
        apply_at,
        target: ServerSettingTarget::Value { name: name.to_string() },
        control: SettingControl::Slider {
            minimum: 0.0,
            maximum: 2_147_483_647.0,
            step: if integer { 1.0 } else { 0.25 },
            integer,
        },
    }
}

/// Boolean setting (donor `serverToggle`).
#[must_use]
pub fn server_toggle(
    id: &str,
    target: ServerSettingTarget,
    label: &str,
    default_value: bool,
    description: &str,
    apply_at: ServerApplyAt,
) -> ServerSettingDefinition {
    ServerSettingDefinition {
        id: ServerSettingId::new(id).expect("server setting id"),
        target,
        label: label.to_string(),
        description: description.to_string(),
        apply_at,
        default_value: if default_value {
            "1".to_string()
        } else {
            "0".to_string()
        },
        control: SettingControl::Toggle,
    }
}

/// Friendly-fire setting (donor `serverFriendlyFire`).
#[must_use]
pub fn server_friendly_fire(target: ServerSettingTarget, default_value: bool) -> ServerSettingDefinition {
    server_toggle(
        "server:friendly-fire",
        target,
        "Friendly fire",
        default_value,
        "Allow damage to teammates using the selected combat rules. Self-damage follows those rules.",
        ServerApplyAt::Live,
    )
}

fn number_text(value: f64) -> String {
    if value == value.trunc() && value.abs() < 1e15 {
        format!("{}", value.trunc() as i64)
    } else {
        format!("{value}")
    }
}

/// A named collection of definitions from one component owner.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerSettingCollection {
    /// Collection id.
    pub id: String,
    /// Owned definitions.
    pub definitions: Vec<ServerSettingDefinition>,
}

/// Desired vs effective values (donor `ServerSettingValues`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerSettingValues {
    /// Desired value (latched when staged).
    pub desired: String,
    /// Effective value.
    pub effective: String,
}

/// Setting status with pending state (donor `ServerSettingStatus`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerSettingStatus {
    /// Desired value.
    pub desired: String,
    /// Effective value.
    pub effective: String,
    /// Whether a staged value differs from the effective one.
    pub pending: bool,
    /// When the setting applies.
    pub apply_at: ServerApplyAt,
}

/// Setting bound to the cvar owner (donor `BoundServerSetting`).
#[derive(Debug, Clone, PartialEq)]
pub struct ServerBinding {
    /// Setting definition.
    pub definition: ServerSettingDefinition,
    /// Whether the owner initializes (writes apply immediately).
    pub initializing: bool,
}

fn parse_flag_number(text: &str) -> f64 {
    text.parse::<f64>().unwrap_or(0.0)
}

fn projected(target: &ServerSettingTarget, value: &str) -> String {
    match target {
        ServerSettingTarget::Value { .. } => value.to_string(),
        ServerSettingTarget::Bit { mask, inverted, .. } => {
            #[allow(clippy::cast_possible_truncation)]
            let bits = parse_flag_number(value).trunc() as i32 as u32;
            let enabled = (bits & mask) != 0;
            if enabled != *inverted {
                "1".to_string()
            } else {
                "0".to_string()
            }
        }
    }
}

fn merged(target: &ServerSettingTarget, current: &str, value: &str) -> String {
    match target {
        ServerSettingTarget::Value { .. } => value.to_string(),
        ServerSettingTarget::Bit { mask, inverted, .. } => {
            let enabled = (value == "1") != *inverted;
            #[allow(clippy::cast_possible_truncation)]
            let current = parse_flag_number(current).trunc() as i64 as u32;
            (if enabled { current | mask } else { current & !mask }).to_string()
        }
    }
}

/// Read desired/effective values through the cvar registry.
pub fn read_server_setting(
    binding: &ServerBinding,
    cvars: &CvarRegistry,
) -> Result<ServerSettingStatus, SettingsError> {
    let snapshot = cvars.get(binding.definition.target.name()).ok_or_else(|| {
        SettingsError::BadValue(format!(
            "Server setting has no cvar owner: {}",
            binding.definition.target.name()
        ))
    })?;
    let effective = projected(&binding.definition.target, &snapshot.value);
    let desired = projected(
        &binding.definition.target,
        snapshot.latched_value.as_deref().unwrap_or(&snapshot.value),
    );
    Ok(ServerSettingStatus {
        pending: desired != effective,
        apply_at: binding.definition.apply_at,
        desired,
        effective,
    })
}

/// Validate and write a setting value (donor `writeServerSetting`).
pub fn write_server_setting(
    binding: &ServerBinding,
    cvars: &mut CvarRegistry,
    value: &str,
) -> Result<ServerSettingStatus, SettingsError> {
    let parsed = parse_server_setting(&binding.definition, value)?;
    let name = binding.definition.target.name().to_string();
    let snapshot = cvars
        .get(&name)
        .ok_or_else(|| SettingsError::BadValue(format!("Server setting has no cvar owner: {name}")))?;
    let latched_or_value = snapshot.latched_value.clone().unwrap_or_else(|| snapshot.value.clone());
    let desired = merged(&binding.definition.target, &latched_or_value, &parsed);
    if binding.initializing {
        cvars.set(&name, &desired, true)?;
        return read_server_setting(binding, cvars);
    }
    if binding.definition.apply_at != ServerApplyAt::Live {
        cvars.stage(&name, &desired)?;
        return read_server_setting(binding, cvars);
    }
    let effective = merged(&binding.definition.target, &snapshot.value, &parsed);
    cvars.set(&name, &effective, true)?;
    if desired != effective {
        cvars.stage(&name, &desired)?;
    }
    read_server_setting(binding, cvars)
}

/// Validate raw input for a definition (donor `parseServerSetting`).
pub fn parse_server_setting(definition: &ServerSettingDefinition, input: &str) -> Result<String, SettingsError> {
    match &definition.control {
        SettingControl::Toggle => {
            if input == "1" || input == "true" {
                Ok("1".to_string())
            } else if input == "0" || input == "false" {
                Ok("0".to_string())
            } else {
                Err(SettingsError::BadValue(format!(
                    "{} requires a boolean",
                    definition.label
                )))
            }
        }
        SettingControl::Slider {
            minimum,
            maximum,
            integer,
            ..
        } => {
            let value: f64 = input.parse().unwrap_or(f64::NAN);
            let safe_integer = value.trunc() == value && value.abs() < 9_007_199_254_740_992.0;
            if input.trim().is_empty()
                || !value.is_finite()
                || (*integer && !safe_integer)
                || value < *minimum
                || value > *maximum
            {
                return Err(SettingsError::BadValue(format!(
                    "{} is outside its allowed range",
                    definition.label
                )));
            }
            Ok(number_text(value))
        }
        SettingControl::Choice { choices } => {
            if choices.iter().any(|choice| choice.id == input) {
                Ok(input.to_string())
            } else {
                Err(SettingsError::BadValue(format!("Unknown {} choice", definition.label)))
            }
        }
        SettingControl::TextEntry { maximum_length } => {
            if input.chars().count() > *maximum_length
                || input.chars().any(|character| matches!(character, '\0' | '\r' | '\n'))
            {
                return Err(SettingsError::BadValue(format!(
                    "{} contains invalid text",
                    definition.label
                )));
            }
            Ok(input.to_string())
        }
    }
}

/// Merge collections, rejecting duplicate owners (donor `collectServerSettings`).
pub fn collect_server_settings(
    collections: &[ServerSettingCollection],
) -> Result<Vec<ServerSettingDefinition>, SettingsError> {
    let mut definitions: Vec<ServerSettingDefinition> = Vec::new();
    for collection in collections {
        for definition in &collection.definitions {
            if definitions.iter().any(|existing| existing.id == definition.id) {
                return Err(SettingsError::BadValue(format!(
                    "Two selected components own {}",
                    definition.id.as_str()
                )));
            }
            parse_server_setting(definition, &definition.default_value.clone())?;
            definitions.push(definition.clone());
        }
    }
    Ok(definitions)
}

/// Server profile document (donor `ServerProfile`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerProfile {
    /// Document version (always 1).
    pub version: u32,
    /// Setting overrides.
    pub overrides: Vec<ServerProfileOverride>,
}

/// One profile override.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerProfileOverride {
    /// Setting id.
    pub id: ServerSettingId,
    /// Override value.
    pub value: String,
}

/// Parse a profile document against selected definitions.
pub fn parse_server_profile(
    value: &Json,
    definitions: &[ServerSettingDefinition],
) -> Result<ServerProfile, SettingsError> {
    let members = match value {
        Json::Object(members) => members,
        _ => return Err(SettingsError::BadValue("Unsupported server profile".to_string())),
    };
    let version = members
        .iter()
        .find(|(name, _)| name == "version")
        .map(|(_, value)| value);
    let overrides = members
        .iter()
        .find(|(name, _)| name == "overrides")
        .map(|(_, value)| value);
    let Some(Json::Array(overrides)) = overrides else {
        return Err(SettingsError::BadValue("Unsupported server profile".to_string()));
    };
    if version != Some(&Json::Number(1.0)) {
        return Err(SettingsError::BadValue("Unsupported server profile".to_string()));
    }
    let mut seen: Vec<&ServerSettingId> = Vec::new();
    let mut parsed = Vec::new();
    for entry in overrides {
        let members = match entry {
            Json::Object(members) => members,
            _ => {
                return Err(SettingsError::BadValue(
                    "Server profile setting has no selected owner".to_string(),
                ))
            }
        };
        let id = members.iter().find(|(name, _)| name == "id").map(|(_, value)| value);
        let value = members.iter().find(|(name, _)| name == "value").map(|(_, value)| value);
        let (Some(Json::String(id)), Some(Json::String(value))) = (id, value) else {
            return Err(SettingsError::BadValue(
                "Server profile setting has no selected owner".to_string(),
            ));
        };
        let Some(definition) = definitions.iter().find(|definition| definition.id.as_str() == id) else {
            return Err(SettingsError::BadValue(
                "Server profile setting has no selected owner".to_string(),
            ));
        };
        if seen.contains(&&definition.id) {
            return Err(SettingsError::BadValue(format!(
                "Duplicate server profile setting {}",
                definition.id.as_str()
            )));
        }
        seen.push(&definition.id);
        parsed.push(ServerProfileOverride {
            id: definition.id.clone(),
            value: parse_server_setting(definition, value)?,
        });
    }
    Ok(ServerProfile {
        version: 1,
        overrides: parsed,
    })
}

/// Capture desired values as a profile (donor `captureServerProfile`).
pub fn capture_server_profile(
    bindings: &[ServerBinding],
    cvars: &CvarRegistry,
) -> Result<ServerProfile, SettingsError> {
    let definitions: Vec<ServerSettingDefinition> = bindings.iter().map(|binding| binding.definition.clone()).collect();
    let overrides: Vec<Json> = bindings
        .iter()
        .map(|binding| {
            let status = read_server_setting(binding, cvars)?;
            Ok(Json::Object(vec![
                (
                    "id".to_string(),
                    Json::String(binding.definition.id.as_str().to_string()),
                ),
                ("value".to_string(), Json::String(status.desired)),
            ]))
        })
        .collect::<Result<Vec<_>, SettingsError>>()?;
    parse_server_profile(
        &Json::Object(vec![
            ("version".to_string(), Json::Number(1.0)),
            ("overrides".to_string(), Json::Array(overrides)),
        ]),
        &definitions,
    )
}

/// Apply a profile: resolve every write before mutating any cvar.
pub fn apply_server_profile(
    profile: &ServerProfile,
    bindings: &[ServerBinding],
    cvars: &mut CvarRegistry,
) -> Result<(), SettingsError> {
    let definitions: Vec<ServerSettingDefinition> = bindings.iter().map(|binding| binding.definition.clone()).collect();
    let overrides: Vec<Json> = profile
        .overrides
        .iter()
        .map(|entry| {
            Json::Object(vec![
                ("id".to_string(), Json::String(entry.id.as_str().to_string())),
                ("value".to_string(), Json::String(entry.value.clone())),
            ])
        })
        .collect();
    let validated = parse_server_profile(
        &Json::Object(vec![
            ("version".to_string(), Json::Number(1.0)),
            ("overrides".to_string(), Json::Array(overrides)),
        ]),
        &definitions,
    )?;
    let mut writes: Vec<(&ServerBinding, String)> = Vec::new();
    for entry in &validated.overrides {
        let Some(binding) = bindings.iter().find(|binding| binding.definition.id == entry.id) else {
            return Err(SettingsError::BadValue(format!(
                "No server owner for {}",
                entry.id.as_str()
            )));
        };
        writes.push((binding, entry.value.clone()));
    }
    for (binding, value) in writes {
        write_server_setting(binding, cvars, &value)?;
    }
    Ok(())
}

/// Render a profile document.
#[must_use]
pub fn server_profile_json(profile: &ServerProfile) -> Json {
    Json::Object(vec![
        ("version".to_string(), Json::Number(1.0)),
        (
            "overrides".to_string(),
            Json::Array(
                profile
                    .overrides
                    .iter()
                    .map(|entry| {
                        Json::Object(vec![
                            ("id".to_string(), Json::String(entry.id.as_str().to_string())),
                            ("value".to_string(), Json::String(entry.value.clone())),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

/// Save a profile document.
pub fn save_server_profile(
    store: &ConfigStore,
    name: &str,
    profile: &ServerProfile,
    definitions: &[ServerSettingDefinition],
) -> Result<(), SettingsError> {
    let validated = parse_server_profile(&server_profile_json(profile), definitions)?;
    store.dump(
        name,
        &format!("{}\n", stringify_pretty(&server_profile_json(&validated))),
    )
}

/// Load a profile document, or [`None`] when the file is absent.
pub fn load_server_profile(
    store: &ConfigStore,
    name: &str,
    definitions: &[ServerSettingDefinition],
) -> Result<Option<ServerProfile>, SettingsError> {
    let Some(text) = store.load_text(name)? else {
        return Ok(None);
    };
    parse_server_profile(&parse_json(&text)?, definitions).map(Some)
}

/// Quake teamplay program (donor `q1MatchSettings`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1Program {
    /// Standard Quake.
    Standard,
    /// Rogue mission pack (adds objective modes).
    Rogue,
    /// Capture the flag (no teamplay setting).
    Ctf,
}

/// Quake teamplay setting (donor `q1MatchSettings`).
#[must_use]
pub fn q1_match_settings(program: Q1Program) -> ServerSettingCollection {
    if program == Q1Program::Ctf {
        return ServerSettingCollection {
            id: "q1:match".to_string(),
            definitions: Vec::new(),
        };
    }
    let mut labels = vec!["Off", "No friendly fire", "Friendly fire"];
    if program == Q1Program::Rogue {
        labels.extend(["Tag", "Capture the flag", "One flag CTF", "Three team CTF"]);
    }
    ServerSettingCollection {
        id: "q1:match".to_string(),
        definitions: vec![ServerSettingDefinition {
            id: ServerSettingId::new("server:q1.teamplay").expect("server setting id"),
            label: "Quake teamplay".to_string(),
            description: "Select source team rules for the next map. Rogue objective entities are initialized when the map starts.".to_string(),
            target: ServerSettingTarget::Value { name: "teamplay".to_string() },
            default_value: "0".to_string(),
            apply_at: ServerApplyAt::NextMap,
            control: SettingControl::Choice {
                choices: labels
                    .iter()
                    .enumerate()
                    .map(|(value, label)| SettingChoice { id: value.to_string(), label: (*label).to_string() })
                    .collect(),
            },
        }],
    }
}

/// Quake II time/frag limits (donor `q2LimitSettings`).
#[must_use]
pub fn q2_limit_settings() -> ServerSettingCollection {
    ServerSettingCollection {
        id: "q2:limits".to_string(),
        definitions: vec![
            server_limit(
                "server:time-limit",
                "timelimit",
                "Time limit (minutes)",
                0.0,
                "End the level after this many minutes. Zero disables the limit.",
                ServerApplyAt::Live,
                false,
            ),
            server_limit(
                "server:frag-limit",
                "fraglimit",
                "Frag limit",
                0.0,
                "End the level when a player reaches this score. Zero disables the limit.",
                ServerApplyAt::Live,
                true,
            ),
        ],
    }
}

/// Quake II CTF capture limit (donor `q2CtfCaptureSettings`).
#[must_use]
pub fn q2_ctf_capture_settings() -> ServerSettingCollection {
    ServerSettingCollection {
        id: "q2:ctf-captures".to_string(),
        definitions: vec![server_limit(
            "server:capture-limit",
            "capturelimit",
            "Capture limit",
            0.0,
            "End the level when a team reaches this capture count. Zero disables the limit.",
            ServerApplyAt::Live,
            true,
        )],
    }
}

/// Quake II friendly fire over `dmflags` bit 256, inverted (donor `q2CombatSettings`).
#[must_use]
pub fn q2_combat_settings() -> ServerSettingCollection {
    ServerSettingCollection {
        id: "q2:combat".to_string(),
        definitions: vec![server_friendly_fire(
            ServerSettingTarget::Bit {
                name: "dmflags".to_string(),
                mask: 256,
                inverted: true,
            },
            true,
        )],
    }
}

/// Quake II DeathBall goal limit (donor `q2DeathBallSettings`).
#[must_use]
pub fn q2_deathball_settings() -> ServerSettingCollection {
    ServerSettingCollection {
        id: "q2:deathball".to_string(),
        definitions: vec![server_limit(
            "server:goal-limit",
            "goallimit",
            "Goal limit",
            0.0,
            "End DeathBall when a team reaches this goal score. Zero disables the limit.",
            ServerApplyAt::Live,
            true,
        )],
    }
}

/// Quake II rerelease settings (donor `q2RereleaseSettings`).
#[must_use]
pub fn q2_rerelease_settings() -> ServerSettingCollection {
    let value = |name: &str| ServerSettingTarget::Value { name: name.to_string() };
    ServerSettingCollection {
        id: "q2:rerelease".to_string(),
        definitions: vec![
            server_toggle(
                "server:q2.random-items",
                value("g_dm_random_items"),
                "Random item respawns",
                false,
                "Replace respawning pickups using the source item categories.",
                ServerApplyAt::Live,
            ),
            server_toggle(
                "server:q2.no-quadfire-drop",
                value("g_dm_no_quadfire_drop"),
                "Prevent DualFire drop",
                false,
                "Do not drop active DualFire Damage on death.",
                ServerApplyAt::Live,
            ),
            server_toggle(
                "server:q2.instant-switch",
                value("g_instant_weapon_switch"),
                "Instant weapon switching",
                false,
                "Skip weapon lowering and raising animations.",
                ServerApplyAt::NextMap,
            ),
            server_limit(
                "server:q2.weapon-respawn",
                "g_weapon_respawn_time",
                "Weapon respawn seconds",
                30.0,
                "Delay before a collected deathmatch weapon returns.",
                ServerApplyAt::Live,
                false,
            ),
            server_toggle(
                "server:q2.weapons-stay",
                value("g_dm_weapons_stay"),
                "Weapons stay",
                false,
                "Leave map weapons for other players.",
                ServerApplyAt::Live,
            ),
            server_toggle(
                "server:q2.instant-items",
                value("g_dm_instant_items"),
                "Instant powerups",
                true,
                "Activate powerups on pickup.",
                ServerApplyAt::Live,
            ),
            server_toggle(
                "server:q2.same-level",
                value("g_dm_same_level"),
                "Repeat current level",
                false,
                "Restart the same map after the match.",
                ServerApplyAt::Live,
            ),
            server_toggle(
                "server:q2.no-quad-drop",
                value("g_dm_no_quad_drop"),
                "Prevent Quad drop",
                false,
                "Do not drop active Quad Damage on death.",
                ServerApplyAt::Live,
            ),
            server_toggle(
                "server:q2.no-stack-double",
                value("g_dm_no_stack_double"),
                "Disable stacked Double Damage",
                false,
                "Do not stack Double Damage with other damage multipliers.",
                ServerApplyAt::Live,
            ),
            server_toggle(
                "server:q2.strong-mines",
                value("g_dm_strong_mines"),
                "Strong proximity mines",
                false,
                "Use the source strong mine damage policy.",
                ServerApplyAt::Live,
            ),
            server_toggle(
                "server:q2.squad-respawn",
                value("g_coop_squad_respawn"),
                "Respawn near teammates",
                true,
                "Use the source safe teammate respawn checks.",
                ServerApplyAt::NextMap,
            ),
            server_toggle(
                "server:q2.instanced-items",
                value("g_coop_instanced_items"),
                "Individual cooperative pickups",
                true,
                "Each cooperative player can collect their own copy of an eligible pickup.",
                ServerApplyAt::NextMap,
            ),
            server_toggle(
                "server:q2.coop-lives",
                value("g_coop_enable_lives"),
                "Limited cooperative lives",
                false,
                "Use source cooperative lives and all-dead restart rules.",
                ServerApplyAt::NextMap,
            ),
            server_limit(
                "server:q2.coop-num-lives",
                "g_coop_num_lives",
                "Extra cooperative lives",
                2.0,
                "Additional lives per cooperative player.",
                ServerApplyAt::NextMap,
                true,
            ),
            server_toggle(
                "server:q2.player-collision",
                value("g_coop_player_collision"),
                "Cooperative player collision",
                false,
                "Allow cooperative players to block one another.",
                ServerApplyAt::NextMap,
            ),
            server_toggle(
                "server:q2.force-respawn",
                value("g_dm_force_respawn"),
                "Force deathmatch respawn",
                false,
                "Respawn dead players using the source respawn delay.",
                ServerApplyAt::Live,
            ),
            server_limit(
                "server:q2.respawn-time",
                "g_dm_force_respawn_time",
                "Forced respawn delay",
                0.0,
                "Seconds before forced respawn. Zero uses source timing.",
                ServerApplyAt::Live,
                false,
            ),
            server_toggle(
                "server:q2.no-fall-damage",
                value("g_dm_no_fall_damage"),
                "Disable deathmatch fall damage",
                false,
                "Suppress falling damage in deathmatch.",
                ServerApplyAt::Live,
            ),
            server_toggle(
                "server:q2.farthest-spawn",
                value("g_dm_spawn_farthest"),
                "Farthest deathmatch spawn",
                true,
                "Prefer the spawn farthest from living players.",
                ServerApplyAt::Live,
            ),
            server_toggle(
                "server:q2.allow-exit",
                value("g_dm_allow_exit"),
                "Allow deathmatch exits",
                false,
                "Allow players to activate authored level exits.",
                ServerApplyAt::Live,
            ),
        ],
    }
}

/// Quake II item-spawn `dmflags` bits, read at spawn (donor `q2SpawnSettings`).
#[must_use]
pub fn q2_spawn_settings() -> ServerSettingCollection {
    let bit = |mask: u32| ServerSettingTarget::Bit {
        name: "dmflags".to_string(),
        mask,
        inverted: false,
    };
    ServerSettingCollection {
        id: "q2:item-spawn".to_string(),
        definitions: vec![
            server_toggle(
                "server:q2.no-health",
                bit(1),
                "Exclude health items",
                false,
                "Remove health pickups when the next map is spawned.",
                ServerApplyAt::NextMap,
            ),
            server_toggle(
                "server:q2.no-powerups",
                bit(2),
                "Exclude powerups",
                false,
                "Remove powerup pickups when the next map is spawned.",
                ServerApplyAt::NextMap,
            ),
            server_toggle(
                "server:q2.no-armor",
                bit(2048),
                "Exclude armor items",
                false,
                "Remove armor pickups when the next map is spawned.",
                ServerApplyAt::NextMap,
            ),
        ],
    }
}

/// Quake III product (donor `Product`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3Product {
    /// Base Quake III.
    Base,
    /// Team Arena mission pack.
    MissionPack,
}

/// Quake III limits (donor `q3LimitSettings`; defaults from the donor
/// `q3GameCvarDefinitions`: timelimit 0, fraglimit 20, capturelimit 8).
#[must_use]
pub fn q3_limit_settings(_product: Q3Product) -> ServerSettingCollection {
    ServerSettingCollection {
        id: "q3:limits".to_string(),
        definitions: vec![
            server_limit(
                "server:time-limit",
                "timelimit",
                "Time limit (minutes)",
                0.0,
                "End play after this many minutes. Zero disables the limit.",
                ServerApplyAt::Live,
                true,
            ),
            server_limit(
                "server:frag-limit",
                "fraglimit",
                "Frag limit",
                20.0,
                "End applicable matches at this score. Zero disables the limit.",
                ServerApplyAt::Live,
                true,
            ),
            server_limit(
                "server:capture-limit",
                "capturelimit",
                "Capture limit",
                8.0,
                "End applicable team matches at this capture count. Zero disables the limit.",
                ServerApplyAt::Live,
                true,
            ),
        ],
    }
}

/// Quake III friendly fire (donor `q3CombatSettings`).
#[must_use]
pub fn q3_combat_settings(_product: Q3Product) -> ServerSettingCollection {
    ServerSettingCollection {
        id: "q3:combat".to_string(),
        definitions: vec![server_friendly_fire(
            ServerSettingTarget::Value {
                name: "g_friendlyFire".to_string(),
            },
            false,
        )],
    }
}

/// Quake III match type (donor `q3MatchSettings`; donor `GameType` ids).
#[must_use]
pub fn q3_match_settings(product: Q3Product) -> ServerSettingCollection {
    let mut modes = vec![
        ("0", "Free for all"),
        ("1", "Tournament"),
        ("2", "Single player"),
        ("3", "Team deathmatch"),
        ("4", "Capture the flag"),
    ];
    if product == Q3Product::MissionPack {
        modes.extend([("5", "One flag CTF"), ("6", "Overload"), ("7", "Harvester")]);
    }
    ServerSettingCollection {
        id: "q3:match".to_string(),
        definitions: vec![ServerSettingDefinition {
            id: ServerSettingId::new("server:q3.game-type").expect("server setting id"),
            label: "Q3 match type".to_string(),
            description: "Select source match rules for the next map. Equipment selections stay independent."
                .to_string(),
            target: ServerSettingTarget::Value {
                name: "g_gametype".to_string(),
            },
            default_value: "0".to_string(),
            apply_at: ServerApplyAt::NextMap,
            control: SettingControl::Choice {
                choices: modes
                    .iter()
                    .map(|(id, label)| SettingChoice {
                        id: (*id).to_string(),
                        label: (*label).to_string(),
                    })
                    .collect(),
            },
        }],
    }
}

/// LMCTF rune (donor `LMCTF_RUNES`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LmctfRune {
    /// Rune kind.
    pub kind: &'static str,
    /// Rune bit in the `runes` cvar.
    pub bit: u32,
    /// Display name.
    pub name: &'static str,
}

/// LMCTF runes in donor table order.
pub const LMCTF_RUNES: [LmctfRune; 5] = [
    LmctfRune {
        kind: "damage",
        bit: 1,
        name: "Damage Artifact",
    },
    LmctfRune {
        kind: "haste",
        bit: 4,
        name: "Haste Artifact",
    },
    LmctfRune {
        kind: "resist",
        bit: 2,
        name: "Resist Artifact",
    },
    LmctfRune {
        kind: "regen",
        bit: 8,
        name: "Regen Artifact",
    },
    LmctfRune {
        kind: "vampire",
        bit: 16,
        name: "Vampire Artifact",
    },
];

/// Default LMCTF `runes` mask (donor `createLmctfRules`).
pub const LMCTF_DEFAULT_RUNES: u32 = 15;

/// LMCTF limits (donor `lmctfLimitSettings`).
#[must_use]
pub fn lmctf_limit_settings() -> ServerSettingCollection {
    ServerSettingCollection {
        id: "q2:lmctf-limits".to_string(),
        definitions: vec![
            server_limit(
                "server:time-limit",
                "timelimit",
                "Match time (minutes)",
                0.0,
                "Duration used when the next LMCTF match starts. Does not reset an active countdown.",
                ServerApplyAt::NextMatch,
                true,
            ),
            server_limit(
                "server:frag-limit",
                "fraglimit",
                "Frag limit",
                0.0,
                "Start the final countdown when a player reaches this score. Zero disables the limit.",
                ServerApplyAt::Live,
                true,
            ),
        ],
    }
}

/// LMCTF rune spawns (donor `lmctfRuneSettings`).
#[must_use]
pub fn lmctf_rune_settings() -> ServerSettingCollection {
    ServerSettingCollection {
        id: "q2:lmctf-runes".to_string(),
        definitions: LMCTF_RUNES
            .iter()
            .map(|rune| {
                server_toggle(
                    &format!("server:lmctf.rune.{}", rune.kind),
                    ServerSettingTarget::Bit {
                        name: "runes".to_string(),
                        mask: rune.bit,
                        inverted: false,
                    },
                    rune.name,
                    LMCTF_DEFAULT_RUNES & rune.bit != 0,
                    &format!("Spawn the {} on the next map.", rune.name),
                    ServerApplyAt::NextMap,
                )
            })
            .collect(),
    }
}

/// LMCTF weapon behavior (donor `lmctfWeaponSettings`).
#[must_use]
pub fn lmctf_weapon_settings() -> ServerSettingCollection {
    ServerSettingCollection {
        id: "q2:lmctf-weapons".to_string(),
        definitions: vec![server_toggle(
            "server:lmctf.fast-switch",
            ServerSettingTarget::Value {
                name: "fastswitch".to_string(),
            },
            "Fast weapon switching",
            false,
            "Use the selected LMCTF weapon owner's fast switch behavior.",
            ServerApplyAt::Live,
        )],
    }
}

/// LMCTF console cvar names (donor `LMCTF_CONSOLE_NAMES`).
pub const LMCTF_CONSOLE_NAMES: [&str; 14] = [
    "ctfflags",
    "refset",
    "runes",
    "skinset",
    "disabled_weps",
    "timelimit",
    "fraglimit",
    "countdown_time",
    "flag_init",
    "fastswitch",
    "autolock",
    "refpassword",
    "rcon_password",
    "maplist_file",
];

/// Register the LMCTF console cvars with donor defaults and flags.
///
/// The donor also mirrors these into a live rules object; headless code
/// reads them from the registry instead.
pub fn register_lmctf_console_rules(cvars: &mut CvarRegistry) -> Result<(), SettingsError> {
    for (name, value, flags) in [
        ("ctfflags", "0", q2_flags::SERVER_INFO),
        ("refset", "0", q2_flags::SERVER_INFO),
        ("runes", "15", q2_flags::SERVER_INFO),
        ("skinset", "0", q2_flags::SERVER_INFO),
        ("disabled_weps", "0", 0),
        ("timelimit", "0", q2_flags::SERVER_INFO),
        ("fraglimit", "0", q2_flags::SERVER_INFO),
        ("countdown_time", "15", 0),
        ("flag_init", "0", 0),
        ("fastswitch", "0", 0),
        ("autolock", "0", 0),
        ("refpassword", "", 0),
        ("rcon_password", "", 0),
        ("maplist_file", "maplist.txt", 0),
    ] {
        cvars.register(name, value, flags)?;
    }
    Ok(())
}

/// Quake II map rotation settings (donor `q2RotationSettings`).
#[must_use]
pub fn q2_rotation_settings(rerelease: bool) -> ServerSettingCollection {
    let mut definitions = vec![ServerSettingDefinition {
        id: ServerSettingId::new("server:map-rotation").expect("server setting id"),
        label: "Map rotation".to_string(),
        description: "Ordered map names. An empty list follows each map's authored exit.".to_string(),
        control: SettingControl::TextEntry { maximum_length: 2048 },
        default_value: String::new(),
        apply_at: ServerApplyAt::Live,
        target: ServerSettingTarget::Value {
            name: if rerelease {
                "g_map_list".to_string()
            } else {
                "sv_maplist".to_string()
            },
        },
    }];
    if rerelease {
        definitions.push(server_toggle(
            "server:map-rotation-shuffle",
            ServerSettingTarget::Value {
                name: "g_map_list_shuffle".to_string(),
            },
            "Shuffle after last map",
            false,
            "Reshuffle at the end of the rotation and keep the new order for the next cycle.",
            ServerApplyAt::Live,
        ));
    }
    ServerSettingCollection {
        id: "q2:rotation".to_string(),
        definitions,
    }
}

/// Validate a rotation map name (donor `rotationMapName`).
pub fn rotation_map_name(value: &str) -> Result<String, SettingsError> {
    let name = value.trim().strip_prefix("maps/").unwrap_or_else(|| value.trim());
    let name = if name.len() >= 4 && name[name.len() - 4..].eq_ignore_ascii_case(".bsp") {
        &name[..name.len() - 4]
    } else {
        name
    };
    if name.is_empty() || name.len() > 127 || !valid_map_name(name) {
        return Err(SettingsError::BadValue(
            "Enter a map name, such as q2dm1 or q64/outpost".to_string(),
        ));
    }
    Ok(name.to_string())
}

fn valid_map_name(name: &str) -> bool {
    !name.is_empty()
        && name.split('/').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
        })
}

/// Move a rotation entry (donor `moveRotationMap`); invalid moves keep the list.
#[must_use]
pub fn move_rotation_map(maps: &[String], index: isize, direction: isize) -> Vec<String> {
    if !matches!(direction, -1 | 1) || index < 0 {
        return maps.to_vec();
    }
    let destination = index + direction;
    if index as usize >= maps.len() || destination < 0 || destination as usize >= maps.len() {
        return maps.to_vec();
    }
    let mut result = maps.to_vec();
    result.swap(index as usize, destination as usize);
    result
}

/// Content provider reference for collection selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRef {
    /// Provider id (`q1:`, `q2:`, or `q3:`-prefixed).
    pub provider: String,
    /// Content markers (`:rogue:`, `:rerelease:`, `missionpack`, ...).
    pub content: String,
}

/// Select server collections for installed providers (donor `serverDefinitionsForSelection`).
pub fn server_definitions_for_selection(
    source: &ProviderRef,
    match_ref: &ProviderRef,
    combat: &ProviderRef,
) -> Result<Vec<ServerSettingDefinition>, SettingsError> {
    if source.provider.starts_with("q1:") {
        let program = if source.content.contains(":rogue:") {
            Q1Program::Rogue
        } else if source.content.contains(":ctf:") {
            Q1Program::Ctf
        } else {
            Q1Program::Standard
        };
        return collect_server_settings(&[q1_match_settings(program)]);
    }
    if source.provider.starts_with("q2:") {
        let rerelease = source.content.contains(":rerelease:");
        let mut collections =
            q2_server_setting_collections(&match_ref.provider, combat.provider.starts_with("q2:"), rerelease);
        collections.push(q2_rotation_settings(rerelease));
        return collect_server_settings(&collections);
    }
    if source.provider.starts_with("q3:") {
        let product = if match_ref.content.contains("missionpack") {
            Q3Product::MissionPack
        } else {
            Q3Product::Base
        };
        let mut collections = vec![q3_limit_settings(product), q3_match_settings(product)];
        if combat.provider.starts_with("q3:") {
            collections.push(q3_combat_settings(product));
        }
        return collect_server_settings(&collections);
    }
    Ok(Vec::new())
}

/// Quake II collections for a match mode (donor `q2ServerSettingCollections`).
#[must_use]
pub fn q2_server_setting_collections(
    match_provider: &str,
    q2_combat: bool,
    rerelease: bool,
) -> Vec<ServerSettingCollection> {
    let mut collections = vec![q2_spawn_settings()];
    if q2_combat {
        collections.push(q2_combat_settings());
    }
    if match_provider == "q2:lmctf" {
        collections.extend([lmctf_limit_settings(), lmctf_rune_settings(), lmctf_weapon_settings()]);
    } else {
        collections.push(q2_limit_settings());
    }
    if match_provider == "q2:ctf" {
        collections.push(q2_ctf_capture_settings());
    }
    if match_provider == "q2:deathball" {
        collections.push(q2_deathball_settings());
    }
    if rerelease {
        collections.push(q2_rerelease_settings());
    }
    collections
}

/// Register the Quake II server cvars (donor `registerQ2ServerCvars`).
pub fn register_q2_server_cvars(cvars: &mut CvarRegistry, match_provider: &str) -> Result<(), SettingsError> {
    let rerelease = cvars.dialect() == qa_core::cmd::Dialect::Q2Rerelease;
    if match_provider == "q2:deathball" {
        cvars.register("dball_team1_skin", "male/ctf_r", 0)?;
        cvars.register("dball_team2_skin", "male/ctf_b", 0)?;
        cvars.register("goallimit", "0", 0)?;
    }
    if rerelease {
        for (name, value, flags) in [
            ("g_instant_weapon_switch", "0", q2_flags::LATCH),
            ("g_weapon_respawn_time", "30", 0),
            ("g_dm_weapons_stay", "0", 0),
            ("g_dm_instant_items", "1", 0),
            ("g_dm_same_level", "0", 0),
            ("g_no_mines", "0", 0),
            ("g_no_nukes", "0", 0),
            ("g_no_spheres", "0", 0),
            ("g_dm_random_items", "0", 0),
            ("g_dm_no_quadfire_drop", "0", 0),
            ("g_dm_no_quad_drop", "0", 0),
            ("g_dm_no_stack_double", "0", 0),
            ("g_dm_strong_mines", "0", 0),
        ] {
            cvars.register(name, value, flags)?;
        }
    }
    cvars.register("sv_gravity", "800", 0)?;
    cvars.register("sv_airaccelerate", "0", 0)?;
    cvars.register("dmflags", "0", q2_flags::SERVER_INFO)?;
    cvars.register("timelimit", "0", q2_flags::SERVER_INFO)?;
    cvars.register("fraglimit", "0", q2_flags::SERVER_INFO)?;
    for (name, value, flags) in [
        ("maxspectators", "4", q2_flags::SERVER_INFO),
        ("flood_msgs", "4", 0),
        ("flood_persecond", "4", 0),
        ("flood_waitdelay", "10", 0),
        ("sv_rollspeed", "200", 0),
        ("sv_rollangle", "2", 0),
        ("run_pitch", "0.002", 0),
        ("run_roll", "0.005", 0),
        ("bob_up", "0.005", 0),
        ("bob_pitch", "0.002", 0),
        ("bob_roll", "0.002", 0),
    ] {
        cvars.register(name, value, flags)?;
    }
    for name in ["password", "spectator_password"] {
        cvars.register(name, "", q2_flags::USER_INFO)?;
    }
    cvars.register("needpass", "0", q2_flags::SERVER_INFO)?;
    cvars.register("cheats", "0", q2_flags::SERVER_INFO | q2_flags::LATCH)?;
    for axis in ["x", "y", "z"] {
        cvars.register(&format!("gun_{axis}"), "0", 0)?;
    }
    cvars.register(if rerelease { "g_map_list" } else { "sv_maplist" }, "", 0)?;
    if rerelease {
        cvars.register("g_map_list_shuffle", "0", 0)?;
        for (name, value, flags) in [
            ("g_coop_squad_respawn", "1", q2_flags::LATCH),
            ("g_coop_instanced_items", "1", q2_flags::LATCH),
            ("g_coop_enable_lives", "0", q2_flags::LATCH),
            ("g_coop_player_collision", "0", q2_flags::LATCH),
            ("g_dm_force_respawn", "0", 0),
            ("g_dm_no_fall_damage", "0", 0),
            ("g_dm_spawn_farthest", "1", 0),
            ("g_dm_allow_exit", "0", 0),
            ("g_coop_num_lives", "2", q2_flags::LATCH),
            ("g_dm_force_respawn_time", "0", 0),
        ] {
            cvars.register(name, value, flags)?;
        }
    }
    if match_provider == "q2:ctf" {
        cvars.register("capturelimit", "0", q2_flags::SERVER_INFO)?;
    }
    if match_provider == "q2:lmctf" {
        register_lmctf_console_rules(cvars)?;
    }
    Ok(())
}

/// Effective deathmatch flags with rerelease toggles folded in (donor `q2SourceDeathmatchFlags`).
#[must_use]
pub fn q2_source_deathmatch_flags(cvars: &CvarRegistry) -> i32 {
    #[allow(clippy::cast_possible_truncation)]
    let mut flags = cvars.variable_value("dmflags").trunc() as i32;
    if cvars.dialect() != qa_core::cmd::Dialect::Q2Rerelease {
        return flags;
    }
    for (name, mask, inverted) in [
        ("g_dm_weapons_stay", 4, false),
        ("g_dm_instant_items", 16, false),
        ("g_dm_same_level", 32, false),
        ("g_dm_no_quad_drop", 16384, true),
    ] {
        let enabled = (cvars.variable_value(name) != 0.0) != inverted;
        if enabled {
            flags |= mask;
        } else {
            flags &= !mask;
        }
    }
    flags
}

/// Saved server profile entry (donor `ServerProfileEntry`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerProfileEntry {
    /// Profile name (file stem).
    pub name: String,
    /// Store-relative path.
    pub path: String,
}

fn valid_profile_name(file: &str) -> bool {
    if !file.ends_with(".json") || file.len() <= ".json".len() {
        return false;
    }
    let stem = &file[..file.len() - ".json".len()];
    if stem.is_empty() || stem.len() > 32 || !stem.as_bytes()[0].is_ascii_alphanumeric() {
        return false;
    }
    stem.bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

/// List saved server profiles (donor `listServerProfiles`).
pub fn list_server_profiles(store: &ConfigStore) -> Result<Vec<ServerProfileEntry>, SettingsError> {
    let dir = super::config::settings_path(&store.root, "servers")?;
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(SettingsError::from(error)),
    };
    let mut profiles = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.file_type()?.is_file() && valid_profile_name(&name) {
            profiles.push(ServerProfileEntry {
                name: name[..name.len() - 5].to_string(),
                path: format!("servers/{name}"),
            });
        }
    }
    profiles.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(profiles)
}

/// Donor dialect spelling for a registry (donor `CvarRegistry["dialect"]`).
fn cvar_dialect_name(dialect: qa_core::cmd::Dialect) -> &'static str {
    match dialect {
        qa_core::cmd::Dialect::Q1Netquake => "q1-netquake",
        qa_core::cmd::Dialect::Q1Quakeworld => "q1-quakeworld",
        qa_core::cmd::Dialect::Q2Classic => "q2-classic",
        qa_core::cmd::Dialect::Q2Rerelease => "q2-rerelease",
        qa_core::cmd::Dialect::Q3 => "q3",
    }
}

/// Restore saved cvar values into a registry (donor `restoreSaveState`).
///
/// The donor restores full registry state (variables, order, latched
/// values, modification counters); the Rust registry recomputes derived
/// state on write, so this restores names, values, reset values, flags,
/// and latched values through the public registry API and enforces the
/// donor's dialect check. Variables the save does not mention keep their
/// current values, matching the donor's missing-variable merge.
pub fn restore_cvar_save_state(
    cvars: &mut CvarRegistry,
    saved: &qa_world::save::value::SaveJson,
) -> Result<(), SettingsError> {
    use qa_world::save::value::SaveReader;
    let reader = SaveReader::new(saved);
    let dialect = reader
        .field("dialect")
        .string()
        .map_err(|error| SettingsError::BadValue(error.to_string()))?;
    if dialect != cvar_dialect_name(cvars.dialect()) {
        return Err(SettingsError::BadValue(format!("Saved cvars use dialect {dialect}")));
    }
    let variables = reader
        .field("variables")
        .list(|entry| {
            entry.nullable(|item| {
                let name = item.field("name").string()?;
                let value = item.field("value").string()?;
                let reset = item.field("resetValue").string()?;
                let latched = item.field("latchedValue").nullable(|field| field.string())?;
                let flags = item.field("flags").integer(0)?;
                Ok((name, value, reset, latched, flags))
            })
        })
        .map_err(|error: qa_world::WorldError| SettingsError::BadValue(error.to_string()))?;
    for saved in variables.into_iter().flatten() {
        let (name, value, reset, latched, flags) = saved;
        let flags = u32::try_from(flags)
            .map_err(|_| SettingsError::BadValue(format!("Saved cvar {name} has flags out of range")))?;
        if cvars.get(&name).is_none() {
            cvars.register(&name, &reset, flags)?;
        }
        cvars.set(&name, &value, true)?;
        if let Some(latched) = latched {
            cvars.stage(&name, &latched)?;
        }
    }
    Ok(())
}

/// Restore saved Quake II server cvars (donor `restoreQ2ServerCvars`).
///
/// The donor validates the save into a scratch registry, then merges
/// current-only variables ahead of the saved order; value restore keeps
/// current-only variables untouched, which is the same observable merge.
pub fn restore_q2_server_cvars(
    cvars: &mut CvarRegistry,
    saved: &qa_world::save::value::SaveJson,
) -> Result<(), SettingsError> {
    restore_cvar_save_state(cvars, saved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::cmd::Dialect;

    fn binding(definition: ServerSettingDefinition) -> ServerBinding {
        ServerBinding {
            definition,
            initializing: false,
        }
    }

    #[test]
    fn validates_setting_values() {
        let toggle = server_toggle(
            "server:test",
            ServerSettingTarget::Value {
                name: "test".to_string(),
            },
            "Test",
            false,
            "Test setting.",
            ServerApplyAt::Live,
        );
        assert_eq!(parse_server_setting(&toggle, "true").unwrap(), "1");
        assert!(parse_server_setting(&toggle, "maybe").is_err());
        let limit = server_limit("server:test", "test", "Test", 0.0, "Test.", ServerApplyAt::Live, true);
        assert_eq!(parse_server_setting(&limit, "12").unwrap(), "12");
        assert!(parse_server_setting(&limit, "2.5").is_err());
        assert!(parse_server_setting(&limit, "").is_err());
        let q1 = q1_match_settings(Q1Program::Rogue);
        assert_eq!(q1.definitions[0].default_value, "0");
        if let SettingControl::Choice { choices } = &q1.definitions[0].control {
            assert_eq!(choices.len(), 7);
        } else {
            panic!("teamplay must be a choice");
        }
        assert!(collect_server_settings(&[q1.clone(), q1]).is_err());
    }

    #[test]
    fn reads_writes_and_stages_cvars() {
        let mut cvars = CvarRegistry::new(Dialect::Q2Classic);
        cvars.register("dmflags", "0", q2_flags::SERVER_INFO).unwrap();
        let combat = binding(q2_combat_settings().definitions.into_iter().next().unwrap());
        let status = write_server_setting(&combat, &mut cvars, "1").unwrap();
        assert_eq!(status.effective, "1");
        assert_eq!(cvars.get("dmflags").unwrap().value, "0");
        let spawn = binding(q2_spawn_settings().definitions.into_iter().next().unwrap());
        let status = write_server_setting(&spawn, &mut cvars, "1").unwrap();
        assert!(status.pending);
        assert_eq!(cvars.get("dmflags").unwrap().value, "0");
    }

    #[test]
    fn profiles_round_trip_through_the_store() {
        let root = std::env::temp_dir().join(format!("qa-server-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let store = ConfigStore::new(root.clone());
        let mut cvars = CvarRegistry::new(Dialect::Q2Classic);
        register_q2_server_cvars(&mut cvars, "q2:ctf").unwrap();
        let definitions = collect_server_settings(&[q2_limit_settings(), q2_ctf_capture_settings()]).unwrap();
        let bindings: Vec<ServerBinding> = definitions
            .iter()
            .map(|definition| ServerBinding {
                definition: definition.clone(),
                initializing: false,
            })
            .collect();
        write_server_setting(&bindings[0], &mut cvars, "15").unwrap();
        let profile = capture_server_profile(&bindings, &cvars).unwrap();
        save_server_profile(&store, "servers/match.json", &profile, &definitions).unwrap();
        let loaded = load_server_profile(&store, "servers/match.json", &definitions)
            .unwrap()
            .unwrap();
        assert_eq!(loaded, profile);
        write_server_setting(&bindings[0], &mut cvars, "0").unwrap();
        apply_server_profile(&loaded, &bindings, &mut cvars).unwrap();
        assert_eq!(cvars.get("timelimit").unwrap().value, "15");
        let listed = list_server_profiles(&store).unwrap();
        assert_eq!(
            listed,
            vec![ServerProfileEntry {
                name: "match".to_string(),
                path: "servers/match.json".to_string()
            }]
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn rotation_and_selection_follow_the_donor() {
        assert_eq!(rotation_map_name("maps/q2dm1.bsp").unwrap(), "q2dm1");
        assert!(rotation_map_name("../escape").is_err());
        let maps = vec!["a".to_string(), "b".to_string()];
        assert_eq!(move_rotation_map(&maps, 0, 1), vec!["b".to_string(), "a".to_string()]);
        assert_eq!(move_rotation_map(&maps, 5, 1), maps);
        let source = ProviderRef {
            provider: "q2:base".to_string(),
            content: ":rerelease:".to_string(),
        };
        let match_ref = ProviderRef {
            provider: "q2:ctf".to_string(),
            content: String::new(),
        };
        let combat = ProviderRef {
            provider: "q2:combat".to_string(),
            content: String::new(),
        };
        let definitions = server_definitions_for_selection(&source, &match_ref, &combat).unwrap();
        assert!(definitions
            .iter()
            .any(|definition| definition.id.as_str() == "server:capture-limit"));
        assert!(definitions
            .iter()
            .any(|definition| definition.id.as_str() == "server:map-rotation"));
        assert_eq!(q2_source_deathmatch_flags(&CvarRegistry::new(Dialect::Q2Classic)), 0);
        assert_eq!(LMCTF_CONSOLE_NAMES.len(), 14);
        assert_eq!(q3_match_settings(Q3Product::MissionPack).definitions.len(), 1);
    }
}
