"""운영체제 기능: 메모리 상한(Job Object), 격리 실행(AppContainer), 시스템 모니터, 터미널로 열기.

Windows 전용 시험은 `windows` 표시가 있고 리눅스에서는 건너뛴다. 리눅스에서는 상한이 적용되지 않는다는 안내가 남는지만 본다.
"""

import json
import time

import pytest

from harness import procs
from harness.daemon import wait_until
from harness.paths import IS_WINDOWS

DONE = r"Done \([^)]+\)!"


def _session_log_text(folder_or_data) -> str:
    return "\n".join(p.read_text(encoding="utf-8", errors="replace") for p in folder_or_data.rglob("logs/sessions/*.log"))


@pytest.mark.windows
def test_memory_limit_stops_server_over_cap(mc):
    """릴리스 체크리스트 5: 상한보다 큰 힙을 잡으면 러너가 끊고 crash로 판정한다.

    마인크래프트 모듈은 -Xms를 -Xmx와 같게 주므로 힙 1G는 시작하자마자 커밋되어 512 MiB 상한에 걸린다.
    그래서 시작 요청 자체가 실패할 수도, 켜진 뒤 곧 끝날 수도 있다. 어느 쪽이든 상한 때문에 끝났다고 남아야 한다.
    """
    daemon, iid, *_ = mc
    daemon.api.ok("PATCH", f"/api/instance/{iid}", {"ram": "1G", "memory_limit_mb": 512, "auto_restart": False})
    r = daemon.api.post(f"/api/instance/{iid}/start")
    started = r.ok and r.json().get("success") is not False
    if started:
        # 가짜 서버가 힙을 실제로 채운다 (Xms만으로는 커밋이 다 일어나지 않을 수 있다)
        try:
            daemon.wait_console(iid, DONE, timeout=60)
            daemon.stdin(iid, "e2e-alloc 900")
        except Exception:
            pass  # 이미 상한에 걸려 끝났을 수 있다
    daemon.wait_status(iid, "stopped", timeout=60)
    text = daemon.console_text(iid) + _session_log_text(daemon.sb.data)
    assert "Memory limit of 512 MiB reached" in text, text[-2000:]
    if started:
        last = daemon.exit_events(iid)[-1]
        assert last["kind"] == "crash", last


@pytest.mark.windows
def test_memory_limit_hit_during_start_is_named_in_start_error(mc):
    """시작하자마자 상한에 걸려 실패하면, 시작 오류가 종료 코드가 아니라 메모리 상한 때문이라고 알리고 종료 기록을 남겨야 한다."""
    daemon, iid, *_ = mc
    daemon.api.ok("PATCH", f"/api/instance/{iid}", {"ram": "1G", "memory_limit_mb": 512, "auto_restart": False})
    r = daemon.api.post(f"/api/instance/{iid}/start")
    if r.ok and r.json().get("success") is not False:
        pytest.skip("이 PC에서는 시작이 먼저 끝나 상한이 나중에 걸림")
    body = r.json() if r.content else {}
    assert body.get("error_code") == "memory_limit_reached", r.text
    error = str(body.get("error", ""))
    assert "memory" in error.lower() or "메모리" in error, r.text
    # 시작 중에 끝난 것도 종료 기록이 남아야 한다 (시작 실패 또는 crash)
    daemon.wait_status(iid, "stopped", timeout=60)
    events = daemon.exit_events(iid)
    assert events and events[-1]["kind"] in ("start_failed", "crash"), events


@pytest.mark.windows
def test_memory_limit_allows_server_under_cap(mc):
    daemon, iid, *_ = mc
    daemon.api.ok("PATCH", f"/api/instance/{iid}", {"ram": "256M", "memory_limit_mb": 1536})
    daemon.start_instance(iid)
    daemon.wait_console(iid, DONE)
    daemon.stdin(iid, "e2e-alloc 150")
    daemon.wait_console(iid, "e2e-alloc holding 150 MiB")
    time.sleep(3)
    assert daemon.server_status(iid) == "running"


@pytest.mark.skipif(IS_WINDOWS, reason="리눅스 안내 문구")
def test_memory_limit_is_reported_as_not_enforced_off_windows(mc):
    daemon, iid, *_ = mc
    daemon.api.ok("PATCH", f"/api/instance/{iid}", {"memory_limit_mb": 512})
    daemon.start_instance(iid)
    daemon.wait_console(iid, DONE)
    text = daemon.console_text(iid) + _session_log_text(daemon.sb.data) + daemon.log_text()
    assert "only enforced on Windows" in text, "리눅스에서 상한이 적용되지 않는다는 안내가 없음"


def test_memory_limit_rejects_tiny_or_malformed_values(mc):
    daemon, iid, *_ = mc
    for bad in (100, "abc", -1):
        r = daemon.api.patch(f"/api/instance/{iid}", {"memory_limit_mb": bad})
        assert r.status_code == 400, (bad, r.text)


def test_metrics_include_running_server_share(mc):
    daemon, iid, *_ = mc
    daemon.start_instance(iid)
    daemon.wait_console(iid, DONE)
    time.sleep(2)
    snap = wait_until(lambda: (s := daemon.api.ok("GET", "/api/system/metrics")) and iid in json.dumps(s) and s, 30, "모니터에 서버 몫")
    text = json.dumps(snap)
    assert '"memory' in text or "mem" in text, snap
    if IS_WINDOWS:
        # PDH 디스크 IO (#26): 값이 붙어 나오는지
        assert "disk" in text.lower(), snap


@pytest.mark.windows
def test_open_in_terminal_spawns_cli_console(mc):
    daemon, iid, *_ = mc
    r = daemon.api.ok("POST", "/api/clients/launch", {"kind": "cli", "view": "console", "instance": iid})
    pid = r["pid"]
    try:
        assert procs.pid_alive(pid), "터미널 창의 CLI가 바로 끝남"
    finally:
        procs.kill_pid(pid)


def test_open_in_terminal_input_validation(mc):
    daemon, iid, *_ = mc
    assert daemon.api.post("/api/clients/launch", {"kind": "gui", "view": "shell"}).status_code == 400
    assert daemon.api.post("/api/clients/launch", {"kind": "cli", "view": "console"}).status_code == 400
    assert daemon.api.post("/api/clients/launch", {"kind": "cli", "view": "console", "instance": "nope"}).status_code == 404
    if not IS_WINDOWS:
        assert daemon.api.post("/api/clients/launch", {"kind": "cli", "view": "shell"}).status_code == 501


ESCAPE_TOML = """[module]
name = "e2e-escape"
version = "0.0.1"
description = "E2E: 격리 실행이 바깥 쓰기를 막는지"
author = "e2e"
game_name = "Escape"
display_name = "E2E Escape"
entry = "lifecycle.py"

[protocols]
supported = ["stdin"]
primary = "stdin"
"""

ESCAPE_PY = r'''
import json, os, sys

def list_versions(config):
    target = os.environ.get("E2E_ESCAPE_TARGET") or config.get("escape_target")
    results = {}
    for name, path in (("outside", target), ("home", os.path.join(os.path.expanduser("~"), "saba-e2e-escape.txt"))):
        try:
            with open(path, "w") as f:
                f.write("escaped")
            results[name] = "written"
        except Exception as e:
            results[name] = f"denied: {type(e).__name__}"
    return {"success": True, "versions": [], "escape": results}

FUNCTIONS = {"list_versions": list_versions}

if __name__ == "__main__":
    fn = sys.argv[1]
    config = json.loads(sys.stdin.read() or "{}")
    print(json.dumps(FUNCTIONS[fn](config)))
'''


@pytest.mark.windows
def test_unverified_module_cannot_write_outside_its_folders(sandbox, daemon_factory):
    """미검증 모듈은 AppContainer 안에서 돈다: 데이터 폴더 밖·사용자 폴더에 쓰지 못한다."""
    mod = sandbox.modules / "e2e-escape"
    mod.mkdir()
    (mod / "module.toml").write_text(ESCAPE_TOML, encoding="utf-8")
    (mod / "lifecycle.py").write_text(ESCAPE_PY, encoding="utf-8")
    target = sandbox.root / "escape-target.txt"
    d = daemon_factory(extra_env={"E2E_ESCAPE_TARGET": str(target)})
    r = d.api.get("/api/module/e2e-escape/versions", timeout=120)
    body = r.json()
    assert not target.exists(), f"격리된 모듈이 바깥에 씀: {body}"
    assert "escape" not in json.dumps(body) or "written" not in json.dumps(body.get("escape", {})), body
