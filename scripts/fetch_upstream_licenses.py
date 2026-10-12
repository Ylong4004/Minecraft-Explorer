"""仅从已发布依赖对应的版本控制提交补齐缺失许可文件。

更新 Cargo.lock 时显式运行；打包脚本使用本地保存的原文。
"""
import concurrent.futures
import json
from pathlib import Path
import urllib.request
import sys

base = Path(__file__).resolve().parents[1]
graph = json.loads(Path(sys.argv[1]).read_text(encoding='utf-8-sig'))
missing = {'clipboard-win', 'const-field-offset', 'const-field-offset-macro', 'fastnbt',
           'gl_generator', 'khronos_api', 'lyon_algorithms', 'lyon_extra', 'lyon_geom',
           'lyon_path', 'profiling', 'rspolib', 'skia-bindings', 'skia-safe', 'spirv',
           'taffy', 'vtable', 'vtable-macro'}

def fetch(package):
    root = Path(package['manifest_path']).parent
    vcs = json.loads((root / '.cargo_vcs_info.json').read_text())
    commit = vcs['git']['sha1']
    repo = package['repository'].rstrip('/').removesuffix('.git')
    api = repo.replace('https://github.com/', 'https://api.github.com/repos/') + '/git/trees/' + commit + '?recursive=1'
    request = urllib.request.Request(api, headers={'User-Agent': 'MinecraftExplorer-license-packaging'})
    tree = json.load(urllib.request.urlopen(request, timeout=30))
    prefix = vcs.get('path_in_vcs', '')
    candidates = []
    for entry in tree['tree']:
        path = entry['path']
        filename = Path(path).name.lower()
        if entry['type'] != 'blob' or not (filename.startswith(('license', 'copying', 'notice')) or path.startswith('LICENSES/')):
            continue
        if '/' not in path or path.startswith('LICENSES/') or (prefix and path.startswith(prefix + '/')):
            candidates.append(path)
    target = base / 'licenses/upstream' / package['name']
    target.mkdir(parents=True, exist_ok=True)
    urls = []
    for path in candidates:
        url = repo.replace('https://github.com/', 'https://raw.githubusercontent.com/') + '/' + commit + '/' + path
        data = urllib.request.urlopen(url, timeout=30).read()
        output = target / path.replace('/', '__')
        output.write_bytes(data)
        urls.append(url)
    if not urls:
        raise RuntimeError('No upstream license text found for ' + package['name'])
    (target / 'UPSTREAM.txt').write_text('Package: ' + package['name'] + ' ' + package['version'] + '\nDeclared license: ' + package['license'] + '\nSources:\n' + '\n'.join(urls) + '\n', encoding='utf-8')
    return package['name'], len(urls)

with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
    for result in pool.map(fetch, [p for p in graph['packages'] if p['name'] in missing]):
        print(*result, flush=True)
