"""saba-core를 실제 바이너리로 띄우고 HTTP API로 조작한다."""

import json
import re
import subprocess
import threading
import time
from pathlib import Path
from typing import Any, Callable

import requests

from .paths import IS_WINDOWS, binary
from .sandbox import Sandbox


class E2EError(AssertionError):
    pass


def wait_until(predicate: Callable[[], Any], timeout: float, what: str, interval: float = 0.25) -> Any:
    deadline = time.monotonic() + timeout
    last_exc: Exception | None = None
    while time.monotonic() < deadline:
        try:
            value = predicate()
            if value:
                return value
        except AssertionError as exc:
            if not isinstance(exc, E2EError):
                raise  # 시험이 확정한 실패(assert)는 기다리지 않고 바로 올린다
            last_exc = exc  # API 오류 응답 등 하네스 오류는 다시 본다
        except Exception as exc:  # 조건 함수 안의 일시 오류(연결 거부 등)는 다시 본다
            last_exc = exc
        time.sleep(interval)
    raise E2EError(f"{timeout:.0f}초 안에 되지 않음: {what}" + (f" (마지막 오류: {last_exc})" if last_exc else ""))


class Api:
    """X-Saba-Token을 붙이는 얇은 클라이언트. 응답 검사는 시험이 직접 한다."""

    def __init__(self, base: str, token_fn: Callable[[], str]):
        self.base = base
        self._token_fn = token_fn
        self.http = requests.Session()
        self.http.trust_env = False  # 클라우드 프록시를 타지 않게

    def request(self, method: str, path: str, *, token: bool = True, timeout: float = 30, **kw) -> requests.Response:
        headers = kw.pop("headers", {})
        if token:
            headers.setdefault("X-Saba-Token", self._token_fn())
        return self.http.request(method, self.base + path, headers=headers, timeout=timeout, **kw)

    def get(self, path: str, **kw) -> requests.Response:
        return self.request("GET", path, **kw)

    def post(self, path: str, body: Any = None, **kw) -> requests.Response:
        return self.request("POST", path, json=body, **kw)

    def put(self, path: str, body: Any = None, **kw) -> requests.Response:
        return self.request("PUT", path, json=body, **kw)

    def patch(self, path: str, body: Any = None, **kw) -> requests.Response:
        return self.request("PATCH", path, json=body, **kw)

    def delete(self, path: str, **kw) -> requests.Response:
        return self.request("DELETE", path, **kw)

    def ok(self, method: str, path: str, body: Any = None, expect: tuple[int, ...] = (200, 201), **kw) -> Any:
        resp = self.request(method, path, json=body, **kw)
        if resp.status_code not in expect:
            raise E2EError(f"{method} {path} → {resp.status_code}: {resp.text[:800]}")
        return resp.json() if resp.content else None


class Daemon:
    def __init__(self, sandbox: Sandbox, *, extra_env: dict[str, str] | None = None, args: list[str] | None = None, core: Path | None = None):
        self.sb = sandbox
        self.core = core or binary("saba-core")
        self.extra_env = extra_env or {}
        self.args = args or []
        self.proc: subprocess.Popen | None = None
        self.log_path = sandbox.artifacts / "daemon.log"
        self.api = Api(f"http://127.0.0.1:{sandbox.ipc_port}", sandbox.token)
        self._console_cursor: dict[str, int] = {}

    # ── 수명 ──
    def env(self) -> dict[str, str]:
        return self.sb.env(SABA_RUNNER_PATH=str(binary("saba-runner")), **self.extra_env)

    def spawn(self) -> subprocess.Popen:
        log = open(self.log_path, "ab")
        log.write(f"\n===== spawn {time.strftime('%H:%M:%S')} =====\n".encode())
        log.flush()
        flags = subprocess.CREATE_NEW_PROCESS_GROUP if IS_WINDOWS else 0
        self.proc = subprocess.Popen(
            [str(self.core), "--spawned", *self.args],
            env=self.env(),
            cwd=self.sb.root,
            stdin=subprocess.DEVNULL,
            stdout=log,
            stderr=subprocess.STDOUT,
            creationflags=flags,
            start_new_session=not IS_WINDOWS,
        )
        log.close()
        return self.proc

    def start(self, timeout: float = 60) -> "Daemon":
        self.spawn()
        self.wait_ready(timeout)
        return self

    def wait_ready(self, timeout: float = 60) -> None:
        def ready():
            if self.proc and self.proc.poll() is not None:
                raise E2EError(f"데몬이 기동 중 끝남 (종료 코드 {self.proc.returncode})\n{self.log_tail()}")
            r = self.api.get("/health", token=False, timeout=2)
            return r.ok and self.sb.token_path.exists()

        wait_until(ready, timeout, "데몬 /health")
        # 모듈 Python 실행 환경(venv)이 준비돼야 모듈 호출이 된다
        wait_until(lambda: self.api.get("/api/runtime/preflight", timeout=5).json().get("complete"), 300, "Python 런타임 준비")

    def shutdown(self, timeout: float = 30) -> int | None:
        if not self.proc or self.proc.poll() is not None:
            return self.proc.returncode if self.proc else None
        try:
            self.api.post("/api/daemon/shutdown", timeout=5)
        except requests.RequestException:
            pass
        try:
            return self.proc.wait(timeout)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            self.proc.wait(10)
            raise E2EError(f"데몬이 shutdown 뒤 {timeout}초 안에 끝나지 않음")

    def kill(self) -> None:
        if self.proc and self.proc.poll() is None:
            self.proc.kill()
            self.proc.wait(10)

    def alive(self) -> bool:
        return bool(self.proc) and self.proc.poll() is None

    def log_text(self) -> str:
        return self.log_path.read_text(encoding="utf-8", errors="replace") if self.log_path.exists() else ""

    def log_tail(self, n: int = 60) -> str:
        return "\n".join(self.log_text().splitlines()[-n:])

    # ── 인스턴스 ──
    def instances(self) -> list[dict]:
        return self.api.ok("GET", "/api/instances")

    def instance(self, iid: str) -> dict:
        for inst in self.instances():
            if inst["id"] == iid:
                return inst
        raise E2EError(f"인스턴스 없음: {iid}")

    def import_server(self, name: str, folder: Path, module: str = "minecraft", **extra) -> str:
        body = {"name": name, "module_name": module, "migration_source": str(folder), **extra}
        data = self.api.ok("POST", "/api/instances", body)
        return data["id"]

    def server_status(self, iid: str) -> str:
        for s in self.api.ok("GET", "/api/servers").get("servers", []):
            if s.get("id") == iid:
                return s.get("status", "")
        return ""

    def start_instance(self, iid: str) -> dict:
        body = self.api.ok("POST", f"/api/instance/{iid}/start")
        # 시작 실패도 200 + success:false로 오는 경로가 있다 (예: port_conflict)
        if isinstance(body, dict) and body.get("success") is False:
            raise E2EError(f"시작 실패: {body}")
        return body

    def stop_instance(self, iid: str, force: bool = False, timeout: float = 90) -> dict:
        return self.api.ok("POST", f"/api/instance/{iid}/stop", {"force": force}, timeout=timeout)

    def wait_status(self, iid: str, status: str, timeout: float = 60) -> None:
        wait_until(lambda: self.server_status(iid) == status, timeout, f"{iid} 상태 {status} (지금 {self.server_status(iid)!r})")

    # ── 콘솔 ──
    def console(self, iid: str, since: int | None = None, count: int = 500) -> list[dict]:
        q = f"?count={count}" + (f"&since={since}" if since is not None else "")
        data = self.api.ok("GET", f"/api/instance/{iid}/console{q}")
        return data.get("lines", []) if isinstance(data, dict) else []

    def console_text(self, iid: str) -> str:
        return "\n".join(line.get("content", "") for line in self.console(iid, count=5000))

    # 콘솔 줄 번호는 프로세스가 새로 뜰 때마다 0부터 다시 센다 (재시작·자동 재시작).
    # 그래서 첫 줄("Process started with PID …")이 바뀌면 새 실행으로 보고 처음부터 읽는다.
    def _session_key(self, lines: list[dict]) -> str:
        first = next((line for line in lines if line.get("id") == 0), None)
        return first.get("content", "") if first else ""

    def mark_console(self, iid: str) -> None:
        """지금까지의 콘솔을 읽은 것으로 표시한다. 다음 wait_console은 그 뒤 줄만 본다."""
        lines = self.console(iid, count=5000)
        self._console_cursor[iid] = (self._session_key(lines), max((line.get("id", 0) for line in lines), default=-1))

    def wait_console(self, iid: str, pattern: str, timeout: float = 60) -> str:
        rx = re.compile(pattern)
        seen: list[str] = []

        def check():
            lines = self.console(iid, count=5000)
            key = self._session_key(lines)
            session, cursor = self._console_cursor.get(iid, (key, -1))
            if key != session:
                cursor = -1
            fresh = [line for line in lines if line.get("id", 0) > cursor]
            self._console_cursor[iid] = (key, max([cursor] + [line.get("id", 0) for line in fresh]))
            for line in fresh:
                seen.append(line.get("content", ""))
                if rx.search(line.get("content", "")):
                    return line["content"]
            return None

        try:
            return wait_until(check, timeout, f"콘솔에 /{pattern}/")
        except E2EError as exc:
            raise E2EError(f"{exc}\n--- 콘솔 끝부분 ---\n" + "\n".join(seen[-30:])) from None

    def stdin(self, iid: str, command: str) -> dict:
        return self.api.ok("POST", f"/api/instance/{iid}/stdin", {"command": command})

    def exit_events(self, iid: str) -> list[dict]:
        data = self.api.ok("GET", f"/api/instance/{iid}/events")
        if isinstance(data, dict):
            return data.get("events", data.get("history", []))
        return data


class EventStream:
    """GET /api/events (SSE)를 백그라운드에서 읽어 쌓는다."""

    def __init__(self, daemon: Daemon, since: int | None = None, last_event_id: int | None = None):
        self.daemon = daemon
        self.events: list[dict] = []
        self._lock = threading.Lock()
        self._stop = threading.Event()
        self._resp: requests.Response | None = None
        path = "/api/events" + (f"?since={since}" if since is not None else "")
        headers = {"Last-Event-ID": str(last_event_id)} if last_event_id is not None else {}
        self._resp = daemon.api.get(path, stream=True, timeout=(5, None), headers=headers)
        if self._resp.status_code != 200:
            raise E2EError(f"/api/events → {self._resp.status_code}")
        self._thread = threading.Thread(target=self._run, daemon=True)
        self._thread.start()

    def _run(self) -> None:
        kind, data, eid = None, [], None
        try:
            for raw in self._resp.iter_lines(decode_unicode=True):
                if self._stop.is_set():
                    return
                if raw is None:
                    continue
                if raw == "":
                    if kind or data:
                        payload = "\n".join(data)
                        try:
                            parsed = json.loads(payload) if payload else None
                        except ValueError:
                            parsed = payload
                        with self._lock:
                            self.events.append({"type": kind, "id": eid, "data": parsed})
                    kind, data, eid = None, [], None
                elif raw.startswith(":"):
                    continue
                elif raw.startswith("event:"):
                    kind = raw[6:].strip()
                elif raw.startswith("data:"):
                    data.append(raw[5:].lstrip())
                elif raw.startswith("id:"):
                    eid = int(raw[3:].strip())
        except Exception:
            return

    def of_type(self, kind: str) -> list[dict]:
        with self._lock:
            return [e for e in self.events if e["type"] == kind]

    def wait_for(self, kind: str, predicate: Callable[[dict], bool] = lambda d: True, timeout: float = 60) -> dict:
        def check():
            for e in self.of_type(kind):
                if predicate(e["data"] or {}):
                    return e
            return None

        return wait_until(check, timeout, f"이벤트 {kind} (받은 것: {[e['type'] for e in self.events][-15:]})")

    def close(self) -> None:
        self._stop.set()
        if self._resp is not None:
            self._resp.close()
