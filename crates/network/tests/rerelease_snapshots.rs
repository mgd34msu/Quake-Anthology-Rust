use qa_core::primitives::ThinkTime;
use qa_network::{
    commands::packet::Error,
    message::{Encoding, Reader, Writer},
    snapshots::{self, Entity, Frame, Q2Header, Q2KexContext, Q2KexRing, Q2ReproRing},
    states::{self, Q2_RERELEASE_ENTITY_WORDS, Q2KexPlayer, Q2ReproPlayer},
};

fn body(number: u32, origin: f32, beam: bool) -> Entity<Q2_RERELEASE_ENTITY_WORDS> {
    let mut words = [0; Q2_RERELEASE_ENTITY_WORDS];
    words[0] = 65535;
    words[7] = if beam { 128 } else { 0 };
    words[8] = origin.to_bits();
    words[14] = (-20.125f32).to_bits();
    words[18] = 9;
    Entity { number, words }
}

fn player() -> Q2KexPlayer {
    let mut player = Q2KexPlayer::default();
    player.words[1] = 12.75f32.to_bits();
    player.words[8] = 0x8000;
    player.words[10] = (-0.0f32).to_bits();
    player.words[23] = 511;
    player.words[40] = 30;
    player.stats[63] = (-32768i32) as u32;
    player
}

fn wire(
    sequence: u32,
    delta: i32,
    from: &Q2KexPlayer,
    to: &Q2KexPlayer,
    demo: bool,
    bytes: &mut [u8],
    entities: impl FnOnce(&mut Writer<'_>) -> Result<(), qa_network::message::Error>,
) -> Result<usize, Error> {
    let mut writer = Writer::new(bytes, Encoding::Bytes);
    Q2Header {
        sequence,
        delta,
        flags: 5,
        player_flags: 0,
    }
    .write::<false>(&mut writer, &[0x81, 0x42])?;
    states::write_q2_kex_player(&mut writer, from, to)?;
    writer.write_bits(18, 8)?;
    let _ = demo;
    entities(&mut writer)?;
    writer.write_bits(0, 16)?;
    writer.write_bits(1, 8)?;
    Ok(writer.size())
}

fn receive(ring: &mut Q2KexRing, context: &mut Q2KexContext, bytes: &[u8]) -> Result<bool, Error> {
    consume(bytes, |reader| {
        snapshots::read_q2_kex(reader, ring, context, |n| {
            ThinkTime::Milliseconds(i64::from(n) * 25)
        })
    })
}

fn consume(
    bytes: &[u8],
    read: impl FnOnce(&mut Reader<'_>) -> Result<bool, Error>,
) -> Result<bool, Error> {
    let mut reader = Reader::new(bytes, Encoding::Bytes);
    assert_eq!(reader.read_bits(8)?, 20);
    let accepted = read(&mut reader)?;
    assert_eq!(reader.read_bits(8)?, 1);
    assert_eq!(reader.byte_position(), bytes.len());
    Ok(accepted)
}

fn repro_wire(
    sequence: u32,
    delta: i32,
    from: &Q2ReproPlayer,
    to: &Q2ReproPlayer,
    bytes: &mut [u8],
    entities: impl FnOnce(&mut Writer<'_>) -> Result<(), qa_network::message::Error>,
) -> Result<usize, Error> {
    let mut player_bytes = [0; 1400];
    let mut player = Writer::new(&mut player_bytes, Encoding::Bytes);
    let player_flags = states::write_q2_repro_player(&mut player, from, to)?;
    let mut writer = Writer::new(bytes, Encoding::Bytes);
    Q2Header {
        sequence,
        delta,
        flags: 0xf5,
        player_flags,
    }
    .write::<true>(&mut writer, &[0x81, 0x42])?;
    for &byte in player.bytes() {
        writer.write_bits(u32::from(byte), 8)?;
    }
    entities(&mut writer)?;
    writer.write_bits(0, 16)?;
    writer.write_bits(1, 8)?;
    Ok(writer.size())
}

fn receive_repro(ring: &mut Q2ReproRing, bytes: &[u8]) -> Result<bool, Error> {
    consume(bytes, |reader| {
        snapshots::read_q2_repro(reader, ring, |n| {
            ThinkTime::Milliseconds(i64::from(n) * 100)
        })
    })
}

#[test]
fn repro_frames_use_packed_prefix_extra_flags_and_the_shared_merge() -> Result<(), Error> {
    let mut ring = Q2ReproRing::load(16, 8192, 32, Some(8192))?;
    let mut first = body(1, 1.25, false);
    first.words[11] = (-32768i32) as u32;
    let beam = body(3, 3.125, true);
    let baseline = body(8191, 7.25, false);
    assert!(ring.set_baseline(8191, &baseline.words));
    let mut player = Q2ReproPlayer::default();
    player.words[1] = 12.75f32.to_bits();
    player.words[8] = 0x8000;
    player.words[10] = 90.0f32.to_bits();
    player.words[16] = (-32768i32) as u32;
    player.words[22] = 65535;
    player.words[23] = 65535;
    player.words[36] = 127;
    player.words[39] = 255;
    player.words[40] = 255;
    player.words[41] = (-128i32) as u32;
    player.words[42] = (-32768i32) as u32;
    player.stats[63] = 32767;
    let mut bytes = [0; 1400];
    let size = repro_wire(
        1,
        -1,
        &Q2ReproPlayer::default(),
        &player,
        &mut bytes,
        |writer| {
            for entity in [first, beam] {
                states::write_q2_repro_entity(
                    writer,
                    entity.number as u16,
                    &[0; 25],
                    Some(&entity.words),
                    true,
                )?;
            }
            Ok(())
        },
    )?;
    assert!(receive_repro(&mut ring, &bytes[..size])?);
    let frame = ring.current().ok_or(Error::Context)?;
    assert_eq!(frame.entities, [first, beam]);
    assert_eq!(&frame.player[..43], player.words);
    assert_eq!(&frame.player[43..], player.stats);
    assert_eq!(frame.flags, 5);
    assert_eq!(frame.time, ThinkTime::Milliseconds(100));
    let mut next_player = player;
    next_player.words[6] = (-1.125f32).to_bits();
    next_player.words[39] = 0;
    next_player.words[42] = 255;
    next_player.stats[32] = (-32768i32) as u32;
    let mut inserted = baseline;
    inserted.words[0] = 60000;
    let size = repro_wire(2, 1, &player, &next_player, &mut bytes, |writer| {
        states::write_q2_repro_entity(writer, 1, &first.words, None, false)?;
        states::write_q2_repro_entity(writer, 8191, &baseline.words, Some(&inserted.words), true)
    })?;
    assert!(receive_repro(&mut ring, &bytes[..size])?);
    let mut retained_beam = beam;
    retained_beam.words[18] = 0;
    let frame = ring.current().ok_or(Error::Context)?;
    assert_eq!(frame.entities, [retained_beam, inserted]);
    assert_eq!(&frame.player[..43], next_player.words);
    assert_eq!(&frame.player[43..], next_player.stats);
    Ok(())
}

#[test]
fn repro_invalid_bases_and_truncated_frames_do_not_publish() -> Result<(), Error> {
    let player = Q2ReproPlayer::default();
    let mut bytes = [0; 1400];
    let size = repro_wire(2, 1, &player, &player, &mut bytes, |_| Ok(()))?;
    let mut ring = Q2ReproRing::load(1, 8192, 32, Some(8192))?;
    assert!(!receive_repro(&mut ring, &bytes[..size])?);
    assert_eq!(ring.counts().missing_base, 1);
    assert!(ring.current().is_none());
    let size = repro_wire(3, -1, &player, &player, &mut bytes, |_| Ok(()))?;
    assert!(receive_repro(&mut ring, &bytes[..size])?);
    let size = repro_wire(3, 3, &player, &player, &mut bytes, |_| Ok(()))?;
    assert!(!receive_repro(&mut ring, &bytes[..size])?);
    let size = repro_wire(5, -1, &player, &player, &mut bytes, |writer| {
        states::write_q2_repro_entity(
            writer,
            8191,
            &[0; 25],
            Some(&body(8191, 1.25, false).words),
            true,
        )
    })?;
    for cut in 0..size - 1 {
        let mut reader = Reader::new(&bytes[..cut], Encoding::Bytes);
        assert!(
            (|| {
                reader.read_bits(8)?;
                snapshots::read_q2_repro(&mut reader, &mut ring, |_| ThinkTime::Milliseconds(123))
            })()
            .is_err()
        );
        assert!(ring.current().is_none());
    }
    assert!(receive_repro(&mut ring, &bytes[..size])?);
    assert_eq!(ring.current().ok_or(Error::Context)?.sequence, 5);
    Ok(())
}

#[test]
fn unmatched_remove_advances_the_native_q2_old_cursor() -> Result<(), Error> {
    for demo in [false, true] {
        let mut ring = Q2KexRing::load(16, 8192, 32, Some(8192))?;
        let mut context = Q2KexContext::load(&ring, demo)?;
        let entities = [body(1, 1.0, false), body(3, 3.0, false), body(5, 5.0, true)];
        ring.store(Frame {
            sequence: 1,
            time: ThinkTime::Milliseconds(25),
            command: 0,
            flags: 0,
            areas: &[],
            player: &[0; 106],
            entities: &entities,
        })?;
        let mut bytes = [0; 1400];
        let player = Q2KexPlayer::default();
        let size = wire(2, 1, &player, &player, demo, &mut bytes, |writer| {
            states::write_q2_kex_entity(
                writer,
                2,
                &[0; Q2_RERELEASE_ENTITY_WORDS],
                None,
                false,
                demo,
                &mut states::Q2KexWire::default(),
            )
        })?;
        assert!(receive(&mut ring, &mut context, &bytes[..size])?);
        let mut first = entities[0];
        first.words[18] = 0;
        first.words[14..17].copy_from_slice(&entities[0].words[8..11]);
        let mut last = entities[2];
        last.words[18] = 0;
        assert_eq!(
            ring.current().ok_or(Error::Context)?.entities,
            [first, last]
        );
    }
    Ok(())
}

#[test]
fn kex_frames_share_the_ring_and_merge_with_native_beam_and_demo_rules() -> Result<(), Error> {
    for demo in [false, true] {
        let mut ring = Q2KexRing::load(16, 8192, 32, Some(8192))?;
        let beam = body(1, 1.25, true);
        let regular = body(3, 3.125, false);
        let removed = body(5, 5.25, false);
        let baseline = body(7, 7.125, false);
        let mut context = Q2KexContext::load(&ring, demo)?;
        assert!(context.set_baseline(&mut ring, 7, &baseline.words));
        let player = player();
        let mut bytes = [0; 1400];
        let n = wire(
            1,
            -1,
            &Q2KexPlayer::default(),
            &player,
            demo,
            &mut bytes,
            |writer| {
                for entity in [beam, regular, removed] {
                    states::write_q2_kex_entity(
                        writer,
                        entity.number as u16,
                        &[0; Q2_RERELEASE_ENTITY_WORDS],
                        Some(&entity.words),
                        true,
                        demo,
                        &mut states::Q2KexWire {
                            nonzero_solid: (&[0; Q2_RERELEASE_ENTITY_WORDS])[19] != 0,
                            baseline_solid: false,
                        },
                    )?;
                }
                Ok(())
            },
        )?;
        assert!(receive(&mut ring, &mut context, &bytes[..n])?);
        let old = ring.frame(1).ok_or(Error::Context)?;
        assert_eq!(old.entities, [beam, regular, removed]);
        let mut expected_player = player.words;
        // Native player delta comparison treats +0 and -0 as equal, unlike
        // KEX entity angle metadata, which compares packed float bits.
        expected_player[10] = 0;
        assert_eq!(&old.player[..42], expected_player);
        assert_eq!(&old.player[42..], player.stats);
        let mut changed = regular;
        changed.words[8] = (-1.01f32).to_bits();
        changed.words[19] = 1;
        changed.words[18] = 0;
        let mut inserted = baseline;
        inserted.words[0] = 60000;
        inserted.words[18] = 0;
        let mut next_player = player;
        next_player.stats[0] = 127;
        let n = wire(2, 1, &player, &next_player, demo, &mut bytes, |writer| {
            states::write_q2_kex_entity(
                writer,
                3,
                &regular.words,
                Some(&changed.words),
                false,
                demo,
                &mut states::Q2KexWire {
                    nonzero_solid: (&regular.words)[19] != 0,
                    baseline_solid: false,
                },
            )?;
            states::write_q2_kex_entity(
                writer,
                5,
                &removed.words,
                None,
                false,
                demo,
                &mut states::Q2KexWire {
                    nonzero_solid: (&removed.words)[19] != 0,
                    baseline_solid: false,
                },
            )?;
            states::write_q2_kex_entity(
                writer,
                7,
                &baseline.words,
                Some(&inserted.words),
                false,
                demo,
                &mut states::Q2KexWire {
                    nonzero_solid: (&baseline.words)[19] != 0,
                    baseline_solid: false,
                },
            )
        })?;
        assert!(receive(&mut ring, &mut context, &bytes[..n])?);
        let mut unchanged_beam = beam;
        unchanged_beam.words[18] = 0;
        changed.words[14..17].copy_from_slice(&regular.words[8..11]);
        inserted.words[14..17].copy_from_slice(&baseline.words[8..11]);
        let current = ring.frame(2).ok_or(Error::Context)?;
        assert_eq!(current.entities, [unchanged_beam, changed, inserted]);
        assert_eq!(&current.player[42..], next_player.stats);
        assert_eq!(current.time, ThinkTime::Milliseconds(50));
        assert_eq!(current.flags, 5);
        assert_eq!(current.areas, [0x81, 0x42]);
        assert_eq!(ring.counts().accepted, 2);
    }
    Ok(())
}

#[test]
fn missing_invalid_and_current_bases_are_consumed_without_publication() -> Result<(), Error> {
    let mut ring = Q2KexRing::load(8, 8192, 32, None)?;
    let mut context = Q2KexContext::load(&ring, false)?;
    let player = player();
    let mut bytes = [0; 1400];
    for (sequence, delta) in [(2, 1), (3, 2)] {
        let n = wire(
            sequence,
            delta,
            &Q2KexPlayer::default(),
            &player,
            false,
            &mut bytes,
            |_| Ok(()),
        )?;
        assert!(!receive(&mut ring, &mut context, &bytes[..n])?);
        assert!(ring.frame(sequence).is_none());
    }
    assert_eq!(ring.counts().missing_base, 2);
    let n = wire(
        4,
        -1,
        &Q2KexPlayer::default(),
        &player,
        false,
        &mut bytes,
        |_| Ok(()),
    )?;
    assert!(receive(&mut ring, &mut context, &bytes[..n])?);
    let n = wire(4, 4, &player, &player, false, &mut bytes, |_| Ok(()))?;
    assert!(!receive(&mut ring, &mut context, &bytes[..n])?);
    assert!(ring.frame(4).is_none());
    Ok(())
}

#[test]
fn frame_failures_preserve_published_state_and_overflow_is_bounded() -> Result<(), Error> {
    let mut ring = Q2KexRing::load(1, 8192, 32, None)?;
    let mut context = Q2KexContext::load(&ring, true)?;
    let old_player = [0; 106];
    let old = body(1, 1.25, false);
    ring.store(Frame {
        sequence: 1,
        time: ThinkTime::Milliseconds(25),
        command: 0,
        flags: 0,
        areas: &[],
        player: &old_player,
        entities: &[old],
    })?;
    let player = player();
    let mut bytes = [0; 1400];
    let n = wire(
        2,
        -1,
        &Q2KexPlayer::default(),
        &player,
        true,
        &mut bytes,
        |writer| {
            for number in [1, 2] {
                let to = body(number, 1.125, false);
                states::write_q2_kex_entity(
                    writer,
                    number as u16,
                    &[0; Q2_RERELEASE_ENTITY_WORDS],
                    Some(&to.words),
                    true,
                    true,
                    &mut states::Q2KexWire {
                        nonzero_solid: (&[0; Q2_RERELEASE_ENTITY_WORDS])[19] != 0,
                        baseline_solid: false,
                    },
                )?;
            }
            Ok(())
        },
    )?;
    // The final byte is a subsequent service; every shorter frame body fails.
    for size in 1..n - 1 {
        let mut reader = Reader::new(&bytes[1..size], Encoding::Bytes);
        assert!(
            snapshots::read_q2_kex(&mut reader, &mut ring, &mut context, |_| {
                ThinkTime::Milliseconds(50)
            })
            .is_err()
        );
        assert_eq!(ring.frame(1).ok_or(Error::Context)?.entities, [old]);
        assert!(ring.frame(2).is_none());
    }
    assert!(!receive(&mut ring, &mut context, &bytes[..n])?);
    assert_eq!(ring.counts().overflow, 1);
    assert_eq!(ring.frame(1).ok_or(Error::Context)?.entities, [old]);
    assert!(ring.frame(2).is_none());
    // Demo non-solid coordinates use the native signed eighth-unit field.
    let mut complete = Q2KexRing::load(2, 8192, 32, None)?;
    let mut complete_context = Q2KexContext::load(&complete, true)?;
    let to = body(1, -1.01, false);
    let n = wire(
        3,
        -1,
        &Q2KexPlayer::default(),
        &player,
        true,
        &mut bytes,
        |writer| {
            states::write_q2_kex_entity(
                writer,
                1,
                &[0; Q2_RERELEASE_ENTITY_WORDS],
                Some(&to.words),
                true,
                true,
                &mut states::Q2KexWire {
                    nonzero_solid: (&[0; Q2_RERELEASE_ENTITY_WORDS])[19] != 0,
                    baseline_solid: false,
                },
            )
        },
    )?;
    assert!(receive(&mut complete, &mut complete_context, &bytes[..n])?);
    assert_eq!(
        complete.frame(3).ok_or(Error::Context)?.entities[0].words[8],
        (-1.0f32).to_bits()
    );
    Ok(())
}

#[test]
fn enhanced_native_frame_and_entity_numbers_reject_out_of_range_fields() -> Result<(), Error> {
    let mut ring = Q2KexRing::load(2, 8192, 32, None)?;
    let mut context = Q2KexContext::load(&ring, false)?;
    let player = player();
    let mut bytes = [0; 1400];
    let n = wire(
        u32::MAX,
        -1,
        &Q2KexPlayer::default(),
        &player,
        false,
        &mut bytes,
        |_| Ok(()),
    )?;
    assert_eq!(
        snapshots::read_q2_kex(
            &mut Reader::new(&bytes[1..n], Encoding::Bytes),
            &mut ring,
            &mut context,
            |_| ThinkTime::Milliseconds(0)
        ),
        Err(Error::Count)
    );
    let n = wire(
        1,
        -1,
        &Q2KexPlayer::default(),
        &player,
        false,
        &mut bytes,
        |writer| {
            states::write_q2_kex_entity(
                writer,
                8192,
                &[0; Q2_RERELEASE_ENTITY_WORDS],
                Some(&[0; Q2_RERELEASE_ENTITY_WORDS]),
                true,
                false,
                &mut states::Q2KexWire {
                    nonzero_solid: (&[0; Q2_RERELEASE_ENTITY_WORDS])[19] != 0,
                    baseline_solid: false,
                },
            )
        },
    )?;
    assert_eq!(
        snapshots::read_q2_kex(
            &mut Reader::new(&bytes[1..n], Encoding::Bytes),
            &mut ring,
            &mut context,
            |_| ThinkTime::Milliseconds(0)
        ),
        Err(Error::Count)
    );
    assert_eq!(ring.counts().accepted, 0);
    Ok(())
}

#[test]
fn demo_wire_precision_survives_an_older_snapshot_base_and_resets_on_remove() -> Result<(), Error> {
    let mut ring = Q2KexRing::load(4, 8192, 32, None)?;
    let mut context = Q2KexContext::load(&ring, true)?;
    let player = Q2KexPlayer::default();
    let mut transmit = states::Q2KexWire::default();
    let mut bytes = [0; 1400];
    let mut first = body(1, 1.25, false);
    first.words[18] = 0;
    let n = wire(1, -1, &player, &player, true, &mut bytes, |writer| {
        states::write_q2_kex_entity(
            writer,
            1,
            &[0; Q2_RERELEASE_ENTITY_WORDS],
            Some(&first.words),
            true,
            true,
            &mut transmit,
        )
    })?;
    assert!(receive(&mut ring, &mut context, &bytes[..n])?);
    let mut second = first;
    second.words[19] = 1;
    second.words[8] = 3.125f32.to_bits();
    let n = wire(2, 1, &player, &player, true, &mut bytes, |writer| {
        states::write_q2_kex_entity(
            writer,
            1,
            &first.words,
            Some(&second.words),
            false,
            true,
            &mut transmit,
        )
    })?;
    assert!(receive(&mut ring, &mut context, &bytes[..n])?);
    let mut third = first;
    third.words[8] = 7.03125f32.to_bits();
    // No solid delta against frame one, while the last wire solid is nonzero.
    let n = wire(3, 1, &player, &player, true, &mut bytes, |writer| {
        states::write_q2_kex_entity(
            writer,
            1,
            &first.words,
            Some(&third.words),
            false,
            true,
            &mut transmit,
        )
    })?;
    assert!(receive(&mut ring, &mut context, &bytes[..n])?);
    let decoded = ring.frame(3).ok_or(Error::Context)?;
    assert_eq!(decoded.entities[0].words[8], third.words[8]);
    assert_eq!(decoded.entities[0].words[19], 0);
    assert!(transmit.nonzero_solid);
    let n = wire(4, 3, &player, &player, true, &mut bytes, |writer| {
        states::write_q2_kex_entity(writer, 1, &third.words, None, false, true, &mut transmit)
    })?;
    assert!(receive(&mut ring, &mut context, &bytes[..n])?);
    assert!(ring.frame(4).ok_or(Error::Context)?.entities.is_empty());
    assert!(!transmit.nonzero_solid);
    // Insert from zero baseline after removal: native demo precision is low.
    let mut inserted = third;
    inserted.words[8] = (-1.01f32).to_bits();
    let n = wire(5, 4, &player, &player, true, &mut bytes, |writer| {
        states::write_q2_kex_entity(
            writer,
            1,
            &[0; Q2_RERELEASE_ENTITY_WORDS],
            Some(&inserted.words),
            true,
            true,
            &mut transmit,
        )
    })?;
    assert!(receive(&mut ring, &mut context, &bytes[..n])?);
    assert_eq!(
        ring.frame(5).ok_or(Error::Context)?.entities[0].words[8],
        (-1.0f32).to_bits()
    );
    Ok(())
}
