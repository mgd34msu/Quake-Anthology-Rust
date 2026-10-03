//! Q1 fog transitions retained per map and player.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/simulation/q1-fog.ts`
//! (`SimulationQ1Fog`).
//!
//! Finite world/actor transitions, not an event replay log.

use std::collections::{HashMap, HashSet};

use qa_client::materials::fog::{q1_world_fog, Q1Fog, Q1FogState, Q1FogTransition};
use qa_client::ClientError;
use qa_content::contract::ContentId;
use qa_content::q1::addons::context::Q1AddonEvent;
use qa_core::identity::{ActorId, SavedActorId};
use qa_world::save::records::{read_saved_actor, write_saved_actor};
use qa_world::save::shared::read_content_id;
use qa_world::save::value::{arr, int, num, obj, str as save_str, SaveJson, SaveReader};
use qa_world::WorldError;
use thiserror::Error;

use super::types::{SimulationPresentationEvent, SourcePresentationEvent};

/// Fog owner options.
pub struct SimulationQ1FogOptions {
    /// Map content owning the global transition.
    pub content: ContentId,
    /// Extra accepted source contents for component owners.
    pub accepted_contents: Option<HashSet<ContentId>>,
    /// Entity lump text seeding worldspawn fog.
    pub entities: String,
    /// Liveness probe for fogged players.
    pub alive: Box<dyn Fn(&ActorId) -> bool>,
}

/// Source context retained with a fog transition.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1FogContext {
    /// Source content.
    pub content: ContentId,
    /// Source sequence.
    pub sequence: u64,
    /// Source seconds.
    pub seconds: f64,
    /// Source entity.
    pub source_entity: Option<i32>,
}

struct RetainedFog {
    player: Option<ActorId>,
    state: Q1FogState,
    sky_factor: f64,
    context: Option<Q1FogContext>,
}

/// Q1 fog failures.
#[derive(Debug, Error)]
pub enum Q1FogError {
    /// Worldspawn fog parsing failed.
    #[error(transparent)]
    Client(#[from] ClientError),
}

fn key(actor: &ActorId) -> String {
    format!("{}:{}", actor.slot(), actor.generation())
}

fn bounded(reader: &SaveReader, minimum: f64, maximum: f64) -> Result<f64, WorldError> {
    let value = reader.finite()?;
    if value < minimum || value > maximum {
        return Err(reader.fail("fog value out of range"));
    }
    Ok(value)
}

fn read_fog(reader: &SaveReader) -> Result<Q1Fog, WorldError> {
    let color = reader.field("color");
    Ok(Q1Fog {
        density: bounded(&reader.field("density"), 0.0, f64::INFINITY)? as f32,
        color: qa_core::math::vec3(
            bounded(&color.field("x"), 0.0, 1.0)? as f32,
            bounded(&color.field("y"), 0.0, 1.0)? as f32,
            bounded(&color.field("z"), 0.0, 1.0)? as f32,
        ),
    })
}

fn read_transition(reader: &SaveReader) -> Result<Q1FogTransition, WorldError> {
    Ok(Q1FogTransition {
        previous: read_fog(&reader.field("previous"))?,
        target: read_fog(&reader.field("target"))?,
        start: reader.field("start").finite()? as f32,
        duration: bounded(&reader.field("duration"), 0.0, f64::INFINITY)? as f32,
    })
}

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

fn write_transition(transition: &Q1FogTransition) -> SaveJson {
    obj(vec![
        ("previous", write_fog(&transition.previous)),
        ("target", write_fog(&transition.target)),
        ("start", num(f64::from(transition.start))),
        ("duration", num(f64::from(transition.duration))),
    ])
}

fn write_context(context: &Q1FogContext) -> SaveJson {
    obj(vec![
        ("content", save_str(context.content.as_str())),
        ("sequence", int(context.sequence as i64)),
        ("seconds", num(context.seconds)),
        (
            "sourceEntity",
            context
                .source_entity
                .map_or(SaveJson::Null, |entity| int(i64::from(entity))),
        ),
    ])
}

/// Retained Q1 fog transitions for one map content.
pub struct SimulationQ1Fog {
    options: SimulationQ1FogOptions,
    initial: Q1FogTransition,
    global: RetainedFog,
    actors: HashMap<String, RetainedFog>,
}

impl SimulationQ1Fog {
    /// Create fog state seeded from the entity lump.
    pub fn new(options: SimulationQ1FogOptions) -> Result<Self, Q1FogError> {
        let initial = q1_world_fog(&options.entities)?;
        let mut state = Q1FogState::new();
        state.install(initial);
        Ok(Self {
            options,
            initial,
            global: RetainedFog {
                player: None,
                state,
                sky_factor: 0.5,
                context: None,
            },
            actors: HashMap::new(),
        })
    }

    /// Reset to the worldspawn transition.
    pub fn reset(&mut self) {
        self.global.state.install(self.initial);
        self.global.sky_factor = 0.5;
        self.global.context = None;
        self.actors.clear();
    }

    /// Drop a retired player's transition.
    pub fn retire(&mut self, actor: &ActorId) {
        let id = key(actor);
        if self
            .actors
            .get(&id)
            .is_some_and(|retained| retained.player.as_ref() == Some(actor))
        {
            self.actors.remove(&id);
        }
    }

    fn accepts(&self, content: &ContentId) -> bool {
        content == &self.options.content
            || self
                .options
                .accepted_contents
                .as_ref()
                .is_some_and(|set| set.contains(content))
    }

    fn resolved(value: &RetainedFog) -> Vec<SimulationPresentationEvent> {
        let Some(context) = value.context.as_ref() else {
            return Vec::new();
        };
        vec![SimulationPresentationEvent {
            event: SourcePresentationEvent::Q1Fog {
                player: value.player.clone(),
                transition: value.state.capture(),
                sky_factor: value.sky_factor,
            },
            owner: None,
            recipient: None,
            sequence: context.sequence,
            content: context.content.clone(),
            seconds: context.seconds,
            source_entity: context.source_entity,
        }]
    }

    /// Apply a fog addon event.
    ///
    /// Non-fog addon events resolve to no output; the presentation record
    /// only routes fog events here.
    pub fn update(&mut self, context: &Q1FogContext, event: &Q1AddonEvent) -> Vec<SimulationPresentationEvent> {
        let Q1AddonEvent::Fog {
            player,
            density,
            color,
            sky_factor,
            duration,
        } = event
        else {
            return Vec::new();
        };
        if !self.accepts(&context.content) {
            return Vec::new();
        }
        let apply = |value: &mut RetainedFog| {
            value.state.update(
                Q1Fog {
                    density: *density as f32,
                    color: *color,
                },
                context.seconds as f32,
                *duration as f32,
            );
            value.sky_factor = sky_factor.clamp(0.0, 1.0);
            value.context = Some(context.clone());
            Self::resolved(value)
        };
        if player.is_none() {
            let mut output = apply(&mut self.global);
            let ids: Vec<String> = self.actors.keys().cloned().collect();
            for id in ids {
                let remove = self
                    .actors
                    .get(&id)
                    .is_some_and(|value| value.player.as_ref().is_none_or(|player| !(self.options.alive)(player)));
                if remove {
                    self.actors.remove(&id);
                    continue;
                }
                if let Some(value) = self.actors.get_mut(&id) {
                    output.extend(apply(value));
                }
            }
            return output;
        }
        let player = player.as_ref().expect("player checked");
        if !(self.options.alive)(player) {
            return Vec::new();
        }
        let id = key(player);
        let stale = self
            .actors
            .get(&id)
            .is_none_or(|value| value.player.as_ref() != Some(player));
        if stale {
            let mut state = Q1FogState::new();
            state.install(self.global.state.capture());
            let sky_factor = self.global.sky_factor;
            self.actors.insert(
                id.clone(),
                RetainedFog {
                    player: Some(player.clone()),
                    state,
                    sky_factor,
                    context: None,
                },
            );
        }
        let value = self.actors.get_mut(&id).expect("fog retained");
        apply(value)
    }

    /// Currently resolved transitions.
    pub fn presentation(&self) -> Vec<SimulationPresentationEvent> {
        let mut output = Self::resolved(&self.global);
        for value in self.actors.values() {
            output.extend(Self::resolved(value));
        }
        output
    }

    /// Capture live transitions as donor-shaped JSON.
    pub fn capture(&self) -> SaveJson {
        let capture = |value: &RetainedFog| {
            obj(vec![
                ("transition", write_transition(&value.state.capture())),
                ("skyFactor", num(value.sky_factor)),
                ("context", value.context.as_ref().map_or(SaveJson::Null, write_context)),
            ])
        };
        let mut actors: Vec<(&ActorId, &RetainedFog)> = self
            .actors
            .values()
            .filter(|value| value.player.as_ref().is_some_and(|player| (self.options.alive)(player)))
            .map(|value| (value.player.as_ref().expect("player checked"), value))
            .collect();
        actors.sort_by_key(|(player, _)| (player.slot(), player.generation()));
        obj(vec![
            ("content", save_str(self.options.content.as_str())),
            ("global", capture(&self.global)),
            (
                "actors",
                arr(actors
                    .iter()
                    .map(|(player, value)| {
                        obj(vec![
                            ("player", write_saved_actor(SavedActorId::from(*player))),
                            ("transition", write_transition(&value.state.capture())),
                            ("skyFactor", num(value.sky_factor)),
                            ("context", value.context.as_ref().map_or(SaveJson::Null, write_context)),
                        ])
                    })
                    .collect()),
            ),
        ])
    }

    /// Restore captured transitions, returning the resolved output.
    pub fn restore(
        &mut self,
        reader: &SaveReader,
        reference: &dyn Fn(SavedActorId) -> ActorId,
    ) -> Result<Vec<SimulationPresentationEvent>, WorldError> {
        if read_content_id(reader.field("content"))? != self.options.content.as_str() {
            return Err(reader.fail("fog state belongs to another map content"));
        }
        let read = |entry: &SaveReader, player: Option<ActorId>| -> Result<RetainedFog, WorldError> {
            let mut state = Q1FogState::new();
            state.install(read_transition(&entry.field("transition"))?);
            let context = entry.field("context").nullable(|value| {
                let content = ContentId(read_content_id(value.field("content"))?);
                if !self.accepts(&content) {
                    return Err(value.fail("fog source content is not selected"));
                }
                Ok(Q1FogContext {
                    content,
                    sequence: value.field("sequence").integer(0)? as u64,
                    seconds: value.field("seconds").finite()?,
                    source_entity: value
                        .field("sourceEntity")
                        .nullable(|entity| entity.integer(0))?
                        .map(|entity| entity as i32),
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
        };
        let global = read(&reader.field("global"), None)?;
        let mut actors: HashMap<String, RetainedFog> = HashMap::new();
        let entries: Vec<(ActorId, RetainedFog)> = reader.field("actors").list(|entry| {
            let player = reference(read_saved_actor(entry.field("player"))?);
            let id = key(&player);
            if !(self.options.alive)(&player) {
                return Err(entry.fail("fog actor is not alive"));
            }
            if actors.contains_key(&id) {
                return Err(entry.fail("duplicate fog actor"));
            }
            Ok((player, read(&entry, None)?))
        })?;
        for (player, retained) in entries {
            let id = key(&player);
            if actors.contains_key(&id) {
                return Err(reader.fail("duplicate fog actor"));
            }
            if retained.context.is_none() {
                return Err(reader.fail("actor fog requires its source context"));
            }
            actors.insert(
                id,
                RetainedFog {
                    player: Some(player),
                    ..retained
                },
            );
        }
        self.global.state.install(global.state.capture());
        self.global.sky_factor = global.sky_factor;
        self.global.context = global.context;
        self.actors = actors;
        Ok([Self::resolved(&self.global)]
            .into_iter()
            .flatten()
            .chain(self.actors.values().flat_map(Self::resolved))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use qa_core::identity::IdentityOwner;

    use super::*;

    fn content() -> ContentId {
        ContentId("q1:id1:e1m1:1".to_string())
    }

    fn make_options(alive: bool) -> (SimulationQ1FogOptions, Arc<AtomicBool>) {
        let flag = Arc::new(AtomicBool::new(alive));
        let probe = flag.clone();
        (
            SimulationQ1FogOptions {
                content: content(),
                accepted_contents: None,
                entities: "{ \"classname\" \"worldspawn\" }".to_string(),
                alive: Box::new(move |_| probe.load(Ordering::SeqCst)),
            },
            flag,
        )
    }

    fn context() -> Q1FogContext {
        Q1FogContext {
            content: content(),
            sequence: 3,
            seconds: 1.5,
            source_entity: Some(7),
        }
    }

    fn addon(player: Option<ActorId>) -> Q1AddonEvent {
        Q1AddonEvent::Fog {
            player,
            density: 0.2,
            color: qa_core::math::vec3(0.1, 0.2, 0.3),
            sky_factor: 0.8,
            duration: 2.0,
        }
    }

    #[test]
    fn global_update_resolves() {
        let (options, _) = make_options(true);
        let mut fog = SimulationQ1Fog::new(options).expect("fog");
        let output = fog.update(&context(), &addon(None));
        assert_eq!(output.len(), 1);
        assert_eq!(output[0].sequence, 3);
        match &output[0].event {
            SourcePresentationEvent::Q1Fog { player, sky_factor, .. } => {
                assert!(player.is_none());
                assert_eq!(*sky_factor, 0.8);
            }
            other => panic!("unexpected event: {other:?}"),
        }
        assert_eq!(fog.presentation().len(), 1);
    }

    #[test]
    fn unaccepted_content_is_ignored() {
        let (options, _) = make_options(true);
        let mut fog = SimulationQ1Fog::new(options).expect("fog");
        let mut context = context();
        context.content = ContentId("q1:hipnotic:e1m1:1".to_string());
        assert!(fog.update(&context, &addon(None)).is_empty());
    }

    #[test]
    fn player_fog_tracks_liveness() {
        let owner = IdentityOwner::create("fog").expect("owner");
        let player = owner.actor(1, 0);
        let (options, flag) = make_options(true);
        let mut fog = SimulationQ1Fog::new(options).expect("fog");
        let output = fog.update(&context(), &addon(Some(player.clone())));
        assert_eq!(output.len(), 1);
        flag.store(false, Ordering::SeqCst);
        // A global update sweeps dead players.
        let output = fog.update(&context(), &addon(None));
        assert_eq!(output.len(), 1);
        assert!(fog
            .presentation()
            .iter()
            .all(|event| !matches!(&event.event, SourcePresentationEvent::Q1Fog { player: Some(_), .. })));
    }

    #[test]
    fn retire_drops_player() {
        let owner = IdentityOwner::create("fog").expect("owner");
        let player = owner.actor(1, 0);
        let (options, _) = make_options(true);
        let mut fog = SimulationQ1Fog::new(options).expect("fog");
        fog.update(&context(), &addon(Some(player.clone())));
        // Only the player transition resolves; the global has no context yet.
        assert_eq!(fog.presentation().len(), 1);
        fog.retire(&player);
        assert!(fog.presentation().is_empty());
    }

    #[test]
    fn capture_restore_round_trip() {
        let owner = IdentityOwner::create("fog").expect("owner");
        let player = owner.actor(2, 0);
        let (options, _) = make_options(true);
        let mut fog = SimulationQ1Fog::new(options).expect("fog");
        fog.update(&context(), &addon(None));
        fog.update(&context(), &addon(Some(player.clone())));
        let json = fog.capture();
        let (options, _) = make_options(true);
        let mut fog = SimulationQ1Fog::new(options).expect("fog");
        let resolved = fog
            .restore(&SaveReader::new(&json), &|saved| {
                owner.actor(saved.slot, saved.generation)
            })
            .expect("restore");
        assert_eq!(resolved.len(), 2);
        assert_eq!(fog.presentation().len(), 2);
    }

    #[test]
    fn restore_rejects_foreign_content() {
        let owner = IdentityOwner::create("fog").expect("owner");
        let (options, _) = make_options(true);
        let fog = SimulationQ1Fog::new(options).expect("fog");
        let json = fog.capture();
        let (mut options, _) = make_options(true);
        options.content = ContentId("q1:id1:e1m2:1".to_string());
        let mut fog = SimulationQ1Fog::new(options).expect("fog");
        assert!(fog
            .restore(&SaveReader::new(&json), &|saved| owner
                .actor(saved.slot, saved.generation))
            .is_err());
    }
}
