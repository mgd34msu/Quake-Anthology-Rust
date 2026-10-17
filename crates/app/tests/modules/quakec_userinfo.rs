use super::*;
use qa_app::modules::ClientCommandError;

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

#[test]
fn qw_setinfo_uses_effective_first_duplicate_value_and_no_guest_callback() {
    let (mut host, id) = connected(b"\\name\\native\\skin\\first\\skin\\second");
    let calls = host.module_counts(ModuleId(1)).unwrap().calls;
    command(&mut host, id, b"skin", b"third").unwrap();
    assert_eq!(
        host.runtime.server.userinfo(id).unwrap(),
        b"\\name\\native\\skin\\second\\skin\\third"
    );
    assert!(host.runtime.server.userinfo_record(id).unwrap().dirty());
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
    command(&mut host, id, b"name", b"  colored  ").unwrap();
    command(&mut host, id, b"team", b"\xc1RED;\x01").unwrap();
    let text = host.runtime.server.userinfo(id).unwrap();
    for (key, value) in [
        (b"name".as_slice(), b"colored".as_slice()),
        (b"team", b"\xc1RED;"),
    ] {
        let span = qa_core::text::info_value(text, key, false).unwrap();
        assert_eq!(&text[span], value);
    }
    assert!(host.runtime.server.userinfo_record(id).unwrap().dirty());
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
fn qw_setinfo_retains_accepted_bytes_and_retries_after_independent_text_leases_release() {
    let (mut host, id) = connected(b"\\name\\native\\skin\\original");
    let calls = host.module_counts(ModuleId(1)).unwrap().calls;
    let mut leases = Vec::new();
    while let Some(lease) = host.runtime.server.events.texts.insert(b"retained display") {
        leases.push(lease);
    }
    let rejected = host.runtime.server.events.rejected_publications();
    command(&mut host, id, b"name", b" console ").unwrap();
    assert_eq!(
        host.runtime.server.userinfo(id).unwrap(),
        b"\\skin\\original\\name\\unnamed"
    );
    let mut source = Source {
        time: EventTime::default(),
        polls: 0,
    };
    host.dispatch_output(&mut source, &mut Default::default());
    assert!(host.runtime.server.userinfo_record(id).unwrap().dirty());
    assert!(host.runtime.server.events.rejected_publications() > rejected);
    for lease in leases {
        host.runtime.server.events.texts.release(lease);
    }
    host.dispatch_output(&mut source, &mut Default::default());
    assert!(!host.runtime.server.userinfo_record(id).unwrap().dirty());
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, calls);
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
