use super::*;
use qa_network::outputs::PeerUpdate;

#[test]
fn scalar_peer_updates_do_not_invalidate_media_bindings() {
    let mut players = PlayerBindings::load(ConfigLayout::QUAKEWORLD).unwrap();
    let info = b"\\topcolor\\9\\bottomcolor\\3";
    players.dirty = false;
    players.cold_dirty = false;
    assert!(players.receive(
        PeerUpdate::UserInfo {
            slot: 0,
            user_id: 17,
            info
        },
        info
    ));
    assert!(players.dirty && players.cold_dirty);
    players.dirty = false;
    players.cold_dirty = false;
    for update in [
        PeerUpdate::Frags {
            slot: 0,
            frags: i16::MIN,
        },
        PeerUpdate::Frags {
            slot: 0,
            frags: i16::MIN,
        },
        PeerUpdate::UserInfo {
            slot: 0,
            user_id: -42,
            info,
        },
    ] {
        assert!(players.receive(update, info));
        assert!(!players.dirty && !players.cold_dirty);
    }
    assert_eq!(players.metadata(0).unwrap().user_id, Some(-42));
    assert_eq!(players.metadata(0).unwrap().frags, i16::MIN);
    assert!(players.receive(
        PeerUpdate::SetInfo {
            slot: 0,
            key: b"topcolor",
            value: b"10"
        },
        b"\\topcolor\\10\\bottomcolor\\3"
    ));
    assert!(players.dirty && players.cold_dirty);
    players.clear_bindings();
    assert_eq!(players.metadata(0).unwrap().user_id, Some(-42));
    players.clear_received();
    assert_eq!(players.metadata(0), Some(PeerMetadata::default()));
}
