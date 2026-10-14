use std::io::Cursor;

#[global_allocator]
static ALLOCATOR: qa_platform::allocations::CountingAllocator =
    qa_platform::allocations::CountingAllocator;

use ogg::{PacketReader, PacketWriteEndInfo, PacketWriter};
use qa_formats::{
    FormatError,
    sound::{
        self,
        ogg::{self as framing, Packet, Packets, Plan},
    },
};

type Row = (Vec<u8>, u32, Option<u64>, bool, bool, bool);

fn row(packet: Packet<'_>) -> Row {
    (
        packet.data.to_vec(),
        packet.serial,
        packet.granule,
        packet.first_in_stream,
        packet.last_in_stream,
        packet.last_in_page,
    )
}

fn plan(bytes: &[u8]) -> Result<Plan, FormatError> {
    Plan::inspect(bytes.len() as u64, |at, out| {
        framing::slice_read(bytes, at, out)
    })
}

fn read(bytes: &[u8]) -> Vec<Row> {
    let mut reader = Packets::load(plan(bytes).unwrap()).unwrap();
    let mut rows = Vec::new();
    while let Some(packet) = reader.next_slice(bytes).unwrap() {
        rows.push(row(packet));
    }
    rows
}

fn reference(bytes: &[u8]) -> Vec<Row> {
    let mut reader = PacketReader::new(Cursor::new(bytes));
    let mut rows = Vec::new();
    while let Some(packet) = reader.read_packet().unwrap() {
        let granule = packet.absgp_page();
        let metadata = (
            packet.stream_serial(),
            (granule != u64::MAX).then_some(granule),
            packet.first_in_stream(),
            packet.last_in_stream(),
            packet.last_in_page(),
        );
        rows.push((
            packet.data,
            metadata.0,
            metadata.1,
            metadata.2,
            metadata.3,
            metadata.4,
        ));
    }
    rows
}

fn checksum(bytes: &mut [u8]) {
    bytes[22..26].fill(0);
    let checksum = framing::checksum(bytes);
    bytes[22..26].copy_from_slice(&checksum.to_le_bytes());
}

fn page(
    flags: u8,
    serial: u32,
    sequence: u32,
    granule: Option<u64>,
    laces: &[u8],
    body: &[u8],
) -> Vec<u8> {
    assert_eq!(laces.iter().map(|&v| v as usize).sum::<usize>(), body.len());
    let mut bytes = b"OggS\0".to_vec();
    bytes.push(flags);
    bytes.extend(granule.unwrap_or(u64::MAX).to_le_bytes());
    bytes.extend(serial.to_le_bytes());
    bytes.extend(sequence.to_le_bytes());
    bytes.extend([0; 4]);
    bytes.push(laces.len() as u8);
    bytes.extend(laces);
    bytes.extend(body);
    checksum(&mut bytes);
    bytes
}

#[test]
fn original_writer_pages_match_integrity_packets_and_metadata() {
    let mut writer = PacketWriter::new(Vec::new());
    let sizes = [0, 1, 254, 255, 256, 65025, 130050, 2];
    for (index, size) in sizes.into_iter().enumerate() {
        let bytes: Box<[_]> = (0..size).map(|value| (value * 19) as u8).collect();
        writer
            .write_packet(
                bytes,
                19,
                if index + 1 == sizes.len() {
                    PacketWriteEndInfo::EndStream
                } else if index == 0 {
                    PacketWriteEndInfo::EndPage
                } else {
                    PacketWriteEndInfo::NormalPacket
                },
                index as u64,
            )
            .unwrap();
    }
    let bytes = writer.into_inner();
    let requirements = plan(&bytes).unwrap();
    assert_eq!(requirements.streams, 1);
    assert_eq!(requirements.packet_bytes, 130050);
    let rows = read(&bytes);
    assert_eq!(
        rows.iter().map(|row| row.0.len()).collect::<Vec<_>>(),
        sizes
    );
    assert_eq!(rows, reference(&bytes));
}

#[test]
fn interleaved_partial_streams_keep_separate_reusable_packet_storage() {
    let mut bytes = page(2, 31, 0, None, &[255], &[1; 255]);
    bytes.extend(page(2, 32, 0, Some(0), &[1], &[7]));
    bytes.extend(page(5, 31, 1, Some(1), &[0], &[]));
    bytes.extend(page(4, 32, 1, Some(2), &[2], &[8, 9]));
    assert_eq!(plan(&bytes).unwrap().streams, 2);
    let rows = read(&bytes);
    let mut expected = reference(&bytes);
    // The old reader derives this flag from the completion page's BOS bit.
    // The first logical packet remains first when its final lace is on page 1.
    assert!(!expected[1].3);
    expected[1].3 = true;
    assert_eq!(rows, expected);
    assert_eq!(rows[1].0, [1; 255]);
    assert!(rows[1].3 && rows[1].4);
}

#[test]
fn last_completed_packet_gets_granule_even_with_a_partial_packet_after_it() {
    let mut bytes = page(2, 99, 0, Some(3), &[1, 255], &[4; 256]);
    bytes.extend(page(5, 99, 1, Some(8), &[2, 0], &[5, 6]));
    let rows = read(&bytes);
    assert_eq!(rows, reference(&bytes));
    assert!(rows[0].5);
    assert_eq!(rows[0].2, Some(3));
    assert!(!rows[1].5);
    assert!(rows[2].4 && rows[2].5);
    assert!(rows[2].0.is_empty());
}

#[test]
fn chained_streams_reuse_storage_and_rewind_restores_packets() {
    let mut bytes = page(6, 100, 0, Some(2), &[2], &[10, 20]);
    bytes.extend(page(6, 101, 0, Some(3), &[3], &[30, 40, 50]));
    assert_eq!(plan(&bytes).unwrap().streams, 1);
    let mut reader = Packets::load(plan(&bytes).unwrap()).unwrap();
    let first = reader.next_slice(&bytes).unwrap().unwrap();
    let pointer = first.data.as_ptr();
    assert_eq!(first.data, [10, 20]);
    let second = reader.next_slice(&bytes).unwrap().unwrap();
    assert_eq!(second.data.as_ptr(), pointer);
    assert_eq!(second.data, [30, 40, 50]);
    assert!(reader.next_slice(&bytes).unwrap().is_none());
    assert_eq!(reader.position(), bytes.len() as u64);
    reader.rewind();
    assert_eq!(reader.position(), 0);
    let again = reader.next_slice(&bytes).unwrap().unwrap();
    assert_eq!(again.data.as_ptr(), pointer);
    assert_eq!(again.data, [10, 20]);
    assert_eq!(read(&bytes), reference(&bytes));
}

#[test]
fn completed_page_without_eos_is_read_but_partial_packets_and_pages_reject() {
    let complete = page(2, 1, 0, Some(0), &[2], &[5, 6]);
    assert_eq!(read(&complete), reference(&complete));
    let incomplete = page(2, 1, 0, None, &[255], &[5; 255]);
    assert_eq!(plan(&incomplete).unwrap_err(), FormatError::Truncated);
    for end in 0..complete.len() {
        assert!(plan(&complete[..end]).is_err(), "prefix {end}");
    }
    let mut trailing = complete;
    trailing.extend(b"Og");
    assert_eq!(plan(&trailing).unwrap_err(), FormatError::Truncated);
}

#[test]
fn bad_integrity_sequence_continuation_and_stream_lifetimes_reject() {
    let valid = page(6, 41, 0, Some(1), &[1], &[7]);
    for index in [0, 6, 14, 18, 22, 27, 28] {
        let mut bytes = valid.clone();
        bytes[index] ^= 1;
        assert!(plan(&bytes).is_err(), "corruption at {index}");
    }
    for bytes in [
        page(0, 41, 0, Some(1), &[1], &[7]),
        page(3, 41, 0, Some(1), &[1], &[7]),
        page(6, 41, 1, Some(1), &[1], &[7]),
        page(14, 41, 0, Some(1), &[1], &[7]),
        page(6, 41, 0, None, &[255], &[7; 255]),
        [
            page(2, 41, 0, None, &[255], &[7; 255]),
            page(4, 41, 1, Some(1), &[0], &[]),
        ]
        .concat(),
        [
            page(2, 41, 0, Some(1), &[1], &[7]),
            page(4, 41, 2, Some(2), &[1], &[8]),
        ]
        .concat(),
        [valid.clone(), page(4, 41, 1, Some(2), &[1], &[8])].concat(),
        [valid.clone(), valid.clone()].concat(),
    ] {
        assert!(plan(&bytes).is_err());
    }
    let mut version = valid;
    version[4] = 1;
    checksum(&mut version);
    assert_eq!(plan(&version).unwrap_err(), FormatError::Unsupported);
}

#[test]
fn source_errors_and_insufficient_plan_are_bounded() {
    let bytes = page(6, 5, 0, Some(1), &[3], &[1, 2, 3]);
    assert_eq!(
        Plan::inspect(bytes.len() as u64, |_, _| Err(FormatError::InvalidRange)).unwrap_err(),
        FormatError::InvalidRange
    );
    let mut reader = Packets::load(Plan {
        streams: 1,
        packet_bytes: 1,
    })
    .unwrap();
    assert_eq!(
        reader.next_slice(&bytes).unwrap_err(),
        FormatError::InvalidRange
    );
    assert!(
        Packets::load(Plan {
            streams: 0,
            packet_bytes: 1
        })
        .is_err()
    );
    assert!(
        Packets::load(Plan {
            streams: usize::MAX,
            packet_bytes: 2
        })
        .is_err()
    );
}

fn reference_pcm(bytes: &[u8]) -> (u32, u8, Vec<i16>) {
    let mut reader =
        lewton_reference::inside_ogg::OggStreamReader::new(Cursor::new(bytes)).unwrap();
    let format = (
        reader.ident_hdr.audio_sample_rate,
        reader.ident_hdr.audio_channels,
    );
    let mut samples = Vec::new();
    while let Some(packet) = reader.read_dec_packet_itl().unwrap() {
        assert_eq!(
            (
                reader.ident_hdr.audio_sample_rate,
                reader.ident_hdr.audio_channels
            ),
            format
        );
        samples.extend(packet);
    }
    (format.0, format.1, samples)
}

const MONO: &[u8] = include_bytes!("fixtures/sounds/mono.ogg");
const STEREO: &[u8] = include_bytes!("fixtures/sounds/stereo.ogg");
const PAGES: &[u8] = include_bytes!("fixtures/sounds/pages.ogg");

fn change_serial(bytes: &[u8], serial: u32) -> Vec<u8> {
    let mut bytes = bytes.to_vec();
    let mut at = 0;
    while at < bytes.len() {
        let segments = bytes[at + 26] as usize;
        let end = at
            + 27
            + segments
            + bytes[at + 27..at + 27 + segments]
                .iter()
                .map(|&n| n as usize)
                .sum::<usize>();
        bytes[at + 14..at + 18].copy_from_slice(&serial.to_le_bytes());
        checksum(&mut bytes[at..end]);
        at = end;
    }
    bytes
}

#[test]
fn vorbis_keeps_exact_dsp_samples_and_authored_lengths_in_short_multi_page_and_chained_streams() {
    for (source, frames) in [(MONO, 2867), (STEREO, 9120), (PAGES, 60000)] {
        let (rate, channels, mut expected) = reference_pcm(source);
        assert!(expected.len() >= frames * channels as usize);
        expected.truncate(frames * channels as usize);
        for repeats in [1, 2] {
            let bytes = if repeats == 1 {
                source.to_vec()
            } else {
                [change_serial(source, 7), change_serial(source, 8)].concat()
            };
            let actual = sound::vorbis(&bytes).unwrap();
            assert_eq!(actual.rate.get(), rate);
            assert_eq!(actual.channels as u8, channels);
            assert_eq!(actual.frames(), frames * repeats);
            assert_eq!(actual.samples, expected.repeat(repeats));
            assert!(actual.samples.iter().any(|&sample| sample != 0));
            assert_eq!(read(&bytes), reference(&bytes));
        }
    }
}

#[test]
fn changed_chain_pcm_format_is_rejected() {
    let bytes = [change_serial(MONO, 21), change_serial(STEREO, 22)].concat();
    assert_eq!(sound::vorbis(&bytes).unwrap_err(), FormatError::Unsupported);
}

#[test]
fn hostile_granule_cannot_overflow_the_decoded_sample_position() {
    let mut reader = PacketReader::new(Cursor::new(MONO));
    let mut packets = Vec::new();
    while let Some(packet) = reader.read_packet().unwrap() {
        packets.push(packet);
    }
    assert!(packets.len() > 5);
    let count = packets.len();
    let mut writer = PacketWriter::new(Vec::new());
    for (index, packet) in packets.into_iter().enumerate() {
        let granule = if index == 3 {
            u64::MAX - 2
        } else {
            packet.absgp_page()
        };
        let end = if index + 1 == count {
            PacketWriteEndInfo::EndStream
        } else if index < 4 {
            PacketWriteEndInfo::EndPage
        } else {
            PacketWriteEndInfo::NormalPacket
        };
        writer
            .write_packet(packet.data.into_boxed_slice(), 90, end, granule)
            .unwrap();
    }
    assert_eq!(
        sound::vorbis(&writer.into_inner()).unwrap_err(),
        FormatError::InvalidRange
    );
}

#[test]
fn interleaved_non_audio_packets_do_not_change_vorbis_pcm() {
    let mut bytes = page(2, 60, 0, Some(0), &[3], b"tag");
    bytes.extend(MONO);
    bytes.extend(page(4, 60, 1, Some(1), &[3], b"end"));
    assert_eq!(
        sound::vorbis(&bytes).unwrap().samples,
        sound::vorbis(MONO).unwrap().samples
    );
}

#[test]
fn source_reader_reuses_loaded_headers_and_rewinds_chains_without_whole_file_pcm() {
    let bytes = [change_serial(STEREO, 200), change_serial(STEREO, 100)].concat();
    let mut largest_read = 0;
    let mut decoder = sound::Vorbis::load(bytes.len() as u64, |at, out| {
        largest_read = largest_read.max(out.len());
        framing::slice_read(&bytes, at, out)
    })
    .unwrap();
    assert!(largest_read < bytes.len());
    assert_eq!(decoder.logical_streams(), 2);
    assert_eq!(decoder.source_position(), 0);
    assert_eq!(decoder.format().rate.get(), 48000);
    assert_eq!(decoder.format().channels as usize, 2);
    assert!(!decoder.comments().vendor.is_empty());
    let expected = sound::vorbis(&bytes).unwrap();
    for _ in 0..2 {
        let mut actual = Vec::new();
        let mut packets = 0;
        while let Some(samples) = decoder
            .read_packet(|at, out| framing::slice_read(&bytes, at, out))
            .unwrap()
        {
            assert!(!samples.is_empty());
            assert!(samples.len() < expected.samples.len());
            actual.extend_from_slice(samples);
            packets += 1;
        }
        assert!(packets > 2);
        assert_eq!(actual, expected.samples);
        assert_eq!(decoder.source_position(), bytes.len() as u64);
        decoder.rewind();
        assert_eq!(decoder.source_position(), 0);
    }
}

#[test]
fn source_reader_reports_io_errors_and_can_restart_after_failure() {
    let mut decoder = sound::Vorbis::load(MONO.len() as u64, |at, out| {
        framing::slice_read(MONO, at, out)
    })
    .unwrap();
    assert_eq!(
        decoder
            .read_packet(|_, _| Err(FormatError::Io(std::io::ErrorKind::UnexpectedEof)))
            .unwrap_err(),
        FormatError::Io(std::io::ErrorKind::UnexpectedEof)
    );
    decoder.rewind();
    assert!(decoder.read_slice(MONO).unwrap().is_some());
    assert!(
        sound::Vorbis::load(MONO.len() as u64, |_, _| Err(FormatError::Io(
            std::io::ErrorKind::Other
        )))
        .is_err()
    );
}

#[test]
fn loaded_vorbis_decodes_first_packets_and_rewinds_without_heap_growth() {
    for bytes in [
        MONO.to_vec(),
        STEREO.to_vec(),
        PAGES.to_vec(),
        [change_serial(STEREO, 200), change_serial(STEREO, 100)].concat(),
    ] {
        let expected = sound::vorbis(&bytes).unwrap();
        let mut decoder = sound::Vorbis::load(bytes.len() as u64, |at, out| {
            framing::slice_read(&bytes, at, out)
        })
        .unwrap();
        qa_platform::allocations::begin_frame();
        for _ in 0..3 {
            let mut at = 0;
            while let Some(samples) = decoder.read_slice(&bytes).unwrap() {
                assert_eq!(samples, &expected.samples[at..at + samples.len()]);
                at += samples.len();
            }
            assert_eq!(at, expected.samples.len());
            decoder.rewind();
        }
        let counts = qa_platform::allocations::end_frame();
        assert_eq!(counts, qa_platform::allocations::Counts::default());
    }
}

#[test]
#[ignore = "optional full retail music PCM comparison; set QA_TEST_OGG"]
fn retail_pcm_matches_existing_decoder() {
    let bytes =
        std::fs::read(std::env::var("QA_TEST_OGG").expect("set QA_TEST_OGG to a retail Ogg track"))
            .unwrap();
    let (rate, channels, mut samples) = reference_pcm(&bytes);
    let mut packets = PacketReader::new(Cursor::new(&bytes));
    let mut ending = None;
    while let Some(packet) = packets.read_packet().unwrap() {
        if packet.last_in_stream() {
            assert!(
                ending.is_none(),
                "retail comparison expects one logical stream"
            );
            ending = Some(packet.absgp_page() as usize);
        }
    }
    let frames = ending.expect("retail track has an ending granule");
    samples.truncate(frames * channels as usize);
    let actual = sound::vorbis(&bytes).unwrap();
    assert_eq!(actual.frames(), frames);
    assert_eq!(actual.rate.get(), rate);
    assert_eq!(actual.channels as u8, channels);
    assert_eq!(actual.samples, samples);
}
