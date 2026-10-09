#!/usr/bin/env python3
"""Bit-compare production CPU sky endpoints with unchanged WinQuake C."""
import argparse
import json
import os
from pathlib import Path
import random
import struct
import subprocess

from check_hull_trace import function

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--qsrc', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    source = args.qsrc / 'quake/WinQuake'
    functions = '\n'.join([
        function((source / 'mathlib.c').read_text(), 'VectorNormalize'),
        function((source / 'd_sky.c').read_text(), 'D_Sky_uv_To_st'),
    ])
    code = '''#include <math.h>
#include <stdint.h>
#include <stdio.h>
typedef float vec_t, vec3_t[3];
typedef int fixed16_t;
#define SKYSIZE 128
struct { struct { int width, height; } vrect; } r_refdef;
struct { int width, height; } vid;
vec3_t vpn, vright, vup;
float skytime, skyspeed = 8;
''' + functions + '''
int main(int argc, char **argv) {
    if (argc != 3) return 2;
    FILE *in = fopen(argv[1], "rb"), *out = fopen(argv[2], "wb");
    if (!in || !out) return 2;
    uint32_t pixels[4]; float values[10];
    while (fread(pixels, 4, 4, in) == 4) {
        if (fread(values, 4, 10, in) != 10) return 2;
        vid.width = r_refdef.vrect.width = pixels[0];
        vid.height = r_refdef.vrect.height = pixels[1];
        for (int i = 0; i < 3; i++) {
            vpn[i] = values[i]; vright[i] = -values[3+i]; vup[i] = values[6+i];
        }
        skytime = values[9];
        fixed16_t s, t; D_Sky_uv_To_st(pixels[2], pixels[3], &s, &t);
        int64_t endpoints[2] = {s, t};
        if (fwrite(pixels, 4, 4, out) != 4 || fwrite(values, 4, 10, out) != 10 ||
            fwrite(endpoints, 8, 2, out) != 2) return 2;
    }
    return fclose(in) || fclose(out);
}
'''
    original = args.output / 'sky-original.c'
    original.write_text(code)
    binary = args.output / 'sky-original'
    subprocess.run(['cc', '-O3', '-ffp-contract=off', str(original), '-lm', '-o', str(binary)], check=True)
    rng = random.Random(0x534B5955)
    rows = []
    for i in range(16384):
        width, height = [(640, 400), (320, 200), (800, 600), (240, 400)][i % 4]
        axes = [rng.uniform(-1, 1) for _ in range(9)]
        time = rng.uniform(-16, 16)
        rows.append(struct.pack('<4I10f', width, height, rng.randrange(width + 1), rng.randrange(height + 1), *axes, time))
    inputs = args.output / 'inputs.bin'
    inputs.write_bytes(b''.join(rows))
    fixture = args.output / 'native-endpoints.bin'
    subprocess.run([str(binary), str(inputs), str(fixture)], check=True)
    if fixture.stat().st_size != len(rows) * 72:
        raise RuntimeError('original-C sky row count')
    env = {**os.environ, 'QA_SKY_ENDPOINT_FIXTURE': str(fixture.resolve())}
    with (args.output / 'comparison.log').open('w') as log:
        subprocess.run(['cargo', 'test', '--release', '-p', 'qa-render', '--lib', 'cpu::sky::tests::', '--', '--nocapture'], cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
    report = {'result': 'PASS', 'native_rows': len(rows), 'previous_arithmetic_rows': 16384,
              'sources': ['quake/WinQuake/d_sky.c:D_Sky_uv_To_st', 'quake/WinQuake/mathlib.c:VectorNormalize'],
              'scope': 'CPU screen-ray i64 endpoints; not whole sky presentation parity'}
    (args.output / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report))


if __name__ == '__main__':
    main()
