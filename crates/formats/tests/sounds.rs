use qa_formats::sound::{Wav, WavPolicy};

fn chunk(bytes: &mut Vec<u8>, name: &[u8; 4], data: &[u8]) {
    bytes.extend(name);
    bytes.extend((data.len() as u32).to_le_bytes());
    bytes.extend(data);
    if !data.len().is_multiple_of(2) {
        bytes.push(0);
    }
}
fn wav(width: u16, channels: u16) -> Vec<u8> {
    let mut bytes = b"RIFF\0\0\0\0WAVE".to_vec();
    let mut format = Vec::new();
    for value in [1u16, channels] {
        format.extend(value.to_le_bytes());
    }
    format.extend(11025u32.to_le_bytes());
    format.extend((11025u32 * u32::from(channels * width / 8)).to_le_bytes());
    format.extend((channels * width / 8).to_le_bytes());
    format.extend(width.to_le_bytes());
    chunk(&mut bytes, b"fmt ", &format);
    let mut cue = vec![0; 28];
    cue[0..4].copy_from_slice(&1u32.to_le_bytes());
    cue[24..28].copy_from_slice(&2u32.to_le_bytes());
    chunk(&mut bytes, b"cue ", &cue);
    let mut mark = vec![0; 24];
    mark[16..20].copy_from_slice(&3u32.to_le_bytes());
    mark[20..24].copy_from_slice(b"mark");
    chunk(&mut bytes, b"LIST", &mark);
    let values: Vec<_> = (0..8 * channels).map(|i| i as u8 * 13).collect();
    let data: Vec<_> = values
        .iter()
        .flat_map(|v| match width {
            8 => vec![*v],
            16 => vec![*v, *v],
            _ => vec![7, *v, *v],
        })
        .collect();
    chunk(&mut bytes, b"data", &data);
    let size = bytes.len() as u32 - 8;
    bytes[4..8].copy_from_slice(&size.to_le_bytes());
    bytes
}
#[test]
fn native_cue_mark_trims_pcm_but_q3_ignores_loop_metadata() {
    let bytes = wav(8, 1);
    let sound = Wav::parse(&bytes, WavPolicy::Quake).unwrap();
    assert_eq!(sound.frames, 5);
    assert_eq!(sound.loop_start, Some(2));
    assert_eq!(
        sound.decode().samples,
        [-32768, -29440, -26112, -22784, -19456]
    );
    assert_eq!(sound.data.as_ptr(), bytes[sound.data_offset..].as_ptr());
    let q3 = Wav::parse(&bytes, WavPolicy::Quake3).unwrap();
    assert_eq!(q3.frames, 8);
    assert_eq!(q3.loop_start, None);
    let standard = Wav::parse(&bytes, WavPolicy::Standard).unwrap();
    assert_eq!(standard.frames, 8);
    assert_eq!(standard.loop_start, Some(2));
}
#[test]
fn stereo_and_24bit_convert_once_with_native_high_word() {
    let a = Wav::parse(&wav(16, 2), WavPolicy::Quake).unwrap().decode();
    let b = Wav::parse(&wav(24, 2), WavPolicy::Quake).unwrap().decode();
    assert_eq!(a, b);
    assert_eq!(a.frames(), 5);
    assert_eq!(a.samples.len(), 10);
}
#[test]
fn incomplete_trailing_info_is_admitted_only_by_native_policy() {
    let mut bytes = wav(16, 1);
    bytes.extend(b"LIST");
    bytes.extend(200u32.to_le_bytes());
    bytes.extend(b"INFO");
    let size = bytes.len() as u32 - 8;
    bytes[4..8].copy_from_slice(&size.to_le_bytes());
    assert!(
        Wav::parse(&bytes, WavPolicy::Quake)
            .unwrap()
            .info_tail_clamped
    );
    assert!(Wav::parse(&bytes, WavPolicy::Standard).is_err());
}
#[test]
fn native_riff_length_matches_physical_traversal_without_admitting_missing_pcm() {
    let mut bytes = wav(16, 1);
    let baseline = Wav::parse(&bytes, WavPolicy::Quake).unwrap().decode();
    let oversize = bytes.len() as u32 - 4;
    bytes[4..8].copy_from_slice(&oversize.to_le_bytes());
    let native = Wav::parse(&bytes, WavPolicy::Quake).unwrap();
    assert!(native.riff_length_ignored);
    assert_eq!(native.decode(), baseline);
    assert!(Wav::parse(&bytes, WavPolicy::Standard).is_err());
    bytes.pop();
    assert!(Wav::parse(&bytes, WavPolicy::Quake).is_err());
}
#[test]
fn native_zero_tail_never_admits_nonzero_or_missing_pcm_bytes() {
    let mut bytes = wav(16, 1);
    bytes.extend([0, 0, 0]);
    assert!(
        Wav::parse(&bytes, WavPolicy::Quake)
            .unwrap()
            .zero_tail_ignored
    );
    bytes.push(1);
    assert!(Wav::parse(&bytes, WavPolicy::Quake).is_err());
}
#[test]
fn wav_truncations_and_seeded_mutations_return_format_errors() {
    let original = wav(16, 1);
    for end in 0..original.len() {
        assert!(Wav::parse(&original[..end], WavPolicy::Quake).is_err());
    }
    let mut state = 0x534f554eu32;
    for _ in 0..10000 {
        let mut b = original.clone();
        state = state.wrapping_mul(1664525).wrapping_add(1013904223);
        let offset = state as usize % b.len();
        b[offset] ^= (state >> 24) as u8;
        let _ = Wav::parse(&b, WavPolicy::Quake);
    }
}
