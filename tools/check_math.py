#!/usr/bin/env python3
"""Compare core math with unmodified original Q1, Q2 and Q3 functions."""
import argparse
import json
import os
from pathlib import Path
import random
import re
import struct
import subprocess
import time
from check_hull_trace import function

PREFIX = '''#include <math.h>
#include <stdio.h>
typedef float vec_t;
typedef float vec3_t[3];
#define PITCH 0
#define YAW 1
#define ROLL 2
#undef M_PI
'''
SUFFIX = '''
int main(int argc, char **argv) {
    if (argc != 3) return 2;
    FILE *in = fopen(argv[1], "rb"), *out = fopen(argv[2], "wb");
    if (!in || !out) return 2;
    float values[7];
    while (fread(values, sizeof(float), 7, in) == 7) {
        vec3_t forward, right, up, vector = { values[3], values[4], values[5] };
        AngleVectors(values, forward, right, up);
        float length = VectorNormalize(vector), wrapped = WRAP(values[6]);
        if (fwrite(forward, 4, 3, out) != 3 || fwrite(right, 4, 3, out) != 3 ||
            fwrite(up, 4, 3, out) != 3 || fwrite(vector, 4, 3, out) != 3 ||
            fwrite(&length, 4, 1, out) != 1 || fwrite(&wrapped, 4, 1, out) != 1) return 2;
    }
    return fclose(in) || fclose(out);
}
'''

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--qsrc', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    rows = [(0.0,) * 7, (-0.0,) * 7, (90.0, 180.0, 270.0, 3.0, 4.0, 0.0, -90.0)]
    rng = random.Random(0x4D415448)
    rows += [tuple(rng.uniform(-720, 720) for _ in range(3)) + tuple(rng.uniform(-4096, 4096) for _ in range(3)) + (rng.uniform(-1000000, 1000000),) for _ in range(1000)]
    inputs = args.output / 'inputs.bin'
    inputs.write_bytes(b''.join(struct.pack('<7f', *row) for row in rows))
    profiles = [('quake/WinQuake/mathlib.c', 'quake/WinQuake/mathlib.h', 'anglemod'),
                ('quake-2/game/q_shared.c', 'quake-2/game/q_shared.h', 'anglemod'),
                ('quake-iii-arena/code/game/q_math.c', 'quake-iii-arena/code/game/q_shared.h', 'AngleMod')]
    results, references = [], []
    def run(command, log):
        start = time.perf_counter()
        with log.open('w') as stream:
            process = subprocess.run(['timeout', '300', *map(str, command)], stdout=stream, stderr=subprocess.STDOUT, env={**os.environ, 'CARGO_TARGET_DIR': 'target'})
        results.append({'command': list(map(str, command)), 'seconds': time.perf_counter() - start, 'exit_code': process.returncode, 'log': str(log)})
        if process.returncode:
            raise RuntimeError(log.read_text())
    for index, (source_path, header_path, wrap) in enumerate(profiles):
        source = (args.qsrc / source_path).read_text()
        header = (args.qsrc / header_path).read_text()
        pi = re.search(r'#define\s+M_PI\s+([0-9.]+f?)', header)[1]
        code = PREFIX + f'#define M_PI {pi}\n#define WRAP {wrap}\n' + '\n'.join(function(source, name) for name in ['AngleVectors', 'VectorNormalize', wrap]) + SUFFIX
        reference = args.output / f'reference-{index}.c'
        reference.write_text(code)
        binary = args.output / f'reference-{index}'
        expected = args.output / f'expected-{index}.bin'
        run(['cc', '-O3', '-ffp-contract=off', reference, '-lm', '-o', binary], args.output / f'c-build-{index}.log')
        run([binary, inputs, expected], args.output / f'c-run-{index}.log')
        references.append(expected)
    run(['cargo', 'run', '--release', '-p', 'qa-core', '--example', 'math_oracle', '--', inputs, *references], args.output / 'rust.log')
    report = {'scope': 'Headless math; no game launch', 'seed': '0x4D415448', 'random_angles': 1000, 'cases_per_profile': len(rows), 'profiles': profiles, 'checks': results, 'rust': (args.output / 'rust.log').read_text()}
    (args.output / 'verification.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))

if __name__ == '__main__':
    main()
