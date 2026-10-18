use super::*;
use crate::client_resources::ConfigPublication;
use qa_core::{
    events::{EventRing, FrameEvent, OutputSubmission, OutputTarget, dispatch_consumer},
    primitives::{ClientId, ModuleId},
};
#[path = "server_info/directory.rs"]
mod directory;

fn set(cvars: &mut Cvars, name: &str, value: &str) {
    let context = Context {
        side: qa_console::catalog::Scope::Server,
        ..Context::default()
    };
    let view = cvars.bind(name, context).unwrap();
    cvars.force_write(view, value).unwrap();
}

#[test]
fn resource_rebinding_preserves_unrelated_publication_cursors_and_native_receipts() {
    use crate::client_resources::ResourceReadiness;
    use qa_core::primitives::{GeometryId, RuleSetId};
    let bank =
        qa_audio::Bank::with_capacity(std::num::NonZeroU32::new(44100).unwrap(), 1, 32).unwrap();
    let replacement =
        qa_audio::Bank::with_capacity(std::num::NonZeroU32::new(44100).unwrap(), 1, 32).unwrap();
    for source_rules in RuleSetId::ALL {
        let source = ConfigLayout::for_rules(source_rules);
        for target_rules in RuleSetId::ALL {
            let target = ConfigLayout::for_rules(target_rules);
            for affected in [0, 1] {
                let mut p = ConfigPublication::load();
                p.configure_sources(2, 32);
                p.bind_sounds(&bank);
                p.connection(ModuleId(1), source, target);
                p.model_world(Some(GeometryId {
                    slot: 0,
                    generation: 1,
                }));
                let other = |layout: ConfigLayout| {
                    (0..layout.count)
                        .find(|&index| layout.resources.iter().all(|range| !range.contains(index)))
                        .unwrap()
                };
                let source_rows = [
                    other(source),
                    source.resources[affected].first as usize + 1,
                    source.resources[1 - affected].first as usize + 1,
                ];
                let [unrelated, changed, retained] = [
                    other(target),
                    target.resources[affected].first as usize + 1,
                    target.resources[1 - affected].first as usize + 1,
                ];
                for row in source_rows {
                    p.seen(row, 3);
                }
                for row in [unrelated, changed, retained] {
                    p.published(row, Some(row as u64 + 100));
                }
                p.acknowledged(changed, changed as u64 + 100);
                p.acknowledged(retained, retained as u64 + 100);
                if let Some(index) = target.max_clients() {
                    p.published_client_limit(index, 8, 99);
                }
                p.finish(ModuleId(1), 7);
                if affected == 0 {
                    p.model_world(Some(GeometryId {
                        slot: 1,
                        generation: 1,
                    }));
                } else {
                    p.bind_sounds(&replacement);
                }
                assert!(
                    p.begin(ModuleId(1), 7),
                    "affected producer must be revisited"
                );
                assert!(
                    p.needs(source_rows[1], 3),
                    "{source_rules:?} -> {target_rules:?} resource {affected} kept the affected source cursor"
                );
                assert_eq!(p.readiness(changed), ResourceReadiness::Pending);
                assert!(
                    !p.needs(source_rows[0], 3),
                    "{source_rules:?} -> {target_rules:?} resource {affected} cleared an unrelated cursor"
                );
                assert!(!p.needs(source_rows[2], 3));
                assert_eq!(p.readiness(retained), ResourceReadiness::Ready);
                assert_eq!(p.readiness(unrelated), ResourceReadiness::Pending);
                p.acknowledged(unrelated, unrelated as u64 + 100);
                assert_eq!(
                    p.readiness(unrelated),
                    ResourceReadiness::Ready,
                    "unrelated pending receipt must still accept its native ACK"
                );
                if let Some(index) = target.max_clients() {
                    assert!(!p.needs_client_limit(8));
                    p.acknowledged(index, 99);
                    assert_eq!(p.readiness(index), ResourceReadiness::Ready);
                }
            }
        }
    }
}

#[test]
fn server_info_prepares_once_and_publishes_each_client_only_after_success() {
    let mut cvars = Cvars::new().unwrap();
    set(&mut cvars, "hostname", "original");
    set(&mut cvars, "sv_pure", "0");
    let vfs = Vfs::default();
    let mut info = ServerInfo::load(&cvars, Context::default(), 0);
    assert!(info.prepare(&cvars, &vfs, None, None));
    let initial = [0, 1].map(|index| Some(info.revision(index)));
    let mut healthy = ConfigPublication::load();
    let mut stalled = ConfigPublication::load();
    healthy.server_info = initial;
    stalled.server_info = initial;
    let mut events = EventRing::load(8, 2, 4, MAX_TEXT).unwrap();
    let client = ClientId(1);
    let peer = ClientId(2);
    let consumer = events.bind(OutputTarget::Client(client)).unwrap();
    let peer_consumer = events.bind(OutputTarget::Client(peer)).unwrap();
    let mut leases = (0..4)
        .map(|_| events.texts.insert(b"display lease").unwrap())
        .collect::<Vec<_>>()
        .into_iter();
    set(&mut cvars, "hostname", "live server");
    assert!(info.prepare(&cvars, &vfs, None, None));
    info.publish(&mut healthy, &mut events, client, ModuleId(1));
    assert_eq!(
        healthy.server_info, initial,
        "rejected publication cannot advance a cursor"
    );
    assert_eq!(events.rejected_publications(), 1);
    events.texts.release(leases.next().unwrap());
    info.publish(&mut healthy, &mut events, client, ModuleId(1));
    let mut batch = events.batch(consumer).unwrap();
    let record = events.next(&mut batch).unwrap();
    let FrameEvent::ConfigString {
        destination_index,
        text,
        ..
    } = record.event
    else {
        panic!("native info row")
    };
    assert_eq!(destination_index, ConfigLayout::Q3_SERVER_INFO as u32);
    let bytes = events.texts.get(text).unwrap();
    let field = qa_core::text::info_value(bytes, b"sv_hostname", true).unwrap();
    assert_eq!(&bytes[field], b"live server");
    info.publish(&mut healthy, &mut events, client, ModuleId(1));
    assert_eq!(
        events.len(),
        1,
        "an unread successful row must not be republished"
    );
    info.publish(&mut stalled, &mut events, peer, ModuleId(1));
    assert_eq!(stalled.server_info, initial);
    assert_eq!(events.rejected_publications(), 2);
    assert!(
        events
            .next(&mut events.batch(peer_consumer).unwrap())
            .is_none()
    );
    events.texts.release(leases.next().unwrap());
    info.publish(&mut stalled, &mut events, peer, ModuleId(1));
    assert_eq!(stalled.server_info, healthy.server_info);
    for consumer in [consumer, peer_consumer] {
        let mut rows = 0;
        dispatch_consumer(
            &mut events,
            consumer,
            &mut |record: qa_core::events::OutputRecord, _: &mut qa_core::events::TextStore| {
                rows += usize::from(matches!(record.event, FrameEvent::ConfigString { .. }));
                OutputSubmission::BestEffort
            },
        );
        assert_eq!(rows, 1);
    }
    for lease in leases {
        events.texts.release(lease);
    }
    let revisions = [0, 1].map(|index| info.revision(index));
    let telemetry = cvars
        .register("_qa_live_telemetry", Some("0"), 0, Context::default())
        .unwrap();
    cvars.force_write(telemetry, "1").unwrap();
    cvars.reset_lookup_count();
    vfs.take_lookup_count();
    for _ in 0..3 {
        assert!(info.prepare(&cvars, &vfs, None, None));
    }
    assert_eq!([0, 1].map(|index| info.revision(index)), revisions);
    assert_eq!(cvars.lookup_count(), 0);
    assert_eq!(vfs.take_lookup_count(), 0);
    set(&mut cvars, "sv_pure", "1");
    assert!(info.prepare(&cvars, &vfs, None, None));
    assert!(info.pure);
    assert!(info.publish(&mut healthy, &mut events, client, ModuleId(1)));
    let mut batch = events.batch(consumer).unwrap();
    let record = events.next(&mut batch).unwrap();
    let FrameEvent::ConfigString {
        destination_index,
        text,
        ..
    } = record.event
    else {
        panic!("native system info")
    };
    assert_eq!(destination_index, ConfigLayout::Q3_SYSTEM_INFO as u32);
    assert_eq!(
        qa_core::text::info_integer(events.texts.get(text).unwrap(), b"sv_pure"),
        1
    );
    assert!(!info.publish(&mut healthy, &mut events, client, ModuleId(1)));
}
