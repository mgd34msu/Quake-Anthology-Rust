use super::*;
use outputs::PeerUpdate;

fn host(protocol: Protocol) -> (FrameHost, ClientId) {
    let mut runtime = Runtime::load(1, []).unwrap();
    let client = runtime
        .connect_local(
            SeatId::FIRST,
            SpawnAnchor {
                position: Vec3::default(),
                angles: Vec3::default(),
                entity: 0,
                fixture_fallback: false,
            },
            ClientPolicy {
                input: protocol.rule_set(),
                client: protocol.rule_set(),
                movement: RuleSetId::Quake3,
                trace: RuleSetId::Quake2,
            },
            protocol,
            0,
        )
        .unwrap();
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![],
    )
    .unwrap();
    host.local_clients[0] = Some(client);
    (host, client)
}

fn peers(host: &mut FrameHost, client: ClientId, updates: &[PeerUpdate<'_>]) {
    let protocol = host.runtime.snapshots[client.0 as usize]
        .as_ref()
        .unwrap()
        .protocol;
    let mut bytes = [0; 1400];
    let mut length = 0;
    for &update in updates {
        length += outputs::peer_update(protocol, update, &mut bytes[length..]).unwrap();
    }
    let now = EventTime(host.time.0 + 1_000_000);
    send(host, client, &bytes[..length], now);
}

fn state(host: &FrameHost, client: ClientId) -> &qa_app::client_resources::ResourceState {
    &host.runtime.client_resources[client.0 as usize]
        .as_ref()
        .unwrap()
        .state
}

fn nq_header(host: &mut FrameHost, client: ClientId, max_clients: u8) {
    let mut bytes = vec![11];
    bytes.extend_from_slice(&15u32.to_le_bytes());
    bytes.extend_from_slice(&[max_clients, 0]);
    bytes.extend_from_slice(b"peer state\0\0\0");
    let now = EventTime(host.time.0 + 1_000_000);
    send(host, client, &bytes, now);
}

#[test]
fn native_peer_packets_preserve_raw_names_and_scalar_widths_before_assets() {
    let (mut host, client) = host(Protocol::NetQuake15);
    nq_header(&mut host, client, 2);
    let name = b"raw\\name\"and-high\xc1";
    peers(
        &mut host,
        client,
        &[
            PeerUpdate::Name { slot: 1, name },
            PeerUpdate::Colors {
                slot: 1,
                colors: 255,
            },
            PeerUpdate::Frags {
                slot: 1,
                frags: i16::MIN,
            },
        ],
    );
    assert!(host.runtime.model_assets.is_none());
    let peer = state(&host, client).peer(1).unwrap();
    assert_eq!(peer.text, name);
    assert_eq!(peer.metadata.colors, Some(255));
    assert_eq!(peer.metadata.frags, i16::MIN);
    assert_eq!(
        (peer.metadata.top_color, peer.metadata.bottom_color),
        (15, 15)
    );
    let rejected = state(&host, client).rejected;
    peers(
        &mut host,
        client,
        &[PeerUpdate::Colors {
            slot: 2,
            colors: 12,
        }],
    );
    assert_eq!(state(&host, client).rejected, rejected + 1);
    assert_eq!(state(&host, client).peer(2).unwrap().metadata.colors, None);
    nq_header(&mut host, client, 2);
    let peer = state(&host, client).peer(1).unwrap();
    assert!(peer.text.is_empty());
    assert_eq!(peer.metadata, Default::default());
}

#[test]
fn qw_userinfo_setinfo_and_native_capacity_mutations_share_received_rows() {
    let (mut host, client) = host(Protocol::QuakeWorld28);
    peers(
        &mut host,
        client,
        &[
            PeerUpdate::UserInfo {
                slot: 31,
                user_id: i32::MIN,
                info: b"\\name\\early\\skin\\one\\topcolor\\99\\bottomcolor\\-1",
            },
            PeerUpdate::Frags {
                slot: 31,
                frags: -1234,
            },
            PeerUpdate::SetInfo {
                slot: 31,
                key: b"skin",
                value: b"",
            },
            PeerUpdate::SetInfo {
                slot: 31,
                key: b"TEAM",
                value: b"ReD\xf1",
            },
            PeerUpdate::SetInfo {
                slot: 31,
                key: b"name",
                value: b"high\xc1;name",
            },
        ],
    );
    let peer = state(&host, client).peer(31).unwrap();
    assert_eq!(peer.metadata.user_id, Some(i32::MIN));
    assert_eq!(peer.metadata.frags, -1234);
    assert_eq!(
        (peer.metadata.top_color, peer.metadata.bottom_color),
        (99, -1)
    );
    assert!(qa_core::text::info_value(peer.text, b"skin", false).is_none());
    let value = |key| {
        let range = qa_core::text::info_value(peer.text, key, false).unwrap();
        &peer.text[range]
    };
    assert_eq!(value(b"team"), b"redq");
    assert_eq!(value(b"name"), b"high\xc1;name");

    let mut full = b"\\skin\\old\\pad\\".to_vec();
    full.extend_from_slice(&[b'x'; 180]);
    assert_eq!(full.len(), 194);
    peers(
        &mut host,
        client,
        &[PeerUpdate::UserInfo {
            slot: 0,
            user_id: i32::MAX,
            info: &full,
        }],
    );
    peers(
        &mut host,
        client,
        &[PeerUpdate::SetInfo {
            slot: 0,
            key: b"skin",
            value: b"longer",
        }],
    );
    assert_eq!(state(&host, client).peer(0).unwrap().text, full);

    let mut empty = b"\\skin\\\\pad\\".to_vec();
    empty.extend_from_slice(&[b'x'; 184]);
    assert_eq!(empty.len(), 195);
    peers(
        &mut host,
        client,
        &[PeerUpdate::UserInfo {
            slot: 0,
            user_id: 5,
            info: &empty,
        }],
    );
    peers(
        &mut host,
        client,
        &[PeerUpdate::SetInfo {
            slot: 0,
            key: b"skin",
            value: b"longer",
        }],
    );
    let remaining = state(&host, client).peer(0).unwrap().text;
    assert_eq!(remaining, &empty[6..]);
    assert!(qa_core::text::info_value(remaining, b"skin", false).is_none());

    let mut duplicate = b"\\skin\\first\\skin\\second".to_vec();
    peers(
        &mut host,
        client,
        &[PeerUpdate::UserInfo {
            slot: 0,
            user_id: 5,
            info: &duplicate,
        }],
    );
    peers(
        &mut host,
        client,
        &[PeerUpdate::SetInfo {
            slot: 0,
            key: b"skin",
            value: b"third",
        }],
    );
    assert_eq!(
        state(&host, client).peer(0).unwrap().text,
        b"\\skin\\second\\skin\\third"
    );
    duplicate = state(&host, client).peer(0).unwrap().text.to_vec();
    peers(
        &mut host,
        client,
        &[PeerUpdate::SetInfo {
            slot: 0,
            key: b"*spectator",
            value: b"1",
        }],
    );
    assert_eq!(state(&host, client).peer(0).unwrap().text, duplicate);

    let oversized = [b'x'; 300];
    peers(
        &mut host,
        client,
        &[
            PeerUpdate::UserInfo {
                slot: 2,
                user_id: -7,
                info: &oversized,
            },
            PeerUpdate::Frags {
                slot: 2,
                frags: i16::MAX,
            },
        ],
    );
    let peer = state(&host, client).peer(2).unwrap();
    assert_eq!(peer.text, &oversized[..195]);
    assert_eq!(peer.metadata.frags, i16::MAX);
    assert_eq!(peer.metadata.user_id, Some(-7));
}

#[test]
fn received_peer_metadata_survives_real_model_binding_profiles_and_connection_retirement() {
    let (mut host, client) = host(Protocol::QuakeWorld28);
    let info = b"\\name\\before-assets\\skin\\base\\topcolor\\9\\bottomcolor\\3";
    peers(
        &mut host,
        client,
        &[
            PeerUpdate::UserInfo {
                slot: 3,
                user_id: -42,
                info,
            },
            PeerUpdate::Frags {
                slot: 3,
                frags: 271,
            },
        ],
    );
    let metadata = state(&host, client).peer(3).unwrap().metadata;
    let profile = qa_app::model_assets::ModelProfile {
        source_rules: RuleSetId::QuakeWorld,
        rules: RuleSetId::Quake3,
        source_palette: None,
        palette: None,
        image_settings: qa_render::material::resources::ImageSettings::native(RuleSetId::Quake3),
        finish: Default::default(),
    };
    let second = qa_app::model_assets::ModelProfile {
        image_settings: qa_render::material::resources::ImageSettings {
            gamma_exponent: 1.5,
            ..profile.image_settings
        },
        ..profile
    };
    let mut owner = qa_app::model_assets::ModelAssets::new(
        &host.runtime.vfs,
        qa_render::Assets::load().unwrap(),
        [profile, second],
        [],
        Default::default(),
    )
    .unwrap();
    owner.publish_admission(owner.assets().counts()).unwrap();
    host.runtime.model_assets = Some(owner);
    for selected in [profile, second, profile] {
        host.bind_client_model_assets([(client, selected, None)]);
        let peer = state(&host, client).peer(3).unwrap();
        assert_eq!(peer.text, info);
        assert_eq!(peer.metadata, metadata);
    }
    assert!(host.runtime.disconnect_client(client));
    assert!(host.runtime.client_resources[client.0 as usize].is_none());
}
