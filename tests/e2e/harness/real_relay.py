"""실제 relay-server(server-chan, 비공개 저장소)를 임시 PostgreSQL 위에 띄운다.

relay 쪽 하네스는 server-chan `relay-server/test/e2e-harness.ts`에 있다(relay 내부 함수를 쓰므로 비공개 저장소에 둔다).
여기서는 그 스크립트를 실행하고, 디스코드 봇 대신 제어 포트로 페어링 클레임·명령 넣기를 한다.
리눅스에서 root로 돌면 embedded-postgres(initdb)가 거부하므로 시스템 PostgreSQL을 postgres 사용자로 띄운다.
"""

import json
import os
import shutil
import subprocess
import time
from pathlib import Path

import requests

from .paths import IS_WINDOWS, SERVER_CHAN_REPO
from .sandbox import free_port

RELAY_DIR = SERVER_CHAN_REPO / "relay-server"
HARNESS = RELAY_DIR / "test" / "e2e-harness.ts"


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
        self._as_pg([str(self.bin / "initdb"), "-D", str(self.dir / "data"), "-U", "saba", "--auth=trust", "-E", "UTF8"])
        self._as_pg([str(self.bin / "pg_ctl"), "-D", str(self.dir / "data"), "-o", f"-p {self.port} -k {self.dir} -h 127.0.0.1", "-l", str(self.dir / "log.txt"), "-w", "start"])
        self._as_pg([str(self.bin / "createdb"), "-h", "127.0.0.1", "-p", str(self.port), "-U", "saba", "saba_relay_e2e"])
        self.url = f"postgresql://saba@127.0.0.1:{self.port}/saba_relay_e2e"

    def _as_pg(self, argv: list[str]) -> None:
        subprocess.run(["runuser", "-u", "postgres", "--", *argv], check=True, capture_output=True, timeout=60)

    def stop(self) -> None:
        subprocess.run(["runuser", "-u", "postgres", "--", str(self.bin / "pg_ctl"), "-D", str(self.dir / "data"), "-m", "immediate", "stop"], capture_output=True, timeout=60)


class RealRelay:
    def __init__(self, root: Path):
        root.mkdir(parents=True, exist_ok=True)
        self.pg = None
        env = {k: v for k, v in os.environ.items() if k not in ("DATABASE_URL", "DISCORD_TOKEN", "DISCORD_BOT_TOKEN")}
        env.update(E2E_STATE_DIR=str(root), NODE_ENV="test")
        if not IS_WINDOWS and os.geteuid() == 0:
            self.pg = _SystemPostgres(root)
            env["DATABASE_URL"] = self.pg.url
        self.log = open(root / "relay.log", "w", encoding="utf-8")
        tsx = RELAY_DIR / "node_modules" / ".bin" / ("tsx.cmd" if IS_WINDOWS else "tsx")
        self.proc = subprocess.Popen([str(tsx), str(HARNESS)], cwd=RELAY_DIR, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.log, text=True)
        line = self._read_ready(120)
        info = json.loads(line)
        self.url, self.control = info["relayUrl"], info["controlUrl"]
        self.http = requests.Session()
        self.http.trust_env = False

    def _read_ready(self, timeout: float) -> str:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            line = self.proc.stdout.readline()
            if line.strip().startswith("{"):
                return line
            if self.proc.poll() is not None:
                break
        self.log.flush()
        raise RuntimeError("relay 하네스가 뜨지 않음:\n" + Path(self.log.name).read_text()[-3000:])

    def claim(self, code: str, discord_id: str, guild_id: str) -> dict:
        return self.http.post(f"{self.control}/claim", json={"code": code, "discordId": discord_id, "guildId": guild_id}, timeout=30).json()

    def submit(self, guild_id: str, payload: dict, requested_by: str) -> requests.Response:
        return self.http.post(f"{self.control}/command", json={"guildId": guild_id, "payload": payload, "requestedBy": requested_by}, timeout=30)

    def command(self, request_id: str) -> dict:
        return self.http.get(f"{self.control}/command/{request_id}", timeout=30).json()

    def node(self, guild_id: str) -> requests.Response:
        return self.http.get(f"{self.control}/node/{guild_id}", timeout=30)

    def requests_seen(self) -> list[dict]:
        return self.http.get(f"{self.control}/requests", timeout=30).json()

    def close(self) -> None:
        try:
            self.proc.stdin.close()
            self.proc.wait(30)
        except Exception:
            self.proc.kill()
        self.log.close()
        if self.pg:
            self.pg.stop()
