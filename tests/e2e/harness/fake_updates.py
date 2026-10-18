"""가짜 업데이트 relay (/v1/updates/*). scripts/mock-release-server.js와 같은 공개 시험 키로 manifest에 서명한다.

이 키(saba-updater-mock)는 `--features mock-relay`로 빌드한 데몬·업데이터만 믿는다. 릴리스 빌드는 믿지 않는다.
"""

import base64
import hashlib
import io
import json
import threading
import zipfile
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlsplit

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

MOCK_KEY_ID = "saba-updater-mock"
MOCK_SEED = b"saba-mock-relay-not-for-release!"


def canonical_json(value) -> str:
    """updater signature::canonical_json과 같은 정규형: 키 정렬, 공백 없음."""
    if isinstance(value, list):
        return "[" + ",".join(canonical_json(v) for v in value) + "]"
    if isinstance(value, dict):
        return "{" + ",".join(f"{json.dumps(k, ensure_ascii=False)}:{canonical_json(value[k])}" for k in sorted(value)) + "}"
    return json.dumps(value, ensure_ascii=False)


def sign(manifest: dict) -> dict:
    key = Ed25519PrivateKey.from_private_bytes(MOCK_SEED)
    sig = key.sign(canonical_json(manifest).encode())
    return {"manifest": manifest, "signature": base64.b64encode(sig).decode(), "keyId": MOCK_KEY_ID}


def make_zip(files: dict[str, bytes]) -> bytes:
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w") as z:
        for name, data in files.items():
            z.writestr(name, data)
    return buf.getvalue()


class FakeUpdates:
    def __init__(self, platform: str):
        self.platform = platform
        self.version = "0.1.6"
        self.assets: dict[str, bytes] = {}
        self.components: dict[str, dict] = {}
        self.tamper = False  # 서명한 뒤 manifest를 바꾼다
        self.requests: list[dict] = []
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), self._handler())
        self.server.daemon_threads = True
        self.url = f"http://127.0.0.1:{self.server.server_address[1]}"
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def add_component(self, key: str, version: str, files: dict[str, bytes], sha256: str | None = None) -> None:
        asset = f"{key}-{version}-{self.platform}.zip"
        data = make_zip(files)
        self.assets[asset] = data
        self.components[key] = {
            "version": version,
            "asset": asset,
            "sha256": sha256 or hashlib.sha256(data).hexdigest(),
            "url": f"{self.url}/download/{asset}",
            "size_bytes": len(data),
        }

    def manifest(self) -> dict:
        return {"version": self.version, "channel": "stable", "platform": self.platform, "released_at": "2026-10-18T00:00:00Z", "is_critical": False, "components": self.components}

    def close(self) -> None:
        self.server.shutdown()

    def _handler(self):
        fake = self

        class H(BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def log_message(self, *a):
                pass

            def _send(self, code, body: bytes, ctype="application/json"):
                self.send_response(code)
                self.send_header("Content-Type", ctype)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def _json(self, code, value):
                self._send(code, json.dumps(value).encode())

            def do_GET(self):
                u = urlsplit(self.path)
                q = parse_qs(u.query)
                fake.requests.append({"path": u.path, "query": q, "headers": dict(self.headers)})
                # relay API만 클라이언트 헤더를 본다 (에셋은 운영에서 R2가 준다)
                if u.path.startswith("/v1/") and self.headers.get("X-Saba-Client") != "1":
                    return self._json(403, {"error": "INVALID_ACCESS"})
                if u.path == "/v1/updates/check":
                    current = q.get("current", ["0.0.0"])[0]
                    latest = {"version": fake.version, "channel": "stable", "notes": {"ko": "e2e 릴리스 노트", "en": "e2e notes"}, "manifestUrl": f"{fake.url}/v1/updates/manifest/{fake.version}"}
                    return self._json(200, {"upToDate": current == fake.version, "latest": latest})
                if u.path.startswith("/v1/updates/manifest/"):
                    signed = sign(fake.manifest())
                    if fake.tamper:
                        signed["manifest"]["components"] = {k: {**v, "url": v["url"] + "?evil=1"} for k, v in signed["manifest"]["components"].items()}
                    return self._json(200, signed)
                if u.path.startswith("/download/"):
                    data = fake.assets.get(u.path[len("/download/"):])
                    return self._send(200, data, "application/zip") if data else self._json(404, {"error": "NOT_FOUND"})
                return self._json(404, {"error": "NOT_FOUND"})

            def do_POST(self):
                length = int(self.headers.get("Content-Length") or 0)
                self.rfile.read(length)
                fake.requests.append({"path": self.path, "headers": dict(self.headers)})
                self._json(200, {"ok": True})

        return H
