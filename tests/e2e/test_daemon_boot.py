"""L1 데몬 기동·인증·단일 인스턴스·종료."""

import signal
import subprocess

import pytest

from harness.daemon import Daemon, wait_until
from harness.paths import IS_WINDOWS, binary


def test_unknown_argument_exits_2_without_touching_data_dir(sandbox):
    before = sorted(sandbox.data.rglob("*"))  # 설치본 구성이면 modules·extensions가 이미 있다
    proc = subprocess.run([str(binary("saba-core")), "--no-such-flag"], env=sandbox.env(), stdin=subprocess.DEVNULL, capture_output=True, timeout=30)
    assert proc.returncode == 2, proc.stderr
    assert sorted(sandbox.data.rglob("*")) == before


def test_version_flag(sandbox):
    proc = subprocess.run([str(binary("saba-core")), "--version"], env=sandbox.env(), stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=30)
    assert proc.returncode == 0
    assert "0." in proc.stdout


def test_health_open_but_api_needs_token(daemon):
    health = daemon.api.get("/health", token=False)
    assert health.status_code == 200
    body = health.json()
    assert body["ok"] is True and body["pid"] == daemon.proc.pid

    assert daemon.api.get("/api/instances", token=False).status_code == 401
    assert daemon.api.get("/api/instances", headers={"X-Saba-Token": "wrong"}).status_code == 401
    assert daemon.api.get("/api/instances").status_code == 200


def test_non_loopback_host_header_rejected(daemon):
    """DNS 리바인딩 방어: Host가 루프백이 아니면 토큰이 맞아도 403."""
    r = daemon.api.get("/api/instances", headers={"Host": "evil.example"})
    assert r.status_code == 403
    assert daemon.api.get("/health", token=False, headers={"Host": "evil.example"}).status_code == 403


def test_token_file_is_private(daemon):
    if IS_WINDOWS:
        pytest.skip("Unix 권한 비트만 본다")
    mode = daemon.sb.token_path.stat().st_mode & 0o777
    assert mode == 0o600, oct(mode)


def test_second_daemon_on_same_data_dir_exits_2_and_keeps_token(daemon_factory, sandbox):
    first = daemon_factory()
    token = sandbox.token()
    second = Daemon(sandbox)
    proc = second.spawn()
    assert proc.wait(30) == 2
    assert sandbox.token() == token
    assert first.api.get("/api/instances").status_code == 200


def test_shutdown_api_ends_process(daemon):
    code = daemon.shutdown()
    assert code == 0
    assert not daemon.alive()


@pytest.mark.skipif(IS_WINDOWS, reason="SIGTERM은 Unix만")
def test_sigterm_ends_process(daemon):
    daemon.proc.send_signal(signal.SIGTERM)
    assert daemon.proc.wait(30) == 0


def test_daemon_restart_on_same_data_dir_keeps_instances(daemon_factory, fake_server):
    d1 = daemon_factory()
    folder, _ = fake_server("keep")
    iid = d1.import_server("keep", folder)
    d1.shutdown()
    d2 = daemon_factory()
    assert [i["id"] for i in d2.instances()] == [iid]


def test_modules_and_extensions_from_repos_are_discovered(daemon):
    names = {m["name"] for m in daemon.api.ok("GET", "/api/modules")["modules"]}
    assert {"minecraft", "palworld", "zomboid"} <= names
    exts = daemon.api.ok("GET", "/api/extensions")
    ids = {e.get("id") for e in (exts.get("extensions", exts) if isinstance(exts, dict) else exts)}
    assert {"steamcmd", "ue4-ini"} <= ids


def test_daemon_log_has_no_panics(daemon):
    daemon.api.get("/api/instances")
    wait_until(lambda: "IPC listening" in daemon.log_text(), 10, "IPC listening 로그")
    assert "panicked" not in daemon.log_text()
