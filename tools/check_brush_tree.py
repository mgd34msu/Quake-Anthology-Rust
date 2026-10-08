#!/usr/bin/env python3
"""Compare shared brush-tree traces and points with unchanged Q2/Q3 C sources.

Build qa-world's brush_tree example separately. This developer tool compiles
only extracted original-source reference fixtures, never Cargo or a game.
"""
import argparse
import json
from pathlib import Path
import random
import re
import struct
import subprocess

from check_targets import masked

ROOT = Path(__file__).resolve().parents[1]
WORDS = 13
BYTES = WORDS * 4
ALL = 1 | 4 | 32 | 0x10000
COLUMNS = ['fraction', 'end_x', 'end_y', 'end_z', 'normal_x', 'normal_y',
           'normal_z', 'plane_distance', 'plane_axis', 'solid_flags',
           'contents', 'surface', 'point_contents']
C_FLAGS = ['-std=c99', '-O2', '-ffp-contract=off', '-fno-fast-math',
           '-fexcess-precision=standard', '-fno-strict-aliasing']


def default_branch_condition(expression):
    """Evaluate only the portable fixture's simple, undefined-macro guards."""
    expression = expression.strip()
    if expression in ('0', '1'):
        return expression == '1'
    guard = re.fullmatch(r'(!\s*)?defined\s*(?:\(\s*([A-Za-z_]\w*)\s*\)|([A-Za-z_]\w*))', expression)
    if guard:
        return guard[1] is not None
    raise ValueError(f'unsupported native extraction conditional: {expression}')


def native_body_end(code, opening):
    """Count active default-branch braces without changing any source bytes.

    Q3's BSPC and non-BSPC branches each open a brace and share the close.
    All optional macros encountered within these extracted functions are
    undefined by this proof's compiler flags. Other expression forms fail
    closed instead of guessing which brace belongs to the native function.
    """
    if code[opening] != '{':
        raise ValueError('native function opening brace missing')
    depth, offset, active, stack = 1, opening + 1, True, []
    while offset < len(code):
        if offset == 0 or code[offset - 1] == '\n':
            directive = re.match(r'[ \t]*#\s*(\w+)\b([^\n]*)', code[offset:])
            if directive:
                kind, expression = directive[1], directive[2].strip()
                if kind in ('if', 'ifdef', 'ifndef'):
                    if kind in ('ifdef', 'ifndef'):
                        if not re.fullmatch(r'[A-Za-z_]\w*', expression):
                            raise ValueError('invalid native extraction macro guard')
                        selected = kind == 'ifndef'
                    else:
                        selected = default_branch_condition(expression) if active else False
                    stack.append([active, selected, False])
                    active = active and selected
                elif kind == 'elif':
                    if not stack or stack[-1][2]:
                        raise ValueError('unmatched native extraction elif')
                    parent, taken, _ = stack[-1]
                    selected = default_branch_condition(expression) if parent and not taken else False
                    active = parent and not taken and selected
                    stack[-1][1] = taken or selected
                elif kind == 'else':
                    if expression or not stack or stack[-1][2]:
                        raise ValueError('unmatched native extraction else')
                    parent, taken, _ = stack[-1]
                    active = parent and not taken
                    stack[-1][1:] = [True, True]
                elif kind == 'endif':
                    if expression or not stack:
                        raise ValueError('unmatched native extraction endif')
                    active = stack.pop()[0]
                elif kind in ('define', 'undef'):
                    raise ValueError('macro changes inside extracted native function unsupported')
                offset += directive.end()
                continue
        if active:
            depth += (code[offset] == '{') - (code[offset] == '}')
            if depth == 0:
                if stack:
                    raise ValueError('native conditional crosses function ending')
                return offset + 1
        offset += 1
    raise ValueError('native function closing brace missing')


def extract_function(source, name):
    code = masked(source)
    matches = list(re.finditer(r'(?m)^[ \t]*(?:void|int|trace_t|qboolean|vec_t|clipHandle_t)\s+'
                               + re.escape(name) + r'\s*\([^;{}]*\)\s*\{', code))
    if len(matches) != 1:
        raise ValueError(f'{name}: portable native full definition count differs')
    match = matches[0]
    end = native_body_end(code, match.end() - 1)
    value, line = source[match.start():end], source[:match.start()].count('\n') + 1
    offset = sum(len(value) for value in source.splitlines(keepends=True)[:line - 1])
    if offset != match.start() or source[offset:offset + len(value)] != value:
        raise ValueError('native function extraction is not an unchanged contiguous slice')
    return value, {'function': name, 'start_line': line, 'end_line': line + value.count('\n'),
                   'source_bytes': len(value.encode()), 'exact_contiguous_source_slice': True,
                   'brace_selection': 'default undefined optional macros; all source branches retained'}


def extract_define(source, name):
    matches = list(re.finditer(r'(?m)^[ \t]*#\s*define\s+' + re.escape(name) + r'\b[^\n]*', source))
    if len(matches) != 1:
        raise ValueError(f'{name}: native definition missing or duplicated')
    match = matches[0]
    line = source[:match.start()].count('\n') + 1
    return match[0], {'define': name, 'start_line': line, 'end_line': line,
                      'source_bytes': len(match[0].encode()), 'exact_contiguous_source_slice': True}


def tree_plane(normal, distance=0, axis=3):
    return (normal, distance, axis)


def brush(mins, maxs, contents, surface, extra=(), positive_first=False):
    planes = []
    # Native Q3 CM_BoundBrush assumes negative/positive axial pairs first.
    for axis in range(3):
        negative, positive = [0.0] * 3, [0.0] * 3
        negative[axis], positive[axis] = -1.0, 1.0
        pair = [(negative, -float(mins[axis]), 3, surface + axis * 2),
                (positive, float(maxs[axis]), axis, surface + axis * 2 + 1)]
        planes += list(reversed(pair)) if positive_first else pair
    planes += [(normal, distance, 3, surface + 16 + index)
               for index, (normal, distance) in enumerate(extra)]
    return {'planes': planes, 'contents': contents, 'bounds': [mins, maxs],
            'positive_first': positive_first, 'center': [(a + b) * .5 for a, b in zip(mins, maxs)]}


def make_map(brushes, planes, nodes, members, model_leaves, stored=None):
    leaves, refs = [], []
    for index, ids in enumerate(members):
        union = 0
        for brush_id in ids:
            union |= brushes[brush_id]['contents']
        leaves.append((union if stored is None else stored[index], len(refs), len(ids)))
        refs += ids
    return {'brushes': brushes, 'planes': planes, 'nodes': nodes, 'leaves': leaves,
            'refs': refs, 'models': [(0, 0), *((1, leaf) for leaf in model_leaves)],
            'model_centers': [brushes[0]['center'],
                              *(brushes[members[leaf][0]]['center'] if members[leaf] else [0, 0, 0]
                                for leaf in model_leaves)]}


def geometry():
    base = [brush([-8] * 3, [8] * 3, 1, 0x100),
            brush([-8] * 3, [8] * 3, 32, 0x200),
            brush([16, -8, -8], [24, 8, 8], 0x10000, 0x300),
            brush([80, -8, -8], [96, 8, 8], 1, 0x400)]
    x, y, z = tree_plane([1, 0, 0], axis=0), tree_plane([0, 1, 0], axis=1), tree_plane([0, 0, 1], axis=2)
    slope = tree_plane([.6, .8, 0])
    members = [[2, 0, 2], [1, 0, 1], [3], [1, 0]]
    maps = [make_map(base, [x], [(0, -1, -2)], members, [2, 3])]
    maps.append(make_map(base, [slope], [(0, -1, -2)], members, [2, 3]))
    maps.append(make_map(base, [x], [(0, -1, -2)], members, [2, 3], [4 | 32, 4, 0x10000, 1]))
    maps.append(make_map(base, [x, y, z], [(0, 1, 2), (1, -1, -2), (2, -2, -1)], members, [2, 3]))
    maps.append(make_map(base, [x, y, z, slope],
                         [(0, 1, 2), (1, 3, -1), (2, 3, -2), (3, -3, -4)],
                         [[0, 1, 0], [1, 2, 1], [0, 2], [2, 0], [3]], [4, 3]))
    prefix_nodes = [(0, 1, -2)]
    for node in range(1, 11):
        child = node + 1 if node < 10 else -1
        prefix_nodes.append((0, child, child))
    # 1024 repeated front visits fill the native stationary leaf prefix. The
    # enclosed back leaf is deliberately the next visit and must be omitted.
    maps.append(make_map(base, [x], prefix_nodes, [[], [0, 1], [3]], [2, 1]))
    high = 16777216
    large = [brush([high] * 3, [high + 16] * 3, 1, 0x500),
             brush([high] * 3, [high + 16] * 3, 0x10000, 0x600),
             brush([-8] * 3, [8] * 3, 32, 0x700)]
    maps.append(make_map(large, [tree_plane([1, 0, 0], high, 0)], [(0, -1, -2)],
                         [[0, 1, 0], [], [2]], [2, 0]))
    sloped = [brush([-8] * 3, [8] * 3, 1, 0x800, [([.6, .8, 0], 3.5)]),
              brush([-8] * 3, [8] * 3, 32, 0x900, [([-.6, .8, 0], 4)]), base[2], base[3]]
    maps.append(make_map(sloped, [slope, z], [(0, 1, -2), (1, -1, -2)],
                         [[1, 0, 1], [0, 1, 0], [3]], [2, 0]))
    for positive_first in [False, True]:
        rounding = brush([16777218, -10, -10], [16777220, 10, 10], 1, 0xa00,
                         positive_first=positive_first)
        maps.append(make_map([rounding], [x], [(0, -1, -1)], [[0], [0]], [1]))
    return maps


def bits(value):
    return struct.unpack('<I', struct.pack('<f', value))[0]


def workload(seeded_rows):
    maps, rows, labels = geometry(), [], []
    rng = random.Random(0x42545245)
    sizes = [([0.0] * 3, [0.0] * 3), ([-16, -16, -24], [16, 16, 32]),
             ([-.75, -.25, -1], [.25, 2, 1.5]), ([0, 0, 0], [2, 4, 8]),
             ([-32, -32, -24], [32, 32, 64]),
             ([16777216] * 3, [16777218] * 3)]

    def add(map_id, model, mask, start, end, mins, maxs, point, label):
        rows.append((map_id, model, mask, start, end, mins, maxs, point))
        labels.append(label)

    for index in range(seeded_rows):
        map_id = index % len(maps)
        model = (index // len(maps)) % len(maps[map_id]['models'])
        center = maps[map_id]['model_centers'][model]
        start = [value + rng.uniform(-48, 48) for value in center]
        end = [value + rng.uniform(-48, 48) for value in center]
        if index % 13 == 0:
            end = start[:]
        if index % 17 == 0:
            start = center[:]
        mins, maxs = sizes[(index // 3) % len(sizes)]
        point = [value + rng.uniform(-16, 16) for value in center]
        if index % 11 == 0:
            point = center[:]
        add(map_id, model, [0, 1, 4, 32, 0x10000, ALL][index % 6], start, end, mins, maxs, point,
            'seeded tree/model/mask/box/point')
    focused_rows = {}
    for map_id, value in enumerate(maps):
        for model, center in enumerate(value['model_centers']):
            cases = [([-32, 0, 0], [32, 0, 0], [0, 0, 0]),
                     ([32, 0, 0], [-32, 0, 0], [-8, 0, 0]),
                     ([0, 0, 0], [0, 0, 0], [8, 0, 0]),
                     ([-0.0, 0, 0], [0.0, 0, 0], [0, 0, 0]),
                     ([8.03125, 0, 0], [8.01, 0, 0], [8, 0, 0]),
                     ([8.125, 0, 0], [8.124, 0, 0], [8.000000953674316, 0, 0])]
            for case, (local_start, local_end, local_point) in enumerate(cases):
                # Keep the explicit signed-zero case at origin; summing with
                # center would erase its sign before the f32 binary transport.
                start = local_start if case == 3 else [a + b for a, b in zip(center, local_start)]
                end = local_end if case == 3 else [a + b for a, b in zip(center, local_end)]
                point = local_point if case == 3 else [a + b for a, b in zip(center, local_point)]
                for size, (mins, maxs) in enumerate(sizes):
                    for mask in [1, 4, 32, 0x10000, ALL]:
                        row = len(rows)
                        add(map_id, model, mask, start, end, mins, maxs, point,
                            f'focused map={map_id},model={model},case={case},size={size},mask={mask}')
                        if map_id == 5 and model == 0 and case == 2 and size == 0 and mask == 1:
                            focused_rows['stationary_1024_prefix'] = row
                        if map_id == 2 and model == 0 and case == 2 and size == 0 and mask == 1:
                            focused_rows['stored_contents_not_brush_union'] = row
    # Here f32 (mins+maxs)*.5 rounds to mins, so centered size0==0 and
    # size1==2. Q3's tree point classification is size0-only, not both bounds.
    add(6, 0, ALL, [0, 0, 0], [4, 4, 4], *sizes[-1], [16777216] * 3,
        'centered asymmetric rounding: size0 zero, size1 two')
    focused_rows['centered_asymmetric_rounding'] = len(rows) - 1
    for map_id in [8, 9]:
        for model in [0, 1]:
            add(map_id, model, 1, [16777216, 0, 0], [16777216, 0, 0], [-1, 0, 0], [1, 0, 0],
                [16777216, 0, 0], f'Q3 stationary axial rounding: map={map_id},model={model}')
            focused_rows[f'axial_rounding_{"positive" if map_id == 9 else "negative"}_first_model{model}'] = len(rows) - 1
    data = bytearray(struct.pack('<4I', 0x45525442, 1, len(maps), len(rows)))

    def pack_plane(plane):
        normal, distance, axis = plane
        data.extend(struct.pack('<5I', *(bits(x) for x in normal), bits(distance), axis))

    for value in maps:
        data.extend(struct.pack('<I', len(value['brushes'])))
        for b in value['brushes']:
            data.extend(struct.pack('<3I', len(b['planes']), b['contents'], int(b['positive_first'])))
            for bound in b['bounds']:
                data.extend(struct.pack('<3I', *(bits(x) for x in bound)))
            for normal, distance, axis, flags in b['planes']:
                pack_plane((normal, distance, axis))
                data.extend(struct.pack('<I', flags))
        data.extend(struct.pack('<I', len(value['planes'])))
        for p in value['planes']:
            pack_plane(p)
        data.extend(struct.pack('<I', len(value['nodes'])))
        for p, a, b in value['nodes']:
            data.extend(struct.pack('<3I', p, a & 0xffffffff, b & 0xffffffff))
        data.extend(struct.pack('<I', len(value['leaves'])))
        for stored, first, count in value['leaves']:
            data.extend(struct.pack('<4I', 1, stored, first, count))
        data.extend(struct.pack('<I', len(value['refs'])))
        for ref in value['refs']:
            data.extend(struct.pack('<I', ref))
        data.extend(struct.pack('<I', len(value['models'])))
        for tag, root in value['models']:
            data.extend(struct.pack('<2I', tag, root))
    for map_id, model, mask, *vectors in rows:
        data.extend(struct.pack('<3I', map_id, model, mask))
        for vector in vectors:
            data.extend(struct.pack('<3I', *(bits(x) for x in vector)))
    return data, {'rows': len(rows), 'seeded_rows': seeded_rows, 'seed': 0x42545245,
                  'geometry': maps, 'labels': labels, 'focused_rows': focused_rows,
                  'same_payload_for_both_rules': True, 'numbers_encoded_f32': True,
                  'ordered_brush_ids_and_duplicates_retained': True,
                  'models': 'world Tree(0); inline direct leaves; Q2 native negative headnode',
                  'valid_forward_edge_dags_only': True,
                  'q3_bounds': 'unchanged CM_BoundBrush for loaded negative-first; unchanged CM_TempBoxModel for positive-first explicit bounds'}


def compare(expected, actual, metadata):
    count = metadata['rows']
    if len(expected) != count * BYTES or len(actual) != count * BYTES:
        raise ValueError('native/Rust tree output length differs')
    matched, different, mismatch = 0, dict.fromkeys(COLUMNS, 0), []
    for index, (native, rust) in enumerate(zip(struct.iter_unpack('<13I', expected),
                                               struct.iter_unpack('<13I', actual))):
        if native == rust:
            matched += 1
            continue
        fields = []
        for column, left, right in zip(COLUMNS, native, rust):
            if left != right:
                different[column] += 1
                fields.append({'field': column, 'native_raw': f'{left:08x}', 'rust_raw': f'{right:08x}'})
        if len(mismatch) < 20:
            mismatch.append({'row': index, 'case': metadata['labels'][index], 'fields': fields})
    return {'rows_compared': count, 'rows_matched_bit_exact': matched,
            'different_fields': {key: value for key, value in different.items() if value},
            'first_mismatches': mismatch, 'result': 'PASS' if count == matched else 'FAIL'}


def controls(expected, metadata):
    if compare(expected, expected, metadata)['result'] != 'PASS':
        raise ValueError('tree comparator identical positive control failed')
    mutations, evidence = [], []
    for column in [0, 12]:
        row = metadata['rows'] // 2
        mutated = bytearray(expected)
        mutated[row * BYTES + column * 4] ^= 1
        outcome = compare(expected, mutated, metadata)
        if (outcome['result'] != 'FAIL' or outcome['rows_matched_bit_exact'] != metadata['rows'] - 1
                or outcome['different_fields'] != {COLUMNS[column]: 1}):
            raise ValueError('one-bit tree comparator negative control not rejected exactly')
        mutations.append(mutated)
        evidence.append({'row': row, 'field': COLUMNS[column], 'one_bit_rejected': True})
    return mutations, {'scope': 'comparator sensitivity, not independent native semantics',
                       'identical_positive_control': 'PASS', 'negative_controls': evidence}


def run(command, output, name):
    result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, timeout=300)
    (output / f'{name}.log').write_text(result.stdout + result.stderr)
    result.check_returncode()
    return result


def source_for(qsrc, rule, output):
    if rule == 'q2':
        folder = qsrc / 'quake-2'
        groups = [('game/q_shared.c', ['BoxOnPlaneSide']),
                  ('qcommon/cmodel.c', ['CM_PointLeafnum_r', 'CM_PointContents',
                   'CM_ClipBoxToBrush', 'CM_TestBoxInBrush', 'CM_TraceToLeaf', 'CM_TestInLeaf',
                   'CM_BoxLeafnums_r', 'CM_BoxLeafnums_headnode', 'CM_RecursiveHullCheck', 'CM_BoxTrace'])]
        define_path, define_names = 'qcommon/cmodel.c', ['DIST_EPSILON']
    else:
        folder = qsrc / 'quake-iii-arena/code'
        groups = [('game/q_math.c', ['BoxOnPlaneSide']), ('qcommon/cm_load.c', ['CM_BoundBrush', 'CM_TempBoxModel']),
                  ('qcommon/cm_test.c', ['CM_PointLeafnum_r', 'CM_PointContents', 'CM_StoreLeafs', 'CM_BoxLeafnums_r']),
                  ('qcommon/cm_trace.c', ['CM_TestBoxInBrush', 'CM_TestInLeaf', 'CM_PositionTest',
                   'CM_TraceThroughBrush', 'CM_TraceThroughLeaf', 'CM_TraceThroughTree', 'CM_Trace'])]
        define_path, define_names = 'qcommon/cm_trace.c', ['MAX_POSITION_LEAFS']
    bodies, spans = [], []
    for name in define_names:
        value, span = extract_define((folder / define_path).read_text(), name)
        span['path'] = str((folder / define_path).relative_to(qsrc))
        bodies.append(value)
        spans.append(span)
    for path, functions in groups:
        source = (folder / path).read_text()
        for name in functions:
            value, span = extract_function(source, name)
            span['path'] = str((folder / path).relative_to(qsrc))
            (output / f'{rule}-{name}-original.inc').write_text(value + '\n')
            bodies.append(value)
            spans.append(span)
    template = (ROOT / f'tools/probes/tree_{rule}.c').read_text()
    marker = '/* QA_NATIVE_FUNCTIONS */'
    if template.count(marker) != 1:
        raise ValueError('tree native insertion marker count differs')
    generated = template.replace(marker, '\n\n'.join(bodies))
    if any(generated.count(value) != 1 for value in bodies):
        raise ValueError('native tree definition changed or duplicated during insertion')
    target = output / f'{rule}-native.c'
    target.write_text(generated)
    return target, folder, spans


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--qsrc', type=Path, required=True)
    parser.add_argument('--rust-binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--cc', default='cc')
    parser.add_argument('--seeded-rows', type=int, default=10000)
    args = parser.parse_args()
    if not 10000 <= args.seeded_rows <= 40000:
        parser.error('--seeded-rows must be 10000..40000')
    args.qsrc = args.qsrc.resolve(strict=True)
    args.rust_binary = args.rust_binary.resolve(strict=True)
    args.output = args.output.resolve()
    args.output.mkdir(parents=True, exist_ok=True)
    payload, metadata = workload(args.seeded_rows)
    fixture = args.output / 'tree-fixture.bin'
    fixture.write_bytes(payload)
    if fixture.read_bytes() != payload:
        raise ValueError('tree fixture file differs from the exact generated bytes')
    (args.output / 'workload.json').write_text(json.dumps(metadata, indent=2) + '\n')
    results = {}
    for rule in ['q2', 'q3']:
        source, folder, spans = source_for(args.qsrc, rule, args.output)
        binary = args.output / f'{rule}-native'
        run([args.cc, *C_FLAGS, '-I', str(folder / 'game'), '-I', str(folder / 'qcommon'),
             str(source), '-lm', '-o', str(binary)], args.output, f'{rule}-compile')
        expected, actual = args.output / f'{rule}-native.bin', args.output / f'{rule}-rust.bin'
        run([str(binary), str(fixture), str(expected)], args.output, f'{rule}-native')
        rust = run([str(args.rust_binary), rule, str(fixture), str(actual)], args.output, f'{rule}-rust')
        allocation = json.loads(rust.stdout)
        if (allocation['rule'] != rule or allocation['rows'] != metadata['rows']
                or allocation['rust_calling_thread_alloc_or_realloc'] != 0
                or allocation['allocation_positive_control'] < 1):
            raise ValueError('Rust tree row/allocation positive or zero gate differs')
        native = expected.read_bytes()
        outcome = compare(native, actual.read_bytes(), metadata)
        mutations, control = controls(native, metadata)
        for column, mutated in zip(['fraction', 'point_contents'], mutations):
            (args.output / f'{rule}-{column}-mutation.bin').write_bytes(mutated)
        focused = {}
        for name, index in metadata['focused_rows'].items():
            focused[name] = {'row': index, 'native_raw': list(struct.unpack_from('<13I', native, index * BYTES))}
        outcome.update({'source_extraction': spans, 'allocation': allocation, 'comparator_controls': control,
                        'focused_results': focused, 'native_headers': [str(folder / 'game/q_shared.h'),
                         str(folder / 'qcommon/cm_local.h')] if rule == 'q3' else [str(folder / 'game/q_shared.h')]})
        outcome['memory_initialization_helpers'] = ([{
            'function': 'Com_Memset',
            'implementation': 'explicit fixture linkage wrapper calling libc memset',
            'scope': 'traceWork_t byte initialization only; no collision arithmetic or decisions',
            'native_portable_reference': 'quake-iii-arena/code/qcommon/common.c:2827-2830',
            'extracted_original_function': False,
        }] if rule == 'q3' else [])
        results[rule] = outcome
    report = {'scope': 'unchanged original native brush-only trace/tree/leaf/point functions',
              'gameplay': False, 'installation_qualified': False, 'performance_measured': False,
              'fixture_bytes': len(payload), 'fixture_readback_exact': True,
              'same_fixture_for_both_native_rules': True, 'seeded_rows_per_rule': args.seeded_rows,
              'limits': ['synthetic forward-edge DAGs; no retail BSP reader or live world acceptance',
                        'Q3 patches disabled and zero references; excluded paths abort if reached',
                        'capsules/transformed/linked entities and special model handles excluded',
                        'world Tree(0) and direct inline leaves only; arbitrary Q2 root subset excluded',
                        'common exact 32-bit contents fixtures; no file/native wire conversion proof',
                        'zero-normal invalid plane axis normalized to no-axis; float bits untouched',
                        'calling-thread Rust allocation scope only; no C/native heap or timing claim',
                        'valid topology only; malformed topology/stamp wrap checks remain separate'],
              'native_flags': C_FLAGS, 'results': results,
              'result': 'PASS' if all(value['result'] == 'PASS' for value in results.values()) else 'FAIL'}
    (args.output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'report': str(args.output / 'report.json'), 'result': report['result'], 'scope': report['scope']}))
    if report['result'] != 'PASS':
        raise SystemExit(1)


if __name__ == '__main__':
    main()
