# Shared cvar views

The process owns one `Cvars` registry. A load-time, case-insensitive name index
resolves canonical names and aliases to stable numeric handles. A `View` adds
the caller's source, role and side; it never owns another value table. Q1,
QuakeWorld, Q2 classic, Q2 rerelease and Q3 defaults are metadata projections
of unset values. An explicit setting survives source changes.

Engine consumers read cached canonical numbers. Module boundaries retain
converted views; their numeric projections are cached when settings change.
String conversion, parsing, allocation and the name index stay outside those
numeric reads. Cold setting changes refresh projections. Source-native alias
flags remain distinct, such as Q2's settable `game` and protected `gamedir`.

The pure conversions port the C implementation in `src/console/cvars_conversion.c`.
They preserve identity, reciprocal gamma, scale, inverted bool, enum details,
colour tables, composite flags and source-unit conversions. Canonical writes
invalidate related alias details, including details dependent on another row.
All assignments are admitted before a composite is published. If an assignment
must wait for a map restart, the whole coupled write waits together, including
its alias detail. A later overlapping write replaces that pending group.

THE-857 corrects the compiler's missing operand for the owner's Q1
`teamplay 1/2` policy: it writes `g_gametype=3` and `g_friendlyFire=0/1`.
The original C metadata compiler emitted zero operands. The comparison harness
records that reproduction separately, then runs the unchanged C conversion
kernel with the corrected shared metadata. It does not edit the C checkout.

```sh
python3 tools/check_cvar_views.py --c-port "$QA_C_REFERENCE" --evidence "$QA_EVIDENCE/cvar-views"
python3 tools/check_cvars.py --evidence "$QA_EVIDENCE/catalog"
cargo test -p qa-console
```

The headless reference checks compare 100,080 source-specific numeric parses
(f32 bits and integer results) and 344,250 paired alias read/write cases
(success, exact text, details and every secondary assignment). They cover all
1,530 bindings, five sources and three caller roles. Registry checks cover
stable handles, defaults, shared FOV/viewsize/gamma, side-scoped passwords,
native protection, composite colours and immediate/deferred teamplay writes.

Video-mode conversions that need a renderer callback and unresolved owner
policies return scoped errors. Fullscreen zero works without that callback.
Guest-defined registrations, guest overrides and their ABI publication remain
later module-host work. These source checks do not prove console execution,
map sessions or a qualified installation. THE-623 remains In Progress until
the binary supplies the required Q1/Q2/Q3 session logs.
