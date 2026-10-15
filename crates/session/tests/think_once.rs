use qa_core::primitives::*;
use qa_session::dispatch::*;
use qa_world::entities::{AllocationPolicy, EntityTable};
struct World {
    entities: EntityTable,
    subject: NativeEntityRef,
    callback: Option<CallbackId>,
    prepares: u32,
    calls: Vec<u32>,
    fail: bool,
}
impl ThinkWorld for World {
    fn entities(&mut self) -> &mut EntityTable {
        &mut self.entities
    }
    fn prepare_native_think(
        &mut self,
        _: EntityId,
        subject: Option<NativeEntityRef>,
        _: ThinkTime,
    ) -> Result<Option<PreparedNativeThink>, CallError> {
        self.prepares += 1;
        if self.fail {
            return Err(CallError::Rejected);
        }
        Ok(Some(PreparedNativeThink {
            subject: subject.unwrap_or(self.subject),
            callback: self.callback,
        }))
    }
}
fn invoke(w: &mut World, _: ModuleId, entry: u32, call: CallbackCall) -> bool {
    let CallbackCall::Think {
        prepared_native: Some(subject),
        ..
    } = call
    else {
        panic!("native preparation was lost")
    };
    assert_eq!(subject, w.subject);
    w.calls.push(entry);
    entry != 99
}
fn fixture() -> (World, FunctionTable<World>, EntityId) {
    let mut w = World {
        entities: EntityTable::new(8, 1).unwrap(),
        subject: NativeEntityRef::entity(ResourceNamespace::Module(ModuleId(1)), 5),
        callback: Some(CallbackId(1)),
        prepares: 0,
        calls: Vec::new(),
        fail: false,
    };
    let id = w
        .entities
        .allocate(
            ThinkTime::Seconds(0.0),
            ModuleId(1),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    let mut t = FunctionTable::load([(
        ModuleId(1),
        RuleSetId::Quake,
        vec![
            FunctionBinding {
                entry: 11,
                call: invoke,
            },
            FunctionBinding {
                entry: 22,
                call: invoke,
            },
        ],
    )])
    .unwrap();
    assert!(t.bind_think(&mut w.entities, id, Some(CallbackId(0))));
    assert!(t.bind_missing(
        ModuleId(1),
        FunctionBinding {
            entry: 99,
            call: invoke
        }
    ));
    w.entities.columns.next_think[id.slot as usize] = Some(ThinkTime::Seconds(1.0));
    (w, t, id)
}
#[test]
fn ordinary_loop_selects_function_after_the_single_native_preparation() {
    let (mut w, t, id) = fixture();
    let result = run_think(&mut w, &t, id, |_| {
        Some(ThinkFrame::Seconds {
            now: 1.0,
            step: 0.1,
        })
    });
    assert_eq!(result.called, 1);
    assert_eq!(result.rejected, 0);
    assert_eq!(w.prepares, 1);
    assert_eq!(w.calls, [22]);
    assert_eq!(w.entities.columns.next_think[id.slot as usize], None);
}
#[test]
fn explicit_weak_subject_does_not_clear_the_authorizing_anchor_deadline() {
    let (mut w, t, id) = fixture();
    let subject = w.subject;
    let result = call_think_once(
        &mut w,
        &t,
        id,
        Some(subject),
        ThinkTime::Seconds(77.0),
        |_| None,
    );
    assert_eq!(result.called, 1);
    assert_eq!(result.current_entity, None);
    assert_eq!(w.prepares, 1);
    assert_eq!(w.calls, [22]);
    assert_eq!(
        w.entities.columns.next_think[id.slot as usize],
        Some(ThinkTime::Seconds(1.0))
    );
}
#[test]
fn cleared_native_function_uses_the_same_missing_adapter_without_a_second_prepare() {
    let (mut w, t, id) = fixture();
    w.callback = None;
    let result = run_think(&mut w, &t, id, |_| {
        Some(ThinkFrame::Seconds {
            now: 1.0,
            step: 0.1,
        })
    });
    assert_eq!(result.rejected, 1);
    assert_eq!(result.called, 0);
    assert_eq!(w.prepares, 1);
    assert_eq!(w.calls, [99]);
    assert_eq!(w.entities.columns.next_think[id.slot as usize], None);
}
#[test]
fn preparation_failure_clears_only_the_admitted_common_deadline() {
    let (mut w, t, id) = fixture();
    w.fail = true;
    let result = run_think(&mut w, &t, id, |_| {
        Some(ThinkFrame::Seconds {
            now: 1.0,
            step: 0.1,
        })
    });
    assert_eq!(result.rejected, 1);
    assert_eq!(w.prepares, 1);
    assert!(w.calls.is_empty());
    assert_eq!(w.entities.columns.next_think[id.slot as usize], None);
}
