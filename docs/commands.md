# Shared command execution

The host constructs one `Console`, containing one command buffer, one command
table, aliases and the shared cvar registry. Feature commands register static
function pointers into that table. Queued text retains its caller's source,
role, side, local seat and event time; it never selects another console or registry. The current app
constructs the command buffer only while loading the console. THE-887 uses a
65,536-byte array, load-allocated context spans, an 8,192-byte line scratch
and 1,024 reusable argv offsets. Byte shifts stay inside the array. Static
command names use core `NameTable` IDs. Its folded lookup resolves each command
token once, then numeric tables dispatch commands, cvars or aliases. The command
listing retains Q3 `Q_stricmp` order, including punctuation. Names keep their
exact spelling separately from their cached folded equivalence.
Aliases use load-selected slots (4,096 by default); callers can select their
session capacity before play. Native alias text is bounded to 1,024 bytes.
Capacity errors admit no partial alias or buffer change.
The console reserves name slots/bytes at load; alias replacement and removal
never allocate. Cvar flag members and native default roles also resolve at load.

`echo`, `wait`, `alias`, `unalias`, `exec`, `vstr`, `set`, `cmdlist`, `cvarlist`
and `quit` work through that table. Commands take precedence over cvars, then
aliases. Unknown text prints an error and never becomes chat. Aliases are
available in every source, including Q3, as the owner requires. An alias or
script inserts text ahead of the remaining buffer. `wait` defers execution
to later frames; repeated alias expansion is bounded as in Q2. Idle frames
do not tokenize, allocate command text or look up a cvar name.

Tokenizer rules come from the original Q1, QW, Q2 and Q3 command sources.
They preserve source-specific punctuation, comment handling, quote boundaries,
argument counts and borrowed argument tails. Native commands that combine argv
use fixed scratch to preserve quote stripping and spacing. Q2 expands cvar macros outside quotes
and rejects expansion loops within that command. Q2 rerelease currently uses
the common classic parser. NUL input, oversized text and unsafe quoted-token
lengths produce scoped errors. Q2 only enters macro expansion when a dollar
sign exists; its native length and unmatched-quote checks still apply. They do not abort the engine.

`exec` asks the host's one VFS for a mounted script, adding `.cfg` when needed.
The current host accepts UTF-8 script text up to the command-buffer capacity.
The host reads directly into loaded script scratch. Path normalization uses
fixed scratch, and compressed PK3 reads reuse caller-owned inflate state;
empty, large and subsequent streams reset it without frame allocations.
It performs no filesystem write. `cvarlist` prints every one of the 1,260 owner
definitions once; family definitions have their separately stored seat values
indented below the list. Private values are redacted.

```sh
python3 tools/check_commands.py --qsrc "$QA_QSRC" --evidence "$QA_EVIDENCE/commands"
cargo test -p qa-console --test commands
python3 tools/private_run.py --binary "$QA_CANDIDATE" --owner-profile "$QA_PROFILE" --evidence "$QA_EVIDENCE/console-q1" -- --console-source q1 --content "$QA_SCRIPT_PRODUCT" --commands 'alias a "echo hi; wait; echo there"; a; exec outer; fov 110; cg_fov; quit' --startup-hold-ms 2000 --frames 20
```

The headless comparison checks tokens, argument tails and delimiters on
10,060 seeded/fixed inputs against extracted original C helpers. It does not
claim the original unsafe line-copy/truncation behavior. Buffer and callback
checks cover insertion order, waits, nested scripts, source contexts, overflow
atomicity and scoped alias loops. The private command-source launch above
selects parsing/defaults for the window shell; it is not a Q1 map session.

A bare cvar assignment uses argv 1, including a quoted multiword value; extra
arguments are ignored as in Q1/Q2/Q3 Cvar_Command. Q3 `set` combines argv 2
onward in scratch. Q2 `set` uses argv 2 and accepts an optional `u`/`s` flag
through source-specific Cvar_FullSet semantics. The shared `set` extension in
Q1/QW uses the Q3 combining behavior. New user/guest cvars remain later work.

Cvar values and alias details live outside the compact hot numeric records.
Coupled/latch writes use loaded transaction storage; load-built dependency
lists refresh only affected cached projections and invalidate related details.
No per-command String/Vec allocation is needed within loaded capacities.
See `frame-times.md` for the matched real-console workload and its limits.

Interactive editing/focus, chat commands, feature handlers, config order and
archival, user/guest cvar registration and tab completion remain later R2/R8
work. `set` and `vstr` currently use registered stock cvars. THE-639 remains
In Progress until Q1/Q2/Q3 map sessions prove the live alias sequence through
the normal input path and the candidate qualifies for installation.

THE-888 registers `bind`, `unbind`, `unbindall`, `bindlist` and the native
movement/button names, including Q3 `+button0` through `+button14` and their
releases. Runtime owns the single Input table. A bind compiles known action
clauses once into cached Action values; dispatch never looks up their names.
Each loaded slot reserves 1,024 text bytes and 512 clause spans. Normal commands
and unknown `+` aliases enter the same fixed console buffer with the originating
seat and time. Hardware sources have unique key numbers in release metadata.
Quotes preserve semicolons within a command argument. Adding a complete input
line admits both its text and newline before changing the buffer.

One name table maps keyboard, mouse, wheel, JOY and AUX names and canonical Q3
hex numbers to neutral controls. Left/right modifiers share a config binding
and retain distinct held sources. Printable case/shift variants address the
same physical key; character events remain separate. Unnamed native hex keys
have reserved slots for module/device adapters.

Two keys may hold one action. Repeats preserve the first press time; key-up,
focus loss and device removal release acquired bindings. Rebinding a held key
releases its old action and waits for physical up before the replacement can
acquire it. Writing identical text preserves an existing hold. Manual minus
commands without a key number clear both holders. Wheel events produce one
momentary key pulse per nonzero axis.

`tools/check_binds.py` compares 364 named/canonical hex cases and 10,000 seeded
partial-frame hold results with extracted Q3 key/button functions. Host checks
exercise all five command-source views, aliases, two seats, rebinding and native
numbered-button projection. Private normal-candidate checks use real X-server
repeat on direct, alias and composite binds. These prove routing in the window
shell. Native movement scaling, per-seat cvar policies, centerview/editing,
menu capture and map/combined acceptance remain the following R2/R3.5 work.

```sh
python3 tools/check_binds.py --qsrc "$QA_QSRC" --evidence "$QA_EVIDENCE/binds"
cargo test -p qa-app --test binds
python3 tools/check_sys_events.py --binary "$QA_CANDIDATE" --owner-profile "$QA_PROFILE" --evidence "$QA_EVIDENCE/bind-q3" --console-source q3 --commands 'unbindall; bind w "+forward; echo bound"'
```
