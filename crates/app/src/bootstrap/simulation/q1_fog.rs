//! Port of Quake-Anthology-TS `src/app/bootstrap/simulation/q1-fog.ts`
//!
//! Finite world/actor fog transitions (donor `SimulationQ1Fog`), backing the
//! [`PresentationFog`](crate::bootstrap::presentation_state::PresentationFog)
//! seam. Documented folds: the actor map keys `(slot, generation)` exactly like
//! the donor `slot:generation` key but iterates in key order instead of insertion
//! order; fog doubles narrow into the ported f32
//! [`Q1FogState`](qa_client::materials::fog::Q1FogState) while sky factors and
//! context seconds stay `f64`; worldspawn parsing is fallible here so construction
//! reports [`ClientError`](qa_client::ClientError); the concrete transition reaches
//! the foreign `transition` slot through the constructor `wrap` closure.

use std::collections::{BTreeMap, HashSet};

use qa_client::materials::fog::{q1_world_fog, Q1Fog, Q1FogState, Q1FogTransition};
use qa_client::ClientError;
use qa_content::contract::ContentId;
use qa_content::value::{arr, int, num, obj, str as json_str, SaveJson, SaveReader, ValueError};
use qa_core::identity::{ActorId, SavedActorId};
use qa_core::math::vec3;
use qa_world::save::shared::validate_content_id;

use crate::bootstrap::presentation_state::{
    PresentationFog, Q1FogFields, SimulationPresentationEvent, SourcePresentationEvent,
};

/// Fog source context (donor `Context`).
#[derive(Debug, Clone, PartialEq)]
struct Q1FogContext {
    content: ContentId,
    sequence: i64,
    seconds: f64,
    source_entity: Option<i32>,
}

/// One retained fog (donor `RetainedFog`).
#[derive(Debug, Clone, PartialEq)]
struct RetainedFog {
    player: Option<ActorId>,
    state: Q1FogState,
    sky_factor: f64,
    context: Option<Q1FogContext>,
}

/// Fog simulation options (donor `SimulationQ1FogOptions`).
pub struct SimulationQ1FogOptions {
    /// Owning map content.
    pub content: ContentId,
    /// Extra accepted source contents (donor `acceptedContents`).
    pub accepted_contents: HashSet<ContentId>,
    /// Worldspawn entities for the initial fog.
    pub entities: String,
    /// Whether an actor is alive.
    pub alive: Box<dyn Fn(&ActorId) -> bool>,
}

impl SimulationQ1FogOptions {
    /// Whether a source content is accepted (donor `accepts`).
    fn accepts(&self, content: &ContentId) -> bool {
        *content == self.content || self.accepted_contents.contains(content)
    }
}

/// Finite world/actor fog transitions, not an event replay log (donor
/// `SimulationQ1Fog`).
pub struct SimulationQ1Fog<F> {
    initial: Q1FogTransition,
    global: RetainedFog,
    actors: BTreeMap<(u32, u32), RetainedFog>,
    options: SimulationQ1FogOptions,
    wrap: Box<dyn Fn(Q1FogTransition) -> F>,
}

impl<F> SimulationQ1Fog<F> {
    /// Build the simulation over worldspawn entities (donor constructor). The
    /// `wrap` closure lifts the concrete transition into the foreign payload.
    pub fn new(
        options: SimulationQ1FogOptions,
        wrap: impl Fn(Q1FogTransition) -> F + 'static,
    ) -> Result<Self, ClientError> {
        let initial = q1_world_fog(&options.entities)?;
        let mut state = Q1FogState::new();
        state.install(initial);
        Ok(Self {
            initial,
            global: RetainedFog {
                player: None,
                state,
                sky_factor: 0.5,
                context: None,
            },
            actors: BTreeMap::new(),
            options,
            wrap: Box::new(wrap),
        })
    }

    /// Reset to the initial world fog (donor `reset`).
    pub fn reset(&mut self) {
        self.global.state.install(self.initial);
        self.global.sky_factor = 0.5;
        self.global.context = None;
        self.actors.clear();
    }

    /// Retire an actor (donor `retire`).
    pub fn retire(&mut self, actor: &ActorId) {
        let id = (actor.slot(), actor.generation());
        if self
            .actors
            .get(&id)
            .is_some_and(|value| value.player.as_ref() == Some(actor))
        {
            self.actors.remove(&id);
        }
    }

    /// Resolve one fog event into presentation (donor `update`). The presentation
    /// carries the donor context fields; the fog fields carry the donor fog event.
    pub fn update(
        &mut self,
        presentation: &SimulationPresentationEvent<F>,
        fog: &Q1FogFields,
    ) -> Vec<SimulationPresentationEvent<F>> {
        if !self.options.accepts(&presentation.content) {
            return Vec::new();
        }
        if fog.player.is_none() {
            let mut output = Self::apply(&self.wrap, &mut self.global, presentation, fog);
            let mut stale = Vec::new();
            for (id, value) in &self.actors {
                if !value.player.as_ref().is_some_and(|player| (self.options.alive)(player)) {
                    stale.push(*id);
                }
            }
            for id in stale {
                self.actors.remove(&id);
            }
            for value in self.actors.values_mut() {
                output.extend(Self::apply(&self.wrap, value, presentation, fog));
            }
            return output;
        }
        let Some(player) = fog.player.as_ref() else {
            return Vec::new();
        };
        if !(self.options.alive)(player) {
            return Vec::new();
        }
        let id = (player.slot(), player.generation());
        let stale = match self.actors.get(&id) {
            Some(value) => value.player.as_ref() != Some(player),
            None => true,
        };
        if stale {
            let mut state = Q1FogState::new();
            state.install(self.global.state.capture());
            let sky_factor = self.global.sky_factor;
            self.actors.insert(
                id,
                RetainedFog {
                    player: Some(player.clone()),
                    state,
                    sky_factor,
                    context: None,
                },
            );
        }
        let value = self.actors.get_mut(&id).expect("fog actor was just retained");
        Self::apply(&self.wrap, value, presentation, fog)
    }

    /// Current fog presentation (donor `presentation`).
    pub fn presentation(&self) -> Vec<SimulationPresentationEvent<F>> {
        let mut output = Self::resolved(&self.wrap, &self.global);
        for value in self.actors.values() {
            output.extend(Self::resolved(&self.wrap, value));
        }
        output
    }

    /// Capture fog state (donor `capture`).
    pub fn capture(&self) -> SaveJson {
        let mut actors = Vec::new();
        for value in self.actors.values() {
            let Some(player) = value.player.as_ref() else {
                continue;
            };
            if !(self.options.alive)(player) {
                continue;
            }
            let mut entry = vec![("player", write_saved_actor(player))];
            entry.extend(write_retained(value));
            actors.push(obj(entry));
        }
        obj(vec![
            ("content", json_str(self.options.content.as_str())),
            ("global", obj(write_retained(&self.global))),
            ("actors", arr(actors)),
        ])
    }

    /// Restore fog state (donor `restore`).
    pub fn restore(
        &mut self,
        reader: SaveReader,
        reference: &dyn Fn(&SavedActorId) -> ActorId,
    ) -> Result<Vec<SimulationPresentationEvent<F>>, ValueError> {
        if read_content_id(&reader.field("content"))? != self.options.content {
            return Err(reader.fail("fog state belongs to another map content"));
        }
        let global = Self::read_retained(&self.options, &reader.field("global"), None)?;
        let mut actors = BTreeMap::new();
        reader.field("actors").list(|entry| -> Result<(), ValueError> {
            let player = reference(&read_saved_actor(&entry.field("player"))?);
            if !(self.options.alive)(&player) {
                return Err(entry.fail("fog actor is not alive"));
            }
            let id = (player.slot(), player.generation());
            if actors.contains_key(&id) {
                return Err(entry.fail("duplicate fog actor"));
            }
            actors.insert(id, Self::read_retained(&self.options, &entry, Some(player))?);
            Ok(())
        })?;
        self.global.state.install(global.state.capture());
        self.global.sky_factor = global.sky_factor;
        self.global.context = global.context;
        self.actors = actors;
        Ok(self.presentation())
    }

    /// Apply one fog event to retained fog (donor `apply`).
    #[allow(clippy::cast_possible_truncation)]
    fn apply(
        wrap: &dyn Fn(Q1FogTransition) -> F,
        value: &mut RetainedFog,
        presentation: &SimulationPresentationEvent<F>,
        fog: &Q1FogFields,
    ) -> Vec<SimulationPresentationEvent<F>> {
        value.state.update(
            Q1Fog {
                density: fog.density as f32,
                color: fog.color,
            },
            presentation.seconds as f32,
            fog.duration as f32,
        );
        value.sky_factor = fog.sky_factor.clamp(0.0, 1.0);
        value.context = Some(Q1FogContext {
            content: presentation.content.clone(),
            sequence: presentation.sequence,
            seconds: presentation.seconds,
            source_entity: presentation.source_entity,
        });
        Self::resolved(wrap, value)
    }

    /// Current events for retained fog (donor `resolved`).
    fn resolved(wrap: &dyn Fn(Q1FogTransition) -> F, value: &RetainedFog) -> Vec<SimulationPresentationEvent<F>> {
        let Some(context) = value.context.as_ref() else {
            return Vec::new();
        };
        vec![SimulationPresentationEvent {
            source: SourcePresentationEvent::Q1Fog {
                player: value.player.clone(),
                sky_factor: value.sky_factor,
                transition: wrap(value.state.capture()),
            },
            owner: None,
            recipient: None,
            sequence: context.sequence,
            content: context.content.clone(),
            seconds: context.seconds,
            source_entity: context.source_entity,
        }]
    }

    /// Read retained fog (donor `read`).
    fn read_retained(
        options: &SimulationQ1FogOptions,
        entry: &SaveReader,
        player: Option<ActorId>,
    ) -> Result<RetainedFog, ValueError> {
        let mut state = Q1FogState::new();
        state.install(read_transition(&entry.field("transition"))?);
        let context = entry
            .field("context")
            .nullable(|value| -> Result<Q1FogContext, ValueError> {
                let content = read_content_id(&value.field("content"))?;
                if !options.accepts(&content) {
                    return Err(value.fail("fog source content is not selected"));
                }
                Ok(Q1FogContext {
                    content,
                    sequence: value.field("sequence").integer(0)?,
                    seconds: value.field("seconds").finite()?,
                    source_entity: value
                        .field("sourceEntity")
                        .nullable(|slot| -> Result<i32, ValueError> {
                            i32::try_from(slot.integer(0)?).map_err(|_| slot.fail("expected an integer in range"))
                        })?,
                })
            })?;
        if player.is_some() && context.is_none() {
            return Err(entry.fail("actor fog requires its source context"));
        }
        Ok(RetainedFog {
            player,
            state,
            sky_factor: bounded(&entry.field("skyFactor"), 0.0, 1.0)?,
            context,
        })
    }
}

impl<F> PresentationFog<F> for SimulationQ1Fog<F> {
    fn update(
        &mut self,
        presentation: &SimulationPresentationEvent<F>,
        fog: &Q1FogFields,
    ) -> Vec<SimulationPresentationEvent<F>> {
        SimulationQ1Fog::update(self, presentation, fog)
    }

    fn presentation(&self) -> Vec<SimulationPresentationEvent<F>> {
        SimulationQ1Fog::presentation(self)
    }

    fn retire(&mut self, actor: &ActorId) {
        SimulationQ1Fog::retire(self, actor);
    }

    fn capture(&self) -> SaveJson {
        SimulationQ1Fog::capture(self)
    }

    fn restore(
        &mut self,
        reader: SaveReader,
        reference: &dyn Fn(&SavedActorId) -> ActorId,
    ) -> Result<Vec<SimulationPresentationEvent<F>>, ValueError> {
        SimulationQ1Fog::restore(self, reader, reference)
    }

    fn reset(&mut self) {
        SimulationQ1Fog::reset(self);
    }
}

/// Read a range-checked finite number (donor `bounded`).
fn bounded(reader: &SaveReader, minimum: f64, maximum: f64) -> Result<f64, ValueError> {
    let value = reader.finite()?;
    if value < minimum || value > maximum {
        return Err(reader.fail("fog value out of range"));
    }
    Ok(value)
}

/// Read fog (donor `readFog`).
#[allow(clippy::cast_possible_truncation)]
fn read_fog(reader: &SaveReader) -> Result<Q1Fog, ValueError> {
    let color = reader.field("color");
    Ok(Q1Fog {
        density: bounded(&reader.field("density"), 0.0, f64::INFINITY)? as f32,
        color: vec3(
            bounded(&color.field("x"), 0.0, 1.0)? as f32,
            bounded(&color.field("y"), 0.0, 1.0)? as f32,
            bounded(&color.field("z"), 0.0, 1.0)? as f32,
        ),
    })
}

/// Read a transition (donor `readTransition`).
#[allow(clippy::cast_possible_truncation)]
fn read_transition(reader: &SaveReader) -> Result<Q1FogTransition, ValueError> {
    Ok(Q1FogTransition {
        previous: read_fog(&reader.field("previous"))?,
        target: read_fog(&reader.field("target"))?,
        start: reader.field("start").finite()? as f32,
        duration: bounded(&reader.field("duration"), 0.0, f64::INFINITY)? as f32,
    })
}

/// Write fog.
fn write_fog(fog: &Q1Fog) -> SaveJson {
    obj(vec![
        ("density", num(f64::from(fog.density))),
        (
            "color",
            obj(vec![
                ("x", num(f64::from(fog.color.x))),
                ("y", num(f64::from(fog.color.y))),
                ("z", num(f64::from(fog.color.z))),
            ]),
        ),
    ])
}

/// Write a transition.
fn write_transition(transition: &Q1FogTransition) -> SaveJson {
    obj(vec![
        ("previous", write_fog(&transition.previous)),
        ("target", write_fog(&transition.target)),
        ("start", num(f64::from(transition.start))),
        ("duration", num(f64::from(transition.duration))),
    ])
}

/// Write a source context.
fn write_context(context: Option<&Q1FogContext>) -> SaveJson {
    context.map_or(SaveJson::Null, |context| {
        obj(vec![
            ("content", json_str(context.content.as_str())),
            ("sequence", int(context.sequence)),
            ("seconds", num(context.seconds)),
            (
                "sourceEntity",
                context
                    .source_entity
                    .map_or(SaveJson::Null, |slot| int(i64::from(slot))),
            ),
        ])
    })
}

/// Write retained fog fields (donor `capture`).
fn write_retained(value: &RetainedFog) -> Vec<(&'static str, SaveJson)> {
    vec![
        ("transition", write_transition(&value.state.capture())),
        ("skyFactor", num(value.sky_factor)),
        ("context", write_context(value.context.as_ref())),
    ]
}

/// Reference a saved actor id (donor `readSavedActor`).
fn read_saved_actor(reader: &SaveReader) -> Result<SavedActorId, ValueError> {
    Ok(SavedActorId {
        slot: u32::try_from(reader.field("slot").integer(0)?).unwrap_or(u32::MAX),
        generation: u32::try_from(reader.field("generation").integer(0)?).unwrap_or(u32::MAX),
    })
}

/// Write a saved actor id (donor `savedActorId`).
fn write_saved_actor(actor: &ActorId) -> SaveJson {
    obj(vec![
        ("slot", int(i64::from(actor.slot()))),
        ("generation", int(i64::from(actor.generation()))),
    ])
}

/// Read a content id (donor `readContentId`).
fn read_content_id(reader: &SaveReader) -> Result<ContentId, ValueError> {
    let value = reader.string()?;
    validate_content_id(&value).map_err(|_| reader.fail("expected a content identity"))?;
    Ok(ContentId(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;

    fn content() -> ContentId {
        ContentId("q1:classic:id1:1".to_string())
    }

    fn other_content() -> ContentId {
        ContentId("q1:classic:hipnotic:1".to_string())
    }

    fn options(live: Vec<(u32, u32)>, accepted: HashSet<ContentId>) -> SimulationQ1FogOptions {
        SimulationQ1FogOptions {
            content: content(),
            accepted_contents: accepted,
            entities: String::new(),
            alive: Box::new(move |candidate: &ActorId| live.contains(&(candidate.slot(), candidate.generation()))),
        }
    }

    fn simulation(live: Vec<(u32, u32)>) -> SimulationQ1Fog<Q1FogTransition> {
        SimulationQ1Fog::new(options(live, HashSet::new()), |transition| transition).unwrap()
    }

    fn presentation(sequence: i64, seconds: f64) -> SimulationPresentationEvent<Q1FogTransition> {
        SimulationPresentationEvent {
            source: SourcePresentationEvent::Q1Fog {
                player: None,
                sky_factor: 0.5,
                transition: Q1FogState::new().capture(),
            },
            owner: None,
            recipient: None,
            sequence,
            content: content(),
            seconds,
            source_entity: Some(7),
        }
    }

    fn fog_event(player: Option<ActorId>) -> Q1FogFields {
        Q1FogFields {
            player,
            density: 1.0,
            color: vec3(0.1, 0.2, 0.3),
            sky_factor: 0.75,
            duration: 2.0,
        }
    }

    fn member(value: &SaveJson, key: &str) -> SaveJson {
        match value {
            SaveJson::Object(members) => members
                .iter()
                .find(|(name, _)| name == key)
                .map_or(SaveJson::Null, |(_, entry)| entry.clone()),
            _ => SaveJson::Null,
        }
    }

    fn replace_member(value: &SaveJson, key: &str, replacement: SaveJson) -> SaveJson {
        match value {
            SaveJson::Object(members) => SaveJson::Object(
                members
                    .iter()
                    .map(|(name, entry)| {
                        (
                            name.clone(),
                            if name == key {
                                replacement.clone()
                            } else {
                                entry.clone()
                            },
                        )
                    })
                    .collect(),
            ),
            other => other.clone(),
        }
    }

    fn resolved_fog(event: &SimulationPresentationEvent<Q1FogTransition>) -> &Q1FogTransition {
        match &event.source {
            SourcePresentationEvent::Q1Fog { transition, .. } => transition,
            _ => panic!("expected a q1-fog event"),
        }
    }

    #[test]
    fn foreign_content_is_ignored() {
        let mut fog = simulation(vec![(1, 1)]);
        let mut context = presentation(3, 1.0);
        context.content = ContentId("q1:classic:rogue:1".to_string());
        assert!(fog.update(&context, &fog_event(None)).is_empty());
        assert!(fog.presentation().is_empty());
    }

    #[test]
    fn accepted_contents_admit_extra_sources() {
        let mut accepted = HashSet::new();
        accepted.insert(other_content());
        let mut fog = SimulationQ1Fog::new(options(vec![], accepted), |transition| transition).unwrap();
        let mut context = presentation(3, 1.0);
        context.content = other_content();
        let output = fog.update(&context, &fog_event(None));
        assert_eq!(output.len(), 1);
        assert_eq!(output[0].content, other_content());
    }

    #[test]
    fn global_update_resolves_world_transition() {
        let mut fog = simulation(vec![]);
        let output = fog.update(&presentation(41, 1.5), &fog_event(None));
        assert_eq!(output.len(), 1);
        let event = &output[0];
        assert_eq!(event.sequence, 41);
        assert_eq!(event.seconds, 1.5);
        assert_eq!(event.source_entity, Some(7));
        assert_eq!(event.content, content());
        assert!(event.owner.is_none());
        assert!(event.recipient.is_none());
        match &event.source {
            SourcePresentationEvent::Q1Fog {
                player,
                sky_factor,
                transition,
            } => {
                assert!(player.is_none());
                assert_eq!(*sky_factor, 0.75);
                assert_eq!(transition.target.density, 1.0);
                assert_eq!(transition.target.color, vec3(0.1, 0.2, 0.3));
                assert_eq!(transition.start, 1.5);
                assert_eq!(transition.duration, 2.0);
            }
            _ => panic!("expected a q1-fog event"),
        }
        assert_eq!(fog.presentation().len(), 1);
    }

    #[test]
    fn actor_update_seeds_from_global() {
        let owner = IdentityOwner::create("q1-fog-sim").unwrap();
        let actor = owner.actor(1, 1);
        let mut fog = simulation(vec![(1, 1)]);
        fog.update(&presentation(1, 0.0), &fog_event(None));
        let output = fog.update(&presentation(2, 4.0), &fog_event(Some(actor.clone())));
        assert_eq!(output.len(), 1);
        assert_eq!(fog.presentation().len(), 2);
        let transition = resolved_fog(&output[0]);
        assert_eq!(transition.target.density, 1.0);
        match &output[0].source {
            SourcePresentationEvent::Q1Fog { player, sky_factor, .. } => {
                assert_eq!(*player, Some(actor));
                assert_eq!(*sky_factor, 0.75);
            }
            _ => panic!("expected a q1-fog event"),
        }
    }

    #[test]
    fn actor_update_ignores_dead_actors() {
        let owner = IdentityOwner::create("q1-fog-sim").unwrap();
        let actor = owner.actor(9, 1);
        let mut fog = simulation(vec![(1, 1)]);
        assert!(fog.update(&presentation(2, 4.0), &fog_event(Some(actor))).is_empty());
        assert!(fog.presentation().is_empty());
    }

    #[test]
    fn actor_slot_reinstalls_on_token_change() {
        let first_owner = IdentityOwner::create("q1-fog-sim").unwrap();
        let second_owner = IdentityOwner::create("q1-fog-sim-next").unwrap();
        let first = first_owner.actor(1, 1);
        let second = second_owner.actor(1, 1);
        let mut fog = simulation(vec![(1, 1)]);
        fog.update(&presentation(1, 0.0), &fog_event(Some(first)));
        let output = fog.update(&presentation(2, 4.0), &fog_event(Some(second.clone())));
        assert_eq!(output.len(), 1);
        assert_eq!(fog.presentation().len(), 1);
        match &fog.presentation()[0].source {
            SourcePresentationEvent::Q1Fog { player, .. } => assert_eq!(*player, Some(second)),
            _ => panic!("expected a q1-fog event"),
        }
    }

    #[test]
    fn global_update_prunes_dead_actors() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let owner = IdentityOwner::create("q1-fog-sim").unwrap();
        let actor = owner.actor(1, 1);
        let live = Rc::new(RefCell::new(vec![(1, 1)]));
        let mut fog: SimulationQ1Fog<Q1FogTransition> = SimulationQ1Fog::new(
            SimulationQ1FogOptions {
                content: content(),
                accepted_contents: HashSet::new(),
                entities: String::new(),
                alive: Box::new({
                    let live = Rc::clone(&live);
                    move |candidate: &ActorId| live.borrow().contains(&(candidate.slot(), candidate.generation()))
                }),
            },
            |transition| transition,
        )
        .unwrap();
        fog.update(&presentation(1, 0.0), &fog_event(Some(actor)));
        assert_eq!(fog.presentation().len(), 1);
        live.borrow_mut().clear();
        let output = fog.update(&presentation(2, 4.0), &fog_event(None));
        assert_eq!(output.len(), 1);
        assert_eq!(fog.presentation().len(), 1);
        match &fog.presentation()[0].source {
            SourcePresentationEvent::Q1Fog { player, .. } => assert!(player.is_none()),
            _ => panic!("expected a q1-fog event"),
        }
    }

    #[test]
    fn retire_releases_matching_actor_only() {
        let owner = IdentityOwner::create("q1-fog-sim").unwrap();
        let actor = owner.actor(1, 1);
        let successor = owner.actor(1, 2);
        let mut fog = simulation(vec![(1, 1), (1, 2)]);
        fog.update(&presentation(1, 0.0), &fog_event(Some(actor.clone())));
        fog.retire(&successor);
        assert_eq!(fog.presentation().len(), 1);
        fog.retire(&actor);
        assert!(fog.presentation().is_empty());
    }

    #[test]
    fn reset_restores_initial_world_fog() {
        let owner = IdentityOwner::create("q1-fog-sim").unwrap();
        let actor = owner.actor(1, 1);
        let mut fog = simulation(vec![(1, 1)]);
        fog.update(&presentation(1, 0.0), &fog_event(None));
        fog.update(&presentation(2, 4.0), &fog_event(Some(actor)));
        assert_eq!(fog.presentation().len(), 2);
        fog.reset();
        assert!(fog.presentation().is_empty());
        let save = fog.capture();
        assert!(matches!(member(&member(&save, "global"), "context"), SaveJson::Null));
        match member(&save, "actors") {
            SaveJson::Array(actors) => assert!(actors.is_empty()),
            _ => panic!("expected an actors array"),
        }
    }

    #[test]
    fn capture_restore_round_trip() {
        let owner = IdentityOwner::create("q1-fog-sim").unwrap();
        let actor = owner.actor(1, 1);
        let mut fog = simulation(vec![(1, 1)]);
        fog.update(&presentation(41, 1.5), &fog_event(None));
        fog.update(&presentation(42, 4.0), &fog_event(Some(actor)));
        let save = fog.capture();
        let mut restored = simulation(vec![(1, 1)]);
        let output = restored
            .restore(SaveReader::new(&save), &|saved: &SavedActorId| {
                owner.actor(saved.slot, saved.generation)
            })
            .unwrap();
        assert_eq!(output.len(), 2);
        assert_eq!(restored.capture(), save);
        assert_eq!(restored.presentation(), fog.presentation());
    }

    #[test]
    fn restore_rejects_foreign_map_content() {
        let mut fog = simulation(vec![]);
        fog.update(&presentation(1, 0.0), &fog_event(None));
        let save = replace_member(&fog.capture(), "content", json_str(other_content().as_str()));
        let mut fresh = simulation(vec![]);
        let owner = IdentityOwner::create("q1-fog-sim").unwrap();
        let error = fresh
            .restore(SaveReader::new(&save), &|saved: &SavedActorId| {
                owner.actor(saved.slot, saved.generation)
            })
            .unwrap_err()
            .to_string();
        assert!(error.contains("fog state belongs to another map content"), "{error}");
    }

    #[test]
    fn restore_rejects_dead_actor() {
        let owner = IdentityOwner::create("q1-fog-sim").unwrap();
        let actor = owner.actor(1, 1);
        let mut fog = simulation(vec![(1, 1)]);
        fog.update(&presentation(1, 0.0), &fog_event(Some(actor)));
        let save = fog.capture();
        let mut fresh = simulation(vec![]);
        let error = fresh
            .restore(SaveReader::new(&save), &|saved: &SavedActorId| {
                owner.actor(saved.slot, saved.generation)
            })
            .unwrap_err()
            .to_string();
        assert!(error.contains("fog actor is not alive"), "{error}");
    }

    #[test]
    fn restore_rejects_duplicate_actor() {
        let owner = IdentityOwner::create("q1-fog-sim").unwrap();
        let actor = owner.actor(1, 1);
        let mut fog = simulation(vec![(1, 1)]);
        fog.update(&presentation(1, 0.0), &fog_event(Some(actor)));
        let save = fog.capture();
        let entry = match member(&save, "actors") {
            SaveJson::Array(entries) => entries[0].clone(),
            _ => panic!("expected an actors array"),
        };
        let doubled = replace_member(&save, "actors", arr(vec![entry.clone(), entry]));
        let mut fresh = simulation(vec![(1, 1)]);
        let error = fresh
            .restore(SaveReader::new(&doubled), &|saved: &SavedActorId| {
                owner.actor(saved.slot, saved.generation)
            })
            .unwrap_err()
            .to_string();
        assert!(error.contains("duplicate fog actor"), "{error}");
    }

    #[test]
    fn restore_rejects_actor_without_context() {
        let owner = IdentityOwner::create("q1-fog-sim").unwrap();
        let actor = owner.actor(1, 1);
        let mut fog = simulation(vec![(1, 1)]);
        fog.update(&presentation(1, 0.0), &fog_event(Some(actor)));
        let save = fog.capture();
        let entry = match member(&save, "actors") {
            SaveJson::Array(entries) => entries[0].clone(),
            _ => panic!("expected an actors array"),
        };
        let orphaned = replace_member(
            &save,
            "actors",
            arr(vec![replace_member(&entry, "context", SaveJson::Null)]),
        );
        let mut fresh = simulation(vec![(1, 1)]);
        let error = fresh
            .restore(SaveReader::new(&orphaned), &|saved: &SavedActorId| {
                owner.actor(saved.slot, saved.generation)
            })
            .unwrap_err()
            .to_string();
        assert!(error.contains("actor fog requires its source context"), "{error}");
    }

    #[test]
    fn restore_rejects_unselected_source_content() {
        let owner = IdentityOwner::create("q1-fog-sim").unwrap();
        let actor = owner.actor(1, 1);
        let mut fog = simulation(vec![(1, 1)]);
        fog.update(&presentation(1, 0.0), &fog_event(Some(actor)));
        let save = fog.capture();
        let entry = match member(&save, "actors") {
            SaveJson::Array(entries) => entries[0].clone(),
            _ => panic!("expected an actors array"),
        };
        let context = replace_member(
            &member(&entry, "context"),
            "content",
            json_str(other_content().as_str()),
        );
        let resourced = replace_member(&save, "actors", arr(vec![replace_member(&entry, "context", context)]));
        let mut fresh = simulation(vec![(1, 1)]);
        let error = fresh
            .restore(SaveReader::new(&resourced), &|saved: &SavedActorId| {
                owner.actor(saved.slot, saved.generation)
            })
            .unwrap_err()
            .to_string();
        assert!(error.contains("fog source content is not selected"), "{error}");
    }

    #[test]
    fn restore_rejects_out_of_range_fog() {
        let mut fog = simulation(vec![]);
        fog.update(&presentation(1, 0.0), &fog_event(None));
        let save = fog.capture();
        let global = member(&save, "global");
        let transition = member(&global, "transition");
        let target = replace_member(&member(&transition, "target"), "density", num(-1.0));
        let ranged = replace_member(
            &save,
            "global",
            replace_member(&global, "transition", replace_member(&transition, "target", target)),
        );
        let mut fresh = simulation(vec![]);
        let owner = IdentityOwner::create("q1-fog-sim").unwrap();
        let error = fresh
            .restore(SaveReader::new(&ranged), &|saved: &SavedActorId| {
                owner.actor(saved.slot, saved.generation)
            })
            .unwrap_err()
            .to_string();
        assert!(error.contains("fog value out of range"), "{error}");
    }

    #[test]
    fn seam_delegates_through_presentation_fog() {
        let owner = IdentityOwner::create("q1-fog-sim").unwrap();
        let actor = owner.actor(1, 1);
        let mut fog: Box<dyn PresentationFog<Q1FogTransition>> = Box::new(simulation(vec![(1, 1)]));
        assert!(fog.presentation().is_empty());
        let output = fog.update(&presentation(1, 0.0), &fog_event(Some(actor.clone())));
        assert_eq!(output.len(), 1);
        assert_eq!(fog.presentation().len(), 1);
        let save = fog.capture();
        fog.reset();
        assert!(fog.presentation().is_empty());
        fog.restore(SaveReader::new(&save), &|saved: &SavedActorId| {
            owner.actor(saved.slot, saved.generation)
        })
        .unwrap();
        assert_eq!(fog.presentation().len(), 1);
        fog.retire(&actor);
        assert!(fog.presentation().is_empty());
    }
}
