#!/usr/bin/env python3
"""Compare the shared target index with unchanged native Q1/QW/Q2/Q3 find.

Build qa-world's target_native example separately. This developer tool compiles
only original-source C fixtures and never launches Cargo or a game. Native slots
equal common slots here; VM strings and module entity namespaces are not proved.
"""
import argparse
import json
import os
from pathlib import Path
import random
import re
import struct
import subprocess

ROOT = Path(__file__).resolve().parents[1]
NONE = 0xffffffff
MAX_SLOTS = 64
WORDS = 3 + MAX_SLOTS
RECORD_BYTES = WORDS * 4
COLUMNS = ['match_count', 'terminal_native_entity', 'native_fault_or_boundary_rejection']
COLUMNS += [f'matched_slot_{index}' for index in range(MAX_SLOTS)]
C_FLAGS = ['-std=c99', '-O2', '-fno-strict-aliasing']
RULES = [
    ('q1', 1, 'quake/WinQuake/pr_cmds.c', 'PF_Find'),
    ('qw', 2, 'quake/QW/server/pr_cmds.c', 'PF_Find'),
    ('q2', 3, 'quake-2/game/g_utils.c', 'G_Find'),
    ('q3', 4, 'quake-iii-arena/code/game/g_utils.c', 'G_Find'),
]


def masked(source):
    tokens = re.compile(r'//[^\n]*|/\*[\s\S]*?\*/|"(?:\\.|[^"\\])*"|\'(?:\\.|[^\'\\])*\'')
    return tokens.sub(lambda match: ''.join('\n' if c == '\n' else ' ' for c in match[0]), source)


def balanced_end(code, opening):
    if code[opening] != '{':
        raise ValueError('native function opening brace missing')
    depth, end = 1, opening + 1
    while depth and end < len(code):
        depth += (code[end] == '{') - (code[end] == '}')
        end += 1
    if depth:
        raise ValueError('native function closing brace missing')
    return end


def extract_function(source, name):
    code = masked(source)
    pattern = rf'(?m)^[ \t]*(?:void|int|edict_t|gentity_t)\s+\*?\s*{re.escape(name)}\s*\([^;{{}}]*\)'
    matches = list(re.finditer(pattern, code))
    if len(matches) != 1:
        raise ValueError(f'{name}: complete native definition count differs')
    match = matches[0]
    tail = code[match.end():]
    conditional = re.match(r'\s*#\s*ifdef\s+QUAKE2\b', tail)
    conditional_body = conditional is not None
    if conditional_body:
        # PF_Find's signature precedes an entire #ifdef/#else with two bodies.
        # Retain the complete conditional, rather than extracting one body and
        # silently changing the source. The compile never defines QUAKE2.
        depth, end = 0, None
        for directive in re.finditer(r'(?m)^[ \t]*#\s*(if|ifdef|ifndef|elif|else|endif)\b[^\n]*', tail):
            kind = directive[1]
            if kind in ('if', 'ifdef', 'ifndef'):
                depth += 1
            elif kind == 'endif':
                depth -= 1
                if depth == 0:
                    end = match.end() + directive.end()
                    break
        if end is None:
            raise ValueError('native PF_Find conditional ending missing')
    else:
        body = re.match(r'\s*\{', tail)
        if body is None:
            raise ValueError(f'{name}: native function body missing')
        end = balanced_end(code, match.end() + body.end() - 1)
    start, value = match.start(), source[match.start():end]
    line = source[:start].count('\n') + 1
    offset = sum(len(item) for item in source.splitlines(keepends=True)[:line - 1])
    if offset != start or source[offset:offset + len(value)] != value:
        raise ValueError('native extraction is not an unchanged contiguous source slice')
    return value, {'function': name, 'start_line': line, 'end_line': line + value.count('\n'),
                   'exact_contiguous_source_slice': True,
                   'complete_conditional_bodies_retained': conditional_body}


def workload(rule, seeded_rows):
    names = [b'', b'door', b'Door', b'DOOR', b'trigger', b'Trigger', b'world',
             b'WORLD', b'absent', b'\xffDoor', b'\xffdoor', b'\xffDOOR',
             b'\xfeDoor', b'\xfeDOOR', b'\xc3\x9fDoor', b'\xc3\x9fdoor',
             b'\xc3\x9fDOOR', b'a', b'A', b'Alpha', b'ALPHA', b'alpha',
             b'prefix', b'prefix_more', b'prefix\xff', b'\x80x', b'\x80X',
             b'\xe4', b'\xc4', b' ', b'CaseSensitiveOnly', b'casesensitiveonly']
    rng = random.Random(617)
    # Slot zero is always the active world lifetime. Field absence is separate
    # from an explicit empty string in every table and in the serialized pool.
    focused = [(1, 0), (1, NONE), (1, 0), (1, 1), (1, 2), (0, 3),
               (1, 3), (1, 4), (0, 1), (1, 9), (1, 10), (1, 12),
               (1, 14), (1, 15), (1, 25), (1, 26)]
    tables = [focused]
    tables.append([(1 if slot == 0 or slot % 3 else 0, name)
                   for slot, (_, name) in enumerate(focused)])
    tables.append([(1, 0)] + [(1, NONE)] * 15)
    tables.append([(1, 0)] + [(slot % 4 != 0, [1, 2, 3][slot % 3]) for slot in range(1, 32)])
    tables.append([(1, 0)] + [(slot % 5 != 0, [9, 10, 11, 12, 13, 14, 15, 16][slot % 8])
                             for slot in range(1, 32)])
    tables.append([(1, 0)] + [(slot % 6 != 0, NONE if slot % 2 else 0) for slot in range(1, 32)])
    for _ in range(2):
        tables.append([(1, rng.choice([0, 6, NONE]))]
                      + [(rng.randrange(2), rng.choice([NONE, *range(len(names))]))
                         for _ in range(63)])
    queries, labels = [], []

    def add(table, start, match, kind):
        queries.append((table, start, match))
        labels.append({'kind': kind, 'table': table,
                       'start': 'native NULL-from' if start == NONE else start,
                       'match': 'native NULL' if match == NONE else names[match].hex()})

    for table, rows in enumerate(tables):
        starts = [0, 1, 3, len(rows) - 1]
        if rule in ('q2', 'q3'):
            starts += [NONE]
        matches = list(range(len(names)))
        if rule != 'q2':
            matches += [NONE]
        for start in starts:
            for match in matches:
                add(table, start, match, 'focused empty/NULL/case/nonUTF8/inactive/start')
    for _ in range(seeded_rows):
        table = rng.randrange(len(tables))
        start = rng.randrange(len(tables[table]))
        if rule in ('q2', 'q3') and rng.randrange(5) == 0:
            start = NONE
        match = rng.randrange(len(names))
        if rule != 'q2' and rng.randrange(19) == 0:
            match = NONE
        add(table, start, match, 'seeded raw-name and source-start selection')
    data = bytearray(struct.pack('<5I', 0x47524154, 1, len(names), len(tables), len(queries)))
    for name in names:
        data.extend(struct.pack('<I', len(name)))
        data.extend(name)
    for table in tables:
        data.extend(struct.pack('<I', len(table)))
        for active, name in table:
            data.extend(struct.pack('<2I', int(active), name))
    for query in queries:
        data.extend(struct.pack('<3I', *query))
    return data, {'rows': len(queries), 'seed': 617, 'seeded_rows': seeded_rows,
                  'names_hex': [name.hex() for name in names],
                  'tables': [[{'active': bool(active), 'name': None if name == NONE else name}
                              for active, name in table] for table in tables], 'labels': labels,
                  'match_mode': 'Exact' if rule in ('q1', 'qw') else 'Folded',
                  'null_field_is_distinct_from_empty': True,
                  'native_common_slots_equal': True,
                  'q2_null_query': 'excluded; native strcasecmp NULL dereference is undefined',
                  'q1_null_query': 'native fatal caught; Rust fixture boundary rejects request',
                  'embedded_nul': 'excluded; native names terminate at NUL'}


def compare(expected, actual, metadata):
    rows = metadata['rows']
    if len(expected) != rows * RECORD_BYTES or len(actual) != rows * RECORD_BYTES:
        raise ValueError('native/Rust target output record count differs')
    matched, mismatch = 0, []
    different = dict.fromkeys(COLUMNS, 0)
    for index, (native, rust) in enumerate(zip(struct.iter_unpack(f'<{WORDS}I', expected),
                                               struct.iter_unpack(f'<{WORDS}I', actual))):
        if native == rust:
            matched += 1
            continue
        fields = []
        for column, left, right in zip(COLUMNS, native, rust):
            if left != right:
                different[column] += 1
                fields.append({'field': column, 'native': left, 'rust': right})
        if len(mismatch) < 20:
            mismatch.append({'row': index, 'query': metadata['labels'][index], 'fields': fields})
    return {'rows_compared': rows, 'rows_matched_exact': matched,
            'different_fields': {key: value for key, value in different.items() if value},
            'first_mismatches': mismatch, 'result': 'PASS' if rows == matched else 'FAIL'}


def controls(expected, metadata):
    if compare(expected, expected, metadata)['result'] != 'PASS':
        raise ValueError('identical-record target comparator positive control failed')
    first = next((index for index, row in enumerate(struct.iter_unpack(f'<{WORDS}I', expected))
                  if row[0]), None)
    if first is None:
        raise ValueError('native target positive corpus produced no matching slots')
    mutated = bytearray(expected)
    column = 3
    mutated[first * RECORD_BYTES + column * 4] ^= 1
    outcome = compare(expected, mutated, metadata)
    if (outcome['result'] != 'FAIL' or outcome['rows_matched_exact'] != metadata['rows'] - 1
            or outcome['different_fields'] != {COLUMNS[column]: 1}):
        raise ValueError('one-bit target-result mutation was not rejected exactly')
    return mutated, {'scope': 'comparator sensitivity, not independent native semantics',
                     'identical_positive_control': 'PASS', 'one_bit_slot_mutation_rejected': True,
                     'row': first, 'field': COLUMNS[column]}


def run(command, output, name):
    result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, timeout=300,
                            env={**os.environ, 'LC_ALL': 'C'})
    (output / f'{name}.log').write_text(result.stdout + result.stderr)
    result.check_returncode()
    return result


def source_for(qsrc, rule, path, function, output):
    native, span = extract_function((qsrc / path).read_text(), function)
    span['path'] = path
    spans, comparison = [span], []
    if rule == 'q2':
        helpers_path, helpers = 'quake-2/game/q_shared.c', ['Q_stricmp']
    elif rule == 'q3':
        helpers_path, helpers = 'quake-iii-arena/code/game/q_shared.c', ['Q_stricmpn', 'Q_stricmp']
    else:
        helpers_path, helpers = None, []
    for helper in helpers:
        value, helper_span = extract_function((qsrc / helpers_path).read_text(), helper)
        helper_span['path'] = helpers_path
        spans.append(helper_span)
        comparison.append(value)
        (output / f'{rule}-{helper}-original.inc').write_text(value + '\n')
    template = (ROOT / 'tools/probes/targets_native.c').read_text()
    values = {'/* QA_NATIVE_COMPARISON */': '\n\n'.join(comparison),
              '/* QA_NATIVE_FIND */': native}
    for marker, value in values.items():
        if template.count(marker) != 1:
            raise ValueError('native target insertion marker count differs')
        template = template.replace(marker, value)
    for value in [native, *comparison]:
        if template.count(value) != 1:
            raise ValueError('native target function/helper changed or duplicated during insertion')
    (output / f'{rule}-original-function.inc').write_text(native + '\n')
    source = output / f'{rule}-native.c'
    source.write_text(template)
    return source, spans


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--qsrc', type=Path, required=True)
    parser.add_argument('--rust-binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--cc', default='cc')
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
        run([args.cc, *C_FLAGS, f'-DQA_RULE={number}', str(source), '-o', str(binary)],
            args.output, f'{rule}-compile')
        expected, actual = args.output / f'{rule}-native.bin', args.output / f'{rule}-rust.bin'
        run([str(binary), str(fixture), str(expected)], args.output, f'{rule}-native')
        rust = run([str(args.rust_binary), rule, str(fixture), str(actual)],
                   args.output, f'{rule}-rust')
        allocation = json.loads(rust.stdout)
        if (allocation['rows'] != metadata['rows'] or allocation['rule'] != rule
                or allocation['rust_calling_thread_alloc_or_realloc'] != 0
                or allocation['allocation_positive_control'] < 1):
            raise ValueError('Rust target row/allocation positive or zero gate differs')
        native = expected.read_bytes()
        outcome = compare(native, actual.read_bytes(), metadata)
        mutated, control = controls(native, metadata)
        (args.output / f'{rule}-slot-mutation.bin').write_bytes(mutated)
        rows = list(struct.iter_unpack(f'<{WORDS}I', native))
        outcome.update({'source_extraction': extraction, 'comparator_controls': control,
                        'allocation': allocation, 'native_matched_slots': sum(row[0] for row in rows),
                        'native_fatal_caught_count': sum(row[2] for row in rows),
                        'match_mode': metadata['match_mode'], 'locale': 'C',
                        'q1_quake2_define': False, 'c_flags': C_FLAGS})
        results[rule] = outcome
    report = {'scope': 'unchanged native find functions vs shared target index',
              'gameplay': False, 'installation_qualified': False, 'performance_measured': False,
              'limits': ['native slot equals common slot; no arbitrary module namespace mapping',
                        'minimal field/string adapters; no QuakeC/module ABI or complete server',
                        'Q1 complete #ifdef QUAKE2 function retained; native default branch compiled',
                        'Q2 unchanged Q_stricmp invokes host strcasecmp in C locale; Windows excluded',
                        'Q1/QW NULL query fatal caught; Rust fixture rejects at typed boundary',
                        'Q2 NULL query excluded as undefined; NULL fields tested separately from empty',
                        'embedded NUL excluded by native C-string boundary',
                        'calling-thread allocation scope covers hot lookup and unchanged refresh only'],
              'rules': results,
              'result': 'PASS' if all(value['result'] == 'PASS' for value in results.values()) else 'FAIL'}
    (args.output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'report': str(args.output / 'report.json'), 'result': report['result'],
                      'scope': report['scope']}))
    if report['result'] != 'PASS':
        raise SystemExit(1)


if __name__ == '__main__':
    main()
