"""L2b 데몬 ↔ 실제 relay-server(임시 PostgreSQL). 디스코드 봇이 하던 일은 relay 하네스의 stdin op로 대신한다."""

import json

import pytest

from harness import real_relay
from harness.daemon import wait_until
from harness.real_relay import RealRelay

pytestmark = pytest.mark.relay_real

DONE = r"Done \([^)]+\)!"
OWNER = "332007306403250178"
GUILD = "123456789012345678"


@pytest.fixture
def relay(sandbox):
    reason = real_relay.available()
    if reason:
        pytest.skip(reason)
    r = RealRelay(sandbox.root / "relay")
    yield r
    r.close()


def _pair_via_daemon(daemon, relay) -> str:
    """페어링하고 hostId를 돌려준다."""
    init = daemon.api.ok("POST", "/api/relay/pair/initiate", {"relayUrl": relay.url})
    claim = relay.claim(init["code"], OWNER, GUILD)
    assert claim.get("ok"), claim
    r = daemon.api.get(f"/api/relay/pair/{init['code']}/status?secret={init['pollSecret']}&relayUrl={relay.url}")
    assert r.status_code == 200 and r.json().get("status") == "claimed", f"{r.status_code} {r.text}"
    return claim["hostId"]


def test_pairing_against_real_relay(sandbox, relay, daemon_factory):
    sandbox.write_config({"relay": {"cloud": {"relayUrl": relay.url}}})
    d = daemon_factory()
    _pair_via_daemon(d, relay)
    assert json.loads((sandbox.data / "config.json").read_text(encoding="utf-8"))["node"]["token"].startswith("sbn_")


@pytest.fixture
def paired(sandbox, relay, daemon_factory):
    """GUI가 하는 그대로 데몬 경유로 페어링한 노드."""
    sandbox.write_config({"relay": {"cloud": {"relayUrl": relay.url}}})
    d = daemon_factory()
    d.host_id = _pair_via_daemon(d, relay)
    d.api.ok("POST", "/api/relay/agent/start")
    wait_until(lambda: d.api.ok("GET", "/api/relay/agent").get("lastHeartbeatAt"), 60, "relay heartbeat")
    return d


def test_agent_heartbeat_marks_host_online(paired, relay):
    body = relay.host(paired.host_id)
    assert body.get("ok") and body["host"]["status"] == "online", body
    assert not paired.api.ok("GET", "/api/relay/agent").get("lastError")


def test_discord_command_roundtrip_through_real_relay(paired, relay, fake_server):
    folder, _ = fake_server("mc1")
    iid = paired.import_server("mc1", folder, module_settings={"ram": "256M"})
    paired.start_instance(iid)
    paired.wait_console(iid, DONE)
    reply = relay.command(GUILD, "mc1 상태", OWNER)
    assert reply.get("ok"), reply
    assert reply["result"]["success"] is True, reply


def test_crash_alert_reaches_real_relay(paired, relay, fake_server):
    folder, _ = fake_server("mc1")
    iid = paired.import_server("mc1", folder, module_settings={"ram": "256M"})
    paired.api.ok("PATCH", f"/api/instance/{iid}", {"auto_restart": False})
    paired.start_instance(iid)
    paired.wait_console(iid, DONE)
    paired.stdin(iid, "e2e-crash")
    # 데몬 /events → relay → 알림 채널. 자동 재시작이 꺼져 있으니 [재시작] 버튼이 붙는다
    notices = wait_until(lambda: relay.notices(), 60, "알림 채널에 크래시 알림")
    assert any("mc1" in n["content"] and n["components"] for n in notices), notices


def test_relay_rejects_agent_after_token_rotation(paired, relay, sandbox):
    """같은 방장이 다시 페어링하면 옛 토큰은 바로 끊긴다(invalidateCacheByNodeId). 에이전트는 교체를 알아채고 멈춘다(#101)."""
    init = paired.api.ok("POST", "/api/relay/pair/initiate", {"relayUrl": relay.url})
    assert relay.claim(init["code"], OWNER, GUILD).get("ok")
    paired.api.ok("POST", "/api/relay/agent/stop")
    paired.api.ok("POST", "/api/relay/agent/start")
    status = wait_until(lambda: (s := paired.api.ok("GET", "/api/relay/agent")).get("lastError") and s, 90, "옛 토큰 거절")
    # relay가 교체를 알리면 lastErrorCode token_replaced로 멈춘다(#101). GUI·CLI는 문구가 아니라 이 코드를 본다.
    # 알림 전에 거절이 먼저 오면 "node token rejected" 또는 "poll 401 … INVALID_TOKEN"
    err = status["lastError"].lower()
    assert status.get("lastErrorCode") == "token_replaced" or "reject" in err or "401" in err, status


def test_owner_discord_setting_runs_without_confirm_button(paired, relay):
    """서버 주인이 직접 친 디스코드 설정 명령(caution 관리 액션)은 확인 버튼 없이 바로 실행된다 (프리뷰 P1, 데몬 #74·relay #44)."""
    reply = relay.action(GUILD, OWNER, {"action": "plan", "id": "discord.settings.set", "input": {"guild": GUILD, "sleeping": True}})
    assert reply.get("ok"), reply
    data = reply["reply"]["data"]
    assert data.get("needsConfirm") is False, f"주인이 직접 친 설정 명령에 확인을 요구함: {reply}"
    ran = relay.action(GUILD, OWNER, {"action": "run_plan", "planId": data["planId"]})
    assert ran.get("ok") and ran["reply"].get("ok"), ran
    settings = paired.api.ok("GET", f"/api/discord/{GUILD}/settings")
    assert settings.get("sleeping") is True or settings.get("settings", {}).get("sleeping") is True, settings


def test_heartbeat_cpu_ram_is_stored_by_real_relay(paired, relay, fake_server):
    """데몬 heartbeat의 서버별 CPU·RAM(#86)이 relay 스키마를 지나 호스트 기록에 남는다 (/사바쨩 상태가 읽는 곳, relay 1605932)."""
    folder, _ = fake_server("mc1")
    iid = paired.import_server("mc1", folder, module_settings={"ram": "256M"})
    paired.start_instance(iid)
    paired.wait_console(iid, DONE)
    paired.api.ok("POST", "/api/relay/agent/stop")
    paired.api.ok("POST", "/api/relay/agent/start")

    def mc1():
        servers = (relay.host(paired.host_id).get("host") or {}).get("metadata", {}).get("servers") or []
        return next((s for s in servers if s.get("name") == "mc1" and s.get("usage")), None)

    entry = wait_until(mc1, 60, "relay 호스트 기록에 mc1의 CPU·RAM")
    usage = entry["usage"]
    assert isinstance(usage.get("cpu"), (int, float)) and usage["cpu"] >= 0, entry
    assert isinstance(usage.get("memMb"), (int, float)) and usage["memMb"] > 0, entry
    assert str(folder) not in str(entry), entry


def test_discord_start_of_unagreed_server_asks_to_agree_on_pc(paired, relay, fake_server):
    """약관에 동의하지 않은 서버를 디스코드에서 켜면 relay가 'PC에서 동의하세요' 안내로 바꿔 답한다 (#83, 데몬 #81)."""
    folder, _ = fake_server("mc1", eula=False)
    iid = paired.import_server("mc1", folder, module_settings={"ram": "256M"}, agree=False)
    call = {"id": "server.start", "instance": iid, "input": {}}
    reply = relay.action(GUILD, OWNER, {"action": "plan", **call})["reply"]
    if reply.get("ok") and reply["data"].get("planId"):
        reply = relay.action(GUILD, OWNER, {"action": "run_plan", **call, "planId": reply["data"]["planId"]})["reply"]
    assert reply.get("ok") is False and reply.get("code") == "agreement_required", reply
    # ko 문구 "PC의 사바쨩에서 …에 동의해야 켤 수 있어요"와 약관 원문 링크
    text = reply.get("text", "")
    assert "PC" in text and "동의" in text and "<https://" in text, reply
    assert paired.server_status(iid) == "stopped"
