//! Q1 mg3 monster navigation (`src/content/q1/addons/monsters/ai/path.ts`).
//!
//! `quakec_mg3/defs.qc` return codes and `ai.qc` call contract.
//! GPL-2.0-or-later.
//!
//! The closed rerelease engine's path algorithm is unavailable. This
//! implementation follows shared, movement-admitted navigation, then
//! commits only source walkMove.
//!
//! `qa-content` cannot depend on `qa-bots`, so the donor
//! `bots/navigation` runtime surfaces below structurally with their
//! donor noted; the selected session supplies them through
//! [`Mg3MonsterNavigationHost`].

use std::cell::RefCell;
use std::collections::HashMap;

use qa_core::identity::{ActorId, OwnedActor, SavedActorId};
use qa_core::math::Vec3;

use crate::q1::base::monsters::BaseMonster;
use crate::q1::foundation::callbacks::Q1StateExtension;
use crate::q1::foundation::checkpoint::{decode_checkpoint_value, encode_checkpoint_value};
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::host::Q1ReleaseHook;
use crate::q1::foundation::types::{length, vsub, yaw_for};
use crate::q1::{q1_error, Q1Error};
use crate::value::{arr, int, num, obj, str as save_str, SaveJson, SaveReader};

/// Mg3 path result codes (`Mg3PathResult`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum Mg3PathResult {
    /// No route.
    Error = 0,
    /// The goal is already reached.
    ReachedGoal = 1,
    /// The route end is reached.
    ReachedPathEnd = 2,
    /// Movement is blocked.
    MoveBlocked = 3,
    /// A step committed.
    InProgress = 4,
}

impl Mg3PathResult {
    /// Donor numeric code.
    #[must_use]
    pub fn as_i32(self) -> i32 {
        self as i32
    }
}

/// Navigation edge travel mode (donor `bots/navigation` edge `mode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mg3TravelMode {
    /// Walking edge.
    Walk,
    /// Drop edge.
    Drop,
    /// Swimming edge.
    Swim,
    /// Any other selected mode.
    Other,
}

/// Navigation edge (donor `bots/navigation` route edge).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mg3NavigationEdge {
    /// Edge id.
    pub id: i32,
    /// Edge travel mode.
    pub mode: Mg3TravelMode,
}

/// Navigation route (donor `bots/navigation` `NavigationRoute`).
#[derive(Debug, Clone)]
pub struct Mg3NavigationRoute {
    /// Route map digest.
    pub map_digest: String,
    /// Route node ids.
    pub nodes: Vec<i32>,
    /// Route edges.
    pub edges: Vec<Mg3NavigationEdge>,
    /// Route waypoints.
    pub points: Vec<Vec3>,
    /// Route travel seconds.
    pub travel_seconds: f64,
    /// Route generation.
    pub generation: i32,
}

/// Route request (`runtime.route`).
#[derive(Debug, Clone)]
pub struct Mg3RouteRequest {
    /// Route start.
    pub start: Vec3,
    /// Route goal.
    pub goal: Vec3,
    /// Admitted edge modes (`walk`, `drop`, `swim`).
    pub edge_modes: &'static [Mg3TravelMode],
}

/// Selected navigation runtime for one actor (donor
/// `bots/navigation` `NavigationRuntime`).
pub trait Mg3NavigationRuntime {
    /// Whether the runtime uses the canonical monster profile.
    fn monster_profile(&self) -> bool;
    /// The runtime self collision exclusion.
    fn pass_actor(&self) -> Option<ActorId>;
    /// The runtime map digest.
    fn map_digest(&self) -> &str;
    /// Whether a node id exists.
    fn node_known(&self, id: i32) -> bool;
    /// Look up an edge by id.
    fn edge_by_id(&self, id: i32) -> Option<Mg3NavigationEdge>;
    /// Whether a saved route is still valid.
    fn route_still_valid(&self, route: &Mg3NavigationRoute) -> bool;
    /// Route from start to goal, or `None` when unreachable.
    fn route(&self, request: &Mg3RouteRequest) -> Option<Mg3NavigationRoute>;
}

/// Selected navigation host (`Mg3MonsterNavigationHost`).
pub trait Mg3MonsterNavigationHost {
    /// Return this actual actor's selected movement profile, shape and
    /// collision exclusion.
    fn for_actor(&mut self, actor: &OwnedActor) -> Option<Box<dyn Mg3NavigationRuntime + '_>>;
}

struct Path {
    goal: Vec3,
    route: Mg3NavigationRoute,
    cursor: usize,
}

struct Mg3MonsterNavigation {
    host: Box<dyn Mg3MonsterNavigationHost>,
    paths: HashMap<ActorId, Path>,
}

/// Run against the thread-local navigation registrations.
///
/// The application simulation is single-threaded (`Rc`-based handles
/// throughout), so navigation hosts are not `Send`; a thread-local table
/// (instead of a global mutex) lets the selected session supply them.
fn with_registrations<R>(f: impl FnOnce(&mut HashMap<usize, Mg3MonsterNavigation>) -> R) -> R {
    thread_local! {
        static REGISTRATIONS: RefCell<HashMap<usize, Mg3MonsterNavigation>> =
            RefCell::new(HashMap::new());
    }
    REGISTRATIONS.with(|registrations| f(&mut registrations.borrow_mut()))
}

fn navigation_key(game: &Q1EntityServices) -> usize {
    std::ptr::from_ref(game) as usize
}

fn saved_actor_id(reader: &SaveReader) -> Result<SavedActorId, Q1Error> {
    Ok(SavedActorId {
        slot: u32::try_from(reader.field("slot").integer(0)?).unwrap_or(u32::MAX),
        generation: u32::try_from(reader.field("generation").integer(0)?).unwrap_or(u32::MAX),
    })
}

fn point(reader: &SaveReader) -> Result<Vec3, Q1Error> {
    Ok(Vec3 {
        x: reader.field("x").number()? as f32,
        y: reader.field("y").number()? as f32,
        z: reader.field("z").number()? as f32,
    })
}

fn point_json(point: &Vec3) -> SaveJson {
    obj(vec![
        ("x", num(f64::from(point.x))),
        ("y", num(f64::from(point.y))),
        ("z", num(f64::from(point.z))),
    ])
}

fn navigation_runtime<'h>(
    host: &'h mut dyn Mg3MonsterNavigationHost,
    actor: &OwnedActor,
) -> Result<Option<Box<dyn Mg3NavigationRuntime + 'h>>, Q1Error> {
    let Some(runtime) = host.for_actor(actor) else {
        return Ok(None);
    };
    if !runtime.monster_profile() || runtime.pass_actor().as_ref() != Some(actor.id()) {
        return Err(q1_error(
            "MG3 pathfinding requires the canonical monster profile and self collision exclusion",
        ));
    }
    Ok(Some(runtime))
}

/// Install with the map runtime before source checkpoint restore or
/// the first Horde think (`registerMg3MonsterNavigation`).
pub fn register_mg3_monster_navigation(
    game: &mut Q1EntityServices,
    host: Box<dyn Mg3MonsterNavigationHost>,
) -> Result<(), Q1Error> {
    use std::cell::RefCell;
    use std::rc::Rc;

    struct Release;

    impl Q1ReleaseHook for Release {
        fn on_release(&mut self, game: &mut Q1EntityServices, actor: &OwnedActor) {
            let id = actor.id().clone();
            with_registrations(|registrations| {
                if let Some(navigation) = registrations.get_mut(&navigation_key(game)) {
                    navigation.paths.remove(&id);
                }
            });
        }
    }

    struct Extension;

    impl Q1StateExtension for Extension {
        fn id(&self) -> &str {
            "mg3:monster-navigation"
        }

        fn capture(&self, game: &Q1EntityServices) -> Vec<u8> {
            let paths = with_registrations(|registrations| {
                registrations
                    .get(&navigation_key(game))
                    .map(|navigation| {
                        navigation
                            .paths
                            .iter()
                            .map(|(actor, path)| {
                                obj(vec![
                                    (
                                        "actor",
                                        obj(vec![
                                            ("slot", int(i64::from(actor.slot()))),
                                            ("generation", int(i64::from(actor.generation()))),
                                        ]),
                                    ),
                                    ("goal", point_json(&path.goal)),
                                    ("cursor", int(path.cursor as i64)),
                                    ("map", save_str(&path.route.map_digest)),
                                    (
                                        "nodes",
                                        arr(path.route.nodes.iter().map(|node| int(i64::from(*node))).collect()),
                                    ),
                                    (
                                        "edges",
                                        arr(path.route.edges.iter().map(|edge| int(i64::from(edge.id))).collect()),
                                    ),
                                    ("points", arr(path.route.points.iter().map(point_json).collect())),
                                    ("seconds", num(path.route.travel_seconds)),
                                    ("generation", int(i64::from(path.route.generation))),
                                ])
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            });
            encode_checkpoint_value(&arr(paths))
        }

        fn restore(&mut self, game: &mut Q1EntityServices, bytes: &[u8]) -> Result<(), Q1Error> {
            let saved = decode_checkpoint_value(bytes)?;
            let entries: Vec<(ActorId, Path)> = SaveReader::new(&saved).list(|reader| {
                let saved_actor = reader.field("actor");
                let actor = game
                    .host
                    .actors
                    .resolve_saved(&saved_actor_id(&saved_actor)?)
                    .ok_or_else(|| Q1Error::from(reader.fail("missing navigation actor")))?;
                with_registrations(|registrations| {
                    let navigation = registrations
                        .get_mut(&navigation_key(game))
                        .ok_or_else(|| Q1Error::from(reader.fail("missing navigation actor")))?;
                    let runtime = navigation_runtime(navigation.host.as_mut(), &actor)?;
                    let Some(runtime) = runtime else {
                        return Err(Q1Error::from(reader.fail("saved navigation map is unavailable")));
                    };
                    if runtime.map_digest() != reader.field("map").string()? {
                        return Err(Q1Error::from(reader.fail("saved navigation map is unavailable")));
                    }
                    let nodes: Vec<i32> = reader.field("nodes").list(|value| {
                        let id = i32::try_from(value.integer(0)?).unwrap_or(i32::MAX);
                        if runtime.node_known(id) {
                            Ok(id)
                        } else {
                            Err(Q1Error::from(value.fail("missing navigation node")))
                        }
                    })?;
                    let edges: Vec<Mg3NavigationEdge> = reader.field("edges").list(|value| {
                        let id = i32::try_from(value.integer(0)?).unwrap_or(i32::MAX);
                        runtime
                            .edge_by_id(id)
                            .ok_or_else(|| Q1Error::from(value.fail("missing navigation edge")))
                    })?;
                    let points: Vec<Vec3> = reader.field("points").list(|entry| point(&entry))?;
                    let route = Mg3NavigationRoute {
                        map_digest: runtime.map_digest().to_string(),
                        nodes,
                        edges,
                        points,
                        travel_seconds: reader.field("seconds").number()?,
                        generation: i32::try_from(reader.field("generation").integer(0)?).unwrap_or(i32::MAX),
                    };
                    let cursor = usize::try_from(reader.field("cursor").integer(0)?).unwrap_or(usize::MAX);
                    if cursor > route.points.len() {
                        return Err(Q1Error::from(reader.fail("navigation cursor exceeds route")));
                    }
                    Ok::<_, Q1Error>((
                        actor.id().clone(),
                        Path {
                            goal: point(&reader.field("goal"))?,
                            route,
                            cursor,
                        },
                    ))
                })
            })?;
            with_registrations(|registrations| {
                if let Some(navigation) = registrations.get_mut(&navigation_key(game)) {
                    navigation.paths.clear();
                    navigation.paths.extend(entries);
                }
            });
            Ok(())
        }

        fn clone_state(
            &mut self,
            game: &mut Q1EntityServices,
            source: &ActorId,
            target: &ActorId,
        ) -> Result<(), Q1Error> {
            let key = navigation_key(game);
            with_registrations(|registrations| {
                let Some(navigation) = registrations.get_mut(&key) else {
                    return;
                };
                let Some(path) = navigation.paths.get(source) else {
                    return;
                };
                navigation.paths.insert(
                    target.clone(),
                    Path {
                        goal: path.goal,
                        route: Mg3NavigationRoute {
                            map_digest: path.route.map_digest.clone(),
                            nodes: path.route.nodes.clone(),
                            edges: path.route.edges.clone(),
                            points: path.route.points.clone(),
                            travel_seconds: path.route.travel_seconds,
                            generation: path.route.generation,
                        },
                        cursor: path.cursor,
                    },
                );
            });
            Ok(())
        }
    }

    let already = with_registrations(|registrations| registrations.contains_key(&navigation_key(game)));
    if already {
        return Err(q1_error("MG3 monster navigation is already registered"));
    }
    with_registrations(|registrations| {
        registrations.insert(
            navigation_key(game),
            Mg3MonsterNavigation {
                host,
                paths: HashMap::new(),
            },
        );
    });
    game.register_state_extension(Box::new(Extension))?;
    game.register_release_hook(Rc::new(RefCell::new(Release)));
    Ok(())
}

/// Walk-plan outcome from under the registrations borrow.
enum StepPlanOutcome {
    /// Continue with the computed step.
    Plan((f64, f64, f64)),
    /// Return early with a path result.
    Done(Mg3PathResult),
    /// Fail the walk.
    Failed(Q1Error),
}

/// Step a monster along the shared route to a goal
/// (`walkMg3PathToGoal`). Walkpathtogoal commits walking only; jumps
/// and source triggers require their own actions.
pub fn walk_mg3_path_to_goal(monster: &mut BaseMonster, distance: f64, goal: Vec3) -> Result<Mg3PathResult, Q1Error> {
    let key = navigation_key(monster.game);
    let registered = with_registrations(|registrations| registrations.contains_key(&key));
    if !registered {
        return Err(q1_error(
            "MG3 walkpathtogoal requires the selected shared navigation host",
        ));
    }
    let actor = monster
        .game
        .entity_ref(&monster.id.clone())
        .map(|entity| entity.actor.clone())
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    let origin = monster.origin()?;
    let close = |point: Vec3| f64::from(length(vsub(point, origin))) <= 1.0;
    if close(goal) {
        with_registrations(|registrations| {
            if let Some(navigation) = registrations.get_mut(&key) {
                navigation.paths.remove(actor.id());
            }
        });
        return Ok(Mg3PathResult::ReachedGoal);
    }
    let step_plan = with_registrations(|registrations| {
        let navigation = match registrations.get_mut(&key) {
            Some(navigation) => navigation,
            None => {
                return StepPlanOutcome::Failed(q1_error(
                    "MG3 walkpathtogoal requires the selected shared navigation host",
                ))
            }
        };
        let runtime = match navigation_runtime(navigation.host.as_mut(), &actor) {
            Ok(Some(runtime)) => runtime,
            Ok(None) => {
                navigation.paths.remove(actor.id());
                return StepPlanOutcome::Done(Mg3PathResult::Error);
            }
            Err(error) => return StepPlanOutcome::Failed(error),
        };
        let stale = navigation.paths.get(actor.id()).is_none_or(|path| {
            f64::from(length(vsub(goal, path.goal))) > 1.0 || !runtime.route_still_valid(&path.route)
        });
        if stale {
            let Some(route) = runtime.route(&Mg3RouteRequest {
                start: origin,
                goal,
                edge_modes: &[Mg3TravelMode::Walk, Mg3TravelMode::Drop, Mg3TravelMode::Swim],
            }) else {
                navigation.paths.remove(actor.id());
                return StepPlanOutcome::Done(Mg3PathResult::Error);
            };
            navigation
                .paths
                .insert(actor.id().clone(), Path { goal, route, cursor: 0 });
        }
        let path = match navigation.paths.get_mut(actor.id()) {
            Some(path) => path,
            None => {
                return StepPlanOutcome::Failed(q1_error(
                    "MG3 walkpathtogoal requires the selected shared navigation host",
                ))
            }
        };
        while path.cursor < path.route.points.len() && close(path.route.points[path.cursor]) {
            path.cursor += 1;
        }
        let Some(next) = path.route.points.get(path.cursor).copied() else {
            navigation.paths.remove(actor.id());
            return StepPlanOutcome::Done(Mg3PathResult::ReachedPathEnd);
        };
        let offset = vsub(next, origin);
        let horizontal = f64::from(offset.x).hypot(f64::from(offset.y));
        let step = distance.abs().min(horizontal);
        StepPlanOutcome::Plan((yaw_for(offset), distance.signum() * step, step))
    });
    let (yaw, signed, step) = match step_plan {
        StepPlanOutcome::Plan(plan) => plan,
        StepPlanOutcome::Done(result) => return Ok(result),
        StepPlanOutcome::Failed(error) => return Err(error),
    };
    if step == 0.0 || !monster.game.host.walk_move(&actor, yaw, signed) {
        with_registrations(|registrations| {
            if let Some(navigation) = registrations.get_mut(&key) {
                navigation.paths.remove(actor.id());
            }
        });
        return Ok(Mg3PathResult::MoveBlocked);
    }
    Ok(Mg3PathResult::InProgress)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_codes_match_donor() {
        assert_eq!(
            [
                Mg3PathResult::Error.as_i32(),
                Mg3PathResult::ReachedGoal.as_i32(),
                Mg3PathResult::ReachedPathEnd.as_i32(),
                Mg3PathResult::MoveBlocked.as_i32(),
                Mg3PathResult::InProgress.as_i32(),
            ],
            [0, 1, 2, 3, 4]
        );
    }
}
