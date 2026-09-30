//! Q2 rerelease native weapon trajectory behavior profiles.
//!
//! Donor: `src/compat/q2/rerelease/weapon-behavior-profile.ts` — bridges
//! evidence-backed trajectory declarations into equip/launch/project calls.

use std::collections::HashMap;

use qa_core::math::Vec3;
use qa_guest::core::contracts::{GuestAddress, GuestAllocationOptions};
use qa_guest::core::memory::SparseGuestMemory;
use qa_guest::GuestError;
use thiserror::Error;

/// Q2Eaks v0.21 artifact digest.
pub const Q2EAKS_WEAPON_DIGEST: &str = "sha256:b60b79f7fb6f115218681a9cbab8765267e34f72466975526df05ad288925dde";

/// Weapon behavior failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WeaponBehaviorError {
    /// Native trajectory behavior requires an artifact-qualified executable profile.
    #[error("Native trajectory behavior requires an artifact-qualified executable profile")]
    NoProfile,
    /// Invalid native profile artifact identity.
    #[error("Invalid native profile artifact identity")]
    BadArtifact,
    /// Fire must identify a declared launch call.
    #[error("Fire must identify a declared launch call")]
    BadFire,
    /// Activation must identify a declared equip call.
    #[error("Activation must identify a declared equip call")]
    BadActivation,
    /// Initialization classes require one worldspawn and unique classes.
    #[error("Initialization classes require one worldspawn and unique classes")]
    BadInitClasses,
    /// Native projectile has a due think with no callback.
    #[error("Native projectile has a due think with no callback")]
    ThinkWithoutCallback,
    /// Native weapon requires its source entity record.
    #[error("Native weapon requires its source entity record")]
    NoEntity,
    /// Native weapon requires its shooter client record.
    #[error("Native weapon requires its shooter client record")]
    NoClient,
    /// Native weapon think registration differs.
    #[error("Native weapon think registration differs")]
    BadThinkRegistration,
    /// Native weapon expects another equipped fire.
    #[error("Native weapon expects another equipped fire")]
    UnexpectedEquip,
    /// Native projectile left its declared fire without a callback.
    #[error("Native projectile left its declared fire without a callback")]
    FireWithoutTouch,
    /// Native source has not registered its provisioning capability.
    #[error("Native source has not registered its provisioning capability")]
    ProvisioningNotRegistered,
    /// Native provisioning changed an undeclared source cvar.
    #[error("Native provisioning changed an undeclared source cvar")]
    ProvisioningChangedOther,
    /// Native component initialization requires one authored worldspawn.
    #[error("Native component initialization requires one authored worldspawn")]
    InitWorldspawn,
    /// Native component entity field cannot be quoted.
    #[error("Native component entity field cannot be quoted")]
    UnquotableField,
    /// Malformed entity text.
    #[error("Malformed entity text")]
    MalformedEntities,
    /// Guest memory failure.
    #[error(transparent)]
    Guest(#[from] GuestError),
}

/// Entity field contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeaponEntityFields {
    /// Record byte length.
    pub byte_length: usize,
    /// Origin offset.
    pub origin: usize,
    /// Angles offset.
    pub angles: usize,
    /// Velocity offset.
    pub velocity: usize,
    /// Client offset.
    pub client: usize,
    /// Owner offset.
    pub owner: usize,
    /// View height offset.
    pub view_height: usize,
    /// Generation offset.
    pub generation: usize,
    /// Next think offset.
    pub next_think: usize,
    /// Think callback offset.
    pub think_callback: usize,
    /// Think registration offset.
    pub think_registration: usize,
    /// Touch callback offset.
    pub touch_callback: usize,
}

/// Client field contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeaponClientFields {
    /// Record byte length.
    pub byte_length: usize,
    /// Weapon offset.
    pub weapon: usize,
    /// View angles offset.
    pub view_angles: usize,
    /// Forward offset.
    pub forward: usize,
}

/// Weapon command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeaponCommand {
    /// Arguments.
    pub arguments: Vec<String>,
    /// Tail text.
    pub tail: String,
}

/// Weapon cvar override.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeaponCvar {
    /// Cvar name.
    pub name: String,
    /// Value.
    pub value: String,
}

/// Compact trajectory declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeaponProfileDeclaration {
    /// Behavior id.
    pub id: String,
    /// Title.
    pub title: String,
    /// Weapon role.
    pub role: String,
    /// Artifact path.
    pub artifact_path: String,
    /// Artifact digest.
    pub artifact_digest: String,
    /// Entity fields.
    pub entity: WeaponEntityFields,
    /// Client fields.
    pub client: WeaponClientFields,
    /// Equipped weapon record length.
    pub equipped_byte_length: usize,
    /// Equipped weapon callback offset.
    pub equipped_callback: usize,
    /// Expected equipped entry RVA.
    pub equipped_expected: u64,
    /// Time RVA.
    pub time_rva: u64,
    /// Think save tag.
    pub think_tag: u32,
    /// Allocate RVA.
    pub allocate_rva: u64,
    /// Free RVA.
    pub free_rva: u64,
    /// Projectile touch RVA.
    pub projectile_touch_rva: u64,
    /// Equip chain RVAs.
    pub equip: Vec<u64>,
    /// Launch chain RVAs.
    pub launch: Vec<u64>,
    /// Activation RVA.
    pub activate_rva: Option<u64>,
    /// Fire RVA.
    pub fire_rva: u64,
    /// Initialization classes.
    pub initialization_classes: Vec<String>,
    /// Equipment commands.
    pub equipment: Vec<WeaponCommand>,
    /// Ammunition command.
    pub ammunition: WeaponCommand,
    /// Initial cvars.
    pub initial_cvars: Vec<WeaponCvar>,
    /// Provisioning cvars.
    pub provisioning_cvars: Vec<WeaponCvar>,
}

/// Built-in faster-rockets declaration for the exact v0.21 artifact.
#[must_use]
pub fn builtin_declaration(artifact_path: &str) -> WeaponProfileDeclaration {
    let command = |arguments: &[&str], tail: &str| WeaponCommand {
        arguments: arguments.iter().map(|value| (*value).to_string()).collect(),
        tail: tail.to_string(),
    };
    WeaponProfileDeclaration {
        id: "native:rocket-trajectory".to_string(),
        title: "Faster rockets".to_string(),
        role: "rocket".to_string(),
        artifact_path: artifact_path.to_string(),
        artifact_digest: Q2EAKS_WEAPON_DIGEST.to_string(),
        entity: WeaponEntityFields {
            byte_length: 0x7a8,
            origin: 4,
            angles: 16,
            velocity: 0x694,
            client: 0x78,
            owner: 0x5b8,
            view_height: 0x7a0,
            generation: 0x5c0,
            next_think: 0x6d8,
            think_callback: 0x700,
            think_registration: 0x708,
            touch_callback: 0x710,
        },
        client: WeaponClientFields {
            byte_length: 0x19b0,
            weapon: 0xbe8,
            view_angles: 0x1998,
            forward: 0x19a4,
        },
        equipped_byte_length: 0x30,
        equipped_callback: 0x28,
        equipped_expected: 0xefaf0,
        time_rva: 0x2999c8,
        think_tag: 20,
        allocate_rva: 0x95010,
        free_rva: 0x95140,
        projectile_touch_rva: 0x98060,
        equip: vec![0xed4d0],
        launch: vec![0xed420, 0xef900],
        activate_rva: Some(0xed4d0),
        fire_rva: 0xef900,
        initialization_classes: [
            "worldspawn",
            "info_player_start",
            "info_player_deathmatch",
            "info_player_coop",
            "info_player_team1",
            "info_player_team2",
            "info_player_intermission",
        ]
        .iter()
        .map(|value| (*value).to_string())
        .collect(),
        equipment: vec![
            command(&["give", "Rocket Launcher"], "Rocket Launcher"),
            command(&["give", "Rockets"], "Rockets"),
            command(&["use", "Rocket Launcher"], "Rocket Launcher"),
        ],
        ammunition: command(&["give", "Rockets"], "Rockets"),
        initial_cvars: vec![WeaponCvar {
            name: "g_faster_rockets".to_string(),
            value: "1".to_string(),
        }],
        provisioning_cvars: vec![WeaponCvar {
            name: "cheats".to_string(),
            value: "1".to_string(),
        }],
    }
}

/// Body state for projection.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WeaponBodyState {
    /// Origin.
    pub origin: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Velocity.
    pub velocity: Vec3,
}

/// Shooter snapshot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WeaponShooter {
    /// Body state.
    pub body: WeaponBodyState,
    /// View angles.
    pub view_angles: Vec3,
    /// View height.
    pub view_height: i32,
}

/// Trajectory update over strictly declared fields.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WeaponTrajectoryUpdate {
    /// Origin.
    pub origin: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Velocity.
    pub velocity: Vec3,
}

/// One recorded native invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeaponInvocation {
    /// Entry RVA.
    pub rva: u64,
    /// Target slot.
    pub slot: u32,
}

/// Headless synthetic weapon module.
pub struct SyntheticWeaponModule {
    /// Guest memory.
    pub memory: SparseGuestMemory,
    /// Image base.
    pub image_base: GuestAddress,
    /// Entity records by slot.
    pub entities: HashMap<u32, GuestAddress>,
    /// Client records by slot.
    pub clients: HashMap<u32, GuestAddress>,
    /// Level time address.
    pub time: GuestAddress,
    /// Recorded invocations.
    pub invocations: Vec<WeaponInvocation>,
    next_slot: u32,
    /// Scripted think results by slot.
    pub think_present: HashMap<u32, bool>,
}

/// Native weapon trajectory profile.
pub struct RereleaseWeaponProfile {
    /// Declaration.
    pub declaration: WeaponProfileDeclaration,
    /// Think registration callback RVA.
    pub think_callback_rva: u64,
}

impl RereleaseWeaponProfile {
    /// Build a profile from an accepted declaration, or the built-in one
    /// when the artifact matches. Returns `None` for other artifacts.
    pub fn new(
        module_digest: &str,
        artifact_path: &str,
        accepted: Option<WeaponProfileDeclaration>,
    ) -> Result<Option<Self>, WeaponBehaviorError> {
        let declaration = match accepted {
            Some(accepted) => {
                if accepted.artifact_digest != module_digest || accepted.artifact_path != artifact_path {
                    return Err(WeaponBehaviorError::BadArtifact);
                }
                accepted
            }
            None => {
                if module_digest != Q2EAKS_WEAPON_DIGEST {
                    return Ok(None);
                }
                builtin_declaration(artifact_path)
            }
        };
        if !declaration.launch.contains(&declaration.fire_rva) {
            return Err(WeaponBehaviorError::BadFire);
        }
        if declaration
            .activate_rva
            .is_some_and(|rva| !declaration.equip.contains(&rva))
        {
            return Err(WeaponBehaviorError::BadActivation);
        }
        let worldspawn = declaration
            .initialization_classes
            .iter()
            .filter(|name| *name == "worldspawn")
            .count();
        let unique: std::collections::HashSet<&str> =
            declaration.initialization_classes.iter().map(String::as_str).collect();
        if worldspawn != 1 || unique.len() != declaration.initialization_classes.len() {
            return Err(WeaponBehaviorError::BadInitClasses);
        }
        Ok(Some(Self {
            declaration,
            think_callback_rva: 0,
        }))
    }

    /// Equip chain invocation over a shooter slot.
    pub fn equip(&self, module: &mut SyntheticWeaponModule, slot: u32) -> Result<(), WeaponBehaviorError> {
        for rva in &self.declaration.equip {
            module.invoke(*rva, slot)?;
        }
        Ok(())
    }

    /// Launch chain invocation over a shooter slot.
    pub fn launch(&self, module: &mut SyntheticWeaponModule, slot: u32) -> Result<(), WeaponBehaviorError> {
        for rva in &self.declaration.launch {
            module.invoke(*rva, slot)?;
        }
        Ok(())
    }

    /// Whether a shooter matches the equipped behavior contract.
    pub fn matches(&self, module: &mut SyntheticWeaponModule, slot: u32) -> Result<bool, WeaponBehaviorError> {
        let record = module.entity(slot)?;
        let client = module
            .memory
            .read_pointer(module.memory.offset(record, self.declaration.entity.client as i64)?)?
            .ok_or(WeaponBehaviorError::NoClient)?;
        let weapon = module
            .memory
            .read_pointer(module.memory.offset(client, self.declaration.client.weapon as i64)?)?;
        let Some(weapon) = weapon else {
            return Ok(false);
        };
        let callback = module.memory.read_pointer(
            module
                .memory
                .offset(weapon, self.declaration.equipped_callback as i64)?,
        )?;
        let expected = module
            .memory
            .offset(module.image_base, self.declaration.equipped_expected as i64)?;
        Ok(callback == Some(expected))
    }

    /// Project a shooter record, computing the forward vector from view
    /// angles exactly like the native equipment path.
    pub fn project_shooter(
        &self,
        module: &mut SyntheticWeaponModule,
        slot: u32,
        shooter: &WeaponShooter,
    ) -> Result<(), WeaponBehaviorError> {
        self.project(module, slot, &shooter.body)?;
        let record = module.entity(slot)?;
        let client = module
            .memory
            .read_pointer(module.memory.offset(record, self.declaration.entity.client as i64)?)?
            .ok_or(WeaponBehaviorError::NoClient)?;
        module.write_vec(
            module
                .memory
                .offset(client, self.declaration.client.view_angles as i64)?,
            shooter.view_angles,
        )?;
        let pitch = shooter.view_angles.x * std::f32::consts::PI / 180.0;
        let yaw = shooter.view_angles.y * std::f32::consts::PI / 180.0;
        module.write_vec(
            module.memory.offset(client, self.declaration.client.forward as i64)?,
            Vec3 {
                x: pitch.cos() * yaw.cos(),
                y: pitch.cos() * yaw.sin(),
                z: -pitch.sin(),
            },
        )?;
        module.memory.write_i32(
            module
                .memory
                .offset(record, self.declaration.entity.view_height as i64)?,
            shooter.view_height,
        )?;
        Ok(())
    }

    /// Project a body onto a record.
    pub fn project(
        &self,
        module: &mut SyntheticWeaponModule,
        slot: u32,
        body: &WeaponBodyState,
    ) -> Result<(), WeaponBehaviorError> {
        let record = module.entity(slot)?;
        let memory = &mut module.memory;
        let write_vec = |memory: &mut SparseGuestMemory, offset: usize, value: Vec3| {
            memory.write_f32(memory.offset(record, offset as i64)?, value.x)?;
            memory.write_f32(memory.offset(record, offset as i64 + 4)?, value.y)?;
            memory.write_f32(memory.offset(record, offset as i64 + 8)?, value.z)?;
            Ok::<(), WeaponBehaviorError>(())
        };
        write_vec(memory, self.declaration.entity.origin, body.origin)?;
        write_vec(memory, self.declaration.entity.angles, body.angles)?;
        write_vec(memory, self.declaration.entity.velocity, body.velocity)?;
        Ok(())
    }

    /// Read the strictly declared trajectory fields.
    pub fn trajectory(
        &self,
        module: &mut SyntheticWeaponModule,
        slot: u32,
    ) -> Result<WeaponTrajectoryUpdate, WeaponBehaviorError> {
        let record = module.entity(slot)?;
        let memory = &mut module.memory;
        let read_vec = |memory: &mut SparseGuestMemory, offset: usize| {
            memory
                .offset(record, offset as i64)
                .and_then(|at| memory.read_f32x3(at))
                .map_err(WeaponBehaviorError::from)
        };
        Ok(WeaponTrajectoryUpdate {
            origin: read_vec(memory, self.declaration.entity.origin)?,
            angles: read_vec(memory, self.declaration.entity.angles)?,
            velocity: read_vec(memory, self.declaration.entity.velocity)?,
        })
    }

    /// Spawn generation.
    pub fn generation(&self, module: &mut SyntheticWeaponModule, slot: u32) -> Result<i32, WeaponBehaviorError> {
        let record = module.entity(slot)?;
        Ok(module.memory.read_i32(
            module
                .memory
                .offset(record, self.declaration.entity.generation as i64)?,
        )?)
    }

    /// Write level time.
    pub fn set_time(&self, module: &mut SyntheticWeaponModule, milliseconds: i64) {
        module.memory.write_i64(module.time, milliseconds).expect("time");
    }

    /// Next think deadline.
    pub fn next_think(&self, module: &mut SyntheticWeaponModule, slot: u32) -> Result<i64, WeaponBehaviorError> {
        let record = module.entity(slot)?;
        Ok(module.memory.read_i64(
            module
                .memory
                .offset(record, self.declaration.entity.next_think as i64)?,
        )?)
    }

    /// Run a due think callback, checking its registration.
    pub fn think(&mut self, module: &mut SyntheticWeaponModule, slot: u32) -> Result<(), WeaponBehaviorError> {
        let record = module.entity(slot)?;
        let callback = module.memory.read_pointer(
            module
                .memory
                .offset(record, self.declaration.entity.think_callback as i64)?,
        )?;
        let registration = module.memory.read_pointer(
            module
                .memory
                .offset(record, self.declaration.entity.think_registration as i64)?,
        )?;
        let Some(callback) = callback else {
            return Err(WeaponBehaviorError::ThinkWithoutCallback);
        };
        if registration.is_none() {
            return Err(WeaponBehaviorError::BadThinkRegistration);
        }
        self.think_callback_rva = callback.offset.wrapping_sub(module.image_base.offset);
        module.invoke(self.think_callback_rva, slot)?;
        Ok(())
    }

    /// Allocate a projectile record.
    pub fn allocate(&self, module: &mut SyntheticWeaponModule) -> Result<u32, WeaponBehaviorError> {
        let slot = module.allocate_record(self.declaration.entity.byte_length)?;
        module.invoke(self.declaration.allocate_rva, slot)?;
        Ok(slot)
    }

    /// Free a projectile record.
    pub fn free(&self, module: &mut SyntheticWeaponModule, slot: u32) -> Result<(), WeaponBehaviorError> {
        module.invoke(self.declaration.free_rva, slot)?;
        module.entities.remove(&slot);
        Ok(())
    }
}

impl SyntheticWeaponModule {
    /// Create over guest memory with an image base.
    pub fn new(
        memory: SparseGuestMemory,
        time_rva: u64,
        entity_byte_length: usize,
    ) -> Result<Self, WeaponBehaviorError> {
        let mut memory = memory;
        let image_base = memory.allocate(&GuestAllocationOptions::bytes(0x2a_0000))?;
        let _ = entity_byte_length;
        let time = memory.offset(image_base, time_rva as i64)?;
        Ok(Self {
            memory,
            image_base,
            entities: HashMap::new(),
            clients: HashMap::new(),
            time,
            invocations: Vec::new(),
            next_slot: 1,
            think_present: HashMap::new(),
        })
    }

    /// Entity record address.
    pub fn entity(&mut self, slot: u32) -> Result<GuestAddress, WeaponBehaviorError> {
        self.entities.get(&slot).copied().ok_or(WeaponBehaviorError::NoEntity)
    }

    /// Allocate an entity record.
    pub fn allocate_record(&mut self, byte_length: usize) -> Result<u32, WeaponBehaviorError> {
        let address = self.memory.allocate(&GuestAllocationOptions::bytes(byte_length))?;
        let slot = self.next_slot;
        self.next_slot += 1;
        self.entities.insert(slot, address);
        Ok(slot)
    }

    /// Attach a client record.
    pub fn attach_client(
        &mut self,
        slot: u32,
        byte_length: usize,
        entity_client_offset: usize,
    ) -> Result<GuestAddress, WeaponBehaviorError> {
        let client = self.memory.allocate(&GuestAllocationOptions::bytes(byte_length))?;
        self.clients.insert(slot, client);
        let record = self.entity(slot)?;
        self.memory
            .write_pointer(self.memory.offset(record, entity_client_offset as i64)?, Some(client))?;
        Ok(client)
    }

    /// Record a native invocation.
    pub fn invoke(&mut self, rva: u64, slot: u32) -> Result<(), WeaponBehaviorError> {
        self.invocations.push(WeaponInvocation { rva, slot });
        Ok(())
    }

    fn write_vec(&mut self, address: GuestAddress, value: Vec3) -> Result<(), WeaponBehaviorError> {
        self.memory.write_f32(address, value.x)?;
        self.memory.write_f32(self.memory.offset(address, 4)?, value.y)?;
        self.memory.write_f32(self.memory.offset(address, 8)?, value.z)?;
        Ok(())
    }
}

/// Cvar registry surface for provisioning.
pub trait WeaponCvarRegistry {
    /// Capture transfer state.
    fn capture(&self) -> Vec<(String, String)>;
    /// Look up a cvar.
    fn get(&self, name: &str) -> Option<String>;
    /// Set a cvar.
    fn set(&mut self, name: &str, value: &str);
    /// Restore transfer state.
    fn restore(&mut self, state: &[(String, String)]);
}

/// Provisioning may change private inventory, but its declared cvar
/// capability ends with the command.
pub fn with_weapon_provisioning<T>(
    provisioning: &[WeaponCvar],
    cvars: &mut dyn WeaponCvarRegistry,
    refresh: &mut dyn FnMut(),
    run: impl FnOnce() -> T,
) -> Result<T, WeaponBehaviorError> {
    let saved = cvars.capture();
    let names: std::collections::HashSet<&str> = provisioning.iter().map(|value| value.name.as_str()).collect();
    let independent = |state: &[(String, String)]| {
        state
            .iter()
            .filter(|(name, _)| !names.contains(name.as_str()))
            .cloned()
            .collect::<Vec<_>>()
    };
    let result = (|| {
        for value in provisioning {
            if cvars.get(&value.name).is_none() {
                return Err(WeaponBehaviorError::ProvisioningNotRegistered);
            }
            cvars.set(&value.name, &value.value);
        }
        refresh();
        let result = run();
        if independent(&cvars.capture()) != independent(&saved) {
            return Err(WeaponBehaviorError::ProvisioningChangedOther);
        }
        Ok(result)
    })();
    cvars.restore(&saved);
    refresh();
    result
}

/// One parsed entity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q1Entity {
    /// Properties in order.
    pub properties: Vec<(String, String)>,
}

/// Parse `quake1` entity text.
pub fn parse_q1_entities(text: &str) -> Result<Vec<Q1Entity>, WeaponBehaviorError> {
    let bytes = text.as_bytes();
    let mut cursor = 0;
    let mut entities = Vec::new();
    let skip_space = |cursor: &mut usize| {
        while *cursor < bytes.len() && bytes[*cursor].is_ascii_whitespace() {
            *cursor += 1;
        }
    };
    let quoted = |cursor: &mut usize| {
        skip_space(cursor);
        if bytes.get(*cursor) != Some(&b'"') {
            return Err(WeaponBehaviorError::MalformedEntities);
        }
        *cursor += 1;
        let start = *cursor;
        while *cursor < bytes.len() && bytes[*cursor] != b'"' {
            *cursor += 1;
        }
        if bytes.get(*cursor) != Some(&b'"') {
            return Err(WeaponBehaviorError::MalformedEntities);
        }
        let value = String::from_utf8_lossy(&bytes[start..*cursor]).into_owned();
        *cursor += 1;
        Ok(value)
    };
    loop {
        skip_space(&mut cursor);
        if cursor >= bytes.len() {
            break;
        }
        if bytes[cursor] != b'{' {
            return Err(WeaponBehaviorError::MalformedEntities);
        }
        cursor += 1;
        let mut properties = Vec::new();
        loop {
            skip_space(&mut cursor);
            if cursor >= bytes.len() {
                return Err(WeaponBehaviorError::MalformedEntities);
            }
            if bytes[cursor] == b'}' {
                cursor += 1;
                break;
            }
            let key = quoted(&mut cursor)?;
            let value = quoted(&mut cursor)?;
            properties.push((key, value));
        }
        entities.push(Q1Entity { properties });
    }
    Ok(entities)
}

/// Filter initialization entities to the declared classes, preserving
/// order, and requiring one authored worldspawn.
pub fn weapon_initialization_entities(text: &str, classes: &[String]) -> Result<String, WeaponBehaviorError> {
    let entities: Vec<Q1Entity> = parse_q1_entities(text)?
        .into_iter()
        .filter(|entity| {
            entity
                .properties
                .iter()
                .find(|(key, _)| key == "classname")
                .is_some_and(|(_, value)| classes.iter().any(|name| name == value))
        })
        .collect();
    let worldspawn = entities
        .iter()
        .filter(|entity| {
            entity
                .properties
                .iter()
                .any(|(key, value)| key == "classname" && value == "worldspawn")
        })
        .count();
    if worldspawn != 1 {
        return Err(WeaponBehaviorError::InitWorldspawn);
    }
    let quote = |value: &str| {
        if value.contains('"') || value.contains('\0') {
            return Err(WeaponBehaviorError::UnquotableField);
        }
        Ok(format!("\"{value}\""))
    };
    let mut out = String::new();
    for entity in &entities {
        out.push_str("{\n");
        for (key, value) in &entity.properties {
            out.push_str(&format!("{} {}\n", quote(key)?, quote(value)?));
        }
        out.push_str("}\n");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::ProviderId;
    use qa_guest::core::contracts::{ContentDigest, ModuleIdentity};

    fn test_module() -> SyntheticWeaponModule {
        let module = ModuleIdentity::new(
            ProviderId::new("q2", "weapon-test"),
            "q2eaks/game.dll",
            ContentDigest::new(Q2EAKS_WEAPON_DIGEST.split(':').next().unwrap_or("sha256"), "00"),
            "test",
        );
        let memory = SparseGuestMemory::new(module, 8, 0x1_0000).expect("memory");
        SyntheticWeaponModule::new(memory, 0x2999c8, 0x7a8).expect("module")
    }

    struct FakeCvars {
        values: HashMap<String, String>,
    }

    impl WeaponCvarRegistry for FakeCvars {
        fn capture(&self) -> Vec<(String, String)> {
            let mut values: Vec<(String, String)> = self
                .values
                .iter()
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect();
            values.sort();
            values
        }
        fn get(&self, name: &str) -> Option<String> {
            self.values.get(name).cloned()
        }
        fn set(&mut self, name: &str, value: &str) {
            self.values.insert(name.to_string(), value.to_string());
        }
        fn restore(&mut self, state: &[(String, String)]) {
            self.values = state.iter().cloned().collect();
        }
    }

    #[test]
    fn profile_gates_artifacts_and_drives_chains() {
        let none = RereleaseWeaponProfile::new("sha256:other", "q2eaks/game.dll", None).expect("none");
        assert!(none.is_none());
        let profile = RereleaseWeaponProfile::new(Q2EAKS_WEAPON_DIGEST, "q2eaks/game.dll", None)
            .expect("profile")
            .expect("built-in");
        assert_eq!(profile.declaration.role, "rocket");
        let mut module = test_module();
        let slot = profile.allocate(&mut module).expect("alloc");
        module
            .attach_client(
                slot,
                profile.declaration.client.byte_length,
                profile.declaration.entity.client,
            )
            .expect("client");
        profile.equip(&mut module, slot).expect("equip");
        profile.launch(&mut module, slot).expect("launch");
        assert_eq!(module.invocations.len(), 4);
        assert!(!profile.matches(&mut module, slot).expect("matches"));
        let shooter = WeaponShooter {
            body: WeaponBodyState {
                origin: Vec3 { x: 1.0, y: 2.0, z: 3.0 },
                angles: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                velocity: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            },
            view_angles: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            view_height: 22,
        };
        profile.project_shooter(&mut module, slot, &shooter).expect("project");
        let update = profile.trajectory(&mut module, slot).expect("trajectory");
        assert_eq!(update.origin.x, 1.0);
        assert_eq!(profile.generation(&mut module, slot).expect("gen"), 0);
        profile.set_time(&mut module, 5000);
        assert_eq!(module.memory.read_i64(module.time).expect("time"), 5000);
        profile.free(&mut module, slot).expect("free");
        assert!(!module.entities.contains_key(&slot));
    }

    #[test]
    fn provisioning_entities_and_think_checks() {
        let declaration = builtin_declaration("q2eaks/game.dll");
        let mut cvars = FakeCvars {
            values: HashMap::from([
                ("cheats".to_string(), "0".to_string()),
                ("sv_gravity".to_string(), "800".to_string()),
            ]),
        };
        let mut refreshes = 0;
        let result = with_weapon_provisioning(
            &declaration.provisioning_cvars,
            &mut cvars,
            &mut || refreshes += 1,
            || 42,
        )
        .expect("provision");
        assert_eq!(result, 42);
        assert_eq!(cvars.get("cheats").as_deref(), Some("0"));
        assert_eq!(refreshes, 2);
        let filtered = weapon_initialization_entities(
            "{ \"classname\" \"worldspawn\" }\n{ \"classname\" \"monster_soldier\" }\n{ \"classname\" \"info_player_start\" }\n",
            &declaration.initialization_classes,
        )
        .expect("entities");
        assert!(filtered.contains("worldspawn"));
        assert!(!filtered.contains("monster_soldier"));
        assert!(filtered.contains("info_player_start"));
        assert!(weapon_initialization_entities(
            "{ \"classname\" \"monster_soldier\" }\n",
            &declaration.initialization_classes
        )
        .is_err());
        let profile = RereleaseWeaponProfile::new(Q2EAKS_WEAPON_DIGEST, "q2eaks/game.dll", None)
            .expect("profile")
            .expect("built-in");
        assert_eq!(profile.declaration.equipment.len(), 3);
        assert_eq!(profile.declaration.ammunition.tail, "Rockets");
    }
}
