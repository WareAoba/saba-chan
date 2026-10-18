"""L1 이벤트 SSE(/api/events): 끊긴 뒤 이어 받기, 데몬 종료 때 스트림 끝남."""

from harness.daemon import EventStream, wait_until


def _backup_event(daemon, iid, events: EventStream) -> dict:
    daemon.api.ok("POST", f"/api/instance/{iid}/backups", timeout=120)
    return events.wait_for("backup.done", lambda d: d.get("instanceId") == iid, timeout=60)


def _prepare(folder) -> None:
    (folder / "world").mkdir(exist_ok=True)
    (folder / "world" / "level.dat").write_text("e2e\n")


def test_last_event_id_resumes_without_repeating(mc):
    daemon, iid, folder, _ = mc
    _prepare(folder)
    first_stream = EventStream(daemon)
    try:
        first = _backup_event(daemon, iid, first_stream)
    finally:
        first_stream.close()
    assert first["id"], f"backup.done에 id가 없어 이어 받을 수 없음: {first}"
    # 디스코드 알림이 ID 대신 서버 이름을 보이도록 이름을 싣는다
    assert first["data"].get("instance") and first["data"]["instance"] != iid, first

    # 끊긴 사이에 일어난 일
    daemon.api.ok("POST", f"/api/instance/{iid}/backups", timeout=120)

    resumed = EventStream(daemon, last_event_id=first["id"])
    try:
        missed = wait_until(lambda: [e for e in resumed.of_type("backup.done") if e["data"].get("instanceId") == iid], 30, "끊긴 사이의 backup.done")
        ids = [e["id"] for e in resumed.events if e["id"] is not None]
        assert all(i > first["id"] for i in ids), f"이미 받은 이벤트가 다시 옴: {ids} (마지막 받은 id {first['id']})"
        assert missed[0]["id"] > first["id"], missed
    finally:
        resumed.close()


def test_since_query_matches_last_event_id(mc):
    daemon, iid, folder, _ = mc
    _prepare(folder)
    s = EventStream(daemon)
    try:
        first = _backup_event(daemon, iid, s)
    finally:
        s.close()
    daemon.api.ok("POST", f"/api/instance/{iid}/backups", timeout=120)
    q = EventStream(daemon, since=first["id"])
    try:
        got = wait_until(lambda: q.of_type("backup.done"), 30, "since 이후 backup.done")
        assert all(e["id"] > first["id"] for e in got), got
    finally:
        q.close()


def test_stream_ends_when_daemon_shuts_down(daemon):
    """열린 SSE 연결이 정상 종료를 붙잡지 않는다."""
    s = EventStream(daemon)
    try:
        daemon.api.ok("POST", "/api/daemon/shutdown")
        wait_until(lambda: not daemon.alive(), 30, "SSE가 열린 채로 데몬 종료")
        s._thread.join(10)
        assert not s._thread.is_alive(), "데몬이 끝났는데 스트림이 닫히지 않음"
    finally:
        s.close()
