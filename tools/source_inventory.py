#!/usr/bin/env python3
"""Index the audit inputs. File/API presence is not behavior acceptance."""
import argparse
import csv
import io
import json
from pathlib import Path
import re
import subprocess

TS_AST = r"""
const ts = require(process.argv[1]);
const fs = require('node:fs');
const files = JSON.parse(fs.readFileSync(0, 'utf8'));
const rows = [];
for (const [path, full] of files) {
  const source = ts.createSourceFile(full, fs.readFileSync(full, 'utf8'), ts.ScriptTarget.Latest, true);
  function visit(node) {
    if (ts.isFunctionLike(node)) {
      const line = source.getLineAndCharacterOfPosition(node.getStart(source)).line + 1;
      const own = node.name?.getText(source) ?? node.parent?.name?.getText(source) ?? '<anonymous>';
      const parents = [];
      for (let parent = node.parent; parent; parent = parent.parent) {
        if ((ts.isClassLike(parent) || ts.isModuleDeclaration(parent)) && parent.name) parents.unshift(parent.name.getText(source));
      }
      rows.push([path, node.body ? 'function' : 'signature', line, [...parents, own].join('.')]);
    }
    ts.forEachChild(node, visit);
  }
  visit(source);
}
process.stdout.write(JSON.stringify(rows));
"""

RUST_DECL = re.compile(
    r'^[ \t]*(?:pub(?:\([^)]*\))?\s+)?'
    r'(?:(?:async|unsafe|const|extern(?:\s+"[^"]*"))\s+)*'
    r'(fn|struct|enum|trait|type|const|static|macro_rules!)\s+(\w+)', re.MULTILINE
)


def files(root, paths):
    result = subprocess.run(
        ['git', '-C', str(root), 'ls-files', '-z', '--cached', '--others',
         '--exclude-standard', '--', *paths], check=True, stdout=subprocess.PIPE
    )
    return sorted(set(name for name in result.stdout.decode().split('\0')
                      if name and (root / name).is_file() and not (root / name).is_symlink()))


def write_csv(path, header, rows):
    output = io.StringIO(newline='')
    writer = csv.writer(output, lineterminator='\n')
    writer.writerow(header)
    writer.writerows(rows)
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + '.tmp')
    temporary.write_text(output.getvalue(), encoding='utf-8')
    temporary.replace(path)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--typescript', required=True, type=Path)
    parser.add_argument('--rust', type=Path, default=Path('.'))
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    ts_root, rust_root = args.typescript.resolve(), args.rust.resolve()
    ts_files = files(ts_root, ['src', 'tests', 'tools', 'verification', 'docs', 'package.json'])
    rust_files = files(rust_root, ['crates', 'tools', 'data', 'docs', 'Cargo.toml'])
    rows = []
    for repository, names in [('typescript', ts_files), ('rust', rust_files)]:
        for name in names:
            role = ('test' if '/tests/' in '/' + name or name.startswith('tests/')
                    else 'runtime' if name.startswith(('src/', 'crates/'))
                    else 'tool' if name.startswith('tools/') else 'reference')
            rows.append([repository, role, name, 'file', 1, '', 'unreviewed'])
    inputs = [[name, str(ts_root / name)] for name in ts_files if Path(name).suffix in ('.ts', '.tsx', '.js', '.jsx')]
    parsed = subprocess.run(
        ['node', '-e', TS_AST, str(ts_root / 'node_modules/typescript/lib/typescript.js')],
        input=json.dumps(inputs), text=True, check=True, stdout=subprocess.PIPE
    )
    for name, kind, line, symbol in json.loads(parsed.stdout):
        role = 'runtime' if name.startswith('src/') else 'test' if name.startswith('tests/') else 'tool'
        rows.append(['typescript', role, name, kind, line, symbol, 'unreviewed'])
    for name in rust_files:
        if not name.endswith('.rs'):
            continue
        source = (rust_root / name).read_text(encoding='utf-8')
        role = 'test' if '/tests/' in name else 'runtime'
        for match in RUST_DECL.finditer(source):
            rows.append(['rust', role, name, 'declaration-candidate', source.count('\n', 0, match.start()) + 1,
                         match.group(2), 'unreviewed'])
    rows.sort(key=lambda row: (row[0], row[2], row[4], row[3], row[5]))
    write_csv(args.output / 'sources.csv', ['repository', 'role', 'path', 'kind', 'line', 'symbol', 'review'], rows)
    requirements = []
    for family in ('q1', 'q2', 'q3'):
        shard = json.loads((ts_root / 'verification/features' / f'{family}.json').read_text())
        for feature in shard['features']:
            requirements.append([feature['id'], family, feature['title'], feature['requirement'],
                                 feature['sourceStatus'], ';'.join(feature['products']), 'unreviewed', ''])
    requirements.sort()
    write_csv(args.output / 'requirements.csv',
              ['id', 'family', 'title', 'requirement', 'reference_status', 'products', 'rust_review', 'evidence'], requirements)
    print(json.dumps({'typescript_files': len(ts_files), 'rust_files': len(rust_files),
                      'source_rows': len(rows), 'reference_requirements': len(requirements)}, sort_keys=True))


if __name__ == '__main__':
    main()
