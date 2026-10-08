#!/usr/bin/env python3
"""Compare actual Rust scene DrawItems with extracted original Q3 draw sorting.

Only this developer helper compiles the original C. No game, display or audio
is involved. Fixture keys preserve the Rust material-handle comparison relation.
"""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]


def function(source, name):
    masked = re.sub(r'/\*.*?\*/|//[^\n]*|"(?:\\.|[^"\\])*"',
                    lambda m: ''.join('\n' if c == '\n' else ' ' for c in m[0]),
                    source, flags=re.S)
    match = re.search(r'(?m)^[ \t]*(?:static\s+)?[A-Za-z_][A-Za-z0-9_ \t*]*\b'
                      + re.escape(name) + r'\s*\([^;{}]*\)\s*\{', masked)
    if not match:
        raise RuntimeError(f'original function missing: {name}')
    start = match.start()
    opening = match.end() - 1
    depth, end = 1, opening + 1
    while depth:
        if masked[end] == '{': depth += 1
        if masked[end] == '}': depth -= 1
        end += 1
    return source[start:end], source.count('\n', 0, start) + 1, source.count('\n', 0, end) + 1


PRELUDE = r'''
#include <stdio.h>
#include <stdlib.h>
#include <stdarg.h>
#include <string.h>
#define ERR_DROP 1
/* Original SWAP_DRAW_SURF requires its original two-word, eight-byte layout.
   surface is an opaque payload number, preserving the original 32-bit word. */
typedef struct { unsigned sort; unsigned surface; } drawSurf_t;
static void original_error(int code, const char *format, ...) {
    (void)code; (void)format;
    fputs("original qsort layout error\n",stderr); exit(2);
}
static struct { void (*Error)(int,const char *,...); } ri={original_error};
'''

DRIVER = r'''
int main(int argc, char **argv) {
    if(argc!=2) return 2;
    FILE *input=fopen(argv[1],"r");
    if(!input) return 2;
    char tag[16],name[64]; unsigned count;
    while(fscanf(input,"%15s",tag)==1) {
        if(strcmp(tag,"CASE") || fscanf(input,"%63s%u",name,&count)!=2 || count>1024) return 2;
        drawSurf_t *draws=malloc((count?count:1)*sizeof(*draws));
        if(!draws) return 2;
        for(unsigned i=0;i<count;i++) {
            if(fscanf(input,"%15s%u%u",tag,&draws[i].sort,&draws[i].surface)!=3 || strcmp(tag,"ITEM")) return 2;
        }
        qsortFast(draws,count,sizeof(*draws));
        printf("CASE %s %u\n",name,count);
        for(unsigned i=0;i<count;i++) printf("ITEM %u %u\n",draws[i].sort,draws[i].surface);
        free(draws);
    }
    if(ferror(input)) return 2;
    fclose(input);
    return 0;
}
'''


def parse_rows(text):
    rows = text.splitlines()
    cases, index = {}, 0
    while index < len(rows):
        header = rows[index].split()
        if len(header) != 3 or header[0] != 'CASE' or header[1] in cases:
            raise RuntimeError(f'invalid or duplicate fixture header at row {index}')
        count = int(header[2])
        if count < 0 or count > 1024:
            raise RuntimeError('fixture count outside comparison boundary')
        index += 1
        items = []
        for _ in range(count):
            if index >= len(rows): raise RuntimeError('truncated fixture')
            item = rows[index].split()
            if len(item) != 3 or item[0] != 'ITEM': raise RuntimeError(f'invalid item row {index}')
            items.append((int(item[1]), int(item[2])))
            index += 1
        cases[header[1]] = items
    return cases


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--qsrc', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.qsrc = args.qsrc.resolve()
    args.output = args.output.resolve()
    args.output.mkdir(parents=True, exist_ok=True)
    relative = 'quake-iii-arena/code/renderer/tr_main.c'
    original = (args.qsrc / relative).read_text()
    references = []
    source = PRELUDE
    for name in ['SWAP_DRAW_SURF', 'CUTOFF']:
        match = re.search(r'(?m)^#define[ \t]+' + name + r'[^\n]*', original)
        if not match: raise RuntimeError(f'original macro missing: {name}')
        line = original.count('\n', 0, match.start()) + 1
        source += f'\n#line {line} "{relative}"\n{match[0]}\n'
        references.append({'source': relative, 'macro': name, 'start': line, 'end': line})
    for name in ['shortsort', 'qsortFast']:
        body, start, end = function(original, name)
        source += f'\n#line {start} "{relative}"\n{body}\n'
        references.append({'source': relative, 'function': name, 'start': start, 'end': end})
    source += '\n#line 1 "draw-sort-driver"\n' + DRIVER
    path = args.output / 'original_draw_sort.c'
    path.write_text(source)
    executable = args.output / 'original_draw_sort'
    flags = ['-std=c99', '-O2', '-ffp-contract=off', '-fno-strict-aliasing']
    start = time.monotonic()
    compiled = subprocess.run(['cc', *flags, str(path), '-o', str(executable)], capture_output=True, text=True, timeout=300)
    c_seconds = time.monotonic() - start
    (args.output / 'c-build.log').write_text(compiled.stdout + compiled.stderr)
    compiled.check_returncode()
    env = dict(os.environ, CARGO_TARGET_DIR=os.environ.get('CARGO_TARGET_DIR', 'target'), QA_DRAW_SORT_QSRC=str(args.qsrc), QA_DRAW_SORT_EVIDENCE=str(args.output))
    start = time.monotonic()
    tested = subprocess.run(['cargo', 'test', '--release', '-p', 'qa-render', '--test', 'draw_sort_original', 'original_draw_sort_fixture_export', '--', '--nocapture'], cwd=ROOT, env=env, capture_output=True, text=True, timeout=300)
    rust_seconds = time.monotonic() - start
    (args.output / 'rust-build.log').write_text(tested.stdout + tested.stderr)
    tested.check_returncode()
    native = subprocess.run([str(executable), str(args.output/'input.txt')], capture_output=True, text=True, timeout=300)
    (args.output / 'original.txt').write_text(native.stdout)
    (args.output / 'original-stderr.txt').write_text(native.stderr)
    native.check_returncode()
    before = parse_rows((args.output/'input.txt').read_text())
    expected = parse_rows(native.stdout)
    actual = parse_rows((args.output/'rust.txt').read_text())
    if list(before) != list(expected) or list(expected) != list(actual):
        raise RuntimeError('fixture names/order do not match')
    results = []
    for name in before:
        if expected[name] != actual[name]:
            row = next((i for i, pair in enumerate(zip(expected[name], actual[name])) if pair[0] != pair[1]), min(len(expected[name]), len(actual[name])))
            raise RuntimeError(f'native payload order mismatch: {name} row {row}')
        if sorted(before[name]) != sorted(actual[name]):
            raise RuntimeError(f'payload permutation mismatch: {name}')
        results.append({'fixture': name, 'items': len(before[name]), 'distinct_keys': len({item[0] for item in before[name]}), 'result': 'PASS'})
    required = ['equal_9', 'equal_17', 'mixed_9_alternating', 'mixed_9_partition']
    required += [f'cutoff_equal_{n}' for n in range(9)]
    if any(name not in actual for name in required) or not any(name.startswith('seed_') for name in actual):
        raise RuntimeError('required comparison fixtures missing')
    revision = subprocess.run(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True,
                              capture_output=True, check=False)
    status = subprocess.run(['git', 'status', '--porcelain'], cwd=ROOT, text=True,
                            capture_output=True, check=False)
    report = {
        'scope': 'actual FrontEnd poly DrawItems; exact unsigned-key comparison relation; no full packed-key or renderer image proof',
        'commit': revision.stdout.strip() if revision.returncode == 0 else None,
        'source_tree_dirty': bool(status.stdout) if status.returncode == 0 else None,
        'references': references, 'original_body_modifications': 0,
        'standalone_adapter': 'two unsigned 32-bit words preserve original SWAP_DRAW_SURF layout; ri.Error exits on layout failure',
        'key_mapping': 'equal material sort/instance/lightmap, monotonic material handles provide native unsigned-key ordering',
        'reference_flags': flags, 'c_build_seconds': c_seconds, 'rust_build_and_test_seconds': rust_seconds,
        'fixtures': len(results), 'items': sum(row['items'] for row in results),
        'seeded_fixtures': sum(row['fixture'].startswith('seed_') for row in results),
        'comparison': 'exact keys and payload order', 'results': results, 'result': 'PASS',
    }
    (args.output/'result.json').write_text(json.dumps(report, indent=2)+'\n')
    print(json.dumps({key: value for key,value in report.items() if key not in ['references','results']}))


if __name__ == '__main__':
    main()
