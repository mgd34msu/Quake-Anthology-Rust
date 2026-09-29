//! Q2 rerelease published edict prefix accessors.
//!
//! Donor: `src/compat/q2/rerelease/public-state.ts` — bridges the API2023
//! published edict prefix (no `g_local.h` private members) into host reads.

use qa_core::math::Vec3;
use qa_guest::GuestError;
use qa_guest::core::contracts::{GuestAccess, GuestAddress, GuestLayout};
use qa_guest::core::memory::SparseGuestMemory;
use thiserror::Error;

use super::layouts::{client_layout, edict_layout, entity_state_layout, field_offset};

/// Public-state failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PublicStateError {
    /// API2023 source slot has no public client prefix.
    #[error("API2023 source slot has no public client prefix")]
    NoClient,
    /// Invalid API2023 ping.
    #[error("Invalid API2023 ping")]
    BadPing,
    /// Guest memory failure.
    #[error(transparent)]
    Guest(#[from] GuestError),
}

/// Body snapshot over the published prefix.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PublicBody {
    /// Origin.
    pub origin: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Minimum corner.
    pub min: Vec3,
    /// Maximum corner.
    pub max: Vec3,
}

/// Entity model presentation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelState {
    /// Model indexes.
    pub model_indexes: [i32; 4],
    /// Skin number.
    pub skin: i32,
}

/// Full entity state record.
#[derive(Debug, Clone, PartialEq)]
pub struct PublicEntityState {
    /// Entity number.
    pub number: u32,
    /// Origin.
    pub origin: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Old origin.
    pub old_origin: Vec3,
    /// Model indexes.
    pub model_indexes: [i32; 4],
    /// Frame.
    pub frame: i32,
    /// Skin.
    pub skin: i32,
    /// Effects.
    pub effects: u64,
    /// Render effects.
    pub render_effects: u32,
    /// Solid.
    pub solid: u32,
    /// Sound.
    pub sound: i32,
    /// Event.
    pub event: u8,
    /// Alpha.
    pub alpha: f32,
    /// Scale.
    pub scale: f32,
    /// Instance bits.
    pub instance_bits: u8,
    /// Loop volume.
    pub loop_volume: f32,
    /// Loop attenuation.
    pub loop_attenuation: f32,
    /// Owner.
    pub owner: i32,
    /// Old frame.
    pub old_frame: i32,
}

/// Player view snapshot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerView {
    /// View offset.
    pub view_offset: Vec3,
    /// View height.
    pub view_height: i8,
    /// Movement flags.
    pub movement_flags: u16,
}

/// API2023's published prefix only; no `g_local.h` private members.
pub struct RereleasePublicEdict<'m> {
    memory: &'m mut SparseGuestMemory,
    address: GuestAddress,
    edict: GuestLayout,
    entity: GuestLayout,
    client: GuestLayout,
    cached_body: Option<PublicBody>,
}

impl<'m> RereleasePublicEdict<'m> {
    /// Create over an edict record address.
    pub fn new(
        memory: &'m mut SparseGuestMemory,
        address: GuestAddress,
    ) -> Result<Self, PublicStateError> {
        let edict = edict_layout();
        memory.check(address, edict.byte_length, GuestAccess::Read)?;
        Ok(Self {
            memory,
            address,
            edict,
            entity: entity_state_layout(),
            client: client_layout(),
            cached_body: None,
        })
    }

    fn at(&self, name: &str) -> Result<GuestAddress, PublicStateError> {
        let offset = field_offset(&self.edict, name)
            .map(|offset| offset as i64)
            .map_err(|_| PublicStateError::NoClient)?;
        Ok(self.memory.offset(self.address, offset)?)
    }

    fn read_vec3(&mut self, address: GuestAddress) -> Result<Vec3, GuestError> {
        self.memory.read_f32x3(address)
    }

    /// Signed 32-bit field.
    pub fn int(&mut self, name: &str) -> Result<i32, PublicStateError> {
        let address = self.at(name)?;
        Ok(self.memory.read_i32(address)?)
    }

    /// Unsigned 32-bit field.
    pub fn uint(&mut self, name: &str) -> Result<u32, PublicStateError> {
        let address = self.at(name)?;
        Ok(self.memory.read_u32(address)?)
    }

    /// Byte field.
    pub fn byte(&mut self, name: &str) -> Result<u8, PublicStateError> {
        let address = self.at(name)?;
        Ok(self.memory.read_u8(address)?)
    }

    /// Float field.
    pub fn float(&mut self, name: &str) -> Result<f32, PublicStateError> {
        let address = self.at(name)?;
        Ok(self.memory.read_f32(address)?)
    }

    /// Pointer field.
    pub fn pointer(&mut self, name: &str) -> Result<Option<GuestAddress>, PublicStateError> {
        let address = self.at(name)?;
        Ok(self.memory.read_pointer(address)?)
    }

    /// Vector field.
    pub fn vector(&mut self, name: &str) -> Result<Vec3, PublicStateError> {
        let address = self.at(name)?;
        Ok(self.read_vec3(address)?)
    }

    /// Write a vector field.
    pub fn set_vector(&mut self, name: &str, value: Vec3) -> Result<(), PublicStateError> {
        let address = self.at(name)?;
        self.memory.write_f32(address, value.x)?;
        self.memory
            .write_f32(self.memory.offset(address, 4)?, value.y)?;
        self.memory
            .write_f32(self.memory.offset(address, 8)?, value.z)?;
        Ok(())
    }

    /// Body snapshot, reusing the cached record when unchanged.
    pub fn body(
        &mut self,
        velocity: Option<Vec3>,
        origin: Option<Vec3>,
    ) -> Result<PublicBody, PublicStateError> {
        let next_origin = match origin {
            Some(origin) => origin,
            None => self.vector("s.origin")?,
        };
        let angles = self.vector("s.angles")?;
        let next_velocity = match velocity {
            Some(velocity) => velocity,
            None => self.vector("sv.velocity")?,
        };
        let min = self.vector("mins")?;
        let max = self.vector("maxs")?;
        if let Some(previous) = self.cached_body {
            if previous.origin == next_origin
                && previous.angles == angles
                && previous.velocity == next_velocity
                && previous.min == min
                && previous.max == max
            {
                return Ok(previous);
            }
        }
        let next = PublicBody {
            origin: next_origin,
            angles,
            velocity: next_velocity,
            min,
            max,
        };
        self.cached_body = Some(next);
        Ok(next)
    }

    /// Client record address.
    pub fn client(&mut self) -> Result<GuestAddress, PublicStateError> {
        let address = self.pointer("client")?.ok_or(PublicStateError::NoClient)?;
        self.memory
            .check(address, self.client.byte_length, GuestAccess::Read)?;
        Ok(address)
    }

    fn client_field(&self, name: &str) -> Result<i64, PublicStateError> {
        field_offset(&self.client, name)
            .map(|offset| offset as i64)
            .map_err(|_| PublicStateError::NoClient)
    }

    /// Raw player-state bytes for the owning player module.
    pub fn player_state_bytes(&mut self) -> Result<Vec<u8>, PublicStateError> {
        let client = self.client()?;
        let length =
            field_offset(&self.client, "ping").map_err(|_| PublicStateError::NoClient)?;
        Ok(self.memory.copy(client, length)?)
    }

    /// Player velocity from the client movement state.
    pub fn player_velocity(&mut self) -> Result<Vec3, PublicStateError> {
        let client = self.client()?;
        let offset = self.client_field("ps.pmove.velocity")?;
        Ok(self.memory.read_f32x3(self.memory.offset(client, offset)?)?)
    }

    /// Player movement flags.
    pub fn player_movement_flags(&mut self) -> Result<u16, PublicStateError> {
        let client = self.client()?;
        let offset = self.client_field("ps.pmove.pm_flags")?;
        Ok(self.memory.read_u16(self.memory.offset(client, offset)?)?)
    }

    /// Player view snapshot.
    pub fn player_view(&mut self) -> Result<PlayerView, PublicStateError> {
        let client = self.client()?;
        let flags = self.client_field("ps.pmove.pm_flags")?;
        let height = self.client_field("ps.pmove.viewheight")?;
        let offset = self.client_field("ps.viewoffset")?;
        Ok(PlayerView {
            view_offset: self
                .memory
                .read_f32x3(self.memory.offset(client, offset)?)?,
            view_height: self.memory.read_i8(self.memory.offset(client, height)?)?,
            movement_flags: self.memory.read_u16(self.memory.offset(client, flags)?)?,
        })
    }

    /// Player team id.
    pub fn player_team_id(&mut self) -> Result<u8, PublicStateError> {
        let client = self.client()?;
        let offset = self.client_field("ps.team_id")?;
        Ok(self.memory.read_u8(self.memory.offset(client, offset)?)?)
    }

    /// Model presentation snapshot.
    pub fn model_state(&mut self) -> Result<ModelState, PublicStateError> {
        let base = self.at("s.modelindex")?;
        let model2 = self.entity_field("modelindex2")? - self.entity_field("modelindex")?;
        let model3 = self.entity_field("modelindex3")? - self.entity_field("modelindex")?;
        let model4 = self.entity_field("modelindex4")? - self.entity_field("modelindex")?;
        let skin = self.entity_field("skinnum")? - self.entity_field("modelindex")?;
        Ok(ModelState {
            model_indexes: [
                self.memory.read_i32(base)?,
                self.memory.read_i32(self.memory.offset(base, model2)?)?,
                self.memory.read_i32(self.memory.offset(base, model3)?)?,
                self.memory.read_i32(self.memory.offset(base, model4)?)?,
            ],
            skin: self.memory.read_i32(self.memory.offset(base, skin)?)?,
        })
    }

    fn entity_field(&self, name: &str) -> Result<i64, PublicStateError> {
        field_offset(&self.entity, name)
            .map(|offset| offset as i64)
            .map_err(|_| PublicStateError::NoClient)
    }

    /// Client ping.
    pub fn ping(&mut self) -> Result<i32, PublicStateError> {
        let client = self.client()?;
        let offset = self.client_field("ping")?;
        Ok(self.memory.read_i32(self.memory.offset(client, offset)?)?)
    }

    /// Set the client ping.
    pub fn set_ping(&mut self, value: i32) -> Result<(), PublicStateError> {
        if value < 0 {
            return Err(PublicStateError::BadPing);
        }
        let client = self.client()?;
        let offset = self.client_field("ping")?;
        self.memory
            .write_i32(self.memory.offset(client, offset)?, value)?;
        Ok(())
    }

    /// Full entity state record.
    pub fn state(&mut self) -> Result<PublicEntityState, PublicStateError> {
        let base = self.at("s.number")?;
        let origin = self.entity_field("origin")?;
        let angles = self.entity_field("angles")?;
        let old_origin = self.entity_field("old_origin")?;
        let model = self.entity_field("modelindex")?;
        let model2 = self.entity_field("modelindex2")?;
        let model3 = self.entity_field("modelindex3")?;
        let model4 = self.entity_field("modelindex4")?;
        let frame = self.entity_field("frame")?;
        let skin = self.entity_field("skinnum")?;
        let effects = self.entity_field("effects")?;
        let renderfx = self.entity_field("renderfx")?;
        let solid = self.entity_field("solid")?;
        let sound = self.entity_field("sound")?;
        let event = self.entity_field("event")?;
        let alpha = self.entity_field("alpha")?;
        let scale = self.entity_field("scale")?;
        let instance_bits = self.entity_field("instance_bits")?;
        let loop_volume = self.entity_field("loop_volume")?;
        let loop_attenuation = self.entity_field("loop_attenuation")?;
        let owner = self.entity_field("owner")?;
        let old_frame = self.entity_field("old_frame")?;
        let memory = &mut *self.memory;
        let vec_at = |memory: &mut SparseGuestMemory, offset: i64| {
            memory
                .offset(base, offset)
                .and_then(|at| memory.read_f32x3(at))
                .map_err(PublicStateError::from)
        };
        let i32_at = |memory: &mut SparseGuestMemory, offset: i64| {
            memory
                .offset(base, offset)
                .and_then(|at| memory.read_i32(at))
                .map_err(PublicStateError::from)
        };
        Ok(PublicEntityState {
            number: memory.read_u32(base)?,
            origin: vec_at(memory, origin)?,
            angles: vec_at(memory, angles)?,
            old_origin: vec_at(memory, old_origin)?,
            model_indexes: [
                i32_at(memory, model)?,
                i32_at(memory, model2)?,
                i32_at(memory, model3)?,
                i32_at(memory, model4)?,
            ],
            frame: i32_at(memory, frame)?,
            skin: i32_at(memory, skin)?,
            effects: memory.read_u64(memory.offset(base, effects)?)?,
            render_effects: memory.read_u32(memory.offset(base, renderfx)?)?,
            solid: memory.read_u32(memory.offset(base, solid)?)?,
            sound: i32_at(memory, sound)?,
            event: memory.read_u8(memory.offset(base, event)?)?,
            alpha: memory.read_f32(memory.offset(base, alpha)?)?,
            scale: memory.read_f32(memory.offset(base, scale)?)?,
            instance_bits: memory.read_u8(memory.offset(base, instance_bits)?)?,
            loop_volume: memory.read_f32(memory.offset(base, loop_volume)?)?,
            loop_attenuation: memory.read_f32(memory.offset(base, loop_attenuation)?)?,
            owner: i32_at(memory, owner)?,
            old_frame: i32_at(memory, old_frame)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::ProviderId;
    use qa_guest::core::contracts::{ContentDigest, GuestAllocationOptions, ModuleIdentity};

    fn test_memory() -> SparseGuestMemory {
        let module = ModuleIdentity::new(
            ProviderId::new("q2", "public-state-test"),
            "game.dll",
            ContentDigest::new("sha256", "00"),
            "test",
        );
        SparseGuestMemory::new(module, 8, 0x1_0000).expect("memory")
    }

    fn edict_block(memory: &mut SparseGuestMemory) -> GuestAddress {
        let layout = edict_layout();
        memory
            .allocate(&GuestAllocationOptions::bytes(layout.byte_length))
            .expect("alloc")
    }

    #[test]
    fn scalars_vectors_and_body_cache() {
        let mut memory = test_memory();
        let address = edict_block(&mut memory);
        let mut edict = RereleasePublicEdict::new(&mut memory, address).expect("edict");
        edict
            .set_vector(
                "s.origin",
                Vec3 {
                    x: 1.0,
                    y: 2.0,
                    z: 3.0,
                },
            )
            .expect("origin");
        assert_eq!(
            edict.vector("s.origin").expect("read"),
            Vec3 {
                x: 1.0,
                y: 2.0,
                z: 3.0
            }
        );
        let first = edict.body(None, None).expect("body");
        let second = edict.body(None, None).expect("cached");
        assert_eq!(first, second);
        let moved = edict
            .body(
                None,
                Some(Vec3 {
                    x: 9.0,
                    y: 9.0,
                    z: 9.0,
                }),
            )
            .expect("override");
        assert_eq!(moved.origin.x, 9.0);
        assert_eq!(edict.uint("svflags").expect("flags"), 0);
        assert_eq!(edict.byte("inuse").expect("inuse"), 0);
    }

    #[test]
    fn client_ping_view_and_entity_state() {
        let mut memory = test_memory();
        let address = edict_block(&mut memory);
        let client_layout = client_layout();
        let client = memory
            .allocate(&GuestAllocationOptions::bytes(client_layout.byte_length))
            .expect("alloc");
        let edict_layout = edict_layout();
        let client_field = memory
            .offset(
                address,
                field_offset(&edict_layout, "client").expect("c") as i64,
            )
            .expect("o");
        memory.write_pointer(client_field, Some(client)).expect("link");
        let mut edict = RereleasePublicEdict::new(&mut memory, address).expect("edict");
        edict.set_ping(72).expect("ping");
        assert_eq!(edict.ping().expect("read"), 72);
        assert!(edict.set_ping(-1).is_err());
        let view = edict.player_view().expect("view");
        assert_eq!(view.movement_flags, 0);
        assert_eq!(edict.player_team_id().expect("team"), 0);
        let models = edict.model_state().expect("models");
        assert_eq!(models.model_indexes, [0, 0, 0, 0]);
        let state = edict.state().expect("state");
        assert_eq!(state.number, 0);
        assert_eq!(
            state.origin,
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0
            }
        );
        let blob = edict.player_state_bytes().expect("blob");
        assert_eq!(blob.len(), 296);
    }
}
