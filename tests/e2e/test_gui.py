"""L6 GUI: 실제 Electron 앱을 Playwright로 띄워 실제 데몬에 붙이고 화면으로 조작한다.

필요: saba-chan-gui에서 `npm ci` 와 `npx vite build`(build/), tests/e2e/gui에서 `npm ci`.
리눅스는 xvfb-run으로 화면 없이 돈다. 화면 글자·스크린샷은 artifacts/gui-*에 남는다.
"""

import functools
import http.server
import json
import os
import shutil
import subprocess
import sys
import threading
from pathlib import Path

import pytest

from harness import procs
from harness.daemon import wait_until
from harness.paths import E2E_DIR, EXE, IS_WINDOWS, REPO

pytestmark = pytest.mark.gui

# 다른 체크아웃의 GUI를 시험할 때 (예: 병합 전 PR): SABA_E2E_GUI_DIR
GUI_DIR = Path(os.environ.get("SABA_E2E_GUI_DIR", REPO / "saba-chan-gui"))
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
    gui = json.loads((GUI_DIR.parent / "locales" / lang / "gui.json").read_text(encoding="utf-8"))
    return {
        "start": gui["shell"]["start"],
        "stop": gui["shell"]["stop"],
        "input_placeholder": gui["console"]["input_placeholder"],
        "confirm": gui["modals"]["confirm"],
        "daemon_down_stale": gui["shell"]["daemon_down_stale"],
        "daemon_down_title": gui["shell"]["daemon_down_title"],
        "failed_hint": gui["loading"].get("failed_hint", ""),
        "retry": gui["loading"].get("retry", ""),
        "continue": gui["loading"].get("continue", ""),
    }


@pytest.fixture
def ko_mc(sandbox, daemon_factory, fake_server):
    """한국어 화면 설정으로 데몬을 띄우고 가짜 서버를 하나 가져온다."""
    sandbox.write_config({"gui": {"language": "ko"}})
    daemon = daemon_factory()
    folder, port = fake_server("mc1")
    iid = daemon.import_server("mc1", folder, module_settings={"ram": "256M"})
    return daemon, iid, folder, port


def gui_command(daemon, renderer, scenario: str, **args) -> tuple[list[str], dict, "Path"]:
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
    return cmd, env, art


def gui_result(art, stdout: str, stderr: str, returncode: int, scenario: str) -> dict:
    art.mkdir(parents=True, exist_ok=True)
    (art / "electron.log").write_text(stdout + "\n--- stderr ---\n" + stderr, encoding="utf-8")
    last = next((line for line in reversed(stdout.splitlines()) if line.startswith("{")), None)
    if not last:
        raise AssertionError(f"GUI 실행기가 결과를 남기지 않음 (코드 {returncode})\n{stderr[-3000:]}")
    result = json.loads(last)
    assert result["ok"], f"GUI 시나리오 {scenario} 실패: {result.get('error')}\n화면: {art}"
    return result


def run_gui(daemon, renderer, scenario: str, timeout: float = 240, **args) -> dict:
    cmd, env, art = gui_command(daemon, renderer, scenario, **args)
    proc = subprocess.run(cmd, cwd=E2E_DIR / "gui", env=env, capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=timeout)
    return gui_result(art, proc.stdout, proc.stderr, proc.returncode, scenario)


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


def test_gui_shows_daemon_down_and_reconnects_to_new_daemon(ko_mc, renderer, daemon_factory):
    """데몬이 꺼지면 알림을 띄우고, 새 데몬(새 토큰)이 같은 포트에 뜨면 다시 조작된다. GUI의 '다시 시작' 버튼은 누르지 않는다."""
    daemon, iid, *_ = ko_mc
    cmd, env, art = gui_command(daemon, renderer, "reconnect", serverName="mc1", l=labels("ko"))
    # xvfb-run → node → Electron 트리 전체를 끝낼 수 있게 새 세션으로 띄운다
    proc = subprocess.Popen(cmd, cwd=E2E_DIR / "gui", env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, encoding="utf-8", errors="replace", start_new_session=not IS_WINDOWS)
    try:
        wait_until(lambda: (art / "attached").exists() or proc.poll() is not None, 120, "GUI가 데몬에 붙음")
        old_token = daemon.sb.token_path.read_text()
        daemon.shutdown()
        wait_until(lambda: (art / "saw-down").exists() or proc.poll() is not None, 90, "GUI가 데몬 끊김을 보여 줌")
        new = daemon_factory()
        assert daemon.sb.token_path.read_text() != old_token, "새 데몬이 토큰을 바꾸지 않아 토큰 다시 읽기를 시험하지 못함"
        out, err = proc.communicate(timeout=240)
    finally:
        if proc.poll() is None:
            procs.kill_group(proc.pid)
            out, err = proc.communicate(timeout=30)
    gui_result(art, out, err, proc.returncode, "reconnect")
    assert new.server_status(iid) == "running"


def test_gui_stops_on_daemon_start_failure_and_offers_retry(sandbox, daemon_factory, renderer):
    """데몬을 띄우지 못하면 메인 화면으로 넘어가지 않고 이유와 [다시 시도]·[그대로 열기]를 보인다 (프리뷰 P1-2, #74·#75)."""
    # 개발 모드 GUI는 <저장소>/target/release/saba-core를 띄운다. 그게 있으면 진짜 데몬이 떠서 이 상황을 만들 수 없다
    if (GUI_DIR.parent / "target" / "release" / f"saba-core{EXE}").exists():
        pytest.skip("target/release/saba-core가 있어 GUI가 데몬을 띄울 수 있음")
    if not labels("ko")["failed_hint"]:
        pytest.skip("이 GUI에는 데몬 기동 실패 화면이 없음")
    # 혹시 띄우더라도 시험 포트로: 기본 57474로 실제 데몬과 부딪히지 않게
    sandbox.write_config({"gui": {"language": "ko", "ipcPort": sandbox.ipc_port}})
    daemon = daemon_factory(start=False)
    run_gui(daemon, renderer, "daemonFailed", timeout=300, l=labels("ko"))
    assert not daemon.alive()
