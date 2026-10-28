#!/usr/bin/env python3
"""Collect the license texts of every crate Modo ships into THIRD_PARTY_NOTICES.md.

Run from the repository root: python3 scripts/third-party-notices.py [--target TRIPLE]
"""
import argparse
import json
import pathlib
import subprocess

parser = argparse.ArgumentParser()
parser.add_argument('--target')
args = parser.parse_args()
command = ['cargo', 'metadata', '--locked', '--format-version', '1']
if args.target:
    command.extend(['--filter-platform', args.target])
metadata = json.loads(subprocess.check_output(command))
packages = {p['id']: p for p in metadata['packages']}
nodes = {n['id']: n for n in metadata['resolve']['nodes']}
root = metadata['resolve']['root']
visited, pending = set(), [root]
while pending:
    current = pending.pop()
    if current in visited:
        continue
    visited.add(current)
    pending.extend(d['pkg'] for d in nodes[current]['deps'] if any(k['kind'] != 'dev' for k in d['dep_kinds']))

out = ['# Third-party notices', '', 'Generated from Cargo.lock for this package. Dependency licenses retain their own terms.', '']
rust_notices = pathlib.Path('assets/package/rust-stdlib-notices.txt')
out.extend(['## Rust standard library', '', '````', rust_notices.read_text(encoding='utf-8'), '````', ''])
for identity in sorted(visited - {root}, key=lambda i: (packages[i]['name'], packages[i]['version'])):
    package = packages[identity]
    out.extend([f"## {package['name']} {package['version']}", '', f"License: {package.get('license') or 'See license file'}", f"Source: {package.get('repository') or package.get('source') or 'crates.io'}", ''])
    if package.get('authors'):
        out.extend([f"Authors: {', '.join(package['authors'])}", ''])
    directory = pathlib.Path(package['manifest_path']).parent
    files = sorted(p for p in directory.rglob('*') if p.is_file() and (
        p.name.lower().startswith(('license', 'licence', 'copying', 'copyright', 'notice'))
        or any(part.lower() in ('licenses', 'licences') for part in p.relative_to(directory).parts[:-1])
    ))
    for file in files:
        content = file.read_text(encoding='utf-8', errors='replace')
        out.extend([f"### {file.relative_to(directory).as_posix()}", '', '````', content, '````', ''])
    if not files:
        out.extend(['License text is provided by the upstream source under the SPDX expression above.', ''])
pathlib.Path('THIRD_PARTY_NOTICES.md').write_text('\n'.join(out), encoding='utf-8')
print(f'Collected notices for {len(visited) - 1} dependencies')
