"""创建并验证便携压缩包，兼容早于 1980 年的依赖文件时间戳。"""
from pathlib import Path
import sys
import zipfile

source, archive = map(Path, sys.argv[1:])
binary = (source / 'MinecraftExplorer.exe').read_bytes()
assert b'MINECRAFT_EXPLORER_UI_SMOKE' not in binary, 'Build the normal release without ui-smoke before packaging'
assert b'MINECRAFT_EXPLORER_UI_STAGE2' not in binary, 'UI test harness must not be distributed'
assert b'MINECRAFT_EXPLORER_UI_STAGE3' not in binary, 'Stage 3 UI test harness must not be distributed'
assert b'MINECRAFT_EXPLORER_UI_STAGE4' not in binary, 'Stage 4 UI test harness must not be distributed'
with zipfile.ZipFile(archive, 'w', compression=zipfile.ZIP_DEFLATED, compresslevel=9, strict_timestamps=False) as package:
    for file in sorted(source.rglob('*')):
        if file.is_file():
            package.write(file, file.relative_to(source.parent).as_posix())
with zipfile.ZipFile(archive) as package:
    assert package.testzip() is None
    assert source.name + '/MinecraftExplorer.exe' in package.namelist()
    assert source.name + '/THIRD_PARTY_NOTICES.md' in package.namelist()
print('Verified archive:', archive)
