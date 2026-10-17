use super::*;

fn command(host: &mut FrameHost, seat: SeatId, text: &str) {
    host.console
        .append(
            text,
            Context {
                source: RULES,
                seat,
                ..Default::default()
            },
        )
        .unwrap();
    host.execute_commands_with_driver(&mut qa_app::modules::Driver::cache_only());
}

fn frame(host: &mut FrameHost, source: &mut Source) {
    assert_eq!(host.frame(source, true).drains, 2);
}

fn replace(host: &FrameHost, name: &str, bytes: &[u8]) {
    let root = &host
        .runtime
        .vfs
        .mounts()
        .find(|mount| mount.kind == qa_content::vfs::MountKind::Directory)
        .unwrap()
        .path;
    let temporary = root.join("skin-replacement.tmp");
    qa_platform::files::write(&temporary, bytes).unwrap();
    qa_platform::files::rename(temporary, root.join(name)).unwrap();
}

fn command_frame(host: &mut FrameHost, source: &mut Source, seat: SeatId, text: &str) {
    let attempts = source.asset_attempts;
    command(host, seat, text);
    assert_eq!(
        source.asset_attempts, attempts,
        "commands only request cold work"
    );
    frame(host, source);
    assert_eq!(
        source.asset_attempts,
        attempts + 1,
        "one backend admission covers the complete refresh batch"
    );
}

#[test]
fn explicit_same_name_commands_rescan_replacements_new_files_and_fallbacks() {
    let (_fixture, mut host, mut source) = loading_model(RULES, colored_mdl(false), &[], false);
    colored_peers(&mut host, &mut source);
    prepare(&mut host, &mut source);
    command_frame(&mut host, &mut source, SeatId::FIRST, "allskins local\n");
    let original = draw(&host, true);
    assert_eq!((original[0].first, original[1].first), (32, 64));

    replace(
        &host,
        "skins/local.pcx",
        &support::pcx_dimensions(296, 194, 17),
    );
    command_frame(
        &mut host,
        &mut source,
        SeatId::FIRST,
        "allskins local\nallskins local\nskins\n",
    );
    let replaced = draw(&host, true);
    assert_eq!((replaced[0].first, replaced[1].first), (33, 65));
    assert_ne!(replaced[0].id, original[0].id);
    assert_eq!(replaced[2], original[2]);

    replace(
        &host,
        "skins/downloaded.pcx",
        &support::pcx_dimensions(296, 194, 18),
    );
    assert!(host.runtime.vfs.open(b"skins/downloaded.pcx").is_none());
    command_frame(
        &mut host,
        &mut source,
        SeatId::FIRST,
        "allskins downloaded\n",
    );
    let downloaded = draw(&host, true);
    assert_eq!((downloaded[0].first, downloaded[1].first), (34, 66));
    assert_eq!(downloaded[0].size, [320, 200]);

    command_frame(&mut host, &mut source, SeatId::FIRST, "allskins absent\n");
    let missing = draw(&host, true);
    assert_eq!((missing[0].first, missing[1].first), (48, 80));
    assert_eq!(missing[0].size, [320, 200]);

    command_frame(&mut host, &mut source, SeatId::FIRST, "allskins broken\n");
    let malformed = draw(&host, true);
    assert_eq!((malformed[0].first, malformed[0].second), (32, 48));
    assert_eq!((malformed[1].first, malformed[1].second), (64, 80));
    assert_eq!(malformed[0].size, [2, 1]);
    assert_eq!(malformed[2], original[2]);
}

#[test]
fn rejected_refresh_preserves_live_pixels_and_override_until_explicit_retry() {
    let (_fixture, mut host, mut source) = loading_model(RULES, colored_mdl(false), &[], false);
    colored_peers(&mut host, &mut source);
    prepare(&mut host, &mut source);
    command_frame(&mut host, &mut source, SeatId::FIRST, "allskins local\n");
    let live = draw(&host, true);
    let counts = host
        .runtime
        .model_assets
        .as_ref()
        .unwrap()
        .assets()
        .counts();
    replace(
        &host,
        "skins/local.pcx",
        &support::pcx_dimensions(296, 194, 17),
    );
    replace(
        &host,
        "skins/base.pcx",
        &support::pcx_dimensions(296, 194, 97),
    );
    source.reject_assets = true;
    command_frame(&mut host, &mut source, SeatId::FIRST, "allskins base\n");
    assert_eq!(draw(&host, true), live, "rejected selectors remain live");
    assert_eq!(
        host.runtime
            .model_assets
            .as_ref()
            .unwrap()
            .assets()
            .counts(),
        counts
    );

    source.reject_assets = false;
    let attempts = source.asset_attempts;
    frame(&mut host, &mut source);
    assert_eq!(
        source.asset_attempts, attempts,
        "failure consumes its request"
    );
    assert_eq!(draw(&host, true), live);

    command_frame(&mut host, &mut source, SeatId::FIRST, "skins\n");
    let refreshed = draw(&host, true);
    assert_eq!(
        (refreshed[0].first, refreshed[1].first),
        (33, 65),
        "skins preserves the accepted local override after rejecting base"
    );
    command_frame(&mut host, &mut source, SeatId::FIRST, "allskins base\n");
    let retried = draw(&host, true);
    assert_eq!((retried[0].first, retried[1].first), (49, 81));
    assert_ne!(retried[0].id, live[0].id);
    assert_eq!(retried[2], live[2]);
}

fn add_seat(host: &mut FrameHost, source: &mut Source) -> ClientId {
    let client = ClientId(1);
    let server = NativeServer::load(Protocol::QuakeWorld28, client);
    let mut channel =
        Channel::load(Protocol::QuakeWorld28.channel(), Endpoint::Client, 8192, 16).unwrap();
    channel.set_qport(27001);
    let admitted = host
        .runtime
        .connect_client(
            SeatId::ALL[1],
            ClientPolicy {
                input: RULES,
                client: RULES,
                movement: RuleSetId::Quake3,
                trace: RuleSetId::Quake2,
            },
            NativePlayerBinding::Supplied(0),
            Connection {
                phase: ConnectionPhase::Connected,
                route: Route {
                    socket: SOCKET,
                    peer: Peer::Socket(server.peer),
                },
                channel,
                output: None,
                pending_output: None,
                commands: Some(Commands::load(Protocol::QuakeWorld28, 8192)),
            },
            None,
        )
        .unwrap();
    assert_eq!(admitted, client);
    let layout = ConfigLayout::for_rules(RULES);
    let resources = host.runtime.client_resources[1].as_mut().unwrap();
    resources
        .state
        .configs
        .set(layout.resources[0].get(1).unwrap(), b"maps/player.bsp")
        .unwrap();
    for (ordinal, name) in [
        (2, b"progs/player.mdl".as_slice()),
        (3, b"progs/ordinary.mdl"),
    ] {
        assert!(resources.apply(layout.resources[0].get(ordinal).unwrap(), name, None, None));
    }
    host.local_clients[1] = Some(client);
    host.local_worlds[1] = host.local_worlds[0];
    host.local_presentations[1] = host.local_presentations[0];
    host.local_presentation_rules[1] = Some(RULES);
    let profile = host
        .runtime
        .model_assets
        .as_ref()
        .unwrap()
        .catalog()
        .profiles()
        .next()
        .unwrap();
    host.bind_client_model_assets([
        (CLIENT, profile, host.local_worlds[0]),
        (client, profile, host.local_worlds[1]),
    ]);
    source.servers.push(server);
    let mut bytes = [0; 512];
    let mut length = 0;
    for update in [
        PeerUpdate::UserInfo {
            slot: 0,
            user_id: 301,
            info: b"\\name\\Other\\skin\\local\\topcolor\\2\\bottomcolor\\3",
        },
        PeerUpdate::UserInfo {
            slot: 1,
            user_id: 302,
            info: b"\\name\\Peer\\skin\\local\\topcolor\\4\\bottomcolor\\5",
        },
    ] {
        length +=
            outputs::peer_update(Protocol::QuakeWorld28, update, &mut bytes[length..]).unwrap();
    }
    send(host, &mut source.servers[1], &bytes[..length]);
    client
}

#[test]
fn skins_preserves_independent_seat_overrides_and_one_seat_clear_keeps_the_other() {
    let (_fixture, mut host, mut source) = loading_model(RULES, colored_mdl(false), &[], false);
    colored_peers(&mut host, &mut source);
    let other = add_seat(&mut host, &mut source);
    prepare(&mut host, &mut source);
    command(&mut host, SeatId::FIRST, "allskins local\n");
    let attempts = source.asset_attempts;
    command_frame(&mut host, &mut source, SeatId::ALL[1], "allskins base\n");
    assert_eq!(source.asset_attempts, attempts + 1);
    assert_eq!(draw(&host, true)[0].first, 32);
    assert_eq!(
        draw_client_colormaps(&host, other, true, 1.0, [1, 2, 1])[0].first,
        48
    );

    replace(
        &host,
        "skins/local.pcx",
        &support::pcx_dimensions(296, 194, 17),
    );
    replace(
        &host,
        "skins/base.pcx",
        &support::pcx_dimensions(296, 194, 97),
    );
    command_frame(&mut host, &mut source, SeatId::ALL[1], "skins\n");
    assert_eq!(draw(&host, true)[0].first, 33);
    assert_eq!(
        draw_client_colormaps(&host, other, true, 1.0, [1, 2, 1])[0].first,
        49
    );

    command_frame(&mut host, &mut source, SeatId::ALL[1], "allskins\n");
    assert_eq!(draw(&host, true)[0].first, 33);
    assert_eq!(
        draw_client_colormaps(&host, other, true, 1.0, [1, 2, 1])[0].first,
        33
    );
}
