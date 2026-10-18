use super::*;
#[path = "../music.rs"]
#[allow(dead_code)]
mod fixture;

struct Source {
    bank: qa_audio::Bank,
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
    fn sound(&mut self, bank: Option<&qa_audio::Bank>, event: SoundEvent) -> bool {
        self.music.sound_control(&mut self.mixer, event);
        self.mixer.sound(bank.unwrap_or(&self.bank), event)
    }
    fn music(
        &mut self,
        vfs: &qa_content::vfs::Vfs,
        control: MusicControl,
        intro: Option<&[u8]>,
        loop_track: Option<&[u8]>,
        search: Option<&qa_content::vfs::SearchView>,
    ) -> bool {
        if let MusicControl::Start { owner, .. } | MusicControl::Track { owner, .. } = control {
            self.owner = Some(owner);
        }
        self.music
            .control(vfs, &mut self.mixer, control, intro, loop_track, search)
    }
    fn update_music(&mut self, vfs: &qa_content::vfs::Vfs, volume: f32) {
        self.updates += 1;
        self.music.update(vfs, &mut self.mixer, volume);
    }
    fn music_report(
        &self,
        vfs: &qa_content::vfs::Vfs,
        query: MusicQuery,
        texts: &mut qa_core::events::TextStore,
    ) -> Option<TextLease> {
        texts.insert_formatted(format_args!("{}", self.music.report(vfs, query)))
    }
    fn mix_audio(
        &mut self,
        bank: Option<&qa_audio::Bank>,
        _: &[Option<qa_app::client_scene::ClientEntities<'_>>; SeatId::COUNT],
    ) {
        assert!(self.mixer.paint(bank.unwrap_or(&self.bank), &mut self.pcm));
    }
}

pub(super) fn console_cd_reaches_music_and_ordered_console_responses() {
    let mut fixture = fixture::Fixture::new();
    fixture.numbered();
    let mut runtime = Runtime::load(1, []).unwrap();
    runtime.vfs = std::mem::take(&mut fixture.vfs);
    let context = Context {
        source: RuleSetId::Quake2,
        ..Default::default()
    };
    let mut host = FrameHost::load(
        Console::new(context).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![],
    )
    .unwrap();
    let volume = host
        .console
        .cvars
        .bind("s_musicvolume", Context::default())
        .unwrap();
    host.console.cvars.force_write(volume, "1").unwrap();
    let mut source = Source {
        bank: fixture::bank(),
        mixer: fixture::mixer(),
        music: qa_app::audio::music::Music::load(&host.runtime.vfs),
        owner: None,
        time: EventTime::from_milliseconds(100),
        updates: 0,
        pcm: [0; 8],
    };
    host.console
        .append(
            "cd remap 3 0 2\ncd loop 1\ncd info\necho between\ncd pause\ncd info\ncd remap\n",
            context,
        )
        .unwrap();
    let frame = host.frame(&mut source, true);
    assert_eq!(frame.output.music, 6);
    assert_eq!(frame.output.unhandled_music, 0);
    assert_eq!(frame.output.prints, 5);
    assert_eq!(source.pcm, [0; 8]);
    let printed: Vec<_> = host
        .console
        .scrollback
        .chunks(&host.runtime.server.events.texts)
        .flat_map(|bytes| bytes.iter().copied())
        .collect();
    assert_eq!(printed, b"Currently looping track 1 (mapped to 3)\nVolume is 1\nbetween \nPaused looping track 1 (mapped to 3)\nVolume is 1\n  1 -> 3\n  2 -> 0\n  3 -> 2\n");
    assert!(host.runtime.server.events.is_empty());
    assert_eq!(host.runtime.server.events.rejected_publications(), 0);
    host.console
        .append("cd resume\ncd info\n", context)
        .unwrap();
    source.time = EventTime::from_milliseconds(200);
    let frame = host.frame(&mut source, true);
    assert_eq!((frame.output.music, frame.output.prints), (2, 1));
    assert_eq!(
        source.pcm,
        [1000, -1000, 2000, -2000, 1000, -1000, 2000, -2000]
    );
    host.console
        .append(
            "cd off\nmusic mono.ogg\ncd info\ncd on\ncd reset\ncd eject\ncd close\n",
            context,
        )
        .unwrap();
    source.time = EventTime::from_milliseconds(300);
    let frame = host.frame(&mut source, true);
    assert_eq!(frame.output.unhandled_music, 0);
    assert_eq!(frame.output.prints, 3);
    assert_eq!(source.pcm, [0; 8]);
    let printed: Vec<_> = host
        .console
        .scrollback
        .chunks(&host.runtime.server.events.texts)
        .flat_map(|bytes| bytes.iter().copied())
        .collect();
    let text = String::from_utf8(printed).unwrap();
    assert!(text.contains("CD music is disabled.\nVolume is 1\ncd eject:"));
    assert!(
        text.contains("cd close: disc tray operations are unavailable with file-backed music.")
    );
    assert!(host.runtime.server.events.is_empty());
    println!("CD controls reached music and ordered ring-backed console responses");
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
        let mut conflicting = fixture::Fixture::new();
        conflicting.numbered();
        let high_root = conflicting.vfs.mounts().next().unwrap().path.clone();
        qa_platform::files::write(
            high_root.join("music/02.ogg"),
            include_bytes!("../../../formats/tests/fixtures/sounds/stereo.ogg"),
        )
        .unwrap();
        // numbered() publishes another mount containing the added tracks.
        // Capture both low-source mounts before adding the conflicting winner.
        let selected: Vec<_> = fixture.vfs.mounts().map(|mount| mount.id).collect();
        let view = fixture.vfs.search_view(selected.into_iter().rev()).unwrap();
        fixture.vfs.mount_directory(&high_root, 100).unwrap();
        let mut runtime = Runtime::load(2, []).unwrap();
        runtime.vfs = std::mem::take(&mut fixture.vfs);
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
                    None,
                )
                .unwrap(),
                protocol,
                0,
            )
            .unwrap();
        runtime.client_resources[id.0 as usize] = qa_app::client_resources::ClientResources::load(
            id,
            protocol,
            RuleSetId::Quake3,
            &runtime.vfs,
            Some(&view),
        )
        .unwrap();
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
            bank: fixture::bank(),
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
            source.time = EventTime::from_milliseconds(250);
            let n = qa_network::outputs::configstring(protocol, row, b"12-theme.ogg", &mut bytes)
                .unwrap();
            send(&mut host, &bytes[..n]);
            let frame = host.frame(&mut source, true);
            assert_eq!(frame.output.music, 1);
            assert_eq!(frame.output.unhandled_music, 0);
            assert_eq!(source.pcm, std::array::from_fn(|i| mono.samples[i / 2]));
            source.time = EventTime::from_milliseconds(275);
            let n = qa_network::outputs::configstring(
                protocol,
                row,
                b"\"music/intro theme.wav\" \"music/loop theme.wav\"",
                &mut bytes,
            )
            .unwrap();
            send(&mut host, &bytes[..n]);
            let frame = host.frame(&mut source, true);
            assert_eq!(frame.output.music, 1);
            assert_eq!(frame.output.unhandled_music, 0);
            assert_eq!(
                source.pcm,
                [1000, -1000, 2000, -2000, 3000, -3000, 4000, -4000]
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
    let (mut host, clients) = host_capacity(
        program,
        1,
        qa_core::command_history::DEFAULT_CAPACITY,
        Some(std::mem::take(&mut fixture.vfs)),
    );
    let volume = host
        .console
        .cvars
        .bind("s_musicvolume", Context::default())
        .unwrap();
    host.console.cvars.force_write(volume, "1").unwrap();
    let mut source = Source {
        bank: fixture::bank(),
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
        reference.paint(&source.bank, &mut expected);
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

#[path = "../support/sound_registration.rs"]
#[allow(dead_code)]
mod effect_wave;

struct SelectedMusic {
    _files: [fixture::Fixture; 2],
    host: FrameHost,
    source: Source,
    views: [qa_content::vfs::SearchView; 2],
}
impl SelectedMusic {
    fn load() -> Self {
        let mut low = fixture::Fixture::new();
        let high = fixture::Fixture::new();
        let high_root = high.vfs.mounts().next().unwrap().path.clone();
        let path = high_root.join("music/intro.wav");
        let mut wave = qa_platform::files::read(&path).unwrap();
        for sample in wave[44..].chunks_exact_mut(2) {
            sample.copy_from_slice(&(-7000i16).to_le_bytes());
        }
        qa_platform::files::write(path, wave).unwrap();
        let low_mount = low.vfs.mounts().next().unwrap().id;
        let low_root = low.vfs.mounts().next().unwrap().path.clone();
        for (root, value) in [(&low_root, 1024i16), (&high_root, -7000i16)] {
            let mut bytes = effect_wave::wav();
            for sample in bytes[44..].chunks_exact_mut(2) {
                sample.copy_from_slice(&value.to_le_bytes());
            }
            qa_platform::files::create_dir_all(root.join("sound")).unwrap();
            qa_platform::files::write(root.join("sound/tone.wav"), bytes).unwrap();
        }
        low.vfs
            .refresh_directory_prefix(low_mount, b"sound")
            .unwrap();
        let high_mount = low.vfs.mount_directory(&high_root, 20).unwrap();
        let views = [
            low.vfs.search_view([low_mount]).unwrap(),
            low.vfs.search_view([high_mount]).unwrap(),
        ];
        let mut runtime = Runtime::load(1, []).unwrap();
        runtime.vfs = std::mem::take(&mut low.vfs);
        runtime.sound_assets = Some(Box::new(qa_app::audio::SoundAssets::from_bank(
            &runtime.vfs,
            qa_audio::Bank::with_capacity(std::num::NonZeroU32::new(22050).unwrap(), 32, 4096)
                .unwrap(),
        )));
        let host = FrameHost::load(
            Console::new(Context::default()).unwrap(),
            runtime,
            TickRate::FrameDriven,
            vec![],
        )
        .unwrap();
        let source = Source {
            bank: fixture::bank(),
            mixer: fixture::mixer(),
            music: qa_app::audio::music::Music::load(&host.runtime.vfs),
            owner: None,
            time: EventTime::default(),
            updates: 0,
            pcm: [0; 8],
        };
        Self {
            _files: [low, high],
            host,
            source,
            views,
        }
    }
    fn dispatch(&mut self) -> qa_app::output::OutputCounts {
        qa_app::output::dispatch(
            &mut self.host.runtime,
            &mut self.source,
            &mut self.host.console.scrollback,
            &[None; SeatId::COUNT],
            EventTime::default(),
            3.,
            3.,
        )
    }
    fn first_sample(&mut self) -> i16 {
        self.source
            .music
            .update(&self.host.runtime.vfs, &mut self.source.mixer, 1.);
        let mut pcm = [0; 4];
        let bank = self
            .host
            .runtime
            .sound_assets
            .as_deref()
            .map(qa_app::audio::SoundAssets::bank)
            .unwrap_or(&self.source.bank);
        self.source.mixer.paint(bank, &mut pcm);
        pcm[0]
    }
}

pub(super) fn queued_native_music_keeps_its_file_selection_after_module_rebind() {
    use qa_compat::services::{ENGINE_CALLS, ServiceStorage};
    let mut fixture = SelectedMusic::load();
    let module = ModuleId(1);
    let mut storage = ServiceStorage::load_native(
        &[(module, 0, RuleSetId::Quake3)],
        0,
        &fixture.host.console.cvars,
    )
    .unwrap();
    storage.prepare_files(&fixture.host.runtime.vfs).unwrap();
    storage
        .set_file_search(
            module,
            Some(fixture.views[0].clone()),
            &fixture.host.runtime.vfs,
        )
        .unwrap();
    let original = storage.file_source(module).unwrap();
    let mut scratch = fixture.host.runtime.geometry.scratch();
    {
        let mut services = fixture.host.runtime.engine_services(
            &mut fixture.host.console,
            &mut storage,
            &mut scratch,
        );
        (ENGINE_CALLS.background)(
            &mut services,
            module,
            qa_gameplay::rules::music_playback(RuleSetId::Quake3),
            b"music/intro.wav",
            b"",
        )
        .unwrap();
    }
    storage
        .set_file_search(
            module,
            Some(fixture.views[1].clone()),
            &fixture.host.runtime.vfs,
        )
        .unwrap();
    assert_ne!(storage.file_source(module), Some(original));
    // A foreign handle must retire as a counted drop, preserving the valid
    // preceding request and the rest of this consumer's progress.
    let foreign = qa_content::vfs::Vfs::default();
    let invalid = foreign
        .bind_search(Some(&foreign.search_view([]).unwrap()))
        .unwrap();
    fixture
        .host
        .runtime
        .server
        .events
        .music(
            MusicOwner::default(),
            qa_gameplay::rules::music_playback(RuleSetId::Quake3),
            b"music/intro.wav",
            b"",
            invalid.id(),
        )
        .unwrap();
    let counts = fixture.dispatch();
    assert_eq!(counts.music, 2);
    assert_eq!(counts.unhandled_music, 1);
    assert!(fixture.first_sample() > 0);
    assert!(fixture.host.runtime.server.events.is_empty());
}

pub(super) fn deferred_console_music_keeps_the_contexts_selected_files() {
    let mut fixture = SelectedMusic::load();
    let low = fixture
        .host
        .runtime
        .vfs
        .bind_search(Some(&fixture.views[0]))
        .unwrap();
    let high = fixture
        .host
        .runtime
        .vfs
        .bind_search(Some(&fixture.views[1]))
        .unwrap();
    let context = Context {
        file_search: low.id(),
        ..Context::default()
    };
    fixture
        .host
        .console
        .append("music music/intro.wav\n", context)
        .unwrap();
    assert_ne!(context.file_search, high.id());
    fixture
        .host
        .console
        .execute_frame(&mut fixture.host.runtime);
    let counts = fixture.dispatch();
    assert_eq!(counts.music, 1);
    assert_eq!(counts.unhandled_music, 0);
    assert!(fixture.first_sample() > 0);
    assert!(fixture.host.runtime.server.events.is_empty());
}

pub(super) fn deferred_console_play_uses_the_selected_native_sound_namespace() {
    let mut fixture = SelectedMusic::load();
    let low = fixture
        .host
        .runtime
        .vfs
        .bind_search(Some(&fixture.views[0]))
        .unwrap();
    fixture
        .host
        .console
        .append(
            "play sound/tone.wav\n",
            Context {
                file_search: low.id(),
                ..Context::default()
            },
        )
        .unwrap();
    fixture
        .host
        .console
        .execute_frame(&mut fixture.host.runtime);
    let counts = fixture.dispatch();
    assert_eq!(counts.sounds, 1);
    assert_eq!(counts.unhandled_sounds, 0);
    assert!(fixture.first_sample() > 0);
}
