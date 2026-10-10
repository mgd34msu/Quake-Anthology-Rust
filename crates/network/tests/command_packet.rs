use qa_core::primitives::Vec3;
use qa_network::commands::{
    Q1Move,
    connection::Commands,
    packet::{self, Acknowledgements, Error, Key, Move, Protocol, ZERO_Q2, ZERO_Q3, ZERO_QW},
};

#[test]
fn qw_delta_requests_are_outside_crc_and_replace_per_message()
-> Result<(), Box<dyn std::error::Error>> {
    let mut bytes = [0; 128];
    let movement = Move::QuakeWorld {
        loss: 9,
        commands: [
            ZERO_QW,
            ZERO_QW,
            qa_network::commands::QwCmd {
                msec: 16,
                movement: [321, -123, 0],
                ..ZERO_QW
            },
        ],
        delta_request: Some(0),
    };
    let length = packet::write(&mut bytes, &movement, 17, Key::default())?;
    assert_eq!(&bytes[length - 2..length], &[5, 0]);
    assert_eq!(
        packet::read(
            Protocol::QuakeWorld28,
            &mut bytes[..length],
            17,
            Key::default()
        )?,
        movement
    );
    // Changing only the request cannot change the checksum or move result.
    bytes[length - 1] = 255;
    let Move::QuakeWorld {
        delta_request,
        commands,
        ..
    } = packet::read(
        Protocol::QuakeWorld28,
        &mut bytes[..length],
        17,
        Key::default(),
    )?
    else {
        return Err(Error::Opcode.into());
    };
    assert_eq!(delta_request, Some(255));
    assert_eq!(commands[2].movement, [321, -123, 0]);

    // Native controls may precede/follow the move; last delta byte wins.
    let mut message = vec![1, 5, 7, 1];
    message.extend_from_slice(&bytes[..length]);
    message.extend_from_slice(&[1, 5, 19, 5, 23, 1]);
    let mut channel = qa_network::channel::Channel::load(
        qa_network::channel::QUAKEWORLD,
        qa_core::loopback::Endpoint::Server,
        8192,
        8,
    )?;
    let mut connection = Commands::load(Protocol::QuakeWorld28);
    let n = connection.stage(&message)?;
    assert!(connection.decode(n, 17, 0, 0, &mut channel)?.is_some());
    assert_eq!(connection.delta_request(), Some(23));
    // The next message omits clc_delta and must not retain its predecessor.
    let n = connection.stage(&bytes[..length - 2])?;
    assert!(connection.decode(n, 17, 0, 0, &mut channel)?.is_some());
    assert_eq!(connection.delta_request(), None);
    let n = connection.stage(&message)?;
    connection.decode(n, 17, 0, 0, &mut channel)?;
    assert_eq!(connection.delta_request(), Some(23));
    message.push(5);
    let n = connection.stage(&message)?;
    assert!(connection.decode(n, 17, 0, 0, &mut channel).is_err());
    assert_eq!(connection.delta_request(), None);

    for end in 0..length - 2 {
        assert!(
            packet::read(
                Protocol::QuakeWorld28,
                &mut bytes[..end],
                17,
                Key::default()
            )
            .is_err()
        );
    }
    assert!(
        packet::read(
            Protocol::QuakeWorld28,
            &mut bytes[..length - 1],
            17,
            Key::default()
        )
        .is_err()
    );
    bytes[length] = 3; // A second move is never admitted.
    assert_eq!(
        packet::read(
            Protocol::QuakeWorld28,
            &mut bytes[..length + 1],
            17,
            Key::default()
        ),
        Err(Error::Opcode)
    );
    bytes[2] ^= 1; // Actual move bytes remain protected by the sequence CRC.
    assert_eq!(
        packet::read(
            Protocol::QuakeWorld28,
            &mut bytes[..length],
            17,
            Key::default()
        ),
        Err(Error::Checksum)
    );
    Ok(())
}

#[test]
fn netquake_15_uses_integer_angles_and_has_no_duration_field() -> Result<(), Error> {
    let mut data = [0; 128];
    let movement = Move::NetQuake {
        timestamp: 12.5,
        command: Q1Move {
            view_angles: Vec3([2.9, -2.9, 359.9]),
            movement: [32767, -32768, 123],
            buttons: 3,
            impulse: 255,
        },
    };
    let n = packet::write(&mut data, &movement, 999, Key::default())?;
    assert_eq!(n, 16);
    assert_eq!(&data[5..8], &[1, 255, 255]);
    let Move::NetQuake { timestamp, command } =
        packet::read(Protocol::NetQuake15, &mut data[..n], 777, Key::default())?
    else {
        return Err(Error::Opcode);
    };
    assert_eq!(timestamp, 12.5);
    assert_eq!(command.view_angles, Vec3([1.40625, -1.40625, -1.40625]));
    assert_eq!(command.movement, [32767, -32768, 123]);
    Ok(())
}

#[test]
fn native_sequence_crc_rejects_corruption_and_wrong_packet_sequence() -> Result<(), Error> {
    let qw = Move::QuakeWorld {
        loss: 77,
        delta_request: None,
        commands: [
            ZERO_QW,
            ZERO_QW,
            qa_network::commands::QwCmd {
                msec: 16,
                movement: [400, -400, 0],
                ..ZERO_QW
            },
        ],
    };
    let q2 = Move::Quake2 {
        last_frame: -1,
        commands: [
            ZERO_Q2,
            ZERO_Q2,
            qa_network::commands::Q2Cmd {
                msec: 16,
                movement: [400, -400, 0],
                light_level: 255,
                ..ZERO_Q2
            },
        ],
    };
    for movement in [qw, q2] {
        let mut data = [0; 128];
        let n = packet::write(&mut data, &movement, 123, Key::default())?;
        assert_eq!(
            packet::read(movement.protocol(), &mut data[..n], 123, Key::default())?,
            movement
        );
        assert_eq!(
            packet::read(movement.protocol(), &mut data[..n], 124, Key::default()),
            Err(Error::Checksum)
        );
        data[n - 1] ^= 1;
        assert_eq!(
            packet::read(movement.protocol(), &mut data[..n], 123, Key::default()),
            Err(Error::Checksum)
        );
    }
    Ok(())
}

#[test]
fn q3_uses_native_header_key_xor_and_validates_the_move_count() -> Result<(), Error> {
    let mut commands = [ZERO_Q3; 32];
    commands[0] = qa_network::commands::Q3Cmd {
        server_time: 123,
        angles: [16384, 32768, 65535],
        movement: [-127, 127, 0],
        buttons: 2049,
        weapon: 7,
    };
    commands[1] = qa_network::commands::Q3Cmd {
        server_time: 255,
        movement: [127, 0, -127],
        ..commands[0]
    };
    let movement = Move::Quake3 {
        commands,
        count: 2,
        delta: false,
    };
    let key = Key {
        challenge: 0x12345,
        checksum_feed: 0x8123ff42,
        acknowledgements: Acknowledgements {
            server_id: 12,
            message: 19,
            reliable: 3,
        },
        server_command: b"print \"100%\"",
    };
    let mut data = [0; 1400];
    let n = packet::write(&mut data, &movement, 123, key)?;
    assert_eq!(packet::acknowledgements(&data[..n])?, key.acknowledgements);
    let decoded = packet::read(Protocol::Quake3_68, &mut data[..n], 123, key)?;
    // Native MSG_ReadDeltaKey applies kbitmask[bits], including its extra bit.
    let Move::Quake3 {
        commands: decoded,
        count,
        delta,
    } = decoded
    else {
        return Err(Error::Opcode);
    };
    assert_eq!((count, delta), (2, false));
    assert_eq!(decoded[0].server_time, 123);
    assert_eq!(decoded[1].server_time, 255);
    assert_eq!(decoded[0].movement, [-127, 127, 0]);
    assert_eq!(decoded[1].movement, [127, 0, -127]);
    assert_eq!(
        packet::write(
            &mut data,
            &Move::Quake3 {
                commands,
                count: 0,
                delta: false
            },
            123,
            key
        ),
        Err(Error::Count)
    );
    Ok(())
}
