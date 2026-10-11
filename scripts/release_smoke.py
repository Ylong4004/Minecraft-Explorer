"""Verify the packaged normal build using a fresh cache and its own HWND only."""
import ctypes
from ctypes import wintypes as w
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import time
import uuid

BASE = Path(__file__).resolve().parents[1]
RUN = BASE / ".test-artifacts" / ("release-" + uuid.uuid4().hex[:8])
RUN.mkdir(parents=True)
exe = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else BASE / "dist/MinecraftExplorer/MinecraftExplorer.exe"
binary = exe.read_bytes()
assert b"MINECRAFT_EXPLORER_UI_STAGE2" not in binary, "UI test harness included in normal build"
assert b"MINECRAFT_EXPLORER_UI_SMOKE" not in binary, "UI test harness included in normal build"
environment = os.environ.copy()
environment["MINECRAFT_EXPLORER_DATA"] = str(RUN / "app-data")
for name in ("MINECRAFT_EXPLORER_UI_SMOKE", "MINECRAFT_EXPLORER_UI_STAGE2", "MINECRAFT_EXPLORER_UI_REAL", "MINECRAFT_EXPLORER_UI_STAGE2_VERIFY"):
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


result = {"exe": str(exe), "test_harness_absent": True, "fresh_cache": True}
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
