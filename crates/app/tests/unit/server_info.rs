use super::*;
use crate::client_resources::ConfigPublication;
use qa_core::{
    events::{EventRing, FrameEvent, OutputSubmission, OutputTarget, dispatch_consumer},
    primitives::{ClientId, ModuleId},
};

fn set(cvars: &mut Cvars, name: &str, value: &str) {
    let context = Context {
        side: qa_console::catalog::Scope::Server,
        ..Context::default()
    };
    let view = cvars.bind(name, context).unwrap();
    cvars.force_write(view, value).unwrap();
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
