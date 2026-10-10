//! Developer-only native snapshot byte/word comparison and heap/timing probe.
use qa_core::primitives::ThinkTime;
use qa_network::{
    message::{Encoding, Reader, Writer},
    snapshots::{self, Entity, Frame, Ring},
    states::{ENTITY_WORDS, PLAYER_WORDS},
};
use qa_platform::{Stopwatch, allocations};
use std::io::{Read, Write};

#[global_allocator]
static ALLOCATOR: allocations::CountingAllocator = allocations::CountingAllocator;

struct Case<const P: usize, const E: usize> {
    distance: u8,
    prime: bool,
    flags: u8,
    areas: Vec<u8>,
    old_player: [u32; P],
    player: [u32; P],
    old: Vec<Entity<E>>,
    entities: Vec<Entity<E>>,
    server: Ring<P, E>,
    client: Ring<P, E>,
    expected: Vec<u8>,
}
type WriteFrame<const P: usize, const E: usize> =
    fn(
        &mut Writer<'_>,
        &Ring<P, E>,
        u32,
        Option<u32>,
    ) -> Result<(), qa_network::commands::packet::Error>;
type ReadFrame<const P: usize, const E: usize> =
    fn(
        &mut Reader<'_>,
        &mut Ring<P, E>,
        u32,
        u32,
        u32,
        Option<u32>,
    ) -> Result<bool, qa_network::commands::packet::Error>;
struct Codec<const P: usize, const E: usize> {
    encoding: Encoding,
    opcodes: [u32; 2],
    end: u32,
    write: WriteFrame<P, E>,
    read: ReadFrame<P, E>,
}
const Q3: Codec<PLAYER_WORDS, ENTITY_WORDS> = Codec {
    encoding: Encoding::Q3,
    opcodes: [7, 7],
    end: 8,
    write: snapshots::write_q3,
    read: |r, s, n, c, _, _| snapshots::read_q3(r, s, n, c),
};
const Q2: Codec<{ qa_network::states::Q2_PLAYER_WORDS }, { qa_network::states::Q2_ENTITY_WORDS }> =
    Codec {
        encoding: Encoding::Bytes,
        opcodes: [20, 20],
        end: 6,
        write: |w, r, n, d| snapshots::write_q2(w, r, n, d, 16),
        read: |r, s, _, _, _, _| snapshots::read_q2(r, s),
    };
const QW: Codec<0, { qa_network::states::QW_ENTITY_WORDS }> = Codec {
    encoding: Encoding::Bytes,
    opcodes: [47, 48],
    end: 6,
    write: |w, r, n, d| snapshots::write_qw(w, r, n, d.map(|base| (base, base as u8))),
    read: |r, s, n, _, opcode, request| snapshots::read_qw(r, s, n, opcode == 48, request, n + 1),
};
fn words<const N: usize>(reader: &mut Reader<'_>) -> Result<[u32; N], String> {
    let mut words = [0; N];
    for word in &mut words {
        *word = reader.read_bits(32).map_err(|e| e.to_string())?;
    }
    Ok(words)
}
fn entities<const E: usize>(
    reader: &mut Reader<'_>,
    count: usize,
) -> Result<Vec<Entity<E>>, String> {
    (0..count)
        .map(|_| {
            Ok(Entity {
                number: reader.read_bits(16).map_err(|e| e.to_string())?,
                words: words(reader)?,
            })
        })
        .collect()
}
fn load<const P: usize, const E: usize>(
    input: &[u8],
    retained: Option<u64>,
) -> Result<Vec<Case<P, E>>, String> {
    let mut reader = Reader::new(input, Encoding::Bytes);
    let count = reader.read_bits(32).map_err(|e| e.to_string())?;
    let mut cases = Vec::new();
    for _ in 0..count {
        let distance = reader.read_bits(8).map_err(|e| e.to_string())? as u8;
        let prime = reader.read_bits(8).map_err(|e| e.to_string())? != 0;
        let flags = reader.read_bits(8).map_err(|e| e.to_string())? as u8;
        let mut areas = vec![0; reader.read_bits(8).map_err(|e| e.to_string())? as usize];
        let a = reader.read_bits(16).map_err(|e| e.to_string())? as usize;
        let b = reader.read_bits(16).map_err(|e| e.to_string())? as usize;
        let baseline_count = reader.read_bits(16).map_err(|e| e.to_string())? as usize;
        let old_player = words(&mut reader)?;
        let player = words(&mut reader)?;
        reader.read_data(&mut areas).map_err(|e| e.to_string())?;
        let baselines = entities(&mut reader, baseline_count)?;
        let old = entities(&mut reader, a)?;
        let entities = entities(&mut reader, b)?;
        let mut server = Ring::load(64, 1024, 32, None).map_err(|e| format!("{e:?}"))?;
        let mut client = Ring::load(64, 1024, 32, retained).map_err(|e| format!("{e:?}"))?;
        for baseline in baselines {
            if !server.set_baseline(baseline.number, &baseline.words)
                || !client.set_baseline(baseline.number, &baseline.words)
            {
                return Err("native baseline".into());
            }
        }
        cases.push(Case {
            distance,
            prime,
            flags,
            areas,
            old_player,
            player,
            old,
            entities,
            server,
            client,
            expected: Vec::new(),
        });
    }
    if reader.byte_position() != input.len() {
        return Err("fixture boundary".into());
    }
    Ok(cases)
}

fn run<const P: usize, const E: usize>(
    case: &mut Case<P, E>,
    codec: &Codec<P, E>,
    base: u32,
    out: &mut [u8; 32768],
) -> Result<usize, String> {
    let mut wire = [0; 8192];
    case.server
        .store(Frame {
            sequence: base,
            time: ThinkTime::Milliseconds(100),
            command: 12,
            flags: case.flags,
            areas: &case.areas,
            player: &case.old_player,
            entities: &case.old,
        })
        .map_err(|e| format!("{e:?}"))?;
    if case.prime {
        let mut writer = Writer::new(&mut wire, codec.encoding);
        (codec.write)(&mut writer, &case.server, base, None).map_err(|e| format!("{e:?}"))?;
        writer.write_bits(codec.end, 8).map_err(|e| e.to_string())?;
        let mut reader = Reader::new(writer.bytes(), codec.encoding);
        let opcode = reader.read_bits(8).map_err(|e| e.to_string())?;
        if !(codec.read)(&mut reader, &mut case.client, base, 12, opcode, None)
            .map_err(|e| format!("{e:?}"))?
        {
            return Err("initial full snapshot".into());
        }
        if reader.read_bits(8).map_err(|e| e.to_string())? != codec.end {
            return Err("full EOF".into());
        }
    }
    let sequence = base + u32::from(case.distance.max(1));
    case.server
        .store(Frame {
            sequence,
            time: ThinkTime::Milliseconds(300),
            command: 12,
            flags: case.flags,
            areas: &case.areas,
            player: &case.player,
            entities: &case.entities,
        })
        .map_err(|e| format!("{e:?}"))?;
    let mut writer = Writer::new(&mut wire, codec.encoding);
    (codec.write)(
        &mut writer,
        &case.server,
        sequence,
        (case.distance != 0).then_some(base),
    )
    .map_err(|e| format!("{e:?}"))?;
    writer.write_bits(codec.end, 8).map_err(|e| e.to_string())?;
    let bits = writer.bit_position() as u32;
    let n = writer.size();
    let mut reader = Reader::new(writer.bytes(), codec.encoding);
    let opcode = reader.read_bits(8).map_err(|e| e.to_string())?;
    if !codec.opcodes.contains(&opcode) {
        return Err("snapshot opcode".into());
    }
    let accepted = (codec.read)(
        &mut reader,
        &mut case.client,
        sequence,
        12,
        opcode,
        (case.distance != 0).then_some(base),
    )
    .map_err(|e| format!("{e:?}"))?;
    if reader.read_bits(8).map_err(|e| e.to_string())? != codec.end {
        return Err("snapshot EOF".into());
    }
    let mut output = Writer::new(out, Encoding::Bytes);
    for value in [
        bits,
        n as u32,
        u32::from(accepted),
        reader.bit_position() as u32,
    ] {
        output.write_bits(value, 32).map_err(|e| e.to_string())?;
    }
    output
        .write_data(writer.bytes())
        .map_err(|e| e.to_string())?;
    if accepted {
        let frame = case.client.frame(sequence).ok_or("accepted snapshot")?;
        for value in [
            match frame.time {
                ThinkTime::Milliseconds(time) => time as u32,
                ThinkTime::Seconds(_) => return Err("native comparison milliseconds".into()),
            },
            frame.command,
            u32::from(frame.flags),
            frame.areas.len() as u32,
            frame.entities.len() as u32,
        ] {
            output.write_bits(value, 32).map_err(|e| e.to_string())?;
        }
        output.write_data(frame.areas).map_err(|e| e.to_string())?;
        for &word in frame.player {
            output.write_bits(word, 32).map_err(|e| e.to_string())?;
        }
        for entity in frame.entities {
            output
                .write_bits(entity.number, 32)
                .map_err(|e| e.to_string())?;
            for word in entity.words {
                output.write_bits(word, 32).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(output.size())
}

fn timing(fixture: &str, oracle: &str) -> Result<(), String> {
    let input = std::fs::read(fixture).map_err(|e| e.to_string())?;
    let oracle = std::fs::read(oracle).map_err(|e| e.to_string())?;
    let mut cases = load(&input, Some(1920))?;
    let mut output = [0; 32768];
    let mut offset = 0;
    for case in &mut cases {
        let n = run(case, &Q3, 1, &mut output)?;
        if oracle.get(offset..offset + n) != Some(&output[..n]) {
            return Err("native snapshot fidelity at load".into());
        }
        case.expected.extend_from_slice(&output[..n]);
        offset += n;
    }
    if cases.len() < 16 || offset != oracle.len() {
        return Err("oracle boundary".into());
    }
    cases.truncate(16);
    let storage_bytes: usize = cases
        .iter()
        .map(|case| case.server.allocated_bytes() + case.client.allocated_bytes())
        .sum();
    check_heap_counter()?;
    let mut samples = [0u64; 600];
    let mut counts = allocations::Counts::default();
    let mut checks = 0;
    let mut output_bytes = 0;
    for frame in 0..660 {
        allocations::begin_frame();
        let watch = Stopwatch::start();
        for case in &mut cases {
            let n = run(
                std::hint::black_box(case),
                &Q3,
                (frame as u32 + 1) * 64 + 1,
                &mut output,
            )?;
            if output[..n] != case.expected {
                return Err("native snapshot fidelity".into());
            }
            checks += 1;
            output_bytes += n;
        }
        let elapsed = watch.elapsed().as_nanos() as u64;
        let actual = allocations::end_frame();
        if frame >= 60 {
            samples[frame - 60] = elapsed;
            counts.allocations += actual.allocations;
            counts.reallocations += actual.reallocations;
            counts.requested_bytes += actual.requested_bytes;
        }
    }
    if counts != allocations::Counts::default() || checks != 10560 {
        return Err(format!("snapshot allocation/count gate {counts:?}"));
    }
    samples.sort_unstable();
    println!(
        "{{\"scope\":\"16 native snapshot pairs per iteration with original-C byte/word checks; no transport/host/gameplay\",\"warmup\":60,\"frames\":600,\"checks\":{checks},\"output_bytes_including_decoded_words\":{output_bytes},\"snapshot_storage_bytes\":{storage_bytes},\"positive_control_allocations\":1,\"allocations\":{},\"reallocations\":{},\"requested_bytes\":{},\"median_ns\":{},\"p99_ns\":{}}}",
        counts.allocations,
        counts.reallocations,
        counts.requested_bytes,
        (samples[299] as f64 + samples[300] as f64) * 0.5,
        samples[593]
    );
    Ok(())
}
fn main() -> Result<(), String> {
    let args = std::env::args().collect::<Vec<_>>();
    if args.get(1).is_some_and(|a| a == "--connected-nq-heap") {
        return connected_nq_heap();
    }
    if args.get(1).is_some_and(|a| a == "--connected-qw-heap") {
        return connected_qw_heap();
    }
    if args.get(1).is_some_and(|a| a == "--connected-heap") {
        return connected_heap();
    }
    if args.get(1).is_some_and(|a| a == "--timing") {
        return timing(
            args.get(2).ok_or("fixture path")?,
            args.get(3).ok_or("oracle path")?,
        );
    }
    let mut input = Vec::new();
    std::io::stdin()
        .read_to_end(&mut input)
        .map_err(|e| e.to_string())?;
    if args.get(1).is_some_and(|a| a == "--q2") {
        return compare(&input, &Q2, Some(896));
    }
    if args.get(1).is_some_and(|a| a == "--qw") {
        return compare(&input, &QW, None);
    }
    compare(&input, &Q3, Some(1920))
}

fn compare<const P: usize, const E: usize>(
    input: &[u8],
    codec: &Codec<P, E>,
    retained: Option<u64>,
) -> Result<(), String> {
    let mut cases = load(input, retained)?;
    check_heap_counter()?;
    let mut stdout = std::io::stdout().lock();
    let mut output = [0; 32768];
    for case in &mut cases {
        allocations::begin_frame();
        let n = run(case, codec, 1, &mut output)?;
        let heap = allocations::end_frame();
        if heap != allocations::Counts::default() {
            return Err(format!("snapshot comparison heap gate {heap:?}"));
        }
        stdout.write_all(&output[..n]).map_err(|e| e.to_string())?;
    }
    eprintln!(
        "{{\"scope\":\"caller Rust heap during native frame encode/decode; no host/workers/driver\",\"cases\":{},\"positive_control_allocations\":1,\"allocations\":0,\"reallocations\":0,\"requested_bytes\":0}}",
        cases.len()
    );
    Ok(())
}

fn check_heap_counter() -> Result<(), String> {
    allocations::begin_frame();
    let control = Box::new(std::hint::black_box(1u64));
    std::hint::black_box(&control);
    drop(control);
    if allocations::end_frame().allocations != 1 {
        return Err("heap positive control".into());
    }
    Ok(())
}

fn connected_heap() -> Result<(), String> {
    use qa_core::{
        loopback::Endpoint,
        primitives::{ClientId, PlayerState, RuleSetId, UserCmd, Vec3},
        sys_events::{EventTime, Peer},
    };
    use qa_network::{
        channel::Channel,
        commands::{connection::Commands, packet::Protocol},
        ingress::{Connection, Connections, Incoming, Route},
        projection::{PlayerContext, PlayerProjection},
        snapshots::ReceivedFrame,
        states,
    };
    let protocol = Protocol::Quake2_34;
    let mut server = Channel::load(protocol.channel(), Endpoint::Server, 8192, 16)
        .map_err(|e| format!("{e:?}"))?;
    server
        .configure_snapshots(protocol)
        .map_err(|e| e.to_string())?;
    let mut server_commands = Commands::load(protocol);
    let mut connections = Connections::load(1);
    connections
        .bind(
            ClientId(0),
            Endpoint::Client,
            Connection {
                route: Route {
                    socket: Endpoint::Client.socket(),
                    peer: Peer::Loopback(ClientId(0)),
                },
                channel: Channel::load(protocol.channel(), Endpoint::Client, 8192, 16)
                    .map_err(|e| format!("{e:?}"))?,
                commands: Some(Commands::load(protocol)),
                output: None,
            },
        )
        .map_err(|e| format!("{e:?}"))?;
    let projection = PlayerProjection::load(protocol, &[]);
    let mut source = PlayerState::with_capacity(2, 2, 4);
    source.health = 100;
    source.body.velocity = Vec3([-14.75, 0., 0.]);
    let mut imported = PlayerState::with_capacity(2, 2, 4);
    imported.movement_rules = RuleSetId::Quake3;
    imported.trace_rules = RuleSetId::Quake;
    let mut context = PlayerContext {
        client_number: None,
        ground_number: None,
        weapon_number: None,
        weapon_model: None,
        gravity: 800.,
        speed: 320.,
        player_info_flags: 0,
        command_age_ms: 0,
        body_yaw: 0.,
    };
    check_heap_counter()?;
    let mut measured = allocations::Counts::default();
    let mut checks = 0;
    for iteration in 0..660u32 {
        let frame = 100 + iteration;
        let delta = server_commands.delta_request();
        let mut player = [0; states::Q2_PLAYER_WORDS];
        let mut entity = Entity {
            number: 30,
            words: [0; states::Q2_ENTITY_WORDS],
        };
        entity.words[8] = (frame as f32).to_bits();
        entity.words[14] = 999.0f32.to_bits();
        allocations::begin_frame();
        let result = (|| -> Result<(), qa_network::commands::packet::Error> {
            source.body.position.0[0] = frame as f32 * 0.125;
            if !projection.reduce(&source, &context, &mut player) {
                return Err(qa_network::commands::packet::Error::Context);
            }
            server.publish_snapshot(ReceivedFrame::Quake2(Frame {
                sequence: frame,
                time: ThinkTime::Milliseconds(0),
                command: 0,
                flags: 0,
                areas: &[0x81],
                player: &player,
                entities: &[entity],
            }))?;
            let mut payload = [0; 1400];
            let mut writer = Writer::new(&mut payload, Encoding::Bytes);
            server.write_snapshot(&mut writer, frame, delta, 16)?;
            writer.write_data(&[10, 2, b'x', 0])?;
            let packet = server
                .prepare_move(writer.bytes(), EventTime(1), None)
                .map_err(|_| qa_network::commands::packet::Error::Context)?
                .ok_or(qa_network::commands::packet::Error::Context)?;
            let mut packet_bytes = [0; 1400];
            let length = packet.bytes.len();
            packet_bytes[..length].copy_from_slice(packet.bytes);
            server
                .submitted(EventTime(1))
                .map_err(|_| qa_network::commands::packet::Error::Context)?;
            let mut frames = 0;
            let mut prints = 0;
            let mut valid = true;
            connections.receive(
                Endpoint::Client.socket(),
                Peer::Loopback(ClientId(0)),
                &packet_bytes[..length],
                EventTime(2),
                |_, _, incoming| match incoming {
                    Incoming::Snapshot(snapshots::ReceivedFrame::Quake2(received)) => {
                        frames += 1;
                        valid &= received.sequence == frame
                            && received.player[1] == frame
                            && received.entities.len() == 1
                            && received.entities[0].words[8] == entity.words[8];
                        valid &=
                            projection
                                .apply(received.player, &mut imported, &mut context, |_| None)
                                && imported.body.position == source.body.position
                                && imported.body.velocity == source.body.velocity
                                && imported.health == 100
                                && imported.movement_rules == RuleSetId::Quake3
                                && imported.trace_rules == RuleSetId::Quake;
                    }
                    Incoming::Print(print) => {
                        prints += 1;
                        valid &= print.text == b"x";
                    }
                    _ => valid = false,
                },
            );
            if !valid || frames != 1 || prints != 1 {
                return Err(qa_network::commands::packet::Error::Context);
            }
            let client = connections
                .get(ClientId(0), Endpoint::Client)
                .ok_or(qa_network::commands::packet::Error::Context)?;
            let codec = client
                .commands
                .as_ref()
                .ok_or(qa_network::commands::packet::Error::Context)?;
            let mut movement = [0; 1400];
            let length = codec.encode(
                &UserCmd {
                    duration_ms: 20,
                    ..Default::default()
                },
                &client.channel,
                &mut movement,
            )?;
            let staged = server_commands.stage(&movement[..length])?;
            server_commands.decode(
                staged,
                client.channel.send_state().sequence,
                0,
                0,
                &mut server,
            )?;
            if server_commands.delta_request() != Some(frame) {
                return Err(qa_network::commands::packet::Error::Context);
            }
            Ok(())
        })();
        let heap = allocations::end_frame();
        result.map_err(|e| e.to_string())?;
        checks += 1;
        if iteration >= 60 {
            measured.allocations += heap.allocations;
            measured.reallocations += heap.reallocations;
            measured.requested_bytes += heap.requested_bytes;
        }
    }
    if measured != allocations::Counts::default() || connections.command_errors != 0 {
        return Err(format!("connected snapshot heap/count gate {measured:?}"));
    }
    println!(
        "{{\"scope\":\"common player reduce/apply, Q2 frame store/write, Channel, CLIENT ingress, print dispatch and native move feedback; caller Rust heap, no workers/OS/app/gameplay\",\"warmup\":60,\"measured_iterations\":600,\"checks_including_warmup\":{checks},\"positive_control_allocations\":1,\"allocations\":0,\"reallocations\":0,\"requested_bytes\":0,\"command_errors\":0,\"timing_run\":false}}"
    );
    Ok(())
}

fn connected_qw_heap() -> Result<(), String> {
    use qa_core::{
        loopback::Endpoint,
        primitives::{ClientId, PlayerState, RuleSetId, UserCmd},
        sys_events::{EventTime, Peer},
    };
    use qa_network::{
        channel::{Channel, Delivery},
        commands::{
            connection::Commands,
            packet::{Error, Protocol},
        },
        ingress::{Connection, Connections, Incoming, Route},
        projection::{PlayerContext, PlayerProjection},
        snapshots::ReceivedFrame,
        states,
    };
    check_heap_counter()?;
    let mut server = Channel::load(Protocol::QuakeWorld28.channel(), Endpoint::Server, 8192, 16)
        .map_err(|e| e.to_string())?;
    server
        .configure_snapshots(Protocol::QuakeWorld28)
        .map_err(|e| e.to_string())?;
    let mut server_commands = Commands::load(Protocol::QuakeWorld28);
    let mut connections = Connections::load(1);
    connections
        .bind(
            ClientId(0),
            Endpoint::Client,
            Connection {
                route: Route {
                    socket: Endpoint::Client.socket(),
                    peer: Peer::Loopback(ClientId(0)),
                },
                channel: Channel::load(
                    Protocol::QuakeWorld28.channel(),
                    Endpoint::Client,
                    8192,
                    16,
                )
                .map_err(|e| e.to_string())?,
                output: None,
                commands: Some(Commands::load(Protocol::QuakeWorld28)),
            },
        )
        .map_err(|e| format!("QW bind {e:?}"))?;
    connections
        .get_mut(ClientId(0), Endpoint::Client)
        .ok_or("QW CLIENT")?
        .channel
        .set_qw_player_model(42)
        .map_err(|e| e.to_string())?;
    let projection = PlayerProjection::load(Protocol::QuakeWorld28, &[]);
    let mut source = PlayerState::default();
    let mut destination = PlayerState {
        movement_rules: RuleSetId::Quake3,
        trace_rules: RuleSetId::Quake2,
        health: 100,
        ..Default::default()
    };
    let mut context = PlayerContext {
        client_number: None,
        ground_number: None,
        weapon_number: None,
        weapon_model: None,
        gravity: 0.,
        speed: 0.,
        player_info_flags: 31,
        command_age_ms: 17,
        body_yaw: 0.,
    };
    let mut measured = allocations::Counts::default();
    let mut checks = 0;
    for iteration in 0..660 {
        let sequence = iteration + 1;
        let x = sequence as f32 * 0.125;
        let mut payload = [0; 1400];
        let mut wire = [0; 1400];
        allocations::begin_frame();
        let result = (|| -> Result<(), Error> {
            let client = connections
                .get_mut(ClientId(0), Endpoint::Client)
                .ok_or(Error::Context)?;
            let codec = client.commands.as_ref().ok_or(Error::Context)?;
            let command = UserCmd {
                duration_ms: 20,
                movement: [300., 0., 0.],
                ..Default::default()
            };
            let length = codec.encode(&command, &client.channel, &mut payload)?;
            let base = codec.snapshot_request(&client.channel);
            let packet = client
                .channel
                .prepare_move(&payload[..length], EventTime(1), base)
                .map_err(|_| Error::Context)?
                .ok_or(Error::Context)?;
            let length = packet.bytes.len();
            wire[..length].copy_from_slice(packet.bytes);
            client
                .channel
                .submitted(EventTime(1))
                .map_err(|_| Error::Context)?;
            let received = server
                .receive(&wire[..length], EventTime(2))
                .map_err(|_| Error::Context)?;
            if received.header.sequence != sequence {
                return Err(Error::Context);
            }
            let Delivery::Payload(bytes) = received.delivery else {
                return Err(Error::Context);
            };
            let length = server_commands.stage(bytes)?;
            let received = server_commands
                .decode(length, sequence, 0, 0, &mut server)?
                .ok_or(Error::Context)?;
            if received.movement[0] != 300.
                || server_commands.delta_request() != base.map(|n| u32::from(n as u8))
            {
                return Err(Error::Context);
            }
            let mut words = [0; states::QW_ENTITY_WORDS];
            words[0] = 1;
            words[5] = x.to_bits();
            server.publish_snapshot(ReceivedFrame::QuakeWorld(Frame {
                sequence,
                time: ThinkTime::Milliseconds(0),
                command: 0,
                flags: 0,
                areas: &[],
                player: &[],
                entities: &[Entity { number: 3, words }],
            }))?;
            let mut writer = Writer::new(&mut payload, Encoding::Bytes);
            source.body.position.0[0] = x;
            source.body.velocity.0[0] = 300.;
            let mut player_words = [0; states::QW_PLAYER_WORDS];
            if !projection.reduce(&source, &context, &mut player_words) {
                return Err(Error::Context);
            }
            if !states::write_qw_player(
                &mut writer,
                31,
                &player_words,
                qa_network::commands::to_qw_usercmd(&command),
            )? {
                return Err(Error::Context);
            }
            server.write_snapshot(&mut writer, sequence, server_commands.delta_request(), 0)?;
            writer.write_bits(8, 8)?;
            writer.write_bits(2, 8)?;
            writer.write_data(b"connected\0")?;
            let packet = server
                .prepare_move(writer.bytes(), EventTime(3), None)
                .map_err(|_| Error::Context)?
                .ok_or(Error::Context)?;
            let length = packet.bytes.len();
            wire[..length].copy_from_slice(packet.bytes);
            server.submitted(EventTime(3)).map_err(|_| Error::Context)?;
            let mut outputs = 0;
            let mut invalid = false;
            connections.receive(
                Endpoint::Client.socket(),
                Peer::Loopback(ClientId(0)),
                &wire[..length],
                EventTime(4),
                |_, _, incoming| match incoming {
                    Incoming::Snapshot(ReceivedFrame::QuakeWorld(frame)) => {
                        invalid |= frame.sequence != sequence
                            || frame.entities.len() != 1
                            || frame.entities[0].words[5] != x.to_bits();
                        outputs += 1;
                    }
                    Incoming::PlayerInfo(info) => {
                        invalid |= info.number != 31
                            || info.words[8] != 42
                            || info.command.msec != 20
                            || info.command.movement != [300, 0, 0]
                            || !projection.apply(
                                &info.words,
                                &mut destination,
                                &mut context,
                                |_| None,
                            )
                            || destination.body.position.0[0] != x
                            || destination.body.velocity.0[0] != 300.
                            || destination.movement_rules != RuleSetId::Quake3
                            || destination.trace_rules != RuleSetId::Quake2
                            || destination.health != 100;
                        outputs += 1;
                    }
                    Incoming::Print(print) => {
                        invalid |= print.text != b"connected";
                        outputs += 1;
                    }
                    _ => invalid = true,
                },
            );
            if invalid || outputs != 3 {
                return Err(Error::Context);
            }
            Ok(())
        })();
        let heap = allocations::end_frame();
        result.map_err(|e| e.to_string())?;
        checks += 1;
        if iteration >= 60 {
            measured.allocations += heap.allocations;
            measured.reallocations += heap.reallocations;
            measured.requested_bytes += heap.requested_bytes;
        }
    }
    if measured != allocations::Counts::default() || connections.command_errors != 0 {
        return Err(format!("connected QW heap gate {measured:?}"));
    }
    println!(
        "{{\"scope\":\"QW submitted move/request association, native reply alignment, playerinfo/common projection, packet frame store/write, connected CLIENT ingress and print dispatch; caller Rust heap, no app/workers/OS/gameplay\",\"warmup\":60,\"measured_iterations\":600,\"checks_including_warmup\":{checks},\"positive_control_allocations\":1,\"allocations\":0,\"reallocations\":0,\"requested_bytes\":0,\"command_errors\":0,\"timing_run\":false}}"
    );
    Ok(())
}

fn connected_nq_heap() -> Result<(), String> {
    use qa_core::{
        loopback::Endpoint,
        primitives::{ClientId, PlayerState, RuleSetId},
        sys_events::{EventTime, Peer},
    };
    use qa_network::{
        channel::Channel,
        commands::{
            connection::Commands,
            packet::{Error, Protocol},
        },
        ingress::{Connection, Connections, Incoming, Route},
        projection::{PlayerContext, PlayerProjection},
        snapshots::ReceivedFrame,
        states,
    };
    check_heap_counter()?;
    let protocol = Protocol::NetQuake15;
    let mut server =
        Channel::load(protocol.channel(), Endpoint::Server, 8192, 16).map_err(|e| e.to_string())?;
    server
        .configure_snapshots(protocol)
        .map_err(|e| e.to_string())?;
    let mut connections = Connections::load(1);
    connections
        .bind(
            ClientId(0),
            Endpoint::Client,
            Connection {
                route: Route {
                    socket: Endpoint::Client.socket(),
                    peer: Peer::Loopback(ClientId(0)),
                },
                channel: Channel::load(protocol.channel(), Endpoint::Client, 8192, 16)
                    .map_err(|e| e.to_string())?,
                commands: Some(Commands::load(protocol)),
                output: None,
            },
        )
        .map_err(|e| format!("NQ bind {e:?}"))?;
    let projection = PlayerProjection::load(protocol, &[]);
    let mut source = PlayerState {
        health: 100,
        ..Default::default()
    };
    source.body.velocity.0[0] = -32.;
    source.view_offset.0[2] = 22.;
    let mut imported = PlayerState {
        movement_rules: RuleSetId::Quake3,
        trace_rules: RuleSetId::Quake2,
        ..Default::default()
    };
    let mut context = PlayerContext {
        client_number: None,
        ground_number: None,
        weapon_number: None,
        weapon_model: None,
        gravity: 0.,
        speed: 0.,
        player_info_flags: 0,
        command_age_ms: 0,
        body_yaw: 0.,
    };
    let mut measured = allocations::Counts::default();
    let mut checks = 0;
    for iteration in 0..660u32 {
        let sequence = server.send_state().datagram_sequence;
        let seconds = iteration as f32 * 0.0625;
        let x = iteration as f32 * 0.125;
        allocations::begin_frame();
        let result = (|| -> Result<(), Error> {
            let mut player = [0; states::NQ_PLAYER_WORDS];
            if !projection.reduce(&source, &context, &mut player) {
                return Err(Error::Context);
            }
            let mut entity = Entity {
                number: 1,
                words: [0; states::NQ_ENTITY_WORDS],
            };
            entity.words[0] = 1.0f32.to_bits();
            entity.words[5] = x.to_bits();
            server.publish_snapshot(ReceivedFrame::NetQuake(Frame {
                sequence,
                time: ThinkTime::Seconds(f64::from(seconds)),
                command: 0,
                flags: 0,
                areas: &[],
                player: &player,
                entities: &[entity],
            }))?;
            let mut payload = [0; 1400];
            let mut writer = Writer::new(&mut payload, Encoding::Bytes);
            server.write_snapshot(&mut writer, sequence, None, 0)?;
            writer.write_bits(26, 8)?;
            writer.write_data(b"connected\0")?;
            let packet = server
                .prepare_move(writer.bytes(), EventTime(1), None)
                .map_err(|_| Error::Context)?
                .ok_or(Error::Context)?;
            let mut bytes = [0; 1400];
            let length = packet.bytes.len();
            bytes[..length].copy_from_slice(packet.bytes);
            server.submitted(EventTime(1)).map_err(|_| Error::Context)?;
            let mut count = 0;
            let mut invalid = false;
            connections.receive(
                Endpoint::Client.socket(),
                Peer::Loopback(ClientId(0)),
                &bytes[..length],
                EventTime(2),
                |_, _, incoming| match incoming {
                    Incoming::Snapshot(ReceivedFrame::NetQuake(frame)) => {
                        invalid |= frame.sequence != sequence
                            || frame.time != ThinkTime::Seconds(f64::from(seconds))
                            || frame.entities.len() != 1
                            || frame.entities[0].words[5] != x.to_bits()
                            || !projection
                                .apply(frame.player, &mut imported, &mut context, |_| None)
                            || imported.health != 100
                            || imported.body.velocity.0[0] != -32.
                            || imported.movement_rules != RuleSetId::Quake3
                            || imported.trace_rules != RuleSetId::Quake2;
                        count += 1;
                    }
                    Incoming::Print(print) => {
                        invalid |= print.text != b"connected";
                        count += 1;
                    }
                    _ => invalid = true,
                },
            );
            if invalid || count != 2 {
                return Err(Error::Context);
            }
            Ok(())
        })();
        let heap = allocations::end_frame();
        result.map_err(|e| e.to_string())?;
        checks += 1;
        if iteration >= 60 {
            measured.allocations += heap.allocations;
            measured.reallocations += heap.reallocations;
            measured.requested_bytes += heap.requested_bytes;
        }
    }
    if measured != allocations::Counts::default() || connections.command_errors != 0 {
        return Err(format!("NQ heap gate {measured:?}"));
    }
    println!(
        "{{\"scope\":\"NQ native time, clientdata, baseline entity store/write, connected CLIENT ingress, common player import and print; caller Rust heap, no app/workers/OS/gameplay\",\"warmup\":60,\"measured_iterations\":600,\"checks_including_warmup\":{checks},\"positive_control_allocations\":1,\"allocations\":0,\"reallocations\":0,\"requested_bytes\":0,\"command_errors\":0,\"timing_run\":false}}"
    );
    Ok(())
}
