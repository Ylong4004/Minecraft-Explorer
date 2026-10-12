"""阶段 3 原生窗口验收，仅使用带标记的生成目录。"""
import ctypes
from ctypes import wintypes as w
import gzip
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
RUN = BASE / ".test-artifacts" / ("ui-stage3-" + uuid.uuid4().hex[:8])
RUN.mkdir(parents=True)
A, B, CACHE, BACKUP = (RUN / name for name in ["Minecraft 甲", "Minecraft 乙", "app-data", "备份"])
for root in [A, B]:
    for category in ["saves", "mods", "resourcepacks", "shaderpacks", "versions"]:
        (root / category).mkdir(parents=True)
CACHE.mkdir()
BACKUP.mkdir()
(RUN / "stage3-fixture.marker").write_bytes(b"minecraft-explorer-generated-stage3-fixture")

def string(value):
    encoded = value.encode("utf-8")
    return struct.pack(">H", len(encoded)) + encoded

world = A / "saves" / "原世界"
world.mkdir()
nbt = b"\x0a\0\0\x0a" + string("Data")
nbt += b"\x08" + string("LevelName") + string("Stage3 验收世界")
nbt += b"\x04" + string("LastPlayed") + struct.pack(">q", 1700000000123) + b"\0\0"
(world / "level.dat").write_bytes(gzip.compress(nbt, mtime=0))
(world / "session.lock").write_bytes(b"closed fixture lock")
(world / "region").mkdir()
(world / "region" / "fixture.mca").write_bytes(b"fixture region" * 5000)
Image.new("RGBA", (64, 64), (65, 145, 94, 255)).save(world / "icon.png")
settings = {"sources": [{"path": str(root), "kind": "Root", "associated_root": None} for root in [A, B]], "launchers": [], "mappings": [], "default_launcher": None, "instance_defaults": {}, "watch_enabled": False}
with sqlite3.connect(CACHE / "index.db") as db:
    db.execute("CREATE TABLE settings(id INTEGER PRIMARY KEY CHECK(id=1),json TEXT NOT NULL)")
    db.execute("INSERT INTO settings VALUES(1,?)", (json.dumps(settings, ensure_ascii=False),))

user32 = ctypes.WinDLL("user32", use_last_error=True)
user32.GetWindowThreadProcessId.argtypes = [w.HWND, ctypes.POINTER(w.DWORD)]
user32.IsWindowVisible.argtypes = [w.HWND]
user32.SendMessageTimeoutW.argtypes = [w.HWND, w.UINT, w.WPARAM, w.LPARAM, w.UINT, w.UINT, ctypes.POINTER(ctypes.c_size_t)]
CALLBACK = ctypes.WINFUNCTYPE(w.BOOL, w.HWND, w.LPARAM)
def window_for(pid):
    handles = []
    @CALLBACK
    def visit(hwnd, unused):
        process = w.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(process))
        if process.value == pid and user32.IsWindowVisible(hwnd):
            handles.append(hwnd)
        return True
    user32.EnumWindows(visit, 0)
    return handles[0] if handles else None

env = dict(os.environ, MINECRAFT_EXPLORER_DATA=str(CACHE), MINECRAFT_EXPLORER_UI_STAGE3=str(RUN))
started = time.monotonic()
with (RUN / "stdout.txt").open("w") as stdout, (RUN / "stderr.txt").open("w") as stderr:
    process = subprocess.Popen([str(Path(sys.argv[1]).resolve())], cwd=BASE, env=env, stdout=stdout, stderr=stderr)
    peak = 0
    responses = []
    try:
        tracked = psutil.Process(process.pid)
        while process.poll() is None:
            if time.monotonic() - started > 140:
                raise RuntimeError("stage 3 native UI timed out")
            try:
                peak = max(peak, tracked.memory_info().rss)
                hwnd = window_for(process.pid)
                if hwnd:
                    result = ctypes.c_size_t()
                    before = time.monotonic()
                    if not user32.SendMessageTimeoutW(hwnd, 0, 0, 0, 2, 2000, ctypes.byref(result)):
                        raise RuntimeError("native window stopped responding")
                    responses.append((time.monotonic() - before) * 1000)
            except psutil.NoSuchProcess:
                break
            time.sleep(.1)
    finally:
        if process.poll() is None:
            process.terminate()
            process.wait(timeout=10)
result = json.loads((RUN / "ui-result.json").read_text(encoding="utf-8")) if (RUN / "ui-result.json").exists() else {"passed": False, "error": "no app result"}
result.update({"exit_code": process.returncode, "wall_seconds": time.monotonic() - started, "peak_rss_mib": peak / 1024**2, "wm_null_max_ms": max(responses, default=0)})
if result["passed"]:
    stages = [path for root in [A, B] for path in root.rglob(".minecraft-explorer-*.partial")]
    assert not stages, "closing the app left an uncommitted stage behind"
    assert (A / "saves" / "新世界" / "level.dat").read_bytes() == gzip.compress(nbt, mtime=0)
    assert (B / "saves" / "新世界" / "level.dat").read_bytes() == gzip.compress(nbt, mtime=0)
    result["close_cleanup_verified"] = True
(RUN / "metrics.json").write_text(json.dumps(result, ensure_ascii=False, indent=2), encoding="utf-8")
print(RUN)
print(json.dumps(result, ensure_ascii=False, indent=2))
if not result["passed"] or process.returncode != 0:
    raise SystemExit(1)
