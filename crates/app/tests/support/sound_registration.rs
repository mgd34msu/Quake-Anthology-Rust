/// A tiny nonempty native PCM WAV, shared by loader and real module fixtures.
pub fn wav() -> Vec<u8> {
    wav_channels(1)
}

pub fn wav_channels(channels: u16) -> Vec<u8> {
    let samples = [1024i16, -2048, 4096, 0];
    let mut bytes = b"RIFF".to_vec();
    bytes.extend((36u32 + samples.len() as u32 * 2).to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16u32.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(channels.to_le_bytes());
    bytes.extend(44100u32.to_le_bytes());
    bytes.extend((88200u32 * u32::from(channels)).to_le_bytes());
    bytes.extend((2 * channels).to_le_bytes());
    bytes.extend(16u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend((samples.len() as u32 * 2).to_le_bytes());
    for sample in samples {
        bytes.extend(sample.to_le_bytes());
    }
    bytes
}
