//! Native staging is decoded once at this boundary into shared sound records.
use super::{CallError, EngineServices};
use qa_core::{
    events::{EventStorageError, FrameEvent},
    primitives::{
        NATIVE_CUE_CAPTURE_BYTES, NativeCueBinding, NativeDelivery, NativeEntity,
        ResourceNamespace, SoundAction, SoundEmitter, SpatialPolicy, Vec3,
    },
};
use qa_network::{
    commands::packet::Protocol,
    outputs::{self, Cue, NativeService, Sound},
};

enum Part<'a> {
    Sound {
        offset: usize,
        wire: Sound,
        static_sound: bool,
    },
    Native {
        offset: usize,
        bytes: &'a [u8],
    },
}
struct Parts<'a> {
    protocol: Protocol,
    bytes: &'a [u8],
    next: usize,
}
impl<'a> Parts<'a> {
    fn new(protocol: Protocol, bytes: &'a [u8]) -> Self {
        Self {
            protocol,
            bytes,
            next: 0,
        }
    }
}
impl<'a> Iterator for Parts<'a> {
    type Item = Part<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        let start = self.next;
        while self.next < self.bytes.len() {
            let at = self.next;
            let Ok((service, length)) =
                outputs::read_native_service(self.protocol, &self.bytes[at..])
            else {
                self.next = self.bytes.len();
                break; // An unknown-length tail remains opaque, never guessed.
            };
            if let NativeService::Cue(Cue::Sound(wire) | Cue::StaticSound(wire)) = service {
                if at != start {
                    return Some(Part::Native {
                        offset: start,
                        bytes: &self.bytes[start..at],
                    });
                }
                self.next += length;
                return Some(Part::Sound {
                    offset: at,
                    wire,
                    static_sound: matches!(service, NativeService::Cue(Cue::StaticSound(_))),
                });
            }
            self.next += length;
        }
        (start != self.next).then(|| Part::Native {
            offset: start,
            bytes: &self.bytes[start..self.next],
        })
    }
}

pub(super) fn publish(
    s: &mut EngineServices<'_>,
    delivery: NativeDelivery,
    spatial: SpatialPolicy,
) -> Result<Option<u64>, CallError> {
    let row = s.storage.entity_range(delivery.module)?;
    if row.written == 0 {
        return Ok(None);
    }
    let wire_length = row.written;
    let (wire, metadata) = row.message.split_at_mut(wire_length);
    let protocol = Protocol::native(delivery.source);
    let mut captured = 0;
    let mut count = 0;
    let mut rejected_sounds = 0;
    for part in Parts::new(protocol, wire) {
        match part {
            Part::Sound { wire, .. } => {
                if row
                    .sound_bindings
                    .as_ref()
                    .and_then(|b| b.sound(u32::from(wire.index)))
                    .is_some()
                {
                    count += 1;
                } else {
                    rejected_sounds += 1;
                }
            }
            Part::Native { offset, bytes } => {
                count += 1;
                let mut local = 0;
                while local < bytes.len() {
                    let Ok((service, length)) =
                        outputs::read_native_service(protocol, &bytes[local..])
                    else {
                        break;
                    };
                    if let NativeService::Cue(Cue::MuzzleFlash(flash)) = service {
                        let Some(target) =
                            metadata.get_mut(captured..captured + NATIVE_CUE_CAPTURE_BYTES)
                        else {
                            row.written = 0;
                            s.server.events.reject_publication();
                            return Err(CallError::Capacity);
                        };
                        let entity = s.server.entities.find_native(
                            NativeEntity {
                                module: delivery.module,
                                slot: i32::from(flash.entity),
                            },
                            None,
                        );
                        let _ = NativeCueBinding { entity }
                            .write_capture((offset + local) as u32, target);
                        captured += NATIVE_CUE_CAPTURE_BYTES;
                    }
                    local += length;
                }
            }
        }
    }
    let result = if count == 0 {
        Ok(None)
    } else {
        let entities = &s.server.entities;
        s.server
            .events
            .publish_group(count, |group| {
                let mut capture_start = 0;
                for part in Parts::new(protocol, wire) {
                    match part {
                        Part::Sound {
                            offset,
                            wire,
                            static_sound,
                        } => {
                            let Some(sound) = row
                                .sound_bindings
                                .as_ref()
                                .and_then(|b| b.sound(u32::from(wire.index)))
                            else {
                                continue;
                            };
                            let emitter = wire.entity().map(|slot| {
                                entities
                                    .find_native(
                                        NativeEntity {
                                            module: delivery.module,
                                            slot: i32::from(slot),
                                        },
                                        None,
                                    )
                                    .map(SoundEmitter::Entity)
                                    .unwrap_or(SoundEmitter::Native {
                                        namespace: ResourceNamespace::Module(delivery.module),
                                        slot: i32::from(slot),
                                    })
                            });
                            let event = wire.into_event(
                                sound,
                                spatial,
                                emitter,
                                wire.position().unwrap_or(Vec3::default()),
                                None,
                                if static_sound {
                                    SoundAction::StartLoop
                                } else {
                                    SoundAction::Play
                                },
                            );
                            group.push_delivered(
                                NativeDelivery {
                                    component: offset as u32,
                                    ..delivery
                                },
                                FrameEvent::Sound(event),
                            )?;
                        }
                        Part::Native { offset, bytes } => {
                            let mut capture_end = capture_start;
                            while capture_end < captured {
                                let at = NativeCueBinding::offset(&metadata[capture_end..])
                                    .ok_or(EventStorageError::TextCapacity)?;
                                if at as usize >= offset + bytes.len() {
                                    break;
                                }
                                capture_end += NATIVE_CUE_CAPTURE_BYTES;
                            }
                            group.native_message_parts(
                                NativeDelivery {
                                    component: offset as u32,
                                    ..delivery
                                },
                                bytes,
                                &metadata[capture_start..capture_end],
                            )?;
                            capture_start = capture_end;
                        }
                    }
                }
                Ok(())
            })
            .map(|range| Some(range.end - 1))
            .map_err(|_| CallError::Capacity)
    };
    // Native multicast/unicast consumes staging even on admission failure.
    row.written = 0;
    if result.is_ok() {
        for _ in 0..rejected_sounds {
            s.server.events.reject_publication();
        }
    }
    result
}
