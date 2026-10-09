# Module services

THE-863's typed engine call table borrows the existing server, cvar registry,
command buffer, VFS, collision store and output ring. It owns none of them.
`Runtime::engine_services` creates that borrow for a module call; the console
splits its existing table and command buffer rather than making a module console.

`ENGINE_CALLS` provides print, sound, effect, trace, explicit link/unlink,
spawn/free, converted cvar writes, commands, configstrings and file reads.
Boundary adapters retain native arguments and return layouts. Trace queries
already contain caller-selected rules; linking and allocation use separately
supplied capability data. The host supplies native time. No service reads an OS
clock or performs another physical intake.

Module configstrings have separate numeric ranges in one load-sized store.
They and print payloads retain original bytes, including high palette-font
glyphs. Print pages use the existing output ring and text leases. Files use
module-owned numeric handles over the existing VFS; partial deflated reads use
the existing archive decoder with fixed discard storage and validate the entire
deflated member's CRC. Closing another module's file is rejected for that call.

The QVM reader checks instructions, branch targets and segment/frame bounds at
load. Its one interpreter preserves native instruction ordinals and saved byte
PCs. A call selects the ordinary or instrumented specialization once; the
ordinary loop does not maintain hook counters or dirty words. Instrumented
writes mark a bitmap allocated at load. Nested system calls use a separate
operand stack and restore the interrupted module stack on success or failure.
Module bytes have one owned backing that engine views can borrow.

`tools/check_qvm.py` compiles the unchanged original interpreted execution
functions and compares return values and memory against the Rust interpreter.
That includes the original interpreter's previous-operand BCOM behavior and
reverse-word overlapping block copies. It does not establish the compiled
QVM ABI, retail gameplay or a native library backend. The retail baseq3 and
Team Arena qagame/cgame/ui images pass structural loading only.

Q3 server, cgame and UI import tables now select the same boundary handlers
for the implemented print/error, supplied platform time, catalog cvar reads and
writes, command arguments, read-only VFS, configstring and memory/math calls.
Their native import numbers and different argument layouts remain separate.
QVM words are widened at the boundary and pointers use the original mask;
native callers retain full-width owned-memory addresses. This is an ABI entry
path, not native machine-code execution. Unknown calls log once in bounded
load-sized storage and return zero; output loss and unknown-log capacity drops
are counted separately. `G_ERROR` rejects the current module call.

The headless engine-call fixture executes real QVM instructions through these
tables, changes a shared cvar, publishes print records, retires them by the
existing best-effort submission rule and executes appended commands through
the existing console. It does not supply native channel ACKs or retail play.
Only catalog cvars and read-only files are implemented. The server console call
currently supports EXEC_APPEND; immediate/insert commands and UI ExecuteText
remain pending. Registration/update, native live entity binding, asset registration, filesystem
writes, dynamic module cvar registration and remaining imports must follow
before retail modules or installed gameplay can be claimed. There is no native
execution backend yet.

The version-six QuakeC reader retains native global words, field ordinals,
function handles and file string offsets. Header CRC policy comes from the ABI
caller. It computes the original file CRC once. Statement operand/branch errors
become invalid prepared instructions; an unreachable bad statement does not
prevent loading. Execution traps that statement only if it runs, then restores
the call's local frames so another call can proceed. Function parameter writes
and saved locals have independent extents, matching retail mission-pack QCC
output and the original enter/leave code.

The QC VM borrows the same optional hook implementation as QVM, with one dirty
bitmap policy and one native signed-conversion helper. Ordinary instruction
execution does not record trace PCs or hook counters. Builtin calls select a
boundary host; its services mapping is still pending. Native edict values are
byte offsets into one owned backing with a caller-supplied header/stride. Engine
code can read those bytes directly, but collision has not yet adopted the
native field binding. `OP_STATE` resolves self/time/nextthink/frame/think once
at load and uses the supplied step, preserving double-literal narrowing and
the original frame comparison.

The original QC functions match the defined seeded global/entity-memory
fixtures. Rerelease id1, Hipnotic, Rogue and AD server/client programs pass
structural loading. Stock builtins, native strings/extensions, providers,
spawning and live gameplay remain open. Those reader results do not qualify
AD, module gameplay or installation.

The native behavior references are qsrc `quake/WinQuake/pr_cmds.c`'s builtin
table, `quake-2/game/game.h`'s `game_import_t`, and
`quake-iii-arena/code/server/sv_game.c`'s `SV_GameSystemCalls`. Tests exercise
the app's actual services borrow across module namespaces, generation lifetimes,
explicit relinks, shared converted cvars, one command buffer, byte strings and
file-handle ownership. They do not qualify installed or native ABI gameplay.
