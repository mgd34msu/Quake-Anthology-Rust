use super::*;
use qa_core::{
    events::EventRing,
    primitives::{
        NativeEmission, NativeTarget, ResourceNamespace, SoundAction, SoundEmitter, SoundId,
    },
};

/// A native static-sound signon record occupies ten bytes. Registrations own
/// the same bounded map-load budget rather than growing an emission queue.
pub const AMBIENT_CAPACITY: usize = NATIVE_WIRE_BYTES / 10;

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
pub struct AmbientCursor {
    owner: Option<(ModuleId, EntityId, Option<ClientId>)>,
    next: usize,
}
impl AmbientCursor {
    fn bind(
        &mut self,
        module: ModuleId,
        lifetime: EntityId,
        target: Option<ClientId>,
    ) -> Result<(), CallError> {
        let owner = (module, lifetime, target);
        if self.owner.is_some_and(|old| old != owner) {
            return Err(CallError::Entity);
        }
        self.owner = Some(owner);
        Ok(())
    }
}

#[derive(Clone, Copy, Default)]
struct Registration {
    native: AmbientSound,
    spatial_rules: RuleSetId,
    /// A Stop retains a real sample that was successfully published, even if
    /// its native resource row was later overwritten or cleared.
    published_sound: Option<SoundId>,
}
pub(super) struct AmbientRegistry {
    rows: Box<[Registration]>,
    length: usize,
    initial_end: Option<usize>,
    initial: AmbientCursor,
}
impl AmbientRegistry {
    pub(super) fn load(capacity: usize) -> Self {
        Self {
            rows: vec![Registration::default(); capacity].into_boxed_slice(),
            length: 0,
            initial_end: None,
            initial: AmbientCursor::default(),
        }
    }
    pub(super) fn reset(&mut self) {
        self.length = 0;
        self.initial_end = None;
        self.initial = AmbientCursor::default();
    }
    pub(super) fn freeze_initial(&mut self) {
        self.initial_end.get_or_insert(self.length);
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
        let registry = &mut self
            .range_mut(context.module)
            .ok_or(CallError::ConfigString)?
            .ambients;
        let slot = registry.length;
        let row = registry.rows.get_mut(slot).ok_or(CallError::Capacity)?;
        *row = Registration {
            native: sound,
            spatial_rules: context.sound_rules,
            published_sound: None,
        };
        registry.length += 1;
        Ok(slot as u32)
    }

    /// Initial map publication ends at the first PCM attachment. Later native
    /// registrations remain signon data for future clients, not broadcasts.
    pub fn publish_initial_ambients(
        &mut self,
        server: &mut Server,
        context: CallContext,
    ) -> Result<usize, CallError> {
        let bank = self.sound_bank.as_deref();
        let row = self
            .ranges
            .iter_mut()
            .find(|row| row.module == context.module)
            .ok_or(CallError::ConfigString)?;
        let Some(end) = row.ambients.initial_end else {
            return Ok(0);
        };
        let (Some(bank), Some(bindings)) = (bank, row.sound_bindings.as_ref()) else {
            return Ok(0);
        };
        let lifetime = row.lifetime.ok_or(CallError::Entity)?;
        if server.entities.resolve(lifetime).is_none() {
            return Err(CallError::Entity);
        }
        publish(
            &mut server.events,
            &mut row.ambients.rows[..end],
            &mut row.ambients.initial,
            context,
            row.native_rules,
            lifetime,
            None,
            SoundAction::StartLoop,
            bank,
            bindings,
        )
    }

    /// Explicit late-client signon uses its own cursor and the current retained
    /// registration count. A missing PCM bank never consumes registrations.
    pub fn publish_ambients(
        &mut self,
        server: &mut Server,
        context: CallContext,
        client: ClientId,
        cursor: &mut AmbientCursor,
    ) -> Result<usize, CallError> {
        self.publish_target_ambients(
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
        self.publish_target_ambients(
            server,
            context,
            client,
            &mut AmbientCursor::default(),
            SoundAction::Stop,
        )
    }

    fn publish_target_ambients(
        &mut self,
        server: &mut Server,
        context: CallContext,
        client: Option<ClientId>,
        cursor: &mut AmbientCursor,
        action: SoundAction,
    ) -> Result<usize, CallError> {
        let bank = self.sound_bank.as_deref();
        let row = self
            .ranges
            .iter_mut()
            .find(|row| row.module == context.module)
            .ok_or(CallError::ConfigString)?;
        let (Some(bank), Some(bindings)) = (bank, row.sound_bindings.as_ref()) else {
            return Ok(0);
        };
        let lifetime = row.lifetime.ok_or(CallError::Entity)?;
        if action == SoundAction::StartLoop && server.entities.resolve(lifetime).is_none() {
            return Err(CallError::Entity);
        }
        let end = row.ambients.length;
        publish(
            &mut server.events,
            &mut row.ambients.rows[..end],
            cursor,
            context,
            row.native_rules,
            lifetime,
            client,
            action,
            bank,
            bindings,
        )
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "One publisher serves initial, late and stop signon paths without copying the admission loop."
)]
fn publish(
    events: &mut EventRing,
    rows: &mut [Registration],
    cursor: &mut AmbientCursor,
    context: CallContext,
    source: RuleSetId,
    lifetime: EntityId,
    client: Option<ClientId>,
    action: SoundAction,
    bank: &qa_audio::Bank,
    bindings: &qa_audio::SoundBindings,
) -> Result<usize, CallError> {
    cursor.bind(context.module, lifetime, client)?;
    let mut published = 0;
    while let Some(row) = rows.get_mut(cursor.next) {
        let sound = if action == SoundAction::Stop {
            row.published_sound
        } else {
            bindings
                .sound(row.native.source_ordinal)
                .filter(|sound| bank.has_loop_marker(*sound))
        };
        let Some(sound) = sound else {
            cursor.next += 1;
            continue;
        };
        let position = Vec3(row.native.origin.map(qa_core::math::short_to_eighth));
        let event = SoundEvent {
            sound,
            source_ordinal: Some(row.native.source_ordinal),
            spatial_rules: row.spatial_rules,
            entity: Some(SoundEmitter::Static {
                namespace: ResourceNamespace::Module(context.module),
                lifetime,
                slot: cursor.next as u32,
            }),
            channel: 0,
            position,
            volume: f32::from(row.native.volume) / 255.0,
            attenuation: f32::from(row.native.attenuation) / 64.0,
            action,
            client: None,
            time_offset: 0.0,
            positioned: true,
        };
        let delivery = NativeDelivery {
            module: context.module,
            source,
            target: client.map_or(NativeTarget::All, NativeTarget::Client),
            origin: position,
            reliable: true,
            dupe_key: 0,
            emission: Some(NativeEmission {
                module_lifetime: lifetime,
                provider_frame: context.server_frame,
            }),
            check_portals: false,
        };
        if events
            .push_delivered(delivery, FrameEvent::Sound(event))
            .is_err()
        {
            break;
        }
        if action == SoundAction::StartLoop {
            row.published_sound = Some(sound);
        }
        cursor.next += 1;
        published += 1;
    }
    Ok(published)
}
