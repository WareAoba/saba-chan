"""L1c CLI(saba-chan-cli)로 같은 흐름을 돌린다. --json 출력과 종료 코드 계약 (0 성공, 1 실패, 2 사용법, 3 데몬 꺼짐, 4 없음)."""

import pytest

from harness.daemon import wait_until

DONE = r"Done \([^)]+\)!"


def test_daemon_down_exit_3(cli):
    # status는 꺼진 것도 보고하는 명령이라 0이다 (running:false)
    assert cli.run("status").json()["daemon"]["running"] is False
    r = cli.run("server", "list")
    assert r.code == 3, r


def test_usage_error_exit_2(cli, daemon):
    assert cli.run("server", "no-such-subcommand").code == 2
    # TTY가 없으면 게임을 묻지 않고 사용법 오류
    assert cli.run("server", "add", "x").code == 2


def test_not_found_exit_4(cli, daemon):
    r = cli.run("server", "info", "없는서버")
    assert r.code == 4, r


def test_status_json(cli, daemon):
    r = cli.run("status", check=0)
    body = r.json()
    assert isinstance(body, dict)


def test_full_server_flow_through_cli(cli, daemon, fake_server):
    folder, port = fake_server("cli1")
    r = cli.run("server", "add", "cli1", "-m", "minecraft", "--from", str(folder), "--set", "ram=256M")
    assert r.code == 0, r
    iid = next(i["id"] for i in daemon.instances() if i["name"] == "cli1")

    listing = cli.run("server", "list", check=0).json()
    assert "cli1" in str(listing)

    r = cli.run("start", "cli1")
    assert r.code == 0, r
    daemon.wait_console(iid, DONE)

    r = cli.run("run", "cli1", "list")
    assert r.code == 0, r
    assert "players online" in r.stdout, f"run 결과에 서버 답이 없음\n{r}"

    r = cli.run("console", "cli1", "--print", "-n", "50")
    assert r.code == 0 and "Done (" in r.stdout, r

    r = cli.run("backup", "create", "cli1", timeout=180)
    assert r.code == 0, r
    assert cli.run("backup", "list", "cli1", check=0).json()

    r = cli.run("stop", "cli1")
    assert r.code == 0, r
    assert daemon.server_status(iid) == "stopped"

    hist = cli.run("server", "history", "cli1", check=0)
    assert "stop" in hist.stdout.lower(), hist


def test_eula_preflight_reported_by_cli(cli, daemon, fake_server):
    folder, _ = fake_server("noeula", eula=False)
    cli.run("server", "add", "noeula", "-m", "minecraft", "--from", str(folder), "--set", "ram=256M", check=0)
    r = cli.run("start", "noeula")
    assert r.code == 1, r
    assert "eula" in (r.stdout + r.stderr).lower(), r
    assert cli.run("server", "accept-eula", "noeula", "--yes").code == 0
    assert cli.run("start", "noeula").code == 0


def test_cli_refuses_second_server_on_same_folder(cli, daemon, fake_server):
    folder, _ = fake_server("dup")
    cli.run("server", "add", "dup", "-m", "minecraft", "--from", str(folder), check=0)
    r = cli.run("server", "add", "dup2", "-m", "minecraft", "--from", str(folder))
    assert r.code != 0, r


def test_daemon_rejects_second_server_on_same_folder(daemon, fake_server):
    """같은 폴더 서버 둘은 서로 덮어쓴다. CLI만 막고 데몬은 받아 줬었다 (cli-remake 점검 8번)."""
    folder, _ = fake_server("dup")
    daemon.import_server("dup", folder)
    r = daemon.api.post("/api/instances", {"name": "dup2", "module_name": "minecraft", "migration_source": str(folder)})
    assert r.status_code >= 400, f"데몬이 같은 폴더의 두 번째 서버를 받아 줌: {r.status_code} {r.text}"


def test_server_set_rejects_unknown_key(cli, daemon, fake_server):
    folder, _ = fake_server("s1")
    cli.run("server", "add", "s1", "-m", "minecraft", "--from", str(folder), check=0)
    r = cli.run("server", "set", "s1", "오타키=1")
    assert r.code != 0, r


def test_daemon_stop_via_cli(cli, daemon):
    r = cli.run("daemon", "stop", "--yes")
    assert r.code == 0, r
    wait_until(lambda: not daemon.alive(), 30, "데몬 종료")


@pytest.mark.parametrize("shell", ["bash", "powershell", "zsh"])
def test_completions_print(cli, shell):
    r = cli.run("completions", shell, json_out=False)
    assert r.code == 0 and len(r.stdout) > 200


READ_ONLY = [
    ("status",),
    ("server", "list"),
    ("module", "list"),
    ("module", "info", "minecraft"),
    ("module", "commands", "minecraft"),
    ("module", "schema"),
    ("extension", "list"),
    ("bot", "status"),
    ("chatbot", "show"),
    ("update", "status"),
    ("config", "show"),
    ("config", "defaults"),
    ("monitor",),
    ("audit", "--limit", "5"),
    ("version",),
    ("do",),
]


@pytest.mark.parametrize("args", READ_ONLY, ids=lambda a: "-".join(a))
def test_read_only_commands_print_json(cli, daemon, args):
    """조회 명령은 --json으로 0과 해석되는 JSON을 낸다 (스크립트·봇이 기대는 계약)."""
    r = cli.run(*args)
    assert r.code == 0, r
    r.json()


def test_config_set_get_roundtrip(cli, daemon):
    r = cli.run("config", "set", "fontScale=110")
    assert r.code == 0, r
    got = cli.run("config", "get", "fontScale", check=0).stdout
    assert "110" in got, got
    # 설정 파일 하나에 저장된다 (데몬이 쓴다)
    assert "110" in (daemon.sb.data / "config.json").read_text(encoding="utf-8")
    assert cli.run("config", "set", "fontScale=abc").code != 0, "숫자 설정에 글자를 받아들임"


def test_config_export_import_moves_servers_to_another_pc(cli, daemon, fake_server, sandbox):
    """설정·서버 목록을 zip으로 옮긴다. 게임 파일은 들어가지 않는다."""
    from harness.cli import Cli
    from harness.daemon import Daemon
    from harness.sandbox import Sandbox

    folder, _ = fake_server("moved")
    daemon.import_server("moved", folder, module_settings={"ram": "256M"})
    cli.run("config", "set", "fontScale=120", check=0)
    archive = sandbox.root / "export.zip"
    r = cli.run("config", "export", str(archive))
    assert r.code == 0 and archive.exists(), r
    import zipfile

    names = zipfile.ZipFile(archive).namelist()
    assert not any(n.endswith(("server.jar", "level.dat")) for n in names), f"게임 파일이 내보내기에 들어감: {names}"

    other = Sandbox(sandbox.root / "pc2")
    d2 = Daemon(other).start()
    try:
        r = Cli(other).run("config", "import", str(archive), "--yes")
        assert r.code == 0, r
        names2 = [i["name"] for i in d2.instances()]
        assert "moved" in names2, names2
        assert "120" in Cli(other).run("config", "get", "fontScale", check=0).stdout
    finally:
        d2.shutdown()
