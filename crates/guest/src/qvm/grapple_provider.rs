//! Grapple provider: launch, tether, pull, and checkpoints.
//!
//! Provenance: `src/compat/qvm/grapple-provider.ts`.
//!
//! Profiles reuse [`super::grapple_profile`]. The bridge mirrors
//! `QvmGrappleBridge`; saved actors mirror `SavedActorId`.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use super::game_data::{QvmArtifact, QvmCheckpoint, QvmGameData, QvmMemoryWindow, QvmModule};
use super::grapple_profile::{qvm_grapple_profile_declaration, read_qvm_grapple_profile, QvmGrappleProfile};
use crate::error::GuestError;

/// Grapple projection.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmGrappleProjection {
    /// Hook address.
    pub hook: usize,
    /// Hook origin.
    pub origin: Vec3,
    /// Hook velocity.
    pub velocity: Vec3,
    /// Target actor, if any.
    pub target: Option<ActorId>,
    /// Mover actor, if any.
    pub mover: Option<ActorId>,
    /// Whether pulling.
    pub pulling: bool,
    /// Tether point.
    pub point: Vec3,
    /// Owner velocity.
    pub owner_velocity: Vec3,
}

/// Reserved scratch span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmGrappleScratch {
    /// Word address.
    pub word: usize,
    /// Byte length.
    pub byte_length: usize,
}

/// Saved actor id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QvmSavedActor {
    /// Slot.
    pub slot: u32,
    /// Generation.
    pub generation: u32,
}

/// Grapple host bridge.
pub trait QvmGrappleBridge {
    /// Refresh mirrored actors and collision inputs.
    fn synchronize(&self);
    /// Source entity address for an actor.
    fn entity(&self, actor: &ActorId) -> Option<usize>;
    /// Canonical actor for a pointer word.
    fn actor(&self, pointer: i32) -> Option<ActorId>;
    /// Restore a saved actor.
    fn restore_actor(&self, saved: &QvmSavedActor) -> ActorId;
    /// Publish (or retire, for `None`) a shared tether.
    fn publish(&self, owner: &ActorId, projection: Option<&QvmGrappleProjection>);
    /// Publish owner velocity.
    fn velocity(&self, owner: &ActorId, velocity: &Vec3);
    /// Reserved scratch span.
    fn scratch(&self) -> QvmGrappleScratch;
}

/// Grapple checkpoint.
#[derive(Debug, Clone)]
pub struct QvmGrappleCheckpoint {
    /// Version (always 1).
    pub version: u32,
    /// Profile declaration.
    pub profile: String,
    /// Module checkpoint.
    pub module: QvmCheckpoint,
    /// Saved owners.
    pub owners: Vec<QvmSavedActor>,
}

/// Source game handles.
#[derive(Clone)]
pub struct QvmGrappleGame {
    /// Source module.
    pub module: QvmModule,
    /// Located game data.
    pub data: QvmGameData,
}

/// Grapple provider over one source VM.
pub struct QvmGrappleProvider {
    /// Source game handles.
    game: QvmGrappleGame,
    /// Validated profile.
    pub profile: QvmGrappleProfile,
    /// Host bridge.
    bridge: Rc<dyn QvmGrappleBridge>,
    /// Firing owners in admission order.
    owners: RefCell<Vec<ActorId>>,
    /// Declaration string.
    declaration: String,
}

impl QvmGrappleProvider {
    /// Validate the profile and reserve scratch.
    pub fn new(
        game: QvmGrappleGame,
        artifact: &QvmArtifact,
        profile: QvmGrappleProfile,
        bridge: Rc<dyn QvmGrappleBridge>,
    ) -> Result<Self, GuestError> {
        let declaration = qvm_grapple_profile_declaration(&profile);
        let profile = read_qvm_grapple_profile(&super::game_data::ProfileReader::new(&declaration), artifact)?;
        let declaration = format!("{profile:?}");
        let actual = game.module.module_id();
        if actual.id != profile.module.id
            || actual.digest != profile.module.digest
            || actual.artifact_path != profile.module.artifact_path
            || actual.revision != profile.module.revision
            || game.module.abi_profile() != profile.abi_profile
        {
            return Err(GuestError::invalid("Grapple module differs from its declared artifact"));
        }
        if game.data.entity_stride_bytes() != profile.entity_stride
            || game.data.client_stride_bytes() != profile.client_stride
        {
            return Err(GuestError::invalid(
                "Grapple profile differs from located entity or client records",
            ));
        }
        let scratch = bridge.scratch();
        let source_end = artifact.image.data_length + artifact.image.literal_length + artifact.image.bss_length;
        if scratch.word % 4 != 0
            || scratch.word < source_end
            || scratch.byte_length < 16.max(profile.movement.byte_length)
        {
            return Err(GuestError::invalid(
                "Grapple scratch must be a reserved span beyond declared source data",
            ));
        }
        QvmMemoryWindow::new(game.module.memory(), scratch.word, scratch.byte_length)?;
        Ok(Self {
            game,
            profile,
            bridge,
            owners: RefCell::new(Vec::new()),
            declaration,
        })
    }

    /// Admit an owner.
    fn admit(&self, actor: &ActorId) {
        let mut owners = self.owners.borrow_mut();
        if !owners.contains(actor) {
            owners.push(actor.clone());
        }
    }

    /// Retire an owner.
    fn retire(&self, actor: &ActorId) {
        self.owners.borrow_mut().retain(|owner| owner != actor);
    }

    /// Read a word.
    fn word(&self, pointer: usize, offset: usize) -> Result<i32, GuestError> {
        self.game.module.memory().read_i32(pointer + offset)
    }

    /// Write a word.
    fn write(&self, pointer: usize, value: i32) -> Result<(), GuestError> {
        self.game.module.memory().write_i32(pointer, value)
    }

    /// Read a vector.
    fn vector(&self, pointer: usize) -> Result<Vec3, GuestError> {
        self.game.module.memory().read_vec3(pointer)
    }

    /// Write a vector.
    fn write_vector(&self, pointer: usize, value: &Vec3) -> Result<(), GuestError> {
        self.game.module.memory().write_vec3(pointer, value)
    }

    /// Resolve an owner's live entity and player record.
    fn owner(&self, actor: &ActorId) -> Result<(usize, usize), GuestError> {
        let entity = self.bridge.entity(actor);
        let Some(entity) = entity else {
            return Err(GuestError::invalid("Grapple owner has no live source entity"));
        };
        if self.word(entity, self.profile.fields.inuse)? == 0 {
            return Err(GuestError::invalid("Grapple owner has no live source entity"));
        }
        self.game.data.number_from_pointer(entity as i32)?;
        let client = self.word(entity, self.profile.fields.client)?;
        if client == 0 {
            return Err(GuestError::invalid("Grapple owner has no source player record"));
        }
        let client =
            usize::try_from(client).map_err(|_| GuestError::invalid("Grapple owner has no source player record"))?;
        QvmMemoryWindow::new(self.game.module.memory(), client, self.profile.client_stride)?;
        Ok((entity, client))
    }

    /// Resolve an owner's live hook, if any.
    fn hook(&self, actor: &ActorId) -> Result<Option<usize>, GuestError> {
        let (entity, client) = self.owner(actor)?;
        let hook = self.word(client, self.profile.fields.hook)?;
        if hook == 0 {
            return Ok(None);
        }
        self.game.data.number_from_pointer(hook)?;
        let hook = usize::try_from(hook)
            .map_err(|_| GuestError::invalid("Source grapple points at an inactive or differently owned hook"))?;
        if self.word(hook, self.profile.fields.inuse)? == 0
            || self.word(hook, self.profile.fields.parent)? as usize != entity
        {
            return Err(GuestError::invalid(
                "Source grapple points at an inactive or differently owned hook",
            ));
        }
        Ok(Some(hook))
    }

    /// Project an owner's live tether, if any.
    fn projection(&self, actor: &ActorId) -> Result<Option<QvmGrappleProjection>, GuestError> {
        let hook = self.hook(actor)?;
        let Some(hook) = hook else {
            return Ok(None);
        };
        let (_, client) = self.owner(actor)?;
        let entity = self.game.data.entity_from_pointer(hook as i32)?;
        Ok(Some(QvmGrappleProjection {
            hook,
            origin: entity.r.current_origin.clone(),
            velocity: entity.s.pos.delta.clone(),
            target: self.bridge.actor(self.word(hook, self.profile.fields.target)?),
            mover: match self.profile.fields.mover {
                Some(mover) => self.bridge.actor(self.word(hook, mover)?),
                None => None,
            },
            pulling: self.word(client, 12)? & self.profile.pulling_flag != 0,
            point: self.vector(client + 92)?,
            owner_velocity: self.vector(client + 32)?,
        }))
    }

    /// Publish an owner's tether and velocity.
    fn publish(&self, actor: &ActorId) -> Result<(), GuestError> {
        let projection = self.projection(actor)?;
        self.bridge.publish(actor, projection.as_ref());
        let (_, client) = self.owner(actor)?;
        self.bridge.velocity(actor, &self.vector(client + 32)?);
        Ok(())
    }

    /// Begin a source frame.
    pub fn begin_frame(&self, time_ms: i32, frame: i32) -> Result<(), GuestError> {
        if time_ms < self.word(self.profile.globals.time, 0)? || frame < self.word(self.profile.globals.frame, 0)? {
            return Err(GuestError::invalid("Invalid grapple source frame"));
        }
        self.write(self.profile.globals.time, time_ms)?;
        self.write(self.profile.globals.frame, frame)?;
        self.bridge.synchronize();
        Ok(())
    }

    /// Fire an owner's hook.
    pub fn fire(&self, actor: &ActorId) -> Result<(), GuestError> {
        self.bridge.synchronize();
        let (entity, _) = self.owner(actor)?;
        self.admit(actor);
        if self.hook(actor)?.is_none() {
            let mut words = vec![entity as i32];
            words.extend(self.profile.fire_arguments.iter().copied());
            self.game.module.call(&words, self.profile.callbacks.fire)?;
        }
        self.publish(actor)
    }

    /// Release an owner's hook.
    pub fn release(&self, actor: &ActorId, force: bool) -> Result<(), GuestError> {
        if !self.owners.borrow().contains(actor) {
            return Ok(());
        }
        if let Some(hook) = self.hook(actor)? {
            self.game.module.call(
                &[hook as i32],
                if force {
                    self.profile.callbacks.force_release
                } else {
                    self.profile.callbacks.release
                },
            )?;
        }
        self.publish(actor)?;
        if self.hook(actor)?.is_none() {
            self.retire(actor);
        }
        Ok(())
    }

    /// Step all owners.
    pub fn step(&self) -> Result<(), GuestError> {
        self.bridge.synchronize();
        let owners: Vec<ActorId> = self.owners.borrow().clone();
        for actor in owners {
            if self.word(self.owner(&actor)?.0, self.profile.fields.health)? <= 0 {
                self.release(&actor, true)?;
                continue;
            }
            if let Some(hook) = self.hook(&actor)? {
                if self.game.data.entity_from_pointer(hook as i32)?.s.e_type == 3 {
                    self.game.module.call(&[hook as i32], self.profile.callbacks.missile)?;
                } else {
                    if let Some(follow) = self.profile.callbacks.follow {
                        self.game.module.call(&[hook as i32], follow)?;
                    }
                    if self.hook(&actor)?.is_some() {
                        self.game.module.call(&[hook as i32], self.profile.callbacks.think)?;
                    }
                }
            }
            self.publish(&actor)?;
            if self.hook(&actor)?.is_none() {
                self.retire(&actor);
            }
        }
        Ok(())
    }

    /// Pull an owner's tether; returns the owner velocity.
    pub fn pull(&self, actor: &ActorId, forward: &Vec3) -> Result<Option<Vec3>, GuestError> {
        if !self.owners.borrow().contains(actor) {
            return Ok(None);
        }
        let projection = self.projection(actor)?;
        let Some(projection) = projection else {
            return Ok(None);
        };
        if !projection.pulling {
            return Ok(None);
        }
        let globals = &self.profile.globals;
        let memory = self.game.module.memory();
        let scratch = self.bridge.scratch().word;
        let movement = self.word(globals.movement, 0)?;
        let saved_forward = memory.read_bytes(globals.forward, 12)?;
        let ground_plane = self.word(globals.ground_plane, 0)?;
        let saved_scratch = memory.read_bytes(scratch, self.profile.movement.byte_length)?;
        let (_, client) = self.owner(actor)?;
        memory.fill(scratch, self.profile.movement.byte_length, 0)?;
        for word in &self.profile.movement.words {
            self.write(scratch + word.offset, word.value)?;
        }
        self.write(scratch, client as i32)?;
        self.write(globals.movement, scratch as i32)?;
        self.write_vector(globals.forward, forward)?;
        let outcome = self
            .game
            .module
            .call(&[], self.profile.callbacks.pull)
            .and_then(|_| self.publish(actor))
            .and_then(|_| self.vector(client + 32));
        self.write(globals.movement, movement)?;
        memory.write_bytes(globals.forward, &saved_forward)?;
        self.write(globals.ground_plane, ground_plane)?;
        memory.write_bytes(scratch, &saved_scratch)?;
        outcome.map(Some)
    }

    /// Notify hooks that a mover moved.
    pub fn mover_moved(&self, actor: &ActorId, translation: &Vec3) -> Result<(), GuestError> {
        let Some(move_hooks) = self.profile.callbacks.move_mover_hooks else {
            return Ok(());
        };
        let mover = self.bridge.entity(actor);
        let Some(mover) = mover else {
            return Err(GuestError::invalid("Grapple mover has no source entity"));
        };
        let memory = self.game.module.memory();
        let scratch = self.bridge.scratch().word;
        let saved = memory.read_bytes(scratch, 12)?;
        self.write_vector(scratch, translation)?;
        let outcome = self.game.module.call(&[mover as i32, scratch as i32], move_hooks);
        let owners: Vec<ActorId> = self.owners.borrow().clone();
        let mut published = outcome;
        if published.is_ok() {
            for owner in owners {
                if let Err(error) = self.publish(&owner) {
                    published = Err(error);
                    break;
                }
            }
        }
        memory.write_bytes(scratch, &saved)?;
        published?;
        Ok(())
    }

    /// Release owners touching a released actor.
    pub fn actor_released(&self, actor: &ActorId) -> Result<(), GuestError> {
        let owners: Vec<ActorId> = self.owners.borrow().clone();
        for owner in owners {
            let state = self.projection(&owner)?;
            if owner == *actor
                || state.as_ref().and_then(|state| state.target.as_ref()) == Some(actor)
                || state.as_ref().and_then(|state| state.mover.as_ref()) == Some(actor)
            {
                self.release(&owner, true)?;
            }
        }
        Ok(())
    }

    /// Capture a checkpoint.
    pub fn capture(&self) -> Result<QvmGrappleCheckpoint, GuestError> {
        Ok(QvmGrappleCheckpoint {
            version: 1,
            profile: self.declaration.clone(),
            module: self.game.module.checkpoint()?,
            owners: self
                .owners
                .borrow()
                .iter()
                .map(|actor| QvmSavedActor {
                    slot: actor.slot(),
                    generation: actor.generation(),
                })
                .collect(),
        })
    }

    /// Restore a checkpoint.
    pub fn restore(&self, checkpoint: &QvmGrappleCheckpoint) -> Result<(), GuestError> {
        if checkpoint.version != 1 || checkpoint.profile != self.declaration {
            return Err(GuestError::invalid(
                "Saved grapple profile differs from its source declaration",
            ));
        }
        let owners: Vec<ActorId> = checkpoint
            .owners
            .iter()
            .map(|saved| self.bridge.restore_actor(saved))
            .collect();
        if owners.iter().collect::<HashSet<_>>().len() != owners.len() {
            return Err(GuestError::invalid("Saved grapple has duplicate owners"));
        }
        self.game.module.restore(&checkpoint.module)?;
        if self.game.data.entity_stride_bytes() != self.profile.entity_stride
            || self.game.data.client_stride_bytes() != self.profile.client_stride
        {
            return Err(GuestError::invalid(
                "Grapple profile differs from located entity or client records",
            ));
        }
        for owner in self.owners.borrow().iter() {
            self.bridge.publish(owner, None);
        }
        self.owners.borrow_mut().clear();
        for owner in owners {
            self.admit(&owner);
            self.publish(&owner)?;
        }
        Ok(())
    }

    /// Release all owners.
    pub fn close(&self) -> Result<(), GuestError> {
        let owners: Vec<ActorId> = self.owners.borrow().clone();
        for owner in owners {
            self.release(&owner, true)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    use qa_core::identity::IdentityOwner;
    use qa_core::math::vec3;

    use super::super::game_data::{
        AbiProfile, ModuleIdentity, ProfileReader, ProfileValue, QvmArtifact, QvmImage, QvmInstruction, QvmOpcode,
        QvmRole, QvmSharedMemory,
    };
    use super::*;

    const ENTITIES: usize = 4096;
    const CLIENTS: usize = 8192;
    const STRIDE: usize = 1024;
    const SCRATCH: usize = 32768;

    struct FixtureBridge {
        entities: HashMap<ActorId, usize>,
        actors: HashMap<i32, ActorId>,
        published: RefCell<Vec<(ActorId, Option<QvmGrappleProjection>)>>,
        velocities: RefCell<Vec<(ActorId, Vec3)>>,
    }

    impl QvmGrappleBridge for FixtureBridge {
        fn synchronize(&self) {}

        fn entity(&self, actor: &ActorId) -> Option<usize> {
            self.entities.get(actor).copied()
        }

        fn actor(&self, pointer: i32) -> Option<ActorId> {
            self.actors.get(&pointer).cloned()
        }

        fn restore_actor(&self, saved: &QvmSavedActor) -> ActorId {
            self.entities
                .keys()
                .find(|actor| actor.slot() == saved.slot && actor.generation() == saved.generation)
                .cloned()
                .unwrap()
        }

        fn publish(&self, owner: &ActorId, projection: Option<&QvmGrappleProjection>) {
            self.published.borrow_mut().push((owner.clone(), projection.cloned()));
        }

        fn velocity(&self, owner: &ActorId, velocity: &Vec3) {
            self.velocities.borrow_mut().push((owner.clone(), velocity.clone()));
        }

        fn scratch(&self) -> QvmGrappleScratch {
            QvmGrappleScratch {
                word: SCRATCH,
                byte_length: 64,
            }
        }
    }

    struct Fixture {
        provider: QvmGrappleProvider,
        module: QvmModule,
        bridge: Rc<FixtureBridge>,
        actor: ActorId,
        hook_actor: ActorId,
    }

    impl Fixture {
        fn memory(&self) -> QvmSharedMemory {
            self.module.memory()
        }

        fn set_hook(&self, hook: i32) {
            self.memory().write_i32(CLIENTS + 468, hook).unwrap();
        }
    }

    fn profile_value() -> ProfileValue {
        let callbacks = [
            "allocate",
            "free",
            "fire",
            "release",
            "forceRelease",
            "missile",
            "think",
            "pull",
            "damage",
            "sameTeam",
            "playerMove",
        ]
        .into_iter()
        .enumerate()
        .map(|(index, name)| (name.to_string(), ProfileValue::Int(1 + index as i64)))
        .chain([
            ("follow".to_string(), ProfileValue::Null),
            ("moveMoverHooks".to_string(), ProfileValue::Null),
        ])
        .collect::<Vec<_>>();
        ProfileValue::record(vec![
            ("version", ProfileValue::Int(1)),
            ("artifactDigest", ProfileValue::Str("test".to_string())),
            ("artifactPath", ProfileValue::Str("test".to_string())),
            ("abiProfile", ProfileValue::Str("q3-modern".to_string())),
            ("id", ProfileValue::Str("grapple".to_string())),
            ("title", ProfileValue::Str("Grapple".to_string())),
            ("entityStride", ProfileValue::Int(STRIDE as i64)),
            ("clientStride", ProfileValue::Int(STRIDE as i64)),
            (
                "fields",
                ProfileValue::record(vec![
                    ("inuse", ProfileValue::Int(516)),
                    ("client", ProfileValue::Int(520)),
                    ("parent", ProfileValue::Int(524)),
                    ("target", ProfileValue::Int(528)),
                    ("mover", ProfileValue::Null),
                    ("hook", ProfileValue::Int(468)),
                    ("health", ProfileValue::Int(532)),
                    ("takedamage", ProfileValue::Int(536)),
                    ("eventTime", ProfileValue::Int(540)),
                    ("freeAfterEvent", ProfileValue::Int(544)),
                ]),
            ),
            (
                "globals",
                ProfileValue::record(vec![
                    ("time", ProfileValue::Int(64)),
                    ("frame", ProfileValue::Int(68)),
                    ("movement", ProfileValue::Int(72)),
                    ("forward", ProfileValue::Int(76)),
                    ("groundPlane", ProfileValue::Int(88)),
                ]),
            ),
            ("callbacks", ProfileValue::Record(callbacks)),
            ("pullingFlag", ProfileValue::Int(64)),
            ("fireArguments", ProfileValue::Array(vec![ProfileValue::Int(7)])),
            (
                "movement",
                ProfileValue::record(vec![
                    ("byteLength", ProfileValue::Int(16)),
                    (
                        "words",
                        ProfileValue::Array(vec![ProfileValue::record(vec![
                            ("offset", ProfileValue::Int(4)),
                            ("value", ProfileValue::Int(9)),
                        ])]),
                    ),
                ]),
            ),
            ("initialCvars", ProfileValue::record(Vec::new())),
            ("eventLifetimeMilliseconds", ProfileValue::Int(500)),
            ("grappleDamageMethod", ProfileValue::Int(3)),
            (
                "presentation",
                ProfileValue::record(vec![
                    ("projectileModel", ProfileValue::Str("hook".to_string())),
                    ("viewModel", ProfileValue::Str("hands".to_string())),
                    ("weaponIndex", ProfileValue::Int(10)),
                    (
                        "viewAnchor",
                        ProfileValue::record(vec![
                            ("path", ProfileValue::Str("view".to_string())),
                            ("tag", ProfileValue::Str("tag".to_string())),
                            (
                                "offset",
                                ProfileValue::record(vec![
                                    ("x", ProfileValue::Float(0.0)),
                                    ("y", ProfileValue::Float(0.0)),
                                    ("z", ProfileValue::Float(0.0)),
                                ]),
                            ),
                            (
                                "fovOffset",
                                ProfileValue::record(vec![
                                    ("above", ProfileValue::Int(4)),
                                    ("scale", ProfileValue::Float(1.0)),
                                ]),
                            ),
                        ]),
                    ),
                    ("viewAttachments", ProfileValue::Array(Vec::new())),
                    (
                        "cable",
                        ProfileValue::record(vec![
                            ("kind", ProfileValue::Str("shader".to_string())),
                            ("path", ProfileValue::Str("cable".to_string())),
                            ("width", ProfileValue::Int(2)),
                        ]),
                    ),
                    ("fireSound", ProfileValue::Null),
                    ("attachSound", ProfileValue::Null),
                    ("releaseSound", ProfileValue::Null),
                    ("pullSound", ProfileValue::Null),
                    ("hangSound", ProfileValue::Null),
                ]),
            ),
        ])
    }

    fn fixture() -> Fixture {
        let owner = IdentityOwner::create("grapple-test").unwrap();
        let actor = owner.actor(0, 1);
        let hook_actor = owner.actor(1, 1);
        let mut image = QvmImage::default();
        image.instructions = (0..32)
            .map(|index| QvmInstruction::word(QvmOpcode::OpEnter, 0, index * 8))
            .collect();
        image.data_length = 4096;
        image.allocated_data_length = 65536;
        let artifact = QvmArtifact {
            module: ModuleIdentity {
                id: "test:qagame".to_string(),
                artifact_path: "test".to_string(),
                digest: "test".to_string(),
                revision: "1".to_string(),
            },
            role: QvmRole::Qagame,
            abi_profile: None,
            image,
        };
        let module = QvmModule::new(artifact.clone(), None, None).unwrap();
        let data = QvmGameData::new(module.memory(), AbiProfile::Modern);
        data.locate(ENTITIES as i32, 4, STRIDE, CLIENTS as i32, STRIDE).unwrap();
        module.memory().write_i32(ENTITIES + 516, 1).unwrap();
        module.memory().write_i32(ENTITIES + 520, CLIENTS as i32).unwrap();
        module.memory().write_i32(ENTITIES + 532, 100).unwrap();
        module.memory().write_i32(ENTITIES + STRIDE + 516, 1).unwrap();
        module
            .memory()
            .write_i32(ENTITIES + STRIDE + 524, ENTITIES as i32)
            .unwrap();
        let profile = read_qvm_grapple_profile(&ProfileReader::new(&profile_value()), &artifact).unwrap();
        let bridge = Rc::new(FixtureBridge {
            entities: [(actor.clone(), ENTITIES), (hook_actor.clone(), ENTITIES + STRIDE)]
                .into_iter()
                .collect(),
            actors: [
                (ENTITIES as i32, actor.clone()),
                ((ENTITIES + STRIDE) as i32, hook_actor.clone()),
            ]
            .into_iter()
            .collect(),
            published: RefCell::new(Vec::new()),
            velocities: RefCell::new(Vec::new()),
        });
        let provider = QvmGrappleProvider::new(
            QvmGrappleGame {
                module: module.clone(),
                data,
            },
            &artifact,
            profile,
            Rc::clone(&bridge) as Rc<dyn QvmGrappleBridge>,
        )
        .unwrap();
        Fixture {
            provider,
            module,
            bridge,
            actor,
            hook_actor,
        }
    }

    #[test]
    fn fire_and_release_cycle() {
        let fixture = fixture();
        fixture.provider.begin_frame(100, 7).unwrap();
        assert!(fixture.provider.begin_frame(50, 7).is_err());
        fixture.provider.fire(&fixture.actor).unwrap();
        let calls = fixture.module.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].entry, 3);
        assert_eq!(calls[0].words, vec![ENTITIES as i32, 7]);
        assert_eq!(fixture.bridge.published.borrow().len(), 1);
        assert!(fixture.bridge.published.borrow()[0].1.is_none());

        fixture.set_hook((ENTITIES + STRIDE) as i32);
        fixture.provider.fire(&fixture.actor).unwrap();
        assert_eq!(fixture.module.calls().len(), 1);
        let projection = fixture.bridge.published.borrow().last().unwrap().1.clone().unwrap();
        assert_eq!(projection.hook, ENTITIES + STRIDE);
        assert!(!projection.pulling);

        fixture.provider.release(&fixture.actor, false).unwrap();
        let calls = fixture.module.calls();
        assert_eq!(calls.last().unwrap().entry, 4);
        fixture.set_hook(0);
        fixture.provider.release(&fixture.actor, true).unwrap();
        assert_eq!(fixture.module.calls().len(), 2);
        fixture.provider.release(&fixture.hook_actor, false).unwrap();
        assert_eq!(fixture.module.calls().len(), 2);
    }

    #[test]
    fn step_drives_missiles_and_force_releases_dead() {
        let fixture = fixture();
        fixture.provider.fire(&fixture.actor).unwrap();
        fixture.set_hook((ENTITIES + STRIDE) as i32);
        fixture.memory().write_i32(ENTITIES + STRIDE + 4, 3).unwrap();
        fixture.provider.step().unwrap();
        let calls = fixture.module.calls();
        assert_eq!(calls.last().unwrap().entry, 6);

        fixture.memory().write_i32(ENTITIES + STRIDE + 4, 1).unwrap();
        fixture.provider.step().unwrap();
        let calls = fixture.module.calls();
        assert_eq!(calls.last().unwrap().entry, 7);

        fixture.memory().write_i32(ENTITIES + 532, 0).unwrap();
        fixture.provider.step().unwrap();
        let calls = fixture.module.calls();
        assert_eq!(calls.last().unwrap().entry, 5);
    }

    #[test]
    fn pull_projects_and_restores() {
        let fixture = fixture();
        assert_eq!(
            fixture.provider.pull(&fixture.actor, &vec3(0.0, 0.0, 1.0)).unwrap(),
            None
        );
        fixture.provider.fire(&fixture.actor).unwrap();
        fixture.set_hook((ENTITIES + STRIDE) as i32);
        assert_eq!(
            fixture.provider.pull(&fixture.actor, &vec3(0.0, 0.0, 1.0)).unwrap(),
            None
        );
        fixture.memory().write_i32(CLIENTS + 12, 64).unwrap();
        fixture.memory().write_vec3(CLIENTS + 32, &vec3(4.0, 5.0, 6.0)).unwrap();
        fixture.memory().write_i32(72, 1234).unwrap();
        let velocity = fixture
            .provider
            .pull(&fixture.actor, &vec3(0.0, 0.0, 1.0))
            .unwrap()
            .unwrap();
        assert_eq!(velocity, vec3(4.0, 5.0, 6.0));
        assert_eq!(fixture.memory().read_i32(72).unwrap(), 1234);
        assert_eq!(fixture.memory().read_i32(SCRATCH).unwrap(), 0);
        let calls = fixture.module.calls();
        assert!(calls.iter().any(|call| call.entry == 8));
    }

    #[test]
    fn mover_moved_and_actor_released() {
        let fixture = fixture();
        fixture
            .provider
            .mover_moved(&fixture.actor, &vec3(1.0, 0.0, 0.0))
            .unwrap();
        assert!(fixture.module.calls().is_empty());

        fixture.provider.fire(&fixture.actor).unwrap();
        fixture.set_hook((ENTITIES + STRIDE) as i32);
        fixture
            .memory()
            .write_i32(ENTITIES + STRIDE + 528, (ENTITIES + STRIDE) as i32)
            .unwrap();
        fixture.provider.actor_released(&fixture.hook_actor).unwrap();
        let calls = fixture.module.calls();
        assert_eq!(calls.last().unwrap().entry, 5);
    }

    #[test]
    fn checkpoints_round_trip() {
        let fixture = fixture();
        fixture.provider.fire(&fixture.actor).unwrap();
        let checkpoint = fixture.provider.capture().unwrap();
        assert_eq!(checkpoint.version, 1);
        assert_eq!(checkpoint.owners.len(), 1);
        fixture.set_hook((ENTITIES + STRIDE) as i32);
        fixture.provider.close().unwrap();
        assert_eq!(fixture.module.calls().len(), 2);
        fixture.set_hook(0);
        fixture.provider.restore(&checkpoint).unwrap();
        assert_eq!(fixture.bridge.published.borrow().len(), 4);

        let mut foreign = checkpoint.clone();
        foreign.version = 2;
        assert!(fixture.provider.restore(&foreign).is_err());
        let mut duplicated = checkpoint;
        duplicated.owners.push(duplicated.owners[0]);
        assert!(fixture.provider.restore(&duplicated).is_err());
    }
}
