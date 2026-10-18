"""외부 다운로드를 고정 fixture로 바꿔 주는 HTTPS 가로채기 프록시.

모듈은 Mojang·Paper·Purpur·Fabric·Adoptium 주소를 코드에 박아 두고 urllib로 받는다. urllib는 HTTPS_PROXY와
SSL_CERT_FILE을 따르므로, 시험용 CA를 만들고 이 프록시를 데몬 환경에 넣으면 제품 코드를 바꾸지 않고 설치 경로 전체를
외부망 없이 돌릴 수 있다. 등록하지 않은 주소는 502로 거절하고 기록한다(시험이 몰래 외부로 나가는지도 이것으로 본다).
"""

import datetime
import hashlib
import ipaddress
import socket
import ssl
import threading
from dataclasses import dataclass, field
from pathlib import Path
from typing import Callable

from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.x509.oid import NameOID

Handler = Callable[[str, str], tuple[int, bytes, str]]  # (method, path) -> (status, body, content-type)


def _name(cn: str) -> x509.Name:
    return x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, cn)])


@dataclass
class Mirror:
    root: Path
    routes: dict[tuple[str, str], Handler] = field(default_factory=dict)
    seen: list[str] = field(default_factory=list)
    refused: list[str] = field(default_factory=list)

    def __post_init__(self):
        self.root.mkdir(parents=True, exist_ok=True)
        now = datetime.datetime.now(datetime.timezone.utc)
        self._ca_key = ec.generate_private_key(ec.SECP256R1())
        self._ca = (
            x509.CertificateBuilder()
            .subject_name(_name("saba-e2e mirror CA"))
            .issuer_name(_name("saba-e2e mirror CA"))
            .public_key(self._ca_key.public_key())
            .serial_number(x509.random_serial_number())
            .not_valid_before(now - datetime.timedelta(days=1))
            .not_valid_after(now + datetime.timedelta(days=2))
            .add_extension(x509.BasicConstraints(ca=True, path_length=None), critical=True)
            .add_extension(x509.KeyUsage(digital_signature=True, key_cert_sign=True, crl_sign=True, content_commitment=False, key_encipherment=False, data_encipherment=False, key_agreement=False, encipher_only=False, decipher_only=False), critical=True)
            .add_extension(x509.SubjectKeyIdentifier.from_public_key(self._ca_key.public_key()), critical=False)
            .sign(self._ca_key, hashes.SHA256())
        )
        self.ca_file = self.root / "mirror-ca.pem"
        self.ca_file.write_bytes(self._ca.public_bytes(serialization.Encoding.PEM))
        self._contexts: dict[str, ssl.SSLContext] = {}
        self._lock = threading.Lock()
        self._sock = socket.create_server(("127.0.0.1", 0))
        self.port = self._sock.getsockname()[1]
        self.url = f"http://127.0.0.1:{self.port}"
        self._closed = False
        threading.Thread(target=self._accept, daemon=True).start()

    # ── 라우트 ──
    def file(self, url: str, body: bytes, content_type: str = "application/octet-stream") -> None:
        host, path = _split(url)
        self.routes[(host, path)] = lambda m, p: (200, body, content_type)

    def json(self, url: str, value) -> None:
        import json as _json

        self.file(url, _json.dumps(value).encode(), "application/json")

    def handler(self, url: str, fn: Handler) -> None:
        self.routes[_split(url)] = fn

    def env(self) -> dict[str, str]:
        """데몬(과 그 자식 Python)에 넣을 환경 변수."""
        return {
            "HTTPS_PROXY": self.url, "https_proxy": self.url,
            "HTTP_PROXY": self.url, "http_proxy": self.url,
            "NO_PROXY": "127.0.0.1,localhost", "no_proxy": "127.0.0.1,localhost",
            "SSL_CERT_FILE": str(self.ca_file), "REQUESTS_CA_BUNDLE": str(self.ca_file),
        }

    def close(self) -> None:
        self._closed = True
        self._sock.close()

    # ── 프록시 ──
    def _context(self, host: str) -> ssl.SSLContext:
        with self._lock:
            if host in self._contexts:
                return self._contexts[host]
            key = ec.generate_private_key(ec.SECP256R1())
            now = datetime.datetime.now(datetime.timezone.utc)
            try:
                san = x509.IPAddress(ipaddress.ip_address(host))
            except ValueError:
                san = x509.DNSName(host)
            cert = (
                x509.CertificateBuilder()
                .subject_name(_name(host))
                .issuer_name(self._ca.subject)
                .public_key(key.public_key())
                .serial_number(x509.random_serial_number())
                .not_valid_before(now - datetime.timedelta(days=1))
                .not_valid_after(now + datetime.timedelta(days=2))
                .add_extension(x509.SubjectAlternativeName([san]), critical=False)
                .add_extension(x509.ExtendedKeyUsage([x509.oid.ExtendedKeyUsageOID.SERVER_AUTH]), critical=False)
                .add_extension(x509.AuthorityKeyIdentifier.from_issuer_public_key(self._ca_key.public_key()), critical=False)
                .add_extension(x509.SubjectKeyIdentifier.from_public_key(key.public_key()), critical=False)
                .sign(self._ca_key, hashes.SHA256())
            )
            pem = self.root / f"leaf-{hashlib.sha1(host.encode()).hexdigest()[:10]}.pem"
            pem.write_bytes(
                cert.public_bytes(serialization.Encoding.PEM)
                + key.private_bytes(serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8, serialization.NoEncryption())
            )
            ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
            ctx.load_cert_chain(pem)
            self._contexts[host] = ctx
            return ctx

    def _accept(self) -> None:
        while not self._closed:
            try:
                conn, _ = self._sock.accept()
            except OSError:
                return
            threading.Thread(target=self._serve, args=(conn,), daemon=True).start()

    def _serve(self, conn: socket.socket) -> None:
        try:
            head = _read_head(conn)
            if not head:
                return
            method, target, _ = head[0].split(" ", 2)
            if method == "CONNECT":
                host = target.rsplit(":", 1)[0]
                conn.sendall(b"HTTP/1.1 200 Connection established\r\n\r\n")
                tls = self._context(host).wrap_socket(conn, server_side=True)
                while True:
                    req = _read_head(tls)
                    if not req:
                        return
                    m, p, _ = req[0].split(" ", 2)
                    _drain_body(tls, req)
                    if not self._respond(tls, host, m, p):
                        return
            else:  # 평문 HTTP 프록시 요청: 절대 URL
                host, path = _split(target)
                self._respond(conn, host, method, path)
        except (OSError, ssl.SSLError, ValueError):
            return
        finally:
            try:
                conn.close()
            except OSError:
                pass

    def _respond(self, sock, host: str, method: str, path: str) -> bool:
        key = f"{host}{path}"
        handler = self.routes.get((host, path)) or self.routes.get((host, path.split("?", 1)[0]))
        with self._lock:
            (self.seen if handler else self.refused).append(f"{method} https://{key}")
        status, body, ctype = handler(method, path) if handler else (502, b"not mirrored by saba e2e", "text/plain")
        reason = {200: "OK", 404: "Not Found", 502: "Bad Gateway"}.get(status, "X")
        sock.sendall(f"HTTP/1.1 {status} {reason}\r\nContent-Type: {ctype}\r\nContent-Length: {len(body)}\r\n\r\n".encode() + body)
        return True


def _split(url: str) -> tuple[str, str]:
    from urllib.parse import urlsplit

    u = urlsplit(url)
    return u.hostname, (u.path or "/") + (f"?{u.query}" if u.query else "")


def _read_head(sock) -> list[str]:
    data = b""
    while b"\r\n\r\n" not in data:
        chunk = sock.recv(4096)
        if not chunk:
            return []
        data += chunk
        if len(data) > 65536:
            return []
    head, rest = data.split(b"\r\n\r\n", 1)
    lines = head.decode("latin-1").split("\r\n")
    lines.append(rest.decode("latin-1"))  # 마지막 원소: 이미 읽은 본문 앞부분
    return lines


def _drain_body(sock, head: list[str]) -> None:
    length = next((int(h.split(":", 1)[1]) for h in head[1:-1] if h.lower().startswith("content-length:")), 0)
    remaining = length - len(head[-1].encode("latin-1"))
    while remaining > 0:
        chunk = sock.recv(min(remaining, 65536))
        if not chunk:
            return
        remaining -= len(chunk)
