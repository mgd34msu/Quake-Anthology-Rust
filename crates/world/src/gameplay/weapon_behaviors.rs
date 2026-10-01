//! Weapon behavior attachments: separately owned trajectory contributions
//! with save-safe artifact-pinned definitions.
//!
//! Donor provenance: `/home/buzzkill/Projects/quake-typescript/src/world/gameplay/weapon-behaviors.ts`
//! with contract shapes from `src/contracts/weapon-behavior.ts`.
//!
//! No replacement of launcher damage, visuals, or sound. Saved data can
//! never introduce an executable entrypoint: checkpoint readers reuse the
//! loaded callback identities.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use qa_core::identity::{ActorId, OwnedActor, ProviderId, SavedActorId};
use qa_core::math::Vec3;

use crate::body::BodyState;
use crate::combat::ItemId;
use crate::registry::ActorRegistry;
use crate::save::records::read_saved_actor;
use crate::save::shared::read_digest;
use crate::save::value::{namespaced, SaveJson, SaveReader};
use crate::WorldError;

/// Projectile role selecting a behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProjectileRole {
    /// Rocket.
    Rocket,
    /// Grenade.
    Grenade,
    /// Nail.
    Nail,
    /// Bolt.
    Bolt,
    /// Plasma.
    Plasma,
    /// Energy.
    Energy,
    /// Grapple.
    Grapple,
}

impl ProjectileRole {
    fn name(self) -> &'static str {
        match self {
            ProjectileRole::Rocket => "rocket",
            ProjectileRole::Grenade => "grenade",
            ProjectileRole::Nail => "nail",
            ProjectileRole::Bolt => "bolt",
            ProjectileRole::Plasma => "plasma",
            ProjectileRole::Energy => "energy",
            ProjectileRole::Grapple => "grapple",
        }
    }

    /// Parse a role name.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "rocket" => ProjectileRole::Rocket,
            "grenade" => ProjectileRole::Grenade,
            "nail" => ProjectileRole::Nail,
            "bolt" => ProjectileRole::Bolt,
            "plasma" => ProjectileRole::Plasma,
            "energy" => ProjectileRole::Energy,
            "grapple" => ProjectileRole::Grapple,
            _ => return None,
        })
    }
}

/// Module identity: selected artifact plus its retained digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeaponBehaviorModule {
    /// Module id (`namespace:name`).
    pub id: String,
    /// Artifact path.
    pub artifact_path: String,
    /// Artifact digest.
    pub digest: String,
    /// Module revision.
    pub revision: String,
}

/// Native call ABI of an artifact entrypoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeaponBehaviorAbi {
    /// Platform kind.
    pub kind: String,
    /// Image format.
    pub image: String,
    /// Calling convention.
    pub call: String,
    /// Pointer width in bytes.
    pub pointer_bytes: u32,
}

/// Artifact-pinned behavior callback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WeaponBehaviorCallback {
    /// QuakeC function.
    QuakeC {
        /// Module.
        module: WeaponBehaviorModule,
        /// Function index.
        function_index: u32,
    },
    /// QVM instruction.
    Qvm {
        /// Module.
        module: WeaponBehaviorModule,
        /// Instruction index.
        instruction_index: u32,
    },
    /// Native artifact entrypoint.
    NativeArtifact {
        /// Module.
        module: WeaponBehaviorModule,
        /// Image offset.
        image_offset: i128,
        /// Call ABI.
        abi: WeaponBehaviorAbi,
    },
}

impl WeaponBehaviorCallback {
    fn kind_name(&self) -> &'static str {
        match self {
            WeaponBehaviorCallback::QuakeC { .. } => "quakec",
            WeaponBehaviorCallback::Qvm { .. } => "qvm",
            WeaponBehaviorCallback::NativeArtifact { .. } => "native-artifact",
        }
    }
}

/// Behavior definition published by the source provider. The aspect is
/// always `trajectory`; the checkpoint reader validates the literal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeaponBehaviorDefinition {
    /// Definition id (`namespace:name`).
    pub id: String,
    /// Title.
    pub title: String,
    /// Module.
    pub module: WeaponBehaviorModule,
    /// Projectile role.
    pub role: ProjectileRole,
    /// Activation callback, if any.
    pub activate: Option<WeaponBehaviorCallback>,
    /// Fire callback.
    pub fire: WeaponBehaviorCallback,
}

/// Whether two definitions name the same artifact entrypoints.
#[must_use]
pub fn same_weapon_behavior(left: &WeaponBehaviorDefinition, right: &WeaponBehaviorDefinition) -> bool {
    fn same_module(a: &WeaponBehaviorModule, b: &WeaponBehaviorModule) -> bool {
        a.id == b.id && a.digest == b.digest && a.artifact_path == b.artifact_path && a.revision == b.revision
    }
    fn same_callback(a: &Option<WeaponBehaviorCallback>, b: &Option<WeaponBehaviorCallback>) -> bool {
        match (a, b) {
            (None, None) => true,
            (Some(a), Some(b)) => {
                fn module_of(callback: &WeaponBehaviorCallback) -> &WeaponBehaviorModule {
                    match callback {
                        WeaponBehaviorCallback::QuakeC { module, .. }
                        | WeaponBehaviorCallback::Qvm { module, .. }
                        | WeaponBehaviorCallback::NativeArtifact { module, .. } => module,
                    }
                }
                let module = module_of;
                if !same_module(module(a), module(b)) {
                    return false;
                }
                match (a, b) {
                    (
                        WeaponBehaviorCallback::QuakeC {
                            function_index: left, ..
                        },
                        WeaponBehaviorCallback::QuakeC {
                            function_index: right, ..
                        },
                    ) => left == right,
                    (
                        WeaponBehaviorCallback::Qvm {
                            instruction_index: left,
                            ..
                        },
                        WeaponBehaviorCallback::Qvm {
                            instruction_index: right,
                            ..
                        },
                    ) => left == right,
                    (
                        WeaponBehaviorCallback::NativeArtifact {
                            image_offset: left,
                            abi: left_abi,
                            ..
                        },
                        WeaponBehaviorCallback::NativeArtifact {
                            image_offset: right,
                            abi: right_abi,
                            ..
                        },
                    ) => {
                        left == right
                            && left_abi.kind == right_abi.kind
                            && left_abi.call == right_abi.call
                            && left_abi.image == right_abi.image
                            && left_abi.pointer_bytes == right_abi.pointer_bytes
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }
    left.id == right.id
        && left.role == right.role
        && same_module(&left.module, &right.module)
        && same_callback(&Some(left.fire.clone()), &Some(right.fire.clone()))
        && same_callback(&left.activate, &right.activate)
}

/// Behavior launch request.
#[derive(Debug, Clone, PartialEq)]
pub struct WeaponBehaviorLaunch {
    /// Projectile actor.
    pub projectile: OwnedActor,
    /// Shooter actor.
    pub shooter: ActorId,
    /// Fired weapon.
    pub weapon: ItemId,
    /// Projectile role.
    pub role: ProjectileRole,
    /// Launch time in seconds.
    pub time_seconds: f64,
    /// Launch body.
    pub body: BodyState,
}

/// Trajectory update: origin, velocity, angles.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WeaponTrajectoryUpdate {
    /// Origin.
    pub origin: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Angles.
    pub angles: Vec3,
}

/// Live behavior instance. The source keeps its private fields and
/// nextthink; the launcher keeps presentation and impact.
pub trait WeaponBehaviorInstance {
    /// Initial update.
    fn initial(&self) -> WeaponTrajectoryUpdate;
    /// Definition.
    fn definition(&self) -> &WeaponBehaviorDefinition;
    /// Step the trajectory.
    fn step(&mut self, body: &BodyState, time_seconds: f64) -> Option<WeaponTrajectoryUpdate>;
    /// Close the instance.
    fn close(&mut self);
}

/// Behavior source publishing attach/resume entrypoints.
pub trait WeaponBehaviorSource {
    /// Definition.
    fn definition(&self) -> &WeaponBehaviorDefinition;
    /// Attach a launch, or decline with `None`.
    fn attach(&self, launch: &WeaponBehaviorLaunch) -> Option<Box<dyn WeaponBehaviorInstance>>;
    /// Resume a checkpointed projectile.
    fn resume(&self, projectile: &ActorId) -> Result<Box<dyn WeaponBehaviorInstance>, WorldError>;
}

/// One checkpointed attachment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeaponBehaviorAttachment {
    /// Saved projectile reference.
    pub projectile: SavedActorId,
    /// Owning provider.
    pub owner: ProviderId,
    /// Definition (reused loaded identity).
    pub definition: WeaponBehaviorDefinition,
}

/// Attachment checkpoint (version 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeaponBehaviorAttachmentCheckpoint {
    /// Checkpoint version (always 1).
    pub version: i64,
    /// Attachments.
    pub attachments: Vec<WeaponBehaviorAttachment>,
}

/// One separately owned trajectory contribution.
pub struct WeaponBehaviorAttachments {
    registry: Rc<RefCell<ActorRegistry>>,
    sources: HashMap<String, Rc<dyn WeaponBehaviorSource>>,
    actors: HashMap<ActorId, AttachedBehavior>,
}

struct AttachedBehavior {
    owner: OwnedActor,
    instance: Box<dyn WeaponBehaviorInstance>,
}

impl WeaponBehaviorAttachments {
    /// Build attachments over a registry; actor releases detach behaviors.
    pub fn new(registry: Rc<RefCell<ActorRegistry>>) -> Rc<RefCell<Self>> {
        let attachments = Rc::new(RefCell::new(Self {
            registry: Rc::clone(&registry),
            sources: HashMap::new(),
            actors: HashMap::new(),
        }));
        let weak = Rc::downgrade(&attachments);
        registry.borrow_mut().on_release(Box::new(move |actor| {
            if let Some(attachments) = weak.upgrade() {
                attachments.borrow_mut().detach(actor.id());
            }
        }));
        attachments
    }

    /// Register a behavior source.
    pub fn register(&mut self, source: Rc<dyn WeaponBehaviorSource>) -> Result<(), WorldError> {
        if self.sources.contains_key(source.definition().id.as_str()) {
            return Err(WorldError::BadWeaponBehavior(format!(
                "Weapon behavior already registered: {}",
                source.definition().id
            )));
        }
        self.sources.insert(source.definition().id.clone(), source);
        Ok(())
    }

    /// Attach a behavior to a fired projectile.
    pub fn attach(&mut self, selection: &str, launch: &WeaponBehaviorLaunch) -> Result<bool, WorldError> {
        self.registry.borrow().assert_owned(&launch.projectile)?;
        if !self.registry.borrow().is_live(&launch.shooter) {
            return Err(WorldError::BadWeaponBehavior(
                "Weapon behavior shooter is retired".to_string(),
            ));
        }
        let Some(source) = self.sources.get(selection) else {
            return Err(WorldError::BadWeaponBehavior(format!(
                "Weapon behavior is unavailable: {selection}"
            )));
        };
        if source.definition().role != launch.role {
            return Err(WorldError::BadWeaponBehavior(
                "Weapon behavior does not match the fired projectile role".to_string(),
            ));
        }
        if self.actors.contains_key(launch.projectile.id()) {
            return Err(WorldError::BadWeaponBehavior(
                "Projectile already has a selected trajectory behavior".to_string(),
            ));
        }
        let Some(instance) = source.attach(launch) else {
            return Ok(false);
        };
        self.actors.insert(
            launch.projectile.id().clone(),
            AttachedBehavior {
                owner: launch.projectile.clone(),
                instance,
            },
        );
        Ok(true)
    }

    /// Attach and return the initial update, or `None` when declined.
    pub fn launch(
        &mut self,
        selection: &str,
        input: &WeaponBehaviorLaunch,
    ) -> Result<Option<WeaponTrajectoryUpdate>, WorldError> {
        if !self.attach(selection, input)? {
            return Ok(None);
        }
        let entry = self
            .actors
            .get(input.projectile.id())
            .ok_or_else(|| WorldError::BadWeaponBehavior("Attached projectile has no behavior instance".to_string()))?;
        Ok(Some(entry.instance.initial()))
    }

    /// Whether a behavior controls the projectile trajectory.
    #[must_use]
    pub fn controls_trajectory(&self, projectile: &ActorId) -> bool {
        self.actors.contains_key(projectile)
    }

    /// Step an attached behavior.
    pub fn step(
        &mut self,
        projectile: &OwnedActor,
        body: &BodyState,
        time_seconds: f64,
    ) -> Result<Option<WeaponTrajectoryUpdate>, WorldError> {
        let Some(entry) = self.actors.get_mut(projectile.id()) else {
            return Ok(None);
        };
        self.registry.borrow().assert_owned(projectile)?;
        if entry.owner != *projectile {
            return Err(WorldError::BadWeaponBehavior(
                "Weapon behavior actor ownership changed".to_string(),
            ));
        }
        Ok(entry.instance.step(body, time_seconds))
    }

    /// Detach and close a behavior.
    pub fn detach(&mut self, actor: &ActorId) {
        if let Some(mut entry) = self.actors.remove(actor) {
            entry.instance.close();
        }
    }

    /// Checkpoint live attachments.
    #[must_use]
    pub fn checkpoint(&self) -> WeaponBehaviorAttachmentCheckpoint {
        WeaponBehaviorAttachmentCheckpoint {
            version: 1,
            attachments: self
                .actors
                .values()
                .map(|entry| WeaponBehaviorAttachment {
                    projectile: SavedActorId::from(entry.owner.id()),
                    owner: entry.owner.owner().clone(),
                    definition: entry.instance.definition().clone(),
                })
                .collect(),
        }
    }

    /// Restore into an empty owner from a matching checkpoint.
    pub fn restore(&mut self, checkpoint: &WeaponBehaviorAttachmentCheckpoint) -> Result<(), WorldError> {
        if checkpoint.version != 1 || !self.actors.is_empty() {
            return Err(WorldError::BadWeaponBehavior(
                "Weapon behavior attachments require an empty matching checkpoint owner".to_string(),
            ));
        }
        let mut seen = HashSet::new();
        let mut pending = Vec::with_capacity(checkpoint.attachments.len());
        for entry in &checkpoint.attachments {
            let owner = self.registry.borrow().resolve_saved(&entry.projectile);
            let source = self.sources.get(entry.definition.id.as_str());
            let valid = owner.as_ref().is_some_and(|owner| owner.owner() == &entry.owner)
                && source.is_some_and(|source| same_weapon_behavior(source.definition(), &entry.definition))
                && seen.insert(owner.as_ref().map(|owner| owner.id().clone()));
            if !valid {
                return Err(WorldError::BadWeaponBehavior(
                    "Saved weapon behavior owner or source differs".to_string(),
                ));
            }
            let (owner, source) = (owner.expect("validated owner"), source.expect("validated source"));
            pending.push((owner, Rc::clone(source)));
        }
        for (owner, source) in pending {
            match source.resume(owner.id()) {
                Ok(instance) => {
                    self.actors
                        .insert(owner.id().clone(), AttachedBehavior { owner, instance });
                }
                Err(error) => {
                    for actor in self.actors.keys().cloned().collect::<Vec<_>>() {
                        self.detach(&actor);
                    }
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    /// Detach every behavior and clear sources.
    pub fn close(&mut self) {
        for actor in self.actors.keys().cloned().collect::<Vec<_>>() {
            self.detach(&actor);
        }
        self.sources.clear();
    }
}

fn read_module(reader: SaveReader) -> Result<WeaponBehaviorModule, WorldError> {
    Ok(WeaponBehaviorModule {
        id: namespaced(reader.field("id"))?,
        artifact_path: reader.field("artifactPath").string()?,
        digest: read_digest(reader.field("digest"))?,
        revision: reader.field("revision").string()?,
    })
}

fn check_module(reader: SaveReader, wanted: &WeaponBehaviorModule) -> Result<(), WorldError> {
    let found = read_module(reader.clone())?;
    if found.id != wanted.id
        || found.artifact_path != wanted.artifact_path
        || found.digest != wanted.digest
        || found.revision != wanted.revision
    {
        return Err(reader.fail("weapon behavior module differs from the loaded artifact"));
    }
    Ok(())
}

fn read_abi(reader: SaveReader) -> Result<WeaponBehaviorAbi, WorldError> {
    let kind = reader
        .field("kind")
        .choice_str(&["windows-i386", "windows-x86-64", "linux-i386", "linux-x86-64"])?;
    let (image, pointer_bytes, call) = match kind.as_str() {
        "windows-i386" => (
            reader.field("image").literal_str("pe32")?,
            reader.field("pointerBytes").literal_i64(4)?,
            reader
                .field("call")
                .choice_str(&["cdecl", "stdcall", "thiscall", "fastcall"])?,
        ),
        "windows-x86-64" => (
            reader.field("image").literal_str("pe32+")?,
            reader.field("pointerBytes").literal_i64(8)?,
            reader.field("call").literal_str("microsoft-x64")?,
        ),
        "linux-i386" => (
            reader.field("image").literal_str("elf32")?,
            reader.field("pointerBytes").literal_i64(4)?,
            reader.field("call").literal_str("system-v-i386")?,
        ),
        _ => (
            reader.field("image").literal_str("elf64")?,
            reader.field("pointerBytes").literal_i64(8)?,
            reader.field("call").literal_str("system-v-x86-64")?,
        ),
    };
    Ok(WeaponBehaviorAbi {
        kind,
        image,
        call,
        pointer_bytes: u32::try_from(pointer_bytes)
            .map_err(|_| reader.field("pointerBytes").fail("expected an integer in range"))?,
    })
}

fn check_callback(reader: SaveReader, wanted: &Option<WeaponBehaviorCallback>) -> Result<(), WorldError> {
    let Some(wanted) = wanted else {
        if !matches!(reader.value, Some(SaveJson::Null)) {
            return Err(reader.fail("unexpected activation callback"));
        }
        return Ok(());
    };
    reader.field("kind").literal_str(wanted.kind_name())?;
    let module = match wanted {
        WeaponBehaviorCallback::QuakeC { module, .. }
        | WeaponBehaviorCallback::Qvm { module, .. }
        | WeaponBehaviorCallback::NativeArtifact { module, .. } => module,
    };
    check_module(reader.field("module"), module)?;
    match wanted {
        WeaponBehaviorCallback::QuakeC { function_index, .. } => {
            reader.field("functionIndex").literal_i64(i64::from(*function_index))?;
        }
        WeaponBehaviorCallback::Qvm { instruction_index, .. } => {
            reader
                .field("instructionIndex")
                .literal_i64(i64::from(*instruction_index))?;
        }
        WeaponBehaviorCallback::NativeArtifact { image_offset, abi, .. } => {
            if reader.field("imageOffset").bigint()? != *image_offset {
                return Err(reader.fail("native weapon entry differs from the loaded artifact"));
            }
            let found = read_abi(reader.field("abi"))?;
            if found.kind != abi.kind
                || found.call != abi.call
                || found.image != abi.image
                || found.pointer_bytes != abi.pointer_bytes
            {
                return Err(reader.fail("native weapon callback ABI differs from the loaded artifact"));
            }
        }
    }
    Ok(())
}

/// Reuse loaded callback identities; saved data cannot introduce an
/// executable entrypoint. Returns the expected definition.
pub fn read_weapon_behavior_definition(
    reader: SaveReader,
    expected: &WeaponBehaviorDefinition,
) -> Result<WeaponBehaviorDefinition, WorldError> {
    reader.field("id").literal_str(&expected.id)?;
    reader.field("title").string()?;
    reader.field("role").literal_str(expected.role.name())?;
    reader.field("aspect").literal_str("trajectory")?;
    check_module(reader.field("module"), &expected.module)?;
    check_callback(reader.field("fire"), &Some(expected.fire.clone()))?;
    check_callback(reader.field("activate"), &expected.activate)?;
    Ok(expected.clone())
}

fn parse_provider(text: &str, reader: SaveReader) -> Result<ProviderId, WorldError> {
    let Some(colon) = text.find(':') else {
        return Err(reader.fail("expected a namespaced identity"));
    };
    if colon == 0 || colon + 1 >= text.len() {
        return Err(reader.fail("expected a namespaced identity"));
    }
    Ok(ProviderId::new(&text[..colon], &text[colon + 1..]))
}

/// Read an attachment checkpoint against the selected definitions.
pub fn read_weapon_behavior_attachment_checkpoint(
    reader: SaveReader,
    definitions: &HashMap<String, WeaponBehaviorDefinition>,
) -> Result<WeaponBehaviorAttachmentCheckpoint, WorldError> {
    let version = reader.field("version").literal_i64(1)?;
    let attachments = reader.field("attachments").list(|entry| {
        let definition_reader = entry.field("definition");
        let id = definition_reader.field("id").string()?;
        let expected = definitions
            .get(&id)
            .ok_or_else(|| definition_reader.fail("saved weapon behavior is not selected"))?;
        let owner_text = namespaced(entry.field("owner"))?;
        Ok(WeaponBehaviorAttachment {
            projectile: read_saved_actor(entry.field("projectile"))?,
            owner: parse_provider(&owner_text, entry.field("owner"))?,
            definition: read_weapon_behavior_definition(definition_reader, expected)?,
        })
    })?;
    Ok(WeaponBehaviorAttachmentCheckpoint { version, attachments })
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;
    use qa_core::math::vec3;
    use qa_core::math::Bounds;

    use crate::save::value::{arr, int, obj, str as json_str};

    fn module() -> WeaponBehaviorModule {
        WeaponBehaviorModule {
            id: "q1:game".to_string(),
            artifact_path: "progs.dat".to_string(),
            digest: "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_string(),
            revision: "1".to_string(),
        }
    }

    fn definition() -> WeaponBehaviorDefinition {
        WeaponBehaviorDefinition {
            id: "q1:rocket-behavior".to_string(),
            title: "Rocket".to_string(),
            module: module(),
            role: ProjectileRole::Rocket,
            activate: None,
            fire: WeaponBehaviorCallback::QuakeC {
                module: module(),
                function_index: 7,
            },
        }
    }

    struct StubInstance {
        definition: WeaponBehaviorDefinition,
        steps: Rc<RefCell<usize>>,
        closed: Rc<RefCell<bool>>,
    }

    impl WeaponBehaviorInstance for StubInstance {
        fn initial(&self) -> WeaponTrajectoryUpdate {
            WeaponTrajectoryUpdate {
                origin: vec3(1.0, 2.0, 3.0),
                velocity: vec3(0.0, 0.0, 0.0),
                angles: vec3(0.0, 0.0, 0.0),
            }
        }
        fn definition(&self) -> &WeaponBehaviorDefinition {
            &self.definition
        }
        fn step(&mut self, _body: &BodyState, _time_seconds: f64) -> Option<WeaponTrajectoryUpdate> {
            *self.steps.borrow_mut() += 1;
            Some(self.initial())
        }
        fn close(&mut self) {
            *self.closed.borrow_mut() = true;
        }
    }

    struct StubSource {
        definition: WeaponBehaviorDefinition,
        steps: Rc<RefCell<usize>>,
        closed: Rc<RefCell<bool>>,
        decline: bool,
    }

    impl WeaponBehaviorSource for StubSource {
        fn definition(&self) -> &WeaponBehaviorDefinition {
            &self.definition
        }
        fn attach(&self, _launch: &WeaponBehaviorLaunch) -> Option<Box<dyn WeaponBehaviorInstance>> {
            if self.decline {
                return None;
            }
            Some(Box::new(StubInstance {
                definition: self.definition.clone(),
                steps: Rc::clone(&self.steps),
                closed: Rc::clone(&self.closed),
            }))
        }
        fn resume(&self, _projectile: &ActorId) -> Result<Box<dyn WeaponBehaviorInstance>, WorldError> {
            if self.decline {
                return Err(WorldError::BadWeaponBehavior("declined".to_string()));
            }
            Ok(Box::new(StubInstance {
                definition: self.definition.clone(),
                steps: Rc::clone(&self.steps),
                closed: Rc::clone(&self.closed),
            }))
        }
    }

    fn body() -> BodyState {
        BodyState {
            origin: vec3(0.0, 0.0, 0.0),
            angles: vec3(0.0, 0.0, 0.0),
            velocity: vec3(0.0, 0.0, 0.0),
            bounds: Bounds {
                min: vec3(-1.0, -1.0, -1.0),
                max: vec3(1.0, 1.0, 1.0),
            },
            ground: None,
        }
    }

    fn harness() -> (
        Rc<RefCell<ActorRegistry>>,
        Rc<RefCell<WeaponBehaviorAttachments>>,
        OwnedActor,
        OwnedActor,
    ) {
        let registry = Rc::new(RefCell::new(
            ActorRegistry::new(IdentityOwner::create("weapon-test").unwrap(), 8).unwrap(),
        ));
        let attachments = WeaponBehaviorAttachments::new(Rc::clone(&registry));
        let owner = ProviderId::new("q1", "game");
        let shooter = registry.borrow_mut().allocate(owner.clone(), "q1:player").unwrap();
        let projectile = registry.borrow_mut().allocate(owner, "q1:rocket").unwrap();
        (registry, attachments, shooter, projectile)
    }

    fn launch(projectile: &OwnedActor, shooter: &ActorId) -> WeaponBehaviorLaunch {
        WeaponBehaviorLaunch {
            projectile: projectile.clone(),
            shooter: shooter.clone(),
            weapon: "q1:rocket-launcher".to_string(),
            role: ProjectileRole::Rocket,
            time_seconds: 1.0,
            body: body(),
        }
    }

    #[test]
    fn attach_launch_step_detach_flow() {
        let (_registry, attachments, shooter, projectile) = harness();
        let source = Rc::new(StubSource {
            definition: definition(),
            steps: Rc::new(RefCell::new(0)),
            closed: Rc::new(RefCell::new(false)),
            decline: false,
        });
        attachments.borrow_mut().register(source).unwrap();
        let input = launch(&projectile, shooter.id());
        let initial = attachments
            .borrow_mut()
            .launch("q1:rocket-behavior", &input)
            .unwrap()
            .expect("attached");
        assert_eq!(initial.origin, vec3(1.0, 2.0, 3.0));
        assert!(attachments.borrow().controls_trajectory(projectile.id()));
        assert!(attachments
            .borrow_mut()
            .step(&projectile, &body(), 2.0)
            .unwrap()
            .is_some());
        attachments.borrow_mut().detach(projectile.id());
        assert!(!attachments.borrow().controls_trajectory(projectile.id()));
        assert!(attachments
            .borrow_mut()
            .step(&projectile, &body(), 2.0)
            .unwrap()
            .is_none());
    }

    #[test]
    fn attach_validates_roles_and_duplicates() {
        let (_registry, attachments, shooter, projectile) = harness();
        let source = Rc::new(StubSource {
            definition: definition(),
            steps: Rc::new(RefCell::new(0)),
            closed: Rc::new(RefCell::new(false)),
            decline: false,
        });
        attachments
            .borrow_mut()
            .register(Rc::clone(&source) as Rc<dyn WeaponBehaviorSource>)
            .unwrap();
        assert!(attachments
            .borrow_mut()
            .register(source as Rc<dyn WeaponBehaviorSource>)
            .is_err());
        let mut bad_role = launch(&projectile, shooter.id());
        bad_role.role = ProjectileRole::Grenade;
        assert!(attachments
            .borrow_mut()
            .attach("q1:rocket-behavior", &bad_role)
            .is_err());
        let input = launch(&projectile, shooter.id());
        assert!(attachments.borrow_mut().attach("q1:rocket-behavior", &input).unwrap());
        assert!(attachments.borrow_mut().attach("q1:rocket-behavior", &input).is_err());
        assert!(attachments.borrow_mut().attach("q1:missing", &input).is_err());
    }

    #[test]
    fn release_detaches_through_registry_hook() {
        let (registry, attachments, shooter, projectile) = harness();
        attachments
            .borrow_mut()
            .register(Rc::new(StubSource {
                definition: definition(),
                steps: Rc::new(RefCell::new(0)),
                closed: Rc::new(RefCell::new(false)),
                decline: false,
            }) as Rc<dyn WeaponBehaviorSource>)
            .unwrap();
        let input = launch(&projectile, shooter.id());
        attachments.borrow_mut().attach("q1:rocket-behavior", &input).unwrap();
        registry.borrow_mut().release(&projectile).unwrap();
        assert!(!attachments.borrow().controls_trajectory(projectile.id()));
    }

    #[test]
    fn checkpoint_restore_round_trips() {
        let (_registry, attachments, shooter, projectile) = harness();
        attachments
            .borrow_mut()
            .register(Rc::new(StubSource {
                definition: definition(),
                steps: Rc::new(RefCell::new(0)),
                closed: Rc::new(RefCell::new(false)),
                decline: false,
            }) as Rc<dyn WeaponBehaviorSource>)
            .unwrap();
        let input = launch(&projectile, shooter.id());
        attachments.borrow_mut().attach("q1:rocket-behavior", &input).unwrap();
        let checkpoint = attachments.borrow().checkpoint();
        assert_eq!(checkpoint.version, 1);
        assert_eq!(checkpoint.attachments.len(), 1);
        assert_eq!(checkpoint.attachments[0].definition, definition());

        attachments.borrow_mut().detach(projectile.id());
        attachments.borrow_mut().restore(&checkpoint).unwrap();
        assert!(attachments.borrow().controls_trajectory(projectile.id()));
        // Restore requires an empty owner.
        assert!(attachments.borrow_mut().restore(&checkpoint).is_err());
        attachments.borrow_mut().close();
        assert!(!attachments.borrow().controls_trajectory(projectile.id()));
    }

    #[test]
    fn definition_reader_reuses_loaded_identities() {
        let expected = definition();
        let value = obj(vec![
            ("id", json_str("q1:rocket-behavior")),
            ("title", json_str("Rocket")),
            ("role", json_str("rocket")),
            ("aspect", json_str("trajectory")),
            (
                "module",
                obj(vec![
                    ("id", json_str("q1:game")),
                    ("artifactPath", json_str("progs.dat")),
                    (
                        ("digest"),
                        json_str("sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"),
                    ),
                    ("revision", json_str("1")),
                ]),
            ),
            (
                "fire",
                obj(vec![
                    ("kind", json_str("quakec")),
                    (
                        "module",
                        obj(vec![
                            ("id", json_str("q1:game")),
                            ("artifactPath", json_str("progs.dat")),
                            (
                                ("digest"),
                                json_str("sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"),
                            ),
                            ("revision", json_str("1")),
                        ]),
                    ),
                    ("functionIndex", int(7)),
                ]),
            ),
            ("activate", SaveJson::Null),
        ]);
        let read = read_weapon_behavior_definition(SaveReader::new(&value), &expected).unwrap();
        assert_eq!(read, expected);
        assert!(same_weapon_behavior(&read, &expected));

        let mut different = expected.clone();
        different.role = ProjectileRole::Grenade;
        assert!(!same_weapon_behavior(&read, &different));

        let mut definitions = HashMap::new();
        definitions.insert(expected.id.clone(), expected.clone());
        let checkpoint_value = obj(vec![
            ("version", int(1)),
            (
                "attachments",
                arr(vec![obj(vec![
                    ("projectile", obj(vec![("slot", int(1)), ("generation", int(0))])),
                    ("owner", json_str("q1:game")),
                    ("definition", value),
                ])]),
            ),
        ]);
        let checkpoint =
            read_weapon_behavior_attachment_checkpoint(SaveReader::new(&checkpoint_value), &definitions).unwrap();
        assert_eq!(checkpoint.attachments.len(), 1);
        assert_eq!(checkpoint.attachments[0].owner, ProviderId::new("q1", "game"));

        let unknown = obj(vec![
            ("version", int(1)),
            (
                "attachments",
                arr(vec![obj(vec![
                    ("projectile", obj(vec![("slot", int(1)), ("generation", int(0))])),
                    ("owner", json_str("q1:game")),
                    (
                        "definition",
                        obj(vec![
                            ("id", json_str("q1:unknown")),
                            ("title", json_str("?")),
                            ("role", json_str("rocket")),
                            ("aspect", json_str("trajectory")),
                            (
                                "module",
                                obj(vec![
                                    ("id", json_str("q1:game")),
                                    ("artifactPath", json_str("progs.dat")),
                                    (
                                        ("digest"),
                                        json_str(
                                            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
                                        ),
                                    ),
                                    ("revision", json_str("1")),
                                ]),
                            ),
                            ("fire", SaveJson::Null),
                            ("activate", SaveJson::Null),
                        ]),
                    ),
                ])]),
            ),
        ]);
        assert!(read_weapon_behavior_attachment_checkpoint(SaveReader::new(&unknown), &definitions).is_err());
    }

    #[test]
    fn role_names_round_trip() {
        for role in [
            ProjectileRole::Rocket,
            ProjectileRole::Grenade,
            ProjectileRole::Nail,
            ProjectileRole::Bolt,
            ProjectileRole::Plasma,
            ProjectileRole::Energy,
            ProjectileRole::Grapple,
        ] {
            assert_eq!(ProjectileRole::parse(role.name()), Some(role));
        }
        assert_eq!(ProjectileRole::parse("bogus"), None);
    }
}
