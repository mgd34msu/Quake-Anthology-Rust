#!/usr/bin/env python3
"""Compare shared capsule targets with the original Q3 functions. No game/timing/allocation run."""
import argparse
import json
from pathlib import Path
import random
import struct
from check_brush_trace import C_FLAGS, ROOT, compare, extract_function, run


def workload():
    shapes = [([-15., -15., -24.], [15., 15., 32.]), ([-1.] * 3, [1.] * 3),
              ([-.75, -.25, -1.], [.25, 2., 1.5]), ([0.] * 3, [2., 4., 8.]),
              ([-32., -32., -24.], [32., 32., 64.]), ([0.] * 3, [0.] * 3)]
    rng = random.Random(0x4350534c)
    rows, focused = [], []
    for i in range(10000):
        low, high = shapes[i % len(shapes)]
        qlow, qhigh = shapes[(i // len(shapes)) % len(shapes)]
        origin = [rng.uniform(-64., 64.) for _ in range(3)]
        start = [value + rng.uniform(-96., 96.) for value in origin]
        end = start if i % 17 == 0 else [value + rng.uniform(-96., 96.) for value in origin]
        rows.append((*origin, *low, *high, *start, *end, *qlow, *qhigh, (i // 36) % 2))
    for low, high in shapes:
        for qlow, qhigh in shapes:
            for capsule in [0, 1]:
                for start, end in [([0., 0., 0.], [0., 0., 0.]), ([100., 0., 0.], [0., 0., 0.]),
                                   ([0., 0., 100.], [0., 0., -100.]), ([0., 0., 0.], [100., 0., 0.]),
                                   ([0., 0., 1.], [0., 0., 1.])]:
                    focused.append({'row': len(rows), 'name': f'target={low}/{high} query={qlow}/{qhigh} capsule={capsule} {start}->{end}'})
                    rows.append((0., 0., 0., *low, *high, *start, *end, *qlow, *qhigh, capsule))
                for origin in [[8., -4., 2.], [16777216., -16., 32.]]:
                    start = [origin[0] + 100., origin[1], origin[2]]
                    end = list(origin)
                    focused.append({'row': len(rows), 'name': f'translated target={low}/{high} query={qlow}/{qhigh} capsule={capsule} origin={origin}'})
                    rows.append((*origin, *low, *high, *start, *end, *qlow, *qhigh, capsule))
    return struct.pack('<II', 0x4c535043, len(rows)) + b''.join(struct.pack('<21fI', *row) for row in rows), {
        'seed': 0x4350534c, 'seeded_rows': 10000, 'rows_per_rule': len(rows), 'focused_cases': focused}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--qsrc', type=Path, required=True)
    parser.add_argument('--rust-binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--cc', default='cc')
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    base = args.qsrc / 'quake-iii-arena/code'
    source = (base / 'qcommon/cm_trace.c').read_text()
    names = ['CM_ProjectPointOntoVector', 'CM_DistanceFromLineSquared', 'SquareRootFloat',
             'CM_TestBoxInBrush', 'CM_TraceThroughBrush', 'CM_TestCapsuleInCapsule',
             'CM_TestBoundingBoxInCapsule', 'CM_TraceThroughSphere', 'CM_TraceThroughVerticalCylinder',
             'CM_TraceCapsuleThroughCapsule', 'CM_TraceBoundingBoxThroughCapsule']
    bodies = [extract_function((base / 'game/q_math.c').read_text(), 'VectorNormalize')[0]]
    bodies.extend(extract_function(source, name)[0] for name in names)
    generated = args.output / 'capsule-original.c'
    generated.write_text((ROOT / 'tools/probes/capsule_q3.c').read_text().replace('/* QA_NATIVE_FUNCTIONS */', '#define RADIUS_EPSILON 1.0f\n' + '\n\n'.join(bodies)))
    binary = args.output / 'capsule-original'
    flags = [*C_FLAGS, '-m32', '-msse2', '-mfpmath=sse']
    run([args.cc, *flags, '-I', str(base / 'game'), '-I', str(base / 'qcommon'), str(generated), '-lm', '-o', str(binary)], args.output, 'compile')
    payload, metadata = workload()
    fixture, native, rust = [args.output / name for name in ['fixture.bin', 'native.bin', 'rust.bin']]
    fixture.write_bytes(payload)
    run([str(binary), str(fixture), str(native)], args.output, 'native')
    run([str(args.rust_binary.resolve()), str(fixture), str(rust)], args.output, 'rust')
    result = compare(native.read_bytes(), rust.read_bytes(), metadata)
    report = {'result': result['result'], 'comparison': result, 'workload': metadata,
              'original_source': str(base / 'qcommon/cm_trace.c'), 'functions': names,
              'native_flags': flags, 'timing_measured': False, 'allocation_measured': False,
              'limits': ['synthetic capsule targets; no retail/live module or installation acceptance',
                         'single native brush leaf for box/capsule swap; no BSP traversal']}
    (args.output / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(result))
    return 0 if result['result'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
