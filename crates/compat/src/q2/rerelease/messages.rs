//! Q2 rerelease network message writers, unicast and multicast.
//!
//! Donor: `src/compat/q2/rerelease/messages.ts` — bridges the `game.h`
//! `Write*` imports and `q2repro` `PF_Unicast` into transport events.

use qa_core::math::Vec3;
use qa_guest::GuestError;
use qa_guest::core::contracts::{GuestAddress, GuestCallResult, GuestCallValue};
use qa_guest::core::memory::SparseGuestMemory;
use thiserror::Error;

/// Message import failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MessageError {
    /// Writer requires a source float argument.
    #[error("Q2 message writer requires a source float")]
    NonFloat,
    /// Message string exceeds capacity.
    #[error("Q2 message string exceeds message capacity")]
    StringTooLong,
    /// Buffer overflowed before delivery.
    #[error("Q2 message buffer overflowed")]
    Overflowed,
    /// Unknown multicast destination.
    #[error("Unknown Q2 multicast destination")]
    BadDestination,
    /// Spatial multicast requires an origin.
    #[error("Q2 spatial multicast requires an origin")]
    MissingOrigin,
    /// Guest memory failure.
    #[error(transparent)]
    Guest(#[from] GuestError),
}

/// Growable message assembly buffer (`sizebuf_t`).
#[derive(Debug, Clone)]
pub struct SizeBuf {
    /// Assembled bytes.
    pub data: Vec<u8>,
    /// Committed length.
    pub cursize: usize,
    /// Capacity.
    pub maxsize: usize,
    /// Overflow latch.
    pub overflowed: bool,
    /// Whether overflow clears instead of failing.
    pub allow_overflow: bool,
}

impl SizeBuf {
    /// New buffer with capacity.
    #[must_use]
    pub fn new(maxsize: usize) -> Self {
        Self {
            data: vec![0; maxsize],
            cursize: 0,
            maxsize,
            overflowed: false,
            allow_overflow: false,
        }
    }

    /// Committed bytes.
    #[must_use]
    pub fn committed(&self) -> &[u8] {
        &self.data[..self.cursize]
    }

    /// Clear committed bytes and the overflow latch.
    pub fn clear(&mut self) {
        self.cursize = 0;
        self.overflowed = false;
    }

    fn space(&mut self, length: usize) -> Result<usize, MessageError> {
        if self.cursize + length > self.maxsize {
            if !self.allow_overflow {
                return Err(MessageError::Overflowed);
            }
            if length > self.maxsize {
                return Err(MessageError::Overflowed);
            }
            self.clear();
            self.overflowed = true;
        }
        let offset = self.cursize;
        self.cursize += length;
        Ok(offset)
    }

    /// Write raw bytes.
    pub fn sz_write(&mut self, bytes: &[u8]) -> Result<(), MessageError> {
        let offset = self.space(bytes.len())?;
        self.data[offset..offset + bytes.len()].copy_from_slice(bytes);
        Ok(())
    }

    /// Write one masked byte.
    pub fn write_byte(&mut self, value: i32) -> Result<(), MessageError> {
        let offset = self.space(1)?;
        self.data[offset] = value as u8;
        Ok(())
    }

    /// Write a little-endian short.
    pub fn write_short(&mut self, value: i32) -> Result<(), MessageError> {
        let offset = self.space(2)?;
        self.data[offset] = value as u8;
        self.data[offset + 1] = (value >> 8) as u8;
        Ok(())
    }

    /// Write a little-endian long.
    pub fn write_long(&mut self, value: i32) -> Result<(), MessageError> {
        let offset = self.space(4)?;
        for (index, byte) in self.data[offset..offset + 4].iter_mut().enumerate() {
            *byte = (value >> (index * 8)) as u8;
        }
        Ok(())
    }

    /// Write a little-endian float.
    pub fn write_float(&mut self, value: f32) -> Result<(), MessageError> {
        let offset = self.space(4)?;
        self.data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }

    /// Write the best-fit direction byte from the 162-vector table.
    pub fn write_dir(&mut self, direction: Vec3) -> Result<(), MessageError> {
        let mut best = 0u8;
        let mut best_dot = 0.0f32;
        for (index, candidate) in BYTEDIRS.iter().enumerate() {
            let dot =
                direction.x * candidate[0] + direction.y * candidate[1] + direction.z * candidate[2];
            if dot > best_dot {
                best_dot = dot;
                best = index as u8;
            }
        }
        self.write_byte(i32::from(best))
    }
}

/// Quake 2 vertex-normal table (`anorms.ts`, 162 entries).
#[rustfmt::skip]
pub const BYTEDIRS: [[f32; 3]; 162] = [
    [-0.525731, 0.000000, 0.850651], [-0.442863, 0.238856, 0.864188], [-0.295242, 0.000000, 0.955423],
    [-0.309017, 0.500000, 0.809017], [-0.162460, 0.262866, 0.951056], [0.000000, 0.000000, 1.000000],
    [0.000000, 0.850651, 0.525731], [-0.147621, 0.716567, 0.681718], [0.147621, 0.716567, 0.681718],
    [0.000000, 0.525731, 0.850651], [0.309017, 0.500000, 0.809017], [0.525731, 0.000000, 0.850651],
    [0.295242, 0.000000, 0.955423], [0.442863, 0.238856, 0.864188], [0.162460, 0.262866, 0.951056],
    [-0.681718, 0.147621, 0.716567], [-0.809017, 0.309017, 0.500000], [-0.587785, 0.425325, 0.688191],
    [-0.850651, 0.525731, 0.000000], [-0.864188, 0.442863, 0.238856], [-0.716567, 0.681718, 0.147621],
    [-0.688191, 0.587785, 0.425325], [-0.500000, 0.809017, 0.309017], [-0.238856, 0.864188, 0.442863],
    [-0.425325, 0.688191, 0.587785], [-0.716567, 0.681718, -0.147621], [-0.500000, 0.809017, -0.309017],
    [-0.525731, 0.850651, 0.000000], [0.000000, 0.850651, -0.525731], [-0.238856, 0.864188, -0.442863],
    [0.000000, 0.955423, -0.295242], [-0.262866, 0.951056, -0.162460], [0.000000, 1.000000, 0.000000],
    [0.000000, 0.955423, 0.295242], [-0.262866, 0.951056, 0.162460], [0.238856, 0.864188, 0.442863],
    [0.262866, 0.951056, 0.162460], [0.500000, 0.809017, 0.309017], [0.238856, 0.864188, -0.442863],
    [0.262866, 0.951056, -0.162460], [0.500000, 0.809017, -0.309017], [0.850651, 0.525731, 0.000000],
    [0.716567, 0.681718, 0.147621], [0.716567, 0.681718, -0.147621], [0.525731, 0.850651, 0.000000],
    [0.425325, 0.688191, 0.587785], [0.864188, 0.442863, 0.238856], [0.688191, 0.587785, 0.425325],
    [0.809017, 0.309017, 0.500000], [0.681718, 0.147621, 0.716567], [0.587785, 0.425325, 0.688191],
    [0.955423, 0.295242, 0.000000], [1.000000, 0.000000, 0.000000], [0.951056, 0.162460, 0.262866],
    [0.850651, -0.525731, 0.000000], [0.955423, -0.295242, 0.000000], [0.864188, -0.442863, 0.238856],
    [0.951056, -0.162460, 0.262866], [0.809017, -0.309017, 0.500000], [0.681718, -0.147621, 0.716567],
    [0.850651, 0.000000, 0.525731], [0.864188, 0.442863, -0.238856], [0.809017, 0.309017, -0.500000],
    [0.951056, 0.162460, -0.262866], [0.525731, 0.000000, -0.850651], [0.681718, 0.147621, -0.716567],
    [0.681718, -0.147621, -0.716567], [0.850651, 0.000000, -0.525731], [0.809017, -0.309017, -0.500000],
    [0.864188, -0.442863, -0.238856], [0.951056, -0.162460, -0.262866], [0.147621, 0.716567, -0.681718],
    [0.309017, 0.500000, -0.809017], [0.425325, 0.688191, -0.587785], [0.442863, 0.238856, -0.864188],
    [0.587785, 0.425325, -0.688191], [0.688191, 0.587785, -0.425325], [-0.147621, 0.716567, -0.681718],
    [-0.309017, 0.500000, -0.809017], [0.000000, 0.525731, -0.850651], [-0.525731, 0.000000, -0.850651],
    [-0.442863, 0.238856, -0.864188], [-0.295242, 0.000000, -0.955423], [-0.162460, 0.262866, -0.951056],
    [0.000000, 0.000000, -1.000000], [0.295242, 0.000000, -0.955423], [0.162460, 0.262866, -0.951056],
    [-0.442863, -0.238856, -0.864188], [-0.309017, -0.500000, -0.809017], [-0.162460, -0.262866, -0.951056],
    [0.000000, -0.850651, -0.525731], [-0.147621, -0.716567, -0.681718], [0.147621, -0.716567, -0.681718],
    [0.000000, -0.525731, -0.850651], [0.309017, -0.500000, -0.809017], [0.442863, -0.238856, -0.864188],
    [0.162460, -0.262866, -0.951056], [0.238856, -0.864188, -0.442863], [0.500000, -0.809017, -0.309017],
    [0.425325, -0.688191, -0.587785], [0.716567, -0.681718, -0.147621], [0.688191, -0.587785, -0.425325],
    [0.587785, -0.425325, -0.688191], [0.000000, -0.955423, -0.295242], [0.000000, -1.000000, 0.000000],
    [0.262866, -0.951056, -0.162460], [0.000000, -0.850651, 0.525731], [0.000000, -0.955423, 0.295242],
    [0.238856, -0.864188, 0.442863], [0.262866, -0.951056, 0.162460], [0.500000, -0.809017, 0.309017],
    [0.716567, -0.681718, 0.147621], [0.525731, -0.850651, 0.000000], [-0.238856, -0.864188, -0.442863],
    [-0.500000, -0.809017, -0.309017], [-0.262866, -0.951056, -0.162460], [-0.850651, -0.525731, 0.000000],
    [-0.716567, -0.681718, -0.147621], [-0.716567, -0.681718, 0.147621], [-0.525731, -0.850651, 0.000000],
    [-0.500000, -0.809017, 0.309017], [-0.238856, -0.864188, 0.442863], [-0.262866, -0.951056, 0.162460],
    [-0.864188, -0.442863, 0.238856], [-0.809017, -0.309017, 0.500000], [-0.688191, -0.587785, 0.425325],
    [-0.681718, -0.147621, 0.716567], [-0.442863, -0.238856, 0.864188], [-0.587785, -0.425325, 0.688191],
    [-0.309017, -0.500000, 0.809017], [-0.147621, -0.716567, 0.681718], [-0.425325, -0.688191, 0.587785],
    [-0.162460, -0.262866, 0.951056], [0.442863, -0.238856, 0.864188], [0.162460, -0.262866, 0.951056],
    [0.309017, -0.500000, 0.809017], [0.147621, -0.716567, 0.681718], [0.000000, -0.525731, 0.850651],
    [0.425325, -0.688191, 0.587785], [0.587785, -0.425325, 0.688191], [0.688191, -0.587785, 0.425325],
    [-0.955423, 0.295242, 0.000000], [-0.951056, 0.162460, 0.262866], [-1.000000, 0.000000, 0.000000],
    [-0.850651, 0.000000, 0.525731], [-0.955423, -0.295242, 0.000000], [-0.951056, -0.162460, 0.262866],
    [-0.864188, 0.442863, -0.238856], [-0.951056, 0.162460, -0.262866], [-0.809017, 0.309017, -0.500000],
    [-0.864188, -0.442863, -0.238856], [-0.951056, -0.162460, -0.262866], [-0.809017, -0.309017, -0.500000],
    [-0.681718, 0.147621, -0.716567], [-0.681718, -0.147621, -0.716567], [-0.850651, 0.000000, -0.525731],
    [-0.688191, 0.587785, -0.425325], [-0.587785, 0.425325, -0.688191], [-0.425325, 0.688191, -0.587785],
    [-0.425325, -0.688191, -0.587785], [-0.587785, -0.425325, -0.688191], [-0.688191, -0.587785, -0.425325],
];

/// Client-bound datagram: slot, reliability, duplicate key, bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RereleaseUnicast {
    /// Source client slot.
    pub client_slot: u32,
    /// Reliable channel.
    pub reliable: bool,
    /// Duplicate key.
    pub dupe_key: u32,
    /// Payload bytes.
    pub bytes: Vec<u8>,
}

/// Broadcast datagram: origin, destination, reliability, bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct RereleaseMulticast {
    /// Origin, or `None` for global.
    pub origin: Option<Vec3>,
    /// Destination set.
    pub destination: MulticastDestination,
    /// Reliable channel.
    pub reliable: bool,
    /// Payload bytes.
    pub bytes: Vec<u8>,
}

/// Multicast destination set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MulticastDestination {
    /// All clients.
    All,
    /// Potentially hearable set.
    Phs,
    /// Potentially visible set.
    Pvs,
}

/// Transport services behind the message imports.
pub trait RereleaseMessageServices {
    /// Shared assembly buffer.
    fn buffer_mut(&mut self) -> &mut SizeBuf;
    /// Whether a source slot accepts client traffic.
    fn accepts_client(&self, slot: u32) -> bool;
    /// Deliver a unicast datagram.
    fn unicast(&mut self, message: RereleaseUnicast);
    /// Deliver a multicast datagram.
    fn multicast(&mut self, message: RereleaseMulticast);
}

/// `game.h` writers plus unicast/multicast dispatch.
pub struct RereleaseMessageImports<S> {
    /// Transport services.
    pub services: S,
}

impl<S: RereleaseMessageServices> RereleaseMessageImports<S> {
    /// Create over transport services.
    #[must_use]
    pub fn new(services: S) -> Self {
        Self { services }
    }

    fn vector(
        memory: &mut SparseGuestMemory,
        address: GuestAddress,
    ) -> Result<Vec3, GuestError> {
        memory.read_f32x3(address)
    }

    /// Dispatch one game import; returns `None` for other APIs/names.
    pub fn invoke(
        &mut self,
        memory: &mut SparseGuestMemory,
        api: &str,
        name: &str,
        args: &[GuestCallValue],
        source_slot: &dyn Fn(GuestAddress) -> u32,
    ) -> Option<Result<GuestCallResult, MessageError>> {
        if api != "game" {
            return None;
        }
        let result = self.dispatch(memory, name, args, source_slot);
        Some(result)
    }

    fn dispatch(
        &mut self,
        memory: &mut SparseGuestMemory,
        name: &str,
        args: &[GuestCallValue],
        source_slot: &dyn Fn(GuestAddress) -> u32,
    ) -> Result<GuestCallResult, MessageError> {
        let integer = |index: usize| -> i64 {
            match args.get(index) {
                Some(GuestCallValue::Int32(value)) => i64::from(*value),
                Some(GuestCallValue::Uint32(value)) => i64::from(*value),
                Some(GuestCallValue::Int64(value)) => *value,
                Some(GuestCallValue::Uint64(value)) => *value as i64,
                _ => 0,
            }
        };
        let pointer = |index: usize| -> Option<GuestAddress> {
            match args.get(index) {
                Some(GuestCallValue::Pointer(address)) => *address,
                _ => None,
            }
        };
        match name {
            "WriteChar" | "WriteByte" => {
                self.services
                    .buffer_mut()
                    .write_byte(integer(0) as i32)?;
            }
            "WriteShort" => {
                self.services
                    .buffer_mut()
                    .write_short(integer(0) as i32)?;
            }
            "WriteLong" => {
                self.services
                    .buffer_mut()
                    .write_long(integer(0) as i32)?;
            }
            "WriteFloat" | "WriteAngle" => {
                let value = match args.first() {
                    Some(GuestCallValue::Float32(value)) => *value,
                    _ => return Err(MessageError::NonFloat),
                };
                if name == "WriteFloat" {
                    self.services.buffer_mut().write_float(value)?;
                } else {
                    let packed = ((value * 256.0).round() / 360.0).trunc() as i32 & 255;
                    self.services.buffer_mut().write_byte(packed)?;
                }
            }
            "WritePosition" => {
                let address = pointer(0).ok_or(MessageError::MissingOrigin)?;
                let position = Self::vector(memory, address)?;
                self.services.buffer_mut().write_float(position.x)?;
                self.services.buffer_mut().write_float(position.y)?;
                self.services.buffer_mut().write_float(position.z)?;
            }
            "WriteDir" => match pointer(0) {
                None => {
                    self.services.buffer_mut().write_byte(0)?;
                }
                Some(address) => {
                    let direction = Self::vector(memory, address)?;
                    self.services.buffer_mut().write_dir(direction)?;
                }
            },
            "WriteString" => match pointer(0) {
                None => {
                    self.services.buffer_mut().write_byte(0)?;
                }
                Some(address) => {
                    let maxsize = self.services.buffer_mut().maxsize;
                    let mut length = 0usize;
                    while memory.read_u8(memory.offset(address, length as i64)?)? != 0 {
                        length += 1;
                        if length >= maxsize {
                            return Err(MessageError::StringTooLong);
                        }
                    }
                    let bytes = memory.copy(address, length + 1)?;
                    self.services.buffer_mut().sz_write(&bytes)?;
                }
            },
            "WriteEntity" => {
                let address = pointer(0).ok_or(MessageError::MissingOrigin)?;
                self.services
                    .buffer_mut()
                    .write_short(source_slot(address) as i32)?;
            }
            "unicast" => {
                if self.services.buffer_mut().overflowed {
                    return Err(MessageError::Overflowed);
                }
                match pointer(0) {
                    None => self.services.buffer_mut().clear(),
                    Some(address) => {
                        let slot = source_slot(address);
                        let (reliable, dupe_key, bytes) = {
                            let buffer = self.services.buffer_mut();
                            (
                                integer(1) != 0,
                                integer(2) as u32,
                                buffer.committed().to_vec(),
                            )
                        };
                        if self.services.accepts_client(slot) && !bytes.is_empty() {
                            self.services.unicast(RereleaseUnicast {
                                client_slot: slot,
                                reliable,
                                dupe_key,
                                bytes,
                            });
                        }
                        self.services.buffer_mut().clear();
                    }
                }
            }
            "multicast" => {
                if self.services.buffer_mut().overflowed {
                    return Err(MessageError::Overflowed);
                }
                let target = integer(1);
                let address = pointer(0);
                let destination = match target {
                    0 => MulticastDestination::All,
                    1 => MulticastDestination::Phs,
                    2 => MulticastDestination::Pvs,
                    _ => return Err(MessageError::BadDestination),
                };
                if target != 0 && address.is_none() {
                    return Err(MessageError::MissingOrigin);
                }
                let origin = match address {
                    None => None,
                    Some(at) => Some(Self::vector(memory, at)?),
                };
                let (reliable, bytes) = {
                    let buffer = self.services.buffer_mut();
                    (integer(2) != 0, buffer.committed().to_vec())
                };
                if !bytes.is_empty() {
                    self.services.multicast(RereleaseMulticast {
                        origin,
                        destination,
                        reliable,
                        bytes,
                    });
                }
                self.services.buffer_mut().clear();
            }
            _ => return Ok(GuestCallResult::Void),
        }
        Ok(GuestCallResult::Void)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::ProviderId;
    use qa_guest::core::contracts::{ContentDigest, GuestAllocationOptions, ModuleIdentity};

    struct FakeServices {
        buffer: SizeBuf,
        unicasts: Vec<RereleaseUnicast>,
        multicasts: Vec<RereleaseMulticast>,
    }

    impl RereleaseMessageServices for FakeServices {
        fn buffer_mut(&mut self) -> &mut SizeBuf {
            &mut self.buffer
        }
        fn accepts_client(&self, slot: u32) -> bool {
            slot == 1
        }
        fn unicast(&mut self, message: RereleaseUnicast) {
            self.unicasts.push(message);
        }
        fn multicast(&mut self, message: RereleaseMulticast) {
            self.multicasts.push(message);
        }
    }

    fn harness() -> (
        SparseGuestMemory,
        RereleaseMessageImports<FakeServices>,
    ) {
        let module = ModuleIdentity::new(
            ProviderId::new("q2", "messages-test"),
            "game.dll",
            ContentDigest::new("sha256", "00"),
            "test",
        );
        let memory = SparseGuestMemory::new(module, 8, 0x1_0000).expect("memory");
        let imports = RereleaseMessageImports::new(FakeServices {
            buffer: SizeBuf::new(1024),
            unicasts: Vec::new(),
            multicasts: Vec::new(),
        });
        (memory, imports)
    }

    fn int(value: i32) -> GuestCallValue {
        GuestCallValue::Int32(value)
    }

    #[test]
    fn writers_encode_and_unicast_delivers() {
        let (mut memory, mut imports) = harness();
        let slot_of = |address: GuestAddress| {
            if address.offset == 0x100 { 1 } else { 7 }
        };
        imports
            .invoke(&mut memory, "game", "WriteByte", &[int(0x41)], &slot_of)
            .unwrap()
            .expect("byte");
        imports
            .invoke(&mut memory, "game", "WriteShort", &[int(0x1234)], &slot_of)
            .unwrap()
            .expect("short");
        imports
            .invoke(
                &mut memory,
                "game",
                "WriteFloat",
                &[GuestCallValue::Float32(1.5)],
                &slot_of,
            )
            .unwrap()
            .expect("float");
        assert_eq!(imports.services.buffer.cursize, 7);
        let entity = GuestAddress::new(memory.address_space(), 0x100);
        imports
            .invoke(
                &mut memory,
                "game",
                "unicast",
                &[
                    GuestCallValue::Pointer(Some(entity)),
                    int(1),
                    int(9),
                ],
                &slot_of,
            )
            .unwrap()
            .expect("unicast");
        assert_eq!(imports.services.unicasts.len(), 1);
        let sent = &imports.services.unicasts[0];
        assert_eq!(sent.client_slot, 1);
        assert!(sent.reliable);
        assert_eq!(sent.dupe_key, 9);
        assert_eq!(sent.bytes.len(), 7);
        assert_eq!(imports.services.buffer.cursize, 0);
        assert!(
            imports
                .invoke(&mut memory, "cgame", "WriteByte", &[int(1)], &slot_of)
                .is_none()
        );
    }

    #[test]
    fn multicast_validates_destination_and_dir() {
        let (mut memory, mut imports) = harness();
        let slot_of = |_: GuestAddress| 1;
        let origin = memory
            .allocate(&GuestAllocationOptions::bytes(12))
            .expect("alloc");
        memory.write_f32(origin, 1.0).expect("x");
        memory
            .write_f32(memory.offset(origin, 4).expect("o"), 2.0)
            .expect("y");
        memory
            .write_f32(memory.offset(origin, 8).expect("o"), 3.0)
            .expect("z");
        imports
            .invoke(
                &mut memory,
                "game",
                "WritePosition",
                &[GuestCallValue::Pointer(Some(origin))],
                &slot_of,
            )
            .unwrap()
            .expect("position");
        imports
            .invoke(
                &mut memory,
                "game",
                "multicast",
                &[
                    GuestCallValue::Pointer(Some(origin)),
                    int(2),
                    int(0),
                ],
                &slot_of,
            )
            .unwrap()
            .expect("multicast");
        assert_eq!(imports.services.multicasts.len(), 1);
        let sent = &imports.services.multicasts[0];
        assert_eq!(sent.destination, MulticastDestination::Pvs);
        assert_eq!(sent.bytes.len(), 12);
        let bad = imports
            .invoke(
                &mut memory,
                "game",
                "multicast",
                &[GuestCallValue::Pointer(Some(origin)), int(5), int(0)],
                &slot_of,
            )
            .unwrap()
            .unwrap_err();
        assert_eq!(bad, MessageError::BadDestination);
        imports
            .invoke(
                &mut memory,
                "game",
                "WriteDir",
                &[GuestCallValue::Pointer(None)],
                &slot_of,
            )
            .unwrap()
            .expect("dir");
        assert_eq!(imports.services.buffer.committed(), &[0]);
    }
}
