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

This is the services foundation, not a completed module host. Numbered ABI
tables, module execution, native live entity backing, asset registration,
filesystem writes, dynamic module cvar registration and remaining imports must
follow before retail modules or installed gameplay can be claimed. There is no
second runner or native execution backend in this slice.

The native behavior references are qsrc `quake/WinQuake/pr_cmds.c`'s builtin
table, `quake-2/game/game.h`'s `game_import_t`, and
`quake-iii-arena/code/server/sv_game.c`'s `SV_GameSystemCalls`. Tests exercise
the app's actual services borrow across module namespaces, generation lifetimes,
explicit relinks, shared converted cvars, one command buffer, byte strings and
file-handle ownership. They do not qualify installed or native ABI gameplay.
