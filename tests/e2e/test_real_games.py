"""L5 실기: 진짜 게임 서버를 진짜로 받아서 켜고 끈다. 외부망과 시간이 든다 (PC: run-pc.ps1 -Network / -Steam).

- network: 마인크래프트 바닐라·Paper·Fabric (Mojang·PaperMC·Fabric, 필요하면 Adoptium JRE)
- steam: Palworld(약 5.5GB)·Project Zomboid(약 3GB) — SteamCMD 익명 로그인
받은 파일은 모두 시험 폴더 안(인스턴스 폴더, HOME 아래 SteamCMD)에 남고 실행이 끝나면 함께 지울 수 있다.
"""

import time

import pytest

from harness.daemon import Daemon, wait_until

pytestmark = [pytest.mark.network, pytest.mark.slow]

MC_DONE = r"Done \([^)]+\)!"


def _provision(daemon: Daemon, name: str, module: str, port: int, timeout: float, **settings) -> str:
    # 사람이 약관(EULA 등)을 읽고 체크하는 대신 만들 때 동의한다
    body = {"name": name, "module_name": module, "port": port, "module_settings": settings, "accept_agreements": daemon.agreement_ids(module)}
    iid = daemon.api.ok("POST", "/api/instances", body, timeout=60)["id"]
    last: dict = {}

    def finished():
        nonlocal last
        p = daemon.api.ok("GET", f"/api/provision-progress/{name}")
        if p.get("active"):
            last = p
            if p.get("error"):
                raise AssertionError(f"{name} 설치 실패: {p}")
            return p.get("done")
        return daemon.instance(iid).get("executable_path") and daemon.server_status(iid) != "provisioning"

    wait_until(finished, timeout, f"{name} 설치 ({last.get('message')})", interval=2)
    return iid


@pytest.mark.parametrize("server_type", ["vanilla", "paper", "fabric"])
def test_minecraft_real_install_run_stop(daemon, game_ports, server_type):
    iid = _provision(daemon, f"mc-{server_type}", "minecraft", game_ports[0], 1800, server_type=server_type, ram="1G")
    daemon.start_instance(iid)
    # 첫 실행은 월드를 만들고, Fabric은 바닐라 jar와 라이브러리를 더 받는다
    daemon.wait_console(iid, MC_DONE, timeout=600)
    reply = daemon.stdin(iid, "list")
    assert "players online" in reply["data"]["response"], reply
    daemon.stop_instance(iid, timeout=180)
    daemon.wait_status(iid, "stopped", timeout=180)
    last = daemon.exit_events(iid)[-1]
    assert last["kind"] in ("user_stop", "in_game_stop"), last


def test_minecraft_real_update_check(daemon, game_ports):
    iid = _provision(daemon, "mc-upd", "minecraft", game_ports[1], 1800, ram="1G")
    r = daemon.api.get(f"/api/instance/{iid}/check-update", timeout=120)
    assert r.status_code == 200, r.text


def _enable_extensions(daemon: Daemon, *ids: str) -> None:
    for ext in ids:
        r = daemon.api.post(f"/api/extensions/{ext}/enable", timeout=300)
        assert r.status_code == 200, r.text


@pytest.mark.steam
def test_palworld_real_install_run_stop(daemon, game_ports):
    _enable_extensions(daemon, "steamcmd", "ue4-ini")
    iid = _provision(daemon, "pal", "palworld", game_ports[2], 7200)
    daemon.start_instance(iid)
    # Palworld는 stdin 콘솔이 없다: REST가 답하면 떠 있는 것
    wait_until(lambda: daemon.api.post(f"/api/instance/{iid}/rest", {"endpoint": "/v1/api/info"}, timeout=10).status_code == 200, 600, "Palworld REST 응답", interval=5)
    daemon.stop_instance(iid, timeout=180)
    daemon.wait_status(iid, "stopped", timeout=180)


@pytest.mark.steam
def test_zomboid_real_install_run_stop(daemon, game_ports):
    _enable_extensions(daemon, "steamcmd")
    iid = _provision(daemon, "pz", "zomboid", game_ports[3], 7200, admin_password="e2e-admin-pass")
    daemon.start_instance(iid)
    daemon.wait_console(iid, r"SERVER STARTED|Server Started", timeout=900)
    daemon.stop_instance(iid, timeout=300)
    daemon.wait_status(iid, "stopped", timeout=300)
    time.sleep(1)
