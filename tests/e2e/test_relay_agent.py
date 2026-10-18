"""L2a 데몬 릴레이 에이전트 ↔ 가짜 릴레이: 서명, heartbeat, 명령 왕복, 알림 이벤트, 버전 하한, 페어링."""

import json

import pytest

from harness.daemon import wait_until
from harness.fake_relay import FakeRelay

DONE = r"Done \([^)]+\)!"


@pytest.fixture
def relay():
    r = FakeRelay()
    yield r
    r.close()


@pytest.fixture
def paired(sandbox, relay, daemon_factory):
    """릴레이 주소와 노드 토큰을 미리 넣고 데몬을 띄운다 (페어링을 마친 노드)."""
    sandbox.write_config({"relay": {"cloud": {"relayUrl": relay.url}}})
    (sandbox.data / ".node_token").write_text(relay.node_token)
    d = daemon_factory()
    wait_until(lambda: relay.heartbeats, 30, "첫 heartbeat")
    return d


def test_heartbeat_is_signed_and_carries_metadata(paired, relay, fake_server):
    folder, _ = fake_server("mc1")
    iid = paired.import_server("mc1", folder, module_settings={"ram": "256M"})
    assert relay.signature_failures == []
    hb = relay.heartbeats[0]
    assert hb["agentVersion"] and hb["os"]
    meta = hb["metadata"]
    assert {m["name"] for m in meta["modules"]} >= {"minecraft"}
    # 새 서버는 다음 heartbeat에 실린다: 에이전트를 다시 시작해서 바로 보내게 한다
    paired.api.ok("POST", "/api/relay/agent/stop")
    paired.api.ok("POST", "/api/relay/agent/start")
    wait_until(lambda: any(s.get("id") == iid or s.get("name") == "mc1" for s in relay.heartbeats[-1]["metadata"]["servers"]), 30, "heartbeat에 새 서버")
    status = paired.api.ok("GET", "/api/relay/agent")
    assert status["running"] and status["hasToken"] and not status.get("lastError")


def _second_heartbeat(paired, relay) -> str:
    paired.api.ok("POST", "/api/relay/agent/stop")
    paired.api.ok("POST", "/api/relay/agent/start")
    wait_until(lambda: len(relay.heartbeats) >= 2, 30, "두 번째 heartbeat")
    return json.dumps(relay.heartbeats[-1], ensure_ascii=False)


def test_heartbeat_never_leaks_tokens_or_password_settings(paired, relay, fake_server):
    folder, _ = fake_server("mc1")
    paired.import_server("mc1", folder, module_settings={"ram": "256M", "management_server_secret": "e2e-secret-xyz"})
    blob = _second_heartbeat(paired, relay)
    assert relay.secret not in blob
    assert paired.sb.token() not in blob
    assert "e2e-secret-xyz" not in blob, "password 형식 모듈 설정값이 heartbeat로 릴레이에 올라감"


def test_heartbeat_never_leaks_local_paths(paired, relay, fake_server):
    folder, _ = fake_server("mc1")
    paired.import_server("mc1", folder, module_settings={"ram": "256M"})
    blob = _second_heartbeat(paired, relay)
    assert str(paired.sb.root) not in blob, "heartbeat에 로컬 경로(executable_path, working_dir)가 실림"


def test_raw_command_roundtrip_reaches_server_console(paired, relay, fake_server):
    folder, _ = fake_server("mc1")
    iid = paired.import_server("mc1", folder, module_settings={"ram": "256M"})
    paired.start_instance(iid)
    paired.wait_console(iid, DONE)

    cid = relay.push_command({"action": "raw_command", "text": "mc1 상태", "userName": "e2e", "via": "slash"}, requestedBy="332007306403250178", guildId="1", channelId="2", ownerDiscordId="332007306403250178")
    result = relay.wait_result(cid)
    assert result["success"] is True, result
    assert result["data"]["text"], result


def test_unknown_action_is_answered_not_dropped(paired, relay):
    cid = relay.push_command({"action": "e2e-no-such-action"}, ownerDiscordId="1")
    result = relay.wait_result(cid)
    assert result == {"success": False, "data": {"error": "Unknown action: e2e-no-such-action"}}


def test_crash_alert_is_uploaded_without_local_details(paired, relay, fake_server):
    folder, _ = fake_server("mc1")
    iid = paired.import_server("mc1", folder, module_settings={"ram": "256M", "auto_restart": False})
    paired.start_instance(iid)
    paired.wait_console(iid, DONE)
    paired.stdin(iid, "e2e-crash")
    wait_until(lambda: any(e["type"] == "instance.exit" for e in relay.events), 60, "릴레이에 instance.exit")
    ev = next(e for e in relay.events if e["type"] == "instance.exit")
    blob = json.dumps(ev, ensure_ascii=False)
    assert ev["data"].get("kind") == "crash", ev
    assert str(paired.sb.root) not in blob, "알림에 로컬 경로가 실림"
    assert "IllegalStateException" not in blob, "알림에 콘솔 원문이 실림"


def test_user_stop_is_not_alerted(paired, relay, fake_server):
    folder, _ = fake_server("mc1")
    iid = paired.import_server("mc1", folder, module_settings={"ram": "256M"})
    paired.start_instance(iid)
    paired.wait_console(iid, DONE)
    paired.stop_instance(iid)
    paired.wait_status(iid, "stopped")
    # 이벤트는 2초 모아 보낸다
    import time

    time.sleep(5)
    assert not [e for e in relay.events if e["type"] == "instance.exit"], relay.events


def test_agent_refuses_relay_requiring_newer_version(sandbox, relay, daemon_factory):
    relay.min_agent_version = "99.0.0"
    sandbox.write_config({"relay": {"cloud": {"relayUrl": relay.url}}})
    (sandbox.data / ".node_token").write_text(relay.node_token)
    d = daemon_factory()
    status = wait_until(lambda: (s := d.api.ok("GET", "/api/relay/agent")) and not s["running"] and s, 30, "에이전트 정지")
    assert "99.0.0" in (status.get("lastError") or ""), status
    assert relay.heartbeats == []


def test_wrong_node_token_reports_rejection(sandbox, relay, daemon_factory):
    sandbox.write_config({"relay": {"cloud": {"relayUrl": relay.url}}})
    (sandbox.data / ".node_token").write_text(f"sbn_{relay.node_id}.wrongsecret")
    d = daemon_factory()
    status = wait_until(lambda: (s := d.api.ok("GET", "/api/relay/agent")).get("lastError") and s, 30, "lastError")
    # heartbeat가 먼저 실패하면 "(node token rejected)", poll이 먼저면 "poll 401 … INVALID_SIGNATURE"
    err = status["lastError"].lower()
    assert "reject" in err or "401" in err, status


def test_token_replaced_by_another_pc_stops_the_agent(paired, relay):
    """같은 디스코드 계정으로 다른 PC를 페어링하면 이 PC는 멈추고, 끊긴 이유를 상태에 남긴다 (프리뷰 P2 #19)."""
    relay.token_replaced = True
    status = wait_until(lambda: (s := paired.api.ok("GET", "/api/relay/agent")) and not s["running"] and s, 90, "에이전트 정지")
    assert status["lastErrorCode"] == "token_replaced", status
    seen = len(relay.requests)
    import time

    time.sleep(3)
    assert len(relay.requests) == seen, "대체된 토큰으로 계속 요청함"


def test_no_token_means_no_agent(sandbox, relay, daemon_factory):
    sandbox.write_config({"relay": {"cloud": {"relayUrl": relay.url}}})
    d = daemon_factory()
    status = d.api.ok("GET", "/api/relay/agent")
    assert not status["running"] and not status["hasToken"]
    assert d.api.post("/api/relay/agent/start").status_code == 409
    assert relay.requests == []


def test_pairing_flow_saves_token_and_starts_agent(sandbox, relay, daemon_factory):
    """GUI가 하는 그대로: initiate → 상태 조회(대기) → 디스코드에서 코드 입력 → 상태 조회(claimed) → 토큰 저장."""
    sandbox.write_config({"relay": {"cloud": {"relayUrl": relay.url}}})
    d = daemon_factory()
    init = d.api.ok("POST", "/api/relay/pair/initiate", {"relayUrl": relay.url})
    code, poll_secret = init["code"], init["pollSecret"]

    pending = d.api.get(f"/api/relay/pair/{code}/status?secret={poll_secret}&relayUrl={relay.url}")
    assert pending.status_code == 200 and pending.json().get("status") == "pending", (
        f"페어링 대기 조회가 실패함: {pending.status_code} {pending.text}. "
        f"릴레이가 받은 요청: {[(r['path'], r['query'], r['headers'].get('x-poll-secret')) for r in relay.requests if '/status' in r['path']]}"
    )
    relay.claim_pairing(code)
    claimed = d.api.ok("GET", f"/api/relay/pair/{code}/status?secret={poll_secret}&relayUrl={relay.url}")
    assert claimed["status"] == "claimed"
    assert (sandbox.data / ".node_token").read_text().strip() == relay.node_token
    secret_in_query = [r for r in relay.requests if "/status" in r["path"] and poll_secret in r["query"]]
    assert not secret_in_query, "poll secret이 URL 쿼리로 나감 (relay 로그에 남는다)"
    # 페어링을 마치면 데몬을 다시 켜지 않아도 바로 디스코드 명령을 받는다 (프리뷰 P1, #74)
    wait_until(lambda: relay.heartbeats, 30, "페어링 직후 heartbeat")
    assert d.api.ok("GET", "/api/relay/agent")["running"]


def test_saving_node_token_starts_agent_without_restart(sandbox, relay, daemon_factory):
    """다른 곳에서 페어링한 토큰을 손으로 넣어도(`saba bot token`) 데몬 재시작 없이 연결된다."""
    sandbox.write_config({"relay": {"cloud": {"relayUrl": relay.url}}})
    d = daemon_factory()
    assert not d.api.ok("GET", "/api/relay/agent")["running"]
    d.api.ok("PUT", "/api/config/node-token", {"token": relay.node_token})
    wait_until(lambda: relay.heartbeats, 30, "토큰 저장 뒤 heartbeat")
    cid = relay.push_command({"action": "e2e-no-such-action"}, ownerDiscordId="1")
    assert relay.wait_result(cid, 30)["success"] is False


def test_replacing_node_token_drops_the_old_session(sandbox, relay, daemon_factory):
    """틀린 토큰으로 돌던 에이전트가 맞는 토큰을 저장하면 그 토큰으로 다시 붙는다."""
    sandbox.write_config({"relay": {"cloud": {"relayUrl": relay.url}}})
    (sandbox.data / ".node_token").write_text(f"sbn_{relay.node_id}.wrongsecret")
    d = daemon_factory()
    wait_until(lambda: d.api.ok("GET", "/api/relay/agent").get("lastError"), 30, "틀린 토큰 오류")
    d.api.ok("PUT", "/api/config/node-token", {"token": relay.node_token})
    wait_until(lambda: relay.heartbeats, 30, "맞는 토큰으로 heartbeat")


def test_heartbeat_carries_cpu_and_ram_only_for_running_servers(paired, relay, fake_server):
    """`/사바쨩 상태`의 서버별 CPU·RAM (Q-5, #86): 켜진 서버에만 싣고, 꺼진 서버에는 없다."""
    on, _ = fake_server("on1")
    off, _ = fake_server("off1")
    iid = paired.import_server("on1", on, module_settings={"ram": "256M"})
    paired.import_server("off1", off, module_settings={"ram": "256M"})
    paired.start_instance(iid)
    paired.wait_console(iid, DONE)
    paired.api.ok("POST", "/api/relay/agent/stop")
    paired.api.ok("POST", "/api/relay/agent/start")

    def latest():
        return {s["name"]: s for s in relay.heartbeats[-1]["metadata"]["servers"]}

    wait_until(lambda: {"on1", "off1"} <= latest().keys() and "usage" in latest()["on1"], 30, "켜진 서버의 CPU·RAM이 heartbeat에")
    servers = latest()
    usage = servers["on1"]["usage"]
    assert usage and usage["memMb"] > 0 and usage["cpu"] >= 0, servers["on1"]
    assert "usage" not in servers["off1"], servers["off1"]


def test_text_command_replies_in_the_guild_language(paired, relay):
    """디스코드 텍스트 명령 답은 데몬 GUI 언어가 아니라 릴레이가 실어 보낸 길드 언어로 쓴다."""
    ko = relay.wait_result(relay.push_command({"action": "raw_command", "text": "", "locale": "ko"}, ownerDiscordId="1"))
    assert "<모듈>" in ko["data"]["text"], ko
    en = relay.wait_result(relay.push_command({"action": "raw_command", "text": "", "locale": "en-US"}, ownerDiscordId="1"))
    assert "<module>" in en["data"]["text"], en


def test_host_id_follows_the_node_token(sandbox, relay, daemon_factory):
    """호스트 ID는 토큰(`sbn_<hostId>.…`)이 기준: 토큰을 손으로 넣으면 같이 채우고, 연결 해제하면 같이 지운다 (프리뷰 P2)."""
    sandbox.write_config({"relay": {"cloud": {"relayUrl": relay.url}}})
    d = daemon_factory()
    d.api.ok("PUT", "/api/config/node-token", {"token": relay.node_token})
    assert d.api.ok("GET", "/api/config/bot")["cloud"]["hostId"] == relay.node_id
    # 예전 값을 들고 있던 클라이언트가 봇 설정을 저장해도 토큰 쪽 ID가 남는다
    d.api.ok("PUT", "/api/config/bot", {"prefix": "!x", "cloud": {"relayUrl": relay.url, "hostId": "stale"}})
    assert d.api.ok("GET", "/api/config/bot")["cloud"]["hostId"] == relay.node_id
    d.api.ok("DELETE", "/api/config/node-token")
    assert d.api.ok("GET", "/api/config/bot")["cloud"].get("hostId", "") == ""
