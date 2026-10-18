"""L4 업데이트 확인·내려받기: 서명 검증, 변조·다운그레이드·해시 불일치 거부.

`--features mock-relay`로 빌드한 데몬이 필요하다 (공개 시험 키를 믿는 빌드). 위치: SABA_E2E_MOCK_CORE,
기본 target/e2e-mock/debug/saba-core. 적용(apply)은 업데이터 GUI를 띄우므로 PC 층(run-pc.ps1)에서 본다.
"""

import os
import platform
import shutil
from pathlib import Path

import pytest

from harness.daemon import wait_until
from harness.fake_updates import FakeUpdates
from harness.paths import EXE, REPO

pytestmark = pytest.mark.mock_relay

MOCK_CORE = Path(os.environ.get("SABA_E2E_MOCK_CORE", REPO / "target" / "e2e-mock" / "debug" / f"saba-core{EXE}"))


def _platform() -> str:
    os_name = {"win32": "windows", "darwin": "macos"}.get(os.sys.platform, "linux")
    arch = {"x86_64": "x64", "AMD64": "x64", "aarch64": "arm64", "arm64": "arm64"}.get(platform.machine(), platform.machine())
    return f"{os_name}-{arch}"


@pytest.fixture
def updates():
    if not MOCK_CORE.exists():
        pytest.skip(f"mock-relay 빌드가 없습니다: {MOCK_CORE} (CARGO_TARGET_DIR=target/e2e-mock cargo build -p saba-core --features mock-relay)")
    u = FakeUpdates(_platform())
    u.add_component("saba-core", "0.1.6", {f"saba-core{EXE}": b"new core"})
    u.add_component("cli", "0.1.6", {f"saba-chan-cli{EXE}": b"new cli"})
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
    assert core["update_available"] and core["latest_version"] == "0.1.6", body
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
