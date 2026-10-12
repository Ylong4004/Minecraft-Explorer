"""阶段 2 原生界面及重启验收，仅使用本地生成的数据。

使用 --features ui-smoke 构建后运行：
    E:\\python\\python.exe scripts/ui_stage2.py target/debug/minecraft-explorer.exe

使用 Pillow 和 psutil，不向 Windows 发送鼠标或键盘输入。
截图来自本程序的 Slint 软件渲染器；启动器测试文件仅保存为配置，不执行。
"""
import ctypes
from ctypes import wintypes as w
import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import sqlite3
import struct
import subprocess
import sys
import time
import uuid
import zipfile

from PIL import Image
import psutil

BASE = Path(__file__).resolve().parents[1]
RUN = BASE / ".test-artifacts" / ("ui-stage2-" + uuid.uuid4().hex[:8])
RUN.mkdir(parents=True)
ROOT = RUN / "我的 Minecraft"
CACHE = RUN / "app-data"
CACHE.mkdir()


def string(value):
    encoded = value.encode("utf-8")
    return struct.pack(">H", len(encoded)) + encoded


def png(color):
    output = io.BytesIO()
    Image.new("RGBA", (64, 64), color).save(output, format="PNG")
    return output.getvalue()


def pack(path, description, fields, color):
    path.parent.mkdir(parents=True, exist_ok=True)
    metadata = {"pack": {"description": description, **fields}}
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as archive:
        archive.writestr("pack.mcmeta", json.dumps(metadata))
        archive.writestr("pack.png", png(color))
        archive.writestr("data/stage2/fixture.json", b"{}")
        archive.writestr("unread/test.bin", b"fixture content")


def world(path):
    path.mkdir(parents=True)
    nbt = b"\x0a\0\0\x0a" + string("Data")
    nbt += b"\x08" + string("LevelName") + string("Stage2 世界")
    nbt += b"\x04" + string("LastPlayed") + struct.pack(">q", 1700000000123)
    nbt += b"\x0a" + string("Version") + b"\x08" + string("Name") + string("1.21.11") + b"\0\0\0"
    (path / "level.dat").write_bytes(gzip.compress(nbt, mtime=0))
    (path / "icon.png").write_bytes(png((65, 145, 94, 255)))
    (path / "region").mkdir()
    (path / "region" / "fixture.mca").write_bytes(b"fixture region")


version = ROOT / "versions" / "Stage2 Fixture"
version.mkdir(parents=True)
(version / "Stage2 Fixture.json").write_text(json.dumps({"id": "stage2-fixture", "inheritsFrom": "1.21.11"}), encoding="utf-8")
(version / "Stage2 Fixture.jar").write_bytes(b"version core fixture")
world(ROOT / "saves" / "Stage2World")
pack(ROOT / "resourcepacks" / "Stage2 Pack.zip", "Stage2 textures", {"pack_format": 15}, (55, 135, 200, 255))
(ROOT / "resourcepacks" / "Stage2 Copy.zip").write_bytes((ROOT / "resourcepacks" / "Stage2 Pack.zip").read_bytes())
pack(ROOT / "shaderpacks" / "Stage2 Shader.zip", "Stage2 shaders", {"min_format": [69, 0], "max_format": [69, 1]}, (150, 75, 220, 255))
pack(ROOT / "saves" / "Stage2World" / "datapacks" / "Stage2 Data.zip", "Stage2 data", {"pack_format": 88}, (235, 145, 45, 255))
(ROOT / "mods").mkdir()
with zipfile.ZipFile(ROOT / "mods" / "plain-file-name.jar", "w", zipfile.ZIP_DEFLATED) as archive:
    archive.writestr("fabric.mod.json", json.dumps({
        "schemaVersion": 1, "id": "stage2_fabric", "version": "2.0", "name": "Stage2 Fabric",
        "description": "Stage2 sample mod", "authors": ["Stage2 author"],
        "depends": {"minecraft": "1.21.x", "fabricloader": ">=0.15"}, "icon": "assets/stage2/icon.png",
    }))
    archive.writestr("assets/stage2/icon.png", png((225, 80, 115, 255)))
    archive.writestr("ignored.class", b"not executable fixture")

launcher = RUN / "launcher fixture" / "PCL-CE-fixture.exe"
launcher.parent.mkdir()
launcher.write_bytes(b"MZ inert configuration fixture; never execute")
(RUN / "stage2-fixture.marker").write_bytes(b"minecraft-explorer-generated-stage2-fixture")
(RUN / "fixture.json").write_text(json.dumps({"root": str(ROOT), "cache": str(CACHE), "pcl_ce": str(launcher)}, ensure_ascii=False), encoding="utf-8")
settings = {"sources": [{"path": str(ROOT), "kind": "Root", "associated_root": None}],
            "launchers": [], "mappings": [], "default_launcher": None, "instance_defaults": {}, "watch_enabled": True}
with sqlite3.connect(CACHE / "index.db") as connection:
    connection.execute("CREATE TABLE settings (id INTEGER PRIMARY KEY CHECK(id=1), json TEXT NOT NULL)")
    connection.execute("INSERT INTO settings VALUES(1,?)", (json.dumps(settings, ensure_ascii=False),))


def hashes(root):
    return {str(path.relative_to(root)): hashlib.sha256(path.read_bytes()).hexdigest()
            for path in sorted(root.rglob("*")) if path.is_file()}


original_hashes = hashes(ROOT)
original_launcher_hash = hashlib.sha256(launcher.read_bytes()).hexdigest()
user32 = ctypes.WinDLL("user32", use_last_error=True)
user32.GetWindowThreadProcessId.argtypes = [w.HWND, ctypes.POINTER(w.DWORD)]
user32.IsWindowVisible.argtypes = [w.HWND]
user32.GetWindowTextW.argtypes = [w.HWND, w.LPWSTR, ctypes.c_int]
user32.SendMessageTimeoutW.argtypes = [w.HWND, w.UINT, w.WPARAM, w.LPARAM, w.UINT, w.UINT, ctypes.POINTER(ctypes.c_size_t)]
user32.PostMessageW.argtypes = [w.HWND, w.UINT, w.WPARAM, w.LPARAM]
CALLBACK = ctypes.WINFUNCTYPE(w.BOOL, w.HWND, w.LPARAM)


def window_for(pid):
    windows = []

    @CALLBACK
    def visit(hwnd, unused):
        found = w.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(found))
        title = ctypes.create_unicode_buffer(256)
        user32.GetWindowTextW(hwnd, title, len(title))
        if found.value == pid and user32.IsWindowVisible(hwnd) and title.value == "Minecraft Explorer":
            windows.append(hwnd)
        return True

    user32.EnumWindows(visit, 0)
    return windows[0] if windows else None


exe = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else BASE / "target" / "debug" / "minecraft-explorer.exe"
metrics = {"exe": str(exe), "fixture_only": True, "system_input_sent": False, "runs": []}


def run_ui(phase):
    environment = os.environ.copy()
    environment.pop("MINECRAFT_EXPLORER_UI_SMOKE", None)
    environment.pop("MINECRAFT_EXPLORER_UI_REAL", None)
    environment.pop("MINECRAFT_EXPLORER_UI_STAGE2_VERIFY", None)
    environment["MINECRAFT_EXPLORER_DATA"] = str(CACHE)
    environment["MINECRAFT_EXPLORER_UI_STAGE2"] = str(RUN)
    if phase == "verify":
        environment["MINECRAFT_EXPLORER_UI_STAGE2_VERIFY"] = "1"
    report_path = RUN / f"stage2-ui-{phase}.json"
    run_metrics = {"phase": phase}
    metrics["runs"].append(run_metrics)
    hwnd = None
    process = None
    with (RUN / f"{phase}-stderr.txt").open("w", encoding="utf-8") as stderr:
        try:
            process = subprocess.Popen([str(exe)], cwd=BASE, env=environment,
                                       stdout=subprocess.DEVNULL, stderr=stderr, creationflags=0x08000000)
            deadline = time.monotonic() + 30
            while time.monotonic() < deadline and not hwnd:
                if process.poll() is not None:
                    raise RuntimeError(f"{phase}: app exited before creating a window")
                hwnd = window_for(process.pid)
                time.sleep(.1)
            if not hwnd:
                raise RuntimeError(f"{phase}: no app window within 30 seconds")
            monitor = psutil.Process(process.pid)
            samples = []
            cpu_before = monitor.cpu_times()
            cpu_after = cpu_before
            started = time.monotonic()
            deadline = started + 190
            while process.poll() is None and time.monotonic() < deadline:
                before = time.perf_counter()
                result = ctypes.c_size_t()
                if not user32.SendMessageTimeoutW(hwnd, 0, 0, 0, 2, 1500, ctypes.byref(result)):
                    if report_path.exists():
                        break
                    raise RuntimeError(f"{phase}: app window stopped responding")
                samples.append((time.perf_counter() - before) * 1000)
                try:
                    run_metrics["peak_rss_bytes"] = max(run_metrics.get("peak_rss_bytes", 0), monitor.memory_info().rss)
                    cpu_after = monitor.cpu_times()
                except psutil.NoSuchProcess:
                    break
                time.sleep(.2)
            if process.poll() is None and not report_path.exists():
                raise RuntimeError(f"{phase}: no stage report before timeout; build with --features ui-smoke and attach the harness")
            code = process.wait(timeout=8)
            run_metrics["exit_code"] = code
            run_metrics["wm_null_max_ms"] = max(samples, default=0)
            run_metrics["cpu_percent_sample"] = 100 * (cpu_after.user + cpu_after.system - cpu_before.user - cpu_before.system) / max(.01, time.monotonic() - started)
            if code != 0:
                raise RuntimeError(f"{phase}: process exited {code}; inspect {phase}-stderr.txt")
            if not report_path.exists():
                raise RuntimeError(f"{phase}: application did not write a UI result")
            result = json.loads(report_path.read_text(encoding="utf-8"))
            run_metrics["ui_checks"] = result
            if not result["passed"]:
                raise RuntimeError(f"{phase}: stage {result['stage']} failed: {result['error']}")
            run_metrics["responsive"] = True
        finally:
            if process is not None and process.poll() is None:
                if hwnd:
                    user32.PostMessageW(hwnd, 0x0010, 0, 0)
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.terminate()
                    process.wait(timeout=5)


try:
    run_ui("exercise")
    if hashes(ROOT) != original_hashes:
        raise RuntimeError("Generated Minecraft fixture files changed beyond the explicit temporary watcher test")
    run_ui("verify")
    if hashes(ROOT) != original_hashes:
        raise RuntimeError("Restart verification changed generated Minecraft fixture files")
    if hashlib.sha256(launcher.read_bytes()).hexdigest() != original_launcher_hash:
        raise RuntimeError("Launcher configuration fixture was modified")
    with sqlite3.connect(CACHE / "index.db") as connection:
        metrics["indexed_kinds"] = dict(connection.execute("SELECT kind, COUNT(*) FROM items GROUP BY kind"))
        saved = json.loads(connection.execute("SELECT json FROM settings WHERE id=1").fetchone()[0])
        assert len(saved["launchers"]) == 1 and saved["launchers"][0]["kind"] == "PclCe", saved
        annotated = connection.execute("SELECT favorite,tags_json,last_opened_ms FROM annotations a JOIN items i ON i.id=a.item_id WHERE i.name='Stage2 Fabric'").fetchone()
        assert annotated[0] == 1 and "主模组" in json.loads(annotated[1]) and annotated[2] is not None, annotated
        metrics["favorite_and_tags_survived_restart"] = True
        metrics["pcl_ce_configuration_survived_restart"] = True
    for kind, count in {"world": 1, "resource_pack": 2, "mod": 1, "shader_pack": 1, "data_pack": 1}.items():
        assert metrics["indexed_kinds"].get(kind) == count, metrics["indexed_kinds"]
    metrics["fixture_files_unchanged"] = True
    metrics["passed"] = True
except Exception as error:
    metrics["passed"] = False
    metrics["error"] = str(error)
    raise
finally:
    (RUN / "metrics.json").write_text(json.dumps(metrics, ensure_ascii=False, indent=2), encoding="utf-8")
    print(str(RUN), flush=True)
    print(json.dumps(metrics, ensure_ascii=False), flush=True)
