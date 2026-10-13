#!/usr/bin/env python3
"""Compare the tag fixture with unchanged functions from the original Q3 renderer."""
import argparse
import json
from pathlib import Path
import subprocess
from check_hull_trace import function


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--qsrc", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--write-fixture", action="store_true")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    reference = args.qsrc / "quake-iii-arena/code"
    args.output.mkdir(parents=True, exist_ok=True)
    math = (reference / "game/q_math.c").read_text()
    model = (reference / "renderer/tr_model.c").read_text()
    source = r'''
#include "q_shared.h"
#include "../qcommon/qfiles.h"
#include <stddef.h>
#include <stdint.h>
typedef struct { md3Header_t *md3[1]; } model_t;
static model_t model;
static model_t *R_GetModelByHandle(int handle) { (void)handle; return &model; }
'''
    source += "\n".join(function(math, name) for name in ["AxisClear", "VectorNormalize"])
    source += "\n".join(function(model, name) for name in ["R_GetTag", "R_LerpTag"])
    source += r'''
static uint32_t seed = 0x31b8794a;
static uint32_t next(void) { seed ^= seed << 13; seed ^= seed >> 17; seed ^= seed << 5; return seed; }
static float value(void) {
    float number = (float)((int)(next() % 8193) - 4096) / 64.0f;
    int exponent = (int)(next() % 25) - 12;
    return ldexpf(number, exponent);
}
static void word(float value) { uint32_t bits; memcpy(&bits, &value, 4); printf("%08x ", bits); }
static void pose(const orientation_t *value) {
    for (int i = 0; i < 3; ++i) word(value->origin[i]);
    for (int a = 0; a < 3; ++a) for (int i = 0; i < 3; ++i) word(value->axis[a][i]);
}
int main(void) {
    struct { md3Header_t header; md3Tag_t tags[2]; } sample = {0};
    sample.header.numFrames = 2;
    sample.header.numTags = 1;
    sample.header.ofsTags = offsetof(__typeof__(sample), tags);
    model.md3[0] = &sample.header;
    strcpy(sample.tags[0].name, "tag"); strcpy(sample.tags[1].name, "tag");
    for (int row = 0; row < 64; ++row) {
        orientation_t input[2], output;
        for (int f = 0; f < 2; ++f) {
            for (int i = 0; i < 3; ++i) input[f].origin[i] = value();
            for (int a = 0; a < 3; ++a) for (int i = 0; i < 3; ++i) input[f].axis[a][i] = row % 13 ? value() : 0.0f;
        }
        if (row % 7 == 0) for (int i = 0; i < 3; ++i) input[1].axis[0][i] = -input[0].axis[0][i];
        if (row % 17 == 0) input[0].origin[0] = -0.0f;
        float fraction = (float)((int)(next() % 65) - 16) / 32.0f;
        if (row % 7 == 0) fraction = 0.5f;
        for (int f = 0; f < 2; ++f) {
            memcpy(sample.tags[f].origin, input[f].origin, sizeof(vec3_t));
            memcpy(sample.tags[f].axis, input[f].axis, sizeof(input[f].axis));
        }
        if (!R_LerpTag(&output, 1, 0, 1, fraction, "tag")) return 2;
        pose(&input[0]); pose(&input[1]); word(fraction); pose(&output); putchar('\n');
    }
    return 0;
}
'''
    path = args.output / "tag.c"
    path.write_text(source)
    subprocess.run(["cc", "-O2", "-ffp-contract=off", "-I", str(reference / "game"),
                    str(path), "-lm", "-o", str(args.output / "tag")], check=True)
    rows = subprocess.check_output([str(args.output / "tag")], text=True)
    (args.output / "original.txt").write_text(rows)
    fixture = root / "crates/core/tests/fixtures/q3_tags.txt"
    if args.write_fixture:
        fixture.parent.mkdir(parents=True, exist_ok=True)
        fixture.write_text(rows)
    if fixture.read_text() != rows:
        raise RuntimeError("original tag fixture differs")
    result = {"rows": len(rows.splitlines()), "exact": True, "timing_measured": False,
              "reference": "qsrc quake-iii-arena tr_model.c R_GetTag/R_LerpTag; q_math.c AxisClear/VectorNormalize"}
    (args.output / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result))


if __name__ == "__main__":
    main()
