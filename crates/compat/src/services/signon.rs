use super::*;
use qa_core::{
    events::{EventRing, OutputConsumerId},
    primitives::{
        NativeEmission, NativeTarget, ResourceNamespace, SoundAction, SoundEmitter, SoundId,
    },
};

/// Native signon sounds and static models share one fixed byte budget.
pub const SIGNON_BYTES: usize = NATIVE_WIRE_BYTES;

/// Native quantization belongs to the ABI adapter, before registration.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AmbientSound {
    pub source_ordinal: u32,
    pub origin: [i16; 3],
    pub volume: u8,
    pub attenuation: u8,
}

/// One cursor belongs to one module lifetime and one signon recipient.
#[derive(Default)]
pub struct SignonCursor {
    owner: Option<(ModuleId, EntityId, Option<(ClientId, EntityId)>)>,
    next: usize,
    end: Option<usize>,
}
impl SignonCursor {
    fn bind(
        &mut self,
        module: ModuleId,
        lifetime: EntityId,
        target: Option<(ClientId, EntityId)>,
        end: Option<usize>,
    ) -> Result<(), CallError> {
        let owner = (module, lifetime, target);
        if let Some(old) = self.owner {
            if old.0 != module || old.2.map(|client| client.0) != target.map(|client| client.0) {
                return Err(CallError::Entity);
            }
        }
        if self.owner != Some(owner) {
            self.next = 0;
            self.end = None;
        }
        self.end = self.end.or(end);
        self.owner = Some(owner);
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum SignonRecord {
    Sound(AmbientSound),
    StaticEntity(EntityRecord),
}
#[derive(Clone, Copy)]
struct Registration {
    native: SignonRecord,
    spatial: qa_core::primitives::SpatialPolicy,
    /// A Stop retains a real sample that was successfully published, even if
    /// its native resource row was later overwritten or cleared.
    published_sound: Option<SoundId>,
}
impl Default for Registration {
    fn default() -> Self {
        Self {
            native: SignonRecord::Sound(AmbientSound::default()),
            spatial: qa_core::primitives::SpatialPolicy::default(),
            published_sound: None,
        }
    }
}
pub(super) struct SignonRegistry {
    rows: Box<[Registration]>,
    length: usize,
    bytes: usize,
}
impl SignonRegistry {
    pub(super) fn load(capacity: usize) -> Self {
        Self {
            rows: vec![Registration::default(); capacity].into_boxed_slice(),
            length: 0,
            bytes: 0,
        }
    }
    pub(super) fn reset(&mut self) {
        self.length = 0;
        self.bytes = 0;
    }
}

impl ServiceStorage {
    pub(super) fn register_ambient(
        &mut self,
        context: CallContext,
        sound: AmbientSound,
    ) -> Result<u32, CallError> {
        let range = self.range(context.module).ok_or(CallError::ConfigString)?;
        range.lifetime.ok_or(CallError::Entity)?;
        let sounds = range.resources.ok_or(CallError::ConfigString)?[1];
        self.check_range(context.module, sounds)?;
        let ordinal = sounds
            .get(sound.source_ordinal as usize)
            .filter(|_| sound.source_ordinal != 0)
            .ok_or(CallError::ConfigString)?;
        if self.configstring(context.module, ordinal)?.0.is_empty() {
            return Err(CallError::ConfigString);
        }
        self.register_signon(context, SignonRecord::Sound(sound), 10)
    }

    pub(super) fn register_static(
        &mut self,
        context: CallContext,
        entity: EntityRecord,
    ) -> Result<u32, CallError> {
        let range = self.range(context.module).ok_or(CallError::ConfigString)?;
        range.lifetime.ok_or(CallError::Entity)?;
        if let Some(model) = entity.appearance.models[0] {
            if model.namespace != ResourceNamespace::Module(context.module) {
                return Err(CallError::ConfigString);
            }
            let models = range.resources.ok_or(CallError::ConfigString)?[0];
            self.check_range(context.module, models)?;
            let ordinal = models
                .get(model.ordinal as usize)
                .ok_or(CallError::ConfigString)?;
            if self.configstring(context.module, ordinal)?.0.is_empty() {
                return Err(CallError::ConfigString);
            }
        }
        self.register_signon(context, SignonRecord::StaticEntity(entity), 14)
    }

    fn register_signon(
        &mut self,
        context: CallContext,
        native: SignonRecord,
        bytes: usize,
    ) -> Result<u32, CallError> {
        let registry = &mut self
            .range_mut(context.module)
            .ok_or(CallError::ConfigString)?
            .signon;
        let slot = registry.length;
        if registry.bytes + bytes > SIGNON_BYTES {
            return Err(CallError::Capacity);
        }
        let row = registry.rows.get_mut(slot).ok_or(CallError::Capacity)?;
        *row = Registration {
            native,
            spatial: context.sound_policy,
            published_sound: None,
        };
        registry.length += 1;
        registry.bytes += bytes;
        Ok(slot as u32)
    }

    /// Each connected recipient snapshots retained registrations on its first
    /// eligible admission. Static models do not require a sound bank. Later
    /// registrations belong to future connections.
    pub fn publish_signon(
        &mut self,
        server: &mut Server,
        context: CallContext,
        client: ClientId,
        cursor: &mut SignonCursor,
    ) -> Result<usize, CallError> {
        self.publish_target_signon(
            server,
            context,
            Some(client),
            cursor,
            SoundAction::StartLoop,
        )
    }

    /// Stop the exact static identities previously published by this module.
    /// The caller invokes this before dropping storage or rebinding its lifetime.
    pub fn stop_ambients(
        &mut self,
        server: &mut Server,
        context: CallContext,
        client: Option<ClientId>,
    ) -> Result<usize, CallError> {
        self.publish_target_signon(
            server,
            context,
            client,
            &mut SignonCursor::default(),
            SoundAction::Stop,
        )
    }

    fn publish_target_signon(
        &mut self,
        server: &mut Server,
        context: CallContext,
        client: Option<ClientId>,
        cursor: &mut SignonCursor,
        action: SoundAction,
    ) -> Result<usize, CallError> {
        let bank = self.sound_bank.as_deref();
        let row = self
            .ranges
            .iter_mut()
            .find(|row| row.module == context.module)
            .ok_or(CallError::ConfigString)?;
        let sounds = bank.zip(row.sound_bindings.as_ref());
        let lifetime = row.lifetime.ok_or(CallError::Entity)?;
        if action == SoundAction::StartLoop && server.entities.resolve(lifetime).is_none() {
            return Err(CallError::Entity);
        }
        let recipient = client
            .map(|id| {
                let client = server.clients.get(id.0 as usize).ok_or(CallError::Entity)?;
                if action == SoundAction::StartLoop
                    && (client.connection.is_none()
                        || client.connection == Some(qa_session::clients::Connection::Bot)
                        || server.entities.resolve(client.entity).is_none())
                {
                    return Err(CallError::Entity);
                }
                Ok((id, client.entity))
            })
            .transpose()?;
        let consumer = if action == SoundAction::StartLoop {
            let id = client.ok_or(CallError::Entity)?;
            let output = server.clients[id.0 as usize]
                .output
                .ok_or(CallError::Entity)?;
            if server.events.counters(output).is_none() {
                return Err(CallError::Entity);
            }
            if server.events.needs_resync(output) {
                return Ok(0);
            }
            Some(output)
        } else {
            None
        };
        let end = (sounds.is_some() || action == SoundAction::Stop).then_some(row.signon.length);
        cursor.bind(context.module, lifetime, recipient, end)?;
        publish(
            &mut server.events,
            &mut row.signon.rows[..row.signon.length],
            cursor,
            context,
            row.native_rules,
            lifetime,
            client,
            action,
            sounds,
            consumer,
        )
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "One publisher serves recipient and stop signon paths without copying the admission loop."
)]
fn publish(
    events: &mut EventRing,
    rows: &mut [Registration],
    cursor: &mut SignonCursor,
    context: CallContext,
    source: RuleSetId,
    lifetime: EntityId,
    client: Option<ClientId>,
    action: SoundAction,
    sounds: Option<(&qa_audio::Bank, &qa_audio::SoundBindings)>,
    consumer: Option<OutputConsumerId>,
) -> Result<usize, CallError> {
    let mut published = 0;
    let end = rows.len();
    while let Some(row) = rows.get_mut(cursor.next) {
        if cursor.end.is_some_and(|end| cursor.next >= end) {
            break;
        }
        if consumer.is_some_and(|consumer| events.needs_resync(consumer)) {
            break;
        }
        let (event, position, sound) = match row.native {
            SignonRecord::StaticEntity(entity) => {
                if action == SoundAction::Stop {
                    cursor.next += 1;
                    continue;
                }
                cursor.end.get_or_insert(end);
                (FrameEvent::StaticEntity(entity), entity.position, None)
            }
            SignonRecord::Sound(native) => {
                let sound = if action == SoundAction::Stop {
                    row.published_sound
                } else {
                    let Some((bank, bindings)) = sounds else {
                        break;
                    };
                    bindings
                        .sound(native.source_ordinal)
                        .filter(|sound| bank.has_loop_marker(*sound))
                };
                let Some(sound) = sound else {
                    cursor.next += 1;
                    continue;
                };
                let position = Vec3(native.origin.map(qa_core::math::short_to_eighth));
                let event = SoundEvent {
                    sound,
                    source_ordinal: Some(native.source_ordinal),
                    native_fields: None,
                    spatial: row.spatial,
                    entity: Some(SoundEmitter::Static {
                        namespace: ResourceNamespace::Module(context.module),
                        lifetime,
                        slot: cursor.next as u32,
                    }),
                    channel: 0,
                    position,
                    volume: f32::from(native.volume) / 255.0,
                    attenuation: f32::from(native.attenuation) / 64.0,
                    action,
                    client: None,
                    time_offset: 0.0,
                    positioned: true,
                };
                (FrameEvent::Sound(event), position, Some(sound))
            }
        };
        let delivery = NativeDelivery {
            module: context.module,
            source,
            target: client.map_or(NativeTarget::All, NativeTarget::Client),
            origin: position,
            reliable: true,
            dupe_key: 0,
            component: 0,
            emission: Some(NativeEmission {
                module_lifetime: lifetime,
                provider_frame: context.server_frame,
            }),
            check_portals: false,
        };
        let Ok(sequence) = events.push_delivered(delivery, event) else {
            break;
        };
        if consumer.is_some_and(|consumer| !events.retained_for(consumer, sequence)) {
            break;
        }
        if action == SoundAction::StartLoop {
            row.published_sound = sound;
        }
        cursor.next += 1;
        published += 1;
    }
    Ok(published)
}
