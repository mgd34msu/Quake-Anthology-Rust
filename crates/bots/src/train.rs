//! Train rides over directed source routes, ported from
//! `src/bots/navigation/train.ts`.

use qa_core::math::Vec3;

use crate::types::{NavigationEdge, NavigationEntityState, NavigationProfile, TrainStop, TravelMode};

/// Boarding and arrival stops of a train ride.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NavigationTrainRide {
    /// Boarding stop.
    pub boarding: TrainStop,
    /// Arrival stop.
    pub arrival: TrainStop,
}

fn supports(
    state: &NavigationEntityState,
    train: &crate::types::TrainState,
    stop: &TrainStop,
    point: Vec3,
    profile: &NavigationProfile,
) -> bool {
    let x = f64::from(stop.origin.x) - f64::from(train.origin.x);
    let y = f64::from(stop.origin.y) - f64::from(train.origin.y);
    let z = f64::from(stop.origin.z) - f64::from(train.origin.z);
    let bounds = profile.shape.bounds();
    f64::from(point.x) + f64::from(bounds.max.x) > f64::from(state.bounds.min.x) + x
        && f64::from(point.x) + f64::from(bounds.min.x) < f64::from(state.bounds.max.x) + x
        && f64::from(point.y) + f64::from(bounds.max.y) > f64::from(state.bounds.min.y) + y
        && f64::from(point.y) + f64::from(bounds.min.y) < f64::from(state.bounds.max.y) + y
        && (f64::from(point.z) + f64::from(bounds.min.z) - (f64::from(state.bounds.max.z) + z)).abs()
            <= profile.maximum_step
}

/// A ride is a directed source route, including waits and teleport
/// corners, rather than a line to a mover's current destination.
pub fn navigation_train_ride(
    state: &NavigationEntityState,
    edge: &NavigationEdge,
    profile: &NavigationProfile,
) -> Option<NavigationTrainRide> {
    let train = state.train.as_ref()?;
    if !state.enabled || state.locked || !profile.capabilities.contains(&TravelMode::Mover) {
        return None;
    }
    let boarding = train
        .stops
        .iter()
        .find(|stop| !stop.teleport && supports(state, train, stop, edge.start, profile))?;
    let arrival = train
        .stops
        .iter()
        .find(|stop| !stop.teleport && supports(state, train, stop, edge.end, profile))?;
    if boarding.id == arrival.id {
        return None;
    }
    let mut visited = std::collections::HashSet::new();
    let mut current: Option<&TrainStop> = Some(boarding);
    while let Some(stop) = current {
        if visited.contains(&stop.id) {
            break;
        }
        if stop.id == arrival.id {
            return Some(NavigationTrainRide {
                boarding: *boarding,
                arrival: *arrival,
            });
        }
        if stop.wait < 0.0 || stop.teleport {
            return None;
        }
        visited.insert(stop.id);
        current = match stop.next {
            Some(next) => train.stops.iter().find(|stop| stop.id == next),
            None => None,
        };
    }
    None
}
