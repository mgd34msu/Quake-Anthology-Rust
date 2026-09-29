//! Q2 rerelease sound import events.
//!
//! Donor: `src/compat/q2/rerelease/sounds.ts` — bridges the `game.h`
//! `sound` / `positioned_sound` / `local_sound` imports into audio events.

use qa_core::math::Vec3;
use qa_guest::GuestError;
use qa_guest::core::contracts::{GuestAddress, GuestCallResult, GuestCallValue};
use qa_guest::core::memory::SparseGuestMemory;
use thiserror::Error;

/// Sound import failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SoundError {
    /// Sound argument requires a source float.
    #[error("Q2 sound requires a source float")]
    NonFloat,
    /// Guest memory failure.
    #[error(transparent)]
    Guest(#[from] GuestError),
}

/// Sound audience: world broadcast or one client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundAudience {
    /// Audible in the world.
    World,
    /// Audible to one client slot with a duplicate key.
    Client {
        /// Client slot.
        client_slot: u32,
        /// Duplicate key.
        dupe_key: u32,
    },
}

/// One `game.h` sound request with preserved arguments.
#[derive(Debug, Clone, PartialEq)]
pub struct RereleaseSoundEvent {
    /// Emitting entity slot, if any.
    pub entity_slot: Option<u32>,
    /// Emission origin, if positioned.
    pub origin: Option<Vec3>,
    /// Channel.
    pub channel: i32,
    /// Sound index.
    pub sound_index: i32,
    /// Volume.
    pub volume: f32,
    /// Attenuation.
    pub attenuation: f32,
    /// Time offset.
    pub time_offset: f32,
    /// Audience.
    pub audience: SoundAudience,
}

/// `sound` / `positioned_sound` / `local_sound` import dispatch.
pub struct RereleaseSoundImports {
    /// Emitted events in dispatch order.
    pub events: Vec<RereleaseSoundEvent>,
}

impl RereleaseSoundImports {
    /// Create an empty dispatcher.
    #[must_use]
    pub fn new() -> Self {
        Self { events: Vec::new() }
    }

    /// Dispatch one import; returns `None` for other APIs/names.
    pub fn invoke(
        &mut self,
        memory: &mut SparseGuestMemory,
        api: &str,
        name: &str,
        args: &[GuestCallValue],
        source_slot: &dyn Fn(GuestAddress) -> u32,
    ) -> Option<Result<GuestCallResult, SoundError>> {
        if api != "game"
            || (name != "sound" && name != "positioned_sound" && name != "local_sound")
        {
            return None;
        }
        Some(self.dispatch(memory, name, args, source_slot))
    }

    fn dispatch(
        &mut self,
        memory: &mut SparseGuestMemory,
        name: &str,
        args: &[GuestCallValue],
        source_slot: &dyn Fn(GuestAddress) -> u32,
    ) -> Result<GuestCallResult, SoundError> {
        let offset = match name {
            "sound" => 0,
            "positioned_sound" => 1,
            _ => 2,
        };
        let pointer = |index: usize| -> Option<GuestAddress> {
            match args.get(index) {
                Some(GuestCallValue::Pointer(address)) => *address,
                _ => None,
            }
        };
        let integer = |index: usize| -> i32 {
            match args.get(index) {
                Some(GuestCallValue::Int32(value)) => *value,
                Some(GuestCallValue::Uint32(value)) => *value as i32,
                Some(GuestCallValue::Int64(value)) => *value as i32,
                Some(GuestCallValue::Uint64(value)) => *value as i32,
                _ => 0,
            }
        };
        let float = |index: usize| -> Result<f32, SoundError> {
            match args.get(index) {
                Some(GuestCallValue::Float32(value)) => Ok(*value),
                _ => Err(SoundError::NonFloat),
            }
        };
        let entity = pointer(offset);
        let origin_address = match name {
            "sound" => None,
            "positioned_sound" => pointer(0),
            _ => pointer(1),
        };
        let origin = match origin_address {
            None => None,
            Some(address) => Some(memory.read_f32x3(address)?),
        };
        let audience = if name == "local_sound" {
            match pointer(0) {
                Some(client) => SoundAudience::Client {
                    client_slot: source_slot(client),
                    dupe_key: integer(8) as u32,
                },
                None => SoundAudience::World,
            }
        } else {
            SoundAudience::World
        };
        self.events.push(RereleaseSoundEvent {
            entity_slot: entity.map(source_slot),
            origin,
            channel: integer(offset + 1),
            sound_index: integer(offset + 2),
            volume: float(offset + 3)?,
            attenuation: float(offset + 4)?,
            time_offset: float(offset + 5)?,
            audience,
        });
        Ok(GuestCallResult::Void)
    }
}

impl Default for RereleaseSoundImports {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::ProviderId;
    use qa_guest::core::contracts::{ContentDigest, GuestAllocationOptions, ModuleIdentity};

    fn test_memory() -> SparseGuestMemory {
        let module = ModuleIdentity::new(
            ProviderId::new("q2", "sounds-test"),
            "game.dll",
            ContentDigest::new("sha256", "00"),
            "test",
        );
        SparseGuestMemory::new(module, 8, 0x1_0000).expect("memory")
    }

    fn float(value: f32) -> GuestCallValue {
        GuestCallValue::Float32(value)
    }

    #[test]
    fn world_and_positioned_sounds_emit() {
        let mut memory = test_memory();
        let mut imports = RereleaseSoundImports::new();
        let slot_of = |address: GuestAddress| {
            if address.offset == 0x200 {
                3
            } else {
                9
            }
        };
        let entity = GuestAddress::new(memory.address_space(), 0x200);
        imports
            .invoke(
                &mut memory,
                "game",
                "sound",
                &[
                    GuestCallValue::Pointer(Some(entity)),
                    GuestCallValue::Int32(1),
                    GuestCallValue::Int32(7),
                    float(1.0),
                    float(0.5),
                    float(0.0),
                ],
                &slot_of,
            )
            .unwrap()
            .expect("sound");
        assert_eq!(imports.events.len(), 1);
        let first = &imports.events[0];
        assert_eq!(first.entity_slot, Some(3));
        assert_eq!(first.origin, None);
        assert_eq!(first.channel, 1);
        assert_eq!(first.sound_index, 7);
        assert_eq!(first.audience, SoundAudience::World);
        let origin = memory
            .allocate(&GuestAllocationOptions::bytes(12))
            .expect("alloc");
        memory
            .write_f32(memory.offset(origin, 8).expect("o"), 4.0)
            .expect("z");
        imports
            .invoke(
                &mut memory,
                "game",
                "positioned_sound",
                &[
                    GuestCallValue::Pointer(Some(origin)),
                    GuestCallValue::Pointer(None),
                    GuestCallValue::Int32(2),
                    GuestCallValue::Int32(8),
                    float(0.8),
                    float(1.0),
                    float(0.25),
                ],
                &slot_of,
            )
            .unwrap()
            .expect("positioned");
        let second = &imports.events[1];
        assert_eq!(second.entity_slot, None);
        assert_eq!(
            second.origin,
            Some(Vec3 {
                x: 0.0,
                y: 0.0,
                z: 4.0
            })
        );
        assert_eq!(second.time_offset, 0.25);
    }

    #[test]
    fn local_sound_targets_a_client() {
        let mut memory = test_memory();
        let mut imports = RereleaseSoundImports::new();
        let slot_of = |address: GuestAddress| {
            if address.offset == 0x300 {
                2
            } else {
                5
            }
        };
        let client = GuestAddress::new(memory.address_space(), 0x300);
        let entity = GuestAddress::new(memory.address_space(), 0x400);
        imports
            .invoke(
                &mut memory,
                "game",
                "local_sound",
                &[
                    GuestCallValue::Pointer(Some(client)),
                    GuestCallValue::Pointer(None),
                    GuestCallValue::Pointer(Some(entity)),
                    GuestCallValue::Int32(0),
                    GuestCallValue::Int32(3),
                    float(1.0),
                    float(0.0),
                    float(0.0),
                    GuestCallValue::Uint32(12),
                ],
                &slot_of,
            )
            .unwrap()
            .expect("local");
        assert_eq!(imports.events.len(), 1);
        assert_eq!(
            imports.events[0].audience,
            SoundAudience::Client {
                client_slot: 2,
                dupe_key: 12
            }
        );
        assert_eq!(imports.events[0].entity_slot, Some(5));
        assert!(
            imports
                .invoke(&mut memory, "game", "Com_Print", &[], &slot_of)
                .is_none()
        );
        let bad = imports
            .invoke(
                &mut memory,
                "game",
                "sound",
                &[
                    GuestCallValue::Pointer(None),
                    GuestCallValue::Int32(0),
                    GuestCallValue::Int32(0),
                    GuestCallValue::Int32(0),
                    float(0.0),
                    float(0.0),
                ],
                &slot_of,
            )
            .unwrap()
            .unwrap_err();
        assert_eq!(bad, SoundError::NonFloat);
    }
}
