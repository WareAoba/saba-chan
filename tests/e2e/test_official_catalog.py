"""L5 공식 카탈로그: 사용자가 받는 길 그대로. 운영 릴리스의 서명된 카탈로그에서 모듈·익스텐션을 받으면
신뢰 표시가 verified이고, 격리(AppContainer·Defender 검사) 없이 진짜 게임이 돈다 (PC: run-pc.ps1 -Network / -Steam).

test_real_games.py는 저장소 복사본을 쓰므로 unverified라 Windows에서 격리 상태로 돈다. 이 파일은 그 반대쪽이다.
두 파일을 함께 돌려야 "공식 모듈은 그대로, 확인되지 않은 모듈은 격리해서" 둘 다 실제로 도는지 본다.
카탈로그 주소는 데몬 기본값(운영 relay)이라 받기만 한다. 받은 파일은 시험 폴더 안에만 남는다.
"""

import shutil

import pytest

from harness.daemon import Daemon, wait_until
from test_real_games import MC_DONE, _enable_extensions, _provision

pytestmark = [pytest.mark.network, pytest.mark.slow]


@pytest.fixture
def official(sandbox, daemon_factory) -> Daemon:
    """저장소 복사본을 비우고 데몬을 띄운다 (공식 카탈로그에서 받은 것만 남게)"""
    for folder in (sandbox.modules, sandbox.extensions):
        shutil.rmtree(folder)
        folder.mkdir(parents=True)
    return daemon_factory()


def _install_module(daemon: Daemon, module: str) -> None:
    r = daemon.api.post(f"/api/modules/manifest/{module}/install", timeout=300)
    assert r.status_code == 200 and r.json().get("ok"), f"{module} 카탈로그 설치 실패: {r.text[:500]}"


def _install_extension(daemon: Daemon, ext: str) -> None:
    r = daemon.api.post(f"/api/extensions/{ext}/install", timeout=300)
    assert r.status_code == 200 and r.json().get("success"), f"{ext} 카탈로그 설치 실패: {r.text[:500]}"


def _trust(daemon: Daemon, kind: str) -> dict[str, str]:
    key = "name" if kind == "modules" else "id"
    return {m[key]: m.get("trust") for m in daemon.api.ok("GET", f"/api/{kind}")[kind]}


def test_catalog_modules_and_extensions_are_verified(official):
    for module in ("minecraft", "palworld"):
        _install_module(official, module)
    for ext in ("steamcmd", "ue4-ini"):
        _install_extension(official, ext)
    assert _trust(official, "modules") == {"minecraft": "verified", "palworld": "verified"}
    exts = _trust(official, "extensions")
    assert {k: exts.get(k) for k in ("steamcmd", "ue4-ini")} == {"steamcmd": "verified", "ue4-ini": "verified"}, exts


def test_catalog_minecraft_runs_with_daemon_found_java(official, game_ports):
    """사용자처럼 java 경로를 주지 않는다: 모듈·데몬이 찾거나 받은 Java로 떠야 한다"""
    _install_module(official, "minecraft")
    assert _trust(official, "modules")["minecraft"] == "verified"  # 격리 없이 도는 상태에서 본다
    iid = _provision(official, "mc-official", "minecraft", game_ports[0], 1800, server_type="vanilla", ram="1G")
    official.start_instance(iid)
    official.wait_console(iid, MC_DONE, timeout=600)
    official.stop_instance(iid, timeout=180)
    official.wait_status(iid, "stopped", timeout=180)


@pytest.mark.steam
def test_catalog_palworld_installs_through_steamcmd(official, game_ports):
    _install_module(official, "palworld")
    _install_extension(official, "steamcmd")
    _install_extension(official, "ue4-ini")
    _enable_extensions(official, "steamcmd", "ue4-ini")
    assert _trust(official, "modules")["palworld"] == "verified"
    exts = _trust(official, "extensions")
    assert exts.get("steamcmd") == "verified" and exts.get("ue4-ini") == "verified", exts
    iid = _provision(official, "pal-official", "palworld", game_ports[2], 7200)
    official.start_instance(iid)
    wait_until(lambda: official.api.post(f"/api/instance/{iid}/rest", {"endpoint": "/v1/api/info"}, timeout=10).status_code == 200, 600, "Palworld REST 응답", interval=5)
    official.stop_instance(iid, timeout=180)
    official.wait_status(iid, "stopped", timeout=180)
