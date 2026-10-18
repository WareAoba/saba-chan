"""가짜 릴레이 서버. 데몬 릴레이 에이전트가 기대하는 계약을 실제 relay-server(server-chan)와 같은 규칙으로 흉내 낸다.

- 노드 요청: Authorization: Bearer sbn_<id>.<secret> + x-request-{timestamp,nonce,signature}
  signature = hex(HMAC-SHA256(secret, "METHOD\\nPATH\\nTS\\nNONCE\\nBODY")) — 서명이 틀리면 401로 거절해 데몬 쪽 서명을 검증한다.
- 페어링 상태 조회는 실제 relay처럼 x-poll-secret 헤더만 받는다(relay-server/src/routes/pair.ts).
"""

import hashlib
import hmac
import json
import queue
import secrets
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any
from urllib.parse import parse_qs, urlsplit

POLL_HOLD = 2.0


class FakeRelay:
    def __init__(self, node_id: str = "e2enode", secret: str | None = None):
        self.node_id = node_id
        self.secret = secret or secrets.token_hex(16)
        self.min_agent_version: str | None = None
        self.requests: list[dict] = []
        self.heartbeats: list[dict] = []
        self.results: dict[str, dict] = {}
        self.events: list[dict] = []
        self.signature_failures: list[str] = []
        self.pairings: dict[str, dict] = {}
        self._commands: "queue.Queue[dict]" = queue.Queue()
        self._lock = threading.Lock()
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), self._handler())
        self.server.daemon_threads = True
        self.port = self.server.server_address[1]
        self.url = f"http://127.0.0.1:{self.port}"
        self._thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self._thread.start()

    @property
    def node_token(self) -> str:
        return f"sbn_{self.node_id}.{self.secret}"

    # ── 시험이 부르는 것 ──
    def push_command(self, payload: dict, **meta: Any) -> str:
        cid = meta.pop("id", f"cmd-{secrets.token_hex(4)}")
        self._commands.put({"id": cid, "payload": payload, **meta})
        return cid

    def wait_result(self, cid: str, timeout: float = 60) -> dict:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            with self._lock:
                if cid in self.results:
                    return self.results[cid]
            time.sleep(0.2)
        raise AssertionError(f"릴레이가 {cid} 결과를 {timeout}초 안에 못 받음. 요청: {[r['path'] for r in self.requests][-20:]}")

    def claim_pairing(self, code: str) -> str:
        """디스코드에서 /사바쨩 연결 코드를 입력한 것처럼 처리한다."""
        with self._lock:
            self.pairings[code]["status"] = "claimed"
        return self.node_token

    def paths(self) -> list[str]:
        with self._lock:
            return [f"{r['method']} {r['path']}" for r in self.requests]

    def close(self) -> None:
        self.server.shutdown()
        self.server.server_close()

    # ── 서버 ──
    def _verify(self, method: str, path: str, headers, body: bytes) -> bool:
        auth = headers.get("Authorization", "")
        if auth != f"Bearer {self.node_token}":
            self.signature_failures.append(f"{method} {path}: bearer {auth[:20]!r}")
            return False
        ts, nonce, sig = (headers.get(h, "") for h in ("x-request-timestamp", "x-request-nonce", "x-request-signature"))
        if not ts or abs(time.time() - int(ts)) > 300:
            self.signature_failures.append(f"{method} {path}: timestamp {ts!r}")
            return False
        msg = f"{method}\n{path}\n{ts}\n{nonce}\n".encode() + body
        want = hmac.new(self.secret.encode(), msg, hashlib.sha256).hexdigest()
        if not hmac.compare_digest(want, sig):
            self.signature_failures.append(f"{method} {path}: signature mismatch")
            return False
        return True

    def _handler(self):
        relay = self

        class Handler(BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def log_message(self, *args):
                pass

            def _send(self, code: int, body: Any = None) -> None:
                data = b"" if body is None else json.dumps(body).encode()
                self.send_response(code)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

            def _route(self, method: str) -> None:
                length = int(self.headers.get("Content-Length") or 0)
                body = self.rfile.read(length) if length else b""
                parts = urlsplit(self.path)
                path = parts.path
                record = {"method": method, "path": path, "query": parts.query, "headers": dict(self.headers), "body": body.decode(errors="replace")}
                with relay._lock:
                    relay.requests.append(record)
                parsed = json.loads(body) if body else None

                # 서명 없는 공개 경로
                if method == "GET" and path == "/info":
                    return self._send(200, {"minAgentVersion": relay.min_agent_version} if relay.min_agent_version else {"ok": True})
                if method == "POST" and path == "/api/pair/initiate":
                    code = f"{secrets.randbelow(10**6):06d}"
                    poll_secret = secrets.token_hex(8)
                    with relay._lock:
                        relay.pairings[code] = {"status": "pending", "pollSecret": poll_secret}
                    return self._send(200, {"code": code, "pollSecret": poll_secret, "expiresAt": "2099-01-01T00:00:00Z", "ttl": 300})
                if method == "GET" and path.startswith("/api/pair/") and path.endswith("/status"):
                    code = path.split("/")[3]
                    got = self.headers.get("x-poll-secret")
                    if not got:
                        return self._send(401, {"error": "MISSING_POLL_SECRET"})
                    row = relay.pairings.get(code)
                    if not row or row["pollSecret"] != got:
                        return self._send(404, {"error": "NOT_FOUND"})
                    if row["status"] == "claimed":
                        return self._send(200, {"status": "claimed", "nodeToken": relay.node_token})
                    return self._send(200, {"status": row["status"], "expiresAt": "2099-01-01T00:00:00Z"})

                if not relay._verify(method, path, self.headers, body):
                    return self._send(401, {"error": "INVALID_SIGNATURE"})
                if method == "POST" and path == "/heartbeat":
                    with relay._lock:
                        relay.heartbeats.append(parsed)
                    return self._send(200, {"status": "ok"})
                if method == "GET" and path == "/poll":
                    try:
                        cmd = relay._commands.get(timeout=POLL_HOLD)
                    except queue.Empty:
                        self.send_response(204)
                        self.send_header("Content-Length", "0")
                        self.end_headers()
                        return None
                    return self._send(200, {"commands": [cmd]})
                if method == "POST" and path.startswith("/result/"):
                    from urllib.parse import unquote

                    with relay._lock:
                        relay.results[unquote(path[len("/result/"):])] = parsed
                    return self._send(200, {"ok": True})
                if method == "POST" and path == "/events":
                    with relay._lock:
                        relay.events.extend(parsed.get("events", []))
                    return self._send(200, {"accepted": len(parsed.get("events", []))})
                return self._send(404, {"error": "NOT_FOUND"})

            def do_GET(self):
                self._route("GET")

            def do_POST(self):
                self._route("POST")

            def do_PUT(self):
                self._route("PUT")

        return Handler


def query(record: dict) -> dict:
    return parse_qs(record.get("query", ""))
