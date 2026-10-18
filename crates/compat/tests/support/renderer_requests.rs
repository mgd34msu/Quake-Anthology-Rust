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
    fn loaded_world(
        &self,
        _: &[u8],
        _search: Option<&qa_content::vfs::SearchView>,
    ) -> Option<qa_compat::services::LoadedWorldBinding> {
        None
    }
    fn bounds(
        &self,
        _: &[u8],
        _: RuleSetId,
        _search: Option<&qa_content::vfs::SearchView>,
    ) -> Option<Bounds> {
        Some(BOUNDS)
    }
    fn shader_handle(
        &self,
        name: &[u8],
        source: RuleSetId,
        _search: Option<&qa_content::vfs::SearchView>,
    ) -> Option<i32> {
        if name == b"failed" {
            return match source {
                RuleSetId::Quake2 => Some(93),
                RuleSetId::Quake3 => Some(0),
                _ => None,
            };
        }
        (name != b"unknown").then_some(93)
    }
    fn skin_handle(
        &self,
        name: &[u8],
        _: RuleSetId,
        _search: Option<&qa_content::vfs::SearchView>,
    ) -> Option<i32> {
        (name != b"unknown").then_some(92)
    }
    fn model_handle(
        &self,
        name: &[u8],
        _: RuleSetId,
        _search: Option<&qa_content::vfs::SearchView>,
    ) -> Option<i32> {
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
    submitted: &'a Cell<Option<i32>>,
    world: Option<WorldId>,
}
impl SceneService for Scene<'_> {
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
    fn entity(&mut self, entity: EntitySubmission, _: &qa_console::cvars::Cvars) -> bool {
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
    fn view(&mut self, _: SceneView, _: &qa_console::cvars::Cvars) -> Result<bool, CallError> {
        Ok(true)
    }
}
pub(super) fn with_call<T>(
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
pub(super) fn step(
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
pub(super) fn returned(step: ImportStep) -> u64 {
    match step {
        ImportStep::Return(word) => word,
        ImportStep::Host { .. } => panic!("unexpected host operation"),
    }
}

#[test]
fn deferred_renderer_handle_owns_name_and_resumes_before_bounds_and_scene_use() {
    for addresses in modes() {
        let mut fixture = Fixture::new();
        let selected = fixture.vfs.search_view([]).unwrap();
        fixture
            .storage
            .set_file_search(ModuleId(1), Some(selected.clone()), &fixture.vfs)
            .unwrap();
        let submitted = Cell::new(None);
        let mut scene = Scene {
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
        assert!(request.search.as_ref().unwrap().same_selection(&selected));
        engine
            .storage
            .set_file_search(ModuleId(1), None, engine.vfs)
            .unwrap();
        assert!(request.search.as_ref().unwrap().same_selection(&selected));
        assert!(
            !engine
                .storage
                .file_search(ModuleId(1))
                .unwrap()
                .same_selection(&selected)
        );
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
            panic!("cached registrations must enter logical admission")
        };
        assert_eq!(request.name.as_bytes(), b"changed");
        assert_eq!(
            returned(
                with_call(&mut engine, &mut memory, addresses, &[], |call| {
                    continuation.resume(call, 91)
                })
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
        let submitted = Cell::new(None);
        let mut scene = Scene {
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
                RendererRegistration::Shader {
                    mipmaps: true,
                    lighting: qa_compat::services::ShaderLighting::TwoDimensional,
                },
            ),
            (
                &Q3_CLIENT,
                57,
                RendererRegistration::Shader {
                    mipmaps: false,
                    lighting: qa_compat::services::ShaderLighting::TwoDimensional,
                },
            ),
            (&Q3_UI, 18, RendererRegistration::Model),
            (&Q3_UI, 19, RendererRegistration::Skin),
            (
                &Q3_UI,
                20,
                RendererRegistration::Shader {
                    mipmaps: false,
                    lighting: qa_compat::services::ShaderLighting::TwoDimensional,
                },
            ),
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
                    RendererRegistration::Shader {
                        mipmaps: a,
                        lighting: qa_compat::services::ShaderLighting::TwoDimensional,
                    },
                    RendererRegistration::Shader {
                        mipmaps: b,
                        lighting: qa_compat::services::ShaderLighting::TwoDimensional,
                    },
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
fn cached_headless_failed_and_initial_loader_requests_share_the_parked_operation() {
    for addresses in modes() {
        let mut fixture = Fixture::new();
        let catalog = Catalog;
        let mut loader = ShaderLoader::default();
        let base = base(addresses);
        let mut bytes = [0; 256];
        let mut memory = ModuleMemory::borrow(base, &mut bytes).unwrap();
        for initial in [false, true] {
            {
                let mut engine = fixture.engine(false);
                engine.model_lookup = Some(&catalog);
                if initial {
                    engine.model_loader = Some(&mut loader);
                }
                for name in [b"known".as_slice(), b"unknown", b"failed"] {
                    memory.write(base + 16, name).unwrap();
                    memory.write(base + 16 + name.len() as u64, &[0]).unwrap();
                    for number in [37, 38, 39] {
                        let ImportStep::Host {
                            request: HostRequest::RegisterRenderer(request),
                            continuation,
                        } = step(
                            &mut engine,
                            &mut memory,
                            addresses,
                            &Q3_CLIENT,
                            number,
                            &[base + 16],
                        )
                        .unwrap()
                        else {
                            panic!("all resource states require logical admission")
                        };
                        assert_eq!(request.name.as_bytes(), name);
                        assert!(matches!(continuation, ImportContinuation::HostValue));
                        assert_eq!(
                            returned(
                                with_call(&mut engine, &mut memory, addresses, &[], |call| {
                                    continuation.resume(call, 0)
                                })
                                .unwrap()
                            ),
                            0
                        );
                    }
                }
                engine.model_loader = None;
            }
        }
        assert!(
            loader.calls.is_empty(),
            "the parked ABI must not call the initial loader directly"
        );
    }
}

#[test]
fn load_world_keeps_a_long_cached_name_out_of_the_host_request() {
    for addresses in modes() {
        let mut fixture = Fixture::new();
        let selected = fixture.vfs.search_view([]).unwrap();
        fixture
            .storage
            .set_file_search(ModuleId(1), Some(selected.clone()), &fixture.vfs)
            .unwrap();
        struct WorldCatalog {
            search: qa_content::vfs::SearchView,
        }
        impl ModelLookup for WorldCatalog {
            fn loaded_world(
                &self,
                name: &[u8],
                search: Option<&qa_content::vfs::SearchView>,
            ) -> Option<qa_compat::services::LoadedWorldBinding> {
                (name.len() == 96
                    && search.is_some_and(|search| search.same_selection(&self.search)))
                .then_some(qa_compat::services::LoadedWorldBinding {
                    render: WorldId(9),
                    collision: None,
                })
            }
            fn bounds(
                &self,
                _: &[u8],
                _: RuleSetId,
                _search: Option<&qa_content::vfs::SearchView>,
            ) -> Option<Bounds> {
                None
            }
            fn shader_handle(
                &self,
                _: &[u8],
                _: RuleSetId,
                _search: Option<&qa_content::vfs::SearchView>,
            ) -> Option<i32> {
                None
            }
            fn model_handle(
                &self,
                _: &[u8],
                _: RuleSetId,
                _search: Option<&qa_content::vfs::SearchView>,
            ) -> Option<i32> {
                None
            }
            fn model_bounds(&self, _: i32) -> Option<Bounds> {
                None
            }
            fn model_has_tags(&self, _: i32) -> bool {
                false
            }
            fn lerp_tag(&self, _: i32, _: i32, _: i32, _: f32, _: &[u8]) -> Option<Orientation> {
                None
            }
        }
        let catalog = WorldCatalog { search: selected };
        let base = base(addresses);
        let mut bytes = [0; 256];
        let mut memory = ModuleMemory::borrow(base, &mut bytes).unwrap();
        memory.write(base + 16, &[b'w'; 96]).unwrap();
        memory.write(base + 112, &[0]).unwrap();
        let mut engine = fixture.engine(false);
        engine.model_lookup = Some(&catalog);
        let ImportStep::Host {
            request: HostRequest::SelectRendererWorld(world),
            continuation,
        } = step(
            &mut engine,
            &mut memory,
            addresses,
            &Q3_CLIENT,
            36,
            &[base + 16],
        )
        .unwrap()
        else {
            panic!("world selection requires logical admission")
        };
        assert_eq!(world, WorldId(9));
        let foreign = qa_content::vfs::Vfs::default();
        let original_vfs = engine.vfs;
        engine.vfs = &foreign;
        assert!(matches!(
            step(
                &mut engine,
                &mut memory,
                addresses,
                &Q3_CLIENT,
                36,
                &[base + 16]
            ),
            Err(CallError::File)
        ));
        engine.vfs = original_vfs;
        assert!(matches!(continuation, ImportContinuation::Return(0)));
        memory.write(base + 16, b"changed\0").unwrap();
        assert_eq!(
            returned(
                with_call(&mut engine, &mut memory, addresses, &[], |call| {
                    continuation.resume(call, 0)
                })
                .unwrap()
            ),
            0
        );
    }
}

#[test]
fn remap_traps_park_raw_words_without_reading_unreached_strings_and_return_void() {
    for addresses in modes() {
        let mut fixture = Fixture::new();
        let base = base(addresses);
        // No name needs to be readable until the host operation reaches it.
        let mut bytes = [0xff; 64];
        let mut memory = ModuleMemory::borrow(base, &mut bytes).unwrap();
        let mut engine = fixture.engine(false);
        for (table, number) in [(&Q3_CLIENT, 79), (&Q3_UI, 80)] {
            for arguments in [
                [0, 0, 0],
                [base + 1, u64::MAX, u64::MAX - 1],
                [0x1_0000_0000, 0xffff_ffff_0000_0001, 0x8000_0000_0000_0000],
            ] {
                let ImportStep::Host {
                    request:
                        HostRequest::RemapShader {
                            original,
                            replacement,
                            time_offset,
                        },
                    continuation,
                } = step(
                    &mut engine,
                    &mut memory,
                    addresses,
                    table,
                    number,
                    &arguments,
                )
                .unwrap()
                else {
                    panic!("remap must use the shared parked host path")
                };
                assert_eq!([original, replacement, time_offset], arguments);
                assert!(matches!(continuation, ImportContinuation::Return(0)));
                assert_eq!(
                    returned(
                        with_call(&mut engine, &mut memory, addresses, &[], |call| {
                            continuation.resume(call, 73)
                        },)
                        .unwrap()
                    ),
                    0
                );
            }
            assert!(matches!(
                step(&mut engine, &mut memory, addresses, table, number, &[1, 2]),
                Err(CallError::Memory)
            ));
        }
    }
}
