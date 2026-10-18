"""L1·L2 모듈 약관(마인크래프트 EULA) 동의 흐름: 데몬이 대신 동의하지 않고, 이 PC에서 동의해야 켜진다.
디스코드·챗봇에서는 동의할 수 없고 무엇에 동의해야 하는지 안내만 받는다 (saba-chan #81, modules #12)."""

import pytest

from harness.daemon import wait_until
from harness.fake_relay import FakeRelay

DONE = r"Done \([^)]+\)!"
OWNER = "332007306403250178"


@pytest.fixture
def unagreed(daemon, fake_server):
    """EULA에 동의하지 않은 가짜 서버를 가져온 인스턴스."""
    folder, _ = fake_server("noeula", eula=False)
    iid = daemon.import_server("noeula", folder, module_settings={"ram": "256M"}, agree=False)
    return daemon, iid, folder


def _eula(inst: dict) -> dict:
    return next(a for a in inst["agreements"] if a["id"] == "eula")


def test_instance_shows_agreement_with_name_link_and_state(unagreed):
    daemon, iid, _ = unagreed
    a = _eula(daemon.api.ok("GET", f"/api/instance/{iid}"))
    assert a["url"].startswith("https://") and a.get("label"), a
    assert not a.get("accepted"), a


def test_unknown_agreement_id_is_rejected(unagreed):
    daemon, iid, _ = unagreed
    r = daemon.api.post(f"/api/instance/{iid}/agreements", {"accept": ["tos"]})
    assert r.status_code == 400 and r.json()["error_code"] == "invalid_format", r.text
    r = daemon.api.post(f"/api/instance/{iid}/agreements", {"accept": "eula"})
    assert r.status_code == 400, r.text


def test_acceptance_survives_daemon_restart(unagreed, daemon_factory):
    daemon, iid, folder = unagreed
    daemon.api.ok("POST", f"/api/instance/{iid}/agreements", {"accept": ["eula"]})
    daemon.shutdown()
    d2 = daemon_factory()
    assert _eula(d2.api.ok("GET", f"/api/instance/{iid}")).get("accepted")
    d2.start_instance(iid)
    d2.wait_console(iid, DONE)


def test_settings_patch_cannot_mark_agreement_accepted(unagreed):
    """설정 바꾸기(PATCH)로 동의 기록을 써 넣을 수 없다 (챗봇의 instance.settings.set도 이 경로다)."""
    daemon, iid, _ = unagreed
    daemon.api.patch(f"/api/instance/{iid}", {"accepted_agreements": {"eula": "2026-01-01T00:00:00Z"}})
    assert not _eula(daemon.api.ok("GET", f"/api/instance/{iid}")).get("accepted")
    assert daemon.api.post(f"/api/instance/{iid}/start").json().get("error_code") == "agreement_required"


def test_create_with_accepted_agreement_starts_without_asking(daemon, fake_server):
    folder, _ = fake_server("pre", eula=False)
    iid = daemon.import_server("pre", folder, module_settings={"ram": "256M"}, accept_agreements=["eula"])
    daemon.start_instance(iid)
    daemon.wait_console(iid, DONE)


# ── 디스코드·챗봇: 안내만 ──
@pytest.fixture
def relay():
    r = FakeRelay()
    yield r
    r.close()


@pytest.fixture
def paired_unagreed(sandbox, relay, daemon_factory, fake_server):
    sandbox.write_config({"relay": {"cloud": {"relayUrl": relay.url}}})
    (sandbox.data / ".node_token").write_text(relay.node_token)
    d = daemon_factory()
    wait_until(lambda: relay.heartbeats, 30, "첫 heartbeat")
    folder, _ = fake_server("noeula", eula=False)
    iid = d.import_server("noeula", folder, module_settings={"ram": "256M"}, agree=False)
    return d, iid, folder


def _action(relay, action: str, via: str = "direct", **payload) -> dict:
    cid = relay.push_command(
        {"action": action, "locale": "ko", "via": via, "userName": "e2e", **payload},
        requestedBy=OWNER, guildId="1", channelId="2", ownerDiscordId=OWNER,
    )
    return relay.wait_result(cid)


def test_discord_start_explains_the_agreement_instead_of_starting(paired_unagreed, relay):
    daemon, iid, _ = paired_unagreed
    plan = _action(relay, "plan", id="server.start", instance=iid, input={})
    if plan["success"] and plan["data"].get("planId"):
        result = _action(relay, "run_plan", id="server.start", instance=iid, input={}, planId=plan["data"]["planId"])
    else:
        result = plan
    text = str(result)
    assert result["success"] is False, result
    assert "agreement_required" in text and "https://" in text, f"디스코드 답에 무엇에 동의해야 하는지(이름·링크)가 없음: {result}"
    assert daemon.server_status(iid) == "stopped"


def test_agreement_cannot_be_accepted_from_discord_or_chatbot(paired_unagreed, relay, fake_server):
    daemon, iid, _ = paired_unagreed
    folder, port = fake_server("bychat", eula=False)
    for via in ("direct", "chatbot"):
        plan = _action(relay, "plan", via=via, id="instance.create", input={"name": f"x-{via}", "module_name": "minecraft", "migration_source": str(folder), "accept_agreements": ["eula"]})
        if plan["success"] and plan["data"].get("planId"):
            plan = _action(relay, "run_plan", via=via, id="instance.create", input=plan["data"].get("input", {}), planId=plan["data"]["planId"])
        assert plan["success"] is False, f"{via}에서 약관 동의가 받아들여짐: {plan}"
        assert plan["data"]["code"] in ("invalid_input", "agreement_local_only", "PERMISSION_DENIED"), plan
    assert not [i for i in daemon.instances() if i["name"].startswith("x-")]
    assert not _eula(daemon.api.ok("GET", f"/api/instance/{iid}")).get("accepted")
