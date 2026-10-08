#[path = "support/assets.rs"]
mod assets;
#[path = "support/retail.rs"]
mod retail;
use qa_formats::sound::{self, Wav, WavPolicy};
use std::{collections::BTreeMap, path::Path};

fn main() -> Result<(), String> {
    let root = std::env::args_os().nth(1).ok_or("qfiles root required")?;
    let evidence = std::env::args_os().nth(2).map(std::path::PathBuf::from);
    if let Some(ref path) = evidence {
        std::fs::create_dir_all(path).map_err(|e| e.to_string())?;
    }
    let vfs = assets::mount(Path::new(&root))?;
    let mut counts = BTreeMap::new();
    let mut failures = 0;
    let mut loops = 0;
    let mut frames = 0u64;
    let mut ambient = 0;
    for (reference, name) in vfs.files() {
        if !name.ends_with(b".wav") && !name.ends_with(b".ogg") {
            continue;
        }
        let mut bytes = vec![0; vfs.length(reference).map_err(|e| format!("{e:?}"))? as usize];
        vfs.read_at(reference, 0, &mut bytes)
            .map_err(|e| format!("{e:?}"))?;
        let origin = vfs.origin(reference).ok_or("origin")?;
        match sound::decode(&bytes, WavPolicy::Quake) {
            Ok(pcm) => {
                let format = if bytes.starts_with(b"OggS") {
                    "Vorbis"
                } else {
                    "WAV"
                };
                *counts.entry(format).or_insert(0usize) += 1;
                frames += pcm.frames() as u64;
                loops += usize::from(pcm.loop_start.is_some());
                if name.windows(7).any(|p| p == b"ambient") && bytes.starts_with(b"RIFF") {
                    let wav = Wav::parse(&bytes, WavPolicy::Quake).map_err(|e| format!("{e:?}"))?;
                    if wav.channels == qa_core::primitives::PcmChannels::Mono && wav.width <= 2 {
                        if let Some(ref root) = evidence {
                            std::fs::write(root.join(format!("{ambient}.wav")), &bytes)
                                .map_err(|e| e.to_string())?;
                            let info = format!(
                                "{} {} {} {} {} {}\n",
                                wav.rate,
                                wav.channels as u8,
                                wav.width,
                                wav.frames,
                                wav.loop_start.map_or(-1, |i| i as i64),
                                wav.data_offset
                            );
                            std::fs::write(root.join(format!("{ambient}.info")), info)
                                .map_err(|e| e.to_string())?;
                        }
                        ambient += 1;
                    }
                }
            }
            Err(error) => {
                failures += 1;
                println!(
                    "failed {:?}:{:?} bytes={} {error:?}",
                    origin.path,
                    String::from_utf8_lossy(origin.member),
                    bytes.len()
                );
            }
        }
    }
    println!(
        "scope=headless sound decode, not playback; counts={counts:?} failures={failures} looped={loops} frames={frames} ambient_native={ambient}"
    );
    if failures > 0 {
        return Err(format!("{failures} sounds failed"));
    }
    Ok(())
}
