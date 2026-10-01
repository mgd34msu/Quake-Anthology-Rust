//! Bot navigation: AAS reachability/routing/movement, Kex NAV2/NAV3,
//! construction, estimates, and live routing. Ported from
//! `src/bots/navigation/*` (24 modules); the module layout mirrors the
//! donor file for file.

pub mod aas;
pub mod aas_cluster;
pub mod aas_optimize;
pub mod aas_prediction_stop;
pub mod aas_reachability;
pub mod aas_reachability_geometry;
pub mod aas_reachability_spatial;
pub mod aas_reachability_special;
pub mod aas_reachability_types;
pub mod aas_write;
pub mod behavior;
pub mod collision_support;
pub mod construct;
pub mod content;
pub mod entities;
pub mod entity_binding;
pub mod error;
pub mod estimate_aas;
pub mod estimates;
pub mod graph;
pub mod helpers;
pub mod load;
pub mod md4;
pub mod movement;
pub mod movement_contract;
pub mod nav;
pub mod q2_collision;
pub mod q3_collision;
pub mod rerelease_path;
pub mod runtime;
pub mod save;
pub mod scene;
pub mod train;
pub mod types;

pub use aas::{
    aas_bbox_areas, aas_point_area, aas_trace_areas, parse_aas, AasArea, AasAreaCrossing, AasAreaSettings, AasAsset,
    AasBbox, AasCluster, AasEdge, AasFace, AasLump, AasNode, AasPlane, AasPortal, AasReachability,
};
pub use aas_cluster::cluster_aas;
pub use aas_optimize::optimize_aas;
pub use aas_prediction_stop::aas_prediction_stop;
pub use aas_reachability::{build_aas_reachability, AasReachabilityDebugState, AasReachabilityOptions};
pub use aas_reachability_geometry::{
    aas_area_ground_face_area, aas_area_volume, aas_at, aas_barrier_jump_travel_time, aas_closest_edge_points,
    aas_face_area, aas_face_center, aas_fall_damage_distance, aas_fall_delta, aas_max_jump_distance,
    aas_max_jump_height, AasClosestEdgeRange, AasClosestEdgeState,
};
pub use aas_reachability_types::{
    default_aas_movement_settings, init_aas_movement_settings, AasClientMove, AasLibVarValue, AasMovementSettings,
    AasReachabilityWorld, AasStopEvent,
};
pub use aas_write::write_aas;
pub use behavior::{BotMovementPrediction, BotMovementStop, BotTravelPredictionResult, TravelType};
pub use construct::{construct_navigation, NavigationConnection, NavigationConstruction};
pub use content::{
    ContentDigest, ContentId, NavigationResources, OpenedResource, ResourceProvenance, ResourceReference,
};
pub use entity_binding::source_mover_bounds_match;
pub use error::BotsError;
pub use estimate_aas::{aas_estimate_area_time, AasNavigationEstimates};
pub use estimates::{Eligibility, NavigationEstimates};
pub use graph::{
    aas_area_travel_flags, aas_travel_flag, aas_travel_mode, kex_travel_mode, navigation_clusters,
    navigation_edge_travel_flag, navigation_from_asset,
};
pub use helpers::{
    at, clear, contents, crouched_profile, distance, midpoint, node_profile, trace, translated, validate_profile,
    NavigationContents,
};
pub use load::{
    load_navigation, load_prepared_navigation, preload_navigation, LoadedNavigation, NavigationLoadOptions,
    PreloadOptions, PreparedNavigation,
};
pub use movement::{
    create_movement_admission, create_movement_route_admission, MovementRouteAdmission, NavigationPrediction,
    NavigationPredictionDriver, NavigationPredictionLimits,
};
pub use movement_contract::{
    MovementExecution, MovementInput, MovementKind, MovementProfile, MovementProvider, MovementResult,
    MovementServices, MovementState,
};
pub use nav::{parse_kex_navigation, KexEntity, KexLink, KexNavigationAsset, KexNode};
pub use rerelease_path::{rerelease_path_to_goal, RereleasePathInfo, RereleasePathRequest};
pub use runtime::{
    BoardingElevator, BoardingTrain, NavigationDebugLine, NavigationRouteQuery, NavigationRuntime,
    NavigationRuntimeCheckpoint, TrainStage, TrainStep,
};
pub use train::{navigation_train_ride, NavigationTrainRide};
pub use types::{
    ElevatorPhase, ElevatorState, KexGeneration, NavigationAsset, NavigationEdge, NavigationEntityBinding,
    NavigationEntityState, NavigationEstimateQuery, NavigationEstimateResult, NavigationGraph, NavigationMapIdentity,
    NavigationNode, NavigationProfile, NavigationRoute, NavigationRoutePrediction, NavigationRouteResult,
    NavigationSource, NavigationWorld, RejectedSource, Team, TrainState, TrainStop, TravelMode, TraversalAdmission,
    TraversalHint, TraversalRequest,
};
