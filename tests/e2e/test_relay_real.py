"""L2b 데몬 ↔ 실제 relay-server(임시 PostgreSQL). 디스코드 봇이 하던 일은 relay 하네스의 제어 포트로 대신한다."""

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


def _pair_via_daemon(daemon, relay) -> None:
    init = daemon.api.ok("POST", "/api/relay/pair/initiate", {"relayUrl": relay.url})
    claim = relay.claim(init["code"], OWNER, GUILD)
    assert claim.get("ok"), claim
    r = daemon.api.get(f"/api/relay/pair/{init['code']}/status?secret={init['pollSecret']}&relayUrl={relay.url}")
    assert r.status_code == 200 and r.json().get("status") == "claimed", f"{r.status_code} {r.text}"


def test_pairing_against_real_relay(sandbox, relay, daemon_factory):
    sandbox.write_config({"relay": {"cloud": {"relayUrl": relay.url}}})
    d = daemon_factory()
    _pair_via_daemon(d, relay)
    assert (sandbox.data / ".node_token").read_text().startswith("sbn_")


@pytest.fixture
def paired(sandbox, relay, daemon_factory):
    """GUI가 하는 그대로 데몬 경유로 페어링한 노드."""
    sandbox.write_config({"relay": {"cloud": {"relayUrl": relay.url}}})
    d = daemon_factory()
    _pair_via_daemon(d, relay)
    d.api.ok("POST", "/api/relay/agent/start")
    wait_until(lambda: d.api.ok("GET", "/api/relay/agent").get("lastHeartbeatAt"), 60, "relay heartbeat")
    return d


def test_agent_heartbeat_marks_host_online(paired, relay):
    body = relay.node(GUILD).json()
    assert body["host"]["status"] == "online", body
    assert not paired.api.ok("GET", "/api/relay/agent").get("lastError")


def test_discord_command_roundtrip_through_real_relay(paired, relay, fake_server):
    folder, _ = fake_server("mc1")
    iid = paired.import_server("mc1", folder, module_settings={"ram": "256M"})
    paired.start_instance(iid)
    paired.wait_console(iid, DONE)
    r = relay.submit(GUILD, {"action": "raw_command", "text": "mc1 상태", "userName": "e2e"}, OWNER)
    assert r.status_code == 200, r.text
    rid = r.json()["requestId"]
    row = wait_until(lambda: (c := relay.command(rid)).get("status") in ("completed", "failed") and c, 60, "명령 결과")
    assert row["status"] == "completed", row
    assert row["result"]["success"] is True, row


def test_crash_alert_reaches_real_relay(paired, relay, fake_server):
    folder, _ = fake_server("mc1")
    iid = paired.import_server("mc1", folder, module_settings={"ram": "256M", "auto_restart": False})
    paired.start_instance(iid)
    paired.wait_console(iid, DONE)
    paired.stdin(iid, "e2e-crash")
    posted = wait_until(lambda: [r for r in relay.requests_seen() if r["url"] == "/events"], 60, "relay가 /events를 받음")
    assert posted[0]["status"] == 200, posted
    kinds = [e["type"] for r in posted for e in (r.get("body") or {}).get("events", [])]
    assert "instance.exit" in kinds, posted


def test_relay_rejects_agent_after_token_rotation(paired, relay, sandbox):
    """같은 방장이 다시 페어링하면 옛 토큰은 바로 끊긴다(invalidateCacheByNodeId)."""
    init = paired.api.ok("POST", "/api/relay/pair/initiate", {"relayUrl": relay.url})
    assert relay.claim(init["code"], OWNER, GUILD).get("ok")
    paired.api.ok("POST", "/api/relay/agent/stop")
    paired.api.ok("POST", "/api/relay/agent/start")
    status = wait_until(lambda: (s := paired.api.ok("GET", "/api/relay/agent")).get("lastError") and s, 90, "옛 토큰 거절")
    # heartbeat가 먼저 맞으면 "node token rejected", poll이 먼저면 "poll 401 … INVALID_TOKEN"
    err = status["lastError"].lower()
    assert "reject" in err or "401" in err, status
