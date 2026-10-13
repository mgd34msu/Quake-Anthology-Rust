use qa_core::{
    primitives::{ClientId, ModuleId, NativeEntity, RuleSetId},
    text::{InfoBytes, InfoError, InfoRules},
};
use qa_session::clients::{ClientInfoError, Connection, Server, USERINFO_BYTES};

fn rules(bytes: usize) -> InfoRules {
    InfoRules {
        bytes,
        key_bytes: bytes,
        value_bytes: bytes,
        filter: InfoBytes::Exact,
    }
}
fn client() -> (Server, ClientId) {
    let mut server = Server::load(2, 8, 1, 1, 1, 0).unwrap();
    let native = NativeEntity {
        module: ModuleId(10),
        slot: 0,
    };
    let id = server
        .connect(Connection::Local, native.module, Some(native))
        .unwrap();
    (server, id)
}

#[test]
fn canonical_userinfo_preserves_raw_names_duplicates_and_stable_storage() {
    let (mut server, id) = client();
    let address = server.userinfo(id).unwrap().as_ptr();
    let bytes = b"\\Name\\raw\xff\\name\\second\\Name\\third";
    server.replace_userinfo(id, bytes, rules(1024)).unwrap();
    assert_eq!(server.userinfo(id), Some(bytes.as_slice()));
    assert_eq!(server.userinfo(id).unwrap().as_ptr(), address);
    server
        .replace_userinfo(id, b"key\\value", rules(1024))
        .unwrap();
    assert_eq!(server.userinfo(id), Some(b"key\\value".as_slice()));
    assert_eq!(server.userinfo(id).unwrap().as_ptr(), address);
}

#[test]
fn checked_replacement_is_atomic_at_native_and_canonical_limits() {
    let (mut server, id) = client();
    let mut maximum = b"\\k\\".to_vec();
    maximum.resize(USERINFO_BYTES - 1, b'x');
    server
        .replace_userinfo(id, &maximum, rules(USERINFO_BYTES))
        .unwrap();
    maximum.push(b'y');
    assert_eq!(
        server.replace_userinfo(id, &maximum, rules(USERINFO_BYTES)),
        Err(ClientInfoError::Info(InfoError::Capacity))
    );
    assert_eq!(server.userinfo(id).unwrap(), &maximum[..USERINFO_BYTES - 1]);
    server
        .replace_userinfo(id, b"\\safe\\old", rules(1024))
        .unwrap();
    for bytes in [
        b"\\bad\\semicolon;".as_slice(),
        b"\\bad\\quote\"",
        b"\\bad\\nul\0tail",
        b"\\dangling",
    ] {
        assert_eq!(
            server.replace_userinfo(id, bytes, rules(1024)),
            Err(ClientInfoError::Info(InfoError::Invalid))
        );
        assert_eq!(server.userinfo(id), Some(b"\\safe\\old".as_slice()));
    }
    let small = InfoRules {
        bytes: 64,
        key_bytes: 4,
        value_bytes: 64,
        filter: InfoBytes::Exact,
    };
    assert_eq!(
        server.replace_userinfo(id, b"\\long\\value", small),
        Err(ClientInfoError::Info(InfoError::Invalid))
    );
    assert_eq!(server.userinfo(id), Some(b"\\safe\\old".as_slice()));
    let mut boundary = b"\\k\\".to_vec();
    boundary.resize(63, b'x');
    server.replace_userinfo(id, &boundary, small).unwrap();
    boundary.push(b'x');
    assert_eq!(
        server.replace_userinfo(id, &boundary, small),
        Err(ClientInfoError::Info(InfoError::Capacity))
    );
    assert_eq!(server.userinfo(id), Some(&boundary[..63]));
}

#[test]
fn native_raw_replacement_keeps_truncation_and_disconnect_resets_the_one_record() {
    let (mut server, id) = client();
    let old_lifetime = server.clients[id.0 as usize].entity;
    let raw = b"broken;\"\x01\xffinfo";
    server.replace_userinfo_raw(id, raw, 9).unwrap();
    assert_eq!(server.userinfo(id), Some(&raw[..8]));
    server
        .replace_userinfo_raw(id, b"head\0ignored", 1024)
        .unwrap();
    assert_eq!(server.userinfo(id), Some(b"head".as_slice()));
    assert!(server.disconnect(id));
    assert_eq!(server.userinfo(id), None);
    assert_eq!(
        server.replace_userinfo(id, b"", rules(1024)),
        Err(ClientInfoError::Client)
    );
    let next = server
        .connect(Connection::Remote, ModuleId(12), None)
        .unwrap();
    assert_eq!(next, id);
    assert_ne!(server.clients[next.0 as usize].entity, old_lifetime);
    assert_eq!(server.userinfo(next), Some(b"".as_slice()));
}

#[test]
fn module_admission_rules_do_not_inherit_player_movement_or_trace() {
    let (mut server, id) = client();
    server.clients[id.0 as usize].player.movement_rules = RuleSetId::Quake3;
    server.clients[id.0 as usize].player.trace_rules = RuleSetId::QuakeWorld;
    let bytes = b"\\name\\long-native-name";
    assert_eq!(
        server.replace_userinfo(id, bytes, rules(8)),
        Err(ClientInfoError::Info(InfoError::Capacity))
    );
    server.replace_userinfo(id, bytes, rules(1024)).unwrap();
    assert_eq!(server.userinfo(id), Some(bytes.as_slice()));
    assert_eq!(
        (
            server.clients[id.0 as usize].player.movement_rules,
            server.clients[id.0 as usize].player.trace_rules
        ),
        (RuleSetId::Quake3, RuleSetId::QuakeWorld)
    );
}
