use qa_compat::{
    abi::{Addresses, Invocation, Q3_SERVER, UnknownCalls},
    entities::EntityProjection,
    memory::ModuleMemory,
    services::{CallContext, CallError, EngineServices, ServiceStorage},
};
use qa_console::{command_buffer::CommandBuffer, cvars::Cvars, views::Context};
use qa_core::{
    names::NameTable,
    primitives::{Bounds, ModuleId, Plane, RuleSetId, ThinkTime, Vec3},
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
            PvsRows::load(vec![Some(0), Some(1)], vec![3, 3]).unwrap(),
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
            world,
            leaves,
            row,
            portals: AreaPortals::load(2, 0, vec![]).unwrap(),
        }
    }

    fn services(&mut self) -> EngineServices<'_> {
        EngineServices {
            server: &mut self.server,
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
            world: None,
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
            command: &[],
            addresses,
            arguments,
        },
        &mut unknown,
    );
    assert_eq!(unknown.calls, 0);
    result
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
