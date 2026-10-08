# Platform console input

The Linux platform reopens stdin with an independent nonblocking file
description, retaining inherited flags and a regular file's starting offset.
Pipes and terminals use the same reader. Other operating systems currently have
no stdin source; they still consume the shared ConsoleLine event kind.

Each event poll consumes at most 4,096 bytes and emits at most eight lines,
leaving room for the final Time event. This includes the first and second
Com_Frame drains and polling during the capped wait. Partial lines survive
across polls. CR, LF and CR/LF terminate a line; EOF emits a final unterminated
line once. Empty lines are skipped. Lines over 8,191 bytes, containing NUL or
invalid UTF-8 are discarded whole. Ring backpressure retains a completed line
until it can be copied into the one system-event arena.

The host appends ConsoleLine to its existing fixed console buffer. Commands,
cvars, vstr and exec therefore use the same path as local console input for
all module families. No terminal reader, input recording or replay is added
outside platform. The checker already rejects std::io::stdin and imported
aliases outside platform, including examples.

```sh
python3 tools/check_stdin.py --binary "$QA_CANDIDATE" --owner-profile "$QA_PROFILE" --evidence "$QA_EVIDENCE/stdin-pipe"
python3 tools/check_stdin.py --tty --binary "$QA_CANDIDATE" --owner-profile "$QA_PROFILE" --evidence "$QA_EVIDENCE/stdin-tty"
```

These checks create an owned pipe or canonical PTY on the private display.
They exercise a split command, an oversized line, invalid UTF-8, EOF and the
idle terminal path, plus cvar/vstr and output dispatch. They preserve the
candidate and owner profile. Allocation qualification covers 600 instrumented
Rust frames. Current evidence is the window shell; per-game and combined
gameplay acceptance remains at the R3.5 gate.
