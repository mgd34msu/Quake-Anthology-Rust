//! Component presentation media delivery.
//!
//! Donor: `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/component-media.ts`
//! (`presentationAudioControl`, `preparePresentationAudio`,
//! `preparePresentationShaders`, `primaryShaderControl`,
//! `componentMediaControl`).
//! Retained local requests apply in the same chronology as incoming
//! source controls. Presentation state, audio, and assets arrive through
//! the [`PresentationMediaEvents`], [`ComponentMediaAudio`], and
//! [`ComponentMediaAssets`] traits (donors `presentation-state.ts`,
//! `audio.ts`, and `assets.ts`); source events arrive as the absorbed
//! [`ComponentMediaEvent`] pick (donor `SimulationPresentationEvent`,
//! owned by the simulation lane). Sync delivery never suspends, so the
//! shader-delivery serialization collapses to a direct drain.

use std::rc::Rc;

use qa_content::contract::{ComponentPresentationMediaRequest, ContentId, PresentationOwner};
use qa_core::identity::SeatId;
use thiserror::Error;

/// Component media failure.
#[derive(Debug, Error)]
pub enum ComponentMediaError<E> {
    /// Shader consumer retired before delivery.
    #[error("Shader consumer is retired")]
    RetiredShader,
    /// Component media consumer retired before delivery.
    #[error("Component media consumer is retired")]
    RetiredMedia,
    /// Shader destination changed before local media delivery.
    #[error("Shader destination changed before local media delivery")]
    StaleDestination,
    /// Audio or asset service failure.
    #[error(transparent)]
    Service(E),
}

/// Absorbed presentation event pick consumed by component media.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentMediaEvent {
    /// Event source.
    pub kind: ComponentMediaEventKind,
    /// Presentation sequence.
    pub sequence: i64,
}

/// Consumed media control sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComponentMediaEventKind {
    /// Owner lifecycle (`presentation-owner`).
    PresentationOwner,
    /// Music control (`music`).
    Music,
    /// Q2 event; `music` marks `music` events.
    Q2 {
        /// Music event.
        music: bool,
    },
    /// Q1 level event; `finale` marks `finale` events.
    Q1Level {
        /// Finale event.
        finale: bool,
    },
    /// Any other source.
    Other,
}

/// Local presentation media (`LocalPresentationMedia`).
#[derive(Debug, Clone, PartialEq)]
pub struct LocalPresentationMedia {
    /// Presenting owner, if component-owned.
    pub owner: Option<PresentationOwner>,
    /// Source content.
    pub content: ContentId,
    /// Presentation sequence.
    pub sequence: i64,
    /// Event time in seconds.
    pub seconds: f64,
    /// Media request.
    pub event: ComponentPresentationMediaRequest,
}

/// Retained presentation: a source event or local media.
#[derive(Debug, Clone, PartialEq)]
pub enum RetainedMedia {
    /// Retained source event.
    Source(ComponentMediaEvent),
    /// Retained local media.
    Local(LocalPresentationMedia),
}

impl RetainedMedia {
    fn sequence(&self) -> i64 {
        match self {
            Self::Source(event) => event.sequence,
            Self::Local(media) => media.sequence,
        }
    }

    fn is_owner(&self) -> bool {
        matches!(
            self,
            Self::Source(ComponentMediaEvent {
                kind: ComponentMediaEventKind::PresentationOwner,
                ..
            })
        )
    }
}

/// Liveness check retained by published local media.
pub type MediaCurrent = Rc<dyn Fn() -> bool>;

/// Absorbed presentation-state surface used by media delivery.
pub trait PresentationMediaEvents {
    /// Enable local media output.
    fn enable_local_media(&mut self);
    /// Retained local media, oldest first.
    fn pending_local_media(&self) -> Vec<RetainedMedia>;
    /// Acknowledge one retained request.
    fn acknowledge_local_media(&mut self, item: &RetainedMedia, committed: bool);
    /// Whether a retained request is still current.
    fn local_media_current(&self, item: &RetainedMedia) -> bool;
    /// Highest applied media sequence.
    fn applied_media_sequence(&self) -> i64;
    /// Record an applied media sequence.
    fn applied_media(&mut self, sequence: i64);
    /// Record an applied shader remap.
    fn applied_shader(&mut self, original: &str, sequence: i64);
    /// Resolve a shader replay for a pending remap.
    fn resolve_shader_replay(&self, pending: &LocalPresentationMedia) -> Option<LocalPresentationMedia>;
    /// Publish local media.
    fn publish_local_media(
        &mut self,
        owner: Option<&PresentationOwner>,
        content: &ContentId,
        event: &ComponentPresentationMediaRequest,
        initializing: bool,
        current: Option<MediaCurrent>,
    );
}

/// Audio audience (`AudioAudience` pick).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaAudience {
    /// World audience.
    World,
    /// One seat.
    Seat(SeatId),
}

/// Absorbed audio surface used by media delivery.
pub trait ComponentMediaAudio {
    /// Audio failure.
    type Error;
    /// Play component media while `active` holds.
    fn play_component_media(
        &mut self,
        media: &LocalPresentationMedia,
        active: &dyn Fn() -> bool,
    ) -> Result<(), Self::Error>;
    /// Receive presentation events for an audience.
    fn receive(
        &mut self,
        events: &[ComponentMediaEvent],
        audience: MediaAudience,
        music: bool,
    ) -> Result<(), Self::Error>;
}

/// Absorbed seat audio batch (`ApplicationAudioSeatEvents` pick).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentMediaSeatEvents {
    /// Presenting seat.
    pub seat: SeatId,
    /// Seat events.
    pub events: Vec<ComponentMediaEvent>,
    /// Music enabled.
    pub music: bool,
}

/// Shader registry source for one remap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaShaderSource {
    /// World registry selected.
    pub world: bool,
    /// Provider content, when a provider registry is selected.
    pub content: Option<ContentId>,
}

/// Shader remap outcome (`ShaderRemapResult`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShaderRemapOutcome {
    /// Remap committed.
    Committed,
    /// Remap unchanged.
    Unchanged,
    /// Remap went stale.
    Stale,
}

/// Absorbed asset surface used by shader delivery.
pub trait ComponentMediaAssets {
    /// Asset failure.
    type Error;
    /// Engine-behavior content (`content.recipe.engineBehavior.content`).
    fn engine_behavior_content(&self) -> ContentId;
    /// World shader registry.
    fn world_shaders(&self) -> MediaShaderSource;
    /// One provider's shader registry.
    fn provider_shaders(&mut self, content: &ContentId) -> Result<MediaShaderSource, Self::Error>;
    /// Remap one shader.
    fn remap_shader(
        &mut self,
        original: &str,
        replacement: &str,
        time_offset: f64,
        source: &MediaShaderSource,
        current: &dyn Fn() -> bool,
    ) -> Result<ShaderRemapOutcome, Self::Error>;
}

/// Whether a source event controls presentation audio
/// (`presentationAudioControl`).
#[must_use]
pub fn presentation_audio_control(source: &ComponentMediaEvent) -> bool {
    matches!(
        source.kind,
        ComponentMediaEventKind::PresentationOwner
            | ComponentMediaEventKind::Music
            | ComponentMediaEventKind::Q2 { music: true }
            | ComponentMediaEventKind::Q1Level { finale: true }
    )
}

enum OrderedMedia {
    Pending(RetainedMedia),
    Fresh(ComponentMediaEvent),
}

impl OrderedMedia {
    fn sequence(&self) -> i64 {
        match self {
            Self::Pending(item) => item.sequence(),
            Self::Fresh(event) => event.sequence,
        }
    }

    fn is_owner(&self) -> bool {
        match self {
            Self::Pending(item) => item.is_owner(),
            Self::Fresh(event) => matches!(event.kind, ComponentMediaEventKind::PresentationOwner),
        }
    }
}

/// Apply retained local requests in source chronology
/// (`preparePresentationAudio`).
pub fn prepare_presentation_audio<A: ComponentMediaAudio>(
    events: &mut impl PresentationMediaEvents,
    audio: &mut A,
    source: &[ComponentMediaEvent],
    seats: &[ComponentMediaSeatEvents],
) -> Result<(), A::Error> {
    let pending: Vec<RetainedMedia> = events
        .pending_local_media()
        .into_iter()
        .filter(|request| match request {
            RetainedMedia::Local(media) => {
                !matches!(media.event, ComponentPresentationMediaRequest::ShaderRemap { .. })
            }
            RetainedMedia::Source(_) => true,
        })
        .collect();
    let mut ordered: Vec<OrderedMedia> = source
        .iter()
        .filter(|event| presentation_audio_control(event))
        .cloned()
        .map(OrderedMedia::Fresh)
        .chain(pending.into_iter().map(OrderedMedia::Pending))
        .collect();
    ordered.sort_by_key(OrderedMedia::sequence);
    for request in &ordered {
        if !request.is_owner() && request.sequence() < events.applied_media_sequence() {
            if let OrderedMedia::Pending(item) = request {
                events.acknowledge_local_media(item, true);
            }
            continue;
        }
        match request {
            OrderedMedia::Pending(item) => {
                if !events.local_media_current(item) {
                    continue;
                }
                match item {
                    RetainedMedia::Local(media) => {
                        if matches!(media.event, ComponentPresentationMediaRequest::ShaderRemap { .. }) {
                            continue;
                        }
                        let check = item.clone();
                        audio.play_component_media(media, &|| events.local_media_current(&check))?;
                    }
                    RetainedMedia::Source(event) => {
                        audio.receive(std::slice::from_ref(event), MediaAudience::World, true)?;
                    }
                }
                events.acknowledge_local_media(item, true);
            }
            OrderedMedia::Fresh(event) => {
                audio.receive(std::slice::from_ref(event), MediaAudience::World, true)?;
            }
        }
        if !request.is_owner() {
            events.applied_media(request.sequence());
        }
    }
    for batch in seats {
        let filtered: Vec<ComponentMediaEvent> = batch
            .events
            .iter()
            .filter(|event| presentation_audio_control(event))
            .cloned()
            .collect();
        audio.receive(&filtered, MediaAudience::Seat(batch.seat.clone()), batch.music)?;
    }
    Ok(())
}

/// Drain pending shader remaps (`preparePresentationShaders`).
pub fn prepare_presentation_shaders<A: ComponentMediaAssets>(
    events: &mut impl PresentationMediaEvents,
    assets: &mut A,
) -> Result<(), ComponentMediaError<A::Error>> {
    loop {
        let pending = events
            .pending_local_media()
            .into_iter()
            .find_map(|request| match request {
                RetainedMedia::Local(media)
                    if matches!(media.event, ComponentPresentationMediaRequest::ShaderRemap { .. }) =>
                {
                    Some(media)
                }
                _ => None,
            });
        let Some(pending) = pending else {
            return Ok(());
        };
        let Some(request) = events.resolve_shader_replay(&pending) else {
            continue;
        };
        let ComponentPresentationMediaRequest::ShaderRemap {
            original,
            replacement,
            time_offset,
        } = &request.event
        else {
            return Err(ComponentMediaError::StaleDestination);
        };
        let check = RetainedMedia::Local(request.clone());
        if !events.local_media_current(&check) {
            events.acknowledge_local_media(&RetainedMedia::Local(request), false);
            continue;
        }
        let shaders = if original == replacement {
            assets.world_shaders()
        } else {
            assets
                .provider_shaders(&request.content)
                .map_err(ComponentMediaError::Service)?
        };
        if !events.local_media_current(&check) {
            events.acknowledge_local_media(&RetainedMedia::Local(request), false);
            continue;
        }
        let result = assets
            .remap_shader(original, replacement, *time_offset, &shaders, &|| {
                events.local_media_current(&check)
            })
            .map_err(ComponentMediaError::Service)?;
        if result == ShaderRemapOutcome::Stale && events.local_media_current(&check) {
            return Err(ComponentMediaError::StaleDestination);
        }
        let sequence = request.sequence;
        let applied_original = (*original).clone();
        events.acknowledge_local_media(&RetainedMedia::Local(request), result == ShaderRemapOutcome::Committed);
        if result != ShaderRemapOutcome::Stale {
            events.applied_shader(&applied_original, sequence);
        }
    }
}

fn parse_time_offset(offset: &str) -> f64 {
    let text = offset.trim_start();
    let (sign, text) = match text.strip_prefix('-') {
        Some(rest) => (-1.0, rest),
        None => (1.0, text.strip_prefix('+').unwrap_or(text)),
    };
    let mut prefix = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.peek() {
        if c.is_ascii_digit() {
            prefix.push(*c);
            chars.next();
        } else {
            break;
        }
    }
    if chars.peek() == Some(&'.') {
        prefix.push('.');
        chars.next();
        while let Some(c) = chars.peek() {
            if c.is_ascii_digit() {
                prefix.push(*c);
                chars.next();
            } else {
                break;
            }
        }
    }
    if prefix.is_empty() || prefix == "." {
        return 0.0;
    }
    let mut with_exponent = prefix.clone();
    if matches!(chars.peek(), Some('e' | 'E')) {
        let mut tail = String::from("e");
        chars.next();
        if matches!(chars.peek(), Some('+' | '-')) {
            tail.push(chars.next().expect("exponent sign checked"));
        }
        let mut digits = String::new();
        while let Some(c) = chars.peek() {
            if c.is_ascii_digit() {
                digits.push(*c);
                chars.next();
            } else {
                break;
            }
        }
        if digits.is_empty() {
            return with_exponent.parse::<f64>().map_or(0.0, |value| value * sign);
        }
        tail.push_str(&digits);
        with_exponent.push_str(&tail);
    }
    with_exponent.parse::<f64>().map_or(0.0, |value| value * sign)
}

/// Primary shader remap control (`primaryShaderControl`).
pub struct PrimaryShaderControl<'e, 'a, E, A> {
    events: &'e mut E,
    assets: &'a mut A,
    source_current: MediaCurrent,
}

impl<'e, 'a, E: PresentationMediaEvents, A: ComponentMediaAssets> PrimaryShaderControl<'e, 'a, E, A> {
    /// Build the control, enabling local media.
    pub fn new(events: &'e mut E, assets: &'a mut A, source_current: MediaCurrent) -> Self {
        events.enable_local_media();
        Self {
            events,
            assets,
            source_current,
        }
    }

    /// Deliver one shader remap.
    pub fn send(
        &mut self,
        original: &str,
        replacement: &str,
        offset: &str,
        initializing: bool,
        consumer_current: &MediaCurrent,
    ) -> Result<(), ComponentMediaError<A::Error>> {
        let current: MediaCurrent = {
            let source_current = Rc::clone(&self.source_current);
            let consumer_current = Rc::clone(consumer_current);
            Rc::new(move || source_current() && consumer_current())
        };
        if !current() {
            return Err(ComponentMediaError::RetiredShader);
        }
        let time_offset = parse_time_offset(offset);
        let content = self.assets.engine_behavior_content();
        self.events.publish_local_media(
            None,
            &content,
            &ComponentPresentationMediaRequest::ShaderRemap {
                original: original.to_string(),
                replacement: replacement.to_string(),
                time_offset,
            },
            initializing,
            Some(Rc::clone(&current)),
        );
        prepare_presentation_shaders(self.events, self.assets)?;
        if !current() {
            return Err(ComponentMediaError::RetiredShader);
        }
        Ok(())
    }
}

/// Absorbed mod presentation pick (`ActiveModPresentation` owner/content).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentMediaSource {
    /// Presenting owner.
    pub owner: PresentationOwner,
    /// Source content (`identity.source.content`).
    pub content: ContentId,
}

/// Component media control (`componentMediaControl`).
pub struct ComponentMediaControl<'e, 'u, 'a, E, U, A> {
    events: &'e mut E,
    audio: &'u mut U,
    assets: &'a mut A,
}

impl<'e, 'u, 'a, E: PresentationMediaEvents, U: ComponentMediaAudio, A: ComponentMediaAssets>
    ComponentMediaControl<'e, 'u, 'a, E, U, A>
{
    /// Build the control, enabling local media.
    pub fn new(events: &'e mut E, audio: &'u mut U, assets: &'a mut A) -> Self {
        events.enable_local_media();
        Self { events, audio, assets }
    }

    /// Deliver one component media request.
    pub fn send(
        &mut self,
        source: &ComponentMediaSource,
        request: &ComponentPresentationMediaRequest,
        initializing: bool,
        current: &MediaCurrent,
    ) -> Result<(), MediaControlError<U::Error, A::Error>> {
        if !current() {
            return Err(ComponentMediaError::RetiredMedia);
        }
        let replay = matches!(request, ComponentPresentationMediaRequest::ShaderRemap { .. });
        self.events.publish_local_media(
            Some(&source.owner),
            &source.content,
            request,
            initializing,
            replay.then(|| Rc::clone(current)),
        );
        if replay {
            prepare_presentation_shaders(self.events, self.assets).map_err(|error| match error {
                ComponentMediaError::Service(error) => ComponentMediaError::Service(MediaServiceError::Assets(error)),
                ComponentMediaError::RetiredShader | ComponentMediaError::RetiredMedia => {
                    ComponentMediaError::RetiredMedia
                }
                ComponentMediaError::StaleDestination => ComponentMediaError::StaleDestination,
            })?;
        } else {
            prepare_presentation_audio(self.events, self.audio, &[], &[])
                .map_err(|error| ComponentMediaError::Service(MediaServiceError::Audio(error)))?;
        }
        if !current() {
            return Err(ComponentMediaError::RetiredMedia);
        }
        Ok(())
    }
}

/// Combined audio/asset service failure.
pub type MediaControlError<U, A> = ComponentMediaError<MediaServiceError<U, A>>;

/// Combined audio/asset service failure.
#[derive(Debug, PartialEq)]
pub enum MediaServiceError<U, A> {
    /// Audio failure.
    Audio(U),
    /// Asset failure.
    Assets(A),
}

impl<U: std::fmt::Display, A: std::fmt::Display> std::fmt::Display for MediaServiceError<U, A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Audio(error) => write!(f, "{error}"),
            Self::Assets(error) => write!(f, "{error}"),
        }
    }
}

impl<U: std::fmt::Debug + std::fmt::Display, A: std::fmt::Debug + std::fmt::Display> std::error::Error
    for MediaServiceError<U, A>
{
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::ProviderId;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct StubError;

    impl std::fmt::Display for StubError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("stub service failed")
        }
    }

    impl std::error::Error for StubError {}

    struct StubEvents {
        pending: Vec<RetainedMedia>,
        acknowledged: Vec<(i64, bool)>,
        applied: Vec<i64>,
        shaders: Vec<(String, i64)>,
        published: Vec<ComponentPresentationMediaRequest>,
        live: bool,
    }

    impl PresentationMediaEvents for StubEvents {
        fn enable_local_media(&mut self) {}
        fn pending_local_media(&self) -> Vec<RetainedMedia> {
            self.pending.clone()
        }
        fn acknowledge_local_media(&mut self, item: &RetainedMedia, committed: bool) {
            self.acknowledged.push((item.sequence(), committed));
            self.pending.retain(|entry| entry.sequence() != item.sequence());
        }
        fn local_media_current(&self, _item: &RetainedMedia) -> bool {
            self.live
        }
        fn applied_media_sequence(&self) -> i64 {
            1
        }
        fn applied_media(&mut self, sequence: i64) {
            self.applied.push(sequence);
        }
        fn applied_shader(&mut self, original: &str, sequence: i64) {
            self.shaders.push((original.to_string(), sequence));
        }
        fn resolve_shader_replay(&self, pending: &LocalPresentationMedia) -> Option<LocalPresentationMedia> {
            Some(pending.clone())
        }
        fn publish_local_media(
            &mut self,
            _owner: Option<&PresentationOwner>,
            _content: &ContentId,
            event: &ComponentPresentationMediaRequest,
            _initializing: bool,
            _current: Option<MediaCurrent>,
        ) {
            self.published.push(event.clone());
        }
    }

    struct StubAudio {
        played: Vec<i64>,
        received: Vec<(usize, MediaAudience, bool)>,
    }

    impl ComponentMediaAudio for StubAudio {
        type Error = StubError;
        fn play_component_media(
            &mut self,
            media: &LocalPresentationMedia,
            _active: &dyn Fn() -> bool,
        ) -> Result<(), StubError> {
            self.played.push(media.sequence);
            Ok(())
        }
        fn receive(
            &mut self,
            events: &[ComponentMediaEvent],
            audience: MediaAudience,
            music: bool,
        ) -> Result<(), StubError> {
            self.received.push((events.len(), audience, music));
            Ok(())
        }
    }

    struct StubAssets {
        remaps: Vec<(String, String, f64)>,
        outcome: ShaderRemapOutcome,
    }

    impl ComponentMediaAssets for StubAssets {
        type Error = StubError;
        fn engine_behavior_content(&self) -> ContentId {
            ContentId("q3:classic:baseq3:1".to_string())
        }
        fn world_shaders(&self) -> MediaShaderSource {
            MediaShaderSource {
                world: true,
                content: None,
            }
        }
        fn provider_shaders(&mut self, content: &ContentId) -> Result<MediaShaderSource, StubError> {
            Ok(MediaShaderSource {
                world: false,
                content: Some(content.clone()),
            })
        }
        fn remap_shader(
            &mut self,
            original: &str,
            replacement: &str,
            time_offset: f64,
            _source: &MediaShaderSource,
            _current: &dyn Fn() -> bool,
        ) -> Result<ShaderRemapOutcome, StubError> {
            self.remaps
                .push((original.to_string(), replacement.to_string(), time_offset));
            Ok(self.outcome)
        }
    }

    fn content() -> ContentId {
        ContentId("q3:classic:baseq3:1".to_string())
    }

    fn local(sequence: i64, event: ComponentPresentationMediaRequest) -> RetainedMedia {
        RetainedMedia::Local(LocalPresentationMedia {
            owner: None,
            content: content(),
            sequence,
            seconds: 1.0,
            event,
        })
    }

    fn music_request() -> ComponentPresentationMediaRequest {
        ComponentPresentationMediaRequest::Music {
            intro: "intro".to_string(),
            loop_track: "loop".to_string(),
        }
    }

    #[test]
    fn audio_control_matches_music_sources() {
        let owner = ComponentMediaEvent {
            kind: ComponentMediaEventKind::PresentationOwner,
            sequence: 0,
        };
        assert!(presentation_audio_control(&owner));
        let music = ComponentMediaEvent {
            kind: ComponentMediaEventKind::Music,
            sequence: 0,
        };
        assert!(presentation_audio_control(&music));
        let q2 = ComponentMediaEvent {
            kind: ComponentMediaEventKind::Q2 { music: true },
            sequence: 0,
        };
        assert!(presentation_audio_control(&q2));
        let q2_other = ComponentMediaEvent {
            kind: ComponentMediaEventKind::Q2 { music: false },
            sequence: 0,
        };
        assert!(!presentation_audio_control(&q2_other));
        let finale = ComponentMediaEvent {
            kind: ComponentMediaEventKind::Q1Level { finale: true },
            sequence: 0,
        };
        assert!(presentation_audio_control(&finale));
        let other = ComponentMediaEvent {
            kind: ComponentMediaEventKind::Other,
            sequence: 0,
        };
        assert!(!presentation_audio_control(&other));
    }

    #[test]
    fn audio_preparation_orders_and_gates_by_sequence() {
        let mut events = StubEvents {
            pending: vec![
                local(0, music_request()),
                local(3, music_request()),
                local(
                    4,
                    ComponentPresentationMediaRequest::ShaderRemap {
                        original: "a".to_string(),
                        replacement: "b".to_string(),
                        time_offset: 0.0,
                    },
                ),
            ],
            acknowledged: Vec::new(),
            applied: Vec::new(),
            shaders: Vec::new(),
            published: Vec::new(),
            live: true,
        };
        let mut audio = StubAudio {
            played: Vec::new(),
            received: Vec::new(),
        };
        let source = vec![ComponentMediaEvent {
            kind: ComponentMediaEventKind::Music,
            sequence: 2,
        }];
        prepare_presentation_audio(&mut events, &mut audio, &source, &[]).unwrap();
        // Stale pending requests acknowledge without playing; shader
        // remaps never reach audio preparation.
        assert_eq!(audio.played, vec![3]);
        assert_eq!(audio.received.len(), 1);
        assert!(events.acknowledged.contains(&(0, true)));
        assert!(events.acknowledged.contains(&(3, true)));
        assert_eq!(events.applied, vec![2, 3]);
    }

    #[test]
    fn shader_drain_commits_and_records() {
        let mut events = StubEvents {
            pending: vec![local(
                5,
                ComponentPresentationMediaRequest::ShaderRemap {
                    original: "a".to_string(),
                    replacement: "b".to_string(),
                    time_offset: 1.5,
                },
            )],
            acknowledged: Vec::new(),
            applied: Vec::new(),
            shaders: Vec::new(),
            published: Vec::new(),
            live: true,
        };
        let mut assets = StubAssets {
            remaps: Vec::new(),
            outcome: ShaderRemapOutcome::Committed,
        };
        prepare_presentation_shaders(&mut events, &mut assets).unwrap();
        assert_eq!(assets.remaps, vec![("a".to_string(), "b".to_string(), 1.5)]);
        assert_eq!(events.shaders, vec![("a".to_string(), 5)]);
        assert!(events.acknowledged.contains(&(5, true)));
    }

    #[test]
    fn stale_remaps_and_retired_consumers_fail() {
        let mut events = StubEvents {
            pending: vec![local(
                5,
                ComponentPresentationMediaRequest::ShaderRemap {
                    original: "a".to_string(),
                    replacement: "b".to_string(),
                    time_offset: 0.0,
                },
            )],
            acknowledged: Vec::new(),
            applied: Vec::new(),
            shaders: Vec::new(),
            published: Vec::new(),
            live: true,
        };
        let mut assets = StubAssets {
            remaps: Vec::new(),
            outcome: ShaderRemapOutcome::Stale,
        };
        assert!(matches!(
            prepare_presentation_shaders(&mut events, &mut assets).unwrap_err(),
            ComponentMediaError::StaleDestination
        ));
        let mut assets = StubAssets {
            remaps: Vec::new(),
            outcome: ShaderRemapOutcome::Committed,
        };
        let mut control = PrimaryShaderControl::new(&mut events, &mut assets, Rc::new(|| false));
        let consumer: MediaCurrent = Rc::new(|| true);
        assert!(matches!(
            control.send("a", "b", "1.5", false, &consumer).unwrap_err(),
            ComponentMediaError::RetiredShader
        ));
    }

    #[test]
    fn component_control_publishes_and_delivers() {
        let mut events = StubEvents {
            pending: Vec::new(),
            acknowledged: Vec::new(),
            applied: Vec::new(),
            shaders: Vec::new(),
            published: Vec::new(),
            live: true,
        };
        let mut audio = StubAudio {
            played: Vec::new(),
            received: Vec::new(),
        };
        let mut assets = StubAssets {
            remaps: Vec::new(),
            outcome: ShaderRemapOutcome::Committed,
        };
        let mut control = ComponentMediaControl::new(&mut events, &mut audio, &mut assets);
        let source = ComponentMediaSource {
            owner: PresentationOwner {
                provider: ProviderId::new("test", "media"),
                generation: 1,
            },
            content: content(),
        };
        let current: MediaCurrent = Rc::new(|| true);
        control.send(&source, &music_request(), false, &current).unwrap();
        assert_eq!(control.events.published.len(), 1);
        let retired: MediaCurrent = Rc::new(|| false);
        assert!(matches!(
            control.send(&source, &music_request(), false, &retired).unwrap_err(),
            ComponentMediaError::RetiredMedia
        ));
    }
}
