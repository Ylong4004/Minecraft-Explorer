"""Read-only real-game UI verification; application data stays in the workspace.

Uses a COPY of inspect's cache. Does not open PCL2 or start Minecraft.
"""
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import uuid

base = Path(__file__).resolve().parents[1]
run = base / '.test-artifacts' / ('real-ui-' + uuid.uuid4().hex[:8])
cache = run / 'app-data'
cache.mkdir(parents=True)
source_cache = Path(sys.argv[2]) if len(sys.argv)>2 else base / '.test-artifacts/real-pcl2/index.db'
with sqlite3.connect('file:' + source_cache.resolve().as_posix() + '?mode=ro', uri=True) as source:
    with sqlite3.connect(cache / 'index.db') as target:
        source.backup(target)
with sqlite3.connect(cache / 'index.db') as db:
    settings = json.loads(db.execute('SELECT json FROM settings WHERE id=1').fetchone()[0])
    entries = [json.loads(row[0]) for row in db.execute('SELECT json FROM items')]
    world = next(item for item in entries if item['kind'] == 'World' and item['name'] == 'Roguelike Bio')
    version = next(item for item in entries if item['kind'] == 'Version' and item['path'] == world['game_dir'])
    settings['launchers'] = [{'id': 'real-pcl2-readonly-test', 'kind': 'Pcl2', 'executable': 'D:\\PCL2\\Plain Craft Launcher 2.exe', 'working_dir': 'D:\\PCL2', 'root': world['root'], 'java': None, 'checked_ms': 0}]
    settings['default_launcher'] = 'real-pcl2-readonly-test'
    settings['mappings'] = [{'subject_id': world['id'], 'launcher_id': 'real-pcl2-readonly-test', 'root': world['root'], 'game_dir': world['game_dir'], 'version_id': version['version_id'], 'version_path': version['path'], 'source': '只读界面测试：明确指定目标'}]
    db.execute('UPDATE settings SET json=? WHERE id=1', (json.dumps(settings, ensure_ascii=False),))
env = os.environ.copy()
env['MINECRAFT_EXPLORER_DATA'] = str(cache)
env['MINECRAFT_EXPLORER_UI_SMOKE'] = str(run)
env['MINECRAFT_EXPLORER_UI_REAL'] = '1'
exe = sys.argv[1] if len(sys.argv)>1 else str(base / 'target/debug/minecraft-explorer.exe')
with (run / 'stderr.txt').open('w', encoding='utf-8') as error:
    process = subprocess.Popen([exe], env=env, stderr=error, creationflags=0x08000000)
    try:
        assert process.wait(timeout=100) == 0
    except BaseException:
        if process.poll() is None:
            process.terminate(); process.wait(timeout=5)
        raise
result = json.loads((run / 'ui-result.json').read_text(encoding='utf-8'))
assert result['passed'] and not result['game_launched']
print(run)
print(json.dumps(result, ensure_ascii=False))
