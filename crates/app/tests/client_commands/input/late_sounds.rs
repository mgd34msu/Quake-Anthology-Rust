use super::*;
use qa_audio::{Bank, Mixer};

#[path = "../../support/sound_registration.rs"]
#[allow(dead_code)]
mod samples;

const HANDLE: u32 = 80;
const NESTED_HANDLE: u32 = 84;
const AFTER_KEY: u32 = 88;
const NESTED_CALLS: u32 = 92;
const NAME: u32 = 608;
const NESTED_NAME: u32 = 656;
const COMMAND: u32 = 704;

fn register_and_play(ui: bool, name: u32, handle: u32) -> Vec<Op> {
    vec![
        call(
            if ui { 31 } else { 34 },
            &[Value::Pointer(name), Value::Constant(0)],
            Some(handle),
        ),
        call(
            if ui { 32 } else { 29 },
            &[Value::Word(handle), Value::Constant(6)],
            None,
        ),
    ]
}

struct SoundReceipt {
    sound: SoundId,
    source_ordinal: Option<u32>,
    listener_owner: Option<ModuleId>,
    client: Option<ClientId>,
}

struct Audio {
    input: super::Source,
    mixer: Mixer,
    played: Vec<SoundReceipt>,
}
impl FrameSource for Audio {
    fn begin_frame(&mut self) -> Duration {
        self.input.begin_frame()
    }
    fn poll_events(&mut self, queue: &mut SysEventQueue) {
        self.input.poll_events(queue);
    }
    fn wait_time(&mut self, span: Duration) -> Duration {
        self.input.wait_time(span)
    }
    fn elapsed(&self) -> Duration {
        self.input.elapsed()
    }
    fn present(&mut self) {}
    fn sound(&mut self, bank: Option<&Bank>, event: SoundEvent) -> bool {
        if event.action == SoundAction::Play {
            self.played.push(SoundReceipt {
                sound: event.sound,
                source_ordinal: event.source_ordinal,
                listener_owner: event.listener_owner,
                client: event.client,
            });
        }
        self.mixer.sound(bank.unwrap(), event)
    }
}

fn seed(host: &mut FrameHost, seat: usize, ui: bool) {
    for (offset, text) in [
        (NAME, b"sound/late.wav\0".as_slice()),
        (NESTED_NAME, b"sound/nested.wav\0"),
        (COMMAND, b"late_sounds\n\0"),
    ] {
        let mut bytes = [0u8; 32];
        bytes[..text.len()].copy_from_slice(text);
        for (index, bytes) in bytes.chunks_exact(4).enumerate() {
            write(host, seat, ui, offset + index as u32 * 4, word(bytes));
        }
    }
}
fn dispatch(host: &mut FrameHost, audio: &mut Audio) {
    let counts = qa_app::output::dispatch(
        &mut host.runtime,
        audio,
        &mut host.console.scrollback,
        &host.local_clients,
        host.time,
        3.,
        2.,
    );
    assert_eq!(counts.unhandled_sounds, 0);
}
fn audible(host: &FrameHost, audio: &mut Audio) {
    let mut pcm = [0; 16];
    assert!(
        audio
            .mixer
            .paint(host.runtime.sound_bank().unwrap(), &mut pcm)
    );
    assert!(pcm.iter().any(|&sample| sample != 0));
}

pub(super) fn run(native: bool) {
    let mut host = fixture_config(
        native,
        |seat, ui, bodies| {
            let mut ordinary = register_and_play(ui, NAME, HANDLE);
            ordinary.push(Op::Return(Value::Word(HANDLE)));
            bodies[30] = ordinary;
            if seat == 0 && ui {
                let mut key = register_and_play(true, NAME, HANDLE);
                key.extend([
                    call(12, &[Value::Constant(0), Value::Pointer(COMMAND)], None),
                    Op::Increment(AFTER_KEY),
                    Op::Return(Value::Constant(0)),
                ]);
                bodies[3] = key;
                let mut nested = register_and_play(true, NESTED_NAME, NESTED_HANDLE);
                nested.extend([Op::Increment(NESTED_CALLS), Op::Return(Value::Constant(1))]);
                bodies[8] = nested;
            }
            vec![]
        },
        |_| {},
    );
    let mut files = model_support::Fixture::new(&[
        ("sound/feedback/hit.wav", samples::wav()),
        ("sound/late.wav", samples::wav()),
        ("sound/nested.wav", samples::wav()),
    ]);
    host.runtime.vfs = std::mem::take(&mut files.vfs);
    qa_app::audio::begin_registration(
        &mut host,
        &[String::from_utf8(Bank::DEFAULT_SOUND.to_vec()).unwrap()],
        RuleSetId::Quake3,
    )
    .unwrap();
    qa_app::audio::finish_registration(&mut host).unwrap();
    let owner = host.runtime.sound_assets.as_ref().unwrap();
    assert!(!owner.initial_registration());
    assert!(owner.bank().find("sound/late.wav").is_none());
    assert!(owner.bank().find("sound/nested.wav").is_none());
    let baseline = (owner.reads, owner.decodes, owner.bank().len());
    let mut audio = Audio {
        input: super::Source::default(),
        mixer: Mixer::load(owner.bank().rate, 16, SeatId::COUNT),
        played: Vec::new(),
    };
    dispatch(&mut host, &mut audio);
    for seat in 0..2 {
        for ui in [true, false] {
            seed(&mut host, seat, ui);
        }
    }
    catch(&mut host, 0, true, key_catchers::UI);
    audio.input.time = 100;
    audio
        .input
        .pending
        .extend([vec![key(DeviceId::Keyboard(10), 4, true, false)], vec![]]);
    let frame = host.frame(&mut audio, true);
    assert_eq!(frame.drains, 2);
    assert_eq!(frame.output.unhandled_sounds, 0);
    assert_eq!(read(&mut host, 0, true, AFTER_KEY), 1);
    assert_eq!(read(&mut host, 0, true, NESTED_CALLS), 1);
    let handle = read(&mut host, 0, true, HANDLE);
    let nested_handle = read(&mut host, 0, true, NESTED_HANDLE);
    assert!(handle > 0 && nested_handle > 0 && handle != nested_handle);
    let owner = host.runtime.sound_assets.as_ref().unwrap();
    let sound = owner.bank().registered_sound(handle as i32).unwrap();
    let nested = owner.bank().registered_sound(nested_handle as i32).unwrap();
    assert!(sound.0 as usize >= baseline.2);
    assert_eq!(owner.bank().find("sound/late.wav"), Some(sound));
    assert_eq!(owner.bank().find("sound/nested.wav"), Some(nested));
    assert_eq!(
        (owner.reads, owner.decodes),
        (baseline.0 + 2, baseline.1 + 2)
    );
    assert_eq!(audio.played.len(), 2);
    assert_eq!(
        [audio.played[0].sound, audio.played[1].sound],
        [sound, nested]
    );
    assert_eq!(audio.played[0].source_ordinal, Some(handle));
    audible(&host, &mut audio);
    let retained = (owner.reads, owner.decodes, owner.bank().len());
    files.remove("sound/late.wav");
    files.remove("sound/nested.wav");
    for seat in 0..2 {
        for ui in [true, false] {
            let played = audio.played.len();
            assert_eq!(invoke(&mut host, seat, ui, 30, &[]), handle);
            dispatch(&mut host, &mut audio);
            assert_eq!(audio.played.len(), played + 1);
            let event = audio.played.last().unwrap();
            assert_eq!(event.sound, sound);
            assert_eq!(event.source_ordinal, Some(handle));
            assert_eq!(event.listener_owner, Some(module(seat, ui)));
            if !ui {
                assert_eq!(event.client, Some(ClientId(seat as u32)));
            }
            audible(&host, &mut audio);
            let owner = host.runtime.sound_assets.as_ref().unwrap();
            assert_eq!((owner.reads, owner.decodes, owner.bank().len()), retained);
        }
    }
    for id in 1..=5 {
        assert_eq!(host.module_counts(ModuleId(id)).unwrap().traps, 0);
    }
    println!("late shared sound registration, nested console and mixed PCM: native={native}");
}
