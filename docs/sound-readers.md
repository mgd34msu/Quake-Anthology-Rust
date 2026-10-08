# Shared sound readers and preparation

`qa_formats::sound` parses borrowed WAV PCM and decodes Ogg Vorbis into the
core's `Pcm`: one interleaved signed 16-bit array, a nonzero sample rate,
mono/stereo channel count and an optional loop frame. WAV supports 8-bit
unsigned, 16-bit signed and the C port's 24-bit high-word conversion. The
Vorbis decoder is Rust. Decoded PCM admission is limited to 256 MiB per file.
Chained Vorbis links must retain the same rate and channels.

WAV policy is selected once at load. Quake/Q2 preserves `cue `/Sound Forge
`LIST mark` loop lengths and native sampler metadata. Q3 ignores loop metadata.
Native policies traverse the physical file, as `GetWavinfo` does, and preserve
the C port's incomplete trailing `LIST INFO` handling. Standard policy enforces
the declared RIFF bounds. Required chunk payloads and PCM frames remain bounded
in every policy; ignoring outer metadata never admits missing PCM bytes.

THE-853 covers Q2 `hero.wav`: both owned copies contain 970,156 bytes and a
complete PCM chunk, but declare a RIFF end four bytes later. The native reader
admits 242,528 stereo frames. Original `GetWavinfo` reports 485,056 interleaved
samples, with the same 44,100 Hz rate, two channels, 16-bit width and byte 44
data offset. This common reader supports stereo; original mono-only sound
cache restrictions are a later playback policy, not a format limitation.
THE-854 preserves the two LMCTF `quad30.wav` radio files' single trailing NUL.
After complete required chunks, the native reader can ignore a short all-zero
tail; nonzero short tails and missing PCM remain format errors.

`qa_audio::prepare` consumes this PCM once at precache. Matching device rates
reuse the original buffer. Other rates use the original float ratio and 8-bit
fractional nearest-sample clock, widened for long sounds; loop positions scale
with the same ratio. The Q1 8-bit option quantises the prepared sample values.
Stereo stays interleaved. The future mixer consumes prepared samples and does
not invoke decoding or resampling per call.

## Verification

```sh
cargo test -p qa-formats --test sounds
cargo test -p qa-audio
cargo run --release -p qa-content --example sounds -- "$QA_QFILES" "$QA_AMBIENT_EVIDENCE"
python3 tools/check_sounds.py --qsrc "$QA_QSRC" --ambient "$QA_AMBIENT_EVIDENCE" --output "$QA_EVIDENCE"
```

The comparison tool extracts unchanged original Q1 `GetWavinfo` and Q3
`ResampleSfxRaw`, then compiles a headless C helper outside the workspace.
All 140 copied ambient WAV entries match native rate, channels, width, sample
count, loop position and data offset. Sixty 8/16-bit resampling cases across
six source rates and five output rates match original C output lengths and
signed PCM bit for bit. The optional `--extra-wav` argument checks the retail
`hero.wav` metadata, including stereo frame versus interleaved sample counts.
The full 2026-10-07 corpus decoded all 9,312 physical sound entries: 8,622 WAV
and 690 Vorbis, including loose files and archive duplicates. There were zero
failures, 615 looped WAV entries and 1,901,963,333 decoded frames in total.

Fixtures check cue/mark trimming, Q3 loop suppression, stereo and 24-bit
conversion, incomplete trailing INFO, physical RIFF traversal, zero-copy
same-rate preparation and interleaved loop scaling. WAV truncations and 10,000
seeded mutations return format errors without panicking.

These checks prove decoding and source preparation. They do not prove a mixer,
playback, spatial attenuation, captured gameplay sound or an installation.
Streaming music belongs to the later audio milestone. Vorbis corpus admission
does not claim bit-identical floating-point decoding to libvorbis.

## Sources

Original `quake/WinQuake/snd_mem.c` (`GetWavinfo`, `ResampleSfx`),
`quake-2/client/snd_mem.c` and
`quake-iii-arena/code/client/snd_mem.c` (`ResampleSfxRaw`). Proven conversion,
sampler and trailing INFO behavior follows C `src/audio/codecs.c`, re-homed
onto shared PCM and boundary policies. No retired audio implementation was
transplanted.
