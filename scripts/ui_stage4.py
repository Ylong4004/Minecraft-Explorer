"""阶段 4 原生界面、双语主题与重启验收，只使用生成数据和本程序窗口。"""
import ctypes
from ctypes import wintypes as w
import gzip
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import struct
import subprocess
import sys
import time
import uuid
import psutil
from PIL import Image

BASE = Path(__file__).resolve().parents[1]
RUN = BASE / ".test-artifacts" / ("ui-stage4-" + uuid.uuid4().hex[:8])
ROOT, CACHE = RUN / "Minecraft 根目录 & 空格", RUN / "app-data"
ROOT.mkdir(parents=True)
CACHE.mkdir()
(RUN / "stage4-fixture.marker").write_bytes(b"minecraft-explorer-generated-stage4-fixture")
def string(value):
    data = value.encode("utf-8")
    return struct.pack(">H",len(data))+data
for i in range(1000):
    world=ROOT/"saves"/f"世界 {i:04}"
    world.mkdir(parents=True)
    nbt=b"\x0a\0\0\x0a"+string("Data")+b"\x08"+string("LevelName")+string(f"地图 {i:04}")+b"\0\0"
    (world/"level.dat").write_bytes(gzip.compress(nbt,mtime=0))
    if i<10: Image.new("RGBA",(64,64),(65,145,94,255)).save(world/"icon.png")
for name,size in [("mods/example.jar",3072),("resourcepacks/pack.zip",4096),("shaderpacks/shader.zip",1024),("screenshots/photo.bin",2048)]:
    path=ROOT/name;path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(bytes(size))
custom=ROOT/"versions"/"独立实例"
(custom/"saves"/"独立世界").mkdir(parents=True)
(custom/"独立实例.json").write_text('{"id":"isolated-fixture"}',encoding="utf-8")
(custom/"saves"/"独立世界"/"level.dat").write_bytes(gzip.compress(nbt,mtime=0))
original={str(p.relative_to(ROOT)):hashlib.sha256(p.read_bytes()).hexdigest() for p in ROOT.rglob("*") if p.is_file()}
total=sum(p.stat().st_size for p in ROOT.rglob("*") if p.is_file())
extra_root=RUN/"自动分析新增目录";extra_root.mkdir();(extra_root/"payload.bin").write_bytes(bytes(73))
cancel_root=RUN/"取消自动分析目录";cancel_root.mkdir();(cancel_root/"payload.bin").write_bytes(bytes(41))
auto_only="--auto-only" in sys.argv[2:]
(RUN/"fixture.json").write_text(json.dumps({"root":str(ROOT),"cache":str(CACHE),"bytes":total,"extra_root":str(extra_root),"cancel_root":str(cancel_root),"auto_only":auto_only},ensure_ascii=False),encoding="utf-8")
settings={"sources":[{"path":str(ROOT),"kind":"Root","associated_root":None},{"path":str(custom),"kind":"Custom","associated_root":str(ROOT)}],"launchers":[],"mappings":[],"default_launcher":None,"instance_defaults":{},"watch_enabled":False}
if auto_only:settings["sources"]=[]
with sqlite3.connect(CACHE/"index.db") as db:
    db.execute("CREATE TABLE settings(id INTEGER PRIMARY KEY CHECK(id=1),json TEXT NOT NULL)")
    db.execute("INSERT INTO settings VALUES(1,?)",(json.dumps(settings,ensure_ascii=False),))

user32=ctypes.WinDLL("user32",use_last_error=True)
user32.GetWindowThreadProcessId.argtypes=[w.HWND,ctypes.POINTER(w.DWORD)]
user32.IsWindowVisible.argtypes=[w.HWND]
user32.SendMessageTimeoutW.argtypes=[w.HWND,w.UINT,w.WPARAM,w.LPARAM,w.UINT,w.UINT,ctypes.POINTER(ctypes.c_size_t)]
CALLBACK=ctypes.WINFUNCTYPE(w.BOOL,w.HWND,w.LPARAM)
def window_for(pid):
    handles=[]
    @CALLBACK
    def visit(hwnd,unused):
        process=w.DWORD();user32.GetWindowThreadProcessId(hwnd,ctypes.byref(process))
        if process.value==pid and user32.IsWindowVisible(hwnd):handles.append(hwnd)
        return True
    user32.EnumWindows(visit,0)
    return handles[0] if handles else None

def run(verify):
    env=dict(os.environ,MINECRAFT_EXPLORER_DATA=str(CACHE),MINECRAFT_EXPLORER_UI_STAGE4=str(RUN))
    if verify:env["MINECRAFT_EXPLORER_UI_STAGE4_VERIFY"]="1"
    else:env.pop("MINECRAFT_EXPLORER_UI_STAGE4_VERIFY",None)
    started=time.monotonic();peak=0;responses=[]
    with (RUN/("restart-stderr.txt" if verify else "stderr.txt")).open("w") as stderr:
        process=subprocess.Popen([str(Path(sys.argv[1]).resolve())],cwd=BASE,env=env,stdout=subprocess.DEVNULL,stderr=stderr)
        tracked=psutil.Process(process.pid)
        try:
            while process.poll() is None:
                if time.monotonic()-started>130:raise RuntimeError("阶段 4 原生窗口超时")
                try:
                    peak=max(peak,tracked.memory_info().rss);hwnd=window_for(process.pid)
                    if hwnd:
                        result=ctypes.c_size_t();before=time.monotonic()
                        if not user32.SendMessageTimeoutW(hwnd,0,0,0,2,2000,ctypes.byref(result)):raise RuntimeError("原生窗口无响应")
                        responses.append((time.monotonic()-before)*1000)
                except psutil.NoSuchProcess:break
                time.sleep(.1)
        finally:
            if process.poll() is None:process.terminate();process.wait(timeout=10)
    report=RUN/("restart-result.json" if verify else "ui-result.json")
    result=json.loads(report.read_text(encoding="utf-8")) if report.exists() else {"passed":False,"error":"缺少程序验收结果"}
    result.update({"exit_code":process.returncode,"wall_seconds":time.monotonic()-started,"peak_rss_mib":peak/1024**2,"wm_null_max_ms":max(responses,default=0)})
    return result
exercise=run(False)
if exercise["passed"]:
    for name,digest in original.items():assert hashlib.sha256((ROOT/name).read_bytes()).hexdigest()==digest,name
    if not auto_only:assert (ROOT/"late.bin").read_bytes()==bytes(37)
    assert (extra_root/"payload.bin").read_bytes()==bytes(73)
    exercise["fixture_hashes_verified"]=True
restart=run(True) if exercise["passed"] else None
metrics={"exercise":exercise,"restart":restart,"scale_factor":os.environ.get("SLINT_SCALE_FACTOR","system")}
(RUN/"metrics.json").write_text(json.dumps(metrics,ensure_ascii=False,indent=2),encoding="utf-8")
print(RUN)
print(json.dumps(metrics,ensure_ascii=False,indent=2))
sys.exit(0 if exercise["passed"] and restart and restart["passed"] else 1)
