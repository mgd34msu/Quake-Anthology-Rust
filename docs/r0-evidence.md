# R0 evidence

Evidence runs live outside the repository under the local qa-rust cache. The
Linear issue comments identify exact folders and commits.

The fresh workspace opens an SDL window and exits after a bounded frame count.
The private harness runs a byte-equal candidate copy on owned Xvfb with Openbox,
copies all 34 saved owner settings into a private HOME, handles 24 real X server
auto-repeat keydown events from a held key, captures the window, exits normally,
and leaves no owned process running. SDL reports x11 and no WAYLAND_DISPLAY.
The original settings remain byte-equal. Normal input emits no per-event prints;
`+set developer 1` enables the shared logger's diagnostics.

This proves the R0 window and harness lifecycle. It does not prove map gameplay,
an engine renderer, sound output or installation qualification. Audio output is
absent because the shell does not yet open an audio device; the summary reports
that fact. Map and sound evidence will come from the playable milestones.

Run a fresh proof after building:

```sh
timeout 300 python3 tools/build.py
timeout 300 python3 tools/private_run.py --binary target/candidate/qa-rust \
  --owner-profile "$PROFILE" --evidence "$EVIDENCE" -- --frames 180
```

PROFILE is the owner's existing settings directory. EVIDENCE must be a fresh
directory outside qfiles and the original profile. Optional input is a JSON
array such as `[{"key":"w","hold_seconds":0.9,"mouse":[12,4]}]`, passed with
`--input`. The X server generates repeat events while the key is held.

Xvfb is software-only. Its measurements do not qualify the hardware GL target.

`tools/build.py --proof` creates a separate development candidate. Timed script
events enter the normal SDL queue. The normal candidate rejects `--proof-script`
and has no symbols containing proof. Walking e1m1 still awaits R4 gameplay.

The rule checker rejects planted violations of all 12 rules before Cargo runs,
including duplicate primitives, crypto code, family gates and oversized test
modules. The main source passes. The checker is a lexical guard; source review
and measured runtime evidence remain required.

The four retired target directories are removed. The owner retired all Muse
branches and worktrees under THE-626; only main and muse-final remain on origin.
The reset commit is a normal child of eb9118cc. No history was rewritten.

R0 remains partial: map/audio runs, a qualified installation receipt, game
renderer timing, movement/trace/save checks and e1m1 scripted input are pending.
The installer refuses the shell because it never reaches gameplay. No shell
binary is installed into qfiles and no installation Slack notice is sent.
