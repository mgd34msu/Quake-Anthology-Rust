//! Gameplay settings bindings.
//!
//! Ported from the TypeScript donor's `src/ui/settings/gameplay.ts`. Shared
//! binding types come from the settings index (`super`).

use std::rc::Rc;

use crate::ui::types::{CommandDialect, UiChoice, UiControlId};

use super::{
    bind_cvar_setting, CvarSettingKind, CvarSettingSpec, SettingBinding, SettingBindingKind, SettingCategory,
    SettingCvars,
};

/// Parse a source byte string with the observed i386 glibc `atoi` profile
/// (donor `nativeAtoi`): leading ASCII whitespace is skipped, one optional
/// sign is accepted, digits accumulate with saturation at the `i32` bounds,
/// and parsing stops at the first non-digit.
///
/// The donor rejects text with code units above 255; this infallible port
/// treats such characters as parse terminators instead.
#[must_use]
pub fn native_atoi(text: &str) -> i32 {
    fn is_space(unit: u32) -> bool {
        unit == 32 || (9..=13).contains(&unit)
    }
    let units: Vec<u32> = text.chars().map(|c| c as u32).collect();
    let mut offset = 0;
    while offset < units.len() && is_space(units[offset]) {
        offset += 1;
    }
    let mut negative = false;
    if offset < units.len() && (units[offset] == 43 || units[offset] == 45) {
        negative = units[offset] == 45;
        offset += 1;
    }
    let limit: i64 = if negative { 2147483648 } else { 2147483647 };
    let mut magnitude: i64 = 0;
    let mut overflow = false;
    while offset < units.len() {
        let unit = units[offset];
        if unit == 0 || !(48..=57).contains(&unit) {
            break;
        }
        let digit = (unit - 48) as i64;
        if !overflow {
            if magnitude > (limit - digit) / 10 {
                magnitude = limit;
                overflow = true;
            } else {
                magnitude = magnitude * 10 + digit;
            }
        }
        offset += 1;
    }
    if magnitude == 0 {
        return 0;
    }
    if negative {
        (-magnitude) as i32
    } else {
        magnitude as i32
    }
}

/// Who owns weapon-pickup switching (donor `weaponPickupPolicy`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum WeaponPickupPolicy {
    /// The shared autoswitch cvar drives pickup switching.
    Shared,
    /// The game source owns pickup switching; menus show a notice instead.
    #[default]
    SourceOwned,
}

/// Cvar sources for gameplay settings (donor `GameplaySettingsSource`).
pub struct GameplaySettingsSource {
    /// Server/game registry (owns `sv_autosave`).
    pub cvars: Rc<dyn SettingCvars>,
    /// Client registry; defaults to `cvars` when absent.
    pub client: Option<Rc<dyn SettingCvars>>,
    /// Weapon-pickup switching ownership.
    pub weapon_pickup_policy: WeaponPickupPolicy,
}

impl GameplaySettingsSource {
    /// Client registry, falling back to the game registry.
    #[must_use]
    pub fn effective_client(&self) -> &Rc<dyn SettingCvars> {
        self.client.as_ref().unwrap_or(&self.cvars)
    }
}

fn quake_colors() -> Vec<UiChoice> {
    (0..14)
        .map(|index: i32| UiChoice {
            id: index.to_string(),
            label: index.to_string(),
        })
        .collect()
}

fn effective_cvar_text(registry: &Rc<dyn SettingCvars>, name: &str) -> String {
    registry
        .find(name)
        .map(|view| view.latched_value.unwrap_or(view.value))
        .unwrap_or_default()
}

/// Bind the gameplay settings controls (donor `bindGameplaySettings`).
#[must_use]
pub fn bind_gameplay_settings(source: &GameplaySettingsSource) -> Vec<SettingBinding> {
    let mut settings = Vec::new();
    let client = source.effective_client();
    for (name, label) in [
        ("sv_autosave", "Autosave on level load"),
        ("cg_drawGun", "Draw weapon"),
        ("cg_simpleItems", "Simple items"),
        ("cg_marks", "Wall marks"),
        ("cg_drawCrosshairNames", "Target names"),
    ] {
        let registry = if name == "sv_autosave" { &source.cvars } else { client };
        if registry.find(name).is_some() {
            if let Ok(binding) = bind_cvar_setting(
                registry,
                CvarSettingSpec {
                    name: name.to_string(),
                    label: label.to_string(),
                    category: SettingCategory::Accessibility,
                    restart: None,
                    kind: CvarSettingKind::Toggle,
                },
                None,
            ) {
                settings.push(binding);
            }
        }
    }

    let auto_switch = match client.dialect() {
        CommandDialect::Q3 => Some("cg_autoswitch"),
        CommandDialect::Q2Rerelease => Some("autoswitch"),
        CommandDialect::Q1Netquake | CommandDialect::Q1Quakeworld => Some("qts_weapon_autoswitch"),
        CommandDialect::Q2Classic => None,
    };
    if let Some(name) = auto_switch {
        if client.find(name).is_some() {
            if name == "qts_weapon_autoswitch" && source.weapon_pickup_policy != WeaponPickupPolicy::Shared {
                if let Ok(id) = UiControlId::new("ui:gameplay:source-weapon-switching") {
                    settings.push(SettingBinding {
                        id,
                        label: "This game controls weapon pickup switching".to_string(),
                        category: SettingCategory::Input,
                        enabled: Rc::new(|| false),
                        kind: SettingBindingKind::Button {
                            activate: Rc::new(|| {}),
                        },
                    });
                }
            } else if client.dialect() == CommandDialect::Q3 {
                if let Ok(binding) = bind_cvar_setting(
                    client,
                    CvarSettingSpec {
                        name: name.to_string(),
                        label: "Switch to picked-up weapons".to_string(),
                        category: SettingCategory::Input,
                        restart: None,
                        kind: CvarSettingKind::Toggle,
                    },
                    None,
                ) {
                    settings.push(binding);
                }
            } else {
                let choices = if client.dialect() == CommandDialect::Q2Rerelease {
                    vec![
                        UiChoice {
                            id: "0".to_string(),
                            label: "Smart".to_string(),
                        },
                        UiChoice {
                            id: "1".to_string(),
                            label: "Always".to_string(),
                        },
                        UiChoice {
                            id: "2".to_string(),
                            label: "Except consumable weapons".to_string(),
                        },
                        UiChoice {
                            id: "3".to_string(),
                            label: "Never".to_string(),
                        },
                    ]
                } else {
                    vec![
                        UiChoice {
                            id: "always".to_string(),
                            label: "Always".to_string(),
                        },
                        UiChoice {
                            id: "new".to_string(),
                            label: "New weapons".to_string(),
                        },
                        UiChoice {
                            id: "never".to_string(),
                            label: "Never".to_string(),
                        },
                    ]
                };
                if let Ok(binding) = bind_cvar_setting(
                    client,
                    CvarSettingSpec {
                        name: name.to_string(),
                        label: "Switch to picked-up weapons".to_string(),
                        category: SettingCategory::Input,
                        restart: None,
                        kind: CvarSettingKind::Choice { choices },
                    },
                    None,
                ) {
                    settings.push(binding);
                }
            }
        }
    }

    let player_name = if client.dialect() == CommandDialect::Q1Netquake && client.find("name").is_none() {
        "_cl_name"
    } else {
        "name"
    };
    let identity: Vec<(&str, &str)> = match client.dialect() {
        CommandDialect::Q3 => vec![
            (player_name, "Player name"),
            ("model", "Player model / skin"),
            ("headmodel", "Head model / skin"),
        ],
        CommandDialect::Q2Classic | CommandDialect::Q2Rerelease => {
            vec![(player_name, "Player name"), ("skin", "Player skin (model/skin)")]
        }
        CommandDialect::Q1Netquake | CommandDialect::Q1Quakeworld => {
            vec![(player_name, "Player name")]
        }
    };
    for (name, label) in identity {
        if client.find(name).is_some() {
            if let Ok(binding) = bind_cvar_setting(
                client,
                CvarSettingSpec {
                    name: name.to_string(),
                    label: label.to_string(),
                    category: SettingCategory::Network,
                    restart: None,
                    kind: CvarSettingKind::TextEntry {
                        maximum_length: if client.dialect() == CommandDialect::Q1Netquake {
                            15
                        } else {
                            63
                        },
                        submit_only: true,
                    },
                },
                None,
            ) {
                settings.push(binding);
            }
        }
    }

    let colors = quake_colors();
    match client.dialect() {
        CommandDialect::Q1Netquake => {
            let name = if client.find("color").is_none() {
                "_cl_color"
            } else {
                "color"
            };
            if client.find(name).is_some() {
                for (id, label, shift) in [("shirt", "Shirt color", 4), ("pants", "Pants color", 0)] {
                    if let Ok(control) = UiControlId::new(&format!("ui:gameplay:{id}-color")) {
                        let listed = colors.clone();
                        let valid = colors.clone();
                        let read_registry = Rc::clone(client);
                        let read_name = name.to_string();
                        let write_registry = Rc::clone(client);
                        let write_name = name.to_string();
                        settings.push(SettingBinding {
                            id: control,
                            label: label.to_string(),
                            category: SettingCategory::Network,
                            enabled: Rc::new(|| true),
                            kind: SettingBindingKind::Choice {
                                read: Rc::new(move || {
                                    let text = effective_cvar_text(&read_registry, &read_name);
                                    13.min((native_atoi(&text) >> shift) & 15).to_string()
                                }),
                                write: Rc::new(move |value| {
                                    let Some(selected) = valid.iter().position(|color| color.id == value) else {
                                        panic!("Unknown Quake color");
                                    };
                                    let previous = native_atoi(&effective_cvar_text(&write_registry, &write_name));
                                    let packed = (previous & !(15 << shift)) | ((selected as i32) << shift);
                                    write_registry.set(&write_name, &packed.to_string());
                                }),
                                choices: Rc::new(move || listed.clone()),
                            },
                        });
                    }
                }
            }
        }
        CommandDialect::Q1Quakeworld => {
            for (name, label) in [("topcolor", "Shirt color"), ("bottomcolor", "Pants color")] {
                if client.find(name).is_some() {
                    if let Ok(binding) = bind_cvar_setting(
                        client,
                        CvarSettingSpec {
                            name: name.to_string(),
                            label: label.to_string(),
                            category: SettingCategory::Network,
                            restart: None,
                            kind: CvarSettingKind::Choice {
                                choices: colors.clone(),
                            },
                        },
                        None,
                    ) {
                        settings.push(binding);
                    }
                }
            }
        }
        CommandDialect::Q2Classic | CommandDialect::Q2Rerelease => {
            if client.find("hand").is_some() {
                if let Ok(binding) = bind_cvar_setting(
                    client,
                    CvarSettingSpec {
                        name: "hand".to_string(),
                        label: "Weapon hand".to_string(),
                        category: SettingCategory::Network,
                        restart: None,
                        kind: CvarSettingKind::Choice {
                            choices: vec![
                                UiChoice {
                                    id: "0".to_string(),
                                    label: "Right".to_string(),
                                },
                                UiChoice {
                                    id: "1".to_string(),
                                    label: "Left".to_string(),
                                },
                                UiChoice {
                                    id: "2".to_string(),
                                    label: "Center".to_string(),
                                },
                            ],
                        },
                    },
                    None,
                ) {
                    settings.push(binding);
                }
            }
            if client.find("fov").is_some() {
                if let Ok(binding) = bind_cvar_setting(
                    client,
                    CvarSettingSpec {
                        name: "fov".to_string(),
                        label: "Field of view".to_string(),
                        category: SettingCategory::Video,
                        restart: None,
                        kind: CvarSettingKind::Slider {
                            minimum: 1.0,
                            maximum: 160.0,
                            step: 1.0,
                        },
                    },
                    None,
                ) {
                    settings.push(binding);
                }
            }
        }
        CommandDialect::Q3 => {}
    }
    settings
}

/// Restore every bound gameplay cvar to its reset value
/// (donor `resetGameplaySettings`).
pub fn reset_gameplay_settings(source: &GameplaySettingsSource) {
    let client = source.effective_client();
    for binding in bind_gameplay_settings(source) {
        let id = binding.id.as_str();
        let name = if id == "ui:gameplay:shirt-color" || id == "ui:gameplay:pants-color" {
            if client.find("color").is_none() {
                "_cl_color"
            } else {
                "color"
            }
        } else {
            id.strip_prefix("ui:settings:").unwrap_or(id)
        };
        let registry = if name == "sv_autosave" { &source.cvars } else { client };
        if let Some(view) = registry.find(name) {
            registry.set(name, &view.reset_value);
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    //! In-memory [`SettingCvars`](super::SettingCvars) fake for settings tests.

    use std::cell::RefCell;
    use std::collections::HashMap;

    use crate::ui::types::CommandDialect;

    use crate::ui::settings::{CvarView, SettingCvars};

    struct Entry {
        value: String,
        latched_value: Option<String>,
        reset_value: String,
        flags: u32,
    }

    /// In-memory cvar registry fake.
    pub struct MemoryCvars {
        dialect: CommandDialect,
        vars: RefCell<HashMap<String, Entry>>,
    }

    impl MemoryCvars {
        /// Create an empty registry for `dialect`.
        #[must_use]
        pub fn new(dialect: CommandDialect) -> Self {
            Self {
                dialect,
                vars: RefCell::new(HashMap::new()),
            }
        }

        /// Register `name` with `value` as both current and reset value.
        #[must_use]
        pub fn with(self, name: &str, value: &str) -> Self {
            self.with_full(name, value, None, value, 0)
        }

        /// Register `name` with distinct current and reset values.
        #[must_use]
        pub fn with_reset(self, name: &str, value: &str, reset: &str) -> Self {
            self.with_full(name, value, None, reset, 0)
        }

        /// Register `name` with full control over latched value and flags.
        #[must_use]
        pub fn with_full(self, name: &str, value: &str, latched: Option<&str>, reset: &str, flags: u32) -> Self {
            self.vars.borrow_mut().insert(
                name.to_string(),
                Entry {
                    value: value.to_string(),
                    latched_value: latched.map(str::to_string),
                    reset_value: reset.to_string(),
                    flags,
                },
            );
            self
        }
    }

    impl SettingCvars for MemoryCvars {
        fn dialect(&self) -> CommandDialect {
            self.dialect
        }

        fn find(&self, name: &str) -> Option<CvarView> {
            self.vars.borrow().get(name).map(|entry| CvarView {
                value: entry.value.clone(),
                latched_value: entry.latched_value.clone(),
                reset_value: entry.reset_value.clone(),
                flags: entry.flags,
            })
        }

        fn set(&self, name: &str, value: &str) {
            let mut vars = self.vars.borrow_mut();
            if let Some(entry) = vars.get_mut(name) {
                entry.value = value.to_string();
            } else {
                vars.insert(
                    name.to_string(),
                    Entry {
                        value: value.to_string(),
                        latched_value: None,
                        reset_value: value.to_string(),
                        flags: 0,
                    },
                );
            }
        }

        fn variable_value(&self, name: &str) -> f32 {
            self.find(name)
                .and_then(|view| view.latched_value.unwrap_or(view.value).parse::<f32>().ok())
                .unwrap_or(0.0)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use crate::ui::settings::{SettingBindingKind, SettingCategory};
    use crate::ui::types::CommandDialect;

    use super::test_support::MemoryCvars;
    use super::{
        bind_gameplay_settings, native_atoi, reset_gameplay_settings, GameplaySettingsSource, SettingCvars,
        WeaponPickupPolicy,
    };

    fn source(
        cvars: MemoryCvars,
        client: Option<MemoryCvars>,
        weapon_pickup_policy: WeaponPickupPolicy,
    ) -> GameplaySettingsSource {
        GameplaySettingsSource {
            cvars: Rc::new(cvars),
            client: client.map(|c| Rc::new(c) as Rc<dyn SettingCvars>),
            weapon_pickup_policy,
        }
    }

    fn ids(bindings: &[super::SettingBinding]) -> Vec<&str> {
        bindings.iter().map(|b| b.id.as_str()).collect()
    }

    fn kind_name(kind: &SettingBindingKind) -> &'static str {
        match kind {
            SettingBindingKind::Toggle { .. } => "toggle",
            SettingBindingKind::Slider { .. } => "slider",
            SettingBindingKind::Choice { .. } => "choice",
            SettingBindingKind::TextEntry { .. } => "text-entry",
            SettingBindingKind::Button { .. } => "button",
        }
    }

    #[test]
    fn atoi_edges() {
        assert_eq!(native_atoi("42"), 42);
        assert_eq!(native_atoi("  -42"), -42);
        assert_eq!(native_atoi("\t\n\r +7 trailing"), 7);
        assert_eq!(native_atoi(""), 0);
        assert_eq!(native_atoi("   "), 0);
        assert_eq!(native_atoi("abc"), 0);
        assert_eq!(native_atoi("12abc"), 12);
        assert_eq!(native_atoi("-"), 0);
        assert_eq!(native_atoi("+"), 0);
        assert_eq!(native_atoi("0x10"), 0);
        assert_eq!(native_atoi("007"), 7);
        assert_eq!(native_atoi("2147483647"), i32::MAX);
        assert_eq!(native_atoi("2147483648"), i32::MAX);
        assert_eq!(native_atoi("99999999999999999999"), i32::MAX);
        assert_eq!(native_atoi("-2147483648"), i32::MIN);
        assert_eq!(native_atoi("-2147483649"), i32::MIN);
        assert_eq!(native_atoi("-99999999999999999999"), i32::MIN);
        assert_eq!(native_atoi("12\u{0}3"), 12);
        assert_eq!(native_atoi("3\u{e9}5"), 3);
        assert_eq!(native_atoi("\u{e9}5"), 0);
        assert_eq!(native_atoi("-0"), 0);
    }

    #[test]
    fn q3_binds_toggles_autoswitch_and_identity() {
        let client = MemoryCvars::new(CommandDialect::Q3)
            .with("cg_drawGun", "1")
            .with("cg_simpleItems", "0")
            .with("cg_marks", "1")
            .with("cg_drawCrosshairNames", "0")
            .with("cg_autoswitch", "1")
            .with("name", "player")
            .with("model", "sarge")
            .with("headmodel", "*james");
        let cvars = MemoryCvars::new(CommandDialect::Q3).with("sv_autosave", "1");
        let source = source(cvars, Some(client), WeaponPickupPolicy::Shared);
        let bindings = bind_gameplay_settings(&source);
        assert_eq!(
            ids(&bindings),
            vec![
                "ui:settings:sv_autosave",
                "ui:settings:cg_drawGun",
                "ui:settings:cg_simpleItems",
                "ui:settings:cg_marks",
                "ui:settings:cg_drawCrosshairNames",
                "ui:settings:cg_autoswitch",
                "ui:settings:name",
                "ui:settings:model",
                "ui:settings:headmodel",
            ]
        );
        assert!(matches!(bindings[5].kind, SettingBindingKind::Toggle { .. }));
        assert_eq!(bindings[5].category, SettingCategory::Input);
        for binding in &bindings[6..] {
            match &binding.kind {
                SettingBindingKind::TextEntry {
                    maximum_length, commit, ..
                } => {
                    assert_eq!(*maximum_length, 63);
                    assert!(commit.is_some());
                }
                other => panic!("expected text entry, got {}", kind_name(other)),
            }
        }
    }

    #[test]
    fn missing_cvars_are_skipped() {
        let source = source(MemoryCvars::new(CommandDialect::Q3), None, WeaponPickupPolicy::Shared);
        assert!(bind_gameplay_settings(&source).is_empty());
    }

    #[test]
    fn q2_rerelease_binds_autoswitch_hand_and_fov() {
        let client = MemoryCvars::new(CommandDialect::Q2Rerelease)
            .with("autoswitch", "1")
            .with("name", "player")
            .with("skin", "male/grunt")
            .with("hand", "0")
            .with("fov", "90");
        let source = source(
            MemoryCvars::new(CommandDialect::Q2Rerelease),
            Some(client),
            WeaponPickupPolicy::Shared,
        );
        let bindings = bind_gameplay_settings(&source);
        assert_eq!(
            ids(&bindings),
            vec![
                "ui:settings:autoswitch",
                "ui:settings:name",
                "ui:settings:skin",
                "ui:settings:hand",
                "ui:settings:fov",
            ]
        );
        match &bindings[0].kind {
            SettingBindingKind::Choice { choices, .. } => {
                let listed = choices();
                assert_eq!(listed.len(), 4);
                assert_eq!(listed[2].label, "Except consumable weapons");
            }
            other => panic!("expected autoswitch choice, got {}", kind_name(other)),
        }
        match &bindings[4].kind {
            SettingBindingKind::Slider {
                minimum,
                maximum,
                step,
                read,
                ..
            } => {
                assert_eq!((*minimum, *maximum, *step), (1.0, 160.0, 1.0));
                assert_eq!(read(), 90.0);
            }
            other => panic!("expected fov slider, got {}", kind_name(other)),
        }
    }

    #[test]
    fn q2_classic_has_no_autoswitch() {
        let client = MemoryCvars::new(CommandDialect::Q2Classic)
            .with("autoswitch", "1")
            .with("name", "player");
        let source = source(
            MemoryCvars::new(CommandDialect::Q2Classic),
            Some(client),
            WeaponPickupPolicy::Shared,
        );
        assert_eq!(ids(&bind_gameplay_settings(&source)), vec!["ui:settings:name"]);
    }

    #[test]
    fn q1_netquake_source_owned_shows_notice_button() {
        let client = MemoryCvars::new(CommandDialect::Q1Netquake)
            .with("qts_weapon_autoswitch", "new")
            .with("_cl_name", "player")
            .with("_cl_color", "35");
        let source = source(
            MemoryCvars::new(CommandDialect::Q1Netquake),
            Some(client),
            WeaponPickupPolicy::SourceOwned,
        );
        let bindings = bind_gameplay_settings(&source);
        assert_eq!(
            ids(&bindings),
            vec![
                "ui:gameplay:source-weapon-switching",
                "ui:settings:_cl_name",
                "ui:gameplay:shirt-color",
                "ui:gameplay:pants-color",
            ]
        );
        assert!(!(bindings[0].enabled)());
        assert!(matches!(bindings[0].kind, SettingBindingKind::Button { .. }));
        match &bindings[1].kind {
            SettingBindingKind::TextEntry { maximum_length, .. } => assert_eq!(*maximum_length, 15),
            other => panic!("expected name entry, got {}", kind_name(other)),
        }
    }

    #[test]
    fn q1_netquake_shared_binds_autoswitch_choice() {
        let client = MemoryCvars::new(CommandDialect::Q1Netquake)
            .with("qts_weapon_autoswitch", "new")
            .with("name", "player");
        let source = source(
            MemoryCvars::new(CommandDialect::Q1Netquake),
            Some(client),
            WeaponPickupPolicy::Shared,
        );
        let bindings = bind_gameplay_settings(&source);
        assert_eq!(
            ids(&bindings),
            vec!["ui:settings:qts_weapon_autoswitch", "ui:settings:name"]
        );
        match &bindings[0].kind {
            SettingBindingKind::Choice { choices, read, .. } => {
                let listed: Vec<String> = choices().iter().map(|c| c.id.clone()).collect();
                assert_eq!(listed, vec!["always", "new", "never"]);
                assert_eq!(read(), "new");
            }
            other => panic!("expected autoswitch choice, got {}", kind_name(other)),
        }
    }

    #[test]
    fn q1_quakeworld_binds_top_and_bottom_color() {
        let client = MemoryCvars::new(CommandDialect::Q1Quakeworld)
            .with("name", "player")
            .with("topcolor", "2")
            .with("bottomcolor", "3");
        let source = source(
            MemoryCvars::new(CommandDialect::Q1Quakeworld),
            Some(client),
            WeaponPickupPolicy::Shared,
        );
        assert_eq!(
            ids(&bind_gameplay_settings(&source)),
            vec!["ui:settings:name", "ui:settings:topcolor", "ui:settings:bottomcolor",]
        );
    }

    fn color_source(packed: &str) -> GameplaySettingsSource {
        let cvars: Rc<dyn SettingCvars> =
            Rc::new(MemoryCvars::new(CommandDialect::Q1Netquake).with("_cl_color", packed));
        GameplaySettingsSource {
            cvars: Rc::clone(&cvars),
            client: Some(Rc::clone(&cvars)),
            weapon_pickup_policy: WeaponPickupPolicy::Shared,
        }
    }

    #[test]
    fn color_bitpack_round_trip() {
        let source = color_source("35");
        let bindings = bind_gameplay_settings(&source);
        assert_eq!(bindings.len(), 2);
        match &bindings[0].kind {
            SettingBindingKind::Choice { read, write, .. } => {
                assert_eq!(read(), "2");
                write("5");
            }
            other => panic!("expected shirt choice, got {}", kind_name(other)),
        }
        assert_eq!(
            source.effective_client().find("_cl_color").map(|v| v.value),
            Some("83".to_string())
        );
        match &bindings[1].kind {
            SettingBindingKind::Choice { read, write, .. } => {
                assert_eq!(read(), "3");
                write("13");
            }
            other => panic!("expected pants choice, got {}", kind_name(other)),
        }
        assert_eq!(
            source.effective_client().find("_cl_color").map(|v| v.value),
            Some("93".to_string())
        );
    }

    #[test]
    #[should_panic(expected = "Unknown Quake color")]
    fn color_write_rejects_unknown_id() {
        let source = color_source("35");
        let bindings = bind_gameplay_settings(&source);
        match &bindings[1].kind {
            SettingBindingKind::Choice { write, .. } => write("bogus"),
            other => panic!("expected pants choice, got {}", kind_name(other)),
        }
    }

    #[test]
    fn color_read_clamps_to_valid_range() {
        let cvars: Rc<dyn SettingCvars> = Rc::new(MemoryCvars::new(CommandDialect::Q1Netquake).with("color", "255"));
        let source = GameplaySettingsSource {
            cvars: Rc::clone(&cvars),
            client: Some(Rc::clone(&cvars)),
            weapon_pickup_policy: WeaponPickupPolicy::Shared,
        };
        let bindings = bind_gameplay_settings(&source);
        match &bindings[0].kind {
            SettingBindingKind::Choice { read, .. } => assert_eq!(read(), "13"),
            other => panic!("expected shirt choice, got {}", kind_name(other)),
        }
    }

    #[test]
    fn reset_restores_reset_values() {
        let client = MemoryCvars::new(CommandDialect::Q3)
            .with_reset("cg_drawGun", "0", "1")
            .with_reset("name", "changed", "player")
            .with_reset("cg_autoswitch", "0", "1");
        let cvars = MemoryCvars::new(CommandDialect::Q3).with_reset("sv_autosave", "0", "1");
        let game: Rc<dyn SettingCvars> = Rc::new(cvars);
        let play: Rc<dyn SettingCvars> = Rc::new(client);
        let source = GameplaySettingsSource {
            cvars: Rc::clone(&game),
            client: Some(Rc::clone(&play)),
            weapon_pickup_policy: WeaponPickupPolicy::Shared,
        };
        reset_gameplay_settings(&source);
        assert_eq!(game.find("sv_autosave").map(|v| v.value), Some("1".to_string()));
        assert_eq!(play.find("cg_drawGun").map(|v| v.value), Some("1".to_string()));
        assert_eq!(play.find("cg_autoswitch").map(|v| v.value), Some("1".to_string()));
        assert_eq!(play.find("name").map(|v| v.value), Some("player".to_string()));
    }

    #[test]
    fn reset_restores_packed_color_once() {
        let cvars: Rc<dyn SettingCvars> =
            Rc::new(MemoryCvars::new(CommandDialect::Q1Netquake).with_reset("_cl_color", "93", "35"));
        let source = GameplaySettingsSource {
            cvars: Rc::clone(&cvars),
            client: Some(Rc::clone(&cvars)),
            weapon_pickup_policy: WeaponPickupPolicy::Shared,
        };
        reset_gameplay_settings(&source);
        assert_eq!(
            source.effective_client().find("_cl_color").map(|v| v.value),
            Some("35".to_string())
        );
    }
}
