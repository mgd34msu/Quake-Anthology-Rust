use super::*;
use qa_compat::{
    abi::{CallTable, HostRequest, ImportContinuation, ImportStep},
    renderer::{EntitySubmission, SceneService, SceneView},
    services::{ModelLookup, RendererRegistration},
};
use qa_core::primitives::{Bounds, Orientation};
use qa_render::{Vertex, Viewport, scene::Light, world::WorldId};
use std::cell::Cell;

const BOUNDS: Bounds = Bounds {
    mins: Vec3([-1., -2., -3.]),
    maxs: Vec3([4., 5., 6.]),
};
struct Catalog;
impl ModelLookup for Catalog {
    fn inline_handle(&self, world: WorldId, index: u32) -> Option<i32> {
        (world == WorldId(9) && index == 1).then_some(77)
    }
    fn loaded_world(&self, _: &[u8]) -> Option<qa_compat::services::LoadedWorldBinding> {
        None
    }
    fn bounds(&self, _: &[u8], _: RuleSetId) -> Option<Bounds> {
        Some(BOUNDS)
    }
    fn shader_handle(&self, name: &[u8], source: RuleSetId) -> Option<i32> {
        if name == b"failed" {
            return match source {
                RuleSetId::Quake2 => Some(93),
                RuleSetId::Quake3 => Some(0),
                _ => None,
            };
        }
        (name != b"unknown").then_some(93)
    }
    fn skin_handle(&self, name: &[u8], _: RuleSetId) -> Option<i32> {
        (name != b"unknown").then_some(92)
    }
    fn model_handle(&self, name: &[u8], _: RuleSetId) -> Option<i32> {
        (name != b"unknown").then_some(91)
    }
    fn model_bounds(&self, handle: i32) -> Option<Bounds> {
        (handle == 91).then_some(BOUNDS)
    }
    fn model_has_tags(&self, _: i32) -> bool {
        false
    }
    fn lerp_tag(&self, _: i32, _: i32, _: i32, _: f32, _: &[u8]) -> Option<Orientation> {
        None
    }
}
struct Scene<'a> {
    available: &'a Cell<bool>,
    ready: &'a Cell<Option<i32>>,
    submitted: &'a Cell<Option<i32>>,
    world: Option<WorldId>,
}
impl SceneService for Scene<'_> {
    fn registration_available(&self) -> bool {
        self.available.get()
    }
    fn registration_cached(
        &self,
        _: RendererRegistration,
        _: &[u8],
        source: RuleSetId,
    ) -> Option<i32> {
        (source == RuleSetId::Quake3)
            .then(|| self.ready.get())
            .flatten()
    }
    fn active(&self) -> bool {
        true
    }
    fn viewport(&self) -> Option<Viewport> {
        Some(Viewport {
            width: 320,
            height: 200,
            ..Default::default()
        })
    }
    fn select_world(&mut self, world: WorldId) -> Result<(), CallError> {
        self.world = Some(world);
        Ok(())
    }
    fn selected_world(&self) -> Option<WorldId> {
        self.world
    }
    fn clear(&mut self) {}
    fn light(&mut self, _: Light) -> bool {
        true
    }
    fn entity(&mut self, entity: EntitySubmission) -> bool {
        self.submitted.set(Some(entity.model));
        true
    }
    fn poly(&mut self, _: i32, _: usize, _: &mut dyn FnMut(usize) -> Vertex) -> bool {
        true
    }
    fn color(&mut self, _: Option<[f32; 4]>) {}
    fn picture(&mut self, _: i32, _: [f32; 4], _: [f32; 4]) -> bool {
        true
    }
    fn view(&mut self, _: SceneView) -> Result<bool, CallError> {
        Ok(true)
    }
}
fn with_call<T>(
    services: &mut EngineServices<'_>,
    memory: &mut ModuleMemory<'_>,
    addresses: Addresses,
    arguments: &[u64],
    run: impl FnOnce(&mut Invocation<'_, '_, '_>) -> T,
) -> T {
    let mut call = Invocation {
        services,
        memory,
        native_cvars: None,
        native_resources: None,
        native_entities: None,
        native_surfaces: None,
        native_command: None,
        native_configs: None,
        native_pmove: None,
        native_player: None,
        context: context(),
        platform_time: EventTime(123_456_789),
        command: Default::default(),
        argument_selection: &mut qa_compat::abi::ArgumentSelection::Caller,
        addresses,
        arguments,
    };
    run(&mut call)
}
fn step(
    services: &mut EngineServices<'_>,
    memory: &mut ModuleMemory<'_>,
    addresses: Addresses,
    table: &CallTable,
    number: u32,
    arguments: &[u64],
) -> Result<ImportStep, CallError> {
    let mut unknown = UnknownCalls::load(1).unwrap();
    with_call(services, memory, addresses, arguments, |call| {
        table.invoke(number, call, &mut unknown)
    })
}
fn returned(step: ImportStep) -> u64 {
    match step {
        ImportStep::Return(word) => word,
        ImportStep::Host { .. } => panic!("unexpected host operation"),
    }
}

#[test]
fn deferred_renderer_handle_owns_name_and_resumes_before_bounds_and_scene_use() {
    for addresses in modes() {
        let mut fixture = Fixture::new();
        let (available, ready, submitted) = (Cell::new(true), Cell::new(None), Cell::new(None));
        let mut scene = Scene {
            available: &available,
            ready: &ready,
            submitted: &submitted,
            world: Some(WorldId(9)),
        };
        let catalog = Catalog;
        let base = base(addresses);
        let mut bytes = [0; 512];
        let mut memory = ModuleMemory::borrow(base, &mut bytes).unwrap();
        let name = b"models/\xff.md3";
        memory.write(base + 16, name).unwrap();
        memory.write(base + 16 + name.len() as u64, &[0]).unwrap();
        let mut engine = fixture.engine(false);
        engine.model_lookup = Some(&catalog);
        engine.scene = Some(&mut scene);
        let ImportStep::Host {
            request: HostRequest::RegisterRenderer(request),
            continuation,
        } = step(
            &mut engine,
            &mut memory,
            addresses,
            &Q3_CLIENT,
            37,
            &[base + 16],
        )
        .unwrap()
        else {
            panic!("unadmitted cached handle must wait for publication")
        };
        assert!(matches!(continuation, ImportContinuation::HostValue));
        assert_eq!(request.name.as_bytes(), name);
        assert_eq!(request.source, RuleSetId::Quake3);
        assert!(matches!(request.kind, RendererRegistration::Model));
        assert_eq!(request.world, None);
        memory.write(base + 16, b"changed\0").unwrap();
        assert_eq!(request.name.as_bytes(), name);
        let resumed = with_call(&mut engine, &mut memory, addresses, &[], |call| {
            continuation.resume(call, 91)
        })
        .unwrap();
        assert_eq!(returned(resumed), 91);
        assert_eq!(
            returned(
                step(
                    &mut engine,
                    &mut memory,
                    addresses,
                    &Q3_CLIENT,
                    47,
                    &[91, base + 128, base + 144]
                )
                .unwrap()
            ),
            0
        );
        assert_eq!(memory.read_vec3(base + 128).unwrap(), BOUNDS.mins);
        assert_eq!(memory.read_vec3(base + 144).unwrap(), BOUNDS.maxs);
        memory.write_word(base + 256 + 8, 91).unwrap();
        assert_eq!(
            returned(
                step(
                    &mut engine,
                    &mut memory,
                    addresses,
                    &Q3_CLIENT,
                    41,
                    &[base + 256]
                )
                .unwrap()
            ),
            0
        );
        assert_eq!(submitted.get(), Some(91));
        ready.set(Some(91));
        assert_eq!(
            returned(
                step(
                    &mut engine,
                    &mut memory,
                    addresses,
                    &Q3_CLIENT,
                    37,
                    &[base + 16]
                )
                .unwrap()
            ),
            91
        );
        let resumed = with_call(&mut engine, &mut memory, addresses, &[], |call| {
            ImportContinuation::Return(7).resume(call, 91)
        })
        .unwrap();
        assert_eq!(returned(resumed), 7);
    }
}

#[test]
fn renderer_requests_keep_kind_inline_world_and_native_name_width() {
    for addresses in modes() {
        let mut fixture = Fixture::new();
        let (available, ready, submitted) = (Cell::new(true), Cell::new(None), Cell::new(None));
        let mut scene = Scene {
            available: &available,
            ready: &ready,
            submitted: &submitted,
            world: Some(WorldId(9)),
        };
        let catalog = Catalog;
        let base = base(addresses);
        let mut bytes = [0; 512];
        let mut memory = ModuleMemory::borrow(base, &mut bytes).unwrap();
        memory.write(base + 16, b"ui/new\0").unwrap();
        let mut engine = fixture.engine(false);
        engine.model_lookup = Some(&catalog);
        engine.scene = Some(&mut scene);
        for (table, number, kind) in [
            (&Q3_CLIENT, 37, RendererRegistration::Model),
            (&Q3_CLIENT, 38, RendererRegistration::Skin),
            (
                &Q3_CLIENT,
                39,
                RendererRegistration::Shader { mipmaps: true },
            ),
            (
                &Q3_CLIENT,
                57,
                RendererRegistration::Shader { mipmaps: false },
            ),
            (&Q3_UI, 18, RendererRegistration::Model),
            (&Q3_UI, 19, RendererRegistration::Skin),
            (&Q3_UI, 20, RendererRegistration::Shader { mipmaps: false }),
        ] {
            let ImportStep::Host {
                request: HostRequest::RegisterRenderer(request),
                ..
            } = step(
                &mut engine,
                &mut memory,
                addresses,
                table,
                number,
                &[base + 16],
            )
            .unwrap()
            else {
                panic!("renderer entry must defer")
            };
            match (request.kind, kind) {
                (RendererRegistration::Model, RendererRegistration::Model)
                | (RendererRegistration::Skin, RendererRegistration::Skin) => {}
                (
                    RendererRegistration::Shader { mipmaps: a },
                    RendererRegistration::Shader { mipmaps: b },
                ) => assert_eq!(a, b),
                _ => panic!("different registration kind"),
            }
            assert_eq!(request.world, None);
        }
        memory.write(base + 16, b"*1\0").unwrap();
        let ImportStep::Host {
            request: HostRequest::RegisterRenderer(request),
            ..
        } = step(
            &mut engine,
            &mut memory,
            addresses,
            &Q3_CLIENT,
            37,
            &[base + 16],
        )
        .unwrap()
        else {
            panic!("inline binding must wait for selected presentation")
        };
        engine
            .scene
            .as_deref_mut()
            .unwrap()
            .select_world(WorldId(10))
            .unwrap();
        assert_eq!(request.world, Some(WorldId(9)));
        assert_eq!(request.name.as_bytes(), b"*1");
        memory.write(base + 16, &[b'x'; 63]).unwrap();
        memory.write(base + 79, &[0]).unwrap();
        assert!(matches!(
            step(
                &mut engine,
                &mut memory,
                addresses,
                &Q3_CLIENT,
                39,
                &[base + 16]
            )
            .unwrap(),
            ImportStep::Host { .. }
        ));
        memory.write(base + 16, &[b'x'; 64]).unwrap();
        memory.write(base + 80, &[0]).unwrap();
        assert_eq!(
            returned(
                step(
                    &mut engine,
                    &mut memory,
                    addresses,
                    &Q3_CLIENT,
                    39,
                    &[base + 16]
                )
                .unwrap()
            ),
            0
        );
        assert_eq!(
            returned(step(&mut engine, &mut memory, addresses, &Q3_CLIENT, 37, &[0]).unwrap()),
            0
        );
        assert!(matches!(
            step(&mut engine, &mut memory, addresses, &Q3_CLIENT, 39, &[0]),
            Err(CallError::Memory)
        ));
    }
}

#[test]
fn unavailable_registration_stays_cache_only_and_cold_loader_does_not_defer() {
    for addresses in modes() {
        let mut fixture = Fixture::new();
        let (available, ready, submitted) = (Cell::new(false), Cell::new(None), Cell::new(None));
        let mut scene = Scene {
            available: &available,
            ready: &ready,
            submitted: &submitted,
            world: None,
        };
        let catalog = Catalog;
        let base = base(addresses);
        let mut bytes = [0; 512];
        let mut memory = ModuleMemory::borrow(base, &mut bytes).unwrap();
        memory.write(base + 16, b"known\0").unwrap();
        memory.write(base + 48, b"unknown\0").unwrap();
        let mut shader = ShaderLoader::default();
        {
            let mut engine = fixture.engine(false);
            engine.model_lookup = Some(&catalog);
            engine.scene = Some(&mut scene);
            for kind in [
                RendererRegistration::Model,
                RendererRegistration::Skin,
                RendererRegistration::Shader { mipmaps: true },
            ] {
                assert_eq!(
                    engine.register_renderer(b"known", RuleSetId::Quake3, kind),
                    Ok(0)
                );
            }
            assert_eq!(
                returned(
                    step(
                        &mut engine,
                        &mut memory,
                        addresses,
                        &Q3_CLIENT,
                        37,
                        &[base + 16]
                    )
                    .unwrap()
                ),
                0
            );
            ready.set(Some(91));
            assert_eq!(
                returned(
                    step(
                        &mut engine,
                        &mut memory,
                        addresses,
                        &Q3_CLIENT,
                        37,
                        &[base + 16]
                    )
                    .unwrap()
                ),
                91
            );
            assert_eq!(
                engine.register_renderer(
                    b"known",
                    RuleSetId::Quake2,
                    RendererRegistration::Model
                ),
                Ok(0)
            );
            ready.set(None);
            assert_eq!(
                returned(
                    step(
                        &mut engine,
                        &mut memory,
                        addresses,
                        &Q3_CLIENT,
                        37,
                        &[base + 48]
                    )
                    .unwrap()
                ),
                0
            );
            available.set(true);
            engine.model_loader = Some(&mut shader);
            memory.write(base + 16, b"ui/pic\0").unwrap();
            assert_eq!(
                returned(
                    step(
                        &mut engine,
                        &mut memory,
                        addresses,
                        &Q3_CLIENT,
                        39,
                        &[base + 16]
                    )
                    .unwrap()
                ),
                42
            );
        }
        assert_eq!(
            shader.calls,
            [(b"ui/pic".to_vec(), RuleSetId::Quake3, true)]
        );
    }
}

#[test]
fn failed_renderer_cache_returns_zero_before_host_or_headless_aggregate_lookup() {
    for addresses in modes() {
        let mut fixture = Fixture::new();
        let (available, ready, submitted) = (Cell::new(true), Cell::new(Some(0)), Cell::new(None));
        let mut scene = Scene {
            available: &available,
            ready: &ready,
            submitted: &submitted,
            world: None,
        };
        let catalog = Catalog;
        let base = base(addresses);
        let mut bytes = [0; 128];
        let mut memory = ModuleMemory::borrow(base, &mut bytes).unwrap();
        memory.write(base + 16, b"known\0").unwrap();
        let mut engine = fixture.engine(false);
        engine.model_lookup = Some(&catalog);
        engine.scene = Some(&mut scene);
        for enabled in [true, false] {
            available.set(enabled);
            for _ in 0..3 {
                assert_eq!(
                    returned(
                        step(
                            &mut engine,
                            &mut memory,
                            addresses,
                            &Q3_CLIENT,
                            39,
                            &[base + 16],
                        )
                        .unwrap()
                    ),
                    0
                );
            }
        }
        available.set(true);
        ready.set(None);
        assert!(matches!(
            step(
                &mut engine,
                &mut memory,
                addresses,
                &Q3_CLIENT,
                39,
                &[base + 16],
            )
            .unwrap(),
            ImportStep::Host { .. }
        ));
        engine.scene = None;
        memory.write(base + 16, b"failed\0").unwrap();
        assert_eq!(
            returned(
                step(
                    &mut engine,
                    &mut memory,
                    addresses,
                    &Q3_CLIENT,
                    39,
                    &[base + 16],
                )
                .unwrap()
            ),
            0
        );
        assert_eq!(
            engine
                .register_renderer(
                    b"failed",
                    RuleSetId::Quake2,
                    RendererRegistration::Shader { mipmaps: false }
                )
                .unwrap(),
            93
        );
        assert_eq!(catalog.shader_handle(b"failed", RuleSetId::Quake), None);
    }
}
