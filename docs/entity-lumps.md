# Shared entity-lump parsing

`qa_formats::entities::EntityLump` parses every map into numeric key ids,
borrowed value slices and contiguous record ranges. It preserves duplicate
field order and raw string escape bytes. The existing core name table now
offers byte-exact lookup as well as ASCII folding, selected once at load:
Q1 field lookup is exact; Q2/Q3 field lookup folds ASCII case.

The shared text cursor also serves MD5. Its boundary configuration preserves
Q1 punctuation and line comments, Q2 whitespace-delimited words, and Q3 block
comments. Entity text terminates at its first NUL. Incomplete records, quotes,
comments and oversized Q2/Q3 tokens return format errors. No game parser,
string-keyed runtime field map or numeric text formatting is introduced.

THE-855 preserves Q3 values after a newline. Original server
`G_GET_ENTITY_TOKEN` calls `COM_Parse`, which permits line breaks for keys and
values. The C port's entity reader disallows a newline before Q3 values; this
restriction was excluded from the Rust implementation.

`convert` accepts a module's resolved field handler. Handled fields write typed
values and disappear from the returned guest-field array. Unhandled pairs are
retained only for guest/module spawning; native underscore filtering is an
explicit boundary option. Modules still need their original field tables,
aliases, string unescaping, spawn filtering and entity-store writes when live
spawning is wired. The conversion API proves no live spawning by itself.

## Verification

```sh
cargo test -p qa-formats --test entities
cargo run --release -p qa-content --example entities -- "$QA_QFILES" "$QA_ENTITY_EVIDENCE"
python3 tools/check_entities.py --qsrc "$QA_QSRC" --selected "$QA_ENTITY_EVIDENCE" --output "$QA_EVIDENCE"
```

On 2026-10-07 all 1,522 owned map entity lumps parse: 562,540 entity records and
2,118,862 fields. This includes two `e1m1.bsp`, two `base1.bsp` and one
`q3dm1.bsp` entry. Their five record/key/value streams agree with extracted
original Q1/Q2/Q3 token parsers; Q2/Q3 keys are compared after ASCII folding.
The damaged geometry in `amine.bsp` does not prevent its separate entity text
from parsing. Its geometry admission remains the open THE-848 defect.

Fixtures check raw values and borrowing, key case, duplicate order, native
punctuation, Q3 newline values, typed conversion and guest-only unknown fields.
Ten thousand seeded mutations return without panicking. Workspace regression
checks include the MD5 reader that shares the text cursor.

THE-780 remains In Progress until live module spawning uses the resolved field
tables and common entity columns. These checks qualify neither a map run nor
the three-game walk-through gate.

## Sources

Original `quake/WinQuake/common.c`, `pr_edict.c`; `quake-2/game/q_shared.c`,
`g_spawn.c`; `quake-iii-arena/code/game/q_shared.c`, `g_spawn.c` and
`server/sv_game.c`. The C port's `src/formats/bsp.c` was inspected for bounded
token parsing; its Q3 newline restriction was checked against original code.
No retired entity implementation was mined.
