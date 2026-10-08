#!/usr/bin/env python3
"""Bit-compare shared convex traces with extracted original Q2/Q3 brush kernels.

This headless developer fixture never launches a game. It is not complete BSP
traversal, capsule, linked-entity, legacy-network, installation or live proof.
Build the Rust example separately before running this tool; it never starts a
concurrent Cargo build. Native C is compiled only into the evidence directory.
"""
import argparse
import json
from pathlib import Path
import random
import re
import struct
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
SEED = 0x42525348
SEEDED_ROWS = 10_000
C_FLAGS = ['-std=c99', '-O2', '-ffp-contract=off', '-fno-fast-math',
           '-fexcess-precision=standard', '-fno-strict-aliasing']
COLUMNS = ['fraction', 'end_x', 'end_y', 'end_z', 'normal_x', 'normal_y',
           'normal_z', 'plane_distance', 'startsolid_allsolid', 'contents', 'surface']


def extract_function(source, name):
    # Mask comments/literals without changing offsets. Braces in a native
    # comment must not truncate or extend the extracted implementation.
    tokens = re.compile(r'//[^\n]*|/\*[\s\S]*?\*/|"(?:\\.|[^"\\])*"|\'(?:\\.|[^\'\\])*\'')
    code = tokens.sub(lambda match: ''.join('\n' if c == '\n' else ' ' for c in match[0]), source)
    match = re.search(r'^[ \t]*(?:static[ \t]+)?void[ \t]+' + re.escape(name)
                      + r'\s*\([^;{}]*\)\s*\{', code, re.MULTILINE)
    if match is None:
        raise ValueError('native function definition missing: ' + name)
    opening = code.index('{', match.start(), match.end())
    depth = 1
    end = opening + 1
    while depth and end < len(code):
        depth += (code[end] == '{') - (code[end] == '}')
        end += 1
    if depth:
        raise ValueError('unterminated native function: ' + name)
    return source[match.start():end], source[:match.start()].count('\n') + 1


def float_bits(value):
    return struct.unpack('<I', struct.pack('<f', value))[0]


def box(mins, maxs, contents=1, extra=()):
    planes = []
    for axis in range(3):
        positive = [0.0] * 3
        negative = [0.0] * 3
        positive[axis], negative[axis] = 1.0, -1.0
        planes.append((positive, maxs[axis], axis))
        planes.append((negative, -mins[axis], 3))
    planes.extend((normal, distance, 3) for normal, distance in extra)
    return {'mins': mins, 'maxs': maxs, 'contents': contents, 'planes': planes}


def workload():
    maps = [
        [box([-10.0] * 3, [10.0] * 3)],
        [box([-10.0] * 3, [10.0] * 3, 1 | 32, [([0.6, 0.8, 0.0], 5.0)])],
        [box([-10.0] * 3, [0.0, 0.0, 10.0])],
        [box([16_777_200.0, -10.0, -10.0], [16_777_216.0, 10.0, 10.0])],
        [box([-10.0] * 3, [10.0] * 3, 0x10000,
             [([0.6, -0.8, 0.0], 5.0), ([0.0, 0.6, 0.8], 6.0)])],
        [box([-10.0] * 3, [10.0] * 3, 1),
         box([-10.0] * 3, [10.0] * 3, 0x10000)],
    ]
    bounds = [([0.0] * 3, [0.0] * 3),
              ([-16.0, -16.0, -24.0], [16.0, 16.0, 32.0]),
              ([-0.75, -0.25, -1.0], [0.25, 2.0, 1.5]),
              ([0.0] * 3, [2.0, 4.0, 8.0]),
              ([-32.0, -32.0, -24.0], [32.0, 32.0, 64.0])]
    rng = random.Random(SEED)
    rows = []
    for index in range(SEEDED_ROWS):
        map_id = index % len(maps)
        brush = maps[map_id][0]
        center = [(low + high) * 0.5 for low, high in zip(brush['mins'], brush['maxs'])]
        start = [rng.uniform(low - 48.0, high + 48.0)
                 for low, high in zip(brush['mins'], brush['maxs'])]
        end = ([value + rng.uniform(-64.0, 64.0) for value in start] if index % 2 else
               [rng.uniform(low - 48.0, high + 48.0)
                for low, high in zip(brush['mins'], brush['maxs'])])
        if index % 17 == 0:
            start = center
            end = [value + rng.uniform(-0.5, 0.5) for value in center]
        if index % 13 == 0:
            end = start[:]
        mins, maxs = bounds[(index // len(maps)) % len(bounds)]
        mask = [1 | 32 | 0x10000, 1, 32, 0x10000, 0][index % 5]
        rows.append((map_id, mask, start, end, mins, maxs))
    focused = []

    def add(name, map_id, start, end, mins=None, maxs=None, mask=1 | 32 | 0x10000):
        focused.append({'name': name, 'row': len(rows)})
        rows.append((map_id, mask, start, end, mins or [0.0] * 3, maxs or [0.0] * 3))

    for distance in [0.0, 0.015625, 0.03125, 0.0625, 0.09375, 0.125, 0.25]:
        for end_distance in [-0.125, 0.0, 0.015625, 0.03125, 0.125, 0.25]:
            add(f'x-contact-{distance}-{end_distance}', 2,
                [distance, -1.0, 0.0], [end_distance, -1.0, 0.0])
    for ulps in [0, 1, 2]:
        end = struct.unpack('<f', struct.pack('<I', float_bits(0.125) - ulps))[0]
        add(f'arena-end-epsilon-minus-{ulps}-ulp', 2, [0.25, -1.0, 0.0], [end, -1.0, 0.0])
    add('near-corner-plane-selection', 2, [0.0625, 0.09375, 0.0], [-0.0625, -0.03125, 0.0])
    add('legacy-embedded-moving', 0, [0.0] * 3, [1.0, 0.0, 0.0])
    add('embedded-stationary', 0, [0.0] * 3, [0.0] * 3)
    add('embedded-exit', 0, [0.0] * 3, [100.0, 0.0, 0.0])
    add('outside-stationary', 0, [40.0, 0.0, 0.0], [40.0, 0.0, 0.0])
    add('asymmetric-large-origin-centering', 3, [16_777_222.0, 0.0, 0.0],
        [16_777_220.0, 0.0, 0.0], [-2.0, 0.0, 0.0], [4.0, 0.0, 0.0])
    add('clear-endpoint-exact', 0, [1.0e10, 40.0, 0.0], [0.1, 40.0, 0.0], mask=0)
    add('first-enclosed-brush-moving', 5, [0.0] * 3, [1.0, 0.0, 0.0])
    add('first-enclosed-brush-stationary', 5, [0.0] * 3, [0.0] * 3)
    add('second-brush-selected-by-mask', 5, [100.0, 0.0, 0.0], [0.0] * 3, mask=0x10000)
    for index, (mins, maxs) in enumerate(bounds):
        add(f'positive-slope-bounds-{index}', 1, [40.0, 0.0, 0.0], [0.0] * 3, mins, maxs)
        add(f'negative-slope-bounds-{index}', 4, [0.0, -40.0, 0.0], [0.0] * 3, mins, maxs)
        add(f'slope-stationary-bounds-{index}', 4, [0.0] * 3, [0.0] * 3, mins, maxs)
    payload = bytearray(struct.pack('<4I', 0x48535242, 1, len(maps), len(rows)))
    for brushes in maps:
        payload.extend(struct.pack('<I', len(brushes)))
        for brush_id, brush in enumerate(brushes):
            payload.extend(struct.pack('<2I6f', len(brush['planes']), brush['contents'],
                                       *brush['mins'], *brush['maxs']))
            for plane_id, (normal, distance, axis) in enumerate(brush['planes']):
                payload.extend(struct.pack('<4f2I', *normal, distance, axis,
                                           0x100 + brush_id * 64 + plane_id))
    for map_id, mask, start, end, mins, maxs in rows:
        payload.extend(struct.pack('<2I12f', map_id, mask, *start, *end, *mins, *maxs))
    metadata = {'seed': SEED, 'seeded_rows_per_rule': SEEDED_ROWS,
                'focused_rows_per_rule': len(focused), 'rows_per_rule': len(rows),
                'maps': len(maps), 'focused_cases': focused,
                'geometry': ['axial box', 'positive slope', 'near corner',
                             'large origin', 'negative/multiple slopes', 'ordered overlapping brushes'],
                'bounds': ['point', 'native player asymmetric Z', 'fractional asymmetric',
                           'positive-only asymmetric', 'large box']}
    return payload, metadata


def run(command, output, name):
    result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, timeout=300)
    (output / f'{name}.log').write_text(result.stdout + result.stderr)
    result.check_returncode()
    return result


def compare(expected, actual, metadata):
    rows = metadata['rows_per_rule']
    if len(expected) != rows * 44 or len(actual) != rows * 44:
        raise ValueError('C/Rust result row count differs from the exact fixture')
    labels = {case['row']: case['name'] for case in metadata['focused_cases']}
    mismatches = []
    different = dict.fromkeys(COLUMNS, 0)
    matched = 0
    seeded_matched = 0
    focused_matched = 0
    for index, (native, rust) in enumerate(zip(struct.iter_unpack('<11I', expected),
                                               struct.iter_unpack('<11I', actual))):
        if native == rust:
            matched += 1
            seeded_matched += index < SEEDED_ROWS
            focused_matched += index >= SEEDED_ROWS
            continue
        for column, left, right in zip(COLUMNS, native, rust):
            different[column] += left != right
        if len(mismatches) < 20:
            mismatches.append({'row': index, 'case': labels.get(index, 'seeded'),
                               'native_bits': [f'{value:08x}' for value in native],
                               'rust_bits': [f'{value:08x}' for value in rust]})
    return {'rows_matched_bit_exact': matched, 'rows_compared': rows,
            'seeded_rows_matched_bit_exact': seeded_matched,
            'focused_rows_matched_bit_exact': focused_matched,
            'different_fields': different, 'first_mismatches': mismatches,
            'result': 'PASS' if matched == rows else 'FAIL'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--qsrc', type=Path, required=True)
    parser.add_argument('--rust-binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--cc', default='cc')
    args = parser.parse_args()
    args.qsrc = args.qsrc.resolve()
    args.rust_binary = args.rust_binary.resolve(strict=True)
    args.output = args.output.resolve()
    args.output.mkdir(parents=True, exist_ok=True)
    payload, metadata = workload()
    fixture = args.output / 'brush-fixture.bin'
    fixture.write_bytes(payload)
    (args.output / 'workload.json').write_text(json.dumps(metadata, indent=2) + '\n')
    results = {}
    for rule, folder, relative, functions in [
        ('q2', 'quake-2', 'qcommon/cmodel.c', ['CM_ClipBoxToBrush', 'CM_TestBoxInBrush']),
        ('q3', 'quake-iii-arena/code', 'qcommon/cm_trace.c', ['CM_TraceThroughBrush', 'CM_TestBoxInBrush']),
    ]:
        base = args.qsrc / folder
        source_path = base / relative
        source = source_path.read_text()
        bodies, spans = [], []
        for name in functions:
            body, line = extract_function(source, name)
            bodies.append(body)
            spans.append({'name': name, 'line': line, 'lines': body.count('\n') + 1})
        if rule == 'q2':
            epsilon = re.search(r'^\s*#define\s+DIST_EPSILON\s+[^\n]+', source, re.MULTILINE)
            if epsilon is None:
                raise ValueError('original Q2 DIST_EPSILON is missing')
            bodies.insert(0, epsilon[0])
        template = (ROOT / f'tools/probes/brush_{rule}.c').read_text()
        marker = '/* QA_NATIVE_FUNCTIONS */'
        if template.count(marker) != 1:
            raise ValueError('probe insertion marker count differs')
        generated = args.output / f'brush-{rule}-original.c'
        generated.write_text(template.replace(marker, '\n\n'.join(bodies)))
        executable = args.output / f'brush-{rule}-original'
        started = time.monotonic()
        run([args.cc, *C_FLAGS, '-I', str(base / 'game'), '-I', str(base / 'qcommon'),
             str(generated), '-lm', '-o', str(executable)], args.output, f'{rule}-compile')
        compile_seconds = time.monotonic() - started
        expected, actual = args.output / f'{rule}-native.bin', args.output / f'{rule}-rust.bin'
        run([str(executable), str(fixture), str(expected)], args.output, f'{rule}-native')
        rust = run([str(args.rust_binary), rule, str(fixture), str(actual)], args.output, f'{rule}-rust')
        stats = json.loads(rust.stdout)
        if stats['rows'] != metadata['rows_per_rule'] or stats['rust_calling_thread_alloc_or_realloc'] != 0:
            raise ValueError('Rust row count/allocation gate differs')
        results[rule] = {**compare(expected.read_bytes(), actual.read_bytes(), metadata),
                         'rust': stats, 'native_source': str(source_path),
                         'extracted_functions': spans, 'native_compile_seconds': compile_seconds}
    report = {'scope': 'extracted native brush sweep/position kernels and direct ordered fixture lists',
              'limits': ['no BSP traversal/leaf selection', 'no capsules or transformed/linked entities',
                         'common contents bits only; no file/wire conversion proof',
                         'no retail gameplay, legacy live connection, performance or installation proof',
                         'allocation gate covers calling Rust thread only, not native heap'],
              'source_commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT,
                                                        text=True, timeout=300).strip(),
              'source_tree_dirty': bool(subprocess.check_output(['git', 'status', '--porcelain'],
                                                                cwd=ROOT, text=True, timeout=300)),
              'rust_binary': str(args.rust_binary), 'rust_build_performed_by_tool': False,
              'native_flags': C_FLAGS, 'workload': metadata, 'results': results,
              'result': 'PASS' if all(result['result'] == 'PASS' for result in results.values()) else 'FAIL'}
    (args.output / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report))
    return 0 if report['result'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
