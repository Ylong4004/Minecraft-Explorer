"""用全新缓存和本程序自己的窗口句柄验证常规发布构建。"""
import ctypes
from ctypes import wintypes as w
import json
import os
from pathlib import Path
import sqlite3
import struct
import subprocess
import sys
import time
import uuid

BASE = Path(__file__).resolve().parents[1]
RUN = BASE / ".test-artifacts" / ("release-" + uuid.uuid4().hex[:8])
RUN.mkdir(parents=True)
exe = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else BASE / "target/release/minecraft-explorer.exe"
binary = exe.read_bytes()
assert (BASE / "assets/third-party-licenses.txt.gz").read_bytes() in binary, "Embedded license texts missing"
kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
kernel32.LoadLibraryExW.argtypes = [w.LPCWSTR, w.HANDLE, w.DWORD]
kernel32.LoadLibraryExW.restype = w.HMODULE
kernel32.FindResourceW.argtypes = [w.HMODULE, ctypes.c_void_p, ctypes.c_void_p]
kernel32.FindResourceW.restype = w.HANDLE
kernel32.LoadResource.argtypes = [w.HMODULE, w.HANDLE]
kernel32.LoadResource.restype = w.HANDLE
kernel32.SizeofResource.argtypes = [w.HMODULE, w.HANDLE]
kernel32.SizeofResource.restype = w.DWORD
kernel32.LockResource.argtypes = [w.HANDLE]
kernel32.LockResource.restype = ctypes.c_void_p
kernel32.FreeLibrary.argtypes = [w.HMODULE]
# 作为数据加载 EXE，不执行入口；核对资源管理器使用的图标组及七个尺寸。
module = kernel32.LoadLibraryExW(str(exe), None, 0x2 | 0x20)
assert module, "Could not read EXE resources"
try:
    resource = kernel32.FindResourceW(module, ctypes.c_void_p(1), ctypes.c_void_p(14))
    assert resource, "EXE icon resource missing"
    data = ctypes.string_at(kernel32.LockResource(kernel32.LoadResource(module, resource)),
                           kernel32.SizeofResource(module, resource))
    reserved, kind, count = struct.unpack_from("<HHH", data)
    assert reserved == 0 and kind == 1 and count == 7, "Invalid EXE icon group"
    sizes = {(data[6 + i * 14] or 256, data[7 + i * 14] or 256) for i in range(count)}
    assert sizes == {(n, n) for n in (16, 24, 32, 48, 64, 128, 256)}, sizes
finally:
    kernel32.FreeLibrary(module)
assert b"MINECRAFT_EXPLORER_UI_STAGE2" not in binary, "UI test harness included in normal build"
assert b"MINECRAFT_EXPLORER_UI_SMOKE" not in binary, "UI test harness included in normal build"
assert b"MINECRAFT_EXPLORER_UI_STAGE3" not in binary, "Stage 3 UI test harness included in normal build"
assert b"MINECRAFT_EXPLORER_UI_STAGE4" not in binary, "Stage 4 UI test harness included in normal build"
environment = os.environ.copy()
environment["MINECRAFT_EXPLORER_DATA"] = str(RUN / "app-data")
for name in ("MINECRAFT_EXPLORER_UI_SMOKE", "MINECRAFT_EXPLORER_UI_STAGE2", "MINECRAFT_EXPLORER_UI_STAGE3", "MINECRAFT_EXPLORER_UI_STAGE4", "MINECRAFT_EXPLORER_UI_STAGE4_VERIFY", "MINECRAFT_EXPLORER_UI_REAL", "MINECRAFT_EXPLORER_UI_STAGE2_VERIFY"):
    environment.pop(name, None)
user32 = ctypes.WinDLL("user32", use_last_error=True)
user32.GetWindowThreadProcessId.argtypes = [w.HWND, ctypes.POINTER(w.DWORD)]
user32.IsWindowVisible.argtypes = [w.HWND]
user32.GetWindowTextW.argtypes = [w.HWND, w.LPWSTR, ctypes.c_int]
user32.SendMessageTimeoutW.argtypes = [w.HWND, w.UINT, w.WPARAM, w.LPARAM, w.UINT, w.UINT, ctypes.POINTER(ctypes.c_size_t)]
user32.PostMessageW.argtypes = [w.HWND, w.UINT, w.WPARAM, w.LPARAM]
CALLBACK = ctypes.WINFUNCTYPE(w.BOOL, w.HWND, w.LPARAM)


def find_window(pid):
    found = []

    @CALLBACK
    def visit(hwnd, unused):
        actual = w.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(actual))
        title = ctypes.create_unicode_buffer(256)
        user32.GetWindowTextW(hwnd, title, len(title))
        if actual.value == pid and user32.IsWindowVisible(hwnd) and title.value == "Minecraft Explorer":
            found.append(hwnd)
        return True

    user32.EnumWindows(visit, 0)
    return found[0] if found else None


result = {"exe": str(exe), "test_harness_absent": True, "embedded_licenses_verified": True, "fresh_cache": True}
process = None
hwnd = None
with (RUN / "stderr.txt").open("w", encoding="utf-8") as error:
    try:
        started = time.monotonic()
        process = subprocess.Popen([str(exe)], env=environment, stderr=error,
                                   stdout=subprocess.DEVNULL, creationflags=0x08000000)
        while time.monotonic() - started < 20 and not hwnd:
            assert process.poll() is None, "Release exited before its window appeared"
            hwnd = find_window(process.pid)
            time.sleep(.1)
        assert hwnd, "Release window did not appear"
        # 原生 HWND 可见时首次绘制可能仍在初始化；启动就绪与运行中响应分别测量。
        startup_timeouts = 0
        while True:
            response = ctypes.c_size_t()
            if user32.SendMessageTimeoutW(hwnd, 0, 0, 0, 2, 1000, ctypes.byref(response)):
                break
            startup_timeouts += 1
            assert process.poll() is None, "Release exited during startup"
            assert time.monotonic() - started < 20, "Release startup never became responsive"
        result.update(first_response_seconds=time.monotonic() - started,
                      startup_response_timeouts=startup_timeouts)
        window_icon = ctypes.c_size_t()
        # Winit 在 Windows 上将 Slint Window.icon 设置为 ICON_SMALL（标题栏图标）。
        assert user32.SendMessageTimeoutW(hwnd, 0x007F, 0, 0, 2, 1000, ctypes.byref(window_icon)), "Could not query window icon"
        assert window_icon.value, "Window icon missing"
        result["exe_and_window_icons_verified"] = True
        samples = []
        for _ in range(10):
            before = time.perf_counter()
            response = ctypes.c_size_t()
            assert user32.SendMessageTimeoutW(hwnd, 0, 0, 0, 2, 1000, ctypes.byref(response)), "Release stopped responding"
            samples.append((time.perf_counter() - before) * 1000)
            time.sleep(.2)
        user32.PostMessageW(hwnd, 0x0010, 0, 0)
        assert process.wait(timeout=10) == 0, "Release did not close normally"
        with sqlite3.connect(RUN / "app-data/index.db") as connection:
            assert connection.execute("PRAGMA user_version").fetchone()[0] == 2
            assert connection.execute("SELECT COUNT(*) FROM items").fetchone()[0] == 0
        result.update(passed=True, elapsed_seconds=time.monotonic() - started, wm_null_max_ms=max(samples))
    finally:
        if process is not None and process.poll() is None:
            if hwnd:
                user32.PostMessageW(hwnd, 0x0010, 0, 0)
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.terminate()
                process.wait(timeout=5)
        (RUN / "result.json").write_text(json.dumps(result, ensure_ascii=False, indent=2), encoding="utf-8")
        print(RUN)
        print(json.dumps(result, ensure_ascii=False))
