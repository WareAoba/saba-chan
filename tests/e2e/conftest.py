"""E2E 하네스 공통 fixture. 설계: /mnt/project-files/e2e/design.md, 사용법: README.md."""

import json
import os
import shutil
import sys
import time
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent))

from harness import fakemc, procs  # noqa: E402
from harness.cli import Cli  # noqa: E402
from harness.daemon import Daemon  # noqa: E402
from harness.paths import IS_WINDOWS, ROOT, binary, real_data_dirs  # noqa: E402
from harness.sandbox import Sandbox, free_ports  # noqa: E402

RUN_DIR = ROOT / time.strftime("run-%Y%m%d-%H%M%S")


def pytest_configure(config):
    RUN_DIR.mkdir(parents=True, exist_ok=True)
    config._saba_real_snapshot = {str(d): _snapshot(d) for d in real_data_dirs()}


def pytest_collection_modifyitems(config, items):
    skip_win = pytest.mark.skip(reason="Windows 전용")
    for item in items:
        if "windows" in item.keywords and not IS_WINDOWS:
            item.add_marker(skip_win)


def _snapshot(path: Path) -> dict[str, tuple[int, int]]:
    """실제 데이터 폴더의 파일 목록·크기·수정 시각."""
    if not path.exists():
        return {}
    out = {}
    for p in path.rglob("*"):
        try:
            if p.is_file():
                st = p.stat()
                out[str(p.relative_to(path))] = (st.st_size, int(st.st_mtime))
        except OSError:
            continue
    return out


def pytest_sessionfinish(session, exitstatus):
    leftover = procs.kill_mentioning(str(RUN_DIR))
    lines = []
    if leftover:
        lines.append("## 시험이 끝난 뒤 남은 프로세스 (하네스가 정리함)")
        lines += [f"- pid {pid}: {cmd[:300]}" for pid, cmd in leftover]
    before = getattr(session.config, "_saba_real_snapshot", {})
    for d in real_data_dirs():
        if _snapshot(d) != before.get(str(d), {}):
            lines.append(f"## 실제 데이터 폴더가 바뀜: {d}")
            session.exitstatus = 1
    if lines:
        (RUN_DIR / "guard.md").write_text("\n".join(lines) + "\n", encoding="utf-8")
        print("\n" + "\n".join(lines))


@pytest.hookimpl(hookwrapper=True, tryfirst=True)
def pytest_runtest_makereport(item, call):
    outcome = yield
    rep = outcome.get_result()
    setattr(item, f"rep_{rep.when}", rep)


# ── 바이너리 ──
@pytest.fixture(scope="session", autouse=True)
def built_binaries():
    missing = [n for n in ("saba-core", "saba-runner", "saba-chan-cli") if not binary(n).exists()]
    if missing:
        pytest.exit(f"바이너리가 없습니다: {missing}. 먼저 `cargo build -p saba-core -p saba-runner -p saba-chan-cli` (SABA_E2E_BIN_DIR로 위치 지정)", 2)


@pytest.fixture(scope="session")
def fake_jar() -> Path:
    if not shutil.which("javac") or not shutil.which("java"):
        pytest.skip("JDK(javac, java)가 PATH에 없습니다")
    return fakemc.build_jar()


# ── 격리 폴더와 데몬 ──
@pytest.fixture
def sandbox(request) -> Sandbox:
    name = request.node.name.replace("[", "_").replace("]", "").replace("/", "_")[:80]
    root = RUN_DIR / name
    if root.exists():
        shutil.rmtree(root)
    return Sandbox(root)


def _dump_artifacts(request, sb: Sandbox, daemon: Daemon | None):
    rep = getattr(request.node, "rep_call", None)
    if not (rep and rep.failed) or daemon is None:
        return
    try:
        if daemon.alive():
            for inst in daemon.instances():
                (sb.artifacts / f"console-{inst['name']}.txt").write_text(daemon.console_text(inst["id"]), encoding="utf-8")
                (sb.artifacts / f"instance-{inst['name']}.json").write_text(json.dumps(inst, ensure_ascii=False, indent=2), encoding="utf-8")
    except Exception as exc:  # 증거 수집 실패가 원래 실패를 가리지 않게
        (sb.artifacts / "dump-error.txt").write_text(repr(exc))
    listing = [str(p.relative_to(sb.root)) for p in sb.data.rglob("*") if "python-env" not in p.parts]
    (sb.artifacts / "data-tree.txt").write_text("\n".join(sorted(listing)), encoding="utf-8")


@pytest.fixture
def daemon_factory(request, sandbox):
    """같은 격리 폴더에서 데몬을 여러 번 띄울 수 있다 (재시작 시험)."""
    started: list[Daemon] = []

    def make(start: bool = True, **kw) -> Daemon:
        d = Daemon(sandbox, **kw)
        started.append(d)
        if start:
            d.start()
        return d

    yield make
    _dump_artifacts(request, sandbox, next((d for d in reversed(started) if d.alive()), None))
    for d in started:
        if d.alive():
            try:
                for inst in d.instances():
                    if d.server_status(inst["id"]) == "running":
                        d.api.post(f"/api/instance/{inst['id']}/stop", {"force": True}, timeout=30)
            except Exception:
                pass
            try:
                d.shutdown()
            except Exception:
                d.kill()
    procs.kill_mentioning(str(sandbox.root))


@pytest.fixture
def daemon(daemon_factory) -> Daemon:
    return daemon_factory()


@pytest.fixture
def cli(sandbox) -> Cli:
    return Cli(sandbox)


@pytest.fixture
def game_ports():
    """가짜 서버용 빈 포트 (서로 2 이상 떨어짐)."""
    return free_ports(4, step=2)


@pytest.fixture
def fake_server(sandbox, fake_jar, game_ports):
    """가짜 마인크래프트 서버 폴더를 만든다: fake_server("mc1", eula=False)."""
    used = iter(game_ports)

    def make(name: str = "mc1", port: int | None = None, **kw) -> tuple[Path, int]:
        port = port or next(used)
        return fakemc.make_server(sandbox.servers / name, port, **kw), port

    return make


@pytest.fixture
def mc(daemon, fake_server):
    """가짜 서버를 가져온 인스턴스 하나. (daemon, id, 폴더, 포트)"""
    folder, port = fake_server("mc1")
    iid = daemon.import_server("mc1", folder, module_settings={"ram": "256M"})
    return daemon, iid, folder, port


def pytest_report_header(config):
    return [f"saba e2e run dir: {RUN_DIR}", f"binaries: {binary('saba-core').parent}", f"platform: {sys.platform}, python {sys.version.split()[0]}", f"pid {os.getpid()}"]
