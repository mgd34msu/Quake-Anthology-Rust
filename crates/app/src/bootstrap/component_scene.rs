//! Component scene snapshot selection.
//!
//! Donor: `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/component-scene.ts`
//! (`selectComponentScene`).
//! The mod scene publication arrives as the absorbed
//! [`ComponentScenePublication`] pick (donor `QvmModScenePublication`,
//! unported) with game state as an opaque passthrough; visibility runs
//! through [`select_application_q3_snapshot`], and server commands
//! tokenize per recipient. The selected context mirrors the donor's
//! `QvmSceneContext` shape over the ported Q3 state types.

use std::collections::HashMap;

use qa_core::cmd::{tokenize_command, CmdError, Dialect, TextMode};
use qa_core::identity::ActorId;
use qa_core::math::Bounds;
use qa_net::q3_net::{Q3EntityState, Q3PlayerState};
use thiserror::Error;

use super::q3_client::visibility::{
    select_application_q3_snapshot, ApplicationQ3SceneQueries, ApplicationQ3SourceEntity, ApplicationQ3VisibilityError,
};

/// Component scene failure.
#[derive(Debug, Error)]
pub enum ComponentSceneError {
    /// No admitted viewing player matches the viewer.
    #[error("Component snapshot has no admitted viewing player")]
    NoViewingPlayer,
    /// Visibility selection failure.
    #[error(transparent)]
    Visibility(#[from] ApplicationQ3VisibilityError),
    /// Server command tokenize failure.
    #[error(transparent)]
    Command(#[from] CmdError),
}

/// One admitted viewing client (publication `clients` row pick).
#[derive(Debug, Clone, PartialEq)]
pub struct ComponentSceneClient {
    /// Client actor.
    pub actor: ActorId,
    /// Client slot.
    pub slot: i32,
    /// Player state.
    pub state: Q3PlayerState,
}

/// One published entity (publication `entities` row pick).
#[derive(Debug, Clone, PartialEq)]
pub struct ComponentSceneEntity {
    /// Entity actor.
    pub actor: ActorId,
    /// Entity state.
    pub state: Q3EntityState,
    /// Shared body bounds.
    pub bounds: Bounds,
    /// Owned flag.
    pub owned: bool,
    /// Whether the entity is linked.
    pub linked: bool,
    /// Server flags.
    pub server_flags: i32,
    /// Single-client target.
    pub single_client: i32,
}

/// One published server command (publication `commands` row pick).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentSceneCommand {
    /// Command sequence.
    pub sequence: u64,
    /// Recipient actor, if targeted.
    pub recipient: Option<ActorId>,
    /// Command text.
    pub text: String,
}

/// Absorbed mod scene publication (`QvmModScenePublication` pick).
#[derive(Debug, Clone, PartialEq)]
pub struct ComponentScenePublication<G> {
    /// Admitted viewing clients.
    pub clients: Vec<ComponentSceneClient>,
    /// Published entities.
    pub entities: Vec<ComponentSceneEntity>,
    /// Published server commands.
    pub commands: Vec<ComponentSceneCommand>,
    /// Scene revision.
    pub revision: u64,
    /// Game state (opaque passthrough).
    pub game_state: G,
    /// Game-state revision.
    pub game_state_revision: u64,
    /// Server time.
    pub server_time: i32,
}

/// Selected scene snapshot (snapshot half of the donor's context).
#[derive(Debug, Clone, PartialEq)]
pub struct ComponentSceneSnapshot {
    /// Server time.
    pub server_time: i32,
    /// Snapshot flags (always zero).
    pub flags: i32,
    /// Area mask (32 bytes).
    pub area_mask: [u8; 32],
    /// Viewer player state.
    pub player_state: Q3PlayerState,
    /// Visible entities.
    pub entities: Vec<Q3EntityState>,
    /// Server command sequence.
    pub server_command_sequence: u64,
}

/// One scene actor (donor `actors` row).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentSceneActor {
    /// Actor.
    pub actor: ActorId,
    /// Centity slot.
    pub slot: i32,
    /// Owned flag.
    pub owned: bool,
}

/// One scene server command (donor `commands` row).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentSceneCommandView {
    /// Command sequence.
    pub sequence: u64,
    /// Tokenized arguments (empty when addressed to another viewer).
    pub arguments: Vec<String>,
}

/// Selected component scene context (donor `QvmSceneContext` shape).
#[derive(Debug, Clone, PartialEq)]
pub struct ComponentSceneContext<G> {
    /// Scene revision.
    pub revision: u64,
    /// Game state (opaque passthrough).
    pub game_state: G,
    /// Game-state revision.
    pub game_state_revision: u64,
    /// Snapshot.
    pub snapshot: ComponentSceneSnapshot,
    /// Scene actors.
    pub actors: Vec<ComponentSceneActor>,
    /// Server commands.
    pub commands: Vec<ComponentSceneCommandView>,
}

/// Select the viewing player's scene (`selectComponentScene`).
pub fn select_component_scene<G: Clone>(
    source: &ComponentScenePublication<G>,
    viewer: &ActorId,
    queries: &dyn ApplicationQ3SceneQueries,
    leaf_count: i32,
    print: &mut dyn FnMut(&str),
) -> Result<ComponentSceneContext<G>, ComponentSceneError> {
    let Some(player) = source.clients.iter().find(|row| &row.actor == viewer) else {
        return Err(ComponentSceneError::NoViewingPlayer);
    };
    let bounds: HashMap<i32, Bounds> = source
        .entities
        .iter()
        .map(|row| (row.state.number, row.bounds))
        .collect();
    let rows: Vec<ApplicationQ3SourceEntity> = source
        .entities
        .iter()
        .map(|row| ApplicationQ3SourceEntity {
            state: row.state.clone(),
            linked: row.linked,
            server_flags: row.server_flags,
            single_client: row.single_client,
        })
        .collect();
    let visible = select_application_q3_snapshot(
        &player.state,
        &rows,
        queries,
        &|slot| bounds.get(&slot).copied(),
        leaf_count,
        print,
    )?;
    let mut area_mask = [0u8; 32];
    let mask_len = visible.area_mask.len().min(32);
    area_mask[..mask_len].copy_from_slice(&visible.area_mask[..mask_len]);
    Ok(ComponentSceneContext {
        revision: source.revision,
        game_state: source.game_state.clone(),
        game_state_revision: source.game_state_revision,
        snapshot: ComponentSceneSnapshot {
            server_time: source.server_time,
            flags: 0,
            area_mask,
            player_state: player.state.clone(),
            entities: visible.entities,
            server_command_sequence: source.commands.last().map_or(0, |command| command.sequence),
        },
        actors: source
            .entities
            .iter()
            .map(|row| ComponentSceneActor {
                actor: row.actor.clone(),
                slot: row.state.number,
                owned: row.owned,
            })
            .collect(),
        commands: source
            .commands
            .iter()
            .map(|command| {
                let addressed = command.recipient.is_none() || command.recipient.as_ref() == Some(viewer);
                let arguments = if addressed {
                    tokenize_command(&command.text, Dialect::Q3, TextMode::Source).map(|tokens| tokens.argv)
                } else {
                    Ok(Vec::new())
                };
                arguments.map(|arguments| ComponentSceneCommandView {
                    sequence: command.sequence,
                    arguments,
                })
            })
            .collect::<Result<Vec<_>, CmdError>>()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;
    use qa_core::math::{vec3, Vec3};
    use qa_net::q3_net::Q3Product;

    struct OpenQueries;

    impl ApplicationQ3SceneQueries for OpenQueries {
        fn point_leaf(&self, _point: Vec3) -> i32 {
            0
        }
        fn leaf_cluster(&self, _leaf: i32) -> i32 {
            0
        }
        fn leaf_area(&self, _leaf: i32) -> i32 {
            0
        }
        fn area_bits(&self, _area: i32) -> Vec<u8> {
            vec![0x01]
        }
        fn box_leaves(&self, _bounds: Bounds, _limit: i32) -> Vec<i32> {
            vec![0]
        }
        fn cluster_visible(&self, _from: i32, _cluster: i32) -> bool {
            true
        }
        fn areas_connected(&self, _first: i32, _second: i32) -> bool {
            true
        }
    }

    fn publication() -> (ComponentScenePublication<String>, ActorId, ActorId) {
        let authority = IdentityOwner::create("component-scene").unwrap();
        let viewer = authority.actor(1, 0);
        let other = authority.actor(2, 0);
        let mut player = Q3PlayerState::new(Q3Product::Base);
        player.client_num = 0;
        let source = ComponentScenePublication {
            clients: vec![ComponentSceneClient {
                actor: viewer.clone(),
                slot: 0,
                state: player,
            }],
            entities: vec![
                ComponentSceneEntity {
                    actor: viewer.clone(),
                    state: Q3EntityState {
                        number: 5,
                        ..Q3EntityState::default()
                    },
                    bounds: Bounds {
                        min: vec3(-8.0, -8.0, -8.0),
                        max: vec3(8.0, 8.0, 8.0),
                    },
                    owned: true,
                    linked: true,
                    server_flags: 0,
                    single_client: 0,
                },
                ComponentSceneEntity {
                    actor: other.clone(),
                    state: Q3EntityState {
                        number: 6,
                        ..Q3EntityState::default()
                    },
                    bounds: Bounds {
                        min: vec3(-8.0, -8.0, -8.0),
                        max: vec3(8.0, 8.0, 8.0),
                    },
                    owned: false,
                    linked: false,
                    server_flags: 0,
                    single_client: 0,
                },
            ],
            commands: vec![
                ComponentSceneCommand {
                    sequence: 1,
                    recipient: None,
                    text: "print \"hello world\"".to_string(),
                },
                ComponentSceneCommand {
                    sequence: 2,
                    recipient: Some(other.clone()),
                    text: "secret".to_string(),
                },
            ],
            revision: 9,
            game_state: "state".to_string(),
            game_state_revision: 4,
            server_time: 1200,
        };
        (source, viewer, other)
    }

    #[test]
    fn selects_viewer_snapshot_and_tokenizes_broadcasts() {
        let (source, viewer, _) = publication();
        let queries = OpenQueries;
        let mut printed = Vec::new();
        let selected = select_component_scene(&source, &viewer, &queries, 64, &mut |text| {
            printed.push(text.to_string())
        })
        .unwrap();
        assert_eq!(selected.revision, 9);
        assert_eq!(selected.game_state, "state");
        assert_eq!(selected.snapshot.server_time, 1200);
        assert_eq!(selected.snapshot.flags, 0);
        assert_eq!(selected.snapshot.area_mask[0], 0xFE);
        assert_eq!(selected.snapshot.server_command_sequence, 2);
        assert_eq!(selected.snapshot.entities.len(), 1);
        assert_eq!(selected.snapshot.entities[0].number, 5);
        assert_eq!(selected.actors.len(), 2);
        assert_eq!(selected.actors[0].slot, 5);
        assert!(selected.actors[0].owned);
        assert_eq!(
            selected.commands[0].arguments,
            vec!["print".to_string(), "hello world".to_string()]
        );
        assert!(selected.commands[1].arguments.is_empty());
        assert!(printed.is_empty());
    }

    #[test]
    fn rejects_unadmitted_viewers() {
        let (source, _, _) = publication();
        let foreign = IdentityOwner::create("foreign").unwrap().actor(1, 0);
        let queries = OpenQueries;
        assert!(matches!(
            select_component_scene(&source, &foreign, &queries, 64, &mut |_| {}).unwrap_err(),
            ComponentSceneError::NoViewingPlayer
        ));
    }
}
