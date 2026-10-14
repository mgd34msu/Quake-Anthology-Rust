use super::*;
#[path = "../music.rs"]
#[allow(dead_code)]
mod fixture;

struct Source {
    mixer: qa_audio::Mixer,
    music: qa_app::audio::music::Music,
    owner: Option<MusicOwner>,
    time: EventTime,
    updates: usize,
    pcm: [i16; 8],
}
impl qa_app::host::FrameSource for Source {
    fn begin_frame(&mut self) -> Duration {
        Duration::from_nanos(self.time.0)
    }
    fn poll_events(&mut self, queue: &mut SysEventQueue) {
        queue
            .push(SysEvent {
                time: self.time,
                kind: EventKind::Time,
            })
            .unwrap();
    }
    fn wait_time(&mut self, remaining: Duration) -> Duration {
        self.time.0 += remaining.as_nanos() as u64;
        Duration::from_nanos(self.time.0)
    }
    fn elapsed(&self) -> Duration {
        Duration::ZERO
    }
    fn present(&mut self) {}
    fn sound(&mut self, event: SoundEvent) -> bool {
        self.music.sound_control(&mut self.mixer, event);
        self.mixer.sound(event)
    }
    fn music(
        &mut self,
        vfs: &qa_content::vfs::Vfs,
        control: MusicControl,
        intro: Option<&[u8]>,
        loop_track: Option<&[u8]>,
    ) -> bool {
        if let MusicControl::Start { owner, .. } | MusicControl::Track { owner, .. } = control {
            self.owner = Some(owner);
        }
        self.music
            .control(vfs, &mut self.mixer, control, intro, loop_track)
    }
    fn update_music(&mut self, vfs: &qa_content::vfs::Vfs, volume: f32) {
        self.updates += 1;
        self.music.update(vfs, &mut self.mixer, volume);
    }
    fn mix_audio(&mut self, _: &[Option<qa_app::client_scene::ClientEntities<'_>>; SeatId::COUNT]) {
        assert!(self.mixer.paint(&mut self.pcm));
    }
}

pub(super) fn wire_music_reaches_shared_paint() {
    for protocol in [
        Protocol::NetQuake15,
        Protocol::QuakeWorld28,
        Protocol::Quake2_34,
        Protocol::Quake2Repro1038,
    ] {
        let mut fixture = fixture::Fixture::new();
        fixture.numbered();
        let mut runtime = Runtime::load(2, []).unwrap();
        let id = runtime
            .connect_local(
                SeatId::FIRST,
                SpawnAnchor {
                    position: Vec3::default(),
                    angles: Vec3::default(),
                    entity: 0,
                    fixture_fallback: false,
                },
                ClientPolicy::select(
                    Some(RuleSetId::Quake3),
                    None,
                    Some(RuleSetId::QuakeWorld),
                    None,
                )
                .unwrap(),
                protocol,
                0,
            )
            .unwrap();
        runtime.vfs = std::mem::take(&mut fixture.vfs);
        let mut host = FrameHost::load(
            Console::new(Context::default()).unwrap(),
            runtime,
            TickRate::fixed(50).unwrap(),
            vec![],
        )
        .unwrap();
        host.local_clients[0] = Some(id);
        let volume = host
            .console
            .cvars
            .bind("s_musicvolume", Context::default())
            .unwrap();
        host.console.cvars.force_write(volume, "1").unwrap();
        let mut source = Source {
            mixer: fixture::mixer(),
            music: qa_app::audio::music::Music::load(&host.runtime.vfs),
            owner: None,
            time: EventTime::from_milliseconds(100),
            updates: 0,
            pcm: [0; 8],
        };
        for millis in [0, 50] {
            source.time = EventTime::from_milliseconds(millis);
            host.frame(&mut source, true);
        }
        source.time = EventTime::from_milliseconds(100);
        let send = |host: &mut FrameHost, payload: &[u8]| {
            let connection = host.runtime.network.get_mut(id, Endpoint::Server).unwrap();
            connection.channel.queue_reliable(payload).unwrap();
        };
        let mut bytes = [0; 96];
        let n = qa_network::outputs::cd_track(
            protocol,
            qa_network::outputs::CdTrack {
                track: 2,
                loop_track: Some(255),
            },
            &mut bytes,
        )
        .unwrap();
        send(&mut host, &bytes[..n]);
        let frame = host.frame(&mut source, true);
        assert_eq!(frame.output.music, 1, "{protocol:?}");
        assert_eq!(frame.output.unhandled_music, 0);
        assert_eq!(
            source.owner,
            Some(MusicOwner {
                module: None,
                client: Some((id, host.runtime.server.clients[id.0 as usize].entity))
            })
        );
        let mono = qa_formats::sound::vorbis(include_bytes!(
            "../../../formats/tests/fixtures/sounds/mono.ogg"
        ))
        .unwrap();
        assert_eq!(source.pcm, std::array::from_fn(|i| mono.samples[i / 2]));
        // Native named soundtrack rows also use the same leased music path.
        if matches!(protocol, Protocol::Quake2_34 | Protocol::Quake2Repro1038) {
            source.time = EventTime::from_milliseconds(200);
            let row = protocol.config_layout().unwrap().cd_track.unwrap() as u32;
            let n =
                qa_network::outputs::configstring(protocol, row, b"music/intro.wav", &mut bytes)
                    .unwrap();
            send(&mut host, &bytes[..n]);
            let frame = host.frame(&mut source, true);
            assert_eq!(frame.output.music, 1);
            assert_eq!(
                source.pcm,
                [1000, -1000, 2000, -2000, 1000, -1000, 2000, -2000]
            );
        }
        source.time = EventTime::from_milliseconds(300);
        let n = qa_network::outputs::cd_track(
            protocol,
            qa_network::outputs::CdTrack {
                track: 0,
                loop_track: None,
            },
            &mut bytes,
        )
        .unwrap();
        send(&mut host, &bytes[..n]);
        let frame = host.frame(&mut source, true);
        assert_eq!(frame.output.music, 1);
        assert_eq!(source.pcm, [0; 8]);
        assert!(!source.music.playing());
        assert_eq!(source.music.counts.invalid, 0);
        println!("native track packets reached ring and PCM: {protocol:?}");
    }
}
pub(super) fn native_music_reaches_shared_paint(program: impl Fn() -> Program, compressed: bool) {
    let mut fixture = fixture::Fixture::new();
    if compressed {
        fixture.archive();
    }
    let (mut host, clients) = host(program, 1);
    host.runtime.vfs = std::mem::take(&mut fixture.vfs);
    let volume = host
        .console
        .cvars
        .bind("s_musicvolume", Context::default())
        .unwrap();
    host.console.cvars.force_write(volume, "1").unwrap();
    let mut source = Source {
        mixer: fixture::mixer(),
        music: qa_app::audio::music::Music::load(&host.runtime.vfs),
        owner: None,
        time: EventTime::from_milliseconds(100),
        updates: 0,
        pcm: [0; 8],
    };
    assert_eq!(probe(&mut host, 0, 15, 0), 0);
    let result = host.frame(&mut source, true);
    assert_eq!(result.output.music, 1);
    assert_eq!(result.output.unhandled_music, 0);
    assert_eq!(
        source.owner,
        Some(MusicOwner {
            module: Some(ModuleId(1)),
            client: Some((clients[0], host.runtime.server.clients[0].entity))
        })
    );
    assert_eq!(source.updates, 1);
    if compressed {
        let pcm = qa_formats::sound::vorbis(include_bytes!(
            "../../../formats/tests/fixtures/sounds/mono.ogg"
        ))
        .unwrap();
        let mut reference = fixture::mixer();
        let format = qa_core::primitives::PcmFormat {
            rate: pcm.rate,
            channels: pcm.channels,
            width: PcmWidth::Short,
        };
        let bytes: Vec<_> = pcm
            .samples
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect();
        reference.queue_raw(format, &bytes, 0.625, false).unwrap();
        let mut expected = [0; 8];
        reference.paint(&mut expected);
        assert_eq!(source.pcm, expected);
    } else {
        assert_eq!(
            source.pcm,
            [625, -625, 1250, -1250, 1875, -1875, 2500, -2500]
        );
    }
    assert_eq!(source.music.counts.invalid, 0);
    assert_eq!(probe(&mut host, 0, 16, 0), 0);
    source.time.0 += 100_000_000;
    let result = host.frame(&mut source, true);
    assert_eq!(result.output.music, 1);
    assert_eq!(source.pcm, [0; 8]);
    assert_eq!(source.music.counts.stopped, 1);
    assert_eq!(probe(&mut host, 0, 15, 0), 0);
    source.time.0 += 100_000_000;
    host.frame(&mut source, true);
    assert!(source.music.playing());
    assert!(host.shutdown_module(ModuleId(1)));
    source.time.0 += 100_000_000;
    let result = host.frame(&mut source, true);
    assert_eq!(result.output.music, 1);
    assert_eq!(source.pcm, [0; 8]);
    assert_eq!(source.music.counts.stopped, 2);
    println!(
        "native background controls reached ring, shared source stream and mixer: vorbis={compressed}"
    );
}
