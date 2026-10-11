"""Windows smoke test using only a generated fixture and this program's own HWND.

Requires Python, Pillow and psutil. No user Minecraft data is used.
The generated screenshots and metrics stay under .test-artifacts.
"""
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

from PIL import Image
import psutil

BASE = Path(__file__).resolve().parents[1]
RUN = BASE / ".test-artifacts" / ("ui-" + uuid.uuid4().hex[:8])
RUN.mkdir(parents=True)
ROOT = RUN / "我的 Minecraft"
CACHE = RUN / "app-data"
CACHE.mkdir()

def string(value):
    b = value.encode("utf-8")
    return struct.pack(">H", len(b)) + b

def world(path, name, last_played, size=32):
    path.mkdir(parents=True, exist_ok=True)
    data = b"\x0a\0\0\x0a" + string("Data")
    data += b"\x08" + string("LevelName") + string(name)
    data += b"\x04" + string("LastPlayed") + struct.pack(">q", last_played)
    data += b"\x0a" + string("Version") + b"\x08" + string("Name") + string("1.21.11") + b"\0\0\0"
    (path / "level.dat").write_bytes(gzip.compress(data))
    (path / "region").mkdir()
    (path / "region" / "test.mca").write_bytes(b"x" * size)
    Image.new("RGB", (64, 64), (65, 145, 94)).save(path / "icon.png")

for version, target in [("任意 Fabric 名称", "fabric-custom"), ("同版本 NeoForge", "neoforge-custom")]:
    folder = ROOT / "versions" / version
    folder.mkdir(parents=True)
    (folder / (version + ".json")).write_text(json.dumps({"id": target, "inheritsFrom": "1.21.11"}), encoding="utf-8")
    (folder / (version + ".jar")).write_bytes(b"fixture")
    world(folder / "saves" / "冒险地图", "冒险地图", 1700000000123, 3200 if "Fabric" in version else 6400)
world(ROOT / "saves" / "共享", "共享世界", 1700000000123)
broken = ROOT / "saves" / "损坏存档"
broken.mkdir(); (broken / "level.dat").write_bytes(b"bad nbt")
for n in range(1000):
    world(ROOT / "saves" / f"world-{n:04}", f"压力世界-{n:04}", 1700000000000+n, n)

settings = {"sources": [{"path": str(ROOT), "kind": "Root", "associated_root": None}], "launchers": [], "mappings": [], "default_launcher": None, "instance_defaults": {}}
with sqlite3.connect(CACHE / "index.db") as db:
    db.execute("CREATE TABLE settings (id INTEGER PRIMARY KEY CHECK(id=1), json TEXT NOT NULL)")
    db.execute("INSERT INTO settings VALUES(1,?)", (json.dumps(settings, ensure_ascii=False),))

user32 = ctypes.WinDLL("user32", use_last_error=True)
gdi32 = ctypes.WinDLL("gdi32", use_last_error=True)
user32.SetProcessDPIAware()
user32.GetWindowThreadProcessId.argtypes = [w.HWND, ctypes.POINTER(w.DWORD)]
user32.GetClientRect.argtypes = [w.HWND, ctypes.POINTER(w.RECT)]
user32.IsWindowVisible.argtypes = [w.HWND]
user32.GetWindowTextW.argtypes = [w.HWND, w.LPWSTR, ctypes.c_int]
user32.GetDpiForWindow.argtypes = [w.HWND]
user32.SetForegroundWindow.argtypes = [w.HWND]
user32.GetForegroundWindow.restype = w.HWND
user32.BringWindowToTop.argtypes = [w.HWND]
user32.AttachThreadInput.argtypes = [w.DWORD, w.DWORD, w.BOOL]
user32.ClientToScreen.argtypes = [w.HWND, ctypes.POINTER(w.POINT)]
user32.SendMessageTimeoutW.argtypes = [w.HWND, w.UINT, w.WPARAM, w.LPARAM, w.UINT, w.UINT, ctypes.POINTER(ctypes.c_size_t)]
user32.PostMessageW.argtypes = [w.HWND, w.UINT, w.WPARAM, w.LPARAM]
user32.GetDC.argtypes = [w.HWND]; user32.GetDC.restype = w.HDC
gdi32.CreateCompatibleDC.argtypes = [w.HDC]; gdi32.CreateCompatibleDC.restype = w.HDC
gdi32.CreateCompatibleBitmap.argtypes = [w.HDC, ctypes.c_int, ctypes.c_int]; gdi32.CreateCompatibleBitmap.restype = w.HBITMAP
gdi32.SelectObject.argtypes = [w.HDC, w.HGDIOBJ]; gdi32.SelectObject.restype = w.HGDIOBJ
gdi32.DeleteObject.argtypes = [w.HGDIOBJ]
gdi32.DeleteDC.argtypes = [w.HDC]
user32.PrintWindow.argtypes = [w.HWND, w.HDC, w.UINT]
user32.ReleaseDC.argtypes = [w.HWND, w.HDC]
CALLBACK = ctypes.WINFUNCTYPE(w.BOOL, w.HWND, w.LPARAM)

def find_window(pid):
    result = []
    @CALLBACK
    def visit(hwnd, unused):
        found = w.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(found))
        title = ctypes.create_unicode_buffer(256)
        user32.GetWindowTextW(hwnd, title, 256)
        if found.value == pid and user32.IsWindowVisible(hwnd) and title.value == "Minecraft Explorer":
            result.append(hwnd)
        return True
    user32.EnumWindows(visit, 0)
    return result[0] if result else None

def click(hwnd, x, y):
    user32.SetForegroundWindow(hwnd)
    if user32.GetForegroundWindow() != hwnd:
        kernel32 = ctypes.WinDLL("kernel32")
        foreground = user32.GetForegroundWindow()
        foreground_thread = user32.GetWindowThreadProcessId(foreground, None)
        current_thread = kernel32.GetCurrentThreadId()
        attached = user32.AttachThreadInput(current_thread, foreground_thread, True)
        try:
            user32.BringWindowToTop(hwnd)
            user32.SetForegroundWindow(hwnd)
        finally:
            if attached: user32.AttachThreadInput(current_thread, foreground_thread, False)
    time.sleep(.08)
    if user32.GetForegroundWindow() != hwnd:
        raise RuntimeError("Cannot focus the test window; stopping to avoid affecting another app")
    dpi = user32.GetDpiForWindow(hwnd) / 96
    position = w.POINT(round(x*dpi), round(y*dpi))
    user32.ClientToScreen(hwnd, ctypes.byref(position))
    user32.SetCursorPos(position.x, position.y)
    # Let the native event loop consume WM_MOUSEMOVE before button events.
    time.sleep(.18)
    actual = w.POINT()
    user32.GetCursorPos(ctypes.byref(actual))
    print('click', x, y, 'dpi', dpi, 'screen', position.x, position.y, 'actual', actual.x, actual.y, flush=True)
    user32.mouse_event(2, 0, 0, 0, 0)
    user32.mouse_event(4, 0, 0, 0, 0)

def type_text(hwnd, value):
    if user32.GetForegroundWindow() != hwnd:
        raise RuntimeError("Test window lost focus")
    class Keyboard(ctypes.Structure):
        _fields_ = [("vk",w.WORD),("scan",w.WORD),("flags",w.DWORD),("time",w.DWORD),("extra",ctypes.c_size_t)]
    class Mouse(ctypes.Structure):
        _fields_ = [("x",w.LONG),("y",w.LONG),("data",w.DWORD),("flags",w.DWORD),("time",w.DWORD),("extra",ctypes.c_size_t)]
    class Union(ctypes.Union):
        _fields_ = [("keyboard",Keyboard),("mouse",Mouse)]
    class Input(ctypes.Structure):
        _fields_ = [("type",w.DWORD),("data",Union)]
    for char in value:
        inputs = (Input*2)(Input(1,Union(keyboard=Keyboard(0,ord(char),4,0,0))),Input(1,Union(keyboard=Keyboard(0,ord(char),6,0,0))))
        assert user32.SendInput(2,ctypes.byref(inputs),ctypes.sizeof(Input)) == 2

def screenshot(hwnd, name):
    rect = w.RECT()
    if not user32.GetClientRect(hwnd, ctypes.byref(rect)):
        raise RuntimeError("App window disappeared before screenshot")
    width, height = rect.right, rect.bottom
    assert width > 0 and height > 0, (width, height)
    dc = user32.GetDC(hwnd); mem = gdi32.CreateCompatibleDC(dc)
    bitmap = gdi32.CreateCompatibleBitmap(dc, width, height)
    old = gdi32.SelectObject(mem, bitmap)
    class Header(ctypes.Structure):
        _fields_ = [("size", w.DWORD),("width",w.LONG),("height",w.LONG),("planes",w.WORD),("bits",w.WORD),("compression",w.DWORD),("image_size",w.DWORD),("x",w.LONG),("y",w.LONG),("used",w.DWORD),("important",w.DWORD)]
    header = Header(40, width, -height, 1, 32, 0, width*height*4, 0, 0, 0, 0)
    buffer = ctypes.create_string_buffer(width*height*4)
    user32.PrintWindow(hwnd, mem, 3)  # PW_CLIENTONLY | PW_RENDERFULLCONTENT
    gdi32.GetDIBits.argtypes = [w.HDC, w.HBITMAP, w.UINT, w.UINT, ctypes.c_void_p, ctypes.c_void_p, w.UINT]
    gdi32.GetDIBits(mem, bitmap, 0, height, buffer, ctypes.byref(header), 0)
    Image.frombuffer("RGB", (width,height), buffer.raw, "raw", "BGRX", 0, 1).save(RUN / name)
    gdi32.SelectObject(mem,old); gdi32.DeleteObject(bitmap); gdi32.DeleteDC(mem); user32.ReleaseDC(hwnd,dc)

exe = Path(sys.argv[1]) if len(sys.argv)>1 else BASE / "target/debug/minecraft-explorer.exe"
env = os.environ.copy(); env["MINECRAFT_EXPLORER_DATA"] = str(CACHE)
env["MINECRAFT_EXPLORER_UI_SMOKE"] = str(RUN)
stderr = (RUN / "stderr.txt").open("w", encoding="utf-8")
process = subprocess.Popen([str(exe)], env=env, stderr=stderr, stdout=subprocess.DEVNULL, creationflags=0x08000000)
metrics = {"fixture_worlds": 1004, "exe": str(exe)}
try:
    deadline = time.monotonic()+30
    hwnd = None
    while time.monotonic()<deadline and not hwnd:
        if process.poll() is not None: raise RuntimeError("App exited before creating a window")
        hwnd = find_window(process.pid); time.sleep(.1)
    if not hwnd: raise RuntimeError("No application window")
    samples = []
    ps = psutil.Process(process.pid)
    cpu_start = ps.cpu_times(); cpu_last = cpu_start; cpu_clock = time.monotonic()
    for n in range(300):
        if process.poll() is not None: break
        start = time.perf_counter(); result = ctypes.c_size_t()
        if not user32.SendMessageTimeoutW(hwnd, 0, 0, 0, 2, 1000, ctypes.byref(result)):
            if (RUN / "ui-result.json").exists(): break
            raise RuntimeError("Window stopped responding")
        samples.append((time.perf_counter()-start)*1000)
        metrics["peak_rss_bytes"] = max(metrics.get("peak_rss_bytes",0),ps.memory_info().rss)
        cpu_last = ps.cpu_times()
        time.sleep(.3)
    metrics["cpu_percent_sample"] = 100 * (cpu_last.user + cpu_last.system - cpu_start.user - cpu_start.system) / (time.monotonic()-cpu_clock)
    metrics["wm_null_max_ms"] = max(samples)
    assert process.wait(timeout=5) == 0, (RUN / "stderr.txt").read_text(encoding="utf-8")
    result = json.loads((RUN / "ui-result.json").read_text(encoding="utf-8"))
    assert result["passed"]
    metrics["ui_checks"] = result
    metrics["responsive"] = True
    with sqlite3.connect(CACHE / "index.db") as db:
        metrics["indexed_worlds"] = db.execute("SELECT COUNT(*) FROM items WHERE kind='world'").fetchone()[0]
    assert metrics["indexed_worlds"] == 1004
finally:
    if process.poll() is None:
        user32.PostMessageW(hwnd,0x0010,0,0)
        try: process.wait(timeout=5)
        except subprocess.TimeoutExpired: process.terminate(); process.wait(timeout=5)
    stderr.close()
    (RUN / "metrics.json").write_text(json.dumps(metrics,indent=2,ensure_ascii=False),encoding="utf-8")
    print(str(RUN)); print(json.dumps(metrics,ensure_ascii=False))
