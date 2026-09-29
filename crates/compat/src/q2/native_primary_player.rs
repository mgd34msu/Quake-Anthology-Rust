//! Port of `src/compat/q2/native-primary-player.ts`.
//! Bridges source spawn selection, scores and private client pose.

use qa_core::math::Vec3;
use qa_guest::core::contracts::{
    GuestAccess, GuestAddress, GuestAllocationOptions, GuestCallResult, GuestCallValue, NativeAbi,
};

use super::native_primary_weapons::{
    HostResult, NativeActorId, NativeHostError, NativePrimaryWeaponProfile, SyntheticHost,
};

/// Match team declaration for scoreboard commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceTeam {
    /// Source selector.
    pub source: String,
    /// Canonical team id.
    pub team: String,
    /// Join arguments.
    pub arguments: Vec<String>,
}

/// Score storage plus match teams.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourcePrimaryMatch {
    /// Score field offset within the client.
    pub score: u32,
    /// Match teams.
    pub teams: Vec<SourceTeam>,
}

/// Objective-drop declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerObjectives {
    /// No objective drop.
    None,
    /// Objective-drop entry RVA.
    Entry(u32),
}

/// Native primary player profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativePrimaryPlayerProfile {
    /// Artifact digest.
    pub digest: String,
    /// Score storage, when declared.
    pub match_profile: Option<SourcePrimaryMatch>,
    /// Spawn selector entry RVA.
    pub spawn: u32,
    /// Objective-drop declaration.
    pub objectives: PlayerObjectives,
    /// Command angles offset within the client.
    pub command_angles: u32,
    /// Velocity offset within the entity.
    pub velocity: u32,
    /// Forward vector offset within the client, when stored.
    pub forward: Option<u32>,
}

/// Degrees-to-radians factor, matching the QVM donor.
const ANGLE_RADIANS: f32 = core::f32::consts::PI * 2.0 / 360.0;

/// QVM angle vectors forward lane: x pitch, y yaw, z roll, in degrees.
fn qvm_forward(angles: Vec3) -> Vec3 {
    let yaw = angles.y * ANGLE_RADIANS;
    let pitch = angles.x * ANGLE_RADIANS;
    let sy = yaw.sin();
    let cy = yaw.cos();
    let sp = pitch.sin();
    let cp = pitch.cos();
    Vec3 {
        x: cp * cy,
        y: cp * sy,
        z: -sp,
    }
}

/// Source spawn selection and private client pose over a synthetic host.
///
/// The caller owns the equipment's teleport effect, as in the donor.
pub struct NativePrimaryPlayer {
    weapon: NativePrimaryWeaponProfile,
    profile: NativePrimaryPlayerProfile,
}

impl NativePrimaryPlayer {
    /// Build the service, checking score bounds and profile digests.
    pub fn new(weapon: NativePrimaryWeaponProfile, profile: NativePrimaryPlayerProfile) -> HostResult<Self> {
        if let Some(match_profile) = &profile.match_profile {
            if match_profile.score % 4 != 0 || match_profile.score.saturating_add(4) > weapon.client.byte_length {
                return Err(NativeHostError::Fault(
                    "native score exceeds the declared client record".to_string(),
                ));
            }
        }
        if profile.digest != weapon.digest {
            return Err(NativeHostError::Fault(
                "native player service belongs to another executable".to_string(),
            ));
        }
        Ok(Self { weapon, profile })
    }

    /// Borrow the profile.
    #[must_use]
    pub fn profile(&self) -> &NativePrimaryPlayerProfile {
        &self.profile
    }

    fn check_host(&self, host: &SyntheticHost) -> HostResult<()> {
        if host.core.digest != self.profile.digest {
            return Err(NativeHostError::Fault(
                "native player service belongs to another executable".to_string(),
            ));
        }
        Ok(())
    }

    fn current(&self, host: &mut SyntheticHost, actor: NativeActorId) -> HostResult<(GuestAddress, GuestAddress)> {
        self.check_host(host)?;
        let entity = host
            .core
            .entity_of(actor)
            .map_err(|_| NativeHostError::Fault("native player service has no current actor".to_string()))?;
        let link = host.core.memory.offset(entity, i64::from(self.weapon.entity.client))?;
        let client = host.core.memory.read_pointer(link)?;
        match client {
            Some(client) => {
                host.core
                    .memory
                    .check(client, self.weapon.client.byte_length as usize, GuestAccess::Read)?;
                Ok((entity, client))
            }
            None => Err(NativeHostError::Fault(
                "native player service has no source client".to_string(),
            )),
        }
    }

    fn match_score(&self) -> HostResult<u32> {
        match &self.profile.match_profile {
            Some(match_profile) => Ok(match_profile.score),
            None => Err(NativeHostError::Fault(
                "original native score storage has no declaration".to_string(),
            )),
        }
    }

    /// Read the actor score.
    pub fn score(&self, host: &mut SyntheticHost, actor: NativeActorId) -> HostResult<i32> {
        let (_, client) = self.current(host, actor)?;
        let address = host.core.memory.offset(client, i64::from(self.match_score()?))?;
        Ok(host.core.memory.read_i32(address)?)
    }

    /// Write the actor score. Values must already be int32, mirroring
    /// the donor `sourceScore` gate.
    pub fn set_score(&self, host: &mut SyntheticHost, actor: NativeActorId, score: i64) -> HostResult<()> {
        let score = i32::try_from(score)
            .map_err(|_| NativeHostError::Fault("original score requires an int32 value".to_string()))?;
        let (_, client) = self.current(host, actor)?;
        let address = host.core.memory.offset(client, i64::from(self.match_score()?))?;
        Ok(host.core.memory.write_i32(address, score)?)
    }

    /// Read max health from the entity record.
    pub fn max_health(&self, host: &mut SyntheticHost, actor: NativeActorId) -> HostResult<i32> {
        let (entity, _) = self.current(host, actor)?;
        let address = host
            .core
            .memory
            .offset(entity, i64::from(self.weapon.entity.max_health.offset))?;
        Ok(host.core.memory.read_i32(address)?)
    }

    /// Write max health; requires a positive int32.
    pub fn set_max_health(&self, host: &mut SyntheticHost, actor: NativeActorId, value: i64) -> HostResult<()> {
        if value <= 0 || value > i64::from(i32::MAX) {
            return Err(NativeHostError::Fault(
                "native player max health requires a positive int32".to_string(),
            ));
        }
        let (entity, _) = self.current(host, actor)?;
        let address = host
            .core
            .memory
            .offset(entity, i64::from(self.weapon.entity.max_health.offset))?;
        Ok(host.core.memory.write_i32(address, value as i32)?)
    }

    /// Drop objectives through the declared entry, if any.
    pub fn drop_objectives(&self, host: &mut SyntheticHost, actor: NativeActorId) -> HostResult<()> {
        let (entity, _) = self.current(host, actor)?;
        let entry = match self.profile.objectives {
            PlayerObjectives::None => return Ok(()),
            PlayerObjectives::Entry(entry) => entry,
        };
        let address = host.core.at(entry)?;
        host.invoke(address, &[GuestCallValue::Pointer(Some(entity))])?;
        self.current(host, actor)?;
        Ok(())
    }

    /// Run the source spawn selector into scratch memory.
    pub fn spawn_point(&self, host: &mut SyntheticHost, actor: NativeActorId) -> HostResult<(Vec3, Vec3)> {
        let (entity, _) = self.current(host, actor)?;
        let mut options = GuestAllocationOptions::bytes(32);
        options.alignment = 8;
        options.label = "source player spawn result".to_string();
        let scratch = host.core.memory.allocate(&options)?;
        let outcome = (|| {
            host.core.memory.fill(scratch, 32, 0)?;
            let origin = scratch;
            let angles = host.core.memory.offset(scratch, 12)?;
            let landmark = host.core.memory.offset(scratch, 24)?;
            let target = host.core.at(self.profile.spawn)?;
            if matches!(self.weapon.abi, NativeAbi::WindowsI386) {
                host.invoke(
                    target,
                    &[
                        GuestCallValue::Pointer(Some(entity)),
                        GuestCallValue::Pointer(Some(origin)),
                        GuestCallValue::Pointer(Some(angles)),
                    ],
                )?;
            } else {
                let result = host.invoke(
                    target,
                    &[
                        GuestCallValue::Pointer(Some(entity)),
                        GuestCallValue::Pointer(Some(origin)),
                        GuestCallValue::Pointer(Some(angles)),
                        GuestCallValue::Uint32(1),
                        GuestCallValue::Pointer(Some(landmark)),
                    ],
                )?;
                match result {
                    GuestCallResult::Value(GuestCallValue::Uint32(value)) if value != 0 => {}
                    _ => {
                        return Err(NativeHostError::Fault(
                            "original native spawn selector could not place the player".to_string(),
                        ));
                    }
                }
            }
            self.current(host, actor)?;
            let mut vector = |address: GuestAddress| {
                let x = host.core.memory.read_f32(address)?;
                let y = host.core.memory.read_f32(host.core.memory.offset(address, 4)?)?;
                let z = host.core.memory.read_f32(host.core.memory.offset(address, 8)?)?;
                HostResult::Ok(Vec3 { x, y, z })
            };
            Ok((vector(origin)?, vector(angles)?))
        })();
        host.core.memory.unmap(scratch, 32)?;
        outcome
    }

    /// Teleport an actor: entity origin and velocity plus the private client
    /// move state, with classic fixed-point or rerelease float encoding.
    pub fn teleport(
        &self,
        host: &mut SyntheticHost,
        actor: NativeActorId,
        origin: Vec3,
        velocity: Vec3,
        angles: Vec3,
        hold_milliseconds: f64,
    ) -> HostResult<()> {
        let (entity, client) = self.current(host, actor)?;
        let classic = matches!(self.weapon.abi, NativeAbi::WindowsI386);
        let write_vector = |host: &mut SyntheticHost, address: GuestAddress, value: Vec3| {
            host.core.memory.write_f32(address, value.x)?;
            host.core
                .memory
                .write_f32(host.core.memory.offset(address, 4)?, value.y)?;
            host.core
                .memory
                .write_f32(host.core.memory.offset(address, 8)?, value.z)
                .map_err(NativeHostError::from)
        };
        let origin_link = host.core.memory.offset(entity, 4)?;
        write_vector(&mut *host, origin_link, origin)?;
        let old_origin = host.core.memory.offset(entity, 28)?;
        write_vector(&mut *host, old_origin, origin)?;
        let velocity_slot = host.core.memory.offset(entity, i64::from(self.profile.velocity))?;
        write_vector(&mut *host, velocity_slot, velocity)?;
        let wrap_i16 = |value: i32| (value & 0xFFFF) as u16 as i16;
        for (axis, value) in [origin.x, origin.y, origin.z].into_iter().enumerate() {
            if classic {
                let scaled = ((f64::from(value) * 8.0) as f32).trunc() as i32;
                let address = host.core.memory.offset(client, 4 + axis as i64 * 2)?;
                host.core.memory.write_i16(address, wrap_i16(scaled))?;
            } else {
                let address = host.core.memory.offset(client, 4 + axis as i64 * 4)?;
                host.core.memory.write_f32(address, value)?;
            }
        }
        for (axis, value) in [velocity.x, velocity.y, velocity.z].into_iter().enumerate() {
            if classic {
                let scaled = ((f64::from(value) * 8.0) as f32).trunc() as i32;
                let address = host.core.memory.offset(client, 10 + axis as i64 * 2)?;
                host.core.memory.write_i16(address, wrap_i16(scaled))?;
            } else {
                let address = host.core.memory.offset(client, 16 + axis as i64 * 4)?;
                host.core.memory.write_f32(address, value)?;
            }
        }
        if classic {
            let flags = host.core.memory.offset(client, 16)?;
            let bits = host.core.memory.read_u8(flags)?;
            host.core.memory.write_u8(flags, bits & !4 | 32)?;
            let hold = (hold_milliseconds / 8.0).ceil().clamp(0.0, 255.0) as u8;
            host.core.memory.write_u8(host.core.memory.offset(client, 17)?, hold)?;
        } else {
            let flags = host.core.memory.offset(client, 28)?;
            let bits = host.core.memory.read_u16(flags)?;
            host.core.memory.write_u16(flags, bits & !4 | 32)?;
            let hold = hold_milliseconds.clamp(0.0, 65535.0) as u16;
            host.core.memory.write_u16(host.core.memory.offset(client, 30)?, hold)?;
        }
        for (axis, value) in [angles.x, angles.y, angles.z].into_iter().enumerate() {
            let stored = host.core.memory.read_f32(
                host.core
                    .memory
                    .offset(client, i64::from(self.profile.command_angles) + axis as i64 * 4)?,
            )?;
            let delta = value - stored;
            if classic {
                let scaled = (f64::from(delta) * (65536.0 / 360.0)) as f32;
                let address = host.core.memory.offset(client, 20 + axis as i64 * 2)?;
                host.core.memory.write_i16(address, wrap_i16(scaled.trunc() as i32))?;
            } else {
                let address = host.core.memory.offset(client, 36 + axis as i64 * 4)?;
                host.core.memory.write_f32(address, delta)?;
            }
            let state = host
                .core
                .memory
                .offset(client, i64::from(if classic { 28 } else { 52 }) + axis as i64 * 4)?;
            host.core.memory.write_f32(state, value)?;
            let view = host
                .core
                .memory
                .offset(client, i64::from(self.weapon.client.view_angles) + axis as i64 * 4)?;
            host.core.memory.write_f32(view, value)?;
        }
        if let Some(forward) = self.profile.forward {
            let forward_slot = host.core.memory.offset(client, i64::from(forward))?;
            write_vector(&mut *host, forward_slot, qvm_forward(angles))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_primary_reader::{NativeItemField, NativeScalar, RecordKind, CLASSIC_DIGEST};
    use super::super::native_primary_weapons::{
        AttackAnimation, DelayEvaluate, EquipmentContext, SpawnGate, WeaponAnimation, WeaponClient, WeaponDamage,
        WeaponDelay, WeaponDispatcher, WeaponEntity, WeaponTime,
    };
    use super::*;

    fn field(offset: u32) -> NativeItemField {
        NativeItemField {
            record: RecordKind::Entity,
            offset,
            encoding: NativeScalar::Int32,
        }
    }

    fn weapon() -> NativePrimaryWeaponProfile {
        NativePrimaryWeaponProfile {
            digest: CLASSIC_DIGEST.to_string(),
            abi: NativeAbi::WindowsI386,
            dispatcher: WeaponDispatcher {
                entry_rva: 1,
                record: RecordKind::Entity,
                argument: 0,
                arguments: 1,
            },
            decisions: vec![],
            spawn: SpawnGate {
                entry: 2,
                accepted: vec![],
            },
            active: vec![],
            committed_input: vec![],
            continuations: vec![],
            time: WeaponTime {
                address: 3,
                encoding: NativeScalar::Float32,
                milliseconds: 1000.0,
            },
            entity: WeaponEntity {
                client: 84,
                water_level: field(20),
                view_height: field(24),
                max_health: field(28),
            },
            client: WeaponClient {
                byte_length: 512,
                view_angles: 100,
                buttons: field(8),
                latched_buttons: field(12),
            },
            attack_animation: AttackAnimation { entry: 4, skip: vec![] },
            animation: WeaponAnimation {
                frame: field(56),
                end: field(48),
                priority: field(52),
                duck: field(60),
                run: field(64),
            },
            equipment_contexts: Vec::<EquipmentContext>::new(),
            delay: WeaponDelay {
                flag: field(68),
                region: super::super::native_primary_reader::NativeRegion { entry: 5, join: 6 },
                evaluate: DelayEvaluate::SourceFlag { factors: vec![1.0] },
            },
            damage: WeaponDamage::SourceFlag {
                address: 7,
                encoding: NativeScalar::Int32,
                factors: vec![1.0],
                region: super::super::native_primary_reader::NativeRegion { entry: 8, join: 9 },
            },
        }
    }

    fn profile() -> NativePrimaryPlayerProfile {
        NativePrimaryPlayerProfile {
            digest: CLASSIC_DIGEST.to_string(),
            match_profile: Some(SourcePrimaryMatch {
                score: 200,
                teams: vec![],
            }),
            spawn: 0x100,
            objectives: PlayerObjectives::None,
            command_angles: 120,
            velocity: 40,
            forward: None,
        }
    }

    fn linked(host: &mut SyntheticHost) -> NativeActorId {
        let actor = host.core.spawn_actor(896, 512).expect("actor");
        let entity = host.core.entity_of(actor).expect("entity");
        let client = host.core.client_of(actor).expect("client");
        host.core.set_client(entity, 84, client).expect("link");
        actor
    }

    #[test]
    fn reads_and_writes_scores_and_health() {
        let mut host = SyntheticHost::synthetic(CLASSIC_DIGEST, 4, 0x2000).expect("host");
        let player = NativePrimaryPlayer::new(weapon(), profile()).expect("player");
        let actor = linked(&mut host);
        player.set_score(&mut host, actor, 41).expect("score");
        assert_eq!(player.score(&mut host, actor).expect("read"), 41);
        assert!(player.set_score(&mut host, actor, i64::from(i32::MAX) + 1).is_err());
        player.set_max_health(&mut host, actor, 200).expect("health");
        assert_eq!(player.max_health(&mut host, actor).expect("read"), 200);
        assert!(player.set_max_health(&mut host, actor, 0).is_err());
    }

    #[test]
    fn teleports_classic_client_state() {
        let mut host = SyntheticHost::synthetic(CLASSIC_DIGEST, 4, 0x2000).expect("host");
        let player = NativePrimaryPlayer::new(weapon(), profile()).expect("player");
        let actor = linked(&mut host);
        let origin = Vec3 {
            x: 8.0,
            y: -4.0,
            z: 1.0,
        };
        let velocity = Vec3 { x: 0.0, y: 0.0, z: 0.0 };
        let angles = Vec3 {
            x: 0.0,
            y: 90.0,
            z: 0.0,
        };
        player
            .teleport(&mut host, actor, origin, velocity, angles, 16.0)
            .expect("teleport");
        let entity = host.core.entity_of(actor).expect("entity");
        let x = host
            .core
            .memory
            .read_f32(host.core.memory.offset(entity, 4).expect("o"))
            .expect("x");
        assert_eq!(x, 8.0);
        let client = host.core.client_of(actor).expect("client").expect("client");
        let packed = host
            .core
            .memory
            .read_i16(host.core.memory.offset(client, 4).expect("p"))
            .expect("packed");
        assert_eq!(packed, 64);
        let flags = host
            .core
            .memory
            .read_u8(host.core.memory.offset(client, 16).expect("f"))
            .expect("flags");
        assert_eq!(flags & 32, 32);
        let hold = host
            .core
            .memory
            .read_u8(host.core.memory.offset(client, 17).expect("h"))
            .expect("hold");
        assert_eq!(hold, 2);
        let view = host
            .core
            .memory
            .read_f32(host.core.memory.offset(client, 104).expect("v"))
            .expect("view");
        assert_eq!(view, 90.0);
    }

    #[test]
    fn selects_spawn_points_through_originals() {
        let mut host = SyntheticHost::synthetic(CLASSIC_DIGEST, 4, 0x2000).expect("host");
        let player = NativePrimaryPlayer::new(weapon(), profile()).expect("player");
        let actor = linked(&mut host);
        host.on_rva(
            0x100,
            Box::new(|core, values| {
                let origin = match values[1] {
                    GuestCallValue::Pointer(Some(address)) => address,
                    _ => return Err(NativeHostError::Fault("no origin".to_string())),
                };
                core.memory.write_f32(origin, 1.0)?;
                core.memory.write_f32(core.memory.offset(origin, 4)?, 2.0)?;
                core.memory.write_f32(core.memory.offset(origin, 8)?, 3.0)?;
                Ok(GuestCallResult::Void)
            }),
        )
        .expect("handler");
        let (origin, _) = player.spawn_point(&mut host, actor).expect("spawn");
        assert_eq!(origin, Vec3 { x: 1.0, y: 2.0, z: 3.0 });
        player.drop_objectives(&mut host, actor).expect("none");
    }

    #[test]
    fn rejects_scores_outside_the_client() {
        let mut profile = profile();
        profile.match_profile = Some(SourcePrimaryMatch {
            score: 511,
            teams: vec![],
        });
        assert!(NativePrimaryPlayer::new(weapon(), profile).is_err());
    }
}
