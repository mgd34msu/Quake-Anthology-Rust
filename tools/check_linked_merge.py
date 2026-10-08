#!/usr/bin/env python3
"""Compare linked-hit merge rules with exact blocks extracted from Q1/Q2/Q3.

Build qa-world's linked_merge developer example separately. This tool builds
only original-C block fixtures; it never launches a game or starts Cargo.
"""
import argparse
import json
from pathlib import Path
import re
import struct
import subprocess
import time

from check_brush_trace import extract_function

ROOT = Path(__file__).resolve().parents[1]
WORDS = 15
RECORD_BYTES = WORDS * 4
COLUMNS = ['fraction', 'end_x', 'end_y', 'end_z', 'normal_x', 'normal_y', 'normal_z',
           'plane_distance', 'plane_axis', 'solid_open_water_brush_flags',
           'contents_low', 'contents_high', 'entity_slot', 'entity_generation', 'surface']
C_FLAGS = ['-std=c99', '-O2', '-ffp-contract=off', '-fno-fast-math',
           '-fexcess-precision=standard', '-fno-strict-aliasing']


def masked(source):
    tokens = re.compile(r'//[^\n]*|/\*[\s\S]*?\*/|"(?:\\.|[^"\\])*"|\'(?:\\.|[^\'\\])*\'')
    return tokens.sub(lambda match: ''.join('\n' if c == '\n' else ' ' for c in match[0]), source)


def balanced_end(code, opening):
    if code[opening] != '{':
        raise ValueError('native merge block opening brace missing')
    depth, end = 1, opening + 1
    while depth and end < len(code):
        depth += (code[end] == '{') - (code[end] == '}')
        end += 1
    if depth:
        raise ValueError('native merge block closing brace missing')
    return end


def extract_merge(source, function, rule):
    body, function_line = extract_function(source, function)
    code = masked(body)
    if rule in ('q1', 'q2'):
        start_pattern = (r'\bif\s*\(\s*trace\.allsolid\s*\|\|\s*trace\.startsolid\s*\|\|'
                         r'\s*trace\.fraction\s*<\s*clip->trace\.fraction\s*\)\s*\{')
    else:
        start_pattern = r'\bif\s*\(\s*trace\.allsolid\s*\)\s*\{'
    matches = list(re.finditer(start_pattern, code))
    if len(matches) != 1:
        raise ValueError(f'{rule}: exact native merge start marker count differs')
    start = code.rfind('\n', 0, matches[0].start()) + 1
    end = balanced_end(code, code.index('{', matches[0].start(), matches[0].end()))
    if rule in ('q1', 'q2'):
        tail = re.match(r'\s*else\s+if\s*\(\s*trace\.startsolid\s*\)'
                        r'\s*clip->trace\.startsolid\s*=\s*true\s*;', code[end:])
        if tail is None:
            raise ValueError(f'{rule}: exact native merge end marker missing')
        end += tail.end()
    else:
        middle = re.match(r'\s*else\s+if\s*\(\s*trace\.startsolid\s*\)\s*\{', code[end:])
        if middle is None:
            raise ValueError('q3: native startsolid block missing')
        opening = end + middle.end() - 1
        end = balanced_end(code, opening)
        tail = re.match(r'\s*if\s*\(\s*trace\.fraction\s*<\s*clip->trace\.fraction\s*\)\s*\{', code[end:])
        if tail is None:
            raise ValueError('q3: native fraction block missing')
        end = balanced_end(code, end + tail.end() - 1)
    block = body[start:end]
    line = function_line + body[:start].count('\n')
    # This assertion checks the unchanged contiguous source bytes, not a digest.
    source_lines = source.splitlines(keepends=True)
    source_offset = sum(len(value) for value in source_lines[:line - 1])
    if source[source_offset:source_offset + len(block)] != block:
        raise ValueError('extracted block is not an exact contiguous source slice')
    return block, {'function': function, 'start_line': line,
                   'end_line': line + block.count('\n'), 'exact_contiguous_source_slice': True}


def bits(value):
    return struct.unpack('<I', struct.pack('<f', value))[0]


def trace_row(fraction, flags, index, incoming, owner):
    base = index * 4 + (2 if incoming else 0)
    slot = (40 if incoming else 10) + index % 8 if owner else 0xffffffff
    generation = (7 if incoming else 3) + index % 5 if owner else 0
    floats = [fraction, base + 0.25, -base - 0.5, base + 0.75,
              0.6 if incoming else -0.6, 0.8 if incoming else -0.8,
              -0.0 if incoming else 0.0, base + 1.25]
    opaque_flags = ((index + int(incoming)) & 7) << 2
    return [*(bits(value) for value in floats),
            [0, 1, 2, 0xffffffff][(index + int(incoming)) % 4], flags | opaque_flags,
            (32 if incoming else 1) | (0x10000 if index % 2 else 0),
            (1 << (index % 8)) if incoming else (1 << ((index + 4) % 8)),
            slot, generation, 0x200 + base]


def workload():
    cases, labels = [], []
    fractions = [0.0, 0.03125, 0.125, 0.5, 0.75, 1.0]
    for old_fraction in [0.0, 0.125, 0.5, 1.0]:
        for old_flags in range(4):
            for new_fraction in fractions:
                for new_flags in range(4):
                    index = len(cases)
                    old = trace_row(old_fraction, old_flags, index, False, index % 2 == 0)
                    new = trace_row(new_fraction, new_flags, index, True,
                                    new_fraction != 1.0 or new_flags != 0)
                    cases.append((old, new))
                    labels.append(f'old={old_fraction}/{old_flags},incoming={new_fraction}/{new_flags}')
    for delta in [-1, 0, 1]:
        fraction = struct.unpack('<f', struct.pack('<I', bits(0.5) + delta))[0]
        for flags in range(4):
            index = len(cases)
            cases.append((trace_row(0.5, 1, index, False, True),
                          trace_row(fraction, flags, index, True, True)))
            labels.append(f'half-fraction-{delta:+d}-ulp/incoming-flags={flags}')
    data = bytearray(struct.pack('<3I', 0x474d4b4c, 1, len(cases)))
    for old, new in cases:
        data.extend(struct.pack('<15I', *old))
        data.extend(struct.pack('<15I', *new))
    return data, {'rows_per_rule': len(cases), 'labels': labels,
                  'old_fractions': [0.0, 0.125, 0.5, 1.0], 'incoming_fractions': fractions,
                  'flags': 'exhaustive startsolid/allsolid pairs; distinct old/incoming payloads',
                  'focused': 'one-ULP strict comparison; incoming clear; old/no old entity ownership'}


def compare(expected, actual, metadata):
    rows = metadata['rows_per_rule']
    if len(expected) != rows * RECORD_BYTES or len(actual) != rows * RECORD_BYTES:
        raise ValueError('native/Rust linked result count differs')
    matched, mismatches = 0, []
    different = dict.fromkeys(COLUMNS, 0)
    for index, (native, rust) in enumerate(zip(struct.iter_unpack('<15I', expected),
                                               struct.iter_unpack('<15I', actual))):
        if native == rust:
            matched += 1
            continue
        for column, left, right in zip(COLUMNS, native, rust):
            different[column] += left != right
        if len(mismatches) < 20:
            mismatches.append({'row': index, 'case': metadata['labels'][index],
                               'native_bits': [f'{value:08x}' for value in native],
                               'rust_bits': [f'{value:08x}' for value in rust]})
    return {'rows_compared': rows, 'rows_matched_bit_exact': matched,
            'different_fields': different, 'first_mismatches': mismatches,
            'result': 'PASS' if rows == matched else 'FAIL'}


def mutation_control(expected, metadata, column):
    mutated = bytearray(expected)
    row = metadata['rows_per_rule'] // 2
    mutated[row * RECORD_BYTES + column * 4] ^= 1
    outcome = compare(expected, mutated, metadata)
    if (outcome['result'] != 'FAIL' or
            outcome['rows_matched_bit_exact'] != metadata['rows_per_rule'] - 1 or
            sum(outcome['different_fields'].values()) != 1 or
            outcome['different_fields'][COLUMNS[column]] != 1):
        raise ValueError('one-bit sensitivity control was not rejected exactly')
    return mutated, {'scope': 'same comparator sensitivity, not independent native semantics',
                     'row': row, 'column': COLUMNS[column], 'flipped_bits': 1,
                     'changed_rows': 1, 'rejected': True}


def run(command, output, name):
    result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, timeout=300)
    (output / f'{name}.log').write_text(result.stdout + result.stderr)
    result.check_returncode()
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--qsrc', type=Path, required=True)
    parser.add_argument('--rust-binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--cc', default='cc')
    args = parser.parse_args()
    args.qsrc = args.qsrc.resolve()
    args.output = args.output.resolve()
    args.rust_binary = args.rust_binary.resolve(strict=True)
    args.output.mkdir(parents=True, exist_ok=True)
    payload, metadata = workload()
    fixture = args.output / 'linked-fixture.bin'
    fixture.write_bytes(payload)
    (args.output / 'workload.json').write_text(json.dumps(metadata, indent=2) + '\n')
    results = {}
    for rule, number, path, function, control_column in [
        ('q1', 1, 'quake/WinQuake/world.c', 'SV_ClipToLinks', 10),
        ('q2', 2, 'quake-2/server/sv_world.c', 'SV_ClipMoveToEntities', 12),
        ('q3', 3, 'quake-iii-arena/code/server/sv_world.c', 'SV_ClipMoveToEntities', 0),
    ]:
        source_path = args.qsrc / path
        block, span = extract_merge(source_path.read_text(), function, rule)
        (args.output / f'{rule}-native-block.inc').write_text(block + '\n')
        template = (ROOT / 'tools/probes/linked_merge.c').read_text()
        marker = '/* QA_NATIVE_MERGE */'
        if template.count(marker) != 1:
            raise ValueError('native merge insertion marker count differs')
        generated = template.replace(marker, block)
        if generated.count(block) != 1:
            raise ValueError('native block changed or duplicated during insertion')
        reference = args.output / f'{rule}-native.c'
        reference.write_text(generated)
        executable = args.output / f'{rule}-native'
        started = time.monotonic()
        run([args.cc, *C_FLAGS, f'-DQA_RULE={number}', str(reference), '-o', str(executable)],
            args.output, f'{rule}-compile')
        compile_seconds = time.monotonic() - started
        expected, actual = args.output / f'{rule}-native.bin', args.output / f'{rule}-rust.bin'
        run([str(executable), str(fixture), str(expected)], args.output, f'{rule}-native')
        rust = run([str(args.rust_binary), rule, str(fixture), str(actual)], args.output, f'{rule}-rust')
        stats = json.loads(rust.stdout)
        if stats['rows'] != metadata['rows_per_rule'] or stats['rust_calling_thread_alloc_or_realloc'] != 0:
            raise ValueError('Rust linked merge row/allocation gate differs')
        native_bytes = expected.read_bytes()
        mutated, control = mutation_control(native_bytes, metadata, control_column)
        (args.output / f'{rule}-one-bit-control.bin').write_bytes(mutated)
        results[rule] = {**compare(native_bytes, actual.read_bytes(), metadata),
                         'rust': stats, 'native_source': str(source_path), 'extracted_block': span,
                         'native_compile_seconds': compile_seconds, 'mutation_control': control}
    report = {'scope': 'original linked-hit merge statement blocks on normalized fixtures',
              'limits': ['not complete SV_Trace, geometry, linked traversal/filtering or gameplay proof',
                         'prior allsolid fixtures call only the block; native traversal may stop before it',
                         'opaque neutral metadata proves assignment retention, not native ABI/wire encoding',
                         'Rust allocation counter covers calling thread only; no performance/native-heap claim'],
              'rust_binary': str(args.rust_binary), 'rust_build_performed_by_tool': False,
              'native_flags': C_FLAGS, 'workload': metadata, 'results': results,
              'result': 'PASS' if all(result['result'] == 'PASS' for result in results.values()) else 'FAIL'}
    (args.output / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report))
    return 0 if report['result'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
