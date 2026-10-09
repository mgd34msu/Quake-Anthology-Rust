#!/usr/bin/env python3
"""Compare every owner CSV cell with the compiled Rust catalog, without hashes."""
import argparse
import json
from pathlib import Path
import struct
import subprocess
from gen_cvars import load_catalog

ROOT = Path(__file__).resolve().parents[1]


def compare_cells(raw, owner_rows, engine_rows):
    rows = owner_rows + engine_rows
    if struct.unpack_from('<I', raw)[0] != len(rows):
        raise ValueError('compiled row count differs')
    differences, offset = [], 4
    for index, row in enumerate(rows):
        for key, expected in row.items():
            length = struct.unpack_from('<I', raw, offset)[0]
            offset += 4
            actual = raw[offset:offset+length].decode('utf-8')
            offset += length
            if actual != expected:
                differences.append(dict(row=row['canonical'], field=key,
                    source='owner' if index < len(owner_rows) else 'engine-extension'))
    if offset != len(raw) or differences:
        raise ValueError(f'compiled catalog differs: {differences}')
    return dict(result='PASS', rows=len(owner_rows), cells=sum(len(row) for row in owner_rows),
        engine_rows=len(engine_rows), engine_cells=sum(len(row) for row in engine_rows), differences=0,
        scope='compiled owner cells and engine-extension metadata; not live cvar consumers')


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
    _, owner_rows, engine_rows = load_catalog(ROOT)
    report = compare_cells(p.stdout, owner_rows, engine_rows)
    (args.evidence/'comparison.json').write_text(json.dumps(report, indent=2)+'\n')
    print(json.dumps(report))


if __name__ == '__main__':
    main()
