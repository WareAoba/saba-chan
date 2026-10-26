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
    atomic_write()                               replace a whole file without leaving it half written
    setting_file_keys() / settings_to_file() / settings_from_file()
                                                 setting name <-> game file key from module.toml `file_key`
    read_key_values() / update_key_values()      `key=value` config files, keeping comments, order, newlines, encoding
    graceful_stop() / request_console() / request_terminate() / stop_timeout()
                                                 the save-then-stop procedure of a module `stop()`
    load_diagnose_rules() / diagnose_lines() / tail_lines()
                                                 log diagnosis from module.toml `[[diagnose]]`
    progress() / download() / safe_extract()     install progress, verified downloads, contained archive extraction
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
import shutil
import subprocess
import sys
import tempfile
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
    "atomic_write": "0.1.10",
    "setting_file_keys": "0.1.11-0",
    "format_setting": "0.1.11-0",
    "settings_to_file": "0.1.11-0",
    "settings_from_file": "0.1.11-0",
    "read_key_values": "0.1.11-0",
    "update_key_values": "0.1.11-0",
    "stop_timeout": "0.1.11-0",
    "request_console": "0.1.11-0",
    "request_terminate": "0.1.11-0",
    "graceful_stop": "0.1.11-0",
    "load_diagnose_rules": "0.1.11-0",
    "diagnose_lines": "0.1.11-0",
    "tail_lines": "0.1.11-0",
    "progress": "0.1.11-0",
    "download": "0.1.11-0",
    "ChecksumMismatch": "0.1.11-0",
    "safe_extract": "0.1.11-0",
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


def _read_reply(reply_path, deadline, timeout_message, limit=None):
    while True:
        try:
            with reply_path.open("rb") as reply_file:
                raw = reply_file.read() if limit is None else reply_file.read(limit + 1)
            if limit is not None and len(raw) > limit:
                raise _host_error("response_too_large", "response exceeds 1 MiB")
            return json.loads(raw)
        except (FileNotFoundError, PermissionError) as error:
            if time.monotonic() >= deadline:
                raise TimeoutError(timeout_message) from error
        except (UnicodeError, json.JSONDecodeError):
            if time.monotonic() >= deadline:
                raise
        time.sleep(0.02)


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
    try:
        try:
            reply = _read_reply(reply_path, deadline, f"no reply from the daemon for {op}", 1024 * 1024)
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
    try:
        reply = _read_reply(reply_path, deadline, f"no reply from the daemon for 127.0.0.1:{port}")
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


# ─── 파일 저장 ─────────────────────────────────────────────────

# ReplaceFileW가 새 파일을 옮기지 못했다: 원래 파일은 백업 이름에, 새 내용은 임시 파일 이름에 남아 있다
_ERROR_UNABLE_TO_MOVE_REPLACEMENT_2 = 1177
_ERROR_ACCESS_DENIED = 5


def atomic_write(path, data):
    """`path`를 `data`(bytes)로 통째로 바꾼다. 같은 폴더의 고유 임시 파일에 다 쓰고 디스크에 내린 뒤 바꾼다.
    쓰다 끊겨도(강제 종료, 데몬 시간 초과) 원래 파일이나 새 파일 중 하나가 온전히 남고, 동시에 저장해도
    서로의 임시 파일을 건드리지 않는다. 원래 파일의 접근 권한을 지킨다 (비밀번호가 든 설정이 넓게 열리지 않게).
    Windows: ReplaceFileW가 원래 파일의 DACL 등을 옮긴다. 백업 이름을 주어 어느 단계에서 실패해도 원래 파일이 남는다.
    격리된 모듈은 DACL을 옮길 권한이 없어 거부되는데, 원래 파일과 같은 폴더의 임시 파일이 소유자·DACL·무결성
    표시가 똑같으면(둘 다 폴더 권한만 물려받은 흔한 경우) os.replace로 바꾼다. 다르면 그대로 거부한다.
    POSIX: 권한 비트를 옮긴 뒤 os.replace"""
    path = os.fspath(path)
    folder = os.path.dirname(os.path.abspath(path))
    os.makedirs(folder, exist_ok=True)
    fd, tmp = tempfile.mkstemp(prefix=f".{os.path.basename(path)}.", suffix=".tmp", dir=folder)
    try:
        with os.fdopen(fd, "wb") as f:
            f.write(data)
            f.flush()
            os.fsync(f.fileno())
        _replace_keeping_access(tmp, path)
    finally:
        if os.path.exists(tmp):
            os.remove(tmp)


def _replace_keeping_access(tmp, path):
    if not os.path.isfile(path):
        if os.name != "nt":
            # mkstemp은 0600으로 만든다: 새 파일은 open()으로 만든 것과 같은 권한으로
            umask = os.umask(0)
            os.umask(umask)
            os.chmod(tmp, 0o666 & ~umask)
        os.replace(tmp, path)
        return
    if os.name != "nt":
        shutil.copymode(path, tmp)
        os.replace(tmp, path)
        return
    import ctypes
    backup = f"{tmp}.bak"
    err = _replace_file(path, tmp, backup)
    if not err:
        try:
            os.remove(backup)
        except OSError:
            pass
        return
    if err == _ERROR_UNABLE_TO_MOVE_REPLACEMENT_2 and not os.path.exists(path) and os.path.isfile(backup):
        # 원래 파일(원래 ACL)을 제자리로 되돌린다. 새 값은 쓰지 못한 것으로 알린다
        os.replace(backup, path)
    elif err == _ERROR_ACCESS_DENIED and os.path.isfile(path) and not os.path.exists(backup):
        # 원래 파일과 임시 파일이 같은 폴더에서 같은 권한을 물려받았다: 바꿔도 권한이 넓어지지 않는다.
        # EFS 암호화도 ReplaceFileW만 옮기므로 같아야 한다
        if _same_access(_access_of(path), _access_of(tmp)) and _encrypted(path) == _encrypted(tmp):
            os.replace(tmp, path)
            return
    raise ctypes.WinError(err) if err > 0 else OSError(f"ReplaceFileW failed: {path}")


def _replace_file(path, tmp, backup):
    """ReplaceFileW. 반환: 0이면 성공, 아니면 Windows 오류 코드"""
    import ctypes
    replace = ctypes.WinDLL("kernel32", use_last_error=True).ReplaceFileW
    replace.argtypes = [ctypes.c_wchar_p, ctypes.c_wchar_p, ctypes.c_wchar_p, ctypes.c_uint32, ctypes.c_void_p, ctypes.c_void_p]
    replace.restype = ctypes.c_int
    if replace(path, tmp, backup, 0, None, None):
        return 0
    return ctypes.get_last_error() or -1


def _encrypted(path):
    return bool(getattr(os.stat(path), "st_file_attributes", 0) & 0x4000)  # FILE_ATTRIBUTE_ENCRYPTED


def _same_access(a, b):
    """두 SDDL이 같은 권한인지. 무결성 표시 SACL의 자동 상속 제어 플래그(AI·AR)는 보지 않는다:
    데몬이 기존 하위 파일에 맞춘 상속 표시에는 없고, OS가 새 파일에 붙인 표시에는 있다 (ACE는 같다)"""
    if a is None or b is None:
        return False
    plain = lambda sddl: re.sub(r"S:(?:AI|AR)*", "S:", sddl)
    return plain(a) == plain(b)


def _access_of(path):
    """파일의 소유자·DACL·무결성 표시(SDDL 문자열). 읽지 못하면 None"""
    import ctypes
    from ctypes import wintypes
    advapi = ctypes.WinDLL("advapi32", use_last_error=True)
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    advapi.GetNamedSecurityInfoW.argtypes = [
        wintypes.LPCWSTR, ctypes.c_int, wintypes.DWORD, ctypes.c_void_p, ctypes.c_void_p,
        ctypes.c_void_p, ctypes.c_void_p, ctypes.POINTER(ctypes.c_void_p)]
    advapi.GetNamedSecurityInfoW.restype = wintypes.DWORD
    advapi.ConvertSecurityDescriptorToStringSecurityDescriptorW.argtypes = [
        ctypes.c_void_p, wintypes.DWORD, wintypes.DWORD, ctypes.POINTER(wintypes.LPWSTR), ctypes.c_void_p]
    kernel.LocalFree.argtypes = [ctypes.c_void_p]
    # SE_FILE_OBJECT, OWNER | DACL | LABEL_SECURITY_INFORMATION, SDDL_REVISION_1
    se_file_object, info, sddl_revision_1 = 1, 0x1 | 0x4 | 0x10, 1
    sd = ctypes.c_void_p()
    if advapi.GetNamedSecurityInfoW(path, se_file_object, info, None, None, None, None, ctypes.byref(sd)) != 0:
        return None
    try:
        text = wintypes.LPWSTR()
        if not advapi.ConvertSecurityDescriptorToStringSecurityDescriptorW(sd, sddl_revision_1, info, ctypes.byref(text), None):
            return None
        try:
            return text.value
        finally:
            kernel.LocalFree(text)
    finally:
        kernel.LocalFree(sd)


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


def _module_toml(module_dir):
    """모듈의 module.toml (dict). 없거나 읽지 못하면 {}"""
    try:
        import tomllib
        return tomllib.loads((Path(module_dir) / "module.toml").read_bytes().decode("utf-8-sig"))
    except (ImportError, OSError, UnicodeError, ValueError):
        return {}


def _setting_fields(module_dir):
    fields = _module_toml(module_dir).get("settings", {}).get("fields", [])
    return [f for f in fields if isinstance(f, dict) and isinstance(f.get("name"), str)]


def setting_file_keys(module_dir):
    """module.toml `[[settings.fields]] file_key`: {설정 이름: 게임 설정 파일의 키}.
    이름과 파일 키를 한 곳(module.toml)에만 적는다. file_key가 없는 설정은 파일에 쓰지 않는다"""
    return {f["name"]: f["file_key"] for f in _setting_fields(module_dir) if isinstance(f.get("file_key"), str) and f["file_key"]}


def format_setting(value, *, true="true", false="false", name=""):
    """설정 값 → 게임 설정 파일의 한 줄 값. 줄바꿈이 든 값은 다른 키를 만들어 내므로 거절한다(ValueError)"""
    if isinstance(value, bool):
        return true if value else false
    if isinstance(value, float) and value.is_integer():
        value = int(value)
    text = str(value)
    if "\n" in text or "\r" in text:
        raise ValueError(f"line break in setting value: {name or text[:20]!r}")
    return text


def settings_to_file(module_dir, settings, *, true="true", false="false"):
    """설정 화면 값 {이름: 값} → {파일 키: 문자열}. file_key가 없는 설정과 None 값은 뺀다"""
    keys = setting_file_keys(module_dir)
    return {keys[name]: format_setting(value, true=true, false=false, name=name)
            for name, value in (settings or {}).items() if name in keys and value is not None}


def settings_from_file(module_dir, values, *, true="true", false="false", case_insensitive=False):
    """게임 설정 파일 값 {파일 키: 문자열} → {설정 이름: 화면 형식}. 타입은 module.toml을 따른다.
    boolean은 true/false 표기가 맞을 때만 넣는다(모르는 표기는 짐작하지 않고 뺀다)"""
    fields = {f["name"]: f.get("type", "text") for f in _setting_fields(module_dir)}
    fold = (lambda k: k.lower()) if case_insensitive else (lambda k: k)
    by_key = {fold(key): name for name, key in setting_file_keys(module_dir).items()}
    out = {}
    for key, raw in (values or {}).items():
        name = by_key.get(fold(key))
        if name is None:
            continue
        if fields.get(name) == "boolean":
            text = str(raw).strip().lower()
            if text in (true.lower(), false.lower()):
                out[name] = text == true.lower()
            continue
        out[name] = coerce_setting(raw, fields.get(name))
    return out


# ─── `키=값` 설정 파일 ─────────────────────────────────────────

# UTF-8이 아니면 ANSI 편집기·게임이 쓴 cp949 등. cp949가 로캘보다 먼저다:
# 영문 Windows의 cp1252는 거의 모든 바이트를 받아 한글을 깨진 글자로 읽는다
_KV_ENCODINGS = ("utf-8", "cp949", None, "latin-1")


def _kv_text(path, encodings):
    """(줄 목록, 줄바꿈, 인코딩, BOM 여부). 파일이 없으면 빈 UTF-8 파일로 본다"""
    try:
        raw = Path(path).read_bytes()
    except FileNotFoundError:
        return [], "\n", "utf-8", False
    bom = raw.startswith(b"\xef\xbb\xbf")
    if bom:
        raw = raw[3:]
    error = None
    for encoding in encodings:
        encoding = encoding or __import__("locale").getpreferredencoding(False)
        try:
            text = raw.decode(encoding)
            break
        except (UnicodeDecodeError, LookupError) as e:
            error = e
    else:
        # 어느 인코딩으로도 못 읽었다: 빈 설정으로 보고 덮어쓰지 않는다
        raise error if isinstance(error, UnicodeDecodeError) else UnicodeDecodeError("utf-8", raw, 0, 1, "no encoding")
    newline = "\r\n" if "\r\n" in text else "\n"
    return text.splitlines(), newline, encoding, bom


def _kv_parse(line, comments):
    """`키=값` → (키, 값). 주석·빈 줄·`=` 없는 줄은 None. 값은 첫 `=` 뒤 그대로(앞뒤 공백도 값이다)"""
    stripped = line.strip()
    if not stripped or stripped.startswith(comments) or "=" not in stripped:
        return None
    key, _, value = line.partition("=")
    return key.strip(), value


def read_key_values(path, *, encodings=_KV_ENCODINGS, case_insensitive=False, first_wins=False,
                    strip_values=False, comments=("#",)):
    """`키=값` 줄 파일(server.properties, serverconfig.txt, 좀보이드 ini 등) → {키: 값}.
    없으면 {}. encodings 어느 것으로도 못 읽으면 UnicodeDecodeError(update_key_values도 쓰지 않는다). case_insensitive면 키를 소문자로 돌려준다. 같은 키가 여러 줄이면 마지막(first_wins면 첫) 줄"""
    lines, _, _, _ = _kv_text(path, encodings)
    out = {}
    for line in lines:
        parsed = _kv_parse(line, tuple(comments))
        if not parsed:
            continue
        key, value = parsed
        if case_insensitive:
            key = key.lower()
        if first_wins and key in out:
            continue
        out[key] = value.strip() if strip_values else value
    return out


def update_key_values(path, updates, *, encodings=_KV_ENCODINGS, case_insensitive=False, delete_empty=False,
                      comments=("#",), write_encoding=None):
    """바뀐 키만 고친다. 주석·다른 줄·순서·줄바꿈 표기·인코딩(BOM 포함)은 그대로 둔다.
    값이 None이면(delete_empty면 ""도) 그 키 줄을 지운다. 같은 키의 둘째 줄부터는 지운다(어느 줄을 읽을지
    애매하지 않게). 파일에 없는 키는 끝에 붙인다. 원래 인코딩으로 못 쓰는 글자가 들어오면 UTF-8로 쓴다.
    write_encoding을 주면(게임이 한 인코딩으로만 읽을 때) 원래 인코딩과 상관없이 그것으로 쓴다(BOM 없이)"""
    fold = (lambda k: k.lower()) if case_insensitive else (lambda k: k)
    pending = {}
    for key, value in updates.items():
        if value is not None and not isinstance(value, str):
            value = format_setting(value, name=key)
        elif isinstance(value, str) and ("\n" in value or "\r" in value):
            raise ValueError(f"line break in value of {key!r}")
        pending[fold(key)] = (key, None if value is None or (delete_empty and value == "") else value)
    lines, newline, encoding, bom = _kv_text(path, encodings)
    out, seen = [], set()
    for line in lines:
        parsed = _kv_parse(line, tuple(comments))
        folded = fold(parsed[0]) if parsed else None
        if folded in pending:
            if folded not in seen and pending[folded][1] is not None:
                out.append(f"{parsed[0]}={pending[folded][1]}")
            seen.add(folded)
            continue
        out.append(line)
    for folded, (key, value) in pending.items():
        if folded not in seen and value is not None:
            out.append(f"{key}={value}")
    text = (newline.join(out) + newline) if out else ""
    if write_encoding:
        encoding, bom = write_encoding, False
    try:
        data = text.encode(encoding)
    except UnicodeEncodeError:
        data, bom = text.encode("utf-8"), False
    atomic_write(path, (b"\xef\xbb\xbf" if bom else b"") + data)


# ─── 정지 ─────────────────────────────────────────────────────

# 데몬이 호출 제한(stop_call_limit)을 알려 주지 않으면(0.1.8) 120초 제한 안에서 끝낸다
_STOP_TIMEOUT_MAX = 90
_STOP_CALL_MARGIN = 30


def stop_timeout(config, module_dir=None, default=30):
    """정지 대기(초): 인스턴스 stop_timeout → module.toml [config] stop_timeout → default.
    상한은 데몬의 호출 제한(stop_call_limit)보다 30초 앞이다"""
    value = config.get("stop_timeout")
    if value in (None, "") and module_dir:
        value = _module_toml(module_dir).get("config", {}).get("stop_timeout")
    try:
        value = int(value)
    except (TypeError, ValueError):
        value = default
    cap = _STOP_TIMEOUT_MAX
    try:
        cap = int(config["stop_call_limit"]) - _STOP_CALL_MARGIN
    except (KeyError, TypeError, ValueError):
        pass
    return max(1, min(value, cap))


def request_console(config, command, *, timeout=10):
    """데몬을 거쳐 서버 콘솔(stdin)에 정지 명령을 보낸다. 콘솔이 없으면(409, 데몬 없음) False"""
    instance_id = config.get("instance_id")
    if not instance_id:
        return False
    try:
        daemon_stdin_command(instance_id, command, timeout=timeout)
        return True
    except PathOutside:
        raise
    except Exception as e:
        print(f"[saba] console stop unavailable: {e}", file=sys.stderr)
        return False


def request_terminate(pid):
    """POSIX: SIGTERM으로 저장 후 종료를 요청한다. Windows 콘솔 앱에는 보낼 정상 종료 신호가 없어 False"""
    if os.name == "nt":
        return False
    import signal
    try:
        os.kill(int(pid), signal.SIGTERM)
        return True
    except (OSError, ValueError) as e:
        print(f"[saba] SIGTERM failed: {e}", file=sys.stderr)
        return False


def graceful_stop(config, request, i18n, *, module_dir=None):
    """모듈 `stop()`의 공용 절차. request()는 저장 후 종료를 요청하고 요청이 들어갔으면 True를 돌려준다.

    force: 그 PID(와 자식)만 강제 종료. 아니면 request() → 요청할 길이 없으면 끄지 않고
    `graceful_stop_unavailable`(데몬이 사용자에게 묻는다) → managed면 바로 반환(기다림·강제 종료는 데몬) →
    stop_timeout만큼 기다린 뒤에만 강제 종료. 문구 키는 공용 문구(common_locales.json)에 있다"""
    pid = config.get("pid")
    if not pid:
        return {"success": True, "message": i18n.t("messages.no_process_running")}
    if not config.get("force"):
        if not request():
            return {"success": False, "error_code": "graceful_stop_unavailable",
                    "message": i18n.t("errors.graceful_stop_unavailable")}
        if config.get("managed"):
            return {"success": True, "message": i18n.t("messages.shutdown_signal_sent", pid=pid)}
        if wait_for_exit(pid, timeout=stop_timeout(config, module_dir)):
            return {"success": True, "message": i18n.t("messages.graceful_stop", pid=pid)}
        print(i18n.t("messages.graceful_stop_timeout"), file=sys.stderr)
    ok, error = force_kill(pid)
    if ok:
        return {"success": True, "message": i18n.t("messages.force_killed", pid=pid)}
    return {"success": False, "message": i18n.t("errors.failed_to_kill", error=error)}


# ─── 로그 진단 ─────────────────────────────────────────────────

_SEVERITIES = ("critical", "error", "warning", "info")


def load_diagnose_rules(module_dir):
    """module.toml `[[diagnose]]` (code, severity, patterns) → 컴파일한 규칙 목록. 정규식은 대소문자를 가리지 않는다.
    틀린 정규식이 든 규칙은 그 규칙만 빼고 stderr에 알린다"""
    rules = []
    for entry in _module_toml(module_dir).get("diagnose", []):
        if not isinstance(entry, dict) or not isinstance(entry.get("code"), str):
            continue
        try:
            patterns = [re.compile(p, re.IGNORECASE) for p in entry.get("patterns", [])]
        except (re.error, TypeError) as e:
            print(f"[saba] diagnose rule {entry['code']} skipped: {e}", file=sys.stderr)
            continue
        severity = entry.get("severity") if entry.get("severity") in _SEVERITIES else "error"
        rules.append({"code": entry["code"], "severity": severity, "patterns": patterns})
    return rules


def diagnose_lines(lines, rules, i18n, *, message="errors.detect.{code}", solution="errors.solutions.{code}"):
    """콘솔 줄에서 규칙마다 처음 맞는 줄 하나를 문제로 돌려준다. 문구 키의 {code}는 소문자 코드다"""
    if isinstance(lines, str):
        lines = lines.splitlines()
    issues, seen = [], set()
    for line in lines:
        for rule in rules:
            if rule["code"] in seen or not any(p.search(line) for p in rule["patterns"]):
                continue
            seen.add(rule["code"])
            code = rule["code"].lower()
            issues.append({
                "code": rule["code"],
                "severity": rule["severity"],
                "matched_line": line.strip()[:200],
                "message": i18n.t(message.format(code=code)),
                "solution": i18n.t(solution.format(code=code)),
            })
    return issues


def tail_lines(path, limit=500, max_bytes=1024 * 1024):
    """파일의 마지막 limit줄 (끝 max_bytes만 읽는다). 없거나 못 읽으면 []"""
    try:
        with open(path, "rb") as f:
            f.seek(0, os.SEEK_END)
            f.seek(max(0, f.tell() - max_bytes))
            data = f.read()
    except OSError:
        return []
    return data.decode("utf-8", errors="replace").splitlines()[-limit:]


# ─── 진행률·내려받기·압축 풀기 ─────────────────────────────────

def progress(percent, message):
    """데몬이 읽는 진행률 줄 (stderr `PROGRESS:{json}`, 설치 진행률 표시)"""
    print("PROGRESS:" + json.dumps({"percent": percent, "message": message}, ensure_ascii=True),
          file=sys.stderr, flush=True)


class ChecksumMismatch(ValueError):
    """받은 파일의 해시가 공식 값과 다르다"""

    def __init__(self, algorithm, expected, actual):
        super().__init__(f"{algorithm} mismatch: expected {expected}, got {actual}")
        self.algorithm, self.expected, self.actual = algorithm, expected, actual


def download(url, dest, *, on_progress=None, timeout=60, checksums=None, user_agent="saba-chan"):
    """url을 dest에 받는다. on_progress(받은 바이트, 전체 바이트 또는 0)를 덩어리마다 부른다.
    checksums={"sha256": "...", "sha1": "...", "md5": "..."}가 있으면 받은 뒤 비교하고 다르면
    ChecksumMismatch. Content-Length보다 적게 받고 끝나면 IOError. dest 옆 `.part`에 받고 성공해야 dest로 바꾸므로
    실패하면 dest에 있던 파일은 그대로다. 반환: {알고리즘: hex} (sha1·sha256은 늘 계산한다).
    urlopen의 timeout은 읽기 한 번의 한도라서, 느리게 이어지는 받기도 전체로 timeout*10 안에 끊는다"""
    import hashlib
    wanted = {k.lower(): v.lower() for k, v in (checksums or {}).items() if v}
    hashes = {name: hashlib.new(name) for name in {"sha1", "sha256", *wanted}}
    deadline = time.monotonic() + timeout * 10
    req = urllib.request.Request(url, headers={"User-Agent": user_agent})
    part = os.fspath(dest) + ".part"
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp, open(part, "wb") as out:
            total = int(resp.headers.get("Content-Length") or 0)
            done = 0
            while True:
                if time.monotonic() > deadline:
                    raise TimeoutError(f"download took longer than {timeout * 10}s")
                chunk = resp.read(1 << 20)
                if not chunk:
                    break
                out.write(chunk)
                for h in hashes.values():
                    h.update(chunk)
                done += len(chunk)
                if on_progress:
                    on_progress(done, total)
            # 연결이 일찍 정상 종료되면 read()는 오류 없이 끝난다
            if total and done != total:
                raise IOError(f"incomplete download: {done}/{total} bytes")
        digests = {name: h.hexdigest() for name, h in hashes.items()}
        for name, expected in wanted.items():
            if digests[name] != expected:
                raise ChecksumMismatch(name.upper(), expected, digests[name])
        os.replace(part, dest)
    except BaseException:
        try:
            os.remove(part)
        except OSError:
            pass
        raise
    return digests


def _link_inside(dest, link_rel, target):
    """압축 속 심볼릭 링크 target이 dest 안을 가리키면 True. 절대 경로·드라이브는 거부한다."""
    if not target or target.startswith(("/", "\\")) or ":" in target:
        return False
    parts = [p for p in link_rel.split("/")[:-1] if p]
    for p in target.replace("\\", "/").split("/"):
        if p in ("", "."):
            continue
        if p == "..":
            if not parts:
                return False
            parts.pop()
        else:
            parts.append(p)
    return True


def safe_extract(archive, dest, *, prefix=None, strip_top=False, keep_existing=()):
    """zip·tar(.gz) 압축을 dest에 푼다. 모든 항목을 먼저 검사하고 나서 쓴다: dest 밖으로 나가는 항목이
    하나라도 있으면 아무것도 풀지 않고 PathOutside. 장치 같은 특수 항목은 풀지 않는다. 심볼릭 링크는 상대 경로로
    dest 안을 가리킬 때만 POSIX에서 만들고(밖을 가리키면 건너뛴다), tar 하드 링크는 푼 대상 파일을 복사한다.
    prefix: 그 폴더 안의 항목만 풀고 그 앞부분을 뗀다. strip_top: 맨 위 폴더 하나를 뗀다(그 폴더가 하나뿐일 때).
    keep_existing: 이미 있으면 덮지 않을 상대 경로('/' 구분). zip은 CRC를 먼저 검사한다. 반환: 쓴 파일 경로 목록"""
    import stat
    import tarfile
    import zipfile
    archive = os.fspath(archive)
    if zipfile.is_zipfile(archive):
        handle = zipfile.ZipFile(archive)
        bad = handle.testzip()
        if bad is not None:
            handle.close()
            raise zipfile.BadZipFile(f"bad CRC: {bad}")
        entries = []
        for i in handle.infolist():
            if i.is_dir():
                continue
            if stat.S_ISLNK(i.external_attr >> 16):
                entries.append((i.filename, "symlink", handle.read(i).decode("utf-8", "replace"), i))
            else:
                entries.append((i.filename, "file", None, i))
        opener = handle.open
    else:
        handle = tarfile.open(archive, "r:*")
        entries = []
        for m in handle.getmembers():
            if m.isfile():
                entries.append((m.name, "file", None, m))
            elif m.issym():
                entries.append((m.name, "symlink", m.linkname, m))
            elif m.islnk():
                entries.append((m.name, "hardlink", m.linkname.replace("\\", "/"), m))
        opener = handle.extractfile
    try:
        names = [e[0].replace("\\", "/") for e in entries]
        cut = ""
        if prefix:
            cut = prefix.strip("/") + "/"
        elif strip_top:
            tops = {n.split("/", 1)[0] for n in names if "/" in n}
            if len(tops) == 1 and all("/" in n for n in names):
                cut = tops.pop() + "/"
        planned = []
        for name, (_, kind, link, info) in zip(names, entries):
            if cut and not name.startswith(cut):
                continue
            rel = name[len(cut):]
            if not rel:
                continue
            if kind == "hardlink":
                if not link.startswith(cut) or not link[len(cut):]:
                    continue
                link = link[len(cut):]
                safe_join(dest, *[p for p in link.split("/") if p])
            planned.append((rel, kind, link, safe_join(dest, *[p for p in rel.split("/") if p]), info))
        written = []
        files = {}
        links = []
        for rel, kind, link, target, info in planned:
            if rel in keep_existing and os.path.lexists(target):
                continue
            if kind == "symlink":
                if os.name != "nt" and _link_inside(dest, rel, link):
                    links.append((link, target))
                continue
            target.parent.mkdir(parents=True, exist_ok=True)
            if os.path.islink(target):
                os.remove(target)
            if kind == "hardlink":
                source = files.get(link)
                if source is None:
                    continue
                shutil.copyfile(source, target)
                shutil.copymode(source, target)
            else:
                with opener(info) as src, open(target, "wb") as out:
                    shutil.copyfileobj(src, out)
                mode = getattr(info, "mode", None) if not isinstance(info, zipfile.ZipInfo) else (info.external_attr >> 16)
                if os.name != "nt" and mode and mode & 0o111:
                    os.chmod(target, 0o755)
                files[rel] = target
            written.append(str(target))
        # 링크는 파일을 다 쓴 뒤에 만든다(파일 쓰기가 압축 속 링크를 타지 않게). 링크끼리 이어져
        # 밖으로 나가는 경우(`a -> .`, `b -> a/..`)는 글자만으로 못 막으므로 다 만든 뒤 실제 경로로 다시 본다
        root = os.path.realpath(dest)
        made = []
        for link, target in links:
            target.parent.mkdir(parents=True, exist_ok=True)
            if os.path.lexists(target):
                if os.path.isdir(target) and not os.path.islink(target):
                    continue
                os.remove(target)
            os.symlink(link, target)
            made.append(target)
        for target in made:
            real = os.path.realpath(target)
            if real != root and not real.startswith(root + os.sep):
                os.remove(target)
            else:
                written.append(str(target))
        return written
    finally:
        handle.close()


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
