"""L1 게임 서버 수명: 가져오기 → EULA → 시작 → 명령 → 정지, 크래시·자동 재시작, 멈춤, 동시 실행, 데몬 재시작 뒤 재연결.

가짜 마인크래프트 서버(fixtures/fake-minecraft)를 실제 minecraft 모듈·saba-runner로 돌린다.
"""

import socket
import time

import pytest

from harness import procs
from harness.daemon import E2EError, EventStream, wait_until

DONE = r"Done \([^)]+\)!"


def _listening(port: int) -> bool:
    with socket.socket() as s:
        s.settimeout(1)
        return s.connect_ex(("127.0.0.1", port)) == 0


def test_import_detects_folder_and_port(mc):
    daemon, iid, folder, port = mc
    inst = daemon.instance(iid)
    assert inst["module_name"] == "minecraft"
    assert inst["working_dir"] == str(folder)
    assert inst["port"] == port
    assert daemon.server_status(iid) == "stopped"


def test_eula_not_accepted_blocks_start_until_accepted(daemon, fake_server):
    folder, _ = fake_server("noeula", eula=False)
    iid = daemon.import_server("noeula", folder, module_settings={"ram": "256M"})
    r = daemon.api.post(f"/api/instance/{iid}/start")
    assert r.status_code == 409, r.text
    body = r.json()
    assert body["error_code"] == "agreement_required" and body["agreements"][0]["id"] == "eula"
    assert body["agreements"][0]["url"].startswith("https://")
    assert daemon.server_status(iid) == "stopped"

    daemon.api.ok("POST", f"/api/instance/{iid}/agreements", {"accept": ["eula"]})
    assert "eula=true" in (folder / "eula.txt").read_text()
    daemon.start_instance(iid)
    daemon.wait_console(iid, DONE)


def test_start_command_stop_roundtrip(mc):
    daemon, iid, folder, port = mc
    daemon.start_instance(iid)
    daemon.wait_status(iid, "running")
    daemon.wait_console(iid, DONE)
    wait_until(lambda: _listening(port), 20, f"게임 포트 {port} 열림")

    reply = daemon.stdin(iid, "list")
    assert "players online" in reply["data"]["response"], reply

    daemon.mark_console(iid)
    daemon.stdin(iid, "say 안녕 사바쨩")
    daemon.wait_console(iid, r"\[Server\] 안녕 사바쨩")

    daemon.stop_instance(iid)
    daemon.wait_status(iid, "stopped")
    assert not _listening(port)
    assert "saved" in (folder / "world" / "level.dat").read_text()
    kinds = [e["kind"] for e in daemon.exit_events(iid)]
    assert kinds and kinds[-1] in ("user_stop", "in_game_stop"), kinds
    assert not procs.processes_mentioning(str(folder)), "정지 뒤 서버 프로세스가 남음"


def test_module_command_route_runs_console_command(mc):
    daemon, iid, *_ = mc
    daemon.start_instance(iid)
    daemon.wait_console(iid, DONE)
    daemon.mark_console(iid)
    r = daemon.api.post(f"/api/instance/{iid}/command", {"command": "say", "args": {"message": "via-command"}})
    assert r.status_code == 200, r.text
    daemon.wait_console(iid, r"via-command")


def test_command_to_stopped_server_is_409(mc):
    daemon, iid, *_ = mc
    r = daemon.api.post(f"/api/instance/{iid}/command", {"command": "list"})
    assert r.status_code == 409, r.text


def test_crash_is_detected_and_auto_restarted(mc):
    daemon, iid, *_ = mc
    events = EventStream(daemon)
    try:
        daemon.start_instance(iid)
        daemon.wait_console(iid, DONE)
        daemon.mark_console(iid)
        daemon.stdin(iid, "e2e-crash")
        ev = events.wait_for("instance.exit", lambda d: d.get("instanceId") == iid, timeout=30)
        assert ev["data"]["kind"] == "crash", ev
        history = daemon.exit_events(iid)
        assert history[-1]["kind"] == "crash" and history[-1]["restart_in_secs"] == 5, history[-1]
        # 첫 재시작 대기 5초
        daemon.wait_console(iid, DONE, timeout=40)
        daemon.wait_status(iid, "running")
    finally:
        events.close()


def test_crash_with_auto_restart_off_stays_stopped(mc):
    daemon, iid, *_ = mc
    daemon.api.ok("PATCH", f"/api/instance/{iid}", {"auto_restart": False})
    daemon.start_instance(iid)
    daemon.wait_console(iid, DONE)
    daemon.stdin(iid, "e2e-crash")
    daemon.wait_status(iid, "stopped", timeout=30)
    last = wait_until(lambda: (daemon.exit_events(iid) or [None])[-1], 20, "종료 기록")
    assert last["kind"] == "crash" and last["restart_in_secs"] is None
    assert last["note"] == "auto_restart is off"
    time.sleep(8)
    assert daemon.server_status(iid) == "stopped"


@pytest.mark.slow
def test_crash_loop_gives_up_after_limit(mc):
    """10분 안 3회 재시작 뒤에는 포기한다 (5s, 10s, 20s)."""
    daemon, iid, *_ = mc
    daemon.start_instance(iid)
    for attempt in range(4):
        daemon.wait_console(iid, DONE, timeout=60)
        daemon.mark_console(iid)
        daemon.stdin(iid, "e2e-crash")
        wait_until(lambda: len(daemon.exit_events(iid)) == attempt + 1, 30, f"{attempt + 1}번째 종료 기록")
    last = daemon.exit_events(iid)[-1]
    assert last["kind"] == "crash" and last["restart_in_secs"] is None and last["note"], last
    daemon.wait_status(iid, "stopped")


def test_nonzero_exit_without_crash_report(mc):
    daemon, iid, *_ = mc
    daemon.api.ok("PATCH", f"/api/instance/{iid}", {"auto_restart": False})
    daemon.start_instance(iid)
    daemon.wait_console(iid, DONE)
    daemon.stdin(iid, "e2e-exit 3")
    daemon.wait_status(iid, "stopped", timeout=30)
    last = wait_until(lambda: (daemon.exit_events(iid) or [None])[-1], 20, "종료 기록")
    assert last["exit_code"] == 3, last
    assert last["kind"] in ("crash", "unknown_exit"), last


def test_hung_server_is_force_stopped(mc):
    daemon, iid, folder, port = mc
    daemon.api.ok("PATCH", f"/api/instance/{iid}", {"stop_timeout": 5})
    daemon.start_instance(iid)
    daemon.wait_console(iid, DONE)
    daemon.stdin(iid, "e2e-hang")
    daemon.wait_console(iid, "e2e-hang on")
    t0 = time.monotonic()
    daemon.stop_instance(iid, timeout=120)
    daemon.wait_status(iid, "stopped", timeout=90)
    elapsed = time.monotonic() - t0
    assert not procs.processes_mentioning(str(folder)), "강제 정지 뒤 프로세스가 남음"
    assert not _listening(port)
    assert elapsed < 60, f"멈춘 서버 정지에 {elapsed:.0f}초"


def test_force_stop_kills_immediately(mc):
    daemon, iid, folder, _ = mc
    daemon.start_instance(iid)
    daemon.wait_console(iid, DONE)
    daemon.stdin(iid, "e2e-hang")
    t0 = time.monotonic()
    daemon.stop_instance(iid, force=True)
    daemon.wait_status(iid, "stopped", timeout=20)
    assert time.monotonic() - t0 < 15
    assert not procs.processes_mentioning(str(folder))


def test_two_instances_run_side_by_side(daemon, fake_server):
    (f1, p1), (f2, p2) = fake_server("a"), fake_server("b")
    a = daemon.import_server("a", f1, module_settings={"ram": "256M"})
    b = daemon.import_server("b", f2, module_settings={"ram": "256M"})
    daemon.start_instance(a)
    daemon.start_instance(b)
    daemon.wait_console(a, DONE)
    daemon.wait_console(b, DONE)
    assert _listening(p1) and _listening(p2)
    assert "max of 7" in daemon.stdin(a, "list")["data"]["response"]
    daemon.stop_instance(a)
    daemon.wait_status(a, "stopped")
    assert daemon.server_status(b) == "running"
    assert "players online" in daemon.stdin(b, "list")["data"]["response"]


def test_port_conflict_between_instances_is_refused(daemon, fake_server):
    (f1, p1), (f2, _) = fake_server("a"), fake_server("b")
    a = daemon.import_server("a", f1, module_settings={"ram": "256M"})
    b = daemon.import_server("b", f2, module_settings={"ram": "256M"})
    patched = daemon.api.patch(f"/api/instance/{b}", {"port": p1})
    if patched.status_code >= 400:
        return  # 설정 단계에서 막으면 그것으로 충분하다
    daemon.start_instance(a)
    daemon.wait_console(a, DONE)
    r = daemon.api.post(f"/api/instance/{b}/start")
    # 시작 실패를 HTTP 200 + success:false로 돌려준다 (실패 상태 코드가 섞여 있다: 01 §2.1)
    body = r.json()
    assert body.get("success") is False and body.get("error_code") == "port_conflict", r.text
    assert daemon.server_status(b) == "stopped"
    assert daemon.server_status(a) == "running"


@pytest.mark.xfail(strict=True, reason="E2E-F4: 다른 프로그램이 포트를 쓰면 시작이 500 'Process exited with code 1'로만 끝나고 포트 이유를 말하지 않음")
def test_port_used_by_foreign_process_is_reported(mc):
    daemon, iid, _, port = mc
    with socket.socket() as squatter:
        squatter.bind(("0.0.0.0", port))
        squatter.listen()
        r = daemon.api.post(f"/api/instance/{iid}/start")
        daemon.wait_status(iid, "stopped", timeout=30)
    assert r.status_code in (409, 400), r.text
    assert "PORT_IN_USE" in r.text or str(port) in r.text, r.text


def test_server_survives_daemon_restart_and_console_reattaches(daemon_factory, fake_server):
    d1 = daemon_factory()
    folder, port = fake_server("keep")
    iid = d1.import_server("keep", folder, module_settings={"ram": "256M"})
    d1.start_instance(iid)
    d1.wait_console(iid, DONE)
    java = [pid for pid, cmd in procs.processes_mentioning(str(folder)) if "java" in cmd]
    assert java, "서버 프로세스를 못 찾음"
    d1.shutdown()

    assert all(procs.pid_alive(p) for p in java), "데몬이 꺼질 때 게임 서버가 같이 꺼짐"
    assert _listening(port)

    d2 = daemon_factory()
    d2.wait_status(iid, "running", timeout=30)
    reply = d2.stdin(iid, "list")
    assert "players online" in reply["data"]["response"], f"재연결 뒤 명령 응답이 안 옴: {reply}"
    d2.stop_instance(iid)
    d2.wait_status(iid, "stopped")
    assert not any(procs.pid_alive(p) for p in java)


def test_server_dies_while_daemon_is_down(daemon_factory, fake_server):
    d1 = daemon_factory()
    folder, _ = fake_server("gone")
    iid = d1.import_server("gone", folder, module_settings={"ram": "256M"})
    d1.start_instance(iid)
    d1.wait_console(iid, DONE)
    java = [pid for pid, cmd in procs.processes_mentioning(str(folder)) if "java" in cmd]
    d1.shutdown()
    for pid in java:
        procs.kill_pid(pid)
    wait_until(lambda: not any(procs.pid_alive(p) for p in java), 10, "서버 종료")
    d2 = daemon_factory()
    d2.wait_status(iid, "stopped", timeout=30)
    d2.start_instance(iid)
    d2.wait_console(iid, DONE)


def test_console_flood_keeps_api_responsive(mc):
    daemon, iid, *_ = mc
    daemon.start_instance(iid)
    daemon.wait_console(iid, DONE)
    daemon.stdin(iid, "e2e-spam 50000")
    worst = 0.0
    for _ in range(20):
        t0 = time.monotonic()
        assert daemon.api.get("/api/servers").status_code == 200
        worst = max(worst, time.monotonic() - t0)
        time.sleep(0.1)
    daemon.wait_console(iid, "e2e-spam 49999", timeout=60)
    assert worst < 2.0, f"콘솔 폭주 중 /api/servers 최악 {worst:.2f}초"


def test_delete_instance_keeps_imported_folder(mc):
    daemon, iid, folder, _ = mc
    r = daemon.api.delete(f"/api/instance/{iid}")
    assert r.status_code == 200, r.text
    assert daemon.instances() == []
    assert (folder / "server.jar").exists(), "가져온 사용자 폴더가 지워짐"


def test_rename_and_reorder(daemon, fake_server):
    ids = [daemon.import_server(n, fake_server(n)[0]) for n in ("one", "two")]
    daemon.api.ok("POST", f"/api/instance/{ids[0]}/rename", {"name": "첫째"})
    assert daemon.instance(ids[0])["name"] == "첫째"
    r = daemon.api.post(f"/api/instance/{ids[1]}/rename", {"name": "첫째"})
    assert r.status_code in (400, 409), f"같은 이름으로 바꾸기가 허용됨: {r.text}"
    daemon.api.ok("PUT", "/api/instances/reorder", {"order": [ids[1], ids[0]]})
    assert [i["id"] for i in daemon.instances()] == [ids[1], ids[0]]


def test_launch_preview_matches_actual_command(mc):
    daemon, iid, folder, _ = mc
    daemon.api.ok("PATCH", f"/api/instance/{iid}", {"ram": "300M"})
    preview = daemon.api.ok("POST", f"/api/instance/{iid}/launch-preview", {})
    text = str(preview)
    assert "-Xmx300M" in text, preview
    daemon.start_instance(iid)
    daemon.wait_console(iid, DONE)
    java = [cmd for _, cmd in procs.processes_mentioning(str(folder)) if "java" in cmd]
    assert java and "-Xmx300M" in java[0], java


def _ensure_error_text(exc: E2EError) -> str:
    return str(exc)


def test_console_input_to_rest_only_server_is_refused(daemon, sandbox):
    """콘솔 입력을 받지 않는 모듈(Palworld, REST 전용)에 /stdin을 보내면 보낸 척하지 않고 409 (프리뷰 P1, #74).
    GUI는 이 답을 보고 명령 API로 넘어간다."""
    folder = sandbox.servers / "pal"
    folder.mkdir()
    for ext in ("steamcmd", "ue4-ini"):
        daemon.api.ok("POST", f"/api/extensions/{ext}/enable")
    iid = daemon.api.ok("POST", "/api/instances", {"name": "pal", "module_name": "palworld", "working_dir": str(folder)})["id"]
    r = daemon.api.post(f"/api/instance/{iid}/stdin", {"command": "say hi"})
    assert r.status_code == 409 and r.json().get("error_code") == "stdin_not_supported", f"{r.status_code} {r.text}"
