The Ogg fixtures contain generated sine waves, not game audio. They were made
with FFmpeg's `libvorbis` encoder:

```sh
ffmpeg -f lavfi -i 'sine=frequency=440:sample_rate=22050:duration=0.13' -c:a libvorbis -q:a 3 mono.ogg
ffmpeg -f lavfi -i 'aevalsrc=0.2*sin(2*PI*330*t)|0.15*sin(2*PI*660*t):s=48000:d=0.19' -c:a libvorbis -q:a 3 stereo.ogg
```

Tests compare every decoded sample with the pinned original Lewton decoder,
including chained streams and final-page granule trimming.
