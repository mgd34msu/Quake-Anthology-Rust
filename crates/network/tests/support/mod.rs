use qa_core::sys_events::EventTime;
use qa_network::{
    channel::{Channel, Delivery},
    commands::{
        connection::Commands,
        packet::{self, Key, Move, Protocol, ZERO_QW},
    },
    headers::{self, Direction, Header},
};

/// Give a SERVER codec fixture an actual sequenced native QW request. Admission
/// runs the same checksum and command decoder that owns reply alignment.
pub fn admit_qw_reply(channel: &mut Channel, sequence: u32) {
    let mut body = [0; 1400];
    let length = packet::write(
        &mut body,
        &Move::QuakeWorld {
            loss: 0,
            commands: [ZERO_QW; 3],
            delta_request: None,
        },
        sequence,
        Key::default(),
    )
    .unwrap();
    let mut bytes = [0; 1400];
    let length = headers::encode(
        headers::QUAKEWORLD,
        Direction::ToServer,
        Header {
            sequence,
            ..Default::default()
        },
        &body[..length],
        &mut bytes,
    )
    .unwrap();
    let received = channel.receive(&bytes[..length], EventTime(0)).unwrap();
    let Delivery::Payload(body) = received.delivery else {
        panic!("native QW request")
    };
    let mut commands = Commands::load(Protocol::QuakeWorld28, 8192);
    let length = commands.stage(body).unwrap();
    commands
        .decode(length, sequence, 0, channel, None, |_| {
            std::ops::ControlFlow::Continue(())
        })
        .unwrap();
}
