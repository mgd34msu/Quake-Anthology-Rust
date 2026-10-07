These small source-only fixtures contain no retail assets. `group.mdl`,
`mesh.md2`, `mesh.md3`, `mesh.md5mesh`, `group.spr` and `sprite.sp2` are generated
from the C port's unchanged `tests/model_test.c` constructors. `mesh.mdc` is
generated from the original RTCW layout by `tools/check_models.py`.

Regenerate in a temporary evidence directory, then copy these seven fixtures:

```sh
python3 tools/check_models.py --qsrc "$QA_QSRC" --c-port "$QA_C_PORT" --output "$QA_EVIDENCE"
```

The command also runs every packed MD3 normal through the Rust reader against
the original Q3 sine initialization and decoder statements. Its larger exhaustive
fixture and comparison bytes stay in the temporary evidence directory.
