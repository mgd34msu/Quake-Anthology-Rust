#!/usr/bin/env python3
"""Compare shared image conversion with extracted original Q2/Q3 routines."""
import argparse
import json
import os
from pathlib import Path
import re
import struct
import subprocess
import time
from check_hull_trace import function


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--qsrc", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    q2 = (args.qsrc / "quake-2/ref_gl/gl_image.c").read_text()
    q3 = (args.qsrc / "quake-iii-arena/code/renderer/tr_image.c").read_text()
    header = (args.qsrc / "quake-2/qcommon/qfiles.h").read_text()
    pcx = re.search(r"typedef struct\s*\{[^{}]+\} pcx_t;", header)[0]
    source = r'''
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdarg.h>
typedef unsigned char byte;
#define PRINT_DEVELOPER 0
#define PRINT_ALL 1
#define LittleShort(x) (x)
#define Com_Memcpy memcpy
static int load(char *path, void **data) {
    FILE *f = fopen(path, "rb");
    if (!f) exit(2);
    fseek(f, 0, SEEK_END); long n = ftell(f); rewind(f);
    *data = malloc(n);
    if (!*data || fread(*data, 1, n, f) != n || fclose(f)) exit(2);
    return n;
}
static void print(int level, char *format, ...) { (void)level; (void)format; }
static void *allocate(int n) { void *p = malloc(n); if (!p) exit(2); return p; }
static struct {
    int (*FS_LoadFile)(char *, void **);
    void (*FS_FreeFile)(void *);
    void (*Con_Printf)(int, char *, ...);
    void *(*Hunk_AllocateTempMemory)(int);
    void (*Hunk_FreeTempMemory)(void *);
} ri = {load, free, print, allocate, free};
'''
    source += pcx + "\n" + "\n".join([function(q2, "LoadPCX"), function(q2, "GL_MipMap"), function(q3, "R_MipMap2")])
    source += r'''
int main(int argc, char **argv) {
    if (argc != 4) return 2;
    FILE *out = fopen(argv[3], "wb");
    if (!out) return 2;
    if (!strcmp(argv[1], "pcx")) {
        byte *pic, *palette; int w, h;
        LoadPCX(argv[2], &pic, &palette, &w, &h);
        if (!pic || !palette) return 2;
        fwrite(&w, 4, 1, out); fwrite(&h, 4, 1, out);
        fwrite(pic, 1, w * h, out); fwrite(palette, 1, 768, out);
        free(pic); free(palette);
    } else {
        FILE *in = fopen(argv[2], "rb");
        if (!in) return 2;
        unsigned w, h;
        while (fread(&w, 4, 1, in) == 1) {
            if (fread(&h, 4, 1, in) != 1 || w < 2 || h < 2 || w > 32 || h > 32) return 2;
            byte *pixels = malloc(w * h * 4), *box = malloc(w * h * 4);
            if (!pixels || !box || fread(pixels, 4, w * h, in) != w * h) return 2;
            memcpy(box, pixels, w * h * 4);
            GL_MipMap(box, w, h); R_MipMap2((unsigned *)pixels, w, h);
            fwrite(box, 1, w * h, out); fwrite(pixels, 1, w * h, out);
            free(pixels); free(box);
        }
        fclose(in);
    }
    return fclose(out);
}
'''
    (args.output / "original-images.c").write_text(source)
    results = []

    def run(command, name):
        log = args.output / name
        started = time.perf_counter()
        with log.open("w") as stream:
            process = subprocess.run(["timeout", "300", *map(str, command)], stdout=stream, stderr=subprocess.STDOUT, env={**os.environ, "CARGO_TARGET_DIR": "target"})
        results.append({"command": list(map(str, command)), "exit_code": process.returncode, "seconds": time.perf_counter() - started, "log": str(log)})
        if process.returncode:
            raise RuntimeError(log.read_text())

    run(["cc", "-O2", args.output / "original-images.c", "-o", args.output / "original-images"], "original-build.log")
    # Valid native indexed inputs only: original C has no bounds checks.
    for case in range(32):
        w, h = 2 + case, 2 + case % 5
        data = bytearray(128)
        data[:4] = bytes([10, 5, 1, 8])
        struct.pack_into("<4H", data, 4, 0, 0, w - 1, h - 1)
        data[65] = [0, 1, 3][case % 3]
        struct.pack_into("<H", data, 66, w + case % 3)
        for i in range(w * h):
            value = (i * 31 + case * 17) % 256
            data += bytes([0xc1, value]) if value >= 0xc0 else bytes([value])
        data += bytes([12]) + bytes((i + case) % 256 for i in range(768))
        path = args.output / f"{case}.pcx"
        path.write_bytes(data)
        run([args.output / "original-images", "pcx", path, args.output / f"{case}.pcx.raw"], f"pcx-{case}.log")
    state = 0x494d4147
    cases = bytearray()
    for case in range(100):
        w, h = 1 << (1 + case % 5), 1 << (1 + case // 5 % 5)
        cases += struct.pack("<2I", w, h)
        for _ in range(w * h * 4):
            state = (state * 1664525 + 1013904223) & 0xffffffff
            cases.append(state >> 24)
    (args.output / "mips.bin").write_bytes(cases)
    run([args.output / "original-images", "mips", args.output / "mips.bin", args.output / "mips.raw"], "mips.log")
    run(["cargo", "run", "--release", "-p", "qa-formats", "--example", "image_reference", "--", args.output], "rust-reference.log")
    report = {"scope": "32 native indexed PCX fixtures and 100 seeded RGBA images, two mip filters; no renderer", "references": ["quake-2/ref_gl/gl_image.c:LoadPCX,GL_MipMap", "quake-iii-arena/code/renderer/tr_image.c:R_MipMap2"], "results": results}
    (args.output / "comparison.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"output": str(args.output), "comparisons": 232, "passed": True}))


if __name__ == "__main__":
    main()
