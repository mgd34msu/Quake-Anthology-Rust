`tracks.pk3` contains the generated Vorbis sine fixtures from
`formats/tests/fixtures/sounds`. Members are deflated ZIP entries, including
`scores/theme.data` to check decoding by file contents with an arbitrary name.
The archive uses a fixed timestamp and contains no game audio.
The `music/intro.wav` and `music/repeat.wav` members also contain Vorbis data
to test the native module path without relying on a filename extension.
