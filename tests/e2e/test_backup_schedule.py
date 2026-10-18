"""L1 백업·복원·보관 개수, 예약 작업(공지·백업·재시작)."""

import pytest

from harness.daemon import EventStream, wait_until

DONE = r"Done \([^)]+\)!"


def _backups(daemon, iid) -> list[dict]:
    data = daemon.api.ok("GET", f"/api/instance/{iid}/backups")
    return data.get("backups", data) if isinstance(data, dict) else data


def _name(b: dict) -> str:
    return b.get("name") or b.get("file") or b.get("id")


def test_backup_restore_roundtrip(mc):
    daemon, iid, folder, _ = mc
    level = folder / "world" / "level.dat"
    daemon.start_instance(iid)
    daemon.wait_console(iid, DONE)
    daemon.mark_console(iid)
    events = EventStream(daemon)
    try:
        created = daemon.api.ok("POST", f"/api/instance/{iid}/backups", timeout=120)
        # 켜져 있으면 저장 명령(save-all)을 먼저 보낸다
        daemon.wait_console(iid, "Saved the game", timeout=30)
        events.wait_for("backup.done", lambda d: d.get("instanceId") == iid, timeout=60)
    finally:
        events.close()
    backups = _backups(daemon, iid)
    assert len(backups) == 1, (created, backups)
    original = level.read_text()

    name = _name(backups[0])
    r = daemon.api.post(f"/api/instance/{iid}/backups/{name}/restore")
    assert r.status_code == 409, f"실행 중 복원이 허용됨: {r.status_code} {r.text}"

    daemon.stop_instance(iid)
    daemon.wait_status(iid, "stopped")
    level.write_text("corrupted by e2e\n")
    daemon.api.ok("POST", f"/api/instance/{iid}/backups/{name}/restore", timeout=120)
    assert level.read_text() == original
    assert (folder / "server.jar").exists()

    # 복원 전 상태도 자동으로 남는다 (되돌리기용)
    kinds = sorted(b["kind"] for b in _backups(daemon, iid))
    assert kinds == ["manual", "pre-restore"], kinds
    daemon.api.ok("DELETE", f"/api/instance/{iid}/backups/{name}")
    assert [b["kind"] for b in _backups(daemon, iid)] == ["pre-restore"]


def test_backup_of_stopped_server(mc):
    daemon, iid, folder, _ = mc
    (folder / "world").mkdir(exist_ok=True)
    (folder / "world" / "level.dat").write_text("offline\n")
    daemon.api.ok("POST", f"/api/instance/{iid}/backups", timeout=120)
    assert len(_backups(daemon, iid)) == 1


def test_backup_keep_never_prunes_manual_backups(mc):
    daemon, iid, folder, _ = mc
    (folder / "world").mkdir(exist_ok=True)
    (folder / "world" / "level.dat").write_text("x\n")
    daemon.api.ok("PUT", f"/api/instance/{iid}/schedule", {"schedule": [], "backup_keep": 2})
    for _ in range(4):
        daemon.api.ok("POST", f"/api/instance/{iid}/backups", timeout=120)
    # 보관 개수는 자동 백업에만 적용된다 (src/backup prune)
    assert len(_backups(daemon, iid)) == 4, _backups(daemon, iid)


def test_invalid_schedule_is_rejected(mc):
    daemon, iid, *_ = mc
    for bad in (
        {"action": "backup"},
        {"action": "backup", "every_minutes": 0},
        {"action": "announce", "every_minutes": 5},
        {"action": "backup", "at": "25:00"},
        {"action": "backup", "every_minutes": 5, "warn_minutes": [1]},
    ):
        r = daemon.api.put(f"/api/instance/{iid}/schedule", {"schedule": [bad]})
        assert r.status_code == 400, (bad, r.text)
    assert daemon.api.ok("GET", f"/api/instance/{iid}/schedule").get("schedule") in ([], None)


@pytest.mark.slow
def test_scheduled_announce_and_backup_fire(mc):
    """1분 주기 작업이 30초 틱 안에 실행된다 (최대 약 2분)."""
    daemon, iid, *_ = mc
    daemon.start_instance(iid)
    daemon.wait_console(iid, DONE)
    daemon.api.ok("PUT", f"/api/instance/{iid}/schedule", {"schedule": [
        {"action": "announce", "every_minutes": 1, "message": "e2e-예약-공지"},
        {"action": "backup", "every_minutes": 1},
    ]})
    events = EventStream(daemon)
    try:
        daemon.wait_console(iid, "e2e-예약-공지", timeout=150)
        events.wait_for("backup.done", lambda d: d.get("instanceId") == iid, timeout=150)
    finally:
        events.close()
    assert _backups(daemon, iid)
    rows = daemon.api.ok("GET", "/api/audit?limit=50") if daemon.api.get("/api/audit").status_code == 200 else None
    if rows is not None:
        assert "schedule" in str(rows), "예약 실행이 활동 기록에 없음"


@pytest.mark.slow
def test_scheduled_restart_warns_then_restarts(mc):
    daemon, iid, folder, _ = mc
    daemon.start_instance(iid)
    daemon.wait_console(iid, DONE)
    daemon.api.ok("PUT", f"/api/instance/{iid}/schedule", {"schedule": [{"action": "restart", "every_minutes": 2, "warn_minutes": [1]}]})
    events = EventStream(daemon)
    try:
        events.wait_for("schedule.warn", lambda d: d.get("instanceId") == iid, timeout=150)
        daemon.wait_console(iid, "Stopping the server", timeout=150)
        daemon.wait_console(iid, DONE, timeout=90)
    finally:
        events.close()
    wait_until(lambda: daemon.server_status(iid) == "running", 30, "재시작 뒤 실행 중")
