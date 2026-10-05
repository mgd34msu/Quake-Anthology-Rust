//! Test-only prediction scaffolding: recipe, scene, postures, and snapshots.
//!
//! No donor; supports the prediction ports' unit tests.

use std::cell::RefCell;
use std::rc::Rc;

use qa_content::contract::{
    CampaignSelection, CharacterSelection, ContentId, DopplerSelection, EnemySelection, EnvironmentSelection,
    EquipmentSelection, ExecutableRecipe, FrameOrdering, GrappleSelection, HandGrenadeSelection, LooseMount, MountId,
    MountIdentity, MountPlanId, PresentationSelection, ProviderReference, RecipeId, ResolvedMap, ResolvedMountPlan,
    ResolvedResourceReference, ResourceId, ResourceIdentity, ResourceProvenance, ResourceResolution,
};
use qa_core::identity::{IdentityOwner, OwnedActor, ProviderId};
use qa_core::math::{vec3, Bounds, Plane, Vec3};
use qa_world::hull::BspPlane;
use qa_world::movement::q1::types::{Q1Trace, Q1TraceQuery};
use qa_world::movement::q2::types::{Q2ContentsQuery, Q2Trace, Q2TracePlane, Q2TraceQuery};
use qa_world::movement::q3::postures::Q3_SOURCE_POSTURES;
use qa_world::movement::q3::types::{Q3Postures, Q3Trace, Q3TraceQuery};
use qa_world::movement::types::{AnimationState, TraceContact, TraceHit};

use super::types::{PredictionPostures, PredictionScene};

/// Empty-world scene: every trace misses, every point is empty air.
pub struct EmptyScene;

impl PredictionScene for EmptyScene {
    fn trace_q1(&mut self, query: Q1TraceQuery) -> Q1Trace {
        Q1Trace {
            fraction: 1.0,
            end: query.end,
            start_solid: false,
            all_solid: false,
            contact: TraceContact::None,
            hit: TraceHit::None,
            in_open: true,
            in_water: false,
            source_plane: Plane {
                normal: vec3(0.0, 0.0, 1.0),
                distance: 0.0,
            },
            surface_flags: None,
        }
    }

    fn point_contents_q1(&mut self, _point: Vec3) -> i32 {
        0
    }

    fn trace_q2(&mut self, query: Q2TraceQuery) -> Q2Trace {
        Q2Trace {
            fraction: 1.0,
            end: query.end,
            start_solid: false,
            all_solid: false,
            contact: TraceContact::None,
            hit: TraceHit::None,
            contents: 0,
            surface: None,
            source_plane: Q2TracePlane {
                normal: vec3(0.0, 0.0, 1.0),
                dist: 0.0,
                plane_type: 2,
                signbits: 0,
            },
            secondary: None,
        }
    }

    fn point_contents_q2(&mut self, _query: Q2ContentsQuery) -> (i32, i32) {
        (0, 0)
    }

    fn trace_q3(&mut self, query: Q3TraceQuery) -> Q3Trace {
        Q3Trace {
            fraction: 1.0,
            end: query.end,
            start_solid: false,
            all_solid: false,
            contact: TraceContact::None,
            hit: TraceHit::None,
            contents: 0,
            surface_flags: 0,
            source_plane: BspPlane {
                normal: vec3(0.0, 0.0, 1.0),
                distance: 0.0,
                plane_type: 2,
                signbits: 0,
            },
        }
    }

    fn point_contents_q3(&mut self, _point: Vec3, _pass_actor: &qa_core::identity::ActorId) -> i32 {
        0
    }
}

/// Stub posture source returning the Q3 source postures.
pub struct StubPostures;

impl PredictionPostures for StubPostures {
    fn postures(&self, _animation: &AnimationState, _standing_bounds: Bounds, _view_height: f64) -> Q3Postures {
        Q3_SOURCE_POSTURES
    }
}

fn provider_reference() -> ProviderReference {
    ProviderReference {
        provider: ProviderId::new("sim", "test"),
        content: ContentId("q1:baseq1:test:1".to_string()),
    }
}

/// Minimal executable recipe for prediction tests.
pub fn test_recipe() -> ExecutableRecipe {
    ExecutableRecipe {
        weapon_behaviors: Vec::new(),
        mods: Vec::new(),
        schema_version: 3,
        id: RecipeId("recipe:test:1".to_string()),
        preset: RecipeId("recipe:test:1".to_string()),
        map: ResolvedMap {
            geometry_content: ContentId("q1:baseq1:test:1".to_string()),
            geometry: ResolvedResourceReference {
                id: ResourceId("resource:test:map:1".to_string()),
                requested_path: "maps/test.bsp".to_string(),
                provenance: ResourceProvenance::Loose {
                    mount: LooseMount {
                        identity: MountIdentity {
                            id: MountId("mount:test:1".to_string()),
                            content: ContentId("q1:baseq1:test:1".to_string()),
                            generation: 1,
                        },
                        root_path: "/tmp".to_string(),
                    },
                    member_path: "maps/test.bsp".to_string(),
                },
                identity: ResourceIdentity {
                    mount_generation: 1,
                    member_index: 0,
                    byte_length: 0,
                    crc: 0,
                },
                byte_length: 0,
                resolution: ResourceResolution::DefaultOrder {
                    plan: MountPlanId("mountplan:test:1".to_string()),
                    rank: 0,
                },
            },
            entities: provider_reference(),
        },
        campaign: CampaignSelection::None,
        movement: provider_reference(),
        character: CharacterSelection {
            definition: provider_reference(),
            appearance: provider_reference(),
        },
        weapons: Vec::new(),
        equipment: EquipmentSelection {
            grapple: GrappleSelection::Disabled,
            hand_grenades: HandGrenadeSelection::Disabled,
        },
        enemies: EnemySelection::MapDefined,
        presentation: PresentationSelection {
            doppler: DopplerSelection::Source,
            environment: EnvironmentSelection::AudioContent,
            assets: ContentId("q1:baseq1:test:1".to_string()),
            hud: provider_reference(),
            effects: provider_reference(),
            audio: provider_reference(),
        },
        engine_behavior: provider_reference(),
        combat: provider_reference(),
        inventory: provider_reference(),
        r#match: provider_reference(),
        transition: provider_reference(),
        execution: Vec::new(),
        mounts: ResolvedMountPlan {
            id: MountPlanId("mountplan:test:1".to_string()),
            mounts: Vec::new(),
            default_order: Vec::new(),
            prefix_orders: Vec::new(),
        },
        resources: Vec::new(),
        timing: Vec::new(),
        ordering: FrameOrdering::Mixed { providers: Vec::new() },
    }
}

/// Minted test actor.
pub fn test_actor() -> OwnedActor {
    let owner = IdentityOwner::create("prediction-test").unwrap();
    let id = owner.actor(1, 1);
    owner.owned_actor(&id, ProviderId::new("sim", "test")).unwrap()
}

/// Shared empty scene handle.
pub fn empty_scene() -> Rc<RefCell<EmptyScene>> {
    Rc::new(RefCell::new(EmptyScene))
}

/// Shared stub posture handle.
pub fn stub_postures() -> Rc<StubPostures> {
    Rc::new(StubPostures)
}

/// Standard Quake standing hull.
pub fn standing_bounds() -> Bounds {
    Bounds {
        min: vec3(-16.0, -16.0, -24.0),
        max: vec3(16.0, 16.0, 32.0),
    }
}
