"""L6 GUI: 실제 Tauri 앱을 tauri-driver(WebDriver)로 띄워 실제 데몬에 붙이고 화면으로 조작한다.

필요: saba-chan-gui에서 `npx vite build`(build/) 후 `cargo build -p saba-chan-gui`(화면이 실행 파일에 들어간다),
`cargo install tauri-driver --locked`, 리눅스는 WebKitWebDriver(webkit2gtk-driver)·Xvfb, Windows는 Edge와 같은 버전의 msedgedriver.
화면 글자·스크린샷은 artifacts/gui-*에 남는다.
"""

import json
import os
import re
import shutil
import time
from pathlib import Path

import pytest

from harness import procs
from harness.paths import EXE, IS_WINDOWS, REPO
from harness.webdriver import ENTER, Gui, tauri_driver

pytestmark = pytest.mark.gui

# 다른 빌드를 시험할 때 (예: 병합 전 PR): SABA_E2E_GUI_EXE
GUI_EXE = Path(os.environ.get("SABA_E2E_GUI_EXE", REPO / "target" / "debug" / f"saba-chan-gui{EXE}"))
# 개발 빌드는 늘 저장소의 target/*/saba-core를 띄우므로, 데몬을 못 띄우는 상황은 배포 빌드를 옮겨서 만든다
# none이면 이 시험을 건너뛴다 (run-pc.ps1이 -Release 없이 돌 때 예전 배포 빌드를 쓰지 않게)
_release = os.environ.get("SABA_E2E_GUI_RELEASE_EXE", str(REPO / "target" / "release" / f"saba-chan-gui{EXE}"))
RELEASE_GUI_EXE = None if _release.lower() in ("", "none") else Path(_release)


@pytest.fixture(scope="module")
def display(xdisplay):
    if tauri_driver() is None:
        pytest.skip("tauri-driver가 없습니다 (cargo install tauri-driver --locked)")
    return xdisplay


def labels(lang: str = "ko") -> dict:
    gui = json.loads((REPO / "locales" / lang / "gui.json").read_text(encoding="utf-8"))
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
        "agreement_title": gui.get("agreements", {}).get("start_title", ""),
        "accept_and_start": gui.get("agreements", {}).get("accept_and_start", ""),
        "tab_settings": gui["shell"]["tab_settings"],
        "save_settings": gui["server_settings"]["save_settings"],
        "memory_limit": gui["memory_limit"]["label"],
        "invalid_format": gui["launch_preview"]["invalid_format"],
        "popout": gui["console"]["popout"],
        "popped_out": gui["shell"]["console_popped_out"],
        "popin": gui["shell"]["console_popin"],
        "open_terminal": gui["terminal"]["open"],
        "terminal_unsupported": gui["terminal"]["err_unsupported_platform"],
        "ext_gui_title": gui["trust"]["extension_gui"]["title"],
        "ext_gui_allow": gui["trust"]["extension_gui"]["allow"],
        "ext_gui_deny": gui["trust"]["extension_gui"]["deny"],
    }


L = labels("ko")


@pytest.fixture
def open_gui(display, request):
    """open_gui(daemon, scenario, exe=GUI_EXE) → Gui. 실패하면 화면을 남기고, 끝나면 앱과 드라이버를 닫는다."""
    opened: list[tuple[Gui, Path]] = []

    def _open(daemon, scenario: str, exe: Path = GUI_EXE) -> Gui:
        if not exe.exists():
            pytest.skip(f"GUI 실행 파일이 없습니다: {exe}")
        art = daemon.sb.artifacts / f"gui-{scenario}"
        art.mkdir(parents=True, exist_ok=True)
        env = daemon.sb.env(IPC_BASE=f"http://127.0.0.1:{daemon.sb.ipc_port}")
        if display:
            env["DISPLAY"] = display
        gui = Gui(exe, env, art / "tauri-driver.log")
        opened.append((gui, art))
        return gui

    yield _open
    rep = getattr(request.node, "rep_call", None)  # conftest가 남긴다
    failed = rep is None or rep.failed
    for gui, art in opened:
        if failed:
            gui.snap(art, "fail")
        gui.close()


@pytest.fixture
def ko_mc(sandbox, daemon_factory, fake_server):
    """한국어 화면 설정으로 데몬을 띄우고 가짜 서버를 하나 가져온다."""
    sandbox.write_config({"gui": {"language": "ko"}})
    daemon = daemon_factory()
    folder, port = fake_server("mc1")
    iid = daemon.import_server("mc1", folder, module_settings={"ram": "256M"})
    return daemon, iid, folder, port


def start_and_open_console(gui: Gui, name: str) -> None:
    """서버 줄의 켜기 버튼 → 서버 이름을 눌러 상세 화면 → 콘솔에 Done"""
    gui.click(f"{name} {L['start']}", timeout=60)
    gui.click(name, kinds="*")
    gui.wait_text("Done (")


def test_gui_attaches_to_running_daemon_and_lists_servers(mc, open_gui):
    daemon, iid, *_ = mc
    pid_before = daemon.proc.pid
    gui = open_gui(daemon, "attach")
    gui.wait_text("mc1")
    gui.snap(daemon.sb.artifacts / "gui-attach", "attach")
    # GUI가 데몬을 새로 띄우거나 갈아 끼우지 않았다
    assert daemon.alive() and daemon.api.get("/health", token=False).json()["pid"] == pid_before


def test_gui_start_console_command_stop(ko_mc, open_gui):
    daemon, iid, *_ = ko_mc
    art = daemon.sb.artifacts / "gui-startConsoleStop"
    gui = open_gui(daemon, "startConsoleStop")
    start_and_open_console(gui, "mc1")
    gui.type(f'[placeholder="{L["input_placeholder"]}"]', "say gui-e2e" + ENTER)
    gui.wait_text("[Server] gui-e2e", timeout=30)
    gui.snap(art, "console")
    gui.click(L["stop"])
    # 정지 확인 창의 "확인"
    gui.click(L["confirm"], timeout=10)
    gui.wait_text("Stopping the server")
    gui.wait(lambda: gui.find(L["start"]), 60, f"[{L['start']}]")
    gui.snap(art, "stopped")
    assert daemon.server_status(iid) == "stopped"
    audit = daemon.api.get("/api/audit?limit=50")
    if audit.status_code == 200:
        assert "gui" in audit.text.lower(), "GUI에서 한 조작이 활동 기록에 GUI로 남지 않음"


def test_gui_explore_screenshot(ko_mc, open_gui):
    """화면 전체를 한 장 남긴다 (사람이 볼 증거). 실패 판정은 하지 않는다."""
    daemon, *_ = ko_mc
    gui = open_gui(daemon, "explore")
    time.sleep(6)
    gui.snap(daemon.sb.artifacts / "gui-explore", "explore")


def test_gui_shows_daemon_down_and_reconnects_to_new_daemon(ko_mc, open_gui, daemon_factory):
    """데몬이 꺼지면 알림을 띄우고, 새 데몬(새 토큰)이 같은 포트에 뜨면 다시 조작된다. GUI의 '다시 시작' 버튼은 누르지 않는다."""
    daemon, iid, *_ = ko_mc
    art = daemon.sb.artifacts / "gui-reconnect"
    gui = open_gui(daemon, "reconnect")
    gui.wait_text("mc1")
    old_token = daemon.sb.token_path.read_text()
    daemon.shutdown()
    gui.wait_text(L["daemon_down_stale"])
    gui.snap(art, "daemon-down")
    new = daemon_factory()
    assert daemon.sb.token_path.read_text() != old_token, "새 데몬이 토큰을 바꾸지 않아 토큰 다시 읽기를 시험하지 못함"
    gui.wait_gone(".daemon-down-notice", timeout=90)
    start_and_open_console(gui, "mc1")
    gui.snap(art, "reconnected")
    assert new.server_status(iid) == "running"


def test_gui_stops_on_daemon_start_failure_and_offers_retry(sandbox, daemon_factory, open_gui, tmp_path):
    """데몬을 띄우지 못하면 메인 화면으로 넘어가지 않고 이유와 [다시 시도]·[그대로 열기]를 보인다 (프리뷰 P1-2, #74·#75)."""
    if RELEASE_GUI_EXE is None or not RELEASE_GUI_EXE.exists():
        pytest.skip(f"배포 빌드 GUI가 없습니다: {RELEASE_GUI_EXE}")
    if not L["failed_hint"]:
        pytest.skip("이 GUI에는 데몬 기동 실패 화면이 없음")
    # 배포 빌드는 실행 파일 옆의 saba-core를 띄운다. 옆에 아무것도 없는 폴더로 옮긴다
    lonely = tmp_path / "gui-only"
    lonely.mkdir()
    exe = Path(shutil.copy2(RELEASE_GUI_EXE, lonely / RELEASE_GUI_EXE.name))
    # 혹시 띄우더라도 시험 포트로: 기본 57474로 실제 데몬과 부딪히지 않게
    sandbox.write_config({"gui": {"language": "ko", "ipcPort": sandbox.ipc_port}})
    daemon = daemon_factory(start=False)
    art = daemon.sb.artifacts / "gui-daemonFailed"
    gui = open_gui(daemon, "daemonFailed", exe=exe)
    gui.wait_text(L["failed_hint"])
    gui.snap(art, "daemon-failed")
    gui.click(L["retry"])
    gui.wait(lambda: gui.enabled(L["retry"]), 90, f"[{L['retry']}] 다시 눌림")
    gui.wait_text(L["failed_hint"], timeout=5)
    gui.snap(art, "retry-failed")
    gui.click(L["continue"])
    gui.wait_text(L["daemon_down_title"], timeout=30)
    gui.snap(art, "opened-without-daemon")
    assert not daemon.alive()


def test_gui_asks_for_agreement_before_first_start(sandbox, daemon_factory, fake_server, open_gui):
    """EULA에 동의하지 않은 서버를 켜면 GUI가 약관 창을 띄우고, 사람이 체크해야 켠다 (GUI #77, 데몬 #81)."""
    if not L["agreement_title"]:
        pytest.skip("이 GUI에는 약관 동의 창이 없음")
    sandbox.write_config({"gui": {"language": "ko"}})
    daemon = daemon_factory()
    folder, _ = fake_server("mc1", eula=False)
    iid = daemon.import_server("mc1", folder, module_settings={"ram": "256M"}, agree=False)
    art = daemon.sb.artifacts / "gui-agreeAndStart"
    gui = open_gui(daemon, "agreeAndStart")
    gui.click(f"mc1 {L['start']}", timeout=60)
    gui.wait_text(L["agreement_title"], timeout=30)
    assert gui.find(L["accept_and_start"]) and not gui.enabled(L["accept_and_start"]), "체크하기 전에 [동의하고 켜기]가 눌림"
    gui.snap(art, "agreement-dialog")
    gui.js("document.querySelector('.agreement-modal input[type=checkbox]').click()")
    gui.wait(lambda: gui.enabled(L["accept_and_start"]), 10, f"[{L['accept_and_start']}] 눌림")
    gui.click(L["accept_and_start"])
    gui.click("mc1", kinds="*")
    gui.wait_text("Done (")
    gui.snap(art, "agreed-and-running")
    assert daemon.server_status(iid) == "running"
    assert "eula=true" in (folder / "eula.txt").read_text()
    eula = next(a for a in daemon.api.ok("GET", f"/api/instance/{iid}")["agreements"] if a["id"] == "eula")
    assert eula["accepted"], eula


def test_gui_launch_preview_and_memory_limit_in_settings(ko_mc, open_gui):
    """설정 탭: 실행 인자 미리보기가 데몬 계산을 보이고, 추가 인자가 설정 칸에 반영되며, 틀린 형식은 토스트로 알린다.
    메모리 상한은 사람이 치는 "2G" 그대로 저장된다 (0.1.5 릴리스 확인에서 빠진 항목)."""
    daemon, iid, *_ = ko_mc
    art = daemon.sb.artifacts / "gui-launchPreview"
    gui = open_gui(daemon, "launchPreview")
    gui.click("mc1", kinds="*", timeout=60)
    gui.click(L["tab_settings"], kinds="[role=tab]")
    line = lambda: gui.js("const el = document.querySelector('.launch-preview-line'); return el ? el.innerText : ''") or ""
    gui.wait(lambda: "-Xmx256M" in line(), 30, "미리보기에 -Xmx256M")
    gui.fill(".launch-preview-input", "-Xmxabc")
    gui.wait_text(L["invalid_format"], timeout=15)
    gui.snap(art, "invalid-format")
    gui.fill(".launch-preview-input", "-Xmx384M")
    gui.wait(lambda: "-Xmx384M" in line() and "-Xmxabc" not in line(), 15, "미리보기에 -Xmx384M")
    gui.fill(".memory-limit-input", "2G")
    gui.snap(art, "edited")
    gui.click(L["save_settings"])
    saved = lambda: daemon.api.ok("GET", f"/api/instance/{iid}")
    gui.wait(lambda: saved().get("memory_limit_mb") == 2048, 15, "memory_limit_mb 2048 저장")
    inst = saved()
    assert inst["module_settings"].get("ram") == "384M", inst["module_settings"]
    assert "-Xmxabc" not in json.dumps(inst), inst


def test_gui_console_popout_and_back(ko_mc, open_gui):
    """콘솔을 별도 창으로 떼면 새 창에 콘솔이 이어지고, 메인 화면은 안내로 바뀌며, 되돌리면 창이 닫힌다."""
    daemon, iid, *_ = ko_mc
    art = daemon.sb.artifacts / "gui-popout"
    gui = open_gui(daemon, "popout")
    start_and_open_console(gui, "mc1")
    main = gui.handles()
    assert len(main) == 1, main
    gui.click(L["popout"])
    gui.wait_text(L["popped_out"], timeout=15)
    popout = gui.wait(lambda: [h for h in gui.handles() if h not in main], 15, "콘솔 창")[0]
    gui.switch(popout)
    gui.wait_text("Done (", timeout=30)
    gui.snap(art, "popout-window")
    gui.type(f'[placeholder="{L["input_placeholder"]}"]', "say popout-e2e" + ENTER)
    gui.wait_text("[Server] popout-e2e", timeout=30)
    gui.switch(main[0])
    gui.snap(art, "main-placeholder")
    gui.click(L["popin"])
    gui.wait(lambda: gui.handles() == main, 15, "콘솔 창 닫힘")
    gui.wait_text("[Server] popout-e2e", timeout=30)
    gui.snap(art, "popped-in")
    assert daemon.server_status(iid) == "running"


def test_gui_open_server_in_terminal(ko_mc, open_gui):
    """서버 화면의 [터미널로 열기]: Windows는 데몬이 그 서버의 CLI 콘솔을 새 창에 띄우고, 다른 OS는 못 연다고 알린다."""
    daemon, iid, *_ = ko_mc
    art = daemon.sb.artifacts / "gui-openTerminal"
    gui = open_gui(daemon, "openTerminal")
    gui.click("mc1", kinds="*", timeout=60)
    gui.click(L["open_terminal"])
    if not IS_WINDOWS:
        gui.wait_text(L["terminal_unsupported"], timeout=15)
        gui.snap(art, "unsupported")
        return
    opened = gui.wait(lambda: re.search(r"\[Launch\] Opened CLI .*\(pid (\d+)\)", daemon.log_text()), 30, "데몬 로그의 Opened CLI")
    pid = int(opened.group(1))
    try:
        assert iid in opened.group(0), opened.group(0)
        time.sleep(2)
        gui.snap(art, "opened")
        assert procs.pid_alive(pid), "터미널 창의 CLI가 바로 끝남"
    finally:
        procs.kill_pid(pid)


def write_gui_extension(sandbox, ext_id: str = "e2e-gui") -> None:
    """GUI 번들만 있는 확인되지 않은 익스텐션 (카탈로그에서 받지 않았으니 trust=unverified). 번들은 로드되면 전역 표시를 남긴다."""
    root = sandbox.extensions / ext_id
    (root / "gui").mkdir(parents=True, exist_ok=True)
    (root / "manifest.json").write_text(
        json.dumps(
            {
                "id": ext_id,
                "name": "E2E GUI",
                "version": "0.0.1",
                "description": "E2E consent fixture",
                "author": "e2e",
                "min_app_version": "0.1.0",
                "gui": {"bundle": "gui/bundle.js"},
            }
        ),
        encoding="utf-8",
    )
    (root / "gui" / "bundle.js").write_text(
        "window.__e2eGuiLoaded = true; window.SabaExtE2eGui = { registerSlots: function () { return {}; } };",
        encoding="utf-8",
    )


def test_gui_asks_before_loading_unverified_extension_gui(sandbox, daemon_factory, open_gui):
    """확인되지 않은 익스텐션의 화면은 묻기 전에 받지 않고, 허락하면 데몬에 기록한 뒤 불러온다."""
    sandbox.write_config({"gui": {"language": "ko"}})
    write_gui_extension(sandbox)
    daemon = daemon_factory()
    daemon.api.ok("POST", "/api/extensions/e2e-gui/enable")
    listed = next(e for e in daemon.api.ok("GET", "/api/extensions")["extensions"] if e["id"] == "e2e-gui")
    assert listed.get("gui_allowed") is False, listed
    assert daemon.api.get("/api/extensions/e2e-gui/gui").status_code == 403

    art = daemon.sb.artifacts / "gui-extConsent"
    gui = open_gui(daemon, "extConsent")
    gui.wait_text(L["ext_gui_title"], timeout=60)
    gui.snap(art, "asked")
    assert not gui.js("return !!window.__e2eGuiLoaded"), "허락 전에 번들이 실행됨"

    gui.click(L["ext_gui_allow"])
    gui.wait(lambda: gui.js("return !!window.__e2eGuiLoaded"), 30, "허락 뒤 번들 로드")
    gui.snap(art, "allowed")
    consents = daemon.api.ok("GET", "/api/sandbox/extension-gui")["components"]
    assert any(c["component"] == "ext-e2e-gui" and c.get("current") for c in consents), consents
