#!/usr/bin/env python3
"""Generate C-port model fixtures and compare packed normals with original Q3."""
import argparse
import json
import os
from pathlib import Path
import re
import struct
import subprocess
import time
from check_hull_trace import function


def md3_normals():
    # Sixteen surfaces stay within the native 4096-vertex surface limit while
    # exercising every possible unsigned packed normal through Model::parse.
    surface_size = 108 + 4096 * 16
    end = 164 + 16 * surface_size
    data = bytearray(b"IDP3" + struct.pack("<i64s9i", 15, b"normals", 0, 1, 0, 16, 0, 108, 164, 164, end))
    data += struct.pack("<10f16s", *([-1.0] * 3 + [1.0] * 3 + [0.0] * 3 + [2.0]), b"frame")
    for surface in range(16):
        data += b"IDP3" + struct.pack("<64s10i", b"body", 0, 1, 0, 4096, 0, 108, 108, 108, 108 + 4096 * 8, surface_size)
        data += bytes(4096 * 8)
        data += b"".join(struct.pack("<hhhH", 0, 0, 0, surface * 4096 + vertex) for vertex in range(4096))
    assert len(data) == end
    return data


def mdc():
    # One base frame, one compressed frame, one tag. Delta bytes include both
    # extrema and zero; normal indices cover the endpoints of the 256-row table.
    data = bytearray(b"IDPC" + struct.pack("<i64s10i", 2, b"compressed", 0, 2, 1, 1, 0, 112, 224, 288, 312, 580))
    for _ in range(2):
        data += struct.pack("<10f16s", *([-3.0] * 3 + [8.0] * 3 + [0.0] * 3 + [10.0]), b"frame")
    data += struct.pack("<64s", b"tag_weapon")
    data += struct.pack("<6h", 64, 0, 0, 0, 0, 0)
    data += struct.pack("<6h", 128, 0, 0, 0, 8175, 0)
    data += b"IDPC" + struct.pack("<64s14i", b"body", 0, 1, 1, 1, 3, 1, 124, 136, 204, 228, 252, 264, 268, 272)
    data += struct.pack("<3i64si", 0, 1, 2, b"textures/body", 7)
    data += struct.pack("<6f", 0, 0, 1, 0, 0, 1)
    data += b"".join(struct.pack("<hhhH", i * 64, 0, 0, 0) for i in range(3))
    data += bytes([127, 0, 255, 0, 128, 127, 127, 255, 127, 127, 127, 128])
    data += struct.pack("<4h", 0, 0, -1, 0)
    # Header model end includes the complete 272-byte surface.
    struct.pack_into("<i", data, 108, len(data))
    assert len(data) == 584
    return data


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--qsrc", type=Path, required=True)
    parser.add_argument("--c-port", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    results = []

    def run(command, log):
        start = time.perf_counter()
        with log.open("w") as stream:
            process = subprocess.run(["timeout", "300", *map(str, command)], stdout=stream, stderr=subprocess.STDOUT, env={**os.environ, "CARGO_TARGET_DIR": "target"})
        results.append({"command": list(map(str, command)), "seconds": time.perf_counter() - start, "exit_code": process.returncode, "log": str(log)})
        if process.returncode:
            raise RuntimeError(log.read_text())

    source = (args.c_port / "tests/model_test.c").read_text()
    prefix = source[:source.index("static qa_bytes bytes(")]
    constructors = "\n".join(function(source, name) for name in ["mdl", "md2", "md3", "sprite"])
    md5 = source[source.index("static const char md5_text"):source.index("int main(void)")]
    writer = prefix + constructors + md5 + r'''
int main(int argc, char **argv) {
    if (argc != 2) return 2;
    fixture f[6] = {mdl(), md2(), md3(), md5(), sprite(false), sprite(true)};
    const char *names[6] = {"group.mdl", "mesh.md2", "mesh.md3", "mesh.md5mesh", "group.spr", "sprite.sp2"};
    for (unsigned i = 0; i < 6; ++i) {
        char path[4096];
        if (snprintf(path, sizeof(path), "%s/%s", argv[1], names[i]) >= sizeof(path)) return 2;
        FILE *out = fopen(path, "wb");
        if (!out || fwrite(f[i].data, 1, f[i].size, out) != f[i].size || fclose(out)) return 2;
    }
    return 0;
}
'''
    fixture_source = args.output / "fixtures.c"
    fixture_source.write_text(writer)
    run(["cc", "-O2", "-I", args.c_port / "include", fixture_source, args.c_port / "src/core/binary.c", args.c_port / "src/core/common.c", "-o", args.output / "fixtures"], args.output / "fixtures-build.log")
    run([args.output / "fixtures", args.output], args.output / "fixtures-run.log")
    (args.output / "mesh.mdc").write_bytes(mdc())
    (args.output / "normals.md3").write_bytes(md3_normals())
    header = (args.qsrc / "quake-iii-arena/code/game/q_shared.h").read_text()
    init = (args.qsrc / "quake-iii-arena/code/renderer/tr_init.c").read_text()
    surface = (args.qsrc / "quake-iii-arena/code/renderer/tr_surface.c").read_text()
    defines = "\n".join(re.search(r"^#define " + name + r"\b[^\n]+", header, re.MULTILINE)[0] for name in ["M_PI", "DEG2RAD"])
    sine = re.search(r"tr\.sinTable\[i\]\s*=\s*sin\([^;]+;", init)[0]
    decode = re.search(r"lat = \( newNormals\[0\][\s\S]+?outNormal\[2\] = [^;]+;", surface)[0]
    original = '#include <math.h>\n#include <stdio.h>\n#include <stdint.h>\n#undef M_PI\n' + defines + '\n#define FUNCTABLE_SIZE 1024\n#define FUNCTABLE_MASK 1023\nstatic struct { float sinTable[1024]; } tr;\n'
    # Keep the proven C port's helper separate to quantify the arithmetic gap.
    original += function((args.c_port / "src/formats/model/md3.c").read_text(), "table_sine") + r'''
int main(int argc, char **argv) {
    if (argc != 2) return 2;
    FILE *out = fopen(argv[1], "wb");
    if (!out) return 2;
    for (int i = 0; i < 1024; ++i) { SINE }
    unsigned differences = 0;
    for (unsigned packed = 0; packed < 65536; ++packed) {
        unsigned short newNormals[1] = {packed};
        int lat, lng;
        float outNormal[3];
        DECODE
        float other[3] = {table_sine(lat + 256) * table_sine(lng), table_sine(lat) * table_sine(lng), table_sine(lng + 256)};
        for (unsigned axis = 0; axis < 3; ++axis) {
            union { float f; unsigned u; } a = {outNormal[axis]}, b = {other[axis]};
            differences += a.u != b.u;
        }
        if (fwrite(outNormal, 4, 3, out) != 3) return 2;
    }
    printf("original_Q3_components=196608 C_port_differing_components=%u\n", differences);
    return fclose(out);
}
'''
    original = original.replace("SINE", sine).replace("DECODE", decode)
    original_source = args.output / "original-normal-code.c"
    original_source.write_text(original)
    run(["cc", "-O3", "-ffp-contract=off", original_source, "-lm", "-o", args.output / "original-normals"], args.output / "original-normals-build.log")
    run([args.output / "original-normals", args.output / "original-normals.bin"], args.output / "original-normals-run.log")
    run(["cargo", "run", "--release", "-p", "qa-formats", "--example", "model_reference", "--", args.output], args.output / "rust-reference.log")
    report = {"scope": "C-port constructors and original Q3 normal arithmetic; no game run", "references": ["tests/model_test.c", "quake-iii-arena/code/game/q_shared.h", "quake-iii-arena/code/renderer/tr_init.c", "quake-iii-arena/code/renderer/tr_surface.c"], "results": results}
    (args.output / "comparison.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
