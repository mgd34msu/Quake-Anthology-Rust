//! Rerelease Quake II guest player presentation and local command mapping.
//!
//! Port of donor `src/app/bootstrap/simulation/rerelease-guest-player.ts`
//! (`rereleaseGuestPlayerView`, `rereleaseGuestPlayerUi`,
//! `rereleaseGuestLocalCommand`).

use std::collections::HashMap;

use qa_content::contract::{ArmorState, ItemId, PoweredProtectionState, ProviderReference, RegularArmorState};
use qa_content::q2::foundation::weapons::types::Q2WeaponDefinition;
use qa_core::math::{Vec3, Vec4};
use qa_net::common::commands::{ActorCommand, CommandSource, UserCommand};
use qa_net::protocol::ProtocolIdentity;
use qa_net::q2_adapters::{Q2RereleasePlayerState, Q2RereleaseUserCommand, Q2Vec3, Q2Vec4};
use thiserror::Error;

use super::arsenal::selected::ArsenalAmmoWarning;
use super::arsenal::weapon_status::q2_weapon_status;
use super::types::{PlayerUi, PlayerView, UiAmmo};
use crate::bootstrap::network::q2_layout::q2_application_layout;

/// Remote armor item reported for rerelease guest players.
const REMOTE_ARMOR_ITEM: &str = "q2:remote-armor";

/// Local command mapping failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RereleaseGuestCommandError {
    /// Remote commands must enter through their wire endpoint.
    #[error("Native remote commands must enter ClientThink through their wire endpoint")]
    RemoteCommand,
    /// Non-rerelease movement commands cannot drive an API2023 player.
    #[error("An API2023 native player requires rerelease Quake II movement commands")]
    DialectMismatch,
}

fn rerelease_model_base() -> u32 {
    q2_application_layout(ProtocolIdentity::Q2Rerelease)
        .expect("Q2 rerelease has an application layout")
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

/// Rerelease keeps stance height in pmove, separately from camera bob and
/// damage kick.
#[must_use]
pub fn rerelease_guest_player_view(state: &Q2RereleasePlayerState) -> PlayerView {
    let origin = &state.movement.origin;
    let offset = &state.view.view_offset;
    PlayerView {
        client_view_offset_delta: None,
        blend: Some(to_vec4(&state.screen_blend)),
        damage_blend: Some(to_vec4(&state.damage_blend)),
        origin: Vec3 {
            x: (origin.x + offset.x) as f32,
            y: (origin.y + offset.y) as f32,
            z: (origin.z + offset.z) as f32,
        },
        angles: to_vec3(&state.view.view_angles),
        view_height: f64::from(state.movement.view_height),
        kick_angles: Some(to_vec3(&state.view.kick_angles)),
        field_of_view: Some(f64::from(state.view.fov)),
        foreign_character_death: false,
        pitch_drift: None,
    }
}

/// Weapon identity comes from the selected provider and public gun-model
/// configstring.
#[must_use]
pub fn rerelease_guest_player_ui(
    state: &Q2RereleasePlayerState,
    configstrings: &HashMap<u32, String>,
    source: ProviderReference,
    definitions: &[Q2WeaponDefinition],
) -> PlayerUi {
    let model = if state.view.gun_index == 0 {
        None
    } else {
        configstrings.get(&(rerelease_model_base() + state.view.gun_index as u32))
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

/// Native rerelease ClientThink adds float delta angles; local input already
/// contains absolute aim.
pub fn rerelease_guest_local_command(
    input: &ActorCommand,
    state: &Q2RereleasePlayerState,
) -> Result<Q2RereleaseUserCommand, RereleaseGuestCommandError> {
    if matches!(input.source, CommandSource::Remote { .. }) {
        return Err(RereleaseGuestCommandError::RemoteCommand);
    }
    let UserCommand::Q2Rerelease {
        milliseconds,
        angles,
        forward_move,
        side_move,
        buttons,
        server_frame,
    } = &input.command
    else {
        return Err(RereleaseGuestCommandError::DialectMismatch);
    };
    let delta = &state.movement.delta_angles;
    Ok(Q2RereleaseUserCommand {
        milliseconds: milliseconds.round() as u8,
        angles: Q2Vec3 {
            x: f64::from((angles[0] - delta.x) as f32),
            y: f64::from((angles[1] - delta.y) as f32),
            z: f64::from((angles[2] - delta.z) as f32),
        },
        forward_move: forward_move.round() as i16,
        side_move: side_move.round() as i16,
        buttons: buttons.round() as u8,
        server_frame: server_frame.round() as i32,
    })
}

#[cfg(test)]
mod tests {
    use qa_content::contract::ContentId;
    use qa_core::identity::{ActorId, ClientId, IdentityOwner, ProviderId};

    use super::*;
    use qa_net::q2_adapters::{Q2PlayerView as Q2View, Q2RereleaseMovementState};

    fn owner() -> IdentityOwner {
        IdentityOwner::create("rerelease-guest-player-test").expect("owner")
    }

    fn actor() -> ActorId {
        owner().actor(1, 1)
    }

    fn client() -> ClientId {
        owner().client(1, 1)
    }

    fn source() -> ProviderReference {
        ProviderReference {
            provider: ProviderId::new("test", "native"),
            content: ContentId("q2:rerelease:baseq2:1".to_string()),
        }
    }

    fn state() -> Q2RereleasePlayerState {
        Q2RereleasePlayerState {
            view: Q2View {
                view_angles: Q2Vec3 {
                    x: 10.0,
                    y: 20.0,
                    z: 30.0,
                },
                view_offset: Q2Vec3 { x: 1.0, y: 2.0, z: 3.0 },
                kick_angles: Q2Vec3 { x: 3.0, y: 4.0, z: 5.0 },
                gun_angles: Q2Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                gun_offset: Q2Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                gun_index: 0,
                gun_frame: 0,
                fov: 90,
                render_flags: 0,
                stats: vec![0, 80, 0, 7, 0, 0],
            },
            movement: Q2RereleaseMovementState {
                move_type: 0,
                origin: Q2Vec3 {
                    x: 100.0,
                    y: 200.0,
                    z: 300.0,
                },
                velocity: Q2Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                flags: 0,
                time: 0,
                gravity: 800,
                delta_angles: Q2Vec3 { x: 1.5, y: 2.5, z: 3.5 },
                view_height: 22,
            },
            gun_skin: 0,
            gun_rate: 0,
            screen_blend: Q2Vec4 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                w: 1.0,
            },
            damage_blend: Q2Vec4 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
                w: 0.5,
            },
            team_id: 0,
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
    fn view_rounds_origin_through_f32() {
        let view = rerelease_guest_player_view(&state());
        assert_eq!(
            view.origin,
            Vec3 {
                x: 101.0,
                y: 202.0,
                z: 303.0
            }
        );
        assert_eq!(view.view_height, 22.0);
        assert!(view.damage_blend.is_some());
    }

    #[test]
    fn ui_reports_zero_armor_as_none() {
        let ui = rerelease_guest_player_ui(&state(), &HashMap::new(), source(), &[]);
        assert_eq!(ui.health, 80.0);
        assert!(matches!(ui.armor.regular, RegularArmorState::None));
        assert!(ui.active_weapon.is_none());
    }

    #[test]
    fn local_command_subtracts_float_delta_angles() {
        let command = UserCommand::Q2Rerelease {
            milliseconds: 16.0,
            angles: [11.5, 22.5, 33.5],
            forward_move: 1.0,
            side_move: 2.0,
            buttons: 3.0,
            server_frame: 44.0,
        };
        let out = rerelease_guest_local_command(&input(command), &state()).expect("command");
        assert_eq!(out.milliseconds, 16);
        assert_eq!(
            (out.angles.x, out.angles.y, out.angles.z),
            (f64::from(10.0f32), f64::from(20.0f32), f64::from(30.0f32))
        );
        assert_eq!(out.server_frame, 44);
    }

    #[test]
    fn local_command_rejects_remote_source() {
        let mut bad = input(UserCommand::Q2Rerelease {
            milliseconds: 0.0,
            angles: [0.0, 0.0, 0.0],
            forward_move: 0.0,
            side_move: 0.0,
            buttons: 0.0,
            server_frame: 0.0,
        });
        bad.source = CommandSource::Remote { client: client() };
        assert_eq!(
            rerelease_guest_local_command(&bad, &state()),
            Err(RereleaseGuestCommandError::RemoteCommand)
        );
    }

    #[test]
    fn local_command_rejects_other_dialects() {
        let bad = input(UserCommand::Q2Classic {
            milliseconds: 0.0,
            angle_shorts: [0.0, 0.0, 0.0],
            forward_move: 0.0,
            side_move: 0.0,
            up_move: 0.0,
            buttons: 0.0,
            impulse: 0.0,
            light_level: 0.0,
        });
        assert_eq!(
            rerelease_guest_local_command(&bad, &state()),
            Err(RereleaseGuestCommandError::DialectMismatch)
        );
    }
}
