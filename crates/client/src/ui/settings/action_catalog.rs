//! Bindable action catalog shared by every settings menu.
//!
//! Ported from the TypeScript donor's `src/ui/settings/action-catalog.ts`
//! (`sharedBindingActions`) plus the weapon-selection half of
//! `src/input/weapon-bindings.ts` (`resolveWeaponSelection`,
//! `weaponBindingItem`) and the wheel half of `src/input/bindings.ts`
//! (`canonicalWheelCommand`). Movement syntax follows the command dialect;
//! item selection follows the supplied arsenal.

use std::rc::Rc;

use crate::input::weapons::{
    q1_weapon_display_name, Q1_MISSION_WEAPONS, Q1_WEAPONS, Q2_BASE_WEAPONS, Q2_ROGUE_WEAPONS, Q2_XATRIX_WEAPONS,
    Q3_WEAPONS,
};
use crate::input::InputBindingTarget;
use crate::ui::settings::bindings::BindingAction;
use crate::ui::types::CommandDialect;

/// Scoreboard command a family understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScoreCommand {
    /// Bare `score` command.
    Score,
    /// Held `+scores` command.
    Scores,
}

impl ScoreCommand {
    /// Donor command text.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            ScoreCommand::Score => "score",
            ScoreCommand::Scores => "+scores",
        }
    }
}

/// Capability gates selecting which optional rows appear.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingCapabilities {
    /// Whether chat rows appear.
    pub chat: bool,
    /// Scoreboard command, when the family has one.
    pub score_command: Option<ScoreCommand>,
    /// Whether the offhand grapple row appears.
    pub offhand_grapple: bool,
    /// Whether the offhand grenade row appears.
    pub offhand_grenades: bool,
}

/// What kind of item a bindable entry selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BindableItemKind {
    /// Weapon selected with `use`.
    Weapon,
    /// Powerup used with `use`.
    Powerup,
}

/// One arsenal entry that can appear as a binding row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindableItem {
    /// Stable item identity.
    pub id: String,
    /// Display label.
    pub label: String,
    /// Item kind.
    pub kind: BindableItemKind,
}

/// Minimal item view used for weapon-command resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeaponBindingItemView {
    /// Stable item identity.
    pub id: String,
    /// Item kind.
    pub kind: BindableItemKind,
}

/// Canonicalize a weapon-wheel command, preserving unknown text.
#[must_use]
pub fn canonical_wheel_command(text: &str) -> String {
    let command = text.trim().to_lowercase();
    let mut chars = command.chars();
    let prefix = chars.next().unwrap_or('\0');
    if prefix != '+' && prefix != '-' {
        return text.to_string();
    }
    let name: String = chars.collect();
    let mode = match name.as_str() {
        "weaponwheel" | "wheel" => Some("weapons"),
        "powerupwheel" | "wheel2" => Some("powerups"),
        _ => None,
    };
    match mode {
        None => text.to_string(),
        Some("weapons") => format!("{prefix}weaponwheel"),
        Some(_) => format!("{prefix}powerupwheel"),
    }
}

/// Q1 mission weapons this catalog resolves beyond the canonical eight.
///
/// Base and mission tables plus display names live in
/// [`crate::input::weapons`]; the catalog keeps only its extras (grapples
/// and the MG3 set) so alias order stays base, mission, extras.
const Q1_MISSION_EXTRA: [&str; 4] = ["rogue:grapple", "mg3:laser", "mg3:mjolnir", "ctf:grapple"];

/// Q2 weapons this catalog resolves beyond the canonical tables.
const Q2_EXTRA_WEAPONS: [(&str, &str, &str); 1] = [("heatbeam", "q2:weapon_plasmabeam", "Plasma Beam")];

/// All Q1 weapon ids in alias order: base, mission, extras.
fn q1_weapon_ids() -> impl Iterator<Item = &'static str> {
    Q1_WEAPONS
        .iter()
        .map(|(id, _)| *id)
        .chain(Q1_MISSION_WEAPONS.iter().map(|(id, _)| *id))
        .chain(Q1_MISSION_EXTRA.iter().copied())
}

/// All Q2 weapons in alias order: base, Xatrix, Rogue, extras.
fn q2_weapons() -> impl Iterator<Item = &'static (&'static str, &'static str, &'static str)> {
    Q2_BASE_WEAPONS
        .iter()
        .chain(Q2_XATRIX_WEAPONS.iter())
        .chain(Q2_ROGUE_WEAPONS.iter())
        .chain(Q2_EXTRA_WEAPONS.iter())
}

/// Lowercase a value and strip ASCII spaces, matching the donor.
fn normalized(value: &str) -> String {
    value.to_lowercase().replace(' ', "")
}

/// Whether an item id is a Q1 base weapon item.
fn is_q1_base_weapon_item(id: &str) -> bool {
    Q1_WEAPONS.iter().any(|(weapon, _)| id == format!("q1:weapon/{weapon}"))
}

/// Whether an argument is forbidden inside a selection command.
fn is_forbidden_argument(argument: &str) -> bool {
    argument
        .chars()
        .any(|ch| ch == ';' || ch == '\r' || ch == '\n' || ch == '\\' || ch == '"')
        || argument.contains("//")
        || argument.contains("/*")
}

/// Resolve one `use` selection against the catalog.
fn resolve_use(argument: &str, items: &[WeaponBindingItemView]) -> Option<String> {
    if is_forbidden_argument(argument) {
        return None;
    }
    let requested = normalized(argument);
    if let Some(canonical) = items.iter().find(|item| item.id == requested) {
        return Some(canonical.id.clone());
    }
    let mut matching: Vec<&WeaponBindingItemView> = Vec::new();
    for item in items {
        let mut hit = false;
        if let Some(q1) = q1_weapon_ids().find(|weapon| format!("q1:weapon/{weapon}") == item.id) {
            if normalized(q1) == requested || normalized(&q1_weapon_display_name(q1)) == requested {
                hit = true;
            }
        }
        if !hit {
            if let Some(short) = item.id.strip_prefix("q3:weapon/") {
                if Q3_WEAPONS.iter().any(|(_, id, _)| *id == item.id) && normalized(short) == requested {
                    hit = true;
                }
            }
        }
        if !hit {
            if let Some((name, _, display)) = q2_weapons().find(|(_, id, _)| *id == item.id) {
                if normalized(name) == requested || normalized(display) == requested {
                    hit = true;
                }
            }
        }
        if hit {
            matching.push(item);
        }
    }
    if matching.len() == 1 {
        matching.first().map(|item| item.id.clone())
    } else {
        None
    }
}

/// Resolve one `weapon` or `impulse` number against the catalog.
fn resolve_numbered(name: &str, argument: &str, items: &[WeaponBindingItemView]) -> Option<String> {
    if argument.is_empty() || !argument.chars().all(|ch| ch.is_ascii_digit()) || argument.starts_with('0') {
        return None;
    }
    let number: u32 = argument.parse().unwrap_or_default();
    if number == 0 {
        return None;
    }
    if name == "impulse"
        && items
            .iter()
            .any(|item| item.kind == BindableItemKind::Weapon && !is_q1_base_weapon_item(&item.id))
    {
        return None;
    }
    let id = if name == "weapon" {
        Q3_WEAPONS
            .iter()
            .find(|(tag, _, _)| *tag as u32 == number)
            .map(|(_, id, _)| *id)?
    } else {
        let (weapon, _) = Q1_WEAPONS.get((number as usize).wrapping_sub(1))?;
        return items
            .iter()
            .find(|item| item.kind == BindableItemKind::Weapon && item.id == format!("q1:weapon/{weapon}"))
            .map(|item| item.id.clone());
    };
    items
        .iter()
        .find(|item| item.kind == BindableItemKind::Weapon && item.id == id)
        .map(|item| item.id.clone())
}

/// Resolve one simple selection command; scripts and aliases stay opaque.
#[must_use]
pub fn weapon_binding_item(text: &str, items: &[WeaponBindingItemView]) -> Option<String> {
    if text
        .chars()
        .any(|ch| ch == ';' || ch == '\r' || ch == '\n' || ch == '\\')
        || text.contains("//")
        || text.contains("/*")
    {
        return None;
    }
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let split = trimmed.find(|ch: char| ch.is_whitespace())?;
    let (command, rest) = trimmed.split_at(split);
    let rest = rest.trim_start();
    if rest.is_empty() {
        return None;
    }
    let name = command.to_lowercase();
    if name != "use" && name != "weapon" && name != "impulse" {
        return None;
    }
    let argument = if rest.starts_with('"') {
        if rest.len() < 2 || !rest.ends_with('"') {
            return None;
        }
        let inner = &rest[1..rest.len() - 1];
        if inner.is_empty() || inner.chars().any(|ch| ch == '"' || ch == '\r' || ch == '\n') {
            return None;
        }
        inner
    } else {
        if rest.chars().any(|ch| ch == '"' || ch == '\r' || ch == '\n') {
            return None;
        }
        rest
    };
    if argument.is_empty() || is_forbidden_argument(argument) {
        return None;
    }
    if name == "use" {
        resolve_use(argument, items)
    } else {
        resolve_numbered(&name, argument, items)
    }
}

/// Shared movement, wheel, console, score, chat, offhand, and item rows.
#[must_use]
pub fn shared_binding_actions(
    dialect: CommandDialect,
    items: &[BindableItem],
    capabilities: &BindingCapabilities,
) -> Vec<BindingAction> {
    let jump = if dialect.is_q1() { "+jump" } else { "+moveup" };
    let rows: [(&str, &str, &str); 18] = [
        ("forward", "Move forward", "+forward"),
        ("back", "Move back", "+back"),
        ("left", "Strafe left", "+moveleft"),
        ("right", "Strafe right", "+moveright"),
        ("jump", "Jump / swim up", jump),
        ("down", "Crouch / swim down", "+movedown"),
        ("walk", "Walk / run modifier", "+speed"),
        ("turn-left", "Turn left", "+left"),
        ("turn-right", "Turn right", "+right"),
        ("look-up", "Look up", "+lookup"),
        ("look-down", "Look down", "+lookdown"),
        ("attack", "Fire primary weapon", "+attack"),
        ("use", "Use / activate", "+use"),
        ("next-weapon", "Next weapon", "weapnext"),
        ("previous-weapon", "Previous weapon", "weapprev"),
        ("weapon-wheel", "Weapon wheel", "+weaponwheel"),
        ("powerup-wheel", "Powerup wheel", "+powerupwheel"),
        ("console", "Toggle console", "toggleconsole"),
    ];
    let mut actions: Vec<BindingAction> = Vec::with_capacity(rows.len() + 8 + items.len());
    for (id, label, text) in rows {
        let matcher = if id == "weapon-wheel" || id == "powerup-wheel" {
            let want = text.to_string();
            Some(Rc::new(move |target: &InputBindingTarget| match target {
                InputBindingTarget::Command(current) => canonical_wheel_command(current) == want,
                InputBindingTarget::Action(_) => false,
            }) as Rc<dyn Fn(&InputBindingTarget) -> bool>)
        } else {
            None
        };
        actions.push(BindingAction {
            id: id.to_string(),
            label: label.to_string(),
            target: InputBindingTarget::Command(text.to_string()),
            matches: matcher,
        });
    }
    let score_text = capabilities
        .score_command
        .map_or_else(|| "+scores".to_string(), |command| command.as_str().to_string());
    actions.push(BindingAction {
        id: "scores".to_string(),
        label: "Show scores".to_string(),
        target: InputBindingTarget::Command(score_text),
        matches: Some(Rc::new(|target: &InputBindingTarget| match target {
            InputBindingTarget::Command(text) => text == "+scores" || text == "+showscores",
            InputBindingTarget::Action(_) => false,
        })),
    });
    if capabilities.chat {
        actions.push(BindingAction {
            id: "chat".to_string(),
            label: "Chat".to_string(),
            target: InputBindingTarget::Command("messagemode".to_string()),
            matches: None,
        });
        actions.push(BindingAction {
            id: "team-chat".to_string(),
            label: "Team chat".to_string(),
            target: InputBindingTarget::Command("messagemode2".to_string()),
            matches: None,
        });
    }
    if capabilities.offhand_grapple {
        actions.push(BindingAction {
            id: "grapple".to_string(),
            label: "Offhand grapple (hold)".to_string(),
            target: InputBindingTarget::Command("+grapple".to_string()),
            matches: None,
        });
    }
    if capabilities.offhand_grenades {
        actions.push(BindingAction {
            id: "grenade".to_string(),
            label: "Cook / throw offhand grenade".to_string(),
            target: InputBindingTarget::Command("+grenade".to_string()),
            matches: None,
        });
    }
    let views: Vec<WeaponBindingItemView> = items
        .iter()
        .map(|item| WeaponBindingItemView {
            id: item.id.clone(),
            kind: item.kind,
        })
        .collect();
    for item in items {
        let want = item.id.clone();
        let catalog = views.clone();
        let verb = match item.kind {
            BindableItemKind::Weapon => "Select",
            BindableItemKind::Powerup => "Use",
        };
        actions.push(BindingAction {
            id: format!("item:{}", item.id),
            label: format!("{verb} {}", item.label),
            target: InputBindingTarget::Command(format!("use {}", item.id)),
            matches: Some(Rc::new(move |target: &InputBindingTarget| match target {
                InputBindingTarget::Command(text) => {
                    weapon_binding_item(text, &catalog).as_deref() == Some(want.as_str())
                }
                InputBindingTarget::Action(_) => false,
            })),
        });
    }
    actions
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capabilities() -> BindingCapabilities {
        BindingCapabilities {
            chat: true,
            score_command: Some(ScoreCommand::Scores),
            offhand_grapple: true,
            offhand_grenades: true,
        }
    }

    #[test]
    fn wheel_commands_canonicalize() {
        assert_eq!(canonical_wheel_command("+wheel"), "+weaponwheel");
        assert_eq!(canonical_wheel_command("-WHEEL2"), "-powerupwheel");
        assert_eq!(canonical_wheel_command("  +WeaponWheel  "), "+weaponwheel");
        assert_eq!(canonical_wheel_command("+attack"), "+attack");
        assert_eq!(canonical_wheel_command("weapnext"), "weapnext");
        assert_eq!(canonical_wheel_command("+unknown"), "+unknown");
    }

    #[test]
    fn weapon_use_matches_ids_and_aliases() {
        let items = vec![
            WeaponBindingItemView {
                id: "q1:weapon/axe".to_string(),
                kind: BindableItemKind::Weapon,
            },
            WeaponBindingItemView {
                id: "q3:weapon/railgun".to_string(),
                kind: BindableItemKind::Weapon,
            },
            WeaponBindingItemView {
                id: "q2:weapon_railgun".to_string(),
                kind: BindableItemKind::Weapon,
            },
        ];
        assert_eq!(
            weapon_binding_item("use q1:weapon/axe", &items).as_deref(),
            Some("q1:weapon/axe")
        );
        assert_eq!(
            weapon_binding_item("USE \"q1:weapon/axe\"", &items).as_deref(),
            Some("q1:weapon/axe")
        );
        assert_eq!(weapon_binding_item("use axe", &items).as_deref(), Some("q1:weapon/axe"));
        assert_eq!(weapon_binding_item("use railgun", &items), None);
        let q3_only = vec![WeaponBindingItemView {
            id: "q3:weapon/railgun".to_string(),
            kind: BindableItemKind::Weapon,
        }];
        assert_eq!(
            weapon_binding_item("use railgun", &q3_only).as_deref(),
            Some("q3:weapon/railgun")
        );
        let q2_only = vec![WeaponBindingItemView {
            id: "q2:weapon_railgun".to_string(),
            kind: BindableItemKind::Weapon,
        }];
        assert_eq!(
            weapon_binding_item("use railgun", &q2_only).as_deref(),
            Some("q2:weapon_railgun")
        );
        assert_eq!(weapon_binding_item("use \"super shotgun\"", &q2_only), None);
    }

    #[test]
    fn weapon_numbered_commands_resolve() {
        let q1 = vec![WeaponBindingItemView {
            id: "q1:weapon/axe".to_string(),
            kind: BindableItemKind::Weapon,
        }];
        assert_eq!(weapon_binding_item("impulse 1", &q1).as_deref(), Some("q1:weapon/axe"));
        assert_eq!(weapon_binding_item("impulse 0", &q1), None);
        assert_eq!(weapon_binding_item("impulse 01", &q1), None);
        assert_eq!(weapon_binding_item("impulse 9", &q1), None);
        let mixed = vec![
            WeaponBindingItemView {
                id: "q1:weapon/axe".to_string(),
                kind: BindableItemKind::Weapon,
            },
            WeaponBindingItemView {
                id: "q3:weapon/gauntlet".to_string(),
                kind: BindableItemKind::Weapon,
            },
        ];
        assert_eq!(weapon_binding_item("impulse 1", &mixed), None);
        let q3 = vec![WeaponBindingItemView {
            id: "q3:weapon/gauntlet".to_string(),
            kind: BindableItemKind::Weapon,
        }];
        assert_eq!(
            weapon_binding_item("weapon 1", &q3).as_deref(),
            Some("q3:weapon/gauntlet")
        );
        assert_eq!(weapon_binding_item("weapon 99", &q3), None);
    }

    #[test]
    fn canonical_alias_order_and_names_match() {
        let ids: Vec<&str> = q1_weapon_ids().collect();
        assert_eq!(
            ids,
            vec![
                "axe",
                "shotgun",
                "supershotgun",
                "nailgun",
                "supernailgun",
                "grenadelauncher",
                "rocketlauncher",
                "lightning",
                "hipnotic:laser",
                "hipnotic:mjolnir",
                "hipnotic:proximity",
                "rogue:lava-nailgun",
                "rogue:lava-supernailgun",
                "rogue:multi-grenade",
                "rogue:multi-rocket",
                "rogue:plasma",
                "rogue:grapple",
                "mg3:laser",
                "mg3:mjolnir",
                "ctf:grapple",
            ]
        );
        assert_eq!(q1_weapon_display_name("supershotgun"), "Double-barrelled Shotgun");
        assert_eq!(q1_weapon_display_name("lightning"), "Thunderbolt");
        assert_eq!(q1_weapon_display_name("rogue:lava-nailgun"), "Lava Nailgun");
        assert_eq!(q1_weapon_display_name("mg3:mjolnir"), "Mjolnir");
        assert_eq!(q2_weapons().count(), 20);
        assert!(q2_weapons()
            .any(|(name, id, label)| (*name, *id, *label) == ("heatbeam", "q2:weapon_plasmabeam", "Plasma Beam")));
        assert_eq!(Q3_WEAPONS.len(), 13);
        for id in ids {
            let item = format!("q1:weapon/{id}");
            let views = vec![WeaponBindingItemView {
                id: item.clone(),
                kind: BindableItemKind::Weapon,
            }];
            assert_eq!(
                weapon_binding_item(&format!("use {id}"), &views).as_deref(),
                Some(item.as_str()),
                "alias for {id}"
            );
            let display = q1_weapon_display_name(id);
            assert_eq!(
                weapon_binding_item(&format!("use {display}"), &views).as_deref(),
                Some(item.as_str()),
                "display name for {id}"
            );
        }
        let heatbeam = vec![WeaponBindingItemView {
            id: "q2:weapon_plasmabeam".to_string(),
            kind: BindableItemKind::Weapon,
        }];
        assert_eq!(
            weapon_binding_item("use heatbeam", &heatbeam).as_deref(),
            Some("q2:weapon_plasmabeam")
        );
        assert_eq!(
            weapon_binding_item("use Plasma Beam", &heatbeam).as_deref(),
            Some("q2:weapon_plasmabeam")
        );
    }

    #[test]
    fn weapon_scripts_stay_opaque() {
        let items = vec![WeaponBindingItemView {
            id: "q1:weapon/axe".to_string(),
            kind: BindableItemKind::Weapon,
        }];
        assert_eq!(weapon_binding_item("use axe; echo hi", &items), None);
        assert_eq!(weapon_binding_item("use axe // comment", &items), None);
        assert_eq!(weapon_binding_item("use axe /* comment */", &items), None);
        assert_eq!(weapon_binding_item("use ax\\e", &items), None);
        assert_eq!(weapon_binding_item("bind axe", &items), None);
        assert_eq!(weapon_binding_item("use ", &items), None);
        assert_eq!(weapon_binding_item("use \"axe", &items), None);
    }

    #[test]
    fn catalog_rows_follow_dialect_and_capabilities() {
        let items = vec![
            BindableItem {
                id: "q1:weapon/axe".to_string(),
                label: "Axe".to_string(),
                kind: BindableItemKind::Weapon,
            },
            BindableItem {
                id: "q1:powerup/quad".to_string(),
                label: "Quad".to_string(),
                kind: BindableItemKind::Powerup,
            },
        ];
        let full = shared_binding_actions(CommandDialect::Q1Netquake, &items, &capabilities());
        let jump = full.iter().find(|action| action.id == "jump").unwrap();
        assert_eq!(jump.target, InputBindingTarget::Command("+jump".to_string()));
        let q3 = shared_binding_actions(CommandDialect::Q3, &[], &capabilities());
        let jump = q3.iter().find(|action| action.id == "jump").unwrap();
        assert_eq!(jump.target, InputBindingTarget::Command("+moveup".to_string()));
        let wheel = full.iter().find(|action| action.id == "weapon-wheel").unwrap();
        assert!(wheel.matches.as_ref().unwrap()(&InputBindingTarget::Command(
            "+wheel".to_string()
        )));
        assert!(!wheel.matches.as_ref().unwrap()(&InputBindingTarget::Command(
            "+attack".to_string()
        )));
        let scores = full.iter().find(|action| action.id == "scores").unwrap();
        assert!(scores.matches.as_ref().unwrap()(&InputBindingTarget::Command(
            "+showscores".to_string()
        )));
        assert!(full.iter().any(|action| action.id == "chat"));
        assert!(full.iter().any(|action| action.id == "grapple"));
        let bare = BindingCapabilities {
            chat: false,
            score_command: None,
            offhand_grapple: false,
            offhand_grenades: false,
        };
        let minimal = shared_binding_actions(CommandDialect::Q3, &[], &bare);
        assert!(!minimal.iter().any(|action| action.id == "chat"));
        assert!(!minimal.iter().any(|action| action.id == "grapple"));
        let scores = minimal.iter().find(|action| action.id == "scores").unwrap();
        assert_eq!(scores.target, InputBindingTarget::Command("+scores".to_string()));
        let scored = BindingCapabilities {
            score_command: Some(ScoreCommand::Score),
            ..bare
        };
        let custom = shared_binding_actions(CommandDialect::Q3, &[], &scored);
        let scores = custom.iter().find(|action| action.id == "scores").unwrap();
        assert_eq!(scores.target, InputBindingTarget::Command("score".to_string()));
        let axe = full.iter().find(|action| action.id == "item:q1:weapon/axe").unwrap();
        assert_eq!(axe.label, "Select Axe");
        assert!(axe.matches.as_ref().unwrap()(&InputBindingTarget::Command(
            "use axe".to_string()
        )));
        let quad = full.iter().find(|action| action.id == "item:q1:powerup/quad").unwrap();
        assert_eq!(quad.label, "Use Quad");
    }
}
