#!/usr/bin/env python3
"""Compare every owner CSV cell with the compiled Rust catalog, without hashes."""
import argparse
import csv
import json
from pathlib import Path
import struct
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--evidence', type=Path, required=True)
    args = parser.parse_args()
    args.evidence.mkdir(parents=True, exist_ok=False)
    subprocess.run(['python3', str(ROOT/'tools/gen_cvars.py'), '--check'], check=True)
    p = subprocess.run(['cargo', 'run', '--release', '-p', 'qa-console', '--example', 'catalog_cells'],
        cwd=ROOT, capture_output=True, check=True)
    (args.evidence/'compiled-cells.bin').write_bytes(p.stdout)
    (args.evidence/'build.log').write_bytes(p.stderr)
    with (ROOT/'data/unified-cvars.csv').open(newline='', encoding='utf-8') as source:
        rows = list(csv.DictReader(source))
    raw, offset = p.stdout, 4
    if struct.unpack_from('<I', raw)[0] != len(rows):
        raise ValueError('compiled row count differs')
    differences, cells = [], 0
    for row in rows:
        for key, expected in row.items():
            length = struct.unpack_from('<I', raw, offset)[0]
            offset += 4
            actual = raw[offset:offset+length].decode('utf-8')
            offset += length
            cells += 1
            if actual != expected:
                differences.append(dict(row=row['canonical'], field=key))
    if offset != len(raw) or differences:
        raise ValueError(f'compiled catalog differs: {differences}')
    report = dict(result='PASS', rows=len(rows), cells=cells, differences=0,
        scope='compiled catalog metadata; not live cvar consumers')
    (args.evidence/'comparison.json').write_text(json.dumps(report, indent=2)+'\n')
    print(json.dumps(report))


if __name__ == '__main__':
    main()
