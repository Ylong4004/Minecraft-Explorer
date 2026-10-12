"""将已核查发行包中的项目许可、依赖原文及归属压缩为可嵌入 EXE 的确定性资源。"""
from pathlib import Path
import gzip
import sys

base = Path(__file__).resolve().parents[1]
source = Path(sys.argv[1]).resolve()
destination = base / 'assets/third-party-licenses.txt.gz'
notices = source / 'THIRD_PARTY_NOTICES.md'
assert notices.is_file(), 'Run collect_licenses.py before embedding license texts.'
parts = [b'Minecraft Explorer - license texts and third-party attribution\n\n',
         b'===== LICENSE (Minecraft Explorer) =====\n', (base / 'LICENSE').read_bytes(),
         b'\n===== THIRD_PARTY_NOTICES.md =====\n', notices.read_bytes()]
files = sorted((source / 'licenses').rglob('*'))
count = 0
for file in files:
    if not file.is_file():
        continue
    path = file.relative_to(source).as_posix()
    parts.extend([f'\n\n===== {path} =====\n'.encode('utf-8'), file.read_bytes()])
    count += 1
assert count > 0, 'No upstream license texts found.'
payload = b''.join(parts)
assert len(payload) <= 16 * 1024 * 1024
destination.write_bytes(gzip.compress(payload, compresslevel=9, mtime=0))
print(f'Embedded {count} license/attribution files: {len(payload)} bytes, {destination.stat().st_size} compressed bytes.')
