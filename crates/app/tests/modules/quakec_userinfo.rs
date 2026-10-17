use super::*;
use qa_app::modules::ClientCommandError;
use qa_core::{events::OutputTarget, primitives::NativeTarget};
use qa_network::{
    commands::packet::Protocol,
    outputs::{NativeService, PeerUpdate},
};

fn connected(info: &[u8]) -> (Box<FrameHost>, ClientId) {
    let (mut host, id, _, _, _) = qc_client_host(
        RuleSetId::QuakeWorld,
        RuleSetId::Quake3,
        qc_client_vm(),
        &[],
    );
    host.connect_module_client(ModuleId(1), id, info).unwrap();
    (host, id)
}

fn command(
    host: &mut FrameHost,
    id: ClientId,
    key: &[u8],
    value: &[u8],
) -> Result<(), ClientCommandError> {
    let mut bytes = b"setinfo \"".to_vec();
    bytes.extend_from_slice(key);
    bytes.extend_from_slice(b"\" \"");
    bytes.extend_from_slice(value);
    bytes.extend_from_slice(b"\"");
    host.client_string_command(id, RuleSetId::QuakeWorld, &bytes)
        .map(|_| ())
}

fn updates(
    host: &FrameHost,
    observer: qa_core::events::OutputConsumerId,
) -> Vec<(u8, Vec<u8>, Vec<u8>)> {
    let events = &host.runtime.server.events;
    let mut batch = events.batch(observer).unwrap();
    let mut result = Vec::new();
    while let Some(record) = events.next(&mut batch) {
        let FrameEvent::Native(payload) = record.event else {
            continue;
        };
        let delivery = record.delivery.unwrap();
        assert_eq!(delivery.source, RuleSetId::QuakeWorld);
        assert_eq!(delivery.target, NativeTarget::All);
        assert!(delivery.reliable);
        let mut bytes = [0; 256];
        let wire = events.texts.copy_payload(payload, &mut bytes).unwrap();
        let (NativeService::Peer(PeerUpdate::SetInfo { slot, key, value }), consumed) =
            qa_network::outputs::read_native_service(
                Protocol::QuakeWorld28,
                &wire[..payload.len()],
            )
            .unwrap()
        else {
            panic!("setinfo must use the shared peer service")
        };
        assert_eq!(consumed, payload.len());
        result.push((slot, key.to_vec(), value.to_vec()));
    }
    result
}

#[test]
fn qw_setinfo_uses_effective_first_duplicate_value_and_no_guest_callback() {
    let (mut host, id) = connected(b"\\name\\native\\skin\\first\\skin\\second");
    let calls = host.module_counts(ModuleId(1)).unwrap().calls;
    let observer = host
        .runtime
        .server
        .events
        .bind(OutputTarget::Module(ModuleId(9)))
        .unwrap();
    command(&mut host, id, b"skin", b"third").unwrap();
    assert_eq!(
        host.runtime.server.userinfo(id).unwrap(),
        b"\\name\\native\\skin\\second\\skin\\third"
    );
    assert_eq!(
        updates(&host, observer),
        [(0, b"skin".to_vec(), b"second".to_vec())]
    );
    command(&mut host, id, b"skin", b"third").unwrap();
    let end = host.runtime.server.events.sequence_range().end;
    command(&mut host, id, b"skin", b"third").unwrap();
    command(&mut host, id, b"*spectator", b"1").unwrap();
    assert_eq!(host.runtime.server.events.sequence_range().end, end);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, calls);
    let previous = host.runtime.server.userinfo(id).unwrap().to_vec();
    assert!(command(&mut host, id, &[b'k'; 64], b"value").is_err());
    assert_eq!(host.runtime.server.userinfo(id).unwrap(), previous);
}

#[test]
fn qw_server_filtering_and_post_extraction_names_share_the_native_record() {
    let (mut host, id) = connected(b"\\name\\  console  ");
    assert_eq!(
        host.runtime.server.userinfo(id).unwrap(),
        b"\\name\\unnamed"
    );
    let observer = host
        .runtime
        .server
        .events
        .bind(OutputTarget::Module(ModuleId(9)))
        .unwrap();
    command(&mut host, id, b"name", b"  colored  ").unwrap();
    command(&mut host, id, b"team", b"\xc1RED;\x01").unwrap();
    assert_eq!(
        updates(&host, observer),
        [
            (0, b"name".to_vec(), b"colored".to_vec()),
            (0, b"team".to_vec(), b"\xc1RED;".to_vec()),
        ]
    );
    let highchars = host
        .console
        .cvars
        .bind(
            "sv_highchars",
            qa_console::views::Context {
                source: RuleSetId::QuakeWorld,
                side: qa_console::catalog::Scope::Server,
                ..Default::default()
            },
        )
        .unwrap();
    host.console.cvars.force_write(highchars, "0").unwrap();
    command(&mut host, id, b"team", b"\xc2LUE\xff\x01").unwrap();
    let text = host.runtime.server.userinfo(id).unwrap();
    let range = qa_core::text::info_value(text, b"team", false).unwrap();
    assert_eq!(&text[range], b"BLUE\x7f");
    command(&mut host, id, b"team", b"").unwrap();
    assert!(
        qa_core::text::info_value(host.runtime.server.userinfo(id).unwrap(), b"team", false)
            .is_none()
    );
}

#[test]
fn qw_setinfo_rolls_back_raw_userinfo_when_native_publication_cannot_fit() {
    let (mut host, id) = connected(b"\\name\\native\\skin\\original");
    let before = host.runtime.server.userinfo(id).unwrap().to_vec();
    host.runtime.server.events = qa_core::events::EventRing::load(8, 8, 1, 1).unwrap();
    assert!(command(&mut host, id, b"name", b" console ").is_err());
    assert_eq!(host.runtime.server.userinfo(id).unwrap(), before);
    assert!(host.runtime.server.events.sequence_range().is_empty());
    assert_eq!(host.runtime.server.events.rejected_publications(), 1);
}

#[test]
fn qw_setinfo_name_collision_uses_original_native_prefix_policy() {
    let (mut host, first, second) = qc_bot_namespace_host(NativeEntity {
        module: ModuleId(2),
        slot: 1,
    });
    host.runtime
        .server
        .replace_userinfo(
            second,
            b"\\name\\second",
            qa_compat::services::userinfo_rules(RuleSetId::QuakeWorld),
        )
        .unwrap();
    command(&mut host, second, b"name", b"first").unwrap();
    assert_eq!(
        host.runtime.server.userinfo(first).unwrap(),
        b"\\name\\first"
    );
    assert_eq!(
        host.runtime.server.userinfo(second).unwrap(),
        b"\\name\\(1)first"
    );
    command(&mut host, second, b"name", b"console").unwrap();
    assert_eq!(
        host.runtime.server.userinfo(second).unwrap(),
        b"\\name\\unnamed"
    );
    host.runtime
        .server
        .replace_userinfo(
            first,
            b"\\name\\abcdefghijklmnopqrst",
            qa_compat::services::userinfo_rules(RuleSetId::QuakeWorld),
        )
        .unwrap();
    command(&mut host, second, b"name", b"abcdefghijklmnopqrst").unwrap();
    assert_eq!(
        host.runtime.server.userinfo(second).unwrap(),
        b"\\name\\(1)abcdefghijklmnopqrst"
    );
}
