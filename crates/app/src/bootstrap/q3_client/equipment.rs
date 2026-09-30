//! Quake III equipment command adaptation.
//!
//! Port of `src/app/bootstrap/q3-client/equipment.ts`
//! (`q3EquipmentCommand`). The arsenal warning reuses the existing
//! [`ArsenalAmmoWarning`](qa_client::ui::types::ArsenalAmmoWarning) and the
//! command reuses [`WireUserCommand`](qa_net::q3::WireUserCommand). The donor
//! transform is total, so [`Q3EquipmentError`] only reserves the module's
//! error domain.

use qa_client::ui::types::ArsenalAmmoWarning;
use qa_net::q3::WireUserCommand;
use thiserror::Error;

/// Quake III equipment presentation (`Q3EquipmentPresentation`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3EquipmentPresentation {
    /// Predicted primary weapon.
    pub primary_weapon: u8,
    /// Ammo warning.
    pub warning: ArsenalAmmoWarning,
}

/// Error for Quake III equipment commands.
///
/// The donor transform is total; this reserves the module's error domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum Q3EquipmentError {}

/// Keep the original predicted held weapon while the selected arsenal owns
/// attack input (donor `q3EquipmentCommand`).
pub fn q3_equipment_command(
    command: &WireUserCommand,
    equipment: Option<&Q3EquipmentPresentation>,
) -> WireUserCommand {
    let Some(equipment) = equipment else {
        return command.clone();
    };
    let mut result = command.clone();
    result.weapon = equipment.primary_weapon;
    result.buttons &= !1;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command() -> WireUserCommand {
        WireUserCommand {
            server_time: 100,
            angles: [1, 2, 3],
            moves: [4, 5, 6],
            buttons: 0b101,
            weapon: 9,
        }
    }

    #[test]
    fn missing_equipment_preserves_command() {
        assert_eq!(q3_equipment_command(&command(), None), command());
    }

    #[test]
    fn equipment_replaces_weapon_and_clears_attack() {
        for warning in [
            ArsenalAmmoWarning::None,
            ArsenalAmmoWarning::Low,
            ArsenalAmmoWarning::Empty,
        ] {
            let equipment = Q3EquipmentPresentation {
                primary_weapon: 2,
                warning,
            };
            let adapted = q3_equipment_command(&command(), Some(&equipment));
            assert_eq!(adapted.weapon, 2);
            assert_eq!(adapted.buttons, 0b100);
            assert_eq!(adapted.server_time, 100);
            assert_eq!(adapted.angles, [1, 2, 3]);
            assert_eq!(adapted.moves, [4, 5, 6]);
        }
    }

    #[test]
    fn attack_bit_edge_cases() {
        let equipment = Q3EquipmentPresentation {
            primary_weapon: 2,
            warning: ArsenalAmmoWarning::None,
        };
        let mut cleared = command();
        cleared.buttons = 0;
        assert_eq!(
            q3_equipment_command(&cleared, Some(&equipment)).buttons,
            0
        );
        let mut all = command();
        all.buttons = 0xFFFF;
        assert_eq!(
            q3_equipment_command(&all, Some(&equipment)).buttons,
            0xFFFE
        );
    }
}
