"""실제 relay-server(server-chan, 비공개 저장소)를 임시 PostgreSQL 위에 띄운다.

relay 쪽 하네스는 server-chan `relay-server/test/e2e/harness.ts`(서버쨩 스레드 소유)다. stdin JSON 줄로 조작한다:
첫 줄 {"ready":true,"url"}, 이후 요청 한 줄({"id","op",…})마다 같은 id의 답 한 줄. op는 claim·command·action·host·notices·stop.
리눅스에서 root로 돌면 embedded-postgres(initdb)가 거부하므로 시스템 PostgreSQL을 postgres 사용자로 띄워 DATABASE_URL로 넘긴다.
"""

import itertools
import json
import os
import queue
import shutil
import subprocess
import tempfile
import threading
from pathlib import Path

from .paths import IS_WINDOWS, SERVER_CHAN_REPO
from .sandbox import free_port

RELAY_DIR = SERVER_CHAN_REPO / "relay-server"
HARNESS = RELAY_DIR / "test" / "e2e" / "harness.ts"


def available() -> str | None:
    """못 쓰면 이유를 돌려준다."""
    if not HARNESS.exists():
        return f"relay 하네스가 없습니다: {HARNESS}"
    if not (RELAY_DIR / "node_modules" / ".bin").exists():
        return f"relay-server 의존성이 없습니다: {RELAY_DIR} 에서 npm ci"
    if not shutil.which("node"):
        return "node가 PATH에 없습니다"
    return None


class _SystemPostgres:
    """root 리눅스용: postgres 사용자로 initdb·pg_ctl."""

    def __init__(self, root: Path):
        bins = sorted(Path("/usr/lib/postgresql").glob("*/bin"))
        if not bins:
            raise RuntimeError("PostgreSQL 바이너리가 없습니다 (/usr/lib/postgresql/*/bin)")
        self.bin = bins[-1]
        self.dir = root / "pg"
        self.dir.mkdir(parents=True)
        shutil.chown(self.dir, "postgres", "postgres")
        self.port = free_port()
        # Unix 소켓 경로는 107바이트 제한: 시험 폴더가 깊으면 PG가 뜨지 않으므로 소켓만 짧은 임시 폴더에 둔다
        self.sock = Path(tempfile.mkdtemp(prefix="sbpg-"))
        shutil.chown(self.sock, "postgres", "postgres")
        self._as_pg([str(self.bin / "initdb"), "-D", str(self.dir / "data"), "-U", "saba", "--auth=trust", "-E", "UTF8"])
        self._as_pg([str(self.bin / "pg_ctl"), "-D", str(self.dir / "data"), "-o", f"-p {self.port} -k {self.sock} -h 127.0.0.1", "-l", str(self.dir / "log.txt"), "-w", "start"])
        self._as_pg([str(self.bin / "createdb"), "-h", "127.0.0.1", "-p", str(self.port), "-U", "saba", "saba_relay_e2e"])
        self.url = f"postgresql://saba@127.0.0.1:{self.port}/saba_relay_e2e"

    def _as_pg(self, argv: list[str]) -> None:
        subprocess.run(["runuser", "-u", "postgres", "--", *argv], check=True, capture_output=True, timeout=60)

    def stop(self) -> None:
        subprocess.run(["runuser", "-u", "postgres", "--", str(self.bin / "pg_ctl"), "-D", str(self.dir / "data"), "-m", "immediate", "stop"], capture_output=True, timeout=60)
        shutil.rmtree(self.sock, ignore_errors=True)


class RealRelay:
    def __init__(self, root: Path):
        root.mkdir(parents=True, exist_ok=True)
        self.pg = None
        env = {k: v for k, v in os.environ.items() if k not in ("DATABASE_URL", "DISCORD_TOKEN", "DISCORD_BOT_TOKEN")}
        env.update(E2E_RELAY_DIR=str(root), NODE_ENV="test")
        if not IS_WINDOWS and os.geteuid() == 0:
            self.pg = _SystemPostgres(root)
            env["DATABASE_URL"] = self.pg.url
        self.log = open(root / "relay.log", "w", encoding="utf-8")
        tsx = RELAY_DIR / "node_modules" / ".bin" / ("tsx.cmd" if IS_WINDOWS else "tsx")
        self.proc = subprocess.Popen([str(tsx), str(HARNESS)], cwd=RELAY_DIR, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.log, text=True, encoding="utf-8")
        # readline이 시한 없이 막히지 않게 stdout은 따로 읽어 줄 단위로 넘긴다
        self._lines: queue.Queue = queue.Queue()
        self._waiting: dict[int, queue.Queue] = {}
        self._lock = threading.Lock()
        self._ids = itertools.count(1)
        self._ready: queue.Queue = queue.Queue()
        threading.Thread(target=self._read, daemon=True).start()
        try:
            info = self._ready.get(timeout=120)
        except queue.Empty:
            info = None
        if not info or not info.get("ready"):
            self.close()
            raise RuntimeError("relay 하네스가 뜨지 않음:\n" + Path(self.log.name).read_text(encoding="utf-8", errors="replace")[-3000:])
        self.url = info["url"]

    def _read(self) -> None:
        for line in self.proc.stdout:
            line = line.strip()
            if not line.startswith("{"):
                continue
            try:
                msg = json.loads(line)
            except ValueError:
                continue
            if msg.get("ready"):
                self._ready.put(msg)
                continue
            with self._lock:
                waiter = self._waiting.pop(msg.get("id"), None)
            if waiter:
                waiter.put(msg)
        self._ready.put(None)  # 하네스가 끝남

    def call(self, op: str, timeout: float = 60, **fields) -> dict:
        rid = next(self._ids)
        waiter: queue.Queue = queue.Queue()
        with self._lock:
            self._waiting[rid] = waiter
        self.proc.stdin.write(json.dumps({"id": rid, "op": op, **fields}, ensure_ascii=False) + "\n")
        self.proc.stdin.flush()
        try:
            return waiter.get(timeout=timeout)
        except queue.Empty:
            raise AssertionError(f"relay 하네스가 {op} 에 {timeout}초 안에 답하지 않음") from None

    def claim(self, code: str, discord_id: str, guild_id: str) -> dict:
        return self.call("claim", code=code, discordId=discord_id, guildId=guild_id)

    def command(self, guild_id: str, text: str, requested_by: str, timeout: float = 90) -> dict:
        """디스코드 명령 대신 raw_command. 데몬 결과까지 기다린다 → {ok, result:{success,data}}"""
        return self.call("command", timeout=timeout, guildId=guild_id, text=text, requestedBy=requested_by)

    def host(self, host_id: str) -> dict:
        return self.call("host", hostId=host_id)

    def notices(self) -> list[dict]:
        return self.call("notices")["notices"]

    def close(self) -> None:
        try:
            self.proc.stdin.close()
            self.proc.wait(30)
        except Exception:
            self.proc.kill()
        self.log.close()
        if self.pg:
            self.pg.stop()
