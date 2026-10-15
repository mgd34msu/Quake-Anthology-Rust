use super::*;
use crate::{
    WorldCollision,
    modules::{CallbackId, Export, ModuleResult, contacts::tests::fixture},
};
use qa_core::primitives::{Bounds, IndexSpan};
use qa_world::collision::{
    brushes::{BrushTree, CollisionLeaf, ModelRoot},
    surfaces::SurfaceTable,
};

#[test]
fn push_entity_foreign_impact_retains_mover_identity_across_removing_relink_trigger() {
    let (mut host, [actor, obstacle, _], stride) = fixture();
    let geometry = host
        .runtime
        .geometry
        .load_brushes(
            vec![],
            vec![],
            Vec::new(),
            SurfaceTable::flags(vec![]),
            BrushTree {
                planes: vec![],
                nodes: vec![],
                leaves: vec![CollisionLeaf {
                    stored_contents: None,
                    brushes: IndexSpan::default(),
                    patches: Default::default(),
                }],
                leaf_brushes: vec![],
                models: vec![ModelRoot::Leaf(0)],
                leaf_patches: Vec::new(),
            },
            vec![Bounds {
                mins: Vec3([-512.0; 3]),
                maxs: Vec3([512.0; 3]),
            }],
        )
        .unwrap();
    host.runtime.collision = Some(WorldCollision::new(&host.runtime.geometry, geometry, 0));
    // The shared contact fixture loads modules before it has a world. Complete
    // their load-sized trace scratch after installing this fixture's geometry.
    for module in [ModuleId(1), ModuleId(2), ModuleId(3)] {
        let scratch = host.runtime.geometry.scratch();
        host.module_row_mut(module).unwrap().scratch = scratch;
    }
    host.module_row_mut(ModuleId(1))
        .unwrap()
        .request
        .context
        .trace_rules = RuleSetId::Quake3;
    host.call_module_export(
        ModuleId(1),
        Export::clocked(CallbackId(12)),
        ThinkTime::Seconds(3.0),
    )
    .unwrap();
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().last_result,
        Some(ModuleResult::QuakeC([3 * stride, 0, 0]))
    );
    for (module, rows) in [
        (
            ModuleId(1),
            vec![
                (2, Vec3([0.0, 0.0, 8.0]), 2.0f32, 5u32),
                (3, Vec3([7.0, 0.0, 8.0]), 1.0, 10),
            ],
        ),
        (ModuleId(2), vec![(2, Vec3([10.0, 0.0, 8.0]), 2.0, 6)]),
    ] {
        edit_quakec(
            &mut host,
            module,
            crate::modules::execution::TraceAccess::empty(),
            |vm, services, context| {
                for (native, position, solid, touch) in rows {
                    let offset = native * vm.stride as u32;
                    for (field, value) in [
                        (b"origin".as_slice(), position),
                        (b"mins", Vec3([-1.0; 3])),
                        (b"maxs", Vec3([1.0; 3])),
                    ] {
                        let address = vm
                            .field_address(offset, vm.image.field(field).unwrap() as u32, 3)
                            .unwrap();
                        vm.entities.write_vec3(address, value).unwrap();
                    }
                    for (field, word) in [(b"solid".as_slice(), solid.to_bits()), (b"touch", touch)]
                    {
                        let address = vm
                            .field_address(offset, vm.image.field(field).unwrap() as u32, 1)
                            .unwrap();
                        vm.entities.write_word(address, word as i32).unwrap();
                    }
                    vm.relink_entity(services, context, offset)?;
                }
                Ok(())
            },
        )
        .unwrap();
    }
    let calls = host.module_counts(ModuleId(2)).unwrap().calls;
    let (trace, current) = {
        let mut moving =
            Host::new(&mut host, ModuleId(1), actor, ThinkTime::Seconds(42.0)).unwrap();
        physics::push_entity(&mut moving, actor, Vec3([20.0, 0.0, 0.0])).unwrap()
    };
    assert_eq!(trace.trace.entity, Some(obstacle));
    assert_eq!(current, None);
    assert!(host.runtime.server.entities.resolve(actor).is_none());
    assert_eq!(host.module_counts(ModuleId(2)).unwrap().calls, calls + 1);
    let Some(ModuleResult::QuakeC(words)) = host.module_counts(ModuleId(2)).unwrap().last_result
    else {
        unreachable!()
    };
    assert_eq!(&words[..2], &[2 * stride, 3 * stride]);
    let Program::QuakeC(qc) = &host.module_row(ModuleId(2)).unwrap().request.program else {
        unreachable!()
    };
    assert_eq!(
        qc.vm
            .resolve_entity(&host.runtime.server.entities, ModuleId(2), words[1])
            .unwrap(),
        None
    );
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
    assert_eq!(host.module_counts(ModuleId(2)).unwrap().traps, 0);
}
