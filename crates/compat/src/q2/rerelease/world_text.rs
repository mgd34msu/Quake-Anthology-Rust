//! Q2 rerelease world-text drawing imports.
//!
//! Donor: `src/compat/q2/rerelease/world-text.ts` — bridges the `game.h`
//! `Draw_OrientedWorldText` / `Draw_StaticWorldText` imports into text events.

use qa_core::math::{Vec3, Vec4};
use qa_guest::GuestError;
use qa_guest::core::contracts::{GuestAddress, GuestCallResult, GuestCallValue};
use qa_guest::core::memory::SparseGuestMemory;
use thiserror::Error;

/// World-text import failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WorldTextError {
    /// World text requires a source float.
    #[error("Q2 world text requires a source float")]
    NonFloat,
    /// Guest memory failure.
    #[error(transparent)]
    Guest(#[from] GuestError),
}

/// World-text orientation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WorldTextOrientation {
    /// Camera-facing billboard.
    Billboard,
    /// Fixed angles.
    Fixed {
        /// Angles.
        angles: Vec3,
    },
}

/// World-text font.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorldTextFont {
    /// Classic textured debug font.
    Classic,
    /// Selected font.
    Selected,
}

/// Owned world-text input for the receiving world.
#[derive(Debug, Clone, PartialEq)]
pub struct WorldTextInput {
    /// Glyph text (up to 127 bytes).
    pub text: String,
    /// Origin.
    pub origin: Vec3,
    /// Color.
    pub color: Vec4,
    /// Cell size in world units.
    pub cell_size: f32,
    /// Distance cull factor.
    pub distance_cull_factor: f32,
    /// Orientation.
    pub orientation: WorldTextOrientation,
    /// Depth test flag.
    pub depth_test: bool,
    /// Font.
    pub font: WorldTextFont,
}

/// One world-text event: owned text plus lifetime.
#[derive(Debug, Clone, PartialEq)]
pub struct RereleaseWorldTextEvent {
    /// Owned text input.
    pub text: WorldTextInput,
    /// Lifetime in seconds.
    pub lifetime: f32,
}

/// Q2's textured debug font uses 127 byte glyphs and eight-unit cells.
#[must_use]
pub fn q2_world_text(
    origin: Vec3,
    angles: Option<Vec3>,
    text: &str,
    color: Vec4,
    size: f32,
    depth_test: bool,
) -> WorldTextInput {
    let masked: String = text.bytes().take(127).map(|byte| byte as char).collect();
    WorldTextInput {
        text: masked,
        origin,
        color,
        cell_size: size * 8.0,
        distance_cull_factor: 0.004,
        orientation: match angles {
            None => WorldTextOrientation::Billboard,
            Some(angles) => WorldTextOrientation::Fixed { angles },
        },
        depth_test,
        font: WorldTextFont::Classic,
    }
}

/// `Draw_OrientedWorldText` / `Draw_StaticWorldText` import dispatch.
///
/// Copies guest arguments; the receiving world owns timing and teardown.
pub struct RereleaseWorldTextImports {
    /// Emitted events in dispatch order.
    pub events: Vec<RereleaseWorldTextEvent>,
}

impl RereleaseWorldTextImports {
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
    ) -> Option<Result<GuestCallResult, WorldTextError>> {
        if api != "game"
            || (name != "Draw_OrientedWorldText" && name != "Draw_StaticWorldText")
        {
            return None;
        }
        Some(self.dispatch(memory, name, args))
    }

    fn dispatch(
        &mut self,
        memory: &mut SparseGuestMemory,
        name: &str,
        args: &[GuestCallValue],
    ) -> Result<GuestCallResult, WorldTextError> {
        let fixed = name == "Draw_StaticWorldText";
        let offset = usize::from(fixed);
        let pointer = |index: usize| -> Option<GuestAddress> {
            match args.get(index) {
                Some(GuestCallValue::Pointer(address)) => *address,
                _ => None,
            }
        };
        let float = |index: usize| -> Result<f32, WorldTextError> {
            match args.get(index) {
                Some(GuestCallValue::Float32(value)) => Ok(*value),
                _ => Err(WorldTextError::NonFloat),
            }
        };
        let integer = |index: usize| -> i64 {
            match args.get(index) {
                Some(GuestCallValue::Int32(value)) => i64::from(*value),
                Some(GuestCallValue::Uint32(value)) => i64::from(*value),
                Some(GuestCallValue::Int64(value)) => *value,
                Some(GuestCallValue::Uint64(value)) => *value as i64,
                _ => 0,
            }
        };
        let origin_address = pointer(0).ok_or(WorldTextError::NonFloat)?;
        let string_address = pointer(1 + offset).ok_or(WorldTextError::NonFloat)?;
        let color_address = pointer(2 + offset).ok_or(WorldTextError::NonFloat)?;
        let origin = memory.read_f32x3(origin_address)?;
        let angles = if fixed {
            Some(memory.read_f32x3(pointer(1).ok_or(WorldTextError::NonFloat)?)?)
        } else {
            None
        };
        let mut text = String::new();
        for index in 0..127 {
            let byte = memory.read_u8(memory.offset(string_address, index)?)?;
            if byte == 0 {
                break;
            }
            text.push(byte as char);
        }
        let channel = |index: i64| -> Result<f32, WorldTextError> {
            Ok(f32::from(
                memory.read_u8(memory.offset(color_address, index)?)?,
            ) / 255.0)
        };
        let color = Vec4 {
            x: channel(0)?,
            y: channel(1)?,
            z: channel(2)?,
            w: channel(3)?,
        };
        self.events.push(RereleaseWorldTextEvent {
            text: q2_world_text(
                origin,
                angles,
                &text,
                color,
                float(3 + offset)?,
                integer(5 + offset) != 0,
            ),
            lifetime: float(4 + offset)?,
        });
        Ok(GuestCallResult::Void)
    }
}

impl Default for RereleaseWorldTextImports {
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
            ProviderId::new("q2", "world-text-test"),
            "game.dll",
            ContentDigest::new("sha256", "00"),
            "test",
        );
        SparseGuestMemory::new(module, 8, 0x1_0000).expect("memory")
    }

    fn write_vec(memory: &mut SparseGuestMemory, value: Vec3) -> GuestAddress {
        let address = memory
            .allocate(&GuestAllocationOptions::bytes(12))
            .expect("alloc");
        memory.write_f32(address, value.x).expect("x");
        memory
            .write_f32(memory.offset(address, 4).expect("o"), value.y)
            .expect("y");
        memory
            .write_f32(memory.offset(address, 8).expect("o"), value.z)
            .expect("z");
        address
    }

    fn write_bytes(memory: &mut SparseGuestMemory, bytes: &[u8]) -> GuestAddress {
        let address = memory
            .allocate(&GuestAllocationOptions::bytes(bytes.len()))
            .expect("alloc");
        memory.write(address, bytes).expect("write");
        address
    }

    #[test]
    fn oriented_text_is_billboard() {
        let mut memory = test_memory();
        let mut imports = RereleaseWorldTextImports::new();
        let origin = write_vec(
            &mut memory,
            Vec3 {
                x: 1.0,
                y: 2.0,
                z: 3.0,
            },
        );
        let string = write_bytes(&mut memory, b"hi\0");
        let color = write_bytes(&mut memory, &[0, 255, 0, 255]);
        imports
            .invoke(
                &mut memory,
                "game",
                "Draw_OrientedWorldText",
                &[
                    GuestCallValue::Pointer(Some(origin)),
                    GuestCallValue::Pointer(Some(string)),
                    GuestCallValue::Pointer(Some(color)),
                    GuestCallValue::Float32(2.0),
                    GuestCallValue::Float32(5.0),
                    GuestCallValue::Int32(1),
                ],
            )
            .unwrap()
            .expect("text");
        assert_eq!(imports.events.len(), 1);
        let event = &imports.events[0];
        assert_eq!(event.text.text, "hi");
        assert_eq!(event.text.cell_size, 16.0);
        assert_eq!(
            event.text.orientation,
            WorldTextOrientation::Billboard
        );
        assert!(event.text.depth_test);
        assert_eq!(event.lifetime, 5.0);
        assert_eq!(event.text.color.y, 1.0);
    }

    #[test]
    fn static_text_is_fixed_and_truncates() {
        let mut memory = test_memory();
        let mut imports = RereleaseWorldTextImports::new();
        let origin = write_vec(
            &mut memory,
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
        );
        let angles = write_vec(
            &mut memory,
            Vec3 {
                x: 0.0,
                y: 90.0,
                z: 0.0,
            },
        );
        let mut glyphs = vec![b'A'; 200];
        glyphs.push(0);
        let string = write_bytes(&mut memory, &glyphs);
        let color = write_bytes(&mut memory, &[255, 255, 255, 255]);
        imports
            .invoke(
                &mut memory,
                "game",
                "Draw_StaticWorldText",
                &[
                    GuestCallValue::Pointer(Some(origin)),
                    GuestCallValue::Pointer(Some(angles)),
                    GuestCallValue::Pointer(Some(string)),
                    GuestCallValue::Pointer(Some(color)),
                    GuestCallValue::Float32(1.0),
                    GuestCallValue::Float32(3.0),
                    GuestCallValue::Int32(0),
                ],
            )
            .unwrap()
            .expect("text");
        let event = &imports.events[0];
        assert_eq!(event.text.text.len(), 127);
        assert!(matches!(
            event.text.orientation,
            WorldTextOrientation::Fixed { .. }
        ));
        assert!(!event.text.depth_test);
        assert!(
            imports
                .invoke(&mut memory, "game", "Draw_Line", &[])
                .is_none()
        );
    }
}
