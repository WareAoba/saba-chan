"""L6 GUI: 실제 Electron 앱을 Playwright로 띄워 실제 데몬에 붙이고 화면으로 조작한다.

필요: saba-chan-gui에서 `npm ci` 와 `npx vite build`(build/), tests/e2e/gui에서 `npm ci`.
리눅스는 xvfb-run으로 화면 없이 돈다. 화면 글자·스크린샷은 artifacts/gui-*에 남는다.
"""

import functools
import http.server
import json
import shutil
import subprocess
import sys
import threading

import pytest

from harness.paths import E2E_DIR, IS_WINDOWS, REPO

pytestmark = pytest.mark.gui

GUI_DIR = REPO / "saba-chan-gui"
RUNNER = E2E_DIR / "gui" / "run.mjs"
DONE = r"Done \([^)]+\)!"


@pytest.fixture(scope="module")
def renderer():
    missing = [p for p in (GUI_DIR / "build" / "index.html", GUI_DIR / "node_modules" / "electron" / "path.txt", E2E_DIR / "gui" / "node_modules" / "playwright-core") if not p.exists()]
    if missing:
        pytest.skip(f"GUI 준비물이 없습니다: {[str(m) for m in missing]}")
    handler = functools.partial(http.server.SimpleHTTPRequestHandler, directory=str(GUI_DIR / "build"))
    handler.log_message = lambda *a: None
    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    yield f"http://127.0.0.1:{srv.server_address[1]}/index.html"
    srv.shutdown()


def labels(lang: str = "ko") -> dict:
    gui = json.loads((REPO / "locales" / lang / "gui.json").read_text(encoding="utf-8"))
    return {"start": gui["shell"]["start"], "stop": gui["shell"]["stop"], "input_placeholder": gui["console"]["input_placeholder"], "confirm": gui["modals"]["confirm"]}


@pytest.fixture
def ko_mc(sandbox, daemon_factory, fake_server):
    """한국어 화면 설정으로 데몬을 띄우고 가짜 서버를 하나 가져온다."""
    sandbox.write_config({"gui": {"language": "ko"}})
    daemon = daemon_factory()
    folder, port = fake_server("mc1")
    iid = daemon.import_server("mc1", folder, module_settings={"ram": "256M"})
    return daemon, iid, folder, port


def run_gui(daemon, renderer, scenario: str, timeout: float = 240, **args) -> dict:
    art = daemon.sb.artifacts / f"gui-{scenario}"
    env = daemon.sb.env(
        IPC_BASE=f"http://127.0.0.1:{daemon.sb.ipc_port}",
        ELECTRON_START_URL=renderer,
        E2E_GUI_DIR=str(GUI_DIR),
        E2E_ARTIFACTS=str(art),
        E2E_ARGS=json.dumps(args),
        ELECTRON_DISABLE_SECURITY_WARNINGS="1",
    )
    cmd = ["node", str(RUNNER), scenario]
    if not IS_WINDOWS and sys.platform != "darwin" and not env.get("DISPLAY"):
        if not shutil.which("xvfb-run"):
            pytest.skip("화면이 없고 xvfb-run도 없습니다")
        cmd = ["xvfb-run", "-a", "-s", "-screen 0 1440x900x24", *cmd]
    proc = subprocess.run(cmd, cwd=E2E_DIR / "gui", env=env, capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=timeout)
    art.mkdir(parents=True, exist_ok=True)
    (art / "electron.log").write_text(proc.stdout + "\n--- stderr ---\n" + proc.stderr, encoding="utf-8")
    last = next((line for line in reversed(proc.stdout.splitlines()) if line.startswith("{")), None)
    if not last:
        raise AssertionError(f"GUI 실행기가 결과를 남기지 않음 (코드 {proc.returncode})\n{proc.stderr[-3000:]}")
    result = json.loads(last)
    assert result["ok"], f"GUI 시나리오 {scenario} 실패: {result.get('error')}\n화면: {art}"
    return result


def test_gui_attaches_to_running_daemon_and_lists_servers(mc, renderer):
    daemon, iid, *_ = mc
    pid_before = daemon.proc.pid
    run_gui(daemon, renderer, "attach", serverName="mc1")
    # GUI가 데몬을 새로 띄우거나 갈아 끼우지 않았다
    assert daemon.alive() and daemon.api.get("/health", token=False).json()["pid"] == pid_before


def test_gui_start_console_command_stop(ko_mc, renderer):
    daemon, iid, *_ = ko_mc
    run_gui(daemon, renderer, "startConsoleStop", serverName="mc1", l=labels("ko"))
    assert daemon.server_status(iid) == "stopped"
    audit = daemon.api.get("/api/audit?limit=50")
    if audit.status_code == 200:
        assert "gui" in audit.text.lower(), "GUI에서 한 조작이 활동 기록에 GUI로 남지 않음"


def test_gui_explore_screenshot(ko_mc, renderer):
    """화면 전체를 한 장 남긴다 (사람이 볼 증거). 실패 판정은 하지 않는다."""
    daemon, *_ = ko_mc
    run_gui(daemon, renderer, "explore", waitMs=6000)
