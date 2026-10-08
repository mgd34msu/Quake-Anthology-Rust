//! Headless comparison with extracted original Q3 source resampling.
use qa_core::primitives::{Pcm, PcmChannels};
use std::{fs, num::NonZeroU32, path::PathBuf};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(std::env::args_os().nth(1).ok_or("evidence root required")?);
    let input = fs::read(root.join("resample.bin"))?;
    let expected = fs::read(root.join("resample.raw"))?;
    for name in ["hero", "fem_quad30", "male_quad30"] {
        if !root.join(format!("{name}.wav")).exists() {
            continue;
        }
        let bytes = fs::read(root.join(format!("{name}.wav")))?;
        let wav = qa_formats::sound::Wav::parse(&bytes, qa_formats::sound::WavPolicy::Quake)
            .map_err(|e| format!("hero: {e:?}"))?;
        let info = fs::read_to_string(root.join(format!("{name}.info")))?;
        let values: Vec<i64> = info
            .split_whitespace()
            .map(str::parse)
            .collect::<Result<_, _>>()?;
        let rust = [
            i64::from(wav.rate.get()),
            wav.channels as i64,
            i64::from(wav.width),
            (wav.frames * wav.channels as usize) as i64,
            wav.loop_start.map_or(-1, |i| i as i64),
            wav.data_offset as i64,
        ];
        assert_eq!(rust.as_slice(), values);
        assert!(wav.riff_length_ignored);
        let pcm = wav.decode();
        if name == "hero" {
            assert_eq!(pcm.frames(), 242528);
        } else {
            assert_eq!(pcm.frames(), 329642);
            assert!(wav.zero_tail_ignored);
        }
        println!(
            "PASS: {name}.wav original metadata; interleaved_samples={} frames={} complete PCM admitted",
            pcm.samples.len(),
            pcm.frames()
        );
    }
    let (mut at, mut out, mut cases) = (0, 0, 0);
    while at < input.len() {
        let rate =
            NonZeroU32::new(u32::from_le_bytes(input[at..at + 4].try_into()?)).ok_or("rate")?;
        let output =
            NonZeroU32::new(u32::from_le_bytes(input[at + 4..at + 8].try_into()?)).ok_or("rate")?;
        let width = u32::from_le_bytes(input[at + 8..at + 12].try_into()?) as usize;
        at += 12;
        let samples = input[at..at + 64 * width]
            .chunks_exact(width)
            .map(|p| {
                if width == 1 {
                    (i16::from(p[0]) - 128) * 256
                } else {
                    i16::from_le_bytes([p[0], p[1]])
                }
            })
            .collect();
        at += 64 * width;
        let source = Pcm {
            rate,
            channels: PcmChannels::Mono,
            samples,
            loop_start: None,
        };
        let pcm = qa_audio::prepare(source, output, false).map_err(|e| format!("{e:?}"))?;
        let count = u32::from_le_bytes(expected[out..out + 4].try_into()?) as usize;
        out += 4;
        assert_eq!(count, pcm.frames());
        let values: Vec<_> = expected[out..out + count * 2]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| i16::from_le_bytes(*p))
            .collect();
        assert_eq!(values, pcm.samples);
        out += count * 2;
        cases += 1;
    }
    assert_eq!(out, expected.len());
    assert_eq!(cases, 60);
    println!("PASS: 60 original-C ResampleSfxRaw cases, frames and signed PCM bit-identical");
    Ok(())
}
