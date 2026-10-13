//! Native staging enters the shared sound, effect and print records.
use super::{CallError, EngineServices};
use qa_core::{
    events::{EventStorageError, FrameEvent},
    primitives::{
        IndexSpan, NATIVE_CUE_CAPTURE_BYTES, NativeCueBinding, NativeDelivery, NativeEntity,
        ResourceNamespace, SoundAction, SoundEmitter, SpatialPolicy, Vec3,
    },
};
use qa_network::{
    commands::packet::Protocol,
    outputs::{self, Cue, NativeService, Particle, Print, Sound},
};

enum Part<'a> {
    Particle {
        offset: usize,
        wire: Particle,
    },
    Sound {
        offset: usize,
        wire: Sound,
        static_sound: bool,
    },
    Print {
        offset: usize,
        wire: &'a [u8],
        print: Print<'a>,
        consumed: usize,
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
    type Item = Result<Part<'a>, CallError>;
    fn next(&mut self) -> Option<Self::Item> {
        let offset = self.next;
        let bytes = self.bytes.get(offset..).filter(|bytes| !bytes.is_empty())?;
        let (service, length) = match outputs::read_native_service(self.protocol, bytes) {
            Ok(value) => value,
            Err(outputs::Error::Unsupported) => {
                self.next = self.bytes.len();
                // An unknown-length tail remains opaque, never guessed.
                return Some(Ok(Part::Native { offset, bytes }));
            }
            Err(_) => {
                self.next = self.bytes.len();
                return Some(Err(CallError::Memory));
            }
        };
        self.next += length;
        Some(Ok(match service {
            NativeService::Particle(wire) => Part::Particle { offset, wire },
            NativeService::Cue(Cue::Sound(wire) | Cue::StaticSound(wire)) => Part::Sound {
                offset,
                wire,
                static_sound: matches!(service, NativeService::Cue(Cue::StaticSound(_))),
            },
            NativeService::Print(print) => {
                // A native reader can stop at its signed/width boundary before
                // a producer's string ends. Keep an undecodable continuation
                // with these raw pages so a rejected prefix cannot send it alone.
                let opaque_tail = length < bytes.len()
                    && matches!(
                        outputs::read_native_service(self.protocol, &bytes[length..]),
                        Err(outputs::Error::Unsupported)
                    );
                let wire = if opaque_tail {
                    self.next = self.bytes.len();
                    bytes
                } else {
                    &bytes[..length]
                };
                Part::Print {
                    offset,
                    wire,
                    print,
                    consumed: length,
                }
            }
            _ => Part::Native {
                offset,
                bytes: &bytes[..length],
            },
        }))
    }
}

pub(super) fn publish(
    s: &mut EngineServices<'_>,
    delivery: NativeDelivery,
    spatial: SpatialPolicy,
) -> Result<Option<u64>, CallError> {
    // During native Init/ClientBegin the same bank is still cold. Resolve
    // numeric sounds before the grouped publisher borrows staging bytes.
    if s.sound_loader.is_some() {
        let sounds = s.storage.resources(delivery.module).map(|ranges| ranges[1]);
        let protocol = Protocol::native(delivery.source);
        let mut position = 0;
        loop {
            let (ordinal, consumed) = {
                let row = s.storage.entity_range(delivery.module)?;
                if position >= row.written {
                    break;
                }
                let mut parts = Parts::new(protocol, &row.message[position..row.written]);
                let ordinal = match parts.next() {
                    Some(Ok(Part::Sound { wire, .. })) => Some(wire.index),
                    _ => None,
                };
                (ordinal, parts.next)
            };
            if consumed == 0 {
                break;
            }
            position += consumed;
            if let Some((sounds, ordinal)) = sounds.zip(ordinal)
                && u32::from(ordinal) < sounds.count
            {
                s.sound_id(delivery.module, sounds, i32::from(ordinal))?;
            }
        }
    }
    let row = s.storage.entity_range(delivery.module)?;
    if row.written == 0 {
        return Ok(None);
    }
    let wire_length = row.written;
    let (wire, metadata) = row.message.split_at_mut(wire_length);
    let protocol = Protocol::native(delivery.source);
    let particles = s.registry.effects.native_particles(delivery.source);
    let mut captured = 0;
    let mut count = 0;
    let mut rejected_sounds = 0;
    for part in Parts::new(protocol, wire) {
        let part = match part {
            Ok(part) => part,
            Err(error) => {
                row.written = 0;
                s.server.events.reject_publication();
                return Err(error);
            }
        };
        match part {
            Part::Particle { .. } => {
                if particles.is_none() {
                    row.written = 0;
                    s.server.events.reject_publication();
                    return Err(CallError::Unsupported);
                }
                count += 1;
            }
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
            Part::Print { .. } => count += 1,
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
                    match part.map_err(|_| EventStorageError::TextCapacity)? {
                        Part::Particle { offset, wire } => {
                            let binding = particles.ok_or(EventStorageError::TextCapacity)?;
                            group.push_delivered(
                                NativeDelivery {
                                    component: offset as u32,
                                    ..delivery
                                },
                                FrameEvent::Effect(binding.event(
                                    wire.position,
                                    wire.direction,
                                    wire.color,
                                    wire.count,
                                )),
                            )?;
                        }
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
                        Part::Print {
                            offset,
                            wire,
                            print,
                            consumed,
                        } => {
                            let first = (print.text.as_ptr() as usize)
                                .checked_sub(wire.as_ptr() as usize)
                                .ok_or(EventStorageError::TextCapacity)?;
                            group.print(
                                NativeDelivery {
                                    component: offset as u32,
                                    ..delivery
                                },
                                print.kind,
                                print.level.unwrap_or(print.kind.default_level()),
                                wire,
                                IndexSpan {
                                    first: first as u32,
                                    count: print.text.len() as u32,
                                },
                                consumed as u32,
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
