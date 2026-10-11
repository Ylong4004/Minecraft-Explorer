"""Preserve upstream license/notice files from the locked Windows Cargo graph."""
import json
from pathlib import Path
import re
import shutil
import sys

metadata = Path(sys.argv[1])
destination = Path(sys.argv[2])
raw = metadata.read_bytes()
graph = json.loads(raw.decode('utf-16' if raw.startswith((b'\xff\xfe', b'\xfe\xff')) else 'utf-8-sig'))
packages = sorted(graph['packages'], key=lambda p: (p['name'], p['version']))
nodes = {node['id']: node for node in graph['resolve']['nodes']}
active = set()
pending = [graph['resolve']['root']]
while pending:
    package_id = pending.pop()
    if package_id in active:
        continue
    active.add(package_id)
    pending.extend(dependency['pkg'] for dependency in nodes[package_id]['deps'])
lines = ['# Third-party packages', '', 'Locked Windows graph; includes build and test dependencies for attribution.',
         'Slint license choice: LicenseRef-Slint-Royalty-free-2.0. Other alternatives retain their upstream expressions.', '',
         '| Package | Version | License expression | Preserved texts |', '| --- | --- | --- | --- |']
missing = []
for package in packages:
    if package['id'] not in active or package['name'] == 'minecraft-explorer':
        continue
    root = Path(package['manifest_path']).parent
    relative = Path('licenses') / (package['name'] + '-' + package['version'])
    target = destination / relative
    texts = []
    for file in root.rglob('*'):
        if not file.is_file():
            continue
        lower = file.name.lower()
        if lower.startswith(('license', 'licence', 'copying', 'notice', 'copyright')) or lower.endswith('.license'):
            output = target / file.relative_to(root)
            output.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(file, output)
            texts.append(output.relative_to(destination).as_posix())
    if package.get('license_file'):
        file = root / package['license_file']
        if file.is_file():
            output = target / file.relative_to(root)
            output.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(file, output)
            if output.relative_to(destination).as_posix() not in texts:
                texts.append(output.relative_to(destination).as_posix())
    if package['name'] == 'libsqlite3-sys':
        sqlite = root / 'sqlite3' / 'sqlite3.c'
        if sqlite.exists():
            header = sqlite.read_text(encoding='utf-8').split('*/', 1)[0] + '*/\n'
            target.mkdir(parents=True, exist_ok=True)
            (target / 'SQLITE-PUBLIC-DOMAIN.txt').write_text(header, encoding='utf-8')
            texts.append((relative / 'SQLITE-PUBLIC-DOMAIN.txt').as_posix())
    copyrights = set()
    for source in root.rglob('*.rs'):
        for line in source.read_text(encoding='utf-8', errors='replace')[:2048].splitlines():
            if re.search(r'copyright|Copyright', line):
                copyrights.add(line.strip())
    if copyrights:
        target.mkdir(parents=True, exist_ok=True)
        (target / 'COPYRIGHT-NOTICES.txt').write_text('\n'.join(sorted(copyrights)) + '\n', encoding='utf-8')
    if not texts:
        # Some published crates omit workspace-owned license files. Preserve their
        # exact package declaration and require reviewed upstream texts to fill it.
        fallback = Path(__file__).resolve().parent.parent / 'licenses' / 'upstream' / package['name']
        if fallback.is_dir():
            for file in fallback.iterdir():
                if file.is_file():
                    target.mkdir(parents=True, exist_ok=True)
                    output = target / file.name
                    shutil.copy2(file, output)
                    texts.append(output.relative_to(destination).as_posix())
        else:
            missing.append(package['name'] + ':' + str(package['license']))
    # Keep the index compact; all recursive texts are present in the package directory.
    links = ', '.join('[' + Path(t).name + '](' + t + ')' for t in texts[:4])
    if len(texts) > 4:
        links += ', [additional texts](' + relative.as_posix() + ')'
    lines.append('| ' + ' | '.join([package['name'], package['version'], package['license'] or 'See upstream file', links]) + ' |')
(destination / 'THIRD_PARTY_NOTICES.md').write_text('\n'.join(lines) + '\n', encoding='utf-8')
if missing:
    raise SystemExit('Missing upstream license texts: ' + ', '.join(missing))
print('Preserved licenses for', len(lines)-7, 'packages')
