//! Quake sky and scoreboard presentation with retained source events.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/q1-service-presentation.ts`
//! (`Q1ClientRow`, `Q1ServiceAssets`, `updateQ1ClientMetadata`, `Q1ServicePresentation`).
//! Sky faces load in [`SKY_FACE_SUFFIXES`](qa_client::materials::sky::SKY_FACE_SUFFIXES)
//! order into the ported [`Q2SkyView`](qa_client::render::scene::q2_sky::Q2SkyView), which
//! matches the donor sky value field for field. The event stream
//! ([`SimulationPresentationEvent`](super::simulation::types::SimulationPresentationEvent),
//! `./simulation/types.ts` port) is shimmed minimally below: this class only
//! reads owner lifecycle plus the `q1-sky`/`q1-client` owner/content/recipient/sequence
//! envelope and the metadata event. Async loads become the sync [`Q1ServiceAssets`] seam,
//! and the refresh applier becomes an explicit [`Q1SkyRefresh`] commit; selection identity
//! is a generation id so stale commits drop exactly like the donor's identity check.

use std::collections::HashMap;

use qa_client::materials::sky::SKY_FACE_SUFFIXES;
use qa_client::render::scene::q2_sky::Q2SkyView;
use qa_client::render::types::RendererImage;
use qa_content::contract::{same_presentation_owner, ContentId, PresentationOwner};
use qa_core::identity::ActorId;
use qa_core::math::vec3;
use thiserror::Error;

/// One scoreboard row (donor `Q1ClientRow`).
///
/// Source slots are scoreboard indices, not engine actors or platform identities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q1ClientRow {
    /// Scoreboard slot.
    pub slot: u32,
    /// Player name.
    pub name: String,
    /// Packed top/bottom colors.
    pub colors: i32,
    /// Frag count.
    pub frags: i32,
    /// Ping in milliseconds.
    pub ping: Option<i32>,
    /// Social handle.
    pub social: Option<String>,
    /// Extended player info.
    pub player_info: Option<String>,
}

/// One client metadata update (donor `Q1ClientMetadataEvent`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Q1ClientMetadataEvent {
    /// Name change.
    Name {
        /// Scoreboard slot.
        slot: u32,
        /// New name.
        value: String,
    },
    /// Color change.
    Colors {
        /// Scoreboard slot.
        slot: u32,
        /// Packed colors.
        value: i32,
    },
    /// Frag change.
    Frags {
        /// Scoreboard slot.
        slot: u32,
        /// Frag count.
        value: i32,
    },
    /// Ping change.
    Ping {
        /// Scoreboard slot.
        slot: u32,
        /// Ping in milliseconds.
        value: i32,
    },
    /// Social handle change.
    Social {
        /// Scoreboard slot.
        slot: u32,
        /// New handle.
        value: String,
    },
    /// Extended info change.
    PlayerInfo {
        /// Scoreboard slot.
        slot: u32,
        /// New info.
        value: String,
    },
}

impl Q1ClientMetadataEvent {
    /// Scoreboard slot.
    #[must_use]
    pub fn slot(&self) -> u32 {
        match self {
            Self::Name { slot, .. }
            | Self::Colors { slot, .. }
            | Self::Frags { slot, .. }
            | Self::Ping { slot, .. }
            | Self::Social { slot, .. }
            | Self::PlayerInfo { slot, .. } => *slot,
        }
    }

    /// Donor event-kind spelling for retained keys.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Name { .. } => "name",
            Self::Colors { .. } => "colors",
            Self::Frags { .. } => "frags",
            Self::Ping { .. } => "ping",
            Self::Social { .. } => "social",
            Self::PlayerInfo { .. } => "player-info",
        }
    }
}

/// Fold one metadata update into a table (donor `updateQ1ClientMetadata`).
pub fn update_q1_client_metadata(table: &mut HashMap<u32, Q1ClientRow>, event: &Q1ClientMetadataEvent) {
    let row = table.entry(event.slot()).or_insert_with(|| Q1ClientRow {
        slot: event.slot(),
        name: String::new(),
        colors: 0,
        frags: 0,
        ping: None,
        social: None,
        player_info: None,
    });
    match event {
        Q1ClientMetadataEvent::Name { value, .. } => row.name = value.clone(),
        Q1ClientMetadataEvent::Colors { value, .. } => row.colors = *value,
        Q1ClientMetadataEvent::Frags { value, .. } => row.frags = *value,
        Q1ClientMetadataEvent::Ping { value, .. } => row.ping = Some(*value),
        Q1ClientMetadataEvent::Social { value, .. } => row.social = Some(value.clone()),
        Q1ClientMetadataEvent::PlayerInfo { value, .. } => row.player_info = Some(value.clone()),
    }
}

/// Presentation input the service observes (donor event-stream subset).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Q1ServiceEvent {
    /// A presentation owner refreshed its media.
    OwnerRefreshed {
        /// Refreshed activation.
        owner: PresentationOwner,
    },
    /// A presentation owner retired.
    OwnerRetired {
        /// Retired activation.
        owner: PresentationOwner,
    },
    /// A skybox selection.
    Sky {
        /// Owning activation, when component-owned.
        owner: Option<PresentationOwner>,
        /// Content the event belongs to.
        content: ContentId,
        /// Target actor, or [`None`] for the shared sky.
        recipient: Option<ActorId>,
        /// Sky name, empty to clear.
        name: String,
        /// Presentation sequence.
        sequence: u64,
    },
    /// A client metadata update.
    Client {
        /// Owning activation, when component-owned.
        owner: Option<PresentationOwner>,
        /// Content the event belongs to.
        content: ContentId,
        /// Target actor, or [`None`] for the shared table.
        recipient: Option<ActorId>,
        /// Metadata update.
        event: Q1ClientMetadataEvent,
        /// Presentation sequence.
        sequence: u64,
    },
    /// Any other event, ignored.
    Other,
}

impl Q1ServiceEvent {
    /// Owning activation, when component-owned.
    #[must_use]
    pub fn owner(&self) -> Option<&PresentationOwner> {
        match self {
            Self::Sky { owner, .. } | Self::Client { owner, .. } => owner.as_ref(),
            Self::OwnerRefreshed { .. } | Self::OwnerRetired { .. } | Self::Other => None,
        }
    }

    /// Target actor, when addressed.
    #[must_use]
    pub fn recipient(&self) -> Option<&ActorId> {
        match self {
            Self::Sky { recipient, .. } | Self::Client { recipient, .. } => recipient.as_ref(),
            Self::OwnerRefreshed { .. } | Self::OwnerRetired { .. } | Self::Other => None,
        }
    }

    /// Presentation sequence (zero for lifecycle events).
    #[must_use]
    pub fn sequence(&self) -> u64 {
        match self {
            Self::Sky { sequence, .. } | Self::Client { sequence, .. } => *sequence,
            Self::OwnerRefreshed { .. } | Self::OwnerRetired { .. } | Self::Other => 0,
        }
    }
}

/// Sky-face loading (donor `Q1ServiceAssets`, narrowed to the faces this module reads).
pub trait Q1ServiceAssets {
    /// Load one texture path, or [`None`] when absent.
    fn sky_face(&mut self, content: &ContentId, path: &str) -> Option<RendererImage>;
    /// Placeholder for missing faces.
    fn missing_image(&self) -> RendererImage;
}

/// Failure of a service operation, with donor messages.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("Q1 service presentation is closed")]
pub struct Q1ServiceClosed;

/// One sky selection (donor `SkySelection`).
#[derive(Debug, Clone, PartialEq)]
struct SkySelection {
    id: u64,
    content: ContentId,
    name: String,
    ready: bool,
    value: Option<Q2SkyView>,
}

/// Client tables for one content (donor `ClientTables`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ClientTables {
    shared: HashMap<u32, Q1ClientRow>,
    recipients: HashMap<ActorId, HashMap<u32, Q1ClientRow>>,
}

/// Which selection a refresh replacement belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
enum SkyKey {
    Shared,
    Actor(ActorId),
}

/// Loaded sky replacements awaiting commit (donor `prepareImageRefresh` applier).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1SkyRefresh {
    replacements: Vec<(SkyKey, u64, Option<Q2SkyView>)>,
}

/// Sky and scoreboard presentation (donor `Q1ServicePresentation`).
#[derive(Debug, Default)]
pub struct Q1ServicePresentation {
    shared_sky: Option<SkySelection>,
    skies: HashMap<ActorId, SkySelection>,
    tables: HashMap<ContentId, ClientTables>,
    closed: bool,
    retained: HashMap<String, Q1ServiceEvent>,
    next_id: u64,
}

impl Q1ServicePresentation {
    /// Build an empty presentation.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Receive presentation events (donor `receive`).
    pub fn receive(&mut self, events: &[Q1ServiceEvent]) -> Result<(), Q1ServiceClosed> {
        if self.closed {
            return Err(Q1ServiceClosed);
        }
        for source in events {
            match source {
                Q1ServiceEvent::OwnerRefreshed { .. } => {
                    if let Some(sky) = self.shared_sky.take() {
                        self.shared_sky = Some(self.refreshed(sky));
                    }
                    let skies: Vec<(ActorId, SkySelection)> = self.skies.drain().collect();
                    for (actor, sky) in skies {
                        let sky = self.refreshed(sky);
                        self.skies.insert(actor, sky);
                    }
                }
                Q1ServiceEvent::OwnerRetired { owner } => {
                    self.retained
                        .retain(|_, event| !same_presentation_owner(event.owner(), owner));
                    self.shared_sky = None;
                    self.skies.clear();
                    self.tables.clear();
                    let mut replay: Vec<Q1ServiceEvent> = self.retained.values().cloned().collect();
                    replay.sort_by_key(Q1ServiceEvent::sequence);
                    self.apply(&replay);
                }
                Q1ServiceEvent::Sky { recipient, .. } | Q1ServiceEvent::Client { recipient, .. } => {
                    let key = retained_key(source, recipient);
                    self.retained.insert(key, source.clone());
                    self.apply(std::slice::from_ref(source));
                }
                Q1ServiceEvent::Other => {}
            }
        }
        Ok(())
    }

    /// Clone a selection with a fresh identity (donor `{ ...sky }`).
    fn refreshed(&mut self, mut sky: SkySelection) -> SkySelection {
        sky.id = self.claim_id();
        sky
    }

    /// Claim a fresh selection identity.
    fn claim_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        id
    }

    /// Fold events into skies and tables (donor `apply`).
    fn apply(&mut self, events: &[Q1ServiceEvent]) {
        for source in events {
            match source {
                Q1ServiceEvent::Sky {
                    content,
                    recipient,
                    name,
                    ..
                } => {
                    let id = self.claim_id();
                    let value = SkySelection {
                        id,
                        content: content.clone(),
                        name: name.clone(),
                        ready: false,
                        value: None,
                    };
                    if let Some(actor) = recipient {
                        self.skies.insert(actor.clone(), value);
                    } else {
                        self.shared_sky = Some(value);
                        self.skies.clear();
                    }
                }
                Q1ServiceEvent::Client {
                    content,
                    recipient,
                    event,
                    ..
                } => {
                    let tables = self.tables.entry(content.clone()).or_default();
                    if let Some(actor) = recipient {
                        let table = tables
                            .recipients
                            .entry(actor.clone())
                            .or_insert_with(|| tables.shared.clone());
                        update_q1_client_metadata(table, event);
                    } else {
                        update_q1_client_metadata(&mut tables.shared, event);
                        for table in tables.recipients.values_mut() {
                            update_q1_client_metadata(table, event);
                        }
                    }
                }
                Q1ServiceEvent::OwnerRefreshed { .. } | Q1ServiceEvent::OwnerRetired { .. } | Q1ServiceEvent::Other => {
                }
            }
        }
    }

    /// Load one selection's faces (donor `load`).
    fn load(selection: &SkySelection, assets: &mut impl Q1ServiceAssets) -> Option<Q2SkyView> {
        if selection.name.is_empty() {
            return None;
        }
        let mut images = Vec::new();
        let mut found = false;
        for suffix in SKY_FACE_SUFFIXES {
            let path = format!("gfx/env/{}{suffix}", selection.name);
            let image = assets
                .sky_face(&selection.content, &format!("{path}.tga"))
                .or_else(|| assets.sky_face(&selection.content, &format!("{path}.png")));
            found |= image.is_some();
            images.push(image.unwrap_or_else(|| assets.missing_image()));
        }
        found.then(|| Q2SkyView {
            images,
            rotation: 0.0,
            auto_rotate: false,
            axis: vec3(0.0, 0.0, 1.0),
        })
    }

    /// Publish one loaded sky when its selection is still current (donor `publish`).
    fn publish(&mut self, key: &SkyKey, id: u64, sky: Option<Q2SkyView>) {
        if self.closed {
            return;
        }
        let current = match key {
            SkyKey::Shared => self.shared_sky.as_mut(),
            SkyKey::Actor(actor) => self.skies.get_mut(actor),
        };
        if let Some(selection) = current {
            if selection.id == id {
                selection.value = sky;
                selection.ready = true;
            }
        }
    }

    /// Load every unready selection (donor `prepare`).
    pub fn prepare(&mut self, assets: &mut impl Q1ServiceAssets) -> Result<(), Q1ServiceClosed> {
        if self.closed {
            return Err(Q1ServiceClosed);
        }
        let mut pending: Vec<(SkyKey, u64, SkySelection)> = Vec::new();
        if let Some(sky) = &self.shared_sky {
            if !sky.ready {
                pending.push((SkyKey::Shared, sky.id, sky.clone()));
            }
        }
        for (actor, sky) in &self.skies {
            if !sky.ready {
                pending.push((SkyKey::Actor(actor.clone()), sky.id, sky.clone()));
            }
        }
        for (key, id, selection) in &pending {
            let sky = Self::load(selection, assets);
            self.publish(key, *id, sky);
        }
        Ok(())
    }

    /// Load replacements for every selection (donor `prepareImageRefresh`).
    pub fn prepare_image_refresh(&self, assets: &mut impl Q1ServiceAssets) -> Result<Q1SkyRefresh, Q1ServiceClosed> {
        if self.closed {
            return Err(Q1ServiceClosed);
        }
        let mut replacements = Vec::new();
        if let Some(sky) = &self.shared_sky {
            replacements.push((SkyKey::Shared, sky.id, Self::load(sky, assets)));
        }
        for (actor, sky) in &self.skies {
            replacements.push((SkyKey::Actor(actor.clone()), sky.id, Self::load(sky, assets)));
        }
        Ok(Q1SkyRefresh { replacements })
    }

    /// Commit a refresh, dropping replacements whose selection moved on.
    pub fn commit_refresh(&mut self, refresh: Q1SkyRefresh) {
        for (key, id, sky) in refresh.replacements {
            self.publish(&key, id, sky);
        }
    }

    /// Sky view for one actor (donor `view`; [`None`] is the donor `{}`).
    #[must_use]
    pub fn view(&self, actor: &ActorId) -> Option<&Q2SkyView> {
        let value = self.skies.get(actor).or(self.shared_sky.as_ref())?;
        value.value.as_ref()
    }

    /// Scoreboard rows for content and actor (donor `clients`).
    #[must_use]
    pub fn clients(&self, content: &ContentId, actor: &ActorId) -> Vec<Q1ClientRow> {
        let mut rows: Vec<Q1ClientRow> = self
            .tables
            .get(content)
            .map(|tables| {
                tables
                    .recipients
                    .get(actor)
                    .unwrap_or(&tables.shared)
                    .values()
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        rows.sort_by_key(|row| row.slot);
        rows
    }

    /// Drop one actor's skies, tables, and retained events (donor `retire`).
    pub fn retire(&mut self, actor: &ActorId) {
        self.skies.remove(actor);
        for tables in self.tables.values_mut() {
            tables.recipients.remove(actor);
        }
        self.retained.retain(|_, event| event.recipient() != Some(actor));
    }

    /// Clear every selection, table, and retained event (donor `reset`).
    pub fn reset(&mut self) {
        self.retained.clear();
        self.shared_sky = None;
        self.skies.clear();
        self.tables.clear();
    }

    /// Reset and close (donor `close`).
    pub fn close(&mut self) {
        self.reset();
        self.closed = true;
    }
}

/// Retained key for a sky or client event (donor `receive` keying).
fn retained_key(event: &Q1ServiceEvent, recipient: &Option<ActorId>) -> String {
    let recipient = match recipient {
        None => "world".to_string(),
        Some(actor) => format!("{}:{}", actor.slot(), actor.generation()),
    };
    match event {
        Q1ServiceEvent::Sky { .. } => format!("sky:{recipient}"),
        Q1ServiceEvent::Client { content, event, .. } => {
            format!("{}:{}:{}:{recipient}", content, event.slot(), event.kind())
        }
        Q1ServiceEvent::OwnerRefreshed { .. } | Q1ServiceEvent::OwnerRetired { .. } | Q1ServiceEvent::Other => {
            unreachable!("lifecycle events are never retained")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_client::render::types::{ImageSource, ResourceOwner};
    use qa_core::identity::IdentityOwner;

    struct Stub {
        faces: HashMap<String, RendererImage>,
        missing: RendererImage,
    }

    impl Stub {
        fn image(session: &qa_core::identity::SessionId, ordinal: u32) -> RendererImage {
            RendererImage {
                owner: ResourceOwner {
                    identity: 0,
                    session: session.clone(),
                    generation: 0,
                },
                ordinal,
                source: ImageSource::Generated {
                    name: "test".to_string(),
                },
                width: 64,
                height: 64,
            }
        }
    }

    impl Q1ServiceAssets for Stub {
        fn sky_face(&mut self, _content: &ContentId, path: &str) -> Option<RendererImage> {
            self.faces.get(path).cloned()
        }
        fn missing_image(&self) -> RendererImage {
            self.missing.clone()
        }
    }

    fn harness() -> (Q1ServicePresentation, Stub, IdentityOwner, ContentId) {
        let owner = IdentityOwner::create("q1-service").unwrap();
        let content = ContentId("q1:classic:dm:1".to_string());
        let assets = Stub {
            faces: HashMap::new(),
            missing: Stub::image(owner.session(), 999),
        };
        (Q1ServicePresentation::new(), assets, owner, content)
    }

    fn sky(content: &ContentId, recipient: Option<ActorId>, name: &str) -> Q1ServiceEvent {
        Q1ServiceEvent::Sky {
            owner: None,
            content: content.clone(),
            recipient,
            name: name.to_string(),
            sequence: 1,
        }
    }

    #[test]
    fn metadata_folds_into_shared_and_recipient_tables() {
        let mut table = HashMap::new();
        update_q1_client_metadata(
            &mut table,
            &Q1ClientMetadataEvent::Name {
                slot: 2,
                value: "Ranger".to_string(),
            },
        );
        update_q1_client_metadata(&mut table, &Q1ClientMetadataEvent::Frags { slot: 2, value: 7 });
        let row = &table[&2];
        assert_eq!((row.name.as_str(), row.frags), ("Ranger", 7));
    }

    #[test]
    fn recipient_tables_clone_shared_on_first_update() {
        let (mut service, _, owner, content) = harness();
        let actor = owner.actor(0, 1);
        service
            .receive(&[Q1ServiceEvent::Client {
                owner: None,
                content: content.clone(),
                recipient: None,
                event: Q1ClientMetadataEvent::Name {
                    slot: 0,
                    value: "Shared".to_string(),
                },
                sequence: 1,
            }])
            .unwrap();
        service
            .receive(&[Q1ServiceEvent::Client {
                owner: None,
                content: content.clone(),
                recipient: Some(actor.clone()),
                event: Q1ClientMetadataEvent::Frags { slot: 0, value: 3 },
                sequence: 2,
            }])
            .unwrap();
        let rows = service.clients(&content, &actor);
        assert_eq!(rows[0].name, "Shared");
        assert_eq!(rows[0].frags, 3);
        let foreign = owner.actor(1, 1);
        assert_eq!(service.clients(&content, &foreign)[0].frags, 0);
    }

    #[test]
    fn sky_prefers_tga_and_falls_back_to_missing() {
        let (mut service, mut assets, owner, content) = harness();
        assets
            .faces
            .insert("gfx/env/nightrt.tga".to_string(), Stub::image(owner.session(), 1));
        service.receive(&[sky(&content, None, "night")]).unwrap();
        service.prepare(&mut assets).unwrap();
        let actor = owner.actor(0, 1);
        let view = service.view(&actor).unwrap();
        assert_eq!(view.images.len(), 6);
        assert_eq!(view.images[0].ordinal, 1);
        assert_eq!(view.images[1].ordinal, 999);
        assert!(!view.auto_rotate);
    }

    #[test]
    fn retirement_replays_retained_events() {
        let (mut service, _, owner, content) = harness();
        let actor = owner.actor(0, 1);
        let provider = qa_core::identity::ProviderId::new("test", "sky");
        let activation = PresentationOwner {
            provider,
            generation: 1,
        };
        service
            .receive(&[Q1ServiceEvent::Sky {
                owner: Some(activation.clone()),
                content: content.clone(),
                recipient: Some(actor.clone()),
                name: "owned".to_string(),
                sequence: 1,
            }])
            .unwrap();
        assert!(service.view(&actor).is_none());
        service.receive(&[sky(&content, None, "shared")]).unwrap();
        service
            .receive(&[Q1ServiceEvent::OwnerRetired { owner: activation }])
            .unwrap();
        let mut faces = HashMap::new();
        faces.insert("gfx/env/sharedrt.tga".to_string(), Stub::image(owner.session(), 5));
        let mut assets = Stub {
            faces,
            missing: Stub::image(owner.session(), 999),
        };
        service.prepare(&mut assets).unwrap();
        assert!(service.view(&actor).is_some());
    }

    #[test]
    fn closed_service_rejects_work() {
        let (mut service, _, owner, _) = harness();
        service.close();
        assert!(service.receive(&[]).is_err());
        let mut assets = Stub {
            faces: HashMap::new(),
            missing: Stub::image(owner.session(), 0),
        };
        assert!(service.prepare(&mut assets).is_err());
    }
}
