# Shared model readers

`qa_formats::model::Model::parse` admits MDL v6, MD2 v8, MD3 v15, MDC v2,
SPR v1, SP2 v2 and MD5 mesh v10. Every mesh has one contiguous frame-major
vertex array, a shared UV array and triangles with separate vertex and UV
indices. Names and indexed skin/sprite pixels borrow the caller's file bytes.
MDL back-face UV adjustments happen once at load. Packed alias frames remain
available for native lighting and interpolation rules.

Frame and skin groups retain native cumulative intervals in one array. MD3 and
MDC tags are frame-major. MDC expands its base/compressed frame references at
load. MD5 keeps numeric bones, model-space bind poses, weights and per-vertex
weight ranges; bind vertices are ready for upload, with bone-local normals
retained separately for later deformation. MD5 animation parsing is a later
capability. Native surface names remain borrowed; material binding normalises
them once when the renderer loads the model.

The reader assigns no identity. The content service retains the VFS reference
and the session assigns its precache index. There is no content fingerprint,
byte-derived cache key or second model provider. Decoding validates counts,
sections, numeric indices and finite values at the file boundary; expanded
vertex/weight arrays have a 256 MiB admission limit per asset. Parsing performs
cold allocations. It has no frame-loop work or filesystem writes.

## Verification

```sh
cargo test -p qa-formats --test models
cargo run --release -p qa-content --example models -- "$QA_QFILES"
python3 tools/check_models.py --qsrc "$QA_QSRC" --c-port "$QA_C_PORT" --output "$QA_EVIDENCE"
```

The owned corpus on 2026-10-07 contains 2,880 physical model/sprite entries:
404 MDL, 923 MD2, 1,263 MD3, 263 MD5 mesh, 10 SPR and 17 SP2. All admit through
the shared VFS and model reader, including loose files and archive duplicates.
There are no owned MDC files: its base/compressed frames, normal indices, tags
and invalid references are verified with a source-only fixture.

The seven small fixtures come from unchanged C-port test constructors and the
original RTCW MDC layout. Tests check frame/skin groups, independently indexed
UVs, packed alias frames, MD3/MDC tags, MDC delta extrema, out-of-order MD5
records, bind weights and sprite payloads. Every fixture truncation and 10,000
seeded mutations return through the format boundary without panicking.

The comparison tool extracts the original Q3 sine initialization and normal
decode statements. All 65,536 packed MD3 normals (196,608 float32 components)
match bit for bit through `Model::parse`. The C port's double-based helper
differs in 144,822 component bit patterns on this same input; this measures an
arithmetic difference and makes no claim about its visible effect or speed.

THE-850 preserves two native cases: tag-only Q3 hand models have inverted
empty-box metadata, represented as empty common bounds while their tags remain
available; the rerelease's `backpackcells.mdl` retains its `INT32_MIN` sync
metadata. Original Quake only selects random synchronisation for `ST_RAND`.
Active meshes with inverted bounds still fail admission.

These checks prove readers and their data conversion. The app does not yet
render these arrays or animate models. They do not qualify an installation,
frame-time target, movement, saves or the three-game walk-through gate.

## Sources

Layouts and decoding follow `quake/WinQuake/modelgen.h`, `spritegn.h`,
`gl_model.c`, `gl_mesh.c`; `quake-2/qcommon/qfiles.h`, `ref_gl/gl_model.c`; and
`quake-iii-arena/code/qcommon/qfiles.h`, `renderer/tr_init.c`, `tr_surface.c`.
MDC follows the [original RTCW layout and renderer](https://github.com/id-Software/RTCW-SP/blob/master/src/qcommon/qfiles.h).
The MDC normal table uses GtkRadiant's PicoModel rows, verified bit-identical to
id's RTCW table; the original copyright and redistribution notices are retained.
MD5 parsing and bind normals follow the proven C reader in
`src/formats/model/md5.c`, with quaternion rotation in the shared core math.

Parsing logic was inspected only at these authorised retired paths:
`muse-final:crates/content/src/mdl.rs`, `md2.rs`, `md3.rs`, `spr.rs` and `md5.rs`.
The readers use the primitive-first workspace and borrow their input instead
of adopting the retired per-game structures or file identities.
