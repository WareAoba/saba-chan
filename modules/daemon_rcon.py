"""
saba-chan daemon helpers for Python lifecycle modules
=====================================================
Shared runtime helpers (every module may use them; the daemon ships this file).
The file keeps its old name so existing ``from daemon_rcon import ...`` lines work;
it holds no RCON client. A module whose server only speaks RCON sends its stop
command with ``daemon_post(f"/api/instance/{id}/rcon", {"command": ...})``.

    daemon_post()                                call any daemon endpoint with the IPC token
    daemon_stdin_command()                       send a line to a managed server's stdin
    daemon_rest()                                send a REST request through the daemon
    loopback_exchange() / loopback_http()        talk to this instance's own port on 127.0.0.1
    is_process_alive() / wait_for_exit() / force_kill()   PID-safe process checks and kills
    config_value()                               top level -> module_settings -> settings lookup
    load_setting_types() / coerce_setting()      cast file values by module.toml field type
    run_lifecycle()                              the `python lifecycle.py <function>` entry point

Environment (the daemon sets these for every Python run):
    SABA_IPC_URL     — base URL of the daemon (DAEMON_API_URL still overrides it)
    SABA_TOKEN_PATH  — file holding the X-Saba-Token value
    SABA_HOST_DIR    — replies to instance-bound HOST requests (0.1.10+)
    SABA_LOOPBACK_DIR — set only for sandboxed runs, which cannot reach 127.0.0.1: the loopback
                        helpers then ask the daemon to make the connection (its own ports only)
"""

import json
import math
import ntpath
import os
from pathlib import Path
import re
import subprocess
import sys
import time
import urllib.request
import urllib.error

# ─── Constants ────────────────────────────────────────────────

_DEFAULT_DAEMON_URL = "http://127.0.0.1:57474"

SINCE = {
    "daemon_post": "0.1.2",
    "daemon_stdin_command": "0.1.2",
    "is_process_alive": "0.1.2",
    "wait_for_exit": "0.1.2",
    "force_kill": "0.1.2",
    "config_value": "0.1.2",
    "load_setting_types": "0.1.2",
    "coerce_setting": "0.1.2",
    "run_lifecycle": "0.1.2",
    "loopback_exchange": "0.1.8",
    "loopback_http": "0.1.8",
    "i18n.I18n": "0.1.1",
    "safe_join": "0.1.10",
    "PathOutside": "0.1.10",
    "daemon_rest": "0.1.10",
}


class PathOutside(ValueError):
    """허용 루트 밖으로 나가는 경로."""

    def __init__(self, path):
        self.path = str(path)
        super().__init__(f"Path is outside root: {self.path}")


def safe_join(root, *parts):
    """이름 조각을 루트 안의 실제 경로로 합친다."""
    path = Path(root).joinpath(*parts)
    for part in parts:
        name = os.fspath(part)
        if name == ".." or "/" in name or "\\" in name or ntpath.splitdrive(name)[0]:
            raise PathOutside(path)
    root = Path(root).resolve()
    path = path.resolve()
    if not path.is_relative_to(root):
        raise PathOutside(path)
    return path


def _daemon_url():
    """Resolve the daemon API base URL."""
    return (
        os.environ.get("DAEMON_API_URL")
        or os.environ.get("SABA_IPC_URL")
        or _DEFAULT_DAEMON_URL
    )


def _daemon_headers():
    """JSON headers plus the daemon auth token when one is available."""
    headers = {"Content-Type": "application/json"}
    token_path = os.environ.get("SABA_TOKEN_PATH")
    if token_path:
        try:
            with open(token_path, encoding="utf-8") as f:
                token = f.read().strip()
            if token:
                headers["X-Saba-Token"] = token
        except OSError:
            pass
    return headers


def daemon_post(path, payload, timeout=10):
    """POST JSON to a daemon endpoint with auth and return the decoded reply.

    Raises:
        ConnectionError: If the daemon is unreachable or answers with an HTTP error.
    """
    url = f"{_daemon_url()}{path}"
    req = urllib.request.Request(
        url,
        data=json.dumps(payload).encode("utf-8"),
        headers=_daemon_headers(),
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            return json.loads(resp.read().decode("utf-8"))
    except urllib.error.HTTPError as e:
        body = e.read().decode("utf-8", errors="replace")
        raise ConnectionError(f"Daemon request failed (HTTP {e.code}): {body}") from e
    except urllib.error.URLError as e:
        raise ConnectionError(f"Daemon unreachable at {url}: {e}") from e


def daemon_stdin_command(instance_id, command, *, timeout=10):
    """Send a line to a managed server's stdin via ``/api/instance/:id/stdin``."""
    if os.environ.get("SABA_HOST_DIR"):
        return _host_request("instance.stdin", {"command": command}, timeout)
    return daemon_post(f"/api/instance/{instance_id}/stdin", {"command": command}, timeout=timeout)


def daemon_rest(instance_id, method, path, *, body=None, timeout=10):
    """Send an instance REST request through HOST or the daemon HTTP API."""
    args = {"method": method, "path": path}
    if body is not None:
        args["body"] = body
    if os.environ.get("SABA_HOST_DIR"):
        return _host_request("instance.rest", args, timeout)
    payload = {"method": method, "endpoint": path}
    if body is not None:
        payload["body"] = body
    return daemon_post(f"/api/instance/{instance_id}/rest", payload, timeout=timeout)


def _host_error(code, message):
    error = ConnectionError(f"Daemon host request failed ({code}): {message}")
    error.error_code = code
    return error


def _host_request(op, args, timeout):
    import uuid
    request_id = uuid.uuid4().hex
    request = "HOST:" + json.dumps({"id": request_id, "op": op, "args": args}, ensure_ascii=False, allow_nan=False)
    if len(request.encode("utf-8")) > 64 * 1024:
        raise _host_error("request_too_large", "request exceeds 64 KiB")
    wait = float(timeout)
    if not math.isfinite(wait) or wait < 0:
        raise ValueError("timeout must be finite and non-negative")
    reply_path = Path(os.environ["SABA_HOST_DIR"]) / f"{request_id}.json"
    deadline = time.monotonic() + min(wait, 30)
    print(request, file=sys.stderr, flush=True)
    while not reply_path.exists():
        if time.monotonic() >= deadline:
            raise TimeoutError(f"no reply from the daemon for {op}")
        time.sleep(0.02)
    try:
        with reply_path.open("rb") as reply_file:
            raw = reply_file.read(1024 * 1024 + 1)
        if len(raw) > 1024 * 1024:
            raise _host_error("response_too_large", "response exceeds 1 MiB")
        try:
            reply = json.loads(raw)
        except (UnicodeError, ValueError) as error:
            raise _host_error("invalid_response", "response is not JSON") from error
        if not isinstance(reply, dict) or not isinstance(reply.get("ok"), bool):
            raise _host_error("invalid_response", "response has no boolean ok field")
        if not reply["ok"]:
            raise _host_error(reply.get("error_code") or "host_failed", reply.get("error") or "host request failed")
        if "data" not in reply:
            raise _host_error("invalid_response", "response has no data field")
        return reply["data"]
    finally:
        try:
            reply_path.unlink()
        except OSError:
            pass


# ─── Loopback ─────────────────────────────────────────────────
# A sandboxed run (AppContainer) cannot open 127.0.0.1. The daemon relays requests to this
# instance's own ports: the request goes out as one stderr line, the reply comes back as a file.

def _read_until_done(sock, idle_ms, limit=1024 * 1024):
    chunks, size = [], 0
    while size < limit:
        if idle_ms is not None and chunks:
            sock.settimeout(idle_ms / 1000)
        try:
            chunk = sock.recv(8192)
        except TimeoutError:
            if idle_ms is not None and chunks:
                break
            raise
        if not chunk:
            break
        chunks.append(chunk)
        size += len(chunk)
    return b"".join(chunks)[:limit]


def loopback_exchange(port, data, *, timeout=5, idle_ms=None):
    """Send ``data`` to ``127.0.0.1:port`` and return what comes back.

    Reading stops when the server closes the connection, or, with ``idle_ms``, once the
    server has been quiet that long after answering (for protocols that keep the
    connection open). Raises OSError on failure, including a refused port.
    """
    relay = os.environ.get("SABA_LOOPBACK_DIR")
    if not relay:
        import socket
        with socket.create_connection(("127.0.0.1", int(port)), timeout=timeout) as sock:
            sock.sendall(data)
            return _read_until_done(sock, idle_ms)
    import base64
    import uuid
    request_id = uuid.uuid4().hex
    request = {"id": request_id, "port": int(port), "data": base64.b64encode(data).decode("ascii"), "timeout": timeout}
    if idle_ms is not None:
        request["idle_ms"] = int(idle_ms)
    print("LOOPBACK:" + json.dumps(request), file=sys.stderr, flush=True)
    reply_path = Path(relay) / f"{request_id}.json"
    deadline = time.monotonic() + timeout + 2
    while not reply_path.exists():
        if time.monotonic() > deadline:
            raise TimeoutError(f"no reply from the daemon for 127.0.0.1:{port}")
        time.sleep(0.02)
    try:
        reply = json.loads(reply_path.read_text(encoding="utf-8"))
    finally:
        try:
            reply_path.unlink()
        except OSError:
            pass
    if not reply.get("ok"):
        raise OSError(f"127.0.0.1:{port}: {reply.get('error')}")
    return base64.b64decode(reply.get("data") or "")


def loopback_http(port, method, path, *, headers=None, body=None, timeout=5):
    """HTTP/1.1 request to ``127.0.0.1:port`` through ``loopback_exchange``.

    Returns ``(status, body_bytes)``. Raises OSError on connection failures or an
    unreadable response.
    """
    body = body or b""
    if isinstance(body, str):
        body = body.encode("utf-8")
    lines = [f"{method} {path} HTTP/1.1", f"Host: 127.0.0.1:{port}", "Connection: close",
             f"Content-Length: {len(body)}"]
    lines += [f"{k}: {v}" for k, v in (headers or {}).items()]
    raw = loopback_exchange(port, ("\r\n".join(lines) + "\r\n\r\n").encode("utf-8") + body, timeout=timeout)
    head, sep, payload = raw.partition(b"\r\n\r\n")
    status_line = head.split(b"\r\n", 1)[0].decode("latin-1")
    parts = status_line.split(" ", 2)
    if not sep or len(parts) < 2 or not parts[1].isdigit():
        raise OSError(f"127.0.0.1:{port}: unreadable HTTP response")
    if b"transfer-encoding: chunked" in head.lower():
        decoded, rest = b"", payload
        while rest:
            size_line, _, rest = rest.partition(b"\r\n")
            size = int(size_line.split(b";")[0] or b"0", 16)
            if size == 0:
                break
            decoded, rest = decoded + rest[:size], rest[size + 2:]
        payload = decoded
    return int(parts[1]), payload


# ─── Processes ────────────────────────────────────────────────

_WIN_PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
_WIN_STILL_ACTIVE = 259
_WIN_NO_WINDOW = 0x08000000


def is_process_alive(pid):
    """PID 프로세스가 살아 있는지 확인한다.

    Windows에서 `os.kill(pid, 0)`은 TerminateProcess를 호출해 프로세스를
    종료시키므로 OpenProcess/GetExitCodeProcess로만 확인한다.
    """
    try:
        pid = int(pid)
    except (TypeError, ValueError):
        return False
    if pid <= 0:
        return False

    if os.name == "nt":
        import ctypes
        from ctypes import wintypes

        kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel32.OpenProcess.restype = wintypes.HANDLE
        kernel32.OpenProcess.argtypes = (wintypes.DWORD, wintypes.BOOL, wintypes.DWORD)
        kernel32.GetExitCodeProcess.argtypes = (wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD))
        kernel32.CloseHandle.argtypes = (wintypes.HANDLE,)
        handle = kernel32.OpenProcess(_WIN_PROCESS_QUERY_LIMITED_INFORMATION, False, pid)
        if not handle:
            # 접근 거부(5)는 프로세스가 존재함을 뜻한다.
            return ctypes.get_last_error() == 5
        try:
            code = wintypes.DWORD()
            if not kernel32.GetExitCodeProcess(handle, ctypes.byref(code)):
                return False
            return code.value == _WIN_STILL_ACTIVE
        finally:
            kernel32.CloseHandle(handle)

    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    except OSError:
        return False
    return True


def wait_for_exit(pid, timeout=30, interval=1.0):
    """프로세스가 종료될 때까지 최대 timeout초 기다린다. 종료되면 True."""
    deadline = time.monotonic() + timeout
    while is_process_alive(pid):
        if time.monotonic() >= deadline:
            return False
        time.sleep(interval)
    return True


def _descendants(pid):
    """pid의 자손 PID를 맨 아래부터 (Unix, pgrep 사용). 못 구하면 빈 목록"""
    try:
        out = subprocess.run(["pgrep", "-P", str(pid)], capture_output=True, text=True, timeout=5).stdout
    except (OSError, subprocess.SubprocessError):
        return []
    result = []
    for child in (int(p) for p in out.split() if p.isdigit()):
        result.extend(_descendants(child))
        result.append(child)
    return result


def force_kill(pid, *, tree=True):
    """프로세스를 강제 종료한다. 반환: (성공 여부, 오류 메시지 또는 None)."""
    try:
        pid = int(pid)
    except (TypeError, ValueError):
        return False, f"invalid pid: {pid!r}"
    if not is_process_alive(pid):
        return True, None
    try:
        if os.name == "nt":
            args = ["taskkill", "/F", "/PID", str(pid)]
            if tree:
                args.insert(2, "/T")
            result = subprocess.run(args, capture_output=True, text=True,
                                    timeout=15, creationflags=_WIN_NO_WINDOW)
            if result.returncode != 0 and is_process_alive(pid):
                return False, (result.stderr or result.stdout).strip()
        else:
            import signal
            # tree: 자식부터(맨 아래부터) 끝낸다. 부모를 먼저 죽이면 손자가 고아가 되어 남는다
            for target in (_descendants(pid) if tree else []) + [pid]:
                try:
                    os.kill(target, signal.SIGKILL)
                except ProcessLookupError:
                    pass
    except (OSError, subprocess.SubprocessError) as e:
        if is_process_alive(pid):
            return False, str(e)
    return True, None


# ─── 설정 ─────────────────────────────────────────────────────

def config_value(config, key, default=None):
    """설정 값을 최상위 → module_settings → settings 순서로 찾는다.

    데몬은 함수별로 module_settings를 평탄화해 보내거나 보내지 않으므로
    모듈은 이 함수로 세 위치를 모두 확인한다. 빈 문자열/None은 없는 값이다.
    """
    sources = [config]
    for nested in ("module_settings", "settings"):
        value = config.get(nested) if isinstance(config, dict) else None
        if isinstance(value, dict):
            sources.append(value)
    for source in sources:
        value = source.get(key) if isinstance(source, dict) else None
        if value is not None and value != "":
            return value
    return default


def load_setting_types(module_dir):
    """module.toml `[[settings.fields]]`에서 {name: type} 맵을 읽는다.

    type이 없으면 데몬과 같게 "text"로 본다. 파일이 없거나 파싱할 수 없으면 {}.
    """
    try:
        import tomllib
        raw = (Path(module_dir) / "module.toml").read_bytes()
        data = tomllib.loads(raw.decode("utf-8-sig"))
    except (ImportError, OSError, UnicodeError, ValueError):
        return {}
    fields = data.get("settings", {}).get("fields", [])
    return {f["name"]: f.get("type", "text") for f in fields if isinstance(f, dict) and "name" in f}


_INT_RE = re.compile(r"^[+-]?\d+$")


def coerce_setting(value, field_type):
    """설정 파일의 문자열 값을 module.toml 필드 타입에 맞게 변환한다.

    number → int/float(유한수만), boolean → bool. 그 외(text/password/select/미선언)는
    받은 문자열을 그대로 둔다 — 앞뒤 공백도, 숫자 같은 비밀번호("000123")도 바꾸지 않는다.
    """
    if field_type not in ("number", "boolean"):
        return value if isinstance(value, str) else str(value)
    text = str(value).strip()
    if field_type == "boolean":
        lowered = text.lower()
        if lowered in ("true", "false"):
            return lowered == "true"
        return text
    if _INT_RE.match(text):
        return int(text)
    try:
        number = float(text)
    except ValueError:
        return text
    # inf/nan은 JSON으로 못 보낸다 (데몬이 파싱에 실패한다): 문자열로 둔다
    if not math.isfinite(number):
        return text
    return int(number) if number.is_integer() and "." not in text and "e" not in text.lower() else number


# ─── CLI 진입점 ───────────────────────────────────────────────

def run_lifecycle(functions):
    """`python lifecycle.py <function>` 진입점.

    stdin의 JSON config를 함수에 넘기고 결과 JSON 한 개를 stdout에 출력한다.
    알 수 없는 함수나 예외도 JSON으로 응답하고 종료 코드 0을 유지한다
    (데몬은 0이 아닌 종료 코드를 HTTP 500으로 바꾼다).
    """
    def emit(result):
        try:
            output = json.dumps(result, ensure_ascii=False, allow_nan=False)
        except Exception as e:
            output = json.dumps({"success": False, "error_code": "module_bad_output", "message": str(e)}, ensure_ascii=False)
        sys.stdout.write(output + "\n")
        sys.stdout.flush()

    if len(sys.argv) < 2:
        emit({"success": False, "message": "Usage: lifecycle.py <function>"})
        return

    name = sys.argv[1]
    # 예약 함수: 데몬이 구현된 함수 목록을 묻는다 (자신은 넣지 않는다)
    if name == "functions" and name not in functions:
        emit({"success": True, "functions": list(functions)})
        return
    fn = functions.get(name)
    if fn is None:
        emit({"success": False, "message": f"Unknown function: {name}", "available": list(functions)})
        return

    try:
        raw = sys.stdin.read()
        config = json.loads(raw) if raw.strip() else {}
    except json.JSONDecodeError as e:
        emit({"success": False, "message": f"Invalid JSON config on stdin: {e}"})
        return

    try:
        result = fn(config)
    except PathOutside as e:
        result = {"success": False, "error_code": "module_path_outside", "message": str(e), "path": e.path}
    except Exception as e:  # 모듈 함수 오류도 JSON으로 돌려준다
        import traceback
        print(traceback.format_exc(), file=sys.stderr)
        result = {"success": False, "error_code": "module_exception", "message": f"Error in {name}: {e}"}
    emit(result)
