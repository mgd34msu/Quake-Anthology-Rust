//! Classic Quake II guest player presentation and local command mapping.
//!
//! Port of donor `src/app/bootstrap/simulation/classic-guest-player.ts`
//! (`classicGuestPlayerView`, `classicGuestPlayerUi`,
//! `classicGuestLocalCommand`).

use std::collections::HashMap;

use qa_content::contract::{ArmorState, ItemId, PoweredProtectionState, ProviderReference, RegularArmorState};
use qa_content::q2::foundation::weapons::types::Q2WeaponDefinition;
use qa_core::math::{Vec3, Vec4};
use qa_net::common::commands::{ActorCommand, CommandSource, UserCommand};
use qa_net::protocol::ProtocolIdentity;
use qa_net::q2_adapters::{Q2PlayerState, Q2UserCommand, Q2Vec3, Q2Vec4};
use thiserror::Error;

use super::arsenal::selected::ArsenalAmmoWarning;
use super::arsenal::weapon_status::q2_weapon_status;
use super::types::{PlayerUi, PlayerView, UiAmmo};
use crate::bootstrap::network::q2_layout::q2_application_layout;

/// Remote armor item reported for classic guest players.
const REMOTE_ARMOR_ITEM: &str = "q2:remote-armor";

/// Local command mapping failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ClassicGuestCommandError {
    /// Remote commands must enter through their wire endpoint.
    #[error("Native remote commands must enter ClientThink through their wire endpoint")]
    RemoteCommand,
    /// Non-classic movement commands cannot drive an API 3 player.
    #[error("An API 3 native player requires classic Quake II movement commands")]
    DialectMismatch,
}

fn classic_model_base() -> u32 {
    q2_application_layout(ProtocolIdentity::Q2Classic)
        .expect("Q2 classic has an application layout")
        .models
}

fn to_vec3(value: &Q2Vec3) -> Vec3 {
    Vec3 {
        x: value.x as f32,
        y: value.y as f32,
        z: value.z as f32,
    }
}

fn to_vec4(value: &Q2Vec4) -> Vec4 {
    Vec4 {
        x: value.x as f32,
        y: value.y as f32,
        z: value.z as f32,
        w: value.w as f32,
    }
}

fn stat(stats: &[i16], index: usize) -> i16 {
    stats.get(index).copied().unwrap_or(0)
}

/// API 3 exposes the same player state as a native client, independently of
/// private gclient fields.
#[must_use]
pub fn classic_guest_player_view(state: &Q2PlayerState) -> PlayerView {
    let origin = state.movement.origin_eighths;
    let offset = &state.view.view_offset;
    PlayerView {
        client_view_offset_delta: None,
        blend: Some(to_vec4(&state.blend)),
        damage_blend: None,
        origin: Vec3 {
            x: (f64::from(origin[0]) / 8.0 + offset.x) as f32,
            y: (f64::from(origin[1]) / 8.0 + offset.y) as f32,
            z: (f64::from(origin[2]) / 8.0) as f32,
        },
        angles: to_vec3(&state.view.view_angles),
        view_height: offset.z,
        kick_angles: Some(to_vec3(&state.view.kick_angles)),
        field_of_view: Some(f64::from(state.view.fov)),
        foreign_character_death: false,
        pitch_drift: None,
    }
}

/// Only the selected weapon's ammo is public here; full inventory comes from
/// svc_inventory.
#[must_use]
pub fn classic_guest_player_ui(
    state: &Q2PlayerState,
    configstrings: &HashMap<u32, String>,
    source: ProviderReference,
    definitions: &[Q2WeaponDefinition],
) -> PlayerUi {
    let model = if state.view.gun_index == 0 {
        None
    } else {
        configstrings.get(&(classic_model_base() + state.view.gun_index as u32))
    };
    let weapon = model.and_then(|model| definitions.iter().find(|definition| definition.view_model == *model));
    let armor = stat(&state.view.stats, 5);
    let ammo = stat(&state.view.stats, 3);
    PlayerUi {
        selected_arsenal: false,
        native_inventory: None,
        powerups: Vec::new(),
        weapon_status: q2_weapon_status(weapon, |_: &ItemId| i32::from(ammo), source),
        arsenal_warning: ArsenalAmmoWarning::None,
        health: f64::from(stat(&state.view.stats, 1)),
        armor: ArmorState {
            powered: PoweredProtectionState::None,
            regular: if armor == 0 {
                RegularArmorState::None
            } else {
                RegularArmorState::Q2 {
                    points: f64::from(armor),
                    normal_protection: 0.0,
                    energy_protection: 0.0,
                    item: REMOTE_ARMOR_ITEM.to_string(),
                }
            },
        },
        active_weapon: weapon.map(|weapon| weapon.item.clone()),
        ammo: match weapon {
            None => None,
            Some(weapon) => weapon.ammo.as_ref().map(|item| UiAmmo {
                item: item.clone(),
                count: f64::from(ammo),
            }),
        },
        inventory: Vec::new(),
        items: Vec::new(),
    }
}

/// Local input contains absolute aim; native ClientThink adds
/// pmove.delta_angles to its command.
pub fn classic_guest_local_command(
    input: &ActorCommand,
    state: &Q2PlayerState,
) -> Result<Q2UserCommand, ClassicGuestCommandError> {
    if matches!(input.source, CommandSource::Remote { .. }) {
        return Err(ClassicGuestCommandError::RemoteCommand);
    }
    let UserCommand::Q2Classic {
        milliseconds,
        angle_shorts,
        forward_move,
        side_move,
        up_move,
        buttons,
        impulse,
        light_level,
    } = &input.command
    else {
        return Err(ClassicGuestCommandError::DialectMismatch);
    };
    let delta = state.movement.delta_angle_shorts;
    let angles = [
        (angle_shorts[0].round() as i32).wrapping_sub(i32::from(delta[0])) & 0xffff,
        (angle_shorts[1].round() as i32).wrapping_sub(i32::from(delta[1])) & 0xffff,
        (angle_shorts[2].round() as i32).wrapping_sub(i32::from(delta[2])) & 0xffff,
    ];
    Ok(Q2UserCommand {
        milliseconds: milliseconds.round() as u8,
        angle_shorts: [
            angles[0] as u16 as i16,
            angles[1] as u16 as i16,
            angles[2] as u16 as i16,
        ],
        forward_move: forward_move.round() as i16,
        side_move: side_move.round() as i16,
        up_move: up_move.round() as i16,
        buttons: buttons.round() as u8,
        impulse: impulse.round() as u8,
        light_level: light_level.round() as u8,
    })
}

#[cfg(test)]
mod tests {
    use qa_core::identity::{ActorId, ClientId, IdentityOwner};

    use super::*;
    use qa_net::q2_adapters::{Q2MovementState, Q2PlayerView as Q2View};

    fn owner() -> IdentityOwner {
        IdentityOwner::create("classic-guest-player-test").expect("owner")
    }

    fn actor() -> ActorId {
        owner().actor(1, 1)
    }

    fn client() -> ClientId {
        owner().client(1, 1)
    }

    fn state() -> Q2PlayerState {
        Q2PlayerState {
            view: Q2View {
                view_angles: Q2Vec3 {
                    x: 10.0,
                    y: 20.0,
                    z: 30.0,
                },
                view_offset: Q2Vec3 {
                    x: 1.0,
                    y: 2.0,
                    z: 22.0,
                },
                kick_angles: Q2Vec3 { x: 3.0, y: 4.0, z: 5.0 },
                gun_angles: Q2Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                gun_offset: Q2Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                gun_index: 0,
                gun_frame: 0,
                fov: 90,
                render_flags: 0,
                stats: vec![0, 100, 0, 12, 0, 50],
            },
            movement: Q2MovementState {
                move_type: 0,
                origin_eighths: [80, 160, 240],
                velocity_eighths: [0, 0, 0],
                flags: 0,
                time: 0,
                gravity: 800,
                delta_angle_shorts: [100, 200, 300],
            },
            blend: Q2Vec4 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                w: 1.0,
            },
        }
    }

    fn input(command: UserCommand) -> ActorCommand {
        ActorCommand {
            actor: actor(),
            source: CommandSource::Bot { client: client() },
            sequence: 7,
            command,
            arsenal: None,
        }
    }

    #[test]
    fn view_adds_offset_to_planar_origin_only() {
        let view = classic_guest_player_view(&state());
        assert_eq!(
            view.origin,
            Vec3 {
                x: 11.0,
                y: 22.0,
                z: 30.0
            }
        );
        assert_eq!(view.view_height, 22.0);
        assert_eq!(view.field_of_view, Some(90.0));
        assert_eq!(
            view.angles,
            Vec3 {
                x: 10.0,
                y: 20.0,
                z: 30.0
            }
        );
    }

    #[test]
    fn ui_reports_health_armor_and_no_weapon() {
        let source = ProviderReference {
            provider: qa_core::identity::ProviderId::new("test", "native"),
            content: qa_content::contract::ContentId("q2:baseq2:baseq2:1".to_string()),
        };
        let ui = classic_guest_player_ui(&state(), &HashMap::new(), source, &[]);
        assert_eq!(ui.health, 100.0);
        assert!(matches!(ui.armor.regular, RegularArmorState::Q2 { .. }));
        assert!(ui.active_weapon.is_none());
        assert!(ui.ammo.is_none());
        assert!(ui.weapon_status.is_none());
    }

    #[test]
    fn local_command_subtracts_delta_angles_with_wrap() {
        let command = UserCommand::Q2Classic {
            milliseconds: 50.0,
            angle_shorts: [50.0, 200.0, 70000.0],
            forward_move: 1.0,
            side_move: 2.0,
            up_move: 3.0,
            buttons: 4.0,
            impulse: 5.0,
            light_level: 6.0,
        };
        let out = classic_guest_local_command(&input(command), &state()).expect("command");
        assert_eq!(out.milliseconds, 50);
        assert_eq!(
            out.angle_shorts,
            [
                (50i32.wrapping_sub(100) & 0xffff) as u16 as i16,
                0,
                (70000i32.wrapping_sub(300) & 0xffff) as u16 as i16,
            ]
        );
        assert_eq!((out.forward_move, out.side_move, out.up_move), (1, 2, 3));
    }

    #[test]
    fn local_command_rejects_remote_source() {
        let mut bad = input(UserCommand::Q2Classic {
            milliseconds: 0.0,
            angle_shorts: [0.0, 0.0, 0.0],
            forward_move: 0.0,
            side_move: 0.0,
            up_move: 0.0,
            buttons: 0.0,
            impulse: 0.0,
            light_level: 0.0,
        });
        bad.source = CommandSource::Remote { client: client() };
        assert_eq!(
            classic_guest_local_command(&bad, &state()),
            Err(ClassicGuestCommandError::RemoteCommand)
        );
    }

    #[test]
    fn local_command_rejects_other_dialects() {
        let bad = input(UserCommand::Q3 {
            server_time_milliseconds: 0.0,
            angle_words: [0.0, 0.0, 0.0],
            buttons: 0.0,
            weapon: 0.0,
            forward_move: 0.0,
            right_move: 0.0,
            up_move: 0.0,
        });
        assert_eq!(
            classic_guest_local_command(&bad, &state()),
            Err(ClassicGuestCommandError::DialectMismatch)
        );
    }
}
