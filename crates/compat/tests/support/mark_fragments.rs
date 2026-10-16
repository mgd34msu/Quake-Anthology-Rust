//! Checked ABI spans and write ordering; shared render tests own geometric oracles.
use super::*;
use qa_compat::renderer::{EntitySubmission, SceneService, SceneView};
use qa_core::primitives::{IndexSpan, Vec3};
use qa_render::{Vertex, Viewport, scene::Light, world::WorldId};
use std::cell::Cell;

struct BoundaryScene<'a> {
    calls: &'a Cell<usize>,
}
impl SceneService for BoundaryScene<'_> {
    fn active(&self) -> bool {
        false
    }
    fn viewport(&self) -> Option<Viewport> {
        None
    }
    fn select_world(&mut self, _: WorldId) -> Result<(), CallError> {
        Ok(())
    }
    fn selected_world(&self) -> Option<WorldId> {
        Some(WorldId(1))
    }
    fn mark_fragments(
        &mut self,
        _: &qa_render::marks::MarkProjection,
        _: usize,
        _: usize,
        emit: &mut dyn FnMut(IndexSpan, &[Vec3]) -> Result<(), CallError>,
    ) -> Result<usize, CallError> {
        self.calls.set(self.calls.get() + 1);
        emit(
            IndexSpan { first: 0, count: 3 },
            &[Vec3([1., 2., 3.]), Vec3([4., 5., 6.]), Vec3([7., 8., 9.])],
        )?;
        Ok(1)
    }
    fn clear(&mut self) {}
    fn light(&mut self, _: Light) -> bool {
        false
    }
    fn entity(&mut self, _: EntitySubmission, _: &qa_console::cvars::Cvars) -> bool {
        false
    }
    fn poly(&mut self, _: i32, _: usize, _: &mut dyn FnMut(usize) -> Vertex) -> bool {
        false
    }
    fn color(&mut self, _: Option<[f32; 4]>) {}
    fn picture(&mut self, _: i32, _: [f32; 4], _: [f32; 4]) -> bool {
        false
    }
    fn view(&mut self, _: SceneView, _: &qa_console::cvars::Cvars) -> Result<bool, CallError> {
        Ok(false)
    }
}

fn source(memory: &mut ModuleMemory<'_>, base: u64) {
    for (index, point) in [
        Vec3([-1., -1., 0.]),
        Vec3([1., -1., 0.]),
        Vec3([1., 1., 0.]),
        Vec3([-1., 1., 0.]),
    ]
    .into_iter()
    .enumerate()
    {
        memory.write_vec3(base + index as u64 * 12, point).unwrap();
    }
    memory
        .write_vec3(base + 1024, Vec3([0., 0., -20.]))
        .unwrap();
    memory.write(base + 2048, &[0xa5; 64]).unwrap();
    memory.write(base + 3072, &[0xa5; 32]).unwrap();
}

#[test]
fn mark_fragments_checks_full_buffers_and_preserves_native_record_layout() {
    for (addresses, base) in modes() {
        let mut fixture = Fixture::new();
        let mut memory = ModuleMemory::load(base, 8192, &[]).unwrap();
        source(&mut memory, base);
        // Raw QVM pointer 8192 wraps to offset zero, but is not NULL.
        let input = if matches!(addresses, Addresses::Qvm { .. }) {
            8192
        } else {
            base
        };
        let valid = [4, input, base + 1024, 4, base + 2048, 2, base + 3072];
        let invalid = [
            [4, 0, base + 1024, 4, base + 2048, 2, base + 3072],
            [4, base + 8180, base + 1024, 4, base + 2048, 2, base + 3072],
            [4, input, base + 8181, 4, base + 2048, 2, base + 3072],
            [4, input, base + 1024, 4, base + 8150, 2, base + 3072],
            [4, input, base + 1024, 4, base + 2048, 2, base + 8180],
        ];
        let empty = [
            [0, 0, 0, 4, 0, 2, 0],
            [u32::MAX as u64, 0, 0, 4, 0, 2, 0],
            [4, 0, 0, 0, 0, 2, 0],
            [4, 0, 0, u32::MAX as u64, 0, 2, 0],
            [4, 0, 0, 4, 0, 0, 0],
            [4, 0, 0, 4, 0, u32::MAX as u64, 0],
        ];
        let calls = Cell::new(0);
        let mut scene = BoundaryScene { calls: &calls };
        let mut engine = fixture.engine();
        engine.scene = Some(&mut scene);
        let mut unknown = UnknownCalls::load(1).unwrap();
        let mut call = Invocation {
            services: &mut engine,
            memory: &mut memory,
            native_cvars: None,
            native_resources: None,
            native_entities: None,
            native_surfaces: None,
            native_command: None,
            native_configs: None,
            native_pmove: None,
            native_player: None,
            context: Fixture::context(SeatId::FIRST),
            platform_time: EventTime(42),
            command: Default::default(),
            argument_selection: &mut qa_compat::abi::ArgumentSelection::Caller,
            addresses,
            arguments: &valid,
        };
        for arguments in &invalid {
            call.arguments = arguments;
            assert!(matches!(
                Q3_CLIENT.invoke(27, &mut call, &mut unknown),
                Err(CallError::Memory)
            ));
            assert_eq!(call.memory.read(base + 2048, 64).unwrap(), &[0xa5; 64]);
            assert_eq!(call.memory.read(base + 3072, 32).unwrap(), &[0xa5; 32]);
            assert_eq!(calls.get(), 0);
        }
        for arguments in &empty {
            call.arguments = arguments;
            assert!(matches!(
                Q3_CLIENT.invoke(27, &mut call, &mut unknown).unwrap(),
                ImportStep::Return(0)
            ));
        }
        assert_eq!(calls.get(), 0);
        call.arguments = &valid;
        assert!(matches!(
            Q3_CLIENT.invoke(27, &mut call, &mut unknown).unwrap(),
            ImportStep::Return(1)
        ));
        assert_eq!(calls.get(), 1);
        assert_eq!(call.memory.read_word(base + 3072).unwrap(), 0);
        assert_eq!(call.memory.read_word(base + 3076).unwrap(), 3);
        assert_eq!(
            call.memory.read_vec3(base + 2048).unwrap(),
            Vec3([1., 2., 3.])
        );
        assert_eq!(
            call.memory.read_vec3(base + 2072).unwrap(),
            Vec3([7., 8., 9.])
        );
        assert_eq!(call.memory.read(base + 2084, 28).unwrap(), &[0xa5; 28]);
        assert_eq!(call.memory.read(base + 3080, 24).unwrap(), &[0xa5; 24]);
        assert_eq!(unknown.calls, 0);
    }
}

#[test]
fn mark_fragment_input_is_prepared_before_any_output_and_headers_precede_points() {
    for (addresses, base) in modes() {
        let mut fixture = Fixture::new();
        let mut memory = ModuleMemory::load(base, 8192, &[]).unwrap();
        source(&mut memory, base);
        let input = if matches!(addresses, Addresses::Qvm { .. }) {
            8192
        } else {
            base
        };
        let overlap = [4, input, base + 1024, 4, base + 2048, 2, base + 2048];
        let bad_tail = [65, input, base + 1024, 4, base + 2048, 2, base + 3072];
        memory
            .write_vec3(base + 64 * 12, Vec3([f32::NAN, 0., 0.]))
            .unwrap();
        let calls = Cell::new(0);
        let mut scene = BoundaryScene { calls: &calls };
        let mut engine = fixture.engine();
        engine.scene = Some(&mut scene);
        let mut unknown = UnknownCalls::load(1).unwrap();
        let mut call = Invocation {
            services: &mut engine,
            memory: &mut memory,
            native_cvars: None,
            native_resources: None,
            native_entities: None,
            native_surfaces: None,
            native_command: None,
            native_configs: None,
            native_pmove: None,
            native_player: None,
            context: Fixture::context(SeatId::FIRST),
            platform_time: EventTime(42),
            command: Default::default(),
            argument_selection: &mut qa_compat::abi::ArgumentSelection::Caller,
            addresses,
            arguments: &bad_tail,
        };
        assert!(matches!(
            Q3_CLIENT.invoke(27, &mut call, &mut unknown),
            Err(CallError::Geometry)
        ));
        assert_eq!(
            calls.get(),
            0,
            "point65 must be read before the64-point plane clamp"
        );
        assert_eq!(call.memory.read(base + 2048, 64).unwrap(), &[0xa5; 64]);
        call.arguments = &overlap;
        assert!(matches!(
            Q3_CLIENT.invoke(27, &mut call, &mut unknown).unwrap(),
            ImportStep::Return(1)
        ));
        assert_eq!(
            call.memory.read_word(base + 2048).unwrap() as u32,
            1f32.to_bits()
        );
        assert_eq!(
            call.memory.read_word(base + 2052).unwrap() as u32,
            2f32.to_bits()
        );
        assert_eq!(calls.get(), 1);
        call.services.scene = None;
        assert!(matches!(
            Q3_CLIENT.invoke(27, &mut call, &mut unknown).unwrap(),
            ImportStep::Return(0)
        ));
        assert_eq!(calls.get(), 1);
    }
}
