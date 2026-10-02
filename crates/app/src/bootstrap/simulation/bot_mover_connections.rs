//! Q2 train connections for bot navigation construction.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/simulation/bot-mover-connections.ts`.
//!
//! Missing siblings: `SharedSimulation` (`runtime.ts`, runtime partition).
//! The [`BotMoverSimulation`] seam exposes exactly the donor's scene plus
//! Q2 source surface; the runtime partition implements it post-merge.

use qa_bots::construct::NavigationConnection;
use qa_bots::scene::BodyShape;
use qa_bots::types::{NavigationEntityBinding, NavigationProfile, TravelMode};
use qa_content::q2::foundation::movers::Q2TrainRoute;
use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3};

/// Brush mover observed through the scene (donor query row).
#[derive(Debug, Clone, PartialEq)]
pub struct BotMoverObservation {
    /// Mover actor.
    pub actor: ActorId,
    /// Shared brush bounds (donor `body.state.bounds`).
    pub bounds: Bounds,
    /// Collision model ordinal, or `None` for non-model shapes.
    pub model: Option<i32>,
}

/// Q2 entity facts used by the connections (donor `q2.game.entity` shape).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BotMoverEntity {
    /// Registry slot (donor `entity.actor.id.slot`).
    pub slot: u32,
    /// Mover speed.
    pub speed: f64,
}

/// Simulation surface for train connections.
pub trait BotMoverSimulation {
    /// Donor `simulation.q2Source() !== null`.
    fn has_q2_source(&self) -> bool;
    /// Donor `scene.queryActors(scene.modelBounds(0))` rows.
    fn train_movers(&self) -> Vec<BotMoverObservation>;
    /// Donor `q2.game.entity(actor)`.
    fn q2_mover_entity(&self, actor: &ActorId) -> Option<BotMoverEntity>;
    /// Donor `q2.movers.trainRoute(entity, q2.game)`.
    fn q2_train_route(&self, actor: &ActorId) -> Option<Q2TrainRoute>;
    /// Donor `q2.game.options.edition === "rerelease"`.
    fn q2_edition_is_rerelease(&self) -> bool;
}

impl<S: BotMoverSimulation> BotMoverSimulation for &S {
    fn has_q2_source(&self) -> bool {
        (*self).has_q2_source()
    }
    fn train_movers(&self) -> Vec<BotMoverObservation> {
        (*self).train_movers()
    }
    fn q2_mover_entity(&self, actor: &ActorId) -> Option<BotMoverEntity> {
        (*self).q2_mover_entity(actor)
    }
    fn q2_train_route(&self, actor: &ActorId) -> Option<Q2TrainRoute> {
        (*self).q2_train_route(actor)
    }
    fn q2_edition_is_rerelease(&self) -> bool {
        (*self).q2_edition_is_rerelease()
    }
}

fn profile_foot(shape: &BodyShape) -> f32 {
    shape.bounds().min.z
}

/// Construction borrows actual source corner routes and shared brush bounds.
pub fn application_train_connections<S: BotMoverSimulation>(
    simulation: &S,
    profile: &NavigationProfile,
) -> Vec<NavigationConnection> {
    if !simulation.has_q2_source() {
        return Vec::new();
    }
    let mut movers = simulation.train_movers();
    movers.sort_by_key(|mover| mover.actor.slot());
    let foot = profile_foot(&profile.shape);
    let mut connections = Vec::new();
    for mover in &movers {
        let Some(model) = mover.model else {
            continue;
        };
        let Some(entity) = simulation.q2_mover_entity(&mover.actor) else {
            continue;
        };
        let Some(route) = simulation.q2_train_route(&mover.actor) else {
            continue;
        };
        let local = &mover.bounds;
        let riding = |origin: &Vec3| Vec3 {
            x: origin.x + (local.min.x + local.max.x) / 2.0,
            y: origin.y + (local.min.y + local.max.y) / 2.0,
            z: origin.z + local.max.z - foot + 0.125,
        };
        for stop in &route.stops {
            let next = stop
                .next
                .as_ref()
                .and_then(|next| route.stops.iter().find(|candidate| &candidate.actor == next));
            let Some(next) = next else {
                continue;
            };
            if stop.wait < 0.0 || stop.teleport || next.teleport {
                continue;
            }
            let destination = simulation.q2_mover_entity(&next.actor);
            let speed = if simulation.q2_edition_is_rerelease()
                && destination.is_some_and(|destination| destination.speed != 0.0)
            {
                destination.map_or(entity.speed, |destination| destination.speed)
            } else {
                entity.speed
            };
            if speed <= 0.0 {
                continue;
            }
            let travel_seconds = stop.wait
                + f64::hypot(
                    f64::from(next.origin.x - stop.origin.x),
                    f64::hypot(
                        f64::from(next.origin.y - stop.origin.y),
                        f64::from(next.origin.z - stop.origin.z),
                    ),
                ) / speed;
            let at_stop = |point: &Vec3| Vec3 {
                x: point.x + stop.origin.x,
                y: point.y + stop.origin.y,
                z: point.z + stop.origin.z,
            };
            connections.push(NavigationConnection {
                from: riding(&stop.origin),
                to: riding(&next.origin),
                mode: TravelMode::Mover,
                hint: None,
                entity: Some(NavigationEntityBinding {
                    model: Some(model),
                    bounds: Bounds {
                        min: at_stop(&local.min),
                        max: at_stop(&local.max),
                    },
                    raw: vec![entity.slot as i32, stop.actor.slot() as i32, next.actor.slot() as i32],
                }),
                id: stop.actor.slot() as i32,
                source_travel_type: 7,
                travel_seconds,
            });
        }
    }
    connections
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_bots::movement_contract::{MovementKind, MovementProfile};
    use qa_bots::scene::TracePolicy;
    use qa_content::q2::foundation::movers::Q2TrainStop;
    use qa_core::identity::{IdentityOwner, ProviderId};
    use qa_core::numeric::Q2_DONOR_PROFILE;
    use std::collections::{HashMap, HashSet};

    struct FakeSimulation {
        movers: Vec<BotMoverObservation>,
        entities: HashMap<ActorId, BotMoverEntity>,
        routes: HashMap<ActorId, Q2TrainRoute>,
        rerelease: bool,
    }

    impl BotMoverSimulation for FakeSimulation {
        fn has_q2_source(&self) -> bool {
            true
        }
        fn train_movers(&self) -> Vec<BotMoverObservation> {
            self.movers.clone()
        }
        fn q2_mover_entity(&self, actor: &ActorId) -> Option<BotMoverEntity> {
            self.entities.get(actor).copied()
        }
        fn q2_train_route(&self, actor: &ActorId) -> Option<Q2TrainRoute> {
            self.routes.get(actor).cloned()
        }
        fn q2_edition_is_rerelease(&self) -> bool {
            self.rerelease
        }
    }

    fn profile() -> NavigationProfile {
        NavigationProfile {
            movement: MovementProfile {
                kind: MovementKind::Q2Classic,
                id: ProviderId {
                    namespace: "q2".to_string(),
                    name: "classic".to_string(),
                },
                numeric: Q2_DONOR_PROFILE,
            },
            shape: BodyShape::Box(Bounds {
                min: Vec3 {
                    x: -16.0,
                    y: -16.0,
                    z: -24.0,
                },
                max: Vec3 {
                    x: 16.0,
                    y: 16.0,
                    z: 32.0,
                },
            }),
            crouched_shape: None,
            policy: TracePolicy::Q3 {
                contents_mask: -1,
                curves: true,
                player_curve_clip: true,
            },
            capabilities: HashSet::new(),
            maximum_step: 18.0,
            minimum_floor_normal: 0.7,
            maximum_drop: 200.0,
            team: None,
            monster: false,
        }
    }

    #[test]
    fn train_stops_become_mover_connections() {
        let owner = IdentityOwner::create("test").unwrap();
        let mover = owner.actor(5, 1);
        let stop_a = owner.actor(6, 1);
        let stop_b = owner.actor(7, 1);
        let bounds = Bounds {
            min: Vec3 {
                x: -32.0,
                y: -32.0,
                z: -8.0,
            },
            max: Vec3 {
                x: 32.0,
                y: 32.0,
                z: 8.0,
            },
        };
        let mut entities = HashMap::new();
        entities.insert(mover.clone(), BotMoverEntity { slot: 5, speed: 100.0 });
        entities.insert(stop_b.clone(), BotMoverEntity { slot: 7, speed: 0.0 });
        let mut routes = HashMap::new();
        routes.insert(
            mover.clone(),
            Q2TrainRoute {
                running: true,
                destination: Some(stop_b.clone()),
                stops: vec![
                    Q2TrainStop {
                        actor: stop_a.clone(),
                        origin: Vec3::default(),
                        next: Some(stop_b.clone()),
                        wait: 1.0,
                        teleport: false,
                    },
                    Q2TrainStop {
                        actor: stop_b.clone(),
                        origin: Vec3 {
                            x: 100.0,
                            y: 0.0,
                            z: 0.0,
                        },
                        next: None,
                        wait: 0.0,
                        teleport: false,
                    },
                ],
            },
        );
        let simulation = FakeSimulation {
            movers: vec![BotMoverObservation {
                actor: mover,
                bounds,
                model: Some(3),
            }],
            entities,
            routes,
            rerelease: false,
        };
        let connections = application_train_connections(&simulation, &profile());
        assert_eq!(connections.len(), 1);
        let connection = &connections[0];
        assert_eq!(connection.id, 6);
        assert_eq!(connection.source_travel_type, 7);
        assert_eq!(connection.mode, TravelMode::Mover);
        assert!((connection.travel_seconds - 2.0).abs() < 1e-9);
        assert_eq!(
            connection.from,
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 32.125
            }
        );
        assert_eq!(
            connection.to,
            Vec3 {
                x: 100.0,
                y: 0.0,
                z: 32.125
            }
        );
        let entity = connection.entity.as_ref().unwrap();
        assert_eq!(entity.model, Some(3));
        assert_eq!(entity.raw, vec![5, 6, 7]);
    }

    #[test]
    fn teleport_and_missing_links_are_skipped() {
        let owner = IdentityOwner::create("test").unwrap();
        let mover = owner.actor(5, 1);
        let bounds = Bounds {
            min: Vec3::default(),
            max: Vec3::default(),
        };
        let mut entities = HashMap::new();
        entities.insert(mover.clone(), BotMoverEntity { slot: 5, speed: 100.0 });
        let stop = owner.actor(6, 1);
        let mut routes = HashMap::new();
        routes.insert(
            mover.clone(),
            Q2TrainRoute {
                running: true,
                destination: None,
                stops: vec![Q2TrainStop {
                    actor: stop,
                    origin: Vec3::default(),
                    next: None,
                    wait: 0.0,
                    teleport: true,
                }],
            },
        );
        let simulation = FakeSimulation {
            movers: vec![BotMoverObservation {
                actor: mover,
                bounds,
                model: None,
            }],
            entities,
            routes,
            rerelease: false,
        };
        assert!(application_train_connections(&simulation, &profile()).is_empty());
    }
}
