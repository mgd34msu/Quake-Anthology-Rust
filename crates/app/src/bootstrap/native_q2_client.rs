//! Native Quake II client presentation from source host messages.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/native-q2-client.ts`
//! (`NativeQ2ClientPresentation`). The reader, configstring layout, and fog
//! update reuse workspace siblings; the source world
//! ([`q2_native_world`](super::simulation::q2_native_world)) and the
//! service-record translation/printing
//! ([`translate_q2_service_records`](super::network::q2_service_presentation::translate_q2_service_records),
//! [`q2_service_print`](super::network::q2_service_presentation::q2_service_print))
//! stay behind [`NativeQ2World`] and [`NativeQ2ServiceTranslation`], which
//! carry the host-side reader state those functions need.

use qa_content::contract::ContentId;
use qa_content::q2::foundation::host::Q2Edition;
use qa_content::q2::rerelease::types::{create_q2_fog, Q2FogState};
use qa_core::identity::ActorId;
use qa_net::protocol::ProtocolIdentity;
use qa_net::q2_net::{Q2NetError, Q2ServerMessageOptions, Q2ServerMessageReader, Q2ServerRecord};
use qa_net::q2_variants::FogData;
use std::collections::{HashMap, HashSet};
use thiserror::Error;

use crate::bootstrap::media::rerelease_presentation::fog::q2_fog_from_wire;
use crate::bootstrap::network::q2_layout::{q2_application_layout, Q2ApplicationLayout, Q2LayoutError};

/// Failure of native Q2 client presentation.
#[derive(Debug, Error)]
pub enum NativeQ2ClientError {
    /// Presentation misuse.
    #[error("{0}")]
    Presentation(String),
    /// Layout failure.
    #[error(transparent)]
    Layout(#[from] Q2LayoutError),
    /// Message decode failure.
    #[error(transparent)]
    Net(#[from] Q2NetError),
}

/// Source Q2 native world backing one client presentation.
pub trait NativeQ2World {
    /// Player state snapshot.
    type PlayerState: Clone;
    /// Entity state snapshot.
    type EntityState: Clone;
    /// World edition.
    fn edition(&self) -> Q2Edition;
    /// Player actor in a source slot.
    fn actor(&self, slot: u32) -> Option<ActorId>;
    /// World configstrings.
    fn configstrings(&self) -> Vec<(u32, String)>;
    /// Player state in a source slot.
    fn player_state(&self, slot: u32) -> Self::PlayerState;
    /// Whether an entity slot is active.
    fn entity_active(&self, slot: u32) -> bool;
    /// Entity state in a slot.
    fn entity_state(&self, slot: u32) -> Self::EntityState;
}

/// Service-record translation and printing over a presentation host.
pub trait NativeQ2ServiceTranslation<W: NativeQ2World> {
    /// Presentation event.
    type Event;
    /// Translate decoded records, emitting presentation events.
    fn translate(records: Vec<Q2ServerRecord>, host: &mut NativeQ2ServiceHost<'_, W, Self>) -> Vec<Q2ServerRecord>
    where
        Self: Sized;
    /// Print a service message at a level.
    fn print(host: &mut NativeQ2ServiceHost<'_, W, Self>, level: i32, text: &str)
    where
        Self: Sized;
}

/// Owned service-host state, reclaimed after each call.
pub struct HostParts<E> {
    content: ContentId,
    seconds: f64,
    sequence: u64,
    edition: Q2Edition,
    actor: ActorId,
    source_slot: u32,
    configs: HashMap<u32, String>,
    inventory: Vec<i32>,
    layout: String,
    fog: Q2FogState,
    image_offset: u32,
    sound_offset: u32,
    player_skin_offset: u32,
    events: Vec<E>,
}

/// Service presentation host: configstrings, inventory, layout, fog, events.
pub struct NativeQ2ServiceHost<'w, W: NativeQ2World, T: NativeQ2ServiceTranslation<W>> {
    parts: HostParts<T::Event>,
    world: &'w W,
}

impl<W: NativeQ2World, T: NativeQ2ServiceTranslation<W>> NativeQ2ServiceHost<'_, W, T> {
    /// Consume the host, returning its owned state.
    #[must_use]
    pub fn into_parts(self) -> HostParts<T::Event> {
        self.parts
    }
}

impl<W: NativeQ2World, T: NativeQ2ServiceTranslation<W>> NativeQ2ServiceHost<'_, W, T> {
    /// Source content.
    #[must_use]
    pub fn content(&self) -> &ContentId {
        &self.parts.content
    }

    /// Presentation time in seconds.
    #[must_use]
    pub fn seconds(&self) -> f64 {
        self.parts.seconds
    }

    /// Next event sequence number.
    pub fn next_sequence(&mut self) -> u64 {
        let sequence = self.parts.sequence;
        self.parts.sequence += 1;
        sequence
    }

    /// World edition.
    #[must_use]
    pub fn edition(&self) -> Q2Edition {
        self.parts.edition
    }

    /// Presenting player actor and source slot.
    #[must_use]
    pub fn player(&self) -> (&ActorId, u32) {
        (&self.parts.actor, self.parts.source_slot)
    }

    /// Player actor in a source slot.
    #[must_use]
    pub fn actor(&self, slot: u32) -> Option<ActorId> {
        self.world.actor(slot)
    }

    /// Active entity state in a slot, if active.
    #[must_use]
    pub fn entity(&self, slot: u32) -> Option<W::EntityState> {
        if self.world.entity_active(slot) {
            Some(self.world.entity_state(slot))
        } else {
            None
        }
    }

    /// Update fog from wire data.
    pub fn fog(&mut self, value: &FogData) -> Q2FogState {
        self.parts.fog = q2_fog_from_wire(&self.parts.fog, value);
        self.parts.fog
    }

    /// Image configstring offset.
    #[must_use]
    pub fn image_config_offset(&self) -> u32 {
        self.parts.image_offset
    }

    /// Sound configstring offset.
    #[must_use]
    pub fn sound_config_offset(&self) -> u32 {
        self.parts.sound_offset
    }

    /// Player-skin configstring offset.
    #[must_use]
    pub fn player_skin_config_offset(&self) -> u32 {
        self.parts.player_skin_offset
    }

    /// Read a configstring.
    #[must_use]
    pub fn configstring(&self, index: u32) -> Option<&str> {
        self.parts.configs.get(&index).map(String::as_str)
    }

    /// Write a configstring.
    pub fn set_configstring(&mut self, index: u32, value: String) {
        self.parts.configs.insert(index, value);
    }

    /// Replace the inventory counts.
    pub fn set_inventory(&mut self, counts: Vec<i32>) {
        self.parts.inventory = counts;
    }

    /// Replace the layout text.
    pub fn set_layout(&mut self, value: String) {
        self.parts.layout = value;
    }

    /// Emit a presentation event.
    pub fn emit(&mut self, event: T::Event) {
        self.parts.events.push(event);
    }
}

/// Client presentation state decoded from the existing source host's recipient-filtered messages.
pub struct NativeQ2ClientPresentation<W: NativeQ2World, T: NativeQ2ServiceTranslation<W>> {
    world: W,
    source_slot: u32,
    actor: ActorId,
    content: ContentId,
    reader: Q2ServerMessageReader,
    layout: Q2ApplicationLayout,
    configs: HashMap<u32, String>,
    pending: Vec<T::Event>,
    sequence: u64,
    counts: Vec<i32>,
    layout_text: String,
    fog: Q2FogState,
}

impl<W: NativeQ2World, T: NativeQ2ServiceTranslation<W>> NativeQ2ClientPresentation<W, T> {
    /// Create a presentation for an admitted source player.
    pub fn new(world: W, source_slot: u32, actor: ActorId, content: ContentId) -> Result<Self, NativeQ2ClientError> {
        if world.actor(source_slot).as_ref() != Some(&actor) {
            return Err(NativeQ2ClientError::Presentation(
                "Native client presentation requires its admitted source player".to_owned(),
            ));
        }
        let configs = world.configstrings().into_iter().collect();
        // Game imports produce multicast FLOAT records, independently of the selected client wire protocol.
        let protocol = if world.edition() == Q2Edition::Rerelease {
            ProtocolIdentity::Q2Rerelease
        } else {
            ProtocolIdentity::Q2Classic
        };
        let layout = q2_application_layout(protocol)?;
        let reader = Q2ServerMessageReader::new(
            protocol,
            Q2ServerMessageOptions {
                max_config_strings: layout.max_config_strings.min(u32::from(u16::MAX)) as u16,
                inventory_slots: 256,
                ..Default::default()
            },
            HashSet::new(),
            None,
        )?;
        Ok(Self {
            world,
            source_slot,
            actor,
            content,
            reader,
            layout,
            configs,
            pending: Vec::new(),
            sequence: 1,
            counts: Vec::new(),
            layout_text: String::new(),
            fog: create_q2_fog(),
        })
    }

    /// Borrow the world.
    #[must_use]
    pub fn world(&self) -> &W {
        &self.world
    }

    /// Configstrings.
    #[must_use]
    pub fn configstrings(&self) -> &HashMap<u32, String> {
        &self.configs
    }

    /// Inventory counts.
    #[must_use]
    pub fn inventory(&self) -> &[i32] {
        &self.counts
    }

    /// Layout text.
    #[must_use]
    pub fn layout_text(&self) -> &str {
        &self.layout_text
    }

    /// Player state in the source slot.
    #[must_use]
    pub fn player_state(&self) -> W::PlayerState {
        self.world.player_state(self.source_slot)
    }

    fn begin(&mut self, seconds: f64) -> HostParts<T::Event> {
        HostParts {
            content: self.content.clone(),
            seconds,
            sequence: self.sequence,
            edition: self.world.edition(),
            actor: self.actor.clone(),
            source_slot: self.source_slot,
            configs: std::mem::take(&mut self.configs),
            inventory: std::mem::take(&mut self.counts),
            layout: std::mem::take(&mut self.layout_text),
            fog: self.fog,
            image_offset: self.layout.images,
            sound_offset: self.layout.sounds,
            player_skin_offset: self.layout.player_skins,
            events: Vec::new(),
        }
    }

    fn reclaim(&mut self, parts: HostParts<T::Event>) {
        self.sequence = parts.sequence;
        self.configs = parts.configs;
        self.counts = parts.inventory;
        self.layout_text = parts.layout;
        self.fog = parts.fog;
        self.pending.extend(parts.events);
    }

    /// Decode guest messages into remaining server records.
    pub fn receive(&mut self, messages: &[Vec<u8>], seconds: f64) -> Result<Vec<Q2ServerRecord>, NativeQ2ClientError> {
        let mut remaining = Vec::new();
        for message in messages {
            let records = self.reader.read(message)?;
            let parts = self.begin(seconds);
            let mut host = NativeQ2ServiceHost {
                parts,
                world: &self.world,
            };
            let kept = T::translate(records, &mut host);
            let parts = host.into_parts();
            self.reclaim(parts);
            remaining.extend(kept);
        }
        Ok(remaining)
    }

    /// Print a service message at a level.
    pub fn print(&mut self, level: i32, text: &str, seconds: f64) {
        let parts = self.begin(seconds);
        let mut host = NativeQ2ServiceHost {
            parts,
            world: &self.world,
        };
        T::print(&mut host, level, text);
        let parts = host.into_parts();
        self.reclaim(parts);
    }

    /// Drain pending presentation events.
    pub fn take_events(&mut self) -> Vec<T::Event> {
        std::mem::take(&mut self.pending)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;

    struct FakeWorld {
        owner: IdentityOwner,
        edition: Q2Edition,
    }

    impl NativeQ2World for FakeWorld {
        type PlayerState = u32;
        type EntityState = u32;

        fn edition(&self) -> Q2Edition {
            self.edition
        }

        fn actor(&self, slot: u32) -> Option<ActorId> {
            (slot == 0).then(|| self.owner.actor(0, 0))
        }

        fn configstrings(&self) -> Vec<(u32, String)> {
            vec![(1, "map".to_owned())]
        }

        fn player_state(&self, _slot: u32) -> Self::PlayerState {
            7
        }

        fn entity_active(&self, slot: u32) -> bool {
            slot == 0
        }

        fn entity_state(&self, _slot: u32) -> Self::EntityState {
            9
        }
    }

    struct FakeTranslation;

    impl NativeQ2ServiceTranslation<FakeWorld> for FakeTranslation {
        type Event = String;

        fn translate(
            records: Vec<Q2ServerRecord>,
            host: &mut NativeQ2ServiceHost<'_, FakeWorld, Self>,
        ) -> Vec<Q2ServerRecord> {
            let sequence = host.next_sequence();
            host.emit(format!("event-{sequence}"));
            host.set_layout("layout".to_owned());
            records
        }

        fn print(host: &mut NativeQ2ServiceHost<'_, FakeWorld, Self>, level: i32, text: &str) {
            host.emit(format!("print-{level}-{text}"));
        }
    }

    fn presentation() -> NativeQ2ClientPresentation<FakeWorld, FakeTranslation> {
        let owner = IdentityOwner::create("native-q2").expect("owner");
        let actor = owner.actor(0, 0);
        let world = FakeWorld {
            owner,
            edition: Q2Edition::Classic,
        };
        NativeQ2ClientPresentation::new(world, 0, actor, ContentId("q2".to_owned())).expect("presentation")
    }

    #[test]
    fn requires_the_admitted_source_player() {
        let world = FakeWorld {
            owner: IdentityOwner::create("native-q2").expect("owner"),
            edition: Q2Edition::Classic,
        };
        let foreign = world.owner.actor(1, 0);
        let err = match NativeQ2ClientPresentation::<FakeWorld, FakeTranslation>::new(
            world,
            0,
            foreign,
            ContentId("q2".to_owned()),
        ) {
            Ok(_) => panic!("foreign actor"),
            Err(err) => err,
        };
        assert_eq!(
            err.to_string(),
            "Native client presentation requires its admitted source player"
        );
    }

    #[test]
    fn exposes_world_configstrings_and_player_state() {
        let presentation = presentation();
        assert_eq!(presentation.configstrings().get(&1).map(String::as_str), Some("map"));
        assert_eq!(presentation.player_state(), 7);
        assert!(presentation.inventory().is_empty());
        assert_eq!(presentation.layout_text(), "");
    }

    #[test]
    fn print_emits_and_drains_events() {
        let mut presentation = presentation();
        presentation.print(1, "hello", 3.0);
        assert_eq!(presentation.take_events(), vec!["print-1-hello".to_owned()]);
        assert!(presentation.take_events().is_empty());
    }
}
