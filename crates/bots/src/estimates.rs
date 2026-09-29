//! Shared metadata estimates, ported from
//! `src/bots/navigation/estimates.ts`. Execution remains in
//! [`NavigationRuntime`](crate::runtime::NavigationRuntime) movement
//! admission.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::error::BotsError;
use crate::estimate_aas::AasNavigationEstimates;
use crate::types::{
    NavigationAsset, NavigationEdge, NavigationEstimateQuery, NavigationEstimateResult, NavigationGraph,
};

/// Static edge eligibility under a travel-flag mask.
pub type Eligibility<'e> = dyn Fn(&NavigationEdge, Option<i32>) -> bool + 'e;

#[derive(Debug, Clone, Copy)]
struct Cost {
    node: i32,
    seconds: f64,
}

#[derive(Debug, Clone)]
struct Tail {
    seconds: f64,
    first_edge: Option<NavigationEdge>,
}

struct CostQueue {
    values: Vec<Cost>,
}

impl CostQueue {
    fn push(&mut self, value: Cost) {
        let mut index = self.values.len();
        self.values.push(value);
        while index > 0 {
            let parent = (index - 1) / 2;
            if self.values[parent].seconds <= self.values[index].seconds {
                break;
            }
            self.values.swap(index, parent);
            index = parent;
        }
    }

    fn pop(&mut self) -> Option<Cost> {
        let first = *self.values.first()?;
        let last = self.values.pop()?;
        if self.values.is_empty() {
            return Some(last);
        }
        let mut index = 0;
        while index * 2 + 1 < self.values.len() {
            let mut child = index * 2 + 1;
            if child + 1 < self.values.len() && self.values[child + 1].seconds < self.values[child].seconds {
                child += 1;
            }
            if last.seconds <= self.values[child].seconds {
                break;
            }
            self.values[index] = self.values[child];
            index = child;
        }
        self.values[index] = last;
        Some(first)
    }
}

/// Metadata cost estimates over a graph. Cache identity is the
/// graph/profile instance and explicit static-topology invalidation.
///
/// The donor holds the graph by reference; this port clones the edge
/// lists it relaxes so runtimes own their estimators without
/// self-references.
pub struct NavigationEstimates {
    aas: Option<AasNavigationEstimates>,
    incoming: HashMap<i32, Vec<NavigationEdge>>,
    node_ids: HashSet<i32>,
    kex_heuristic: Option<f64>,
    caches: HashMap<String, HashMap<i32, Tail>>,
    cache_order: VecDeque<String>,
}

impl NavigationEstimates {
    /// Build estimators over a graph.
    pub fn new(graph: &NavigationGraph) -> Result<Self, BotsError> {
        let aas = match &graph.asset {
            Some(NavigationAsset::Aas(asset)) => Some(AasNavigationEstimates::new(graph, asset)?),
            _ => None,
        };
        let mut incoming: HashMap<i32, Vec<NavigationEdge>> = HashMap::new();
        let mut node_ids = HashSet::new();
        for node in &graph.nodes {
            incoming.insert(node.id, Vec::new());
            node_ids.insert(node.id);
        }
        for edge in &graph.edges {
            let list = incoming
                .get_mut(&edge.to)
                .ok_or_else(|| BotsError::Internal("Estimate edge has no destination node".to_string()))?;
            if !edge.travel_seconds.is_finite() || edge.travel_seconds < 0.0 {
                return Err(BotsError::BadEdgeCost);
            }
            list.push(edge.clone());
        }
        let kex_heuristic = match &graph.asset {
            Some(NavigationAsset::Kex(asset)) => Some(asset.heuristic),
            _ => None,
        };
        Ok(Self {
            aas,
            incoming,
            node_ids,
            kex_heuristic,
            caches: HashMap::new(),
            cache_order: VecDeque::new(),
        })
    }

    /// Drop all cached tails.
    pub fn invalidate(&mut self) {
        if let Some(aas) = self.aas.as_mut() {
            aas.invalidate();
        }
        self.caches.clear();
        self.cache_order.clear();
    }

    fn tails(&mut self, goal: i32, flags: Option<i32>, allowed: &Eligibility<'_>) -> HashMap<i32, Tail> {
        let key = format!(
            "{goal}:{}",
            flags.map_or_else(|| "all".to_string(), |flags| flags.to_string())
        );
        if let Some(cached) = self.caches.get(&key) {
            self.cache_order.retain(|entry| *entry != key);
            self.cache_order.push_back(key);
            return cached.clone();
        }
        let mut tails: HashMap<i32, Tail> = HashMap::from([(
            goal,
            Tail {
                seconds: 0.0,
                first_edge: None,
            },
        )]);
        let mut queue = CostQueue { values: Vec::new() };
        queue.push(Cost {
            node: goal,
            seconds: 0.0,
        });
        while let Some(current) = queue.pop() {
            if tails
                .get(&current.node)
                .is_none_or(|tail| tail.seconds != current.seconds)
            {
                continue;
            }
            if let Some(edges) = self.incoming.get(&current.node) {
                for edge in edges {
                    if !allowed(edge, flags) {
                        continue;
                    }
                    let seconds = current.seconds + edge.travel_seconds;
                    if seconds >= tails.get(&edge.from).map_or(f64::INFINITY, |tail| tail.seconds) {
                        continue;
                    }
                    tails.insert(
                        edge.from,
                        Tail {
                            seconds,
                            first_edge: Some(edge.clone()),
                        },
                    );
                    queue.push(Cost {
                        node: edge.from,
                        seconds,
                    });
                }
            }
        }
        self.cache_order.push_back(key.clone());
        self.caches.insert(key, tails.clone());
        if self.caches.len() > 128 {
            if let Some(oldest) = self.cache_order.pop_front() {
                self.caches.remove(&oldest);
            }
        }
        tails
    }

    /// Estimate a cost query.
    pub fn estimate(
        &mut self,
        query: &NavigationEstimateQuery,
        allowed: &Eligibility<'_>,
    ) -> Result<NavigationEstimateResult, BotsError> {
        if !self.node_ids.contains(&query.start_node) || !self.node_ids.contains(&query.goal_node) {
            return Ok(NavigationEstimateResult::Unreachable);
        }
        if let Some(origin) = query.origin {
            if !origin.x.is_finite() || !origin.y.is_finite() || !origin.z.is_finite() {
                return Err(BotsError::InvalidEstimateOrigin);
            }
        }
        if let Some(aas) = self.aas.as_mut() {
            return aas.estimate(query, allowed);
        }
        if query.start_node == query.goal_node {
            return Ok(NavigationEstimateResult::Estimate {
                travel_time: 1,
                first_edge: None,
            });
        }
        let tail = self
            .tails(query.goal_node, query.travel_flags, allowed)
            .remove(&query.start_node);
        let Some(tail) = tail else {
            return Ok(NavigationEstimateResult::Unreachable);
        };
        let mut seconds = tail.seconds;
        if let (Some(origin), Some(first_edge)) = (query.origin, &tail.first_edge) {
            let start = first_edge.start;
            let dx = f64::from(start.x) - f64::from(origin.x);
            let dy = f64::from(start.y) - f64::from(origin.y);
            let dz = f64::from(start.z) - f64::from(origin.z);
            let heuristic = self.kex_heuristic.unwrap_or(1.0);
            seconds += dx.hypot(dy).hypot(dz) * heuristic / 320.0;
        }
        Ok(NavigationEstimateResult::Estimate {
            travel_time: ((seconds * 100.0) as f32).trunc().max(1.0) as i32,
            first_edge: if query.origin.is_none() { None } else { tail.first_edge },
        })
    }
}
