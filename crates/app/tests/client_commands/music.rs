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
        if let MusicControl::Start { owner, .. } = control {
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
pub(super) fn native_music_reaches_shared_paint(program: impl Fn() -> Program) {
    let mut fixture = fixture::Fixture::new();
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
        music: qa_app::audio::music::Music::default(),
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
    assert_eq!(
        source.pcm,
        [625, -625, 1250, -1250, 1875, -1875, 2500, -2500]
    );
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
    println!("native background controls reached ring, shared source stream and mixer");
}
