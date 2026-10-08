#!/usr/bin/env python3
"""Compare production thinks with unchanged Q1/QW/Q2/rerelease/Q3 functions.

Build qa-session's think_native example separately. This developer tool builds
only extracted native fixtures, never Cargo or a game. Results are exact timing
and callback-side-effect evidence; they do not qualify a live server or gameplay.
"""
import argparse
import json
from pathlib import Path
import random
import re
import struct
import subprocess

ROOT = Path(__file__).resolve().parents[1]
ENTITIES = 3
MAX_CALLS = 16
OUTPUT_WORDS = 100
RECORD_BYTES = OUTPUT_WORDS * 8
MASK = (1 << 64) - 1
RULES = [
    ('q1', 1, 'quake/WinQuake/sv_phys.c', 'SV_RunThink'),
    ('qw', 2, 'quake/QW/server/sv_phys.c', 'SV_RunThink'),
    ('q2', 3, 'quake-2/game/g_phys.c', 'SV_RunThink'),
    ('rr', 4, 'quake2-rerelease-dll/rerelease/g_phys.cpp', 'SV_RunThink'),
    ('q3', 5, 'quake-iii-arena/code/game/g_main.c', 'G_RunThink'),
]
CPP_FLAGS = ['-std=c++17', '-O2', '-ffp-contract=off', '-fno-fast-math',
             '-fno-strict-aliasing']
COLUMNS = ['call_count', 'native_fault_or_scoped_rejection_count']
for entity in range(ENTITIES):
    COLUMNS += [f'entity_{entity + 1}.{name}' for name in
                ('deadline_raw', 'callback', 'alive', 'called', 'fault_or_rejected',
                 'current_lifetime')]
for call in range(MAX_CALLS):
    COLUMNS += [f'call_{call}.{name}' for name in
                ('entity_slot', 'function', 'timestamp_raw', 'cleared_deadline_raw',
                 'preserved_callback')]


def masked(source):
    tokens = re.compile(r'//[^\n]*|/\*[\s\S]*?\*/|"(?:\\.|[^"\\])*"|\'(?:\\.|[^\'\\])*\'')
    return tokens.sub(lambda match: ''.join('\n' if c == '\n' else ' ' for c in match[0]), source)


def balanced_end(code, opening):
    if code[opening] != '{':
        raise ValueError('native item opening brace missing')
    depth, end = 1, opening + 1
    while depth and end < len(code):
        depth += (code[end] == '{') - (code[end] == '}')
        end += 1
    if depth:
        raise ValueError('native item closing brace missing')
    return end


def exact_slice(source, start, end, name):
    value = source[start:end]
    start_line = 1 + source[:start].count('\n')
    source_lines = source.splitlines(keepends=True)
    offset = sum(len(line) for line in source_lines[:start_line - 1])
    if start != offset or source[offset:offset + len(value)] != value:
        raise ValueError(f'{name}: extracted native item is not an unchanged contiguous slice')
    return value, {'item': name, 'start_line': start_line,
                   'end_line': start_line + value.count('\n'),
                   'exact_contiguous_source_slice': True}


def extract_function(source, name):
    code = masked(source)
    pattern = rf'(?m)^[ \t]*(?:qboolean|bool|void)\s+{re.escape(name)}\s*\([^;{{}}]*\)\s*\{{'
    matches = list(re.finditer(pattern, code))
    if len(matches) != 1:
        raise ValueError(f'{name}: complete function definition count differs')
    match = matches[0]
    return exact_slice(source, match.start(), balanced_end(code, match.end() - 1), name)


def extract_rr_time(source):
    code = masked(source)
    matches = list(re.finditer(r'(?m)^struct\s+gtime_t\s*\{', code))
    if len(matches) != 1:
        raise ValueError('rerelease gtime_t definition count differs')
    match = matches[0]
    end = balanced_end(code, match.end() - 1)
    tail = re.match(r'\s*;', code[end:])
    if tail is None:
        raise ValueError('rerelease gtime_t semicolon missing')
    type_source, type_span = exact_slice(source, match.start(), end + tail.end(), 'gtime_t')
    # The literal's quotes are syntax, so match this signature on the original
    # source while locating its body braces on the masked source.
    literals = list(re.finditer(r'(?m)^constexpr\s+gtime_t\s+operator""\s+_ms\s*\([^;{}]*\)\s*\{', source))
    if len(literals) != 2:
        raise ValueError('rerelease _ms literal definition count differs')
    extracted, spans = [], [type_span]
    for match in literals:
        value, span = exact_slice(source, match.start(), balanced_end(code, match.end() - 1),
                                  'operator"" _ms')
        extracted.append(value)
        spans.append(span)
    return type_source, '\n\n'.join(extracted), spans


def bits(value):
    return struct.unpack('<Q', struct.pack('<d', value))[0]


def f32(value):
    return struct.unpack('<f', struct.pack('<f', value))[0]


def float_neighbor(value, direction):
    value = f32(value)
    if value < 0:
        raise ValueError('fixture neighbor expects a nonnegative float')
    n = struct.unpack('<I', struct.pack('<f', value))[0]
    if direction < 0 and n == 0:
        return -struct.unpack('<f', struct.pack('<I', 1))[0]
    return struct.unpack('<f', struct.pack('<I', n + direction))[0]


def entity(rule, now, step, due, function=1, mode=0, reschedule=None, repeats=0):
    if reschedule is None:
        reschedule = max(1, due)
    if rule in ('q1', 'qw', 'q2'):
        due, reschedule = f32(due), f32(reschedule)
        if rule == 'q2':
            now = f32(now)
        return (bits(now), bits(step), bits(due), function, mode, bits(reschedule), repeats)
    if rule == 'q3' and any(not -(1 << 31) <= value < (1 << 31)
                            for value in (now, due, reschedule)):
        raise ValueError('Q3 generated fixture exceeds int32')
    return (now & MASK, bits(0.0), due & MASK, function, mode, reschedule & MASK, repeats)


def workload(rule, seeded_rows):
    cases, labels = [], []
    rng = random.Random(709)

    def add(focus, label):
        index = len(cases)
        if rule in ('q1', 'qw', 'q2'):
            secondary = entity(rule, 17.25, 0.1, 17.25, 2, 0)
            third = entity(rule, 33.5, 0.05, 33.5, 1, 2, 33.5, 1)
        else:
            secondary = entity(rule, 17250, 0, 17250, 2, 0)
            third = entity(rule, 33500, 0, 33500, 1, 2, 33500, 1)
        cases.append((index & 1, focus, secondary, third))
        labels.append(label)

    if rule in ('q1', 'qw', 'q2'):
        clocks = [0.0, 1.0, 10.0, 1024.0, 65536.0, 16777216.0]
        if rule != 'q2':
            # Double clock values strictly between adjacent float deadlines.
            clocks += [1.0 + 2 ** -25, 16777216.25, 16777217.0]
        for now in clocks:
            for step in [0.0, 1 / 72, 0.1]:
                boundary = now + (0.001 if rule == 'q2' else step)
                due_values = [-1.0, 0.0, float_neighbor(0.0, 1),
                              float_neighbor(now, -1), f32(now), float_neighbor(now, 1),
                              float_neighbor(boundary, -1), f32(boundary),
                              float_neighbor(boundary, 1)]
                for due in dict.fromkeys(due_values):
                    for function, mode, repeats in [(1, 0, 0), (0, 0, 0), (1, 1, 3),
                                                     (1, 2, 2), (2, 3, 0)]:
                        # A future/past reschedule remains finite in all rules.
                        reschedule = f32(max(1.0, now))
                        add(entity(rule, now, step, due, function, mode, reschedule, repeats),
                            f'boundary now={now!r},step={step!r},due={due!r},fn={function},mode={mode}')
        for _ in range(seeded_rows):
            now = rng.randrange(1, 1000000) / 128
            if rule != 'q2':
                now += rng.choice([0, 2 ** -30, -2 ** -30])
            step = rng.choice([0.0, 1 / 72, 1 / 85, 0.001, 0.1])
            due = now + rng.choice([-0.5, 0, 0.000999, 0.001, 0.001001, step, step + 0.01])
            add(entity(rule, now, step, due, rng.randrange(3), rng.randrange(4),
                       max(1.0, now + rng.choice([-0.25, 0, step, step + 0.25])), rng.randrange(4)),
                'seeded float boundary and callback mode')
    else:
        clocks = [0, 1, 1000]
        if rule == 'rr':
            clocks += [(1 << 24) + 1, (1 << 53) - 1, (1 << 53) + 1,
                       (1 << 60) + 12345, (1 << 63) - 4]
        else:
            clocks += list(range((1 << 24) - 3, (1 << 24) + 5))
            clocks += [(1 << 25) + 3, (1 << 31) - 1]
        for now in clocks:
            due_values = [-1, 0, 1, now - 2, now - 1, now, now + 1, now + 2]
            if rule == 'q3':
                due_values = [v for v in due_values if -(1 << 31) <= v < (1 << 31)]
            for due in dict.fromkeys(due_values):
                for function, mode, repeats in [(1, 0, 0), (0, 0, 0), (1, 1, 3),
                                                 (1, 2, 2), (2, 3, 0)]:
                    add(entity(rule, now, 0, due, function, mode, max(1, now), repeats),
                        f'boundary now={now},due={due},fn={function},mode={mode}')
        for _ in range(seeded_rows):
            if rule == 'rr':
                now = rng.choice([1000, 1 << 24, 1 << 53, 1 << 60]) + rng.randrange(-64, 65)
            else:
                now = rng.choice([1000, 1 << 24, 1 << 25]) + rng.randrange(-64, 65)
            due = now + rng.randrange(-4, 5)
            add(entity(rule, now, 0, due, rng.randrange(3), rng.randrange(4),
                       now + rng.randrange(-4, 5), rng.randrange(4)),
                'seeded integer boundary and callback mode')
    payload = bytearray(struct.pack('<3I', 0x4b4e4854, 1, len(cases)))
    for order, *entities in cases:
        payload.extend(struct.pack('<Q', order))
        for value in entities:
            payload.extend(struct.pack('<7Q', *value))
    return payload, {'rows': len(cases), 'entities_per_row': ENTITIES,
                     'seed': 709, 'seeded_rows': seeded_rows, 'labels': labels,
                     'caller_orders': [[1, 2, 3], [3, 1, 2]],
                     'independent_module_clocks': True,
                     'boundary_adapters': 'seconds deadlines/reschedules f32; Q2 clock f32; Q3 i32; RR i64',
                     'callback_modes': ['no-op', 'finite reschedule', 'change function/reschedule',
                                        'free without reuse', 'null callback'],
                     'null_callback_difference': 'native fatal caught by fixture; Rust scoped rejection',
                     'lifetime_scope': 'free teardown normalized; no within-call slot reuse'}


def compare(expected, actual, metadata):
    rows = metadata['rows']
    if len(expected) != rows * RECORD_BYTES or len(actual) != rows * RECORD_BYTES:
        raise ValueError('native/Rust think output record count differs')
    matched, mismatch = 0, []
    differences = dict.fromkeys(COLUMNS, 0)
    for index, (native, rust) in enumerate(zip(struct.iter_unpack('<100Q', expected),
                                               struct.iter_unpack('<100Q', actual))):
        if native == rust:
            matched += 1
            continue
        fields = []
        for column, left, right in zip(COLUMNS, native, rust):
            if left != right:
                differences[column] += 1
                fields.append({'field': column, 'native_raw': f'{left:016x}',
                               'rust_raw': f'{right:016x}'})
        if len(mismatch) < 20:
            mismatch.append({'row': index, 'case': metadata['labels'][index], 'fields': fields})
    return {'rows_compared': rows, 'rows_matched_bit_exact': matched,
            'different_fields': {key: value for key, value in differences.items() if value},
            'first_mismatches': mismatch, 'result': 'PASS' if matched == rows else 'FAIL'}


def controls(expected, metadata):
    positive = compare(expected, expected, metadata)
    if positive['result'] != 'PASS':
        raise ValueError('identical-record comparator positive control failed')
    mutated = bytearray(expected)
    row, column = metadata['rows'] // 2, 22  # First callback raw timestamp.
    mutated[row * RECORD_BYTES + column * 8] ^= 1
    negative = compare(expected, mutated, metadata)
    if (negative['result'] != 'FAIL' or negative['rows_matched_bit_exact'] != metadata['rows'] - 1
            or negative['different_fields'] != {COLUMNS[column]: 1}):
        raise ValueError('one-bit timestamp mutation comparator control was not rejected exactly')
    return mutated, {'scope': 'comparator sensitivity, not independent native semantics',
                     'identical_positive_control': 'PASS', 'one_bit_mutation_rejected': True,
                     'row': row, 'field': COLUMNS[column]}


def run(command, output, name):
    result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, timeout=300)
    (output / f'{name}.log').write_text(result.stdout + result.stderr)
    result.check_returncode()
    return result


def source_for(qsrc, rule, path, function, output):
    original = (qsrc / path).read_text()
    native, span = extract_function(original, function)
    span['path'] = path
    sources = [span]
    replacements = {'/* QA_NATIVE_THINK */': native, '/* QA_NATIVE_GTIME */': '',
                    '/* QA_NATIVE_MS_LITERALS */': ''}
    (output / f'{rule}-original-function.inc').write_text(native + '\n')
    if rule == 'rr':
        time_path = 'quake2-rerelease-dll/rerelease/g_local.h'
        time_type, literals, spans = extract_rr_time((qsrc / time_path).read_text())
        for value in spans:
            value['path'] = time_path
        sources += spans
        replacements['/* QA_NATIVE_GTIME */'] = time_type
        replacements['/* QA_NATIVE_MS_LITERALS */'] = literals
        (output / 'rr-original-gtime.inc').write_text(time_type + '\n\n' + literals + '\n')
    generated = (ROOT / 'tools/probes/think_native.cpp').read_text()
    for marker, value in replacements.items():
        if generated.count(marker) != 1:
            raise ValueError('native function/type insertion marker count differs')
        generated = generated.replace(marker, value)
        if value and generated.count(value) != 1:
            raise ValueError('native function/type changed or duplicated during insertion')
    target = output / f'{rule}-native.cpp'
    target.write_text(generated)
    return target, sources


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--qsrc', type=Path, required=True)
    parser.add_argument('--rust-binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--cxx', default='c++')
    parser.add_argument('--seeded-rows', type=int, default=2000)
    args = parser.parse_args()
    if not 1 <= args.seeded_rows <= 40000:
        parser.error('--seeded-rows must be 1..40000')
    args.qsrc = args.qsrc.resolve(strict=True)
    args.rust_binary = args.rust_binary.resolve(strict=True)
    args.output = args.output.resolve()
    args.output.mkdir(parents=True, exist_ok=True)
    results = {}
    for rule, number, path, function in RULES:
        payload, metadata = workload(rule, args.seeded_rows)
        fixture = args.output / f'{rule}-fixture.bin'
        fixture.write_bytes(payload)
        (args.output / f'{rule}-workload.json').write_text(json.dumps(metadata, indent=2) + '\n')
        source, extraction = source_for(args.qsrc, rule, path, function, args.output)
        binary = args.output / f'{rule}-native'
        run([args.cxx, *CPP_FLAGS, f'-DQA_RULE={number}', str(source), '-o', str(binary)],
            args.output, f'{rule}-compile')
        expected, actual = args.output / f'{rule}-native.bin', args.output / f'{rule}-rust.bin'
        run([str(binary), str(fixture), str(expected)], args.output, f'{rule}-native')
        rust = run([str(args.rust_binary), rule, str(fixture), str(actual)],
                   args.output, f'{rule}-rust')
        allocation = json.loads(rust.stdout)
        if (allocation['rows'] != metadata['rows'] or allocation['rule'] != rule
                or allocation['rust_calling_thread_alloc_or_realloc'] != 0
                or allocation['allocation_positive_control'] < 1):
            raise ValueError('Rust think row/allocation positive or zero gate differs')
        native = expected.read_bytes()
        outcome = compare(native, actual.read_bytes(), metadata)
        mutated, control = controls(native, metadata)
        (args.output / f'{rule}-timestamp-mutation.bin').write_bytes(mutated)
        native_rows = list(struct.iter_unpack('<100Q', native))
        outcome.update({'source_extraction': extraction, 'comparator_controls': control,
                        'allocation': allocation, 'native_callback_calls': sum(row[0] for row in native_rows),
                        'native_fatal_caught_count': sum(row[1] for row in native_rows),
                        'native_fatal_is_not_runtime_parity': True,
                        'cpp_flags': CPP_FLAGS})
        results[rule] = outcome
    report = {'scope': 'unchanged native per-entity think functions vs production dispatch',
              'gameplay': False, 'installation_qualified': False, 'performance_measured': False,
              'limits': ['minimal native types/callbacks; no module VM or complete server',
                        'null callback native fatal is caught and compared with scoped Rust rejection',
                        'free teardown is normalized; no within-call slot reuse',
                        'caller order is explicit fixture order, not complete native frame traversal',
                        'allocation scope is this single Rust calling thread after cold load'],
              'rules': results,
              'result': 'PASS' if all(value['result'] == 'PASS' for value in results.values()) else 'FAIL'}
    (args.output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'report': str(args.output / 'report.json'), 'result': report['result'],
                      'scope': report['scope']}))
    if report['result'] != 'PASS':
        raise SystemExit(1)


if __name__ == '__main__':
    main()
