//! Headless end-to-end test: stub map load, spawn, N server ticks with
//! synthetic client input, and demo encode/decode round-trips through
//! `qa-net`. No window, no sockets, no content files.

use qa_app::application::Application;
use qa_app::options::{parse_application_command, ApplicationCommand};
use qa_app::startup::StartupConfig;
use qa_client::render::NullRenderer;
use qa_net::demo::{
    encode_q3_demo, finish_q2_demo, read_q2_demo, write_q2_demo_record, Q3DemoMessage, Q3DemoRead, Q3DemoReader,
};

/// Headless frames to run.
const FRAMES: u64 = 32;

fn test_config() -> StartupConfig {
    let argv: Vec<String> = ["--movement", "q3", "--seed", "1234", "--frames", "32", "--seats", "2"]
        .iter()
        .map(|word| (*word).to_string())
        .collect();
    let options = match parse_application_command(&argv).unwrap() {
        ApplicationCommand::Run { options } => options,
        other => panic!("expected run, got {other:?}"),
    };
    StartupConfig::from_options(&options).unwrap()
}

fn frame_payload(frame: u64, origin: [f32; 3]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(16);
    payload.extend_from_slice(&(frame as u32).to_le_bytes());
    for component in origin {
        payload.extend_from_slice(&component.to_le_bytes());
    }
    payload
}

#[test]
fn headless_stub_map_spawn_tick_and_demo_round_trip() {
    let config = test_config();
    let mut application = Application::open(&config, NullRenderer::new()).unwrap();

    // Stub map load: worldspawn plus four player starts.
    assert_eq!(application.server().simulation().actor_count(), 5);
    assert_eq!(application.seats().len(), 2);
    for seat in application.seats() {
        assert_ne!(seat.slot, qa_app::UNBOUND_SEAT);
    }

    // Run N ticks with synthetic client input, capturing per-frame origins.
    let mut payloads: Vec<Vec<u8>> = Vec::with_capacity(FRAMES as usize);
    for _ in 0..FRAMES {
        let outcome = application.step_frame().unwrap();
        assert!(outcome.server_frames >= 1);
        let seat = &application.seats()[0];
        let actor = application.server().simulation().actor_by_slot(seat.slot).unwrap();
        let state = application.server().simulation().body_state(actor.id()).unwrap();
        assert!(state.origin.x.is_finite() && state.origin.y.is_finite() && state.origin.z.is_finite());
        payloads.push(frame_payload(
            outcome.frame,
            [state.origin.x, state.origin.y, state.origin.z],
        ));
    }
    assert!(application.is_finished());
    assert_eq!(application.frames(), FRAMES);
    // Q3 fixed 50ms step with a 50ms host step: exactly one tick per frame.
    assert_eq!(application.ticks(), FRAMES);
    assert_eq!(application.seats()[0].current_number(), FRAMES as i32);
    assert_eq!(application.renderer().frames(), FRAMES);
    assert_eq!(application.server().simulation().actor_count(), 5);

    // Entity state sanity: every body has a finite origin inside the world.
    for actor in application.server().simulation().body_actors() {
        let state = application.server().simulation().body_state(&actor).unwrap();
        assert!(state.origin.x.is_finite() && state.origin.y.is_finite() && state.origin.z.is_finite());
        assert!(state.origin.x.abs() <= 4096.0 && state.origin.y.abs() <= 4096.0 && state.origin.z.abs() <= 4096.0);
    }

    // Q2 demo round-trip: one record per frame plus the terminator.
    let mut stream = Vec::new();
    for payload in &payloads {
        stream.extend_from_slice(&write_q2_demo_record(payload));
    }
    stream.extend_from_slice(&finish_q2_demo());
    let records = read_q2_demo(&stream).unwrap();
    assert_eq!(records.len(), FRAMES as usize);
    for (record, payload) in records.iter().zip(payloads.iter()) {
        assert_eq!(&record.bytes, payload);
    }
    let mut offset = 0;
    for record in &records {
        assert_eq!(record.offset, offset);
        offset += 4 + record.bytes.len();
    }

    // Q3 demo round-trip: sequenced messages with the same payloads.
    let messages: Vec<Q3DemoMessage> = payloads
        .iter()
        .enumerate()
        .map(|(index, payload)| Q3DemoMessage {
            sequence: index as i32,
            payload: payload.clone(),
        })
        .collect();
    let encoded = encode_q3_demo(&messages).unwrap();
    let mut reader = Q3DemoReader::new(&encoded);
    let mut seen = 0;
    let mut sequences = Vec::new();
    loop {
        match reader.next(|sequence| sequences.push(sequence)).unwrap() {
            Q3DemoRead::Message(message) => {
                assert_eq!(message, messages[seen]);
                seen += 1;
            }
            Q3DemoRead::End(end) => {
                assert_eq!(end.reason, qa_net::demo::Q3DemoEndReason::Terminator);
                break;
            }
        }
    }
    assert_eq!(seen, FRAMES as usize);
    assert_eq!(sequences.len(), FRAMES as usize + 1);
}
