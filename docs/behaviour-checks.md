# Behaviour checks

`cargo test --workspace` runs the public format checks in under a minute. They
check first-match PAK duplicates, borrowed member bytes and malformed BSP bounds
and record widths. CI runs the same checks, formatting and clippy.

The retail test is explicitly ignored unless an owned original Quake PAK is
available. Run it locally with:

```sh
QA_RETAIL_PAK="$PAK" cargo test -p qa-formats --test behaviour -- --include-ignored
```

The owned PAK0 observation matches e1m1's 1,810 planes, 7,358 vertices, 5,516 faces,
58 models, and player.mdl's 212 vertices, 408 triangles and 143 frames. The parser
uses qsrc's binary layouts and the C port's checked load-time bounds and indexing.
PAK names are indexed once at load, payloads remain borrowed, and duplicates keep
the first original ordinal. BSP29, IBSP38 and IBSP46 share the header reader.
ModelHeader reads MDL metadata; it does not yet decode all model records.

Reference layouts are quake/WinQuake/common.c:1225-1236,
quake/WinQuake/bspfile.h:59-95 and quake/WinQuake/modelgen.h:59-75.

Pending behaviour lanes in THE-603 are iterative collision traces, Q1 jump and
friction, Q2/Q3 movement outcomes, and vanilla save round trips. Their engine
implementations enter through the primitives and playable milestones. No stub
tests stand in for those behaviours, and unit tests do not prove live gameplay.
