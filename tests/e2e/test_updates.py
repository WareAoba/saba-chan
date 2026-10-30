"""L4 업데이트 확인·내려받기: 서명 검증, 변조·다운그레이드·해시 불일치 거부.

`--features mock-relay`로 빌드한 데몬이 필요하다 (공개 시험 키를 믿는 빌드). 위치: SABA_E2E_MOCK_CORE,
기본 target/e2e-mock/debug/saba-core. 적용은 같은 키를 믿는 설치기(SABA_E2E_MOCK_UPDATER, 기본 target/e2e-mock/debug/saba-chan-installer,
`cargo build -p saba-chan-installer --no-default-features --features mock-relay`)와 GUI(SABA_E2E_GUI_EXE)가 있어야 본다.
릴리스 업데이터는 시험 키 서명을 거부하므로 쓰지 않는다.
"""

import os
import platform
import re
import shutil
import subprocess
import time
from pathlib import Path

import pytest

from harness import procs
from harness.daemon import wait_until
from harness.fake_updates import NEXT, FakeUpdates
from harness.paths import EXE, REPO

pytestmark = pytest.mark.mock_relay

MOCK_CORE = Path(os.environ.get("SABA_E2E_MOCK_CORE", REPO / "target" / "e2e-mock" / "debug" / f"saba-core{EXE}"))
MOCK_UPDATER = Path(os.environ.get("SABA_E2E_MOCK_UPDATER", MOCK_CORE.parent / f"saba-chan-installer{EXE}"))
GUI_EXE = Path(os.environ.get("SABA_E2E_GUI_EXE", REPO / "target" / "debug" / f"saba-chan-gui{EXE}"))


def _platform() -> str:
    os_name = {"win32": "windows", "darwin": "macos"}.get(os.sys.platform, "linux")
    arch = {"x86_64": "x64", "AMD64": "x64", "aarch64": "arm64", "arm64": "arm64"}.get(platform.machine(), platform.machine())
    return f"{os_name}-{arch}"


@pytest.fixture
def updates():
    if not MOCK_CORE.exists():
        pytest.skip(f"mock-relay 빌드가 없습니다: {MOCK_CORE} (CARGO_TARGET_DIR=target/e2e-mock cargo build -p saba-core --features mock-relay)")
    u = FakeUpdates(_platform())
    u.add_component("saba-core", NEXT, {f"saba-core{EXE}": b"new core"})
    u.add_component("cli", NEXT, {f"saba-chan-cli{EXE}": b"new cli"})
    yield u
    u.close()


@pytest.fixture
def udaemon(sandbox, updates, daemon_factory):
    # 설치 폴더를 시험 폴더 안으로: 데몬 exe 옆이 install_root다
    bindir = sandbox.root / "install"
    bindir.mkdir()
    core = bindir / MOCK_CORE.name
    shutil.copy(MOCK_CORE, core)
    sandbox.write_config({"updates": {"relay_url": updates.url, "auto_download": False, "auto_check": False}})
    return daemon_factory(core=core)


def _check(d) -> dict:
    return d.api.ok("POST", "/api/updates/check", timeout=60)


def _component(body: dict, key: str) -> dict:
    return next(c for c in body["components"] if c["component"] == key)


def test_check_finds_signed_release(udaemon, updates):
    body = _check(udaemon)
    assert body["ok"], body
    core = _component(body, "saba-core")
    assert core["update_available"] and core["latest_version"] == NEXT, body
    checks = [r for r in updates.requests if r["path"] == "/v1/updates/check"]
    assert checks and checks[0]["query"]["platform"] == [_platform()]
    assert checks[0]["query"]["current"] != [""], "current가 빈 값 (운영 relay는 400 INVALID_CURRENT_VERSION)"
    sent = str(updates.requests)
    assert udaemon.sb.token() not in sent


def test_tampered_manifest_is_rejected(udaemon, updates):
    updates.tamper = True
    body = _check(udaemon)
    assert not body["ok"], f"변조한 manifest를 받아들임: {body}"
    assert "signature" in body["error"].lower(), body


def test_wrong_platform_manifest_is_rejected(udaemon, updates):
    updates.platform = "windows-x64" if not _platform().startswith("windows") else "linux-x64"
    body = _check(udaemon)
    assert not body["ok"] or not any(c["update_available"] for c in body["components"]), body


def test_downgrade_is_not_offered(udaemon, updates):
    updates.version = "0.1.0"
    updates.components = {}
    updates.add_component("saba-core", "0.1.0", {f"saba-core{EXE}": b"old"})
    body = _check(udaemon)
    assert not any(c["update_available"] for c in body.get("components", [])), body


def test_download_verifies_sha256(udaemon, updates):
    _check(udaemon)
    r = udaemon.api.ok("POST", "/api/updates/download", {"components": ["cli"]}, timeout=120)
    assert r.get("ok"), r
    wait_until(lambda: _component(udaemon.api.ok("GET", "/api/updates/status"), "cli").get("downloaded"), 60, "cli 내려받음")


def test_download_with_bad_sha256_is_refused(udaemon, updates):
    key = "cli"
    updates.components[key]["sha256"] = "0" * 64
    _check(udaemon)
    r = udaemon.api.ok("POST", "/api/updates/download", {"components": [key]}, timeout=120)
    status = _component(udaemon.api.ok("GET", "/api/updates/status"), key)
    assert not status.get("downloaded"), f"해시가 틀린 파일을 내려받음으로 표시: {r} {status}"


def test_release_build_does_not_trust_mock_key(sandbox, updates, daemon_factory):
    """일반 빌드는 공개 시험 키 서명을 믿지 않아야 한다."""
    sandbox.write_config({"updates": {"relay_url": updates.url, "auto_download": False, "auto_check": False}})
    d = daemon_factory()
    body = _check(d)
    assert not body["ok"], f"일반 빌드가 mock 키 서명을 믿음: {body}"


def test_daemon_started_update_relaunches_the_open_gui_once(sandbox, updates, daemon_factory, xdisplay):
    """CLI·자동 업데이트처럼 GUI가 아닌 쪽이 업데이터를 띄우면, 데몬이 떠 있는 GUI를 끄고 업데이터가 그 GUI를 한 번만 다시 띄운다 (#141)."""
    for exe in (MOCK_UPDATER, GUI_EXE):
        if not exe.exists():
            pytest.skip(f"없습니다: {exe}")
    install = sandbox.root / "install"
    install.mkdir()
    core = Path(shutil.copy2(MOCK_CORE, install / MOCK_CORE.name))
    shutil.copy2(MOCK_UPDATER, install / MOCK_UPDATER.name)
    gui_exe = Path(shutil.copy2(GUI_EXE, install / f"saba-chan-gui{EXE}"))
    # 다시 뜬 GUI는 설정의 포트로 데몬을 띄운다: 기본 57474면 실제 데몬과 부딪힌다
    sandbox.write_config({"updates": {"relay_url": updates.url, "auto_download": False, "auto_check": False}, "gui": {"ipcPort": sandbox.ipc_port}})
    display = {"DISPLAY": xdisplay} if xdisplay else {}
    d = daemon_factory(core=core, extra_env=display)
    env = sandbox.env(IPC_BASE=f"http://127.0.0.1:{sandbox.ipc_port}", **display)
    gui_log = open(sandbox.root / "gui.log", "w", encoding="utf-8")
    gui = subprocess.Popen([str(gui_exe)], env=env, stdout=gui_log, stderr=subprocess.STDOUT, start_new_session=True)
    relaunched: list[int] = []
    try:
        wait_until(lambda: any(c.get("kind") == "gui" and c.get("pid") == gui.pid for c in d.api.ok("GET", "/api/clients")["clients"]), 60, "GUI가 데몬에 등록")
        _check(d)
        d.api.ok("POST", "/api/updates/download", {"components": ["saba-core"]}, timeout=120)
        wait_until(lambda: _component(d.api.ok("GET", "/api/updates/status"), "saba-core").get("downloaded"), 60, "saba-core 내려받음")
        applied = d.api.ok("POST", "/api/updates/apply", {"components": ["saba-core"]}, timeout=60)
        assert applied.get("requires_updater"), applied
        # CLI처럼 GUI가 아닌 것을 다시 띄우라고 부른다: 데몬이 떠 있는 GUI로 바꿔야 한다
        assert d.api.ok("POST", "/api/updates/launch-updater", {"relaunch": [str(core), "--spawned"]})["updater_launched"]
        wait_until(lambda: gui.poll() is not None, 30, "데몬이 원래 GUI를 끔")
        wait_until(lambda: not d.alive(), 30, "데몬 종료")
        log = sandbox.tmp / "saba-updater.log"
        wait_until(lambda: log.exists() and "Relaunch succeeded" in log.read_text(encoding="utf-8", errors="replace"), 90, "업데이터가 GUI를 다시 띄움")
        time.sleep(8)  # 두 번째로 띄우는지 지켜본다
        text = log.read_text(encoding="utf-8", errors="replace")
        spawns = re.findall(r"\[Apply\] Spawning: (.*)", text)
        assert len(spawns) == 1 and str(gui_exe) in spawns[0], text[-3000:]
        relaunched = [pid for pid, cmd in procs.processes_mentioning(str(gui_exe)) if pid != gui.pid]
        assert len(relaunched) == 1, f"다시 뜬 GUI가 하나가 아님: {relaunched}"
        assert (install / core.name).read_bytes() == b"new core", "업데이터가 코어를 바꾸지 않음"
    finally:
        if (sandbox.tmp / "saba-updater.log").exists():
            sandbox.artifacts.mkdir(parents=True, exist_ok=True)
            shutil.copy2(sandbox.tmp / "saba-updater.log", sandbox.artifacts / "saba-updater.log")
        for pid in relaunched:
            procs.kill_pid(pid)
        if gui.poll() is None:
            gui.kill()
        gui_log.close()
        # 다시 뜬 GUI가 띄운 데몬도 끈다 (같은 시험 포트, 뜨는 데 시간이 걸릴 수 있다)
        for _ in range(20):
            try:
                procs.kill_pid(d.api.get("/health", token=False, timeout=2).json()["pid"])
                break
            except Exception:
                time.sleep(0.5)
