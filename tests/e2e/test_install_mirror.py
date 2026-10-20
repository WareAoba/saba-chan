"""L3 마인크래프트 설치 경로를 외부망 없이: 데몬 → lifecycle install_server → (가로채기 프록시) → 고정 fixture.

받는 jar는 가짜 서버 jar라서 설치가 끝나면 그대로 켜서 실행까지 본다. 해시가 틀리면 설치가 거절되는지,
등록하지 않은 외부 주소로 나가지 않는지도 본다.
"""

import hashlib
import json
from pathlib import Path

import pytest

from harness.daemon import Daemon, wait_until
from harness.mirror import Mirror
from harness.paths import IS_WINDOWS

# Windows에서 시험 폴더의 모듈은 확인되지 않은 구성요소라 AppContainer로 돈다. AppContainer는 루프백에 닿지 못해
# 127.0.0.1의 가로채기 프록시를 쓸 수 없다. 설치 경로 자체는 리눅스(클라우드)에서 본다
pytestmark = pytest.mark.skipif(IS_WINDOWS, reason="AppContainer는 루프백 프록시에 닿지 못함")

DONE = r"Done \([^)]+\)!"
MC = "1.21.1"
PISTON_META = "https://piston-meta.mojang.com"


@pytest.fixture
def mirror(sandbox, fake_jar):
    m = Mirror(sandbox.root / "mirror")
    jar = fake_jar.read_bytes()
    sha1 = hashlib.sha1(jar).hexdigest()
    version_url = f"{PISTON_META}/v1/packages/e2e/{MC}.json"
    m.json(f"{PISTON_META}/mc/game/version_manifest_v2.json", {
        "latest": {"release": MC, "snapshot": "24w99a"},
        "versions": [
            {"id": "24w99a", "type": "snapshot", "url": f"{PISTON_META}/v1/packages/e2e/24w99a.json", "releaseTime": "2024-09-01T00:00:00+00:00"},
            {"id": MC, "type": "release", "url": version_url, "releaseTime": "2024-08-08T00:00:00+00:00"},
            {"id": "1.20.4", "type": "release", "url": f"{PISTON_META}/v1/packages/e2e/1.20.4.json", "releaseTime": "2023-12-07T00:00:00+00:00"},
        ],
    })
    for vid in (MC, "1.20.4"):
        m.json(f"{PISTON_META}/v1/packages/e2e/{vid}.json", {
            "id": vid,
            "downloads": {"server": {"sha1": sha1, "size": len(jar), "url": f"https://piston-data.mojang.com/v1/objects/{sha1}/server.jar"}},
            "javaVersion": {"component": "java-runtime-delta", "majorVersion": 17},
        })
    m.file(f"https://piston-data.mojang.com/v1/objects/{sha1}/server.jar", jar)
    # Paper (Fill v3)
    m.json(f"https://fill.papermc.io/v3/projects/paper/versions/{MC}/builds", [
        {"id": 130, "channel": "STABLE", "downloads": {"server:default": {"url": "https://fill-data.papermc.io/v1/objects/e2e/paper-1.21.1-130.jar", "checksums": {"sha256": hashlib.sha256(jar).hexdigest()}}}},
    ])
    m.file("https://fill-data.papermc.io/v1/objects/e2e/paper-1.21.1-130.jar", jar)
    # Purpur
    m.json(f"https://api.purpurmc.org/v2/purpur/{MC}/latest", {"build": "2329", "md5": hashlib.md5(jar).hexdigest()})
    m.file(f"https://api.purpurmc.org/v2/purpur/{MC}/latest/download", jar)
    # Fabric
    m.json(f"https://meta.fabricmc.net/v2/versions/loader/{MC}", [{"loader": {"version": "0.16.5", "stable": True}}])
    m.json("https://meta.fabricmc.net/v2/versions/installer", [{"version": "1.0.1", "stable": True}])
    m.file(f"https://meta.fabricmc.net/v2/versions/loader/{MC}/0.16.5/1.0.1/server/jar", jar)
    m.jar = jar
    yield m
    m.close()


@pytest.fixture
def mdaemon(sandbox, mirror, daemon_factory) -> Daemon:
    return daemon_factory(extra_env=mirror.env())


def _provision(daemon: Daemon, name: str, port: int, timeout: float = 300, **settings) -> tuple[str, dict]:
    body = {"name": name, "module_name": "minecraft", "port": port, "module_settings": {"ram": "256M", **settings}, "accept_agreements": daemon.agreement_ids("minecraft")}
    iid = daemon.api.ok("POST", "/api/instances", body)["id"]
    last: dict = {}

    def finished():
        nonlocal last
        p = daemon.api.ok("GET", f"/api/provision-progress/{name}")
        if p.get("active"):
            last = p
            return p.get("done")
        # 성공하면 5초 뒤 진행 표시가 사라진다
        return last.get("done") or (daemon.instance(iid).get("executable_path") and daemon.server_status(iid) != "provisioning")

    wait_until(finished, timeout, f"{name} 설치")
    return iid, last


# 데몬이 모듈용 venv를 만들며 pip를 올리려고 pypi에 묻는다 (python_env ensure_venv). 설치 경로와 무관하다
_KNOWN = ("https://pypi.org/", "https://files.pythonhosted.org/")


def _expected_hosts(mirror: Mirror) -> None:
    unexpected = [r for r in mirror.refused if not r.split(" ", 1)[1].startswith(_KNOWN)]
    assert not unexpected, f"등록하지 않은 외부 주소로 나감: {unexpected}"


def test_vanilla_latest_install_then_run(mdaemon, mirror, game_ports):
    iid, progress = _provision(mdaemon, "v1", game_ports[0])
    assert not progress.get("error"), progress
    inst = mdaemon.instance(iid)
    jar = inst["executable_path"]
    assert jar.endswith("server.jar"), inst
    with open(jar, "rb") as f:
        assert f.read() == mirror.jar
    workdir = inst["working_dir"]
    assert str(mdaemon.sb.data) in workdir, "설치가 인스턴스 폴더 밖에 됨"
    eula = open(f"{workdir}/eula.txt").read()
    assert "eula=true" in eula

    mdaemon.start_instance(iid)
    mdaemon.wait_console(iid, DONE)
    _expected_hosts(mirror)
    assert any("version_manifest_v2.json" in s for s in mirror.seen)


@pytest.mark.parametrize("server_type,jar_name", [("paper", None), ("purpur", None), ("fabric", "fabric-server-launch.jar")])
def test_server_type_install(mdaemon, mirror, game_ports, server_type, jar_name):
    iid, progress = _provision(mdaemon, f"t-{server_type}", game_ports[1], server_type=server_type)
    assert not progress.get("error"), progress
    inst = mdaemon.instance(iid)
    if jar_name:
        assert inst["executable_path"].endswith(jar_name), inst
    with open(inst["executable_path"], "rb") as f:
        assert f.read() == mirror.jar
    assert inst["module_settings"].get("server_type") == server_type
    _expected_hosts(mirror)


def test_checksum_mismatch_is_refused(mdaemon, mirror, game_ports):
    tampered = mirror.jar + b"tampered"
    sha1 = hashlib.sha1(mirror.jar).hexdigest()
    mirror.file(f"https://piston-data.mojang.com/v1/objects/{sha1}/server.jar", tampered)
    iid, progress = _provision(mdaemon, "bad", game_ports[2])
    assert progress.get("done") and progress.get("error"), f"해시가 틀린 jar를 받아들임: {progress}"
    assert "mismatch" in json.dumps(progress).lower() or "sha1" in json.dumps(progress).lower(), progress
    inst = mdaemon.instance(iid)
    assert not (inst.get("executable_path") or "").endswith("server.jar") or not __import__("os").path.exists(inst["executable_path"]), "틀린 jar가 남음"
    # 반쯤 받은 파일이 서버 폴더에 남지 않는다 (인스턴스 기록은 남아 사용자가 다시 시도하거나 지울 수 있다)
    leftovers = [p.name for p in Path(inst["working_dir"]).rglob("*") if p.is_file()] if Path(inst["working_dir"]).exists() else []
    assert leftovers == [], f"설치 실패 뒤 남은 파일: {leftovers}"
    # 설치가 안 된 서버를 켜면 서버 오류(5xx)가 아니라 이유가 있는 거절.
    # 모듈은 시작과 같은 경로(작업 폴더 + server.jar)를 받으므로 그 파일이 없다고 답한다 (#421)
    r = mdaemon.api.post(f"/api/instance/{iid}/start")
    assert r.status_code == 409 and r.json().get("issue_code") == "JAR_NOT_FOUND", f"{r.status_code} {r.text}"


def test_mojang_unreachable_gives_clear_error(mdaemon, mirror, game_ports):
    mirror.routes.clear()
    r = mdaemon.api.post("/api/instances", {"name": "off", "module_name": "minecraft", "port": game_ports[3], "module_settings": {"ram": "256M"}})
    if r.status_code >= 400:
        assert "version" in r.text.lower(), r.text
        return
    progress = wait_until(lambda: (p := mdaemon.api.ok("GET", "/api/provision-progress/off")).get("done") and p, 120, "설치 실패 보고")
    assert progress["error"], progress


def test_list_versions_through_daemon(mdaemon, mirror):
    data = mdaemon.api.ok("GET", "/api/module/minecraft/versions")
    ids = [v["id"] for v in data["versions"]]
    assert ids == [MC, "1.20.4"], data
    assert data["latest"]["release"] == MC


def test_pinned_version_install_route(mdaemon, mirror, sandbox):
    target = sandbox.root / "pinned"
    r = mdaemon.api.post("/api/module/minecraft/install", {"version": "1.20.4", "install_dir": str(target), "accept_agreements": ["eula"]}, timeout=180)
    assert r.status_code == 200 and r.json().get("success") is not False, r.text
    assert (target / "server.jar").read_bytes() == mirror.jar
    assert "eula=true" in (target / "eula.txt").read_text()
