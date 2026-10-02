//! Q3 arsenal control resolution at the native-movement boundary.
//!
//! Port of donor `src/app/bootstrap/simulation/arsenal-intent.ts`
//! (`resolveQ3ArsenalControls`).

use qa_content::q3::base::shared::definitions::{Product, Weapon};
use qa_content::q3::foundation::arsenal::{Q3ArsenalControls, Q3_WEAPON_ITEMS};
use qa_net::common::commands::ArsenalIntent;
use qa_world::movement::types::{ArsenalState, UserCommand, WeaponState};

/// Errors resolving Q3 arsenal controls.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ArsenalIntentError {
    /// Arsenal is not in Q3 state.
    #[error("Q3 controls require the selected Q3 arsenal")]
    NotQ3Arsenal,
    /// Intent belongs to a different provider.
    #[error("Arsenal command belongs to a different provider")]
    ForeignProvider,
    /// Named weapon is not in the selected product.
    #[error("Weapon does not belong to the selected Q3 product")]
    ForeignWeapon,
}

/// Source weapon numbers above this are mission-pack only (`baseq3` cap).
const BASEQ3_MAX_WEAPON: i32 = Weapon::WpGrapplingHook as i32;

/// Native movement fields and explicit selected-arsenal commands meet at this boundary.
pub fn resolve_q3_arsenal_controls(
    arsenal: &ArsenalState,
    intent: Option<&ArsenalIntent>,
    command: &UserCommand,
    product: Product,
) -> Result<Q3ArsenalControls, ArsenalIntentError> {
    let WeaponState::Q3 { source_weapon, .. } = arsenal.state else {
        return Err(ArsenalIntentError::NotQ3Arsenal);
    };
    let (command_weapon, buttons, is_q3) = match *command {
        UserCommand::Q3(cmd) => (Some(cmd.weapon), cmd.buttons, true),
        UserCommand::Q1Netquake(cmd) => (None, cmd.buttons, false),
        UserCommand::Q1Quakeworld(cmd) => (None, cmd.buttons, false),
        UserCommand::Q2Classic(cmd) => (None, cmd.buttons, false),
        UserCommand::Q2Rerelease(cmd) => (None, cmd.buttons, false),
    };
    let mut requested_weapon = command_weapon.unwrap_or(source_weapon);
    let mut use_holdable = None;
    if let Some(intent) = intent {
        let owner = format!("{}:{}", arsenal.provider.namespace, arsenal.provider.name);
        if intent.provider != owner {
            return Err(ArsenalIntentError::ForeignProvider);
        }
        requested_weapon = source_weapon;
        if let Some(weapon) = &intent.weapon {
            let entry = Q3_WEAPON_ITEMS
                .iter()
                .find(|entry| &entry.item == weapon)
                .filter(|entry| product == Product::Missionpack || entry.weapon as i32 <= BASEQ3_MAX_WEAPON)
                .ok_or(ArsenalIntentError::ForeignWeapon)?;
            requested_weapon = entry.weapon as i32;
        }
        use_holdable = Some(intent.use_holdable);
    }
    Ok(Q3ArsenalControls {
        attack: buttons & 1 != 0,
        use_holdable: use_holdable.unwrap_or(is_q3 && buttons & 4 != 0),
        requested_weapon,
    })
}

#[cfg(test)]
mod tests {
    use qa_core::identity::ProviderId;
    use qa_world::movement::types::Q3UserCommand;

    use super::*;

    fn arsenal(source_weapon: i32) -> ArsenalState {
        ArsenalState {
            provider: ProviderId::new("sim", "test"),
            active_weapon: None,
            state: WeaponState::Q3 {
                source_weapon,
                state: 0,
                time_milliseconds: 0,
            },
            ammo: Vec::new(),
        }
    }

    fn q3_command(weapon: i32, buttons: i32) -> UserCommand {
        UserCommand::Q3(Q3UserCommand {
            server_time_milliseconds: 0,
            angle_words: [0; 3],
            buttons,
            weapon,
            forward_move: 0,
            right_move: 0,
            up_move: 0,
        })
    }

    #[test]
    fn q3_command_drives_request_without_intent() {
        let controls = resolve_q3_arsenal_controls(&arsenal(2), None, &q3_command(5, 1), Product::Baseq3).unwrap();
        assert_eq!(controls.requested_weapon, 5);
        assert!(controls.attack);
        assert!(!controls.use_holdable);
    }

    #[test]
    fn use_button_maps_to_holdable_without_intent() {
        let controls = resolve_q3_arsenal_controls(&arsenal(2), None, &q3_command(2, 4), Product::Baseq3).unwrap();
        assert!(controls.use_holdable);
        assert!(!controls.attack);
    }

    #[test]
    fn intent_weapon_resolves_through_catalog() {
        let intent = ArsenalIntent {
            provider: "sim:test".to_string(),
            weapon: Some("q3:weapon/railgun".to_string()),
            use_holdable: true,
        };
        let controls = resolve_q3_arsenal_controls(&arsenal(2), None, &q3_command(5, 0), Product::Baseq3).unwrap();
        assert_eq!(controls.requested_weapon, 5);
        let controls =
            resolve_q3_arsenal_controls(&arsenal(2), Some(&intent), &q3_command(5, 0), Product::Baseq3).unwrap();
        assert_eq!(controls.requested_weapon, Weapon::WpRailgun as i32);
        assert!(controls.use_holdable);
    }

    #[test]
    fn rejects_foreign_provider_and_product() {
        let foreign = ArsenalIntent {
            provider: "other:mod".to_string(),
            weapon: None,
            use_holdable: false,
        };
        assert_eq!(
            resolve_q3_arsenal_controls(&arsenal(2), Some(&foreign), &q3_command(2, 0), Product::Baseq3),
            Err(ArsenalIntentError::ForeignProvider)
        );
        let prox = ArsenalIntent {
            provider: "sim:test".to_string(),
            weapon: Some("q3:weapon/proxlauncher".to_string()),
            use_holdable: false,
        };
        assert_eq!(
            resolve_q3_arsenal_controls(&arsenal(2), Some(&prox), &q3_command(2, 0), Product::Baseq3),
            Err(ArsenalIntentError::ForeignWeapon)
        );
        assert!(resolve_q3_arsenal_controls(&arsenal(2), Some(&prox), &q3_command(2, 0), Product::Missionpack).is_ok());
    }

    #[test]
    fn rejects_non_q3_arsenal() {
        let mut state = arsenal(2);
        state.state = WeaponState::Q1 {
            frame: 0,
            attack_finished_seconds: 0.0,
            source_weapon: 1,
        };
        assert_eq!(
            resolve_q3_arsenal_controls(&state, None, &q3_command(2, 0), Product::Baseq3),
            Err(ArsenalIntentError::NotQ3Arsenal)
        );
    }
}
