use super::*;
use qa_content::vfs::{MountId, SearchView, Vfs};
use qa_core::{random::Random, text::FixedText};
use std::sync::Arc;

#[path = "../model_assets/support.rs"]
#[allow(dead_code)]
mod support;
#[path = "../../../formats/tests/support/stored_zip.rs"]
mod zip;

fn package(vfs: &mut Vfs, name: &str, priority: i32, marker: &[u8]) -> (MountId, u32) {
    let root = vfs.directory_path(vfs.mounts().next().unwrap().id).unwrap();
    let path = root.join(name);
    qa_platform::files::write(
        &path,
        zip::stored_zip(&[
            (b"vm/cgame.qvm", marker),
            (b"vm/ui.qvm", marker),
            (b"model.bin", marker),
        ]),
    )
    .unwrap();
    let archive = Arc::new(
        qa_formats::archive::Archive::parse(Arc::new(
            qa_platform::files::ReadFile::open(&path).unwrap(),
        ))
        .unwrap(),
    );
    let sum = archive.q3_checksums().unwrap().checksum();
    (vfs.mount_archive(&path, priority, archive).unwrap(), sum)
}
fn client(vfs: &Vfs, id: u32, search: Option<&SearchView>) -> ClientResources {
    ClientResources::load(
        ClientId(id),
        Protocol::Quake3_68,
        RuleSetId::Quake3,
        vfs,
        search,
    )
    .unwrap()
    .unwrap()
}
fn admit(client: &mut ClientResources, vfs: &Vfs, checksums: &[u32], feed: u32) {
    client.startup.connecting(Protocol::Quake3_68);
    client.startup.q3_gamestate();
    let sums = checksums
        .iter()
        .map(|sum| (*sum as i32).to_string())
        .collect::<Vec<_>>()
        .join(" ");
    client
        .state
        .configs
        .set(
            ConfigLayout::Q3_SYSTEM_INFO,
            format!("\\sv_serverid\\7\\sv_paks\\{sums}").as_bytes(),
        )
        .unwrap();
    client.admit_q3_files(vfs, feed, Random::new(42)).unwrap();
    assert!(client.startup.allow_models());
}
fn read(vfs: &Vfs, client: &ClientResources, name: &[u8]) -> Option<Vec<u8>> {
    let view = vfs.read_view(Some(client.state.file_search()));
    let file = view.open(name)?;
    Some(
        view.read_to_vec(&file, 256, &mut Default::default())
            .unwrap(),
    )
}
fn report(vfs: &Vfs, client: &ClientResources) -> String {
    let mut text = FixedText::<8191>::default();
    client
        .state
        .file_search()
        .q3_reference_command(vfs, 7, true, &mut text)
        .unwrap();
    text.as_str().to_owned()
}
#[test]
fn admission_reorders_native_packages_and_keeps_connection_reference_accounts_separate() {
    let mut files = support::Fixture::new(&[
        ("loose.bin", b"loose".to_vec()),
        ("config.cfg", b"allowed".to_vec()),
    ]);
    let (first, first_sum) = package(&mut files.vfs, "first.pk3", 1, b"first");
    let (second, second_sum) = package(&mut files.vfs, "second.pk3", 2, b"second");
    let (_, denied_sum) = package(&mut files.vfs, "denied.pk3", 3, b"denied");
    let mut a = client(&files.vfs, 0, None);
    let mut b = client(&files.vfs, 1, None);
    let feed = 0x8123_4567;
    admit(&mut a, &files.vfs, &[first_sum, second_sum], feed);
    admit(&mut b, &files.vfs, &[second_sum], feed);
    assert_ne!(a.state.file_source(), b.state.file_source());
    assert_eq!(read(&files.vfs, &a, b"model.bin").unwrap(), b"first");
    assert_eq!(read(&files.vfs, &b, b"model.bin").unwrap(), b"second");
    assert!(read(&files.vfs, &a, b"loose.bin").is_none());
    assert_eq!(read(&files.vfs, &a, b"config.cfg").unwrap(), b"allowed");
    assert_eq!(
        files
            .vfs
            .read_to_vec(
                &files.vfs.open(b"model.bin").unwrap(),
                256,
                &mut Default::default()
            )
            .unwrap(),
        b"denied"
    );
    let narrow = files.vfs.search_view([second]).unwrap();
    let child = files
        .vfs
        .admitted_search(Some(&narrow), a.state.file_search())
        .unwrap();
    let child_view = files.vfs.read_view(Some(&child));
    let file = child_view.open(b"model.bin").unwrap();
    assert_eq!(files.vfs.origin(&file).unwrap().mount, second);
    assert!(child_view.open(b"loose.bin").is_none());
    assert!(Vfs::default().admitted_search(None, &child).is_err());
    // Narrowing cannot remove the original caller's read policy, even if the
    // admitted connection would otherwise permit the source.
    let requested = files
        .vfs
        .restricted_search(Some(&narrow), &[], &[b".cfg"])
        .unwrap();
    let denied = files
        .vfs
        .admitted_search(Some(&requested), a.state.file_search())
        .unwrap();
    assert!(
        files
            .vfs
            .read_view(Some(&denied))
            .open(b"model.bin")
            .is_none()
    );
    for c in [&a, &b] {
        read(&files.vfs, c, b"vm/cgame.qvm").unwrap();
        read(&files.vfs, c, b"vm/ui.qvm").unwrap();
    }
    let expected = |mount| {
        let pure = files
            .vfs
            .q3_package_checksums(mount)
            .unwrap()
            .pure_checksum(feed);
        format!(
            "cp 7 {} {} @ {} {}",
            pure as i32,
            pure as i32,
            pure as i32,
            (feed ^ pure ^ 1) as i32
        )
    };
    let first_pure = files
        .vfs
        .q3_package_checksums(first)
        .unwrap()
        .pure_checksum(feed);
    let second_pure = files
        .vfs
        .q3_package_checksums(second)
        .unwrap()
        .pure_checksum(feed);
    assert_eq!(
        report(&files.vfs, &a),
        format!(
            "cp 7 {} {} @ {} {} {}",
            first_pure as i32,
            first_pure as i32,
            first_pure as i32,
            second_pure as i32,
            (feed ^ first_pure ^ second_pure ^ 2) as i32
        )
    );
    assert_eq!(report(&files.vfs, &b), expected(second));
    let previous = a.state.files.clone();
    admit(&mut a, &files.vfs, &[denied_sum], feed + 1);
    assert_eq!(read(&files.vfs, &a, b"model.bin").unwrap(), b"denied");
    assert!(!previous.view().same_selection(a.state.file_search()));
    // The new account has no inherited UI/cgame references, and the other
    // connection retains its exact prior account and ordering.
    let mut text = FixedText::<8191>::default();
    assert!(
        a.state
            .file_search()
            .q3_reference_command(&files.vfs, 8, true, &mut text)
            .is_err()
    );
    assert_eq!(report(&files.vfs, &b), expected(second));
    admit(&mut a, &files.vfs, &[], feed);
    assert_eq!(read(&files.vfs, &a, b"model.bin").unwrap(), b"denied");
    assert_eq!(read(&files.vfs, &a, b"loose.bin").unwrap(), b"loose");
}
#[test]
fn downloaded_mounts_join_live_requests_but_cannot_escape_an_explicit_namespace() {
    let mut files = support::Fixture::new(&[]);
    let (first, sum) = package(&mut files.vfs, "first.pk3", 1, b"first");
    let selected = files.vfs.search_view([first]).unwrap();
    let mut fixed = client(&files.vfs, 0, Some(&selected));
    let mut live = client(&files.vfs, 1, None);
    admit(&mut live, &files.vfs, &[sum], 9);
    let (_, late_sum) = package(&mut files.vfs, "late.pk3", 10, b"downloaded");
    admit(&mut live, &files.vfs, &[late_sum], 10);
    admit(&mut fixed, &files.vfs, &[late_sum], 10);
    assert_eq!(
        read(&files.vfs, &live, b"model.bin").unwrap(),
        b"downloaded"
    );
    assert!(read(&files.vfs, &fixed, b"model.bin").is_none());
    // Existence probes see the original namespace, not the read policy.
    assert!(
        files
            .vfs
            .read_view(Some(fixed.state.file_search()))
            .query(b"model.bin")
            .is_some()
    );
    let source = live.state.file_source();
    assert!(live.admit_q3_files(&files.vfs, 10, Random::new(1)).is_err());
    live.startup.connecting(Protocol::Quake3_68);
    live.startup.q3_gamestate();
    assert!(
        live.admit_q3_files(&Vfs::default(), 10, Random::new(1))
            .is_err()
    );
    assert_eq!(live.state.file_source(), source);
    assert!(live.startup.q3() == Some(startup::Q3Phase::Downloading));
}

#[test]
fn renderer_reset_waits_for_native_reliable_room_then_reports_reloaded_modules() {
    use qa_core::{loopback::Endpoint, sys_events::EventTime};
    use qa_network::{
        channel::{Channel, commands::CommandContext},
        commands::{
            connection::Commands,
            packet::{self, Record},
        },
    };
    let mut files = support::Fixture::new(&[]);
    let (_, checksum) = package(&mut files.vfs, "pak0.pk3", 0, b"module");
    let mut client = client(&files.vfs, 0, None);
    admit(&mut client, &files.vfs, &[checksum], 37);
    client
        .state
        .configs
        .set(
            ConfigLayout::Q3_SYSTEM_INFO,
            b"\\sv_pure\\1\\sv_serverid\\7",
        )
        .unwrap();
    assert!(read(&files.vfs, &client, b"vm/cgame.qvm").is_some());
    assert!(read(&files.vfs, &client, b"vm/ui.qvm").is_some());
    let original = report(&files.vfs, &client);
    let source = client.state.file_source();
    client.startup.q3_primed();
    client.startup.q3_snapshot(0);
    client.begin_q3_renderer_reset(&files.vfs).unwrap();
    assert!(matches!(
        client.startup.q3(),
        Some(startup::Q3Phase::RendererReset)
    ));
    assert!(!client.startup.allows_commands());
    assert_eq!(client.state.file_source(), source);
    let mut channel =
        Channel::load(Protocol::Quake3_68.channel(), Endpoint::Client, 8192, 16).unwrap();
    channel
        .set_command_context(CommandContext {
            server_id: 7,
            checksum_feed: 37,
            ..Default::default()
        })
        .unwrap();
    for _ in 0..64 {
        channel
            .queue_client_command(Protocol::Quake3_68, b"x")
            .unwrap();
    }
    assert!(!client.queue_q3_renderer_reset(&mut channel).unwrap());
    assert_eq!(channel.command_state().unwrap().queued, 64);
    assert!(matches!(
        client.startup.q3(),
        Some(startup::Q3Phase::RendererReset)
    ));
    let codec = Commands::load(Protocol::Quake3_68, 32768);
    let mut bytes = [0; 32768];
    let n = codec.encode_control(&channel, &mut bytes).unwrap();
    channel
        .prepare_move(&bytes[..n], EventTime(0), None, None)
        .unwrap()
        .unwrap();
    let message = channel.send_state().sequence;
    channel.submitted(EventTime(0)).unwrap();
    assert_eq!(channel.command_state().unwrap().submitted, 64);
    // The channel ACK parser has separate wire tests. This unit supplies its
    // validated ACK result to isolate the startup transition under pressure.
    channel.acknowledge_commands(64, message).unwrap();
    assert!(client.queue_q3_renderer_reset(&mut channel).unwrap());
    assert!(matches!(
        client.startup.q3(),
        Some(startup::Q3Phase::Loading)
    ));
    assert!(!client.startup.allows_commands());
    assert!(
        client
            .queue_q3_references(&files.vfs, &mut channel)
            .is_err()
    );
    assert_eq!(channel.command_state().unwrap().queued, 65);
    assert!(read(&files.vfs, &client, b"vm/cgame.qvm").is_some());
    assert!(
        client
            .queue_q3_references(&files.vfs, &mut channel)
            .is_err()
    );
    assert!(read(&files.vfs, &client, b"vm/ui.qvm").is_some());
    assert!(
        client
            .queue_q3_references(&files.vfs, &mut channel)
            .unwrap()
    );
    let n = codec.encode_control(&channel, &mut bytes).unwrap();
    let mut queued = Vec::new();
    packet::read_with_commands(
        Protocol::Quake3_68,
        &mut bytes[..n],
        0,
        channel.command_key(None).unwrap(),
        |record| {
            if let Record::String { sequence, text } = record {
                queued.push((sequence, text.to_vec()));
            }
            Ok(std::ops::ControlFlow::Continue(()))
        },
    )
    .unwrap();
    assert_eq!(
        queued,
        [
            (Some(65), b"vdr".to_vec()),
            (Some(66), original.into_bytes())
        ]
    );
    client.startup.q3_primed();
    assert!(client.startup.allows_commands());
}

#[test]
fn renderer_reset_drains_the_existing_queue_while_the_healthy_peer_and_ticks_continue() {
    use crate::{
        Runtime,
        client_policy::ClientPolicy,
        host::{FrameHost, FrameSource},
        map::SpawnAnchor,
        modules::{Driver, RendererRestart},
    };
    use qa_core::{
        loopback::Endpoint,
        primitives::{ClientId, EntityId, ModuleId},
        sys_events::{EventKind, EventTime, SeatId, SysEvent, SysEventQueue},
    };
    use qa_session::{clients::Connection, timing::TickRate};
    use std::time::Duration;
    struct Source(u64);
    impl FrameSource for Source {
        fn begin_frame(&mut self) -> Duration {
            Duration::from_millis(self.0)
        }
        fn poll_events(&mut self, queue: &mut SysEventQueue) {
            queue
                .push(SysEvent {
                    time: EventTime::from_milliseconds(self.0),
                    kind: EventKind::Time,
                })
                .unwrap();
        }
        fn wait_time(&mut self, remaining: Duration) -> Duration {
            self.0 += remaining.as_millis().max(1) as u64;
            Duration::from_millis(self.0)
        }
        fn elapsed(&self) -> Duration {
            Duration::ZERO
        }
        fn present(&mut self) {}
        fn reset_presentation(&mut self, _: ClientId, _: EntityId) -> bool {
            true
        }
    }
    let mut files = support::Fixture::new(&[]);
    let (_, checksum) = package(&mut files.vfs, "pak0.pk3", 0, b"module");
    let mut runtime = Runtime::load(2, []).unwrap();
    runtime.vfs = std::mem::take(&mut files.vfs);
    let id = runtime
        .connect_local(
            SeatId::FIRST,
            SpawnAnchor {
                position: Default::default(),
                angles: Default::default(),
                entity: 0,
                fixture_fallback: false,
            },
            ClientPolicy {
                input: RuleSetId::Quake3,
                client: RuleSetId::Quake3,
                movement: RuleSetId::Quake,
                trace: RuleSetId::Quake2,
            },
            Protocol::Quake3_68,
            0,
        )
        .unwrap();
    let healthy = runtime
        .server
        .connect(Connection::Bot, ModuleId(2), None)
        .unwrap();
    let resources = runtime.client_resources[id.0 as usize].as_mut().unwrap();
    admit(resources, &runtime.vfs, &[checksum], 0);
    resources
        .state
        .configs
        .set(
            ConfigLayout::Q3_SERVER_INFO,
            b"\\mapname\\waiting-for-media",
        )
        .unwrap();
    assert!(read(&runtime.vfs, resources, b"vm/cgame.qvm").is_some());
    assert!(read(&runtime.vfs, resources, b"vm/ui.qvm").is_some());
    resources.startup.q3_primed();
    resources.startup.q3_snapshot(0);
    let channel = &mut runtime
        .network
        .get_mut(id, Endpoint::Client)
        .unwrap()
        .channel;
    for _ in 0..64 {
        channel
            .queue_client_command(Protocol::Quake3_68, b"x")
            .unwrap();
    }
    let mut host = Box::new(
        FrameHost::load(
            qa_console::commands::Console::new(Default::default()).unwrap(),
            runtime,
            TickRate::fixed(50).unwrap(),
            vec![],
        )
        .unwrap(),
    );
    host.local_clients[0] = Some(id);
    host.restart_client_modules(RendererRestart::Video, &mut Driver::cache_only())
        .unwrap();
    assert!(matches!(
        host.runtime.client_resources[id.0 as usize]
            .as_ref()
            .unwrap()
            .startup
            .q3(),
        Some(startup::Q3Phase::RendererReset)
    ));
    host.frame(&mut Source(0), true);
    assert!(matches!(
        host.runtime.client_resources[id.0 as usize]
            .as_ref()
            .unwrap()
            .startup
            .q3(),
        Some(startup::Q3Phase::RendererReset)
    ));
    let before = host.runtime.server.world_frame;
    for time in [50, 100, 150, 200] {
        host.frame(&mut Source(time), true);
    }
    assert!(host.runtime.server.world_frame > before);
    assert!(host.runtime.server.clients[healthy.0 as usize].is_active());
    assert!(host.runtime.server.clients[id.0 as usize].is_active());
    assert!(matches!(
        host.runtime.client_resources[id.0 as usize]
            .as_ref()
            .unwrap()
            .startup
            .q3(),
        Some(startup::Q3Phase::Loading)
    ));
    let client = &host
        .runtime
        .network
        .get(id, Endpoint::Client)
        .unwrap()
        .channel;
    let state = client.command_state().unwrap();
    // Initial userinfo retries behind the reset in this same native window.
    assert_eq!(state.queued, 66);
    assert!(state.acknowledged >= 65);
    let server = &host
        .runtime
        .network
        .get(id, Endpoint::Server)
        .unwrap()
        .channel;
    let (sequence, text) = server.received_command().unwrap();
    assert_eq!(sequence, 66);
    assert!(text.starts_with(b"userinfo "));
}
