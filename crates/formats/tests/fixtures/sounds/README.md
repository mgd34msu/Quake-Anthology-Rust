The Ogg fixtures contain generated sine waves, not game audio. They were made
with FFmpeg's `libvorbis` encoder:

```sh
ffmpeg -f lavfi -i 'sine=frequency=440:sample_rate=22050:duration=0.13' -c:a libvorbis -q:a 3 mono.ogg
ffmpeg -f lavfi -i 'aevalsrc=0.2*sin(2*PI*330*t)|0.15*sin(2*PI*660*t):s=48000:d=0.19' -c:a libvorbis -q:a 3 stereo.ogg
ffmpeg -f lavfi -i 'sine=frequency=570:sample_rate=48000:duration=1.25' -c:a libvorbis -q:a 3 pages.ogg
```

Tests compare the retained samples with the pinned original Lewton decoder.
The ending granules define the output length, including streams whose audio
fits on one page and chained streams. Padding is excluded.
