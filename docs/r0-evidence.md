# R0 evidence

Evidence runs live outside the repository under the local qa-rust cache. The
Linear issue comments identify exact folders and commits.

The fresh workspace opens an SDL window and exits after a bounded frame count.
The private harness runs a byte-equal candidate copy on owned Xvfb with Openbox,
copies all 34 saved owner settings into a private HOME, records 14 real X server
auto-repeat keydown events from a held key, captures the window, exits normally,
and leaves no owned process running. The original settings remain byte-equal.

This proves the R0 window and harness lifecycle. It does not prove map gameplay,
an engine renderer, sound output or installation qualification. Audio output is
absent because the shell does not yet open an audio device; the summary reports
that fact. Map and sound evidence will come from the playable milestones.

Run a fresh proof after building:

```sh
python3 tools/build.py
python3 tools/private_run.py --binary target/candidate/qa-rust \
  --owner-profile "$PROFILE" --evidence "$EVIDENCE" -- --frames 180
```

PROFILE is the owner's existing settings directory. EVIDENCE must be a fresh
directory outside qfiles and the original profile. Optional input is a JSON
array such as `[{"key":"w","hold_seconds":0.9,"mouse":[12,4]}]`, passed with
`--input`. The X server generates repeat events while the key is held.

Xvfb is software-only. Its measurements do not qualify the hardware GL target.
