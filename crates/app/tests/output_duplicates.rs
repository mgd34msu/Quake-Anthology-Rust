use qa_app::output::duplicates::{Admission, Coordinator, PhysicalGroup};
use qa_core::{events::*, primitives::*};

fn delivery(frame: u64, key: u32) -> NativeDelivery {
    NativeDelivery {
        module: ModuleId(1),
        source: RuleSetId::Quake2Rerelease,
        target: NativeTarget::All,
        origin: Vec3::default(),
        reliable: true,
        dupe_key: key,
        component: 0,
        emission: Some(NativeEmission {
            module_lifetime: EntityId {
                slot: 8,
                generation: 3,
            },
            provider_frame: frame,
        }),
        check_portals: true,
    }
}
fn record(ring: &mut EventRing, owner: OutputConsumerId, frame: u64, key: u32) -> OutputRecord {
    let sequence = ring
        .native_message(delivery(frame, key), b"\x0frepeat\0")
        .unwrap();
    let mut batch = ring.batch(owner).unwrap();
    std::iter::from_fn(|| ring.next(&mut batch))
        .find(|r| r.sequence == sequence)
        .unwrap()
}
#[test]
fn emission_siblings_have_independent_claims_and_duplicate_seats_share_them() {
    let mut ring = EventRing::load(8, 2, 8, 32).unwrap();
    let a = ring.bind(OutputTarget::Client(ClientId(0))).unwrap();
    let b = ring.bind(OutputTarget::Client(ClientId(1))).unwrap();
    ring.publish_group(3, |group| {
        for component in [0, 8, 11] {
            group.native_message_parts(
                NativeDelivery {
                    component,
                    ..delivery(10, 43)
                },
                b"\x0fpart\0",
                &[],
            )?;
        }
        Ok(())
    })
    .unwrap();
    let mut gate = Coordinator::load(8, 2);
    let mut batch = ring.batch(a).unwrap();
    while let Some(record) = ring.next(&mut batch) {
        assert_eq!(
            gate.admit(
                record,
                ClientId(0),
                a,
                PhysicalGroup::Engine,
                ring.sequence_range()
            ),
            Admission::Send
        );
        assert_eq!(
            gate.admit(
                record,
                ClientId(1),
                b,
                PhysicalGroup::Engine,
                ring.sequence_range()
            ),
            Admission::Pending
        );
        let receipt = NativeReceipt(record.sequence + 10);
        gate.submitted(
            a,
            record.sequence,
            OutputSubmission::Reliable(receipt),
            false,
            false,
        );
        gate.acknowledged(a, NativeReceipt(receipt.0 + 100));
        assert_eq!(
            gate.admit(
                record,
                ClientId(1),
                b,
                PhysicalGroup::Engine,
                ring.sequence_range()
            ),
            Admission::Pending
        );
        gate.acknowledged(a, receipt);
        assert_eq!(
            gate.admit(
                record,
                ClientId(1),
                b,
                PhysicalGroup::Engine,
                ring.sequence_range()
            ),
            Admission::Suppressed
        );
    }
}
#[test]
fn native_ack_fragment_failure_and_resync_have_distinct_duplicate_outcomes() {
    let mut ring = EventRing::load(8, 2, 16, 128).unwrap();
    let a = ring.bind(OutputTarget::Client(ClientId(0))).unwrap();
    let b = ring.bind(OutputTarget::Client(ClientId(1))).unwrap();
    let mut gate = Coordinator::load(8, 2);
    let first = record(&mut ring, a, 10, 42);
    let group = PhysicalGroup::Engine;
    assert_eq!(
        gate.admit(first, ClientId(0), a, group, ring.sequence_range()),
        Admission::Send
    );
    let receipt = NativeReceipt(17);
    ring.submit(a, first.sequence, OutputSubmission::Reliable(receipt));
    gate.submitted(
        a,
        first.sequence,
        OutputSubmission::Reliable(receipt),
        false,
        false,
    );
    assert_eq!(
        gate.admit(first, ClientId(1), b, group, ring.sequence_range()),
        Admission::Pending
    );
    assert_eq!(
        ring.acknowledge_with(a, NativeReceipt(18), |_| gate
            .acknowledged(a, NativeReceipt(18))),
        0
    );
    assert_eq!(
        gate.admit(first, ClientId(1), b, group, ring.sequence_range()),
        Admission::Pending
    );
    assert_eq!(
        ring.acknowledge_with(a, receipt, |_| gate.acknowledged(a, receipt)),
        1
    );
    assert_eq!(
        gate.admit(first, ClientId(1), b, group, ring.sequence_range()),
        Admission::Suppressed
    );
    ring.submit(b, first.sequence, OutputSubmission::BestEffort);

    let next = record(&mut ring, a, 11, 43);
    gate.sweep(&ring, |_, _, _, _| false);
    assert_eq!(
        gate.admit(next, ClientId(0), a, group, ring.sequence_range()),
        Admission::Send
    );
    gate.submitted(a, next.sequence, OutputSubmission::Unsent, false, true);
    assert_eq!(
        gate.admit(next, ClientId(1), b, group, ring.sequence_range()),
        Admission::Pending
    );
    // Empty/unsupported/dropped payload is not a successfully sent duplicate.
    gate.submitted(a, next.sequence, OutputSubmission::BestEffort, false, false);
    assert_eq!(
        gate.admit(next, ClientId(1), b, group, ring.sequence_range()),
        Admission::Send
    );
    gate.submitted(b, next.sequence, OutputSubmission::Unsent, false, true);
    assert_eq!(
        gate.admit(next, ClientId(0), a, group, ring.sequence_range()),
        Admission::Pending
    );
    gate.submitted(b, next.sequence, OutputSubmission::BestEffort, true, false);
    assert_eq!(
        gate.admit(next, ClientId(0), a, group, ring.sequence_range()),
        Admission::Suppressed
    );

    let stalled = record(&mut ring, a, 12, 44);
    assert_eq!(
        gate.admit(stalled, ClientId(0), a, group, ring.sequence_range()),
        Admission::Send
    );
    gate.submitted(a, stalled.sequence, OutputSubmission::Unsent, false, true);
    ring.unbind(a);
    gate.sweep(&ring, |_, _, _, _| false);
    assert_eq!(
        gate.admit(stalled, ClientId(1), b, group, ring.sequence_range()),
        Admission::Send
    );
}
#[test]
fn frame_lifetime_physical_group_and_zero_key_are_independent() {
    let mut ring = EventRing::load(8, 2, 16, 128).unwrap();
    let a = ring.bind(OutputTarget::Client(ClientId(0))).unwrap();
    let b = ring.bind(OutputTarget::Client(ClientId(1))).unwrap();
    let mut gate = Coordinator::load(8, 2);
    let first = record(&mut ring, a, 10, 42);
    assert_eq!(
        gate.admit(
            first,
            ClientId(0),
            a,
            PhysicalGroup::Peer(ClientId(0)),
            ring.sequence_range()
        ),
        Admission::Send
    );
    gate.submitted(a, first.sequence, OutputSubmission::BestEffort, true, false);
    assert_eq!(
        gate.admit(
            first,
            ClientId(1),
            b,
            PhysicalGroup::Peer(ClientId(1)),
            ring.sequence_range()
        ),
        Admission::Send
    );
    gate.submitted(b, first.sequence, OutputSubmission::BestEffort, true, false);
    assert!(gate.negotiate(ClientId(0), Some(7)));
    assert!(gate.negotiate(ClientId(1), Some(7)));
    let negotiated = gate.group(ClientId(0), false);
    assert_eq!(negotiated, gate.group(ClientId(1), false));
    assert_eq!(gate.group(ClientId(0), true), PhysicalGroup::Engine);
    assert!(gate.negotiate(ClientId(1), None));
    assert_eq!(
        gate.group(ClientId(1), false),
        PhysicalGroup::Peer(ClientId(1))
    );
    let zero = record(&mut ring, a, 10, 0);
    assert_eq!(
        gate.admit(
            zero,
            ClientId(0),
            a,
            PhysicalGroup::Engine,
            ring.sequence_range()
        ),
        Admission::Send
    );
    gate.submitted(a, zero.sequence, OutputSubmission::BestEffort, true, false);
    assert_eq!(
        gate.admit(
            zero,
            ClientId(1),
            b,
            PhysicalGroup::Engine,
            ring.sequence_range()
        ),
        Admission::Send
    );
    let mut new_lifetime = first;
    new_lifetime
        .delivery
        .as_mut()
        .unwrap()
        .emission
        .as_mut()
        .unwrap()
        .module_lifetime
        .generation += 1;
    assert_eq!(
        gate.admit(
            new_lifetime,
            ClientId(0),
            a,
            PhysicalGroup::Peer(ClientId(0)),
            ring.sequence_range()
        ),
        Admission::Send
    );
}
#[test]
fn old_sent_frame_is_pruned_before_bounded_admission_after_retirement() {
    let mut ring = EventRing::load(8, 1, 16, 128).unwrap();
    let a = ring.bind(OutputTarget::Client(ClientId(0))).unwrap();
    let mut gate = Coordinator::load(1, 1);
    let first = record(&mut ring, a, 10, 42);
    assert_eq!(
        gate.admit(
            first,
            ClientId(0),
            a,
            PhysicalGroup::Engine,
            ring.sequence_range()
        ),
        Admission::Send
    );
    gate.submitted(a, first.sequence, OutputSubmission::BestEffort, true, false);
    ring.submit(a, first.sequence, OutputSubmission::BestEffort);
    gate.sweep(&ring, |_, _, _, _| false);
    assert_eq!(
        gate.admit(
            first,
            ClientId(0),
            a,
            PhysicalGroup::Engine,
            ring.sequence_range()
        ),
        Admission::Suppressed
    );
    let next = record(&mut ring, a, 11, 42);
    assert_eq!(
        gate.admit(
            next,
            ClientId(0),
            a,
            PhysicalGroup::Engine,
            ring.sequence_range()
        ),
        Admission::Send
    );
    let extra = record(&mut ring, a, 11, 43);
    assert_eq!(
        gate.admit(
            extra,
            ClientId(0),
            a,
            PhysicalGroup::Engine,
            ring.sequence_range()
        ),
        Admission::Full
    );
}
#[test]
fn bounded_churn_and_reused_peer_do_not_preserve_old_delivery() {
    let mut ring = EventRing::load(8, 1, 16, 128).unwrap();
    let a = ring.bind(OutputTarget::Client(ClientId(0))).unwrap();
    let mut gate = Coordinator::load(1, 1);
    for frame in 0..256 {
        let record = record(&mut ring, a, frame, 42);
        gate.sweep(&ring, |_, _, _, _| false);
        assert_eq!(
            gate.admit(
                record,
                ClientId(0),
                a,
                PhysicalGroup::Peer(ClientId(0)),
                ring.sequence_range()
            ),
            Admission::Send
        );
        gate.submitted(
            a,
            record.sequence,
            OutputSubmission::BestEffort,
            true,
            false,
        );
        ring.submit(a, record.sequence, OutputSubmission::BestEffort);
    }
    let record = record(&mut ring, a, 255, 42);
    assert_eq!(
        gate.admit(
            record,
            ClientId(0),
            a,
            PhysicalGroup::Peer(ClientId(0)),
            ring.sequence_range()
        ),
        Admission::Suppressed
    );
    gate.negotiate(ClientId(0), Some(7));
    gate.reset_client(ClientId(0));
    assert_eq!(
        gate.group(ClientId(0), false),
        PhysicalGroup::Peer(ClientId(0))
    );
    assert_eq!(
        gate.admit(
            record,
            ClientId(0),
            a,
            PhysicalGroup::Peer(ClientId(0)),
            ring.sequence_range()
        ),
        Admission::Send
    );
}
