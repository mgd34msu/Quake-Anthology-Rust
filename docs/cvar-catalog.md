# Unified cvar catalog

`data/unified-cvars.csv` and its companion policy are the owner's original
1,260-row catalog. `tools/gen_cvars.py` compiles them into one immutable Rust
catalog in `crates/console/src/cvars_generated.rs`. It records all 237 aliases,
1,530 expanded name bindings, 6,363 default clauses, 2,327 flag clauses and
235 conversion descriptors. Eleven seat-name families expand into separate
slots, giving 1,293 canonical storage slots. No game-family lookup gate exists.

The metadata compiler ports the parsing and validation algorithm from C
`tools/generate_unified_cvars.py`. It excludes C emitters and storage layout;
Rust owns the shipped types, data and runtime. Only developer tooling reads
the CSV. Per-source defaults and flags are data views of one registry, and
seat families have independent storage. Historic port-status cells are audit
metadata, not claims about the new engine.

```sh
python3 tools/gen_cvars.py
python3 tools/gen_cvars.py --check
python3 tools/check_cvars.py --evidence "$QA_EVIDENCE/cvars"
cargo test -p qa-console
```

The comparison runs the compiled Rust catalog and checks every original CSV
cell against its output. The generator verifies alias/seat coverage, index
ranges, conversion operands and case-insensitive collisions. `tools/build.py`
and CI reject stale output before compilation. The rule-fixture tool also
plants a stale catalog to verify rejection.

Inferred types, ranges and unresolved policy clauses remain explicit metadata;
they do not become guessed clamps or defaults. The policy report distinguishes
single-name type hints from default, flag, conversion and ownership questions.

THE-613 provides the table and storage. The current window shell starts with
Q3 host defaults; THE-623 supplies source-aware unset values, converted alias
views, side-scoped lookup and flag enforcement. Cached canonical numeric
handles already avoid string lookup in the shell loop. The command table and
live `cvarlist` arrive with THE-639. Metadata comparison and registry tests do
not prove game consumers, console rendering or a qualified installation.
