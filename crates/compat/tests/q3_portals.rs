#[path = "support/quakec_program.rs"]
#[allow(dead_code)]
mod quakec_program;

use qa_compat::{
    abi::{Addresses, Invocation, Q3_SERVER, UnknownCalls},
    entities::EntityProjection,
    memory::ModuleMemory,
    services::{CallContext, CallError, EngineServices, ServiceStorage},
};
use qa_console::{command_buffer::CommandBuffer, cvars::Cvars, views::Context};
use qa_core::{
    names::NameTable,
    primitives::{Bounds, GeometryId, ModuleId, Plane, RuleSetId, ThinkTime, Vec3},
    sys_events::EventTime,
};
use qa_session::clients::Server;
use qa_world::{
    area::LinkOrder,
    collision::{CollisionStore, TraceScratch},
    entities::AllocationPolicy,
    leaves::LeafScratch,
    portals::AreaPortals,
    visibility::{PvsRows, SurfaceSpan, VisLeaf, VisNode, VisibilityWorld},
};

const MODULE: ModuleId = ModuleId(7);
const ENTITY_OFFSET: u64 = 512;
const PLAYER_OFFSET: u64 = 2048;
const ENTITY_BYTES: u64 = 516;
const PLAYER_BYTES: u64 = 468;

fn context() -> CallContext {
    CallContext {
        module: MODULE,
        clock: ThinkTime::Milliseconds(0),
        server_frame: 0,
        native_rules: RuleSetId::Quake3,
        trace_rules: RuleSetId::Quake3,
        sound_policy: qa_gameplay::rules::sound_policy(RuleSetId::Quake3),
        console: Context {
            source: RuleSetId::Quake3,
            ..Context::default()
        },
        allocation: AllocationPolicy::q3(0),
        link_order: LinkOrder::Head,
    }
}

struct Fixture {
    server: Server,
    input: qa_input::Input,
    cvars: Cvars,
    commands: CommandBuffer,
    vfs: qa_content::vfs::Vfs,
    localization: qa_content::localization::Catalog,
    registry: qa_gameplay::registry::Registry,
    names: NameTable,
    storage: ServiceStorage,
    geometry: CollisionStore,
    scratch: TraceScratch,
    collision_world: Option<GeometryId>,
    native_geometry: Option<GeometryId>,
    world: VisibilityWorld,
    leaves: LeafScratch,
    row: Box<[u8]>,
    portals: AreaPortals,
}

impl Fixture {
    fn new() -> Self {
        let names = NameTable::load(qa_gameplay::registry::Registry::names_needed()).unwrap();
        let registry = qa_gameplay::registry::Registry::load(&names).unwrap();
        let cvars = Cvars::new().unwrap();
        let mut storage =
            ServiceStorage::load_native(&[(MODULE, 0, RuleSetId::Quake3)], 0, &cvars).unwrap();
        storage.bind_client_capacity(MODULE, 1).unwrap();
        let bounds = Bounds {
            mins: Vec3([-16.0; 3]),
            maxs: Vec3([16.0; 3]),
        };
        let world = VisibilityWorld::load(
            vec![Plane::oriented(Vec3([1.0, 0.0, 0.0]), 0.0)],
            vec![VisNode {
                plane: 0,
                children: [-1, -2],
                bounds,
                surfaces: SurfaceSpan::default(),
            }],
            [0, 1]
                .map(|area| VisLeaf {
                    selector: Some(area),
                    area: Some(area),
                    solid: false,
                    bounds,
                    surfaces: SurfaceSpan::default(),
                })
                .to_vec(),
            vec![],
            0,
            0,
            PvsRows::load(vec![Some(0), Some(1)], vec![3, 3], None).unwrap(),
        )
        .unwrap();
        let leaves = LeafScratch::new(world.node_count() + 1);
        let row = vec![0; world.pvs().row_bytes()].into_boxed_slice();
        let geometry = CollisionStore::new();
        let scratch = geometry.scratch();
        Self {
            server: Server::load(
                1,
                16,
                registry.items.len() + 1,
                registry.powerup_capacity(),
                registry.weapons.len() + 1,
                64,
            )
            .unwrap(),
            input: qa_input::Input::load(),
            cvars,
            commands: CommandBuffer::new(),
            vfs: Default::default(),
            localization: Default::default(),
            registry,
            names,
            storage,
            geometry,
            scratch,
            collision_world: None,
            native_geometry: None,
            world,
            leaves,
            row,
            portals: AreaPortals::load(2, 0, vec![]).unwrap(),
        }
    }

    fn services(&mut self) -> EngineServices<'_> {
        EngineServices {
            server: &mut self.server,
            callbacks: None,
            client_disconnect: None,
            input: &mut self.input,
            editor: None,
            cvars: &mut self.cvars,
            commands: &mut self.commands,
            client_connection: None,
            client_collision: None,
            renderer: None,
            scene: None,
            client_audio: None,
            vfs: &self.vfs,
            model_loader: None,
            model_lookup: None,
            sound_loader: None,
            sound_bank: None,
            localization: &self.localization,
            registry: &self.registry,
            names: &mut self.names,
            storage: &mut self.storage,
            geometry: &self.geometry,
            world: self.collision_world.map(|geometry| (geometry, 0)),
            native_geometry: self.native_geometry,
            foreign_trace: None,
            visibility: Some((&self.world, &mut self.leaves, &mut self.row)),
            portals: Some(&mut self.portals),
            scratch: &mut self.scratch,
        }
    }
}

fn invoke(
    fixture: &mut Fixture,
    projection: &mut EntityProjection,
    memory: &mut ModuleMemory<'_>,
    addresses: Addresses,
    number: u32,
    arguments: &[u64],
) -> Result<u64, CallError> {
    let mut unknown = UnknownCalls::load(1).unwrap();
    let result = Q3_SERVER.invoke(
        number,
        &mut Invocation {
            services: &mut fixture.services(),
            memory,
            native_cvars: None,
            native_resources: None,
            native_entities: Some(projection),
            native_surfaces: None,
            native_command: None,
            native_configs: None,
            native_pmove: None,
            native_player: None,
            context: context(),
            platform_time: EventTime(0),
            command: Default::default(),
            argument_selection: &mut qa_compat::abi::ArgumentSelection::RetainedClient,
            addresses,
            arguments,
        },
        &mut unknown,
    );
    assert_eq!(unknown.calls, 0);
    result.map(|step| match step {
        qa_compat::abi::ImportStep::Return(value) => value,
        qa_compat::abi::ImportStep::Host { .. } => panic!("unexpected host request"),
    })
}

fn address_modes() -> [(Addresses, u64, u64); 3] {
    [
        (Addresses::Qvm { mask: 8191 }, 0, 8192),
        (
            Addresses::Native {
                abi: qa_platform::native::NativeAbi::SystemV,
                pointer_bytes: 4,
            },
            0x2000_0000,
            0,
        ),
        (
            Addresses::Native {
                abi: qa_platform::native::NativeAbi::SystemV,
                pointer_bytes: 8,
            },
            1 << 40,
            0,
        ),
    ]
}

#[test]
fn q3_brush_model_publication_and_live_queries_use_owner_geometry() {
    use qa_core::primitives::{CollisionShape, IndexSpan, NativeEntity, SurfaceFlags};
    use qa_world::collision::{
        Contents, NativeTraceEntities, NativeTraceTarget, TraceQuery,
        brushes::{Brush, BrushTree, CollisionLeaf, ModelRoot},
        surfaces::SurfaceTable,
        trace_policy,
    };

    fn geometry(store: &mut CollisionStore, extent: f32) -> GeometryId {
        let mut planes = Vec::new();
        for axis in 0..3 {
            let mut normal = Vec3::default();
            normal.0[axis] = 1.0;
            planes.push(Plane::oriented(normal, extent));
            normal.0[axis] = -1.0;
            planes.push(Plane::oriented(normal, extent));
        }
        store
            .load_brushes(
                planes,
                vec![Brush {
                    sides: IndexSpan { first: 0, count: 6 },
                    contents: Contents::SOLID,
                }],
                Vec::new(),
                SurfaceTable::flags(vec![SurfaceFlags::default(); 6]),
                BrushTree {
                    planes: Vec::new(),
                    nodes: Vec::new(),
                    leaves: vec![
                        CollisionLeaf {
                            stored_contents: None,
                            brushes: IndexSpan::default(),
                            patches: Default::default(),
                        },
                        CollisionLeaf {
                            stored_contents: None,
                            brushes: IndexSpan { first: 0, count: 1 },
                            patches: Default::default(),
                        },
                    ],
                    leaf_brushes: vec![0],
                    models: vec![ModelRoot::Leaf(0), ModelRoot::Leaf(1)],
                    leaf_patches: Vec::new(),
                },
                vec![
                    Bounds {
                        mins: Vec3([-1024.0; 3]),
                        maxs: Vec3([1024.0; 3]),
                    },
                    Bounds {
                        mins: Vec3([-extent; 3]),
                        maxs: Vec3([extent; 3]),
                    },
                ],
            )
            .unwrap()
    }

    for (addresses, base, pointer_alias) in address_modes() {
        let mut fixture = Fixture::new();
        let query_geometry = geometry(&mut fixture.geometry, 4.0);
        let owner_geometry = geometry(&mut fixture.geometry, 8.0);
        fixture.scratch = fixture.geometry.scratch();
        fixture.collision_world = Some(query_geometry);
        fixture.native_geometry = Some(owner_geometry);
        let mut projection = EntityProjection::load_q3();
        projection.seed(&fixture.server.entities, MODULE).unwrap();
        let mut memory = ModuleMemory::load(base, 8192, &[]).unwrap();
        let entity = base + ENTITY_OFFSET;
        let player = base + PLAYER_OFFSET;
        let name = base + 3296;
        memory.write_word(entity, 0).unwrap();
        memory.write_word(entity + 512, 1023).unwrap();
        memory.write(name, b"*1\0").unwrap();
        invoke(
            &mut fixture,
            &mut projection,
            &mut memory,
            addresses,
            15,
            &[
                entity + pointer_alias,
                1,
                ENTITY_BYTES,
                player + pointer_alias,
                PLAYER_BYTES,
            ],
        )
        .unwrap();
        invoke(
            &mut fixture,
            &mut projection,
            &mut memory,
            addresses,
            23,
            &[entity + pointer_alias, name + pointer_alias],
        )
        .unwrap();
        assert_eq!(memory.read_vec3(entity + 436).unwrap(), Vec3([-8.0; 3]));
        assert_eq!(memory.read_vec3(entity + 448).unwrap(), Vec3([8.0; 3]));
        let native = NativeEntity {
            module: MODULE,
            slot: 0,
        };
        let common = fixture.server.entities.find_native(native, None).unwrap();
        assert_eq!(
            fixture.server.entities.columns.collision_shape[common.slot as usize],
            CollisionShape::Model {
                geometry: owner_geometry,
                index: 1
            }
        );

        let output = base + 3072;
        let start = base + 3264;
        let end = base + 3280;
        memory.write_vec3(start, Vec3([-50.0, 0.0, 0.0])).unwrap();
        memory.write_vec3(end, Vec3([50.0, 0.0, 0.0])).unwrap();
        let (rules, entities) = trace_policy(context().trace_rules);
        let expected = fixture
            .geometry
            .trace_model(
                owner_geometry,
                1,
                TraceQuery::point(
                    Vec3([-50.0, 0.0, 0.0]),
                    Vec3([50.0, 0.0, 0.0]),
                    rules,
                    entities,
                ),
                &mut fixture.scratch,
            )
            .unwrap();
        invoke(
            &mut fixture,
            &mut projection,
            &mut memory,
            addresses,
            24,
            &[
                output + pointer_alias,
                start + pointer_alias,
                0,
                0,
                end + pointer_alias,
                1023,
                u64::from(u32::MAX),
            ],
        )
        .unwrap();
        assert_eq!(
            memory.read_word(output + 8).unwrap() as u32,
            expected.fraction.to_bits()
        );
        memory.write_vec3(start, Vec3([6.0, 0.0, 0.0])).unwrap();
        assert_eq!(
            invoke(
                &mut fixture,
                &mut projection,
                &mut memory,
                addresses,
                25,
                &[start + pointer_alias, 1023]
            )
            .unwrap(),
            u64::from(Contents::SOLID.to_q3())
        );

        let columns = &fixture.server.entities.columns;
        let cached = NativeTraceTarget {
            body: columns.body(common.slot as usize),
            angles: Vec3::default(),
            shape: columns.collision_shape[common.slot as usize],
            rules: columns.model_rules[common.slot as usize],
            contents_pose: None,
        };
        let view = projection.trace_view(&memory, context(), None).unwrap();
        assert!(view.fields(native, false).is_some());
        assert!(
            view.target(
                native,
                cached,
                false,
                &fixture.geometry,
                Some(&fixture.storage)
            )
            .is_none()
        );
        assert_eq!(view.finish(), Err(CallError::Geometry));
        memory.write_word(entity + 432, 0).unwrap();
        let view = projection.trace_view(&memory, context(), None).unwrap();
        assert_eq!(
            view.target(
                native,
                cached,
                false,
                &fixture.geometry,
                Some(&fixture.storage)
            )
            .unwrap()
            .shape,
            CollisionShape::Box
        );
        assert_eq!(view.finish(), Ok(()));
    }
}

#[test]
fn q3_portal_import_uses_link_cache_and_native_boundaries() {
    for (addresses, base, pointer_alias) in address_modes() {
        let mut fixture = Fixture::new();
        let mut projection = EntityProjection::load_q3();
        projection.seed(&fixture.server.entities, MODULE).unwrap();
        let mut memory = ModuleMemory::load(base, 8192, &[]).unwrap();
        let entity = base + ENTITY_OFFSET;
        let player = base + PLAYER_OFFSET;
        let entity_arg = entity + pointer_alias;
        let player_arg = player + pointer_alias;
        memory.write_word(entity, 0).unwrap();
        memory.write_word(entity + 460, 1).unwrap();
        memory.write_word(entity + 512, 1023).unwrap();
        memory.write_vec3(entity + 436, Vec3([-2.0; 3])).unwrap();
        memory.write_vec3(entity + 448, Vec3([2.0; 3])).unwrap();
        memory.write_vec3(entity + 488, Vec3::default()).unwrap();
        memory.write_vec3(entity + 500, Vec3::default()).unwrap();
        invoke(
            &mut fixture,
            &mut projection,
            &mut memory,
            addresses,
            15,
            &[entity_arg, 1, ENTITY_BYTES, player_arg, PLAYER_BYTES],
        )
        .unwrap();

        assert_eq!(
            invoke(
                &mut fixture,
                &mut projection,
                &mut memory,
                addresses,
                28,
                &[0, 1],
            ),
            Err(CallError::Entity)
        );
        invoke(
            &mut fixture,
            &mut projection,
            &mut memory,
            addresses,
            30,
            &[entity_arg],
        )
        .unwrap();
        assert_q3_link_alias_reads(&mut fixture);
        assert_eq!(fixture.portals.connected(0, 1), Some(false));

        invoke(
            &mut fixture,
            &mut projection,
            &mut memory,
            addresses,
            28,
            &[entity_arg, 1],
        )
        .unwrap();
        assert_eq!(fixture.portals.connected(0, 1), Some(true));

        // qboolean is a native int: unrelated upper bits must not open it.
        invoke(
            &mut fixture,
            &mut projection,
            &mut memory,
            addresses,
            28,
            &[entity_arg, 1 << 32],
        )
        .unwrap();
        assert_eq!(fixture.portals.connected(0, 1), Some(false));
        invoke(
            &mut fixture,
            &mut projection,
            &mut memory,
            addresses,
            28,
            &[entity_arg, (1 << 32) | 1],
        )
        .unwrap();
        assert_eq!(fixture.portals.connected(0, 1), Some(true));

        // SV_UnlinkEntity keeps the private svEntity_t area pair available.
        invoke(
            &mut fixture,
            &mut projection,
            &mut memory,
            addresses,
            31,
            &[entity_arg],
        )
        .unwrap();
        invoke(
            &mut fixture,
            &mut projection,
            &mut memory,
            addresses,
            28,
            &[entity_arg, 0],
        )
        .unwrap();
        assert_eq!(fixture.portals.connected(0, 1), Some(false));

        // A reset linkcount starts a new native lifetime. Its fresh link clears
        // the old pair; touching only area zero cannot adjust that portal again.
        memory.write_word(entity + 420, 0).unwrap();
        memory
            .write_vec3(entity + 488, Vec3([8.0, 0.0, 0.0]))
            .unwrap();
        invoke(
            &mut fixture,
            &mut projection,
            &mut memory,
            addresses,
            30,
            &[entity_arg],
        )
        .unwrap();
        invoke(
            &mut fixture,
            &mut projection,
            &mut memory,
            addresses,
            28,
            &[entity_arg, 1],
        )
        .unwrap();
        assert_eq!(fixture.portals.connected(0, 1), Some(false));
    }
}

fn assert_q3_link_alias_reads(fixture: &mut Fixture) {
    use qa_compat::quakec::{EntityAccess, Layout, Step, Trap, Vm};
    use qa_core::primitives::{EntityFields, NativeEntity};
    use qa_formats::program::quakec::{Image, Opcode};
    let entity = fixture
        .server
        .entities
        .find_native(
            NativeEntity {
                module: MODULE,
                slot: 0,
            },
            None,
        )
        .unwrap();
    let columns = &fixture.server.entities.columns;
    let authority = columns.published_fields[entity.slot as usize];
    assert!(authority.contains(EntityFields::POSITION));
    assert!(authority.contains(EntityFields::BOUNDS));
    assert!(authority.contains(EntityFields::ANGLES));
    assert!(!authority.contains(EntityFields::VELOCITY));
    assert_eq!(columns.classname[entity.slot as usize], None);
    let body = columns.body(entity.slot as usize);
    let angles = columns.angles[entity.slot as usize];
    let bounds = fixture.server.area.bounds(entity).unwrap();
    let links = fixture.server.area.relinks;
    let count = fixture.server.entities.active().count();
    let fields = [
        (3, 0, b"origin".as_slice()),
        (3, 3, b"velocity"),
        (3, 6, b"angles"),
        (3, 9, b"mins"),
        (3, 12, b"maxs"),
        (3, 15, b"absmin"),
        (3, 18, b"absmax"),
        (2, 21, b"solid"),
        (1, 22, b"classname"),
    ];
    let bytes = quakec_program::program_with_definitions(
        &[(Opcode::LoadV, 28, 29, 50), (Opcode::Return, 50, 0, 0)],
        &[0, 1],
        &[],
        &fields,
        23,
        64,
    );
    let mut vm = Vm::load(
        Image::parse(&bytes, Some(5927)).unwrap(),
        Layout {
            entities: 3,
            header_bytes: 16,
            extra_string_bytes: 32,
            state_step: 0.1,
            random_seed: 1,
        },
    )
    .unwrap();
    let mut recipient = context();
    recipient.module = ModuleId(1);
    recipient.native_rules = RuleSetId::Quake;
    recipient.clock = ThinkTime::Seconds(0.0);
    vm.seed_entities(
        &fixture.server.entities,
        recipient.module,
        recipient.native_rules,
        1,
    )
    .unwrap();
    let admitted = vm
        .admit_entity(&fixture.server.entities, recipient, entity)
        .unwrap();
    vm.image.globals[28] = admitted.offset();
    for (offset, expected) in [
        (0, body.position),
        (6, angles),
        (9, body.mins),
        (12, body.maxs),
        (15, bounds.mins),
        (18, bounds.maxs),
    ] {
        vm.image.globals[29] = offset;
        let mut run = vm.begin_root(1, 100, false).unwrap();
        assert_eq!(
            vm.advance(&mut run, &EntityAccess::engine(&fixture.services())),
            Ok(Step::Returned(expected.0.map(f32::to_bits)))
        );
    }
    for field in [3, 21, 22] {
        vm.image.statements[1].opcode = if field == 3 {
            Opcode::LoadV
        } else {
            Opcode::LoadF
        };
        vm.image.globals[29] = field;
        vm.image.globals[50..53].copy_from_slice(&[11, 22, 33]);
        let mut run = vm.begin_root(1, 100, false).unwrap();
        assert_eq!(
            vm.advance(&mut run, &EntityAccess::engine(&fixture.services())),
            Err(Trap::EntityAccess)
        );
        assert_eq!(vm.image.globals[50..53], [11, 22, 33]);
    }
    vm.release_entity(admitted).unwrap();
    assert_eq!(fixture.server.entities.active().count(), count);
    assert_eq!(fixture.server.area.relinks, links);
    assert_eq!(fixture.server.area.bounds(entity), Some(bounds));
}
