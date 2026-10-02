//! Server-side component publication projection.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/network/unified-component-publication.ts`
//! (`UnifiedComponentPublisher`).

use std::collections::{HashMap, HashSet};

use qa_core::identity::ProviderId;

use super::unified_components::{
    same_owner, UnifiedComponentFrame, UnifiedComponentFrameScene, UnifiedComponentFrames, UnifiedComponentPublication,
    UnifiedComponentState, UnifiedComponentUpdate,
};
use super::unified_native_components::{
    project_native_components, UnifiedNativeProjectionError, UnifiedNativePublication,
};
use crate::persistence::mods::same_mod_identity;

/// Component publication failure.
#[derive(Debug, thiserror::Error)]
pub enum UnifiedComponentPublicationError {
    /// Native projection failure.
    #[error(transparent)]
    Native(#[from] UnifiedNativeProjectionError),
    /// Publication failure.
    #[error("{0}")]
    Publication(String),
}

/// Projection of component publications (donor `project` output).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedComponentProjection {
    /// Reliable update, when anything changed.
    pub update: Option<UnifiedComponentUpdate>,
    /// Volatile frames.
    pub frame: UnifiedComponentFrames,
}

/// One authenticated recipient's reliable cursor, independent from volatile frame delivery.
#[derive(Debug, Default)]
pub struct UnifiedComponentPublisher {
    revision: i64,
    native: Vec<UnifiedNativePublication>,
    sources: Vec<UnifiedComponentPublication>,
    sequences: HashMap<ProviderId, i64>,
}

impl UnifiedComponentPublisher {
    /// Empty publisher.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Project publications into a reliable update and volatile frames.
    pub fn project(
        &mut self,
        sources: &[UnifiedComponentPublication],
        native: &[UnifiedNativePublication],
    ) -> Result<UnifiedComponentProjection, UnifiedComponentPublicationError> {
        let previous: HashMap<&ProviderId, &UnifiedComponentPublication> = self
            .sources
            .iter()
            .map(|source| (&source.identity_block.owner.provider, source))
            .collect();
        let mut changed = sources.len() != self.sources.len()
            || sources.iter().enumerate().any(|(index, source)| {
                self.sources
                    .get(index)
                    .is_none_or(|old| !same_owner(&old.identity_block.owner, &source.identity_block.owner))
            });
        let mut states = Vec::with_capacity(sources.len());
        for source in sources {
            let identity = &source.identity_block;
            let old = previous.get(&identity.owner.provider).copied();
            let same = match old {
                Some(old) => {
                    let same = same_owner(&old.identity_block.owner, &identity.owner)
                        && old.identity_block.generation == identity.generation;
                    if same
                        && (!same_mod_identity(&old.identity_block.identity, &identity.identity)
                            || old.identity_block.abi != identity.abi
                            || old.identity_block.runtime != identity.runtime
                            || old.viewer != source.viewer)
                    {
                        return Err(UnifiedComponentPublicationError::Publication(
                            "Component activation changed identity or recipient".to_string(),
                        ));
                    }
                    same
                }
                None => false,
            };
            let scene = source.context.scene.as_ref();
            let sequence = scene.map(|scene| scene.snapshot.server_command_sequence).unwrap_or(0);
            let base = if same {
                self.sequences.get(&identity.owner.provider).copied().unwrap_or(0)
            } else {
                sequence
            };
            if sequence < base {
                return Err(UnifiedComponentPublicationError::Publication(
                    "Original component command sequence moved backward".to_string(),
                ));
            }
            let retained = scene.map(|scene| scene.commands.as_slice()).unwrap_or(&[]);
            let commands = retained
                .iter()
                .filter(|command| i128::from(command.sequence) > i128::from(base))
                .cloned()
                .collect::<Vec<_>>();
            let first = commands.first().map(|command| i128::from(command.sequence));
            let last = commands.last().map(|command| i128::from(command.sequence));
            if sequence != base && (first != Some(i128::from(base) + 1) || last != Some(i128::from(sequence))) {
                return Err(UnifiedComponentPublicationError::Publication(
                    "Original component reliable commands exceeded their retained window".to_string(),
                ));
            }
            let game_changed =
                !same || old.is_some_and(|old| old.context.game_state_revision != source.context.game_state_revision);
            if same && old.is_some_and(|old| source.context.game_state_revision < old.context.game_state_revision) {
                return Err(UnifiedComponentPublicationError::Publication(
                    "Original component configstring revision moved backward".to_string(),
                ));
            }
            changed |= !same || game_changed || !commands.is_empty();
            self.sequences.insert(identity.owner.provider.clone(), sequence);
            states.push(UnifiedComponentState {
                identity_block: identity.clone(),
                game_state_revision: source.context.game_state_revision,
                game_state: if game_changed {
                    Some(source.context.game_state.clone())
                } else {
                    None
                },
                command_base: base,
                commands,
            });
        }
        let live: HashSet<&ProviderId> = sources
            .iter()
            .map(|source| &source.identity_block.owner.provider)
            .collect();
        self.sequences.retain(|id, _| live.contains(id));
        let projected = project_native_components(&self.native, native)?;
        self.sources = sources.to_vec();
        self.native = native.to_vec();
        let update = if changed || projected.changed {
            self.revision += 1;
            Some(UnifiedComponentUpdate {
                revision: self.revision,
                sources: states,
                native: projected.states,
            })
        } else {
            None
        };
        Ok(UnifiedComponentProjection {
            update,
            frame: UnifiedComponentFrames {
                revision: self.revision,
                native: Some(projected.frames),
                sources: sources
                    .iter()
                    .map(|source| UnifiedComponentFrame {
                        owner: source.identity_block.owner.clone(),
                        generation: source.identity_block.generation,
                        abi: source.identity_block.abi,
                        viewer: source.viewer.clone(),
                        client_number: source
                            .context
                            .client_number
                            .unwrap_or_else(|| i64::from(source.context.snapshot.player_state.client_number)),
                        game_state_revision: source.context.game_state_revision,
                        snapshot: source.context.snapshot.clone(),
                        weapon_presented: source.context.weapon_presented,
                        bindings: source.bindings.clone(),
                        scene: source.context.scene.as_ref().map(|scene| UnifiedComponentFrameScene {
                            revision: scene.revision,
                            snapshot: scene.snapshot.clone(),
                        }),
                    })
                    .collect(),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;
    use qa_guest::qvm::client_state::GameStateRecord;
    use qa_guest::qvm::game_data::AbiProfile;
    use qa_guest::qvm::mod_presentation::QvmSceneCommand;
    use qa_guest::qvm::player_record::{qvm_player_state_bytes, read_source_qvm_player_state};

    use super::super::unified_components::{
        UnifiedComponentAbi as Abi, UnifiedComponentContext, UnifiedComponentRuntime, UnifiedComponentSceneData,
        UnifiedComponentSnapshot, UnifiedSceneSnapshotData,
    };
    use crate::persistence::mods::{ModIdentity, ModSelection};
    use qa_content::contract::PresentationOwner;
    use qa_core::identity::ProviderId;
    use qa_world::save::shared::ProviderRef;

    fn owner() -> PresentationOwner {
        PresentationOwner {
            provider: ProviderId::new("test", "mod"),
            generation: 3,
        }
    }

    fn identity() -> ModIdentity {
        ModIdentity {
            selection: ModSelection {
                product: "test".to_string(),
                id: "mod".to_string(),
            },
            source: ProviderRef {
                provider: "test:source".to_string(),
                content: "q3:classic:base:1".to_string(),
            },
            declaration_digest: "sha256:".to_string() + &"ab".repeat(32),
            modules: Vec::new(),
            providers: Vec::new(),
        }
    }

    fn snapshot() -> UnifiedComponentSnapshot {
        let bytes = vec![0u8; qvm_player_state_bytes(AbiProfile::Modern)];
        UnifiedComponentSnapshot {
            server_time: 100,
            player_state: read_source_qvm_player_state(&bytes, AbiProfile::Modern).unwrap(),
        }
    }

    fn publication() -> UnifiedComponentPublication {
        UnifiedComponentPublication {
            identity_block: super::super::unified_components::UnifiedComponentIdentity {
                owner: owner(),
                identity: identity(),
                generation: 1,
                abi: Abi::Modern,
                runtime: UnifiedComponentRuntime::Scene,
            },
            viewer: IdentityOwner::create("test").unwrap().actor(1, 0),
            context: UnifiedComponentContext {
                client_number: None,
                game_state_revision: 4,
                game_state: GameStateRecord {
                    string_offsets: Vec::new(),
                    string_data: Vec::new(),
                    data_count: 0,
                },
                snapshot: snapshot(),
                weapon_presented: false,
                scene: None,
            },
            bindings: Vec::new(),
        }
    }

    #[test]
    fn empty_project_is_stable() {
        let mut publisher = UnifiedComponentPublisher::new();
        let first = publisher.project(&[], &[]).unwrap();
        assert!(first.update.is_none());
        assert_eq!(first.frame.revision, 0);
        let second = publisher.project(&[], &[]).unwrap();
        assert!(second.update.is_none());
    }

    #[test]
    fn added_source_emits_update_then_stabilizes() {
        let mut publisher = UnifiedComponentPublisher::new();
        let source = publication();
        let first = publisher.project(std::slice::from_ref(&source), &[]).unwrap();
        let update = first.update.unwrap();
        assert_eq!(update.revision, 1);
        assert_eq!(update.sources.len(), 1);
        assert!(update.sources[0].game_state.is_some());
        assert_eq!(first.frame.revision, 1);
        assert_eq!(first.frame.sources.len(), 1);
        let second = publisher.project(&[source], &[]).unwrap();
        assert!(second.update.is_none());
        assert_eq!(second.frame.revision, 1);
    }

    #[test]
    fn activation_change_is_rejected() {
        let mut publisher = UnifiedComponentPublisher::new();
        let source = publication();
        publisher.project(std::slice::from_ref(&source), &[]).unwrap();
        let mut changed = source.clone();
        changed.viewer = IdentityOwner::create("test").unwrap().actor(2, 0);
        assert!(publisher.project(&[changed], &[]).is_err());
        let mut changed = source;
        changed.identity_block.abi = Abi::Legacy116n;
        assert!(publisher.project(&[changed], &[]).is_err());
    }

    #[test]
    fn command_gap_is_rejected() {
        let mut publisher = UnifiedComponentPublisher::new();
        let mut source = publication();
        let bytes = vec![0u8; qvm_player_state_bytes(AbiProfile::Modern)];
        publisher.project(std::slice::from_ref(&source), &[]).unwrap();
        source.context.scene = Some(UnifiedComponentSceneData {
            revision: 1,
            snapshot: UnifiedSceneSnapshotData {
                server_time: 100,
                flags: 0,
                area_mask: Vec::new(),
                player_state: read_source_qvm_player_state(&bytes, AbiProfile::Modern).unwrap(),
                entities: Vec::new(),
                server_command_sequence: 5,
            },
            commands: Vec::new(),
        });
        assert!(publisher.project(&[source], &[]).is_err());
    }

    #[test]
    fn commands_advance_the_window() {
        let mut publisher = UnifiedComponentPublisher::new();
        let mut source = publication();
        let bytes = vec![0u8; qvm_player_state_bytes(AbiProfile::Modern)];
        let scene = || UnifiedComponentSceneData {
            revision: 1,
            snapshot: UnifiedSceneSnapshotData {
                server_time: 100,
                flags: 0,
                area_mask: Vec::new(),
                player_state: read_source_qvm_player_state(&bytes, AbiProfile::Modern).unwrap(),
                entities: Vec::new(),
                server_command_sequence: 2,
            },
            commands: vec![
                QvmSceneCommand {
                    sequence: 1,
                    arguments: vec!["a".to_string()],
                },
                QvmSceneCommand {
                    sequence: 2,
                    arguments: vec!["b".to_string()],
                },
            ],
        };
        source.context.scene = Some(scene());
        let first = publisher.project(std::slice::from_ref(&source), &[]).unwrap();
        assert_eq!(first.update.as_ref().unwrap().sources[0].command_base, 2);
        assert!(first.update.as_ref().unwrap().sources[0].commands.is_empty());
        let second = publisher.project(&[source], &[]).unwrap();
        assert!(second.update.is_none());
    }
}
