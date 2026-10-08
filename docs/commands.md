# Shared command execution

The host constructs one `Console`, containing one command buffer, one command
table, aliases and the shared cvar registry. Feature commands register static
function pointers into that table. Queued text retains its caller's source,
role and side; it never selects another console or registry. The current app
constructs the command buffer only in `Console::new`.

`echo`, `wait`, `alias`, `unalias`, `exec`, `vstr`, `set`, `cmdlist`, `cvarlist`
and `quit` work through that table. Commands take precedence over cvars, then
aliases. Unknown text prints an error and never becomes chat. Aliases are
available in every source, including Q3, as the owner requires. An alias or
script inserts text ahead of the remaining buffer. `wait` defers execution
to later frames; repeated alias expansion is bounded as in Q2. Idle frames
do not tokenize, allocate command text or look up a cvar name.

Tokenizer rules come from the original Q1, QW, Q2 and Q3 command sources.
They preserve source-specific punctuation, comment handling, quote boundaries,
argument counts and raw argument tails. Q2 expands cvar macros outside quotes
and rejects expansion loops within that command. Q2 rerelease currently uses
the common classic parser. NUL input, oversized text and unsafe quoted-token
lengths produce scoped errors. They do not abort the engine.

`exec` asks the host's one VFS for a mounted script, adding `.cfg` when needed.
The current host accepts UTF-8 script text up to the command-buffer capacity.
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

Interactive editing/focus, chat commands, feature handlers, config order and
archival, user/guest cvar registration and tab completion remain later R2/R8
work. `set` and `vstr` currently use registered stock cvars. THE-639 remains
In Progress until Q1/Q2/Q3 map sessions prove the live alias sequence through
the normal input path and the candidate qualifies for installation.
