use qa_core::primitives::{
    Body, Bounds, CollisionOwner, CollisionShape, CollisionTags, EntityId, GeometryId, IndexSpan,
    ModuleId, MovementMode, Plane, PlayerState, RuleSetId, SurfaceFlags, ThinkTime, UserCmd, Vec3,
};
use qa_movement::{TraceServices, pmove, set_bounds};
use qa_world::{
    area::{AreaGrid, LinkFlags, LinkIntent, LinkOrder},
    collision::{
        CollisionStore, Contents, EntityTraceFlags, EntityTracePolicy, Trace, TraceError,
        TraceQuery, TraceRules, WorldTrace,
        brushes::{Brush, BrushTree},
        trace_policy,
    },
    entities::{AllocationPolicy, EntityTable},
};

struct Fixture {
    store: CollisionStore,
    geometry: GeometryId,
    entities: EntityTable,
    area: AreaGrid,
    target: EntityId,
}
impl Fixture {
    fn new(tag: CollisionTags) -> Self {
        let bounds = Bounds {
            mins: Vec3([-1024.0; 3]),
            maxs: Vec3([1024.0; 3]),
        };
        let mut store = CollisionStore::new();
        let geometry = store
            .load_brushes(
                vec![Plane::oriented(Vec3([-1.0, 0.0, 0.0]), -80.0)],
                vec![Brush {
                    sides: IndexSpan { first: 0, count: 1 },
                    contents: Contents::SOLID,
                }],
                qa_world::collision::surfaces::SurfaceTable::flags(vec![SurfaceFlags::default()]),
                BrushTree::direct(1).unwrap(),
                vec![bounds],
            )
            .unwrap();
        let mut entities = EntityTable::new(4, 1).unwrap();
        let target = entities
            .allocate(
                ThinkTime::Seconds(0.0),
                ModuleId(7),
                AllocationPolicy::EDICT,
            )
            .unwrap()
            .id;
        let slot = target.slot as usize;
        entities.columns.set_body(
            slot,
            Body {
                position: Vec3([48.0, 0.0, 64.0]),
                mins: Vec3([-4.0, -128.0, -128.0]),
                maxs: Vec3([4.0, 128.0, 128.0]),
                ..Default::default()
            },
        );
        entities.columns.collision_shape[slot] = CollisionShape::Box;
        // RR SVF_PLAYER classifies this temporary CONTENTS_MONSTER/BODY hull;
        // removing BODY would incorrectly remove actual monsters as well.
        entities.columns.collision_contents[slot] = Contents::BODY.0;
        entities.columns.collision_tags[slot] = tag;
        let mut area = AreaGrid::load(4, bounds).unwrap();
        assert!(area.link(
            &entities,
            target,
            LinkFlags::SOLID,
            LinkOrder::Tail,
            LinkIntent::Explicit
        ));
        Self {
            store,
            geometry,
            entities,
            area,
            target,
        }
    }
}
struct CheckedTrace<'a> {
    trace: WorldTrace<'a>,
    rules: TraceRules,
    entities: EntityTracePolicy,
    default_calls: usize,
    ignore: bool,
}
impl TraceServices for CheckedTrace<'_> {
    fn trace(&mut self, query: TraceQuery) -> Result<Trace, TraceError> {
        assert_eq!(query.rules, self.rules);
        assert_eq!(query.entity_rules, self.entities);
        if query.mask.intersects(Contents::BODY) {
            self.default_calls += 1;
            assert_eq!(query.mask.intersects(Contents::PLAYER), !self.ignore);
        }
        self.trace.trace(query)
    }
    fn point_contents(
        &self,
        point: Vec3,
        rules: EntityTracePolicy,
    ) -> Result<Contents, TraceError> {
        assert_eq!(rules, self.entities);
        self.trace.point_contents(point, rules, &[])
    }
}
fn player(rules: RuleSetId, trace: RuleSetId, ignore: bool) -> PlayerState {
    let mut player = PlayerState {
        movement_rules: rules,
        trace_rules: trace,
        health: 100,
        ..Default::default()
    };
    set_bounds(&mut player);
    player.body.position = Vec3([0.0, 0.0, 64.0]);
    player.body.velocity = Vec3([400.0, 0.0, 0.0]);
    player.movement.tuning.gravity = Some(0.0.into());
    player.movement.ignore_player_collision = ignore;
    player
}

#[test]
fn rr_default_ignore_skips_players_but_preserves_monsters_and_world_for_every_trace_role() {
    for trace_role in RuleSetId::ALL {
        for (tag, ignore, passes_target) in [
            (CollisionTags::PLAYER, false, false),
            (CollisionTags::PLAYER, true, true),
            (CollisionTags::MONSTER, true, false),
        ] {
            let fixture = Fixture::new(tag);
            let mut scratch = fixture.store.scratch();
            let (rules, mut entities) = trace_policy(trace_role);
            if ignore {
                entities.filtering.0 |= EntityTraceFlags::PLAYER_MASK;
            }
            let trace = WorldTrace::new(
                &fixture.store,
                fixture.geometry,
                0,
                &fixture.entities,
                &fixture.area,
                &mut scratch,
                CollisionOwner::None,
            );
            let mut trace = CheckedTrace {
                trace,
                rules,
                entities,
                default_calls: 0,
                ignore,
            };
            let mut player = player(RuleSetId::Quake2Rerelease, trace_role, ignore);
            pmove(
                UserCmd {
                    duration_ms: 255,
                    server_time_ms: 255,
                    ..Default::default()
                },
                &mut player,
                &mut trace,
                Default::default(),
                qa_movement::no_boundary,
            )
            .unwrap();
            assert!(trace.default_calls > 0);
            if passes_target {
                assert!(player.body.position.0[0] > 32.0, "{trace_role:?}");
                assert!(
                    player.body.position.0[0] <= 64.0,
                    "world wall still clips {trace_role:?}"
                );
            } else {
                assert!(
                    player.body.position.0[0] < 32.0,
                    "{trace_role:?} tag{}",
                    tag.0
                );
            }
            assert_eq!(player.trace_rules, trace_role);
        }
    }
}

#[test]
fn rr_explicit_player_mask_still_hits_the_same_canonical_player() {
    let fixture = Fixture::new(CollisionTags::PLAYER);
    for trace_role in RuleSetId::ALL {
        let mut scratch = fixture.store.scratch();
        let (rules, mut entity_rules) = trace_policy(trace_role);
        entity_rules.filtering.0 |= EntityTraceFlags::PLAYER_MASK;
        let mut trace = WorldTrace::new(
            &fixture.store,
            fixture.geometry,
            0,
            &fixture.entities,
            &fixture.area,
            &mut scratch,
            CollisionOwner::None,
        );
        let result = trace
            .trace(TraceQuery {
                // The foreign Q1 temporary hull is SOLID, while Q2/Q3 boxes
                // use BODY. The explicit PLAYER admission stays present.
                mask: Contents::SOLID | Contents::BODY | Contents::PLAYER,
                ..TraceQuery::point(
                    Vec3([0.0, 0.0, 64.0]),
                    Vec3([64.0, 0.0, 64.0]),
                    rules,
                    entity_rules,
                )
            })
            .unwrap();
        assert_eq!(
            result.entity,
            Some(fixture.target),
            "{trace_role:?}: {result:?}"
        );
        assert!(result.fraction < 1.0);
    }
}

#[test]
fn foreign_movement_does_not_adopt_the_rr_mask_capability_implicitly() {
    let fixture = Fixture::new(CollisionTags::PLAYER);
    let mut scratch = fixture.store.scratch();
    let mut trace = WorldTrace::new(
        &fixture.store,
        fixture.geometry,
        0,
        &fixture.entities,
        &fixture.area,
        &mut scratch,
        CollisionOwner::None,
    );
    let mut player = player(RuleSetId::Quake3, RuleSetId::Quake2, true);
    pmove(
        UserCmd {
            server_time_ms: 255,
            duration_ms: 255,
            ..Default::default()
        },
        &mut player,
        &mut trace,
        Default::default(),
        qa_movement::no_boundary,
    )
    .unwrap();
    assert!(player.body.position.0[0] < 32.0);
    assert!(player.movement.ignore_player_collision);
}

#[test]
fn rr_free_flags_clear_both_semantic_flags_before_return() {
    let fixture = Fixture::new(CollisionTags::PLAYER);
    for mode in [MovementMode::Spectator, MovementMode::Noclip] {
        let mut scratch = fixture.store.scratch();
        let mut trace = WorldTrace::new(
            &fixture.store,
            fixture.geometry,
            0,
            &fixture.entities,
            &fixture.area,
            &mut scratch,
            CollisionOwner::None,
        );
        let mut player = player(RuleSetId::Quake2Rerelease, RuleSetId::Quake3, true);
        player.movement.mode = mode;
        player.movement.respawned = true;
        pmove(
            UserCmd {
                duration_ms: 16,
                ..Default::default()
            },
            &mut player,
            &mut trace,
            Default::default(),
            qa_movement::no_boundary,
        )
        .unwrap();
        assert!(!player.movement.ignore_player_collision);
        assert!(!player.movement.respawned);
    }
}

struct BrokenTrace;
impl TraceServices for BrokenTrace {
    fn trace(&mut self, _: TraceQuery) -> Result<Trace, TraceError> {
        Err(TraceError::Scratch)
    }
    fn point_contents(&self, _: Vec3, _: EntityTracePolicy) -> Result<Contents, TraceError> {
        Err(TraceError::Scratch)
    }
}
#[test]
fn failed_rr_free_move_restores_both_semantic_flags() {
    let mut player = player(RuleSetId::Quake2Rerelease, RuleSetId::Quake3, true);
    player.movement.mode = MovementMode::Spectator;
    player.movement.respawned = true;
    let before = player.movement;
    assert_eq!(
        pmove(
            UserCmd {
                duration_ms: 16,
                ..Default::default()
            },
            &mut player,
            &mut BrokenTrace,
            Default::default(),
            qa_movement::no_boundary
        )
        .unwrap_err(),
        TraceError::Scratch
    );
    assert_eq!(player.movement, before);
}
