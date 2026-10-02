//! Application bot movement prediction over detached selected movement.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/simulation/bot-prediction.ts`.
//!
//! Missing siblings: `SharedSimulation` (`runtime.ts`, runtime partition),
//! `MovementPredictionPlayer` plus `createPlayerMovementPrediction` and
//! `movementObservation` (`player-movement.ts`, player-movement partition).
//! The [`PlayerMovementPrediction`] seam exposes exactly the donor's
//! projection surface; the partitions implement it post-merge. The donor
//! builds the projection from `(simulation, player)` internally; the port
//! takes the built projection because both live in missing partitions.
//!
//! The donor reads origins through `movementOrigin` from `./players.ts`,
//! which is the q2-classic eighths scaling over [`MovementState::origin`];
//! the port calls [`MovementState::origin`] directly.

use qa_bots::aas::{aas_point_area, AasAsset};
use qa_bots::aas_prediction_stop::aas_prediction_stop;
use qa_bots::behavior::prediction::{project_bot_movement, BotMovementProjection};
use qa_bots::behavior::q3::navigation_types::BotMovementPrediction;
use qa_bots::behavior::q3::travel::types::{BotMovementStop, BotTravelPredictionResult};
use qa_bots::error::BotsError;
use qa_bots::movement_contract::{MovementInput, MovementProvider, MovementResult, MovementServices};
use qa_bots::runtime::NavigationRuntime;
use qa_bots::types::NavigationAsset;
use qa_core::math::Vec3;

/// Prediction medium (donor `movementObservation` medium spellings).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PredictionMedium {
    /// Dry.
    Dry,
    /// Water.
    Water,
    /// Slime.
    Slime,
    /// Lava.
    Lava,
}

/// Movement observation (donor `movementObservation` shape).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MovementObservation {
    /// Origin.
    pub origin: Vec3,
    /// Grounded.
    pub grounded: bool,
    /// Medium.
    pub medium: PredictionMedium,
    /// Damaging fall.
    pub damaging_fall: bool,
}

/// Player movement projection (donor `createPlayerMovementPrediction`
/// result plus `movementObservation`).
pub trait PlayerMovementPrediction {
    /// Selected movement provider.
    fn provider(&self) -> &dyn MovementProvider;
    /// Services for the projection.
    fn services(&self) -> MovementServices;
    /// Build input for a frame.
    fn input(&self, previous: Option<&MovementResult>, frame: i32, command_move: Vec3) -> MovementInput;
    /// Observe movement output (donor `movementObservation`).
    fn observe(&self, result: &MovementResult) -> MovementObservation;
}

/// Navigation source (donor `NavigationRuntime | AasAsset`).
pub enum BotPredictionNavigation<'a> {
    /// Shared navigation runtime.
    Runtime(&'a NavigationRuntime<'a>),
    /// Raw AAS asset.
    Aas(&'a AasAsset),
}

impl BotPredictionNavigation<'_> {
    fn asset(&self) -> Option<&AasAsset> {
        match self {
            BotPredictionNavigation::Runtime(runtime) => match runtime.graph.asset.as_ref() {
                Some(NavigationAsset::Aas(asset)) => Some(asset),
                _ => None,
            },
            BotPredictionNavigation::Aas(asset) => Some(asset),
        }
    }

    fn area_at(&self, origin: Vec3) -> Option<i32> {
        match self {
            BotPredictionNavigation::Runtime(runtime) => runtime.area_at(origin).unwrap_or(None),
            BotPredictionNavigation::Aas(asset) => aas_point_area(asset, origin).ok(),
        }
    }
}

struct ApplicationBotProjection<'a> {
    base: &'a dyn PlayerMovementPrediction,
    query: &'a BotMovementPrediction,
    navigation: &'a BotPredictionNavigation<'a>,
}

impl BotMovementProjection for ApplicationBotProjection<'_> {
    fn provider(&self) -> &dyn MovementProvider {
        self.base.provider()
    }

    fn services(&self) -> MovementServices {
        self.base.services()
    }

    fn input(&self, previous: Option<&MovementResult>, frame: i32, command_move: Vec3) -> MovementInput {
        let command = if self.query.presence == 4 {
            Vec3 {
                x: command_move.x,
                y: command_move.y,
                z: -400.0,
            }
        } else {
            command_move
        };
        self.base.input(previous, frame, command)
    }

    fn stop(&self, previous: Option<&MovementResult>, result: &MovementResult, frame: i32) -> BotMovementStop {
        if !matches!(result, MovementResult::Active { .. }) {
            panic!("Navigation projection removed its actor");
        }
        let observation = self.base.observe(result);
        if let Some(asset) = self.navigation.asset() {
            let start = match previous {
                Some(MovementResult::Active { state, .. }) => state.origin(),
                _ => self.query.origin,
            };
            let crossing = aas_prediction_stop(
                asset,
                start,
                observation.origin,
                frame,
                self.query.stop_events,
                self.query.stop_area,
            )
            .unwrap_or(None);
            if let Some(crossing) = crossing {
                return crossing;
            }
        }
        let was_grounded = match previous {
            Some(previous @ MovementResult::Active { .. }) => self.base.observe(previous).grounded,
            _ => self.query.on_ground,
        };
        let mut flags = if !was_grounded && observation.grounded {
            1
        } else if was_grounded && !observation.grounded {
            2
        } else {
            0
        };
        if observation.medium != PredictionMedium::Dry {
            flags |= match observation.medium {
                PredictionMedium::Slime => 8,
                PredictionMedium::Lava => 16,
                _ => 4,
            };
        }
        let area = self.navigation.area_at(observation.origin);
        if self.query.stop_area != 0 && area == Some(self.query.stop_area) {
            flags |= 512 | if !was_grounded && observation.grounded { 1024 } else { 0 };
        }
        if observation.damaging_fall {
            flags |= 32;
        }
        BotMovementStop {
            events: flags,
            origin: observation.origin,
            area,
        }
    }
}

/// Gameplay and AAS authoring share detached selected movement and the
/// caller's exact area graph.
pub fn predict_application_bot_movement(
    prediction: &dyn PlayerMovementPrediction,
    query: &BotMovementPrediction,
    navigation: &BotPredictionNavigation<'_>,
) -> Result<BotTravelPredictionResult, BotsError> {
    let projection = ApplicationBotProjection {
        base: prediction,
        query,
        navigation,
    };
    project_bot_movement(query, &projection)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_bots::movement_contract::{MovementExecution, MovementKind, MovementProfile, MovementState};
    use qa_bots::scene::TraceHit;
    use qa_core::identity::ProviderId;
    use qa_core::numeric::Q3_BINARY32_PROFILE;
    use qa_core::time::{FrameContext, FramePhase, SourceTime};

    struct StillProvider {
        profile: MovementProfile,
    }

    impl MovementProvider for StillProvider {
        fn kind(&self) -> MovementKind {
            MovementKind::Q3
        }
        fn id(&self) -> &ProviderId {
            &self.profile.id
        }
        fn advance(&self, _input: &MovementInput, _services: &MovementServices) -> MovementResult {
            MovementResult::Active {
                state: MovementState::Q3 {
                    origin: Vec3 { x: 4.0, y: 0.0, z: 0.0 },
                },
                ground: TraceHit::None,
                water_level: 0,
            }
        }
    }

    struct StillPrediction {
        provider: StillProvider,
        grounded: bool,
    }

    impl PlayerMovementPrediction for StillPrediction {
        fn provider(&self) -> &dyn MovementProvider {
            &self.provider
        }
        fn services(&self) -> MovementServices {
            MovementServices {
                numeric: Q3_BINARY32_PROFILE,
            }
        }
        fn input(&self, _previous: Option<&MovementResult>, _frame: i32, _command_move: Vec3) -> MovementInput {
            MovementInput {
                kind: MovementKind::Q3,
                profile: self.provider.profile.clone(),
                frame: FrameContext {
                    frame: 0,
                    time: SourceTime::Milliseconds(0),
                    elapsed: SourceTime::Milliseconds(50),
                    phase: FramePhase::ClientCommand,
                },
                execution: MovementExecution::Prediction,
            }
        }
        fn observe(&self, result: &MovementResult) -> MovementObservation {
            let MovementResult::Active { state, .. } = result else {
                panic!("removed");
            };
            MovementObservation {
                origin: state.origin(),
                grounded: self.grounded,
                medium: PredictionMedium::Dry,
                damaging_fall: false,
            }
        }
    }

    fn prediction(grounded: bool) -> StillPrediction {
        StillPrediction {
            provider: StillProvider {
                profile: MovementProfile {
                    kind: MovementKind::Q3,
                    id: ProviderId {
                        namespace: "q3".to_string(),
                        name: "test".to_string(),
                    },
                    numeric: Q3_BINARY32_PROFILE,
                },
            },
            grounded,
        }
    }

    fn query() -> BotMovementPrediction {
        BotMovementPrediction {
            entity_num: 0,
            origin: Vec3::default(),
            presence: 2,
            on_ground: false,
            velocity: Vec3::default(),
            command_move: Vec3::default(),
            command_frames: 0,
            max_frames: 4,
            frame_time: 0.05,
            stop_events: 1,
            stop_area: 0,
            visualize: false,
        }
    }

    fn empty_asset() -> AasAsset {
        AasAsset {
            source: String::new(),
            version: 0,
            bsp_checksum: 0,
            lumps: Vec::new(),
            bboxes: Vec::new(),
            vertices: Vec::new(),
            planes: Vec::new(),
            edges: Vec::new(),
            edge_indexes: Vec::new(),
            faces: Vec::new(),
            face_indexes: Vec::new(),
            areas: Vec::new(),
            settings: Vec::new(),
            reachability: Vec::new(),
            nodes: Vec::new(),
            portals: Vec::new(),
            portal_indexes: Vec::new(),
            clusters: Vec::new(),
        }
    }

    #[test]
    fn landing_reports_touchdown() {
        let prediction = prediction(true);
        let asset = empty_asset();
        let navigation = BotPredictionNavigation::Aas(&asset);
        let projection = ApplicationBotProjection {
            base: &prediction,
            query: &query(),
            navigation: &navigation,
        };
        let result = MovementResult::Active {
            state: MovementState::Q3 {
                origin: Vec3 { x: 4.0, y: 0.0, z: 0.0 },
            },
            ground: TraceHit::None,
            water_level: 0,
        };
        let stop = projection.stop(None, &result, 1);
        assert_eq!(stop.events, 1);
        assert_eq!(stop.origin, Vec3 { x: 4.0, y: 0.0, z: 0.0 });
    }

    #[test]
    fn projection_stops_on_first_touchdown() {
        let prediction = prediction(true);
        let asset = empty_asset();
        let navigation = BotPredictionNavigation::Aas(&asset);
        let result = predict_application_bot_movement(&prediction, &query(), &navigation).unwrap();
        assert_eq!(result.frames, 1);
        assert_eq!(result.stop_event, 1);
        assert_eq!(result.end, Vec3 { x: 4.0, y: 0.0, z: 0.0 });
    }
}
