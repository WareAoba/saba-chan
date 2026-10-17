"""
saba-chan 모듈 공용 런타임 (daemon_rcon)
=======================================

데몬이 PYTHONPATH로 주입하는 공용 파일이다. 설치기는 `i18n.py`와 이 파일만
복사하므로, 모든 모듈이 공유하는 런타임 헬퍼는 여기에 둔다.

제공 기능:
  - daemon_request()        토큰(X-Saba-Token)을 붙인 데몬 HTTP 호출 (모든 모듈→데몬 호출의 단일 경로)
  - daemon_rcon_command()   데몬 /rcon 경유 RCON 실행 → 응답 텍스트
  - daemon_stdin_command()  데몬 /stdin 경유 managed 프로세스 stdin 입력
  - rcon_command()          레거시 시그니처 RCON (instance_id 없으면 직접 소켓)
  - is_process_alive() / wait_for_exit() / force_kill()  안전한 프로세스 확인·종료
  - config_value()          최상위 → module_settings → settings 순서의 설정 조회
  - load_setting_types() / coerce_setting()  module.toml 필드 타입 기반 값 변환
  - run_lifecycle()         `python lifecycle.py <function>` CLI 진입점

Environment:
    DAEMON_API_URL   데몬 기본 URL (기본값: http://127.0.0.1:57474)
    SABA_TOKEN_PATH  IPC 토큰 파일 경로 (없으면 SABA_DATA_DIR/.ipc_token)
"""

import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time
import urllib.error
import urllib.request

# 데몬이 PYTHONPATH로 이 모듈을 주입하므로, 모든 모듈/익스텐션이 import해서
# 동일한 기본 URL을 공유한다. Rust(`updater::constants::DEFAULT_DAEMON_URL`)와 일치.
DEFAULT_DAEMON_URL = "http://127.0.0.1:57474"


def daemon_url():
    """데몬 API 기본 URL. `DAEMON_API_URL` 환경 변수가 우선한다."""
    return os.environ.get("DAEMON_API_URL") or DEFAULT_DAEMON_URL


def _ipc_token():
    token_path = os.environ.get("SABA_TOKEN_PATH")
    if not token_path:
        data_dir = os.environ.get("SABA_DATA_DIR")
        if not data_dir:
            if os.name == "nt" and "APPDATA" in os.environ:
                data_dir = Path(os.environ["APPDATA"]) / "saba-chan"
            elif os.name != "nt" and "HOME" in os.environ:
                data_dir = Path(os.environ["HOME"]) / ".config" / "saba-chan"
            else:
                data_dir = Path("saba-chan")
        token_path = Path(data_dir) / ".ipc_token"
    try:
        return Path(token_path).read_text(encoding="utf-8").strip()
    except (OSError, UnicodeError):
        return ""


# ─── 데몬 HTTP ────────────────────────────────────────────────

def daemon_request(path, payload=None, *, method="POST", timeout=10):
    """데몬 HTTP API를 호출하고 JSON 응답(dict)을 반환한다.

    모든 데몬 라우트는 `X-Saba-Token` 헤더를 요구하므로 모듈은 반드시 이
    함수를 거쳐 데몬을 호출한다. 응답의 `error` 필드나 HTTP 오류는
    ConnectionError로 올린다.
    """
    url = f"{daemon_url()}{path}"
    headers = {"Content-Type": "application/json"}
    token = _ipc_token()
    if token:
        headers["X-Saba-Token"] = token
    data = json.dumps(payload).encode("utf-8") if payload is not None else None
    req = urllib.request.Request(url, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            body = resp.read().decode("utf-8")
    except urllib.error.HTTPError as e:
        detail = e.read().decode("utf-8", errors="replace")
        e.close()
        raise ConnectionError(f"Daemon request failed (HTTP {e.code}): {detail}") from e
    except urllib.error.URLError as e:
        raise ConnectionError(f"Daemon unreachable at {url}: {e}") from e

    try:
        result = json.loads(body) if body else {}
    except json.JSONDecodeError as e:
        raise ConnectionError(f"Invalid daemon response from {url}: {e}") from e
    if isinstance(result, dict) and result.get("error"):
        raise ConnectionError(f"Daemon error: {result['error']}")
    return result


def daemon_rcon_command(instance_id, command, *, rcon_port=None, rcon_password=None, timeout=10):
    """데몬 `/api/instance/:id/rcon`으로 RCON 명령을 실행하고 응답 텍스트를 반환한다.

    데몬은 게임 설정 파일(get_credentials)에서 포트·비밀번호를 읽는다.
    rcon_port/rcon_password는 하위 호환용 힌트로만 전달된다.
    """
    payload = {"command": command}
    if rcon_port is not None:
        payload["rcon_port"] = int(rcon_port)
    if rcon_password is not None:
        payload["rcon_password"] = rcon_password

    result = daemon_request(f"/api/instance/{instance_id}/rcon", payload, timeout=timeout)
    data = result.get("data")
    if isinstance(data, dict):
        return str(data.get("response", ""))
    if data is not None:
        return str(data)
    return str(result.get("message", ""))


def daemon_stdin_command(instance_id, command, *, timeout=10):
    """데몬 `/api/instance/:id/stdin`으로 managed 프로세스 stdin에 명령을 보낸다."""
    return daemon_request(f"/api/instance/{instance_id}/stdin", {"command": command}, timeout=timeout)


def rcon_command(host, port, password, command, timeout=5, *, instance_id=None):
    """레거시 시그니처 RCON 실행. 응답 텍스트(str)를 반환한다.

    instance_id가 있으면 데몬 경유, 없으면 직접 소켓 RCON으로 폴백한다.
    실패 시 ConnectionError를 올린다.
    """
    if instance_id:
        return daemon_rcon_command(
            instance_id, command,
            rcon_port=port, rcon_password=password, timeout=timeout,
        )

    import random
    import socket
    import struct

    auth_type, exec_type = 3, 2

    def make_packet(req_id, pkt_type, body):
        packet = struct.pack("<ii", req_id, pkt_type) + body + b"\x00\x00"
        return struct.pack("<i", len(packet)) + packet

    def read_exact(sock, size):
        data = b""
        while len(data) < size:
            chunk = sock.recv(size - len(data))
            if not chunk:
                return None
            data += chunk
        return data

    def read_packet(sock):
        raw_size = read_exact(sock, 4)
        if raw_size is None:
            return None
        data = read_exact(sock, struct.unpack("<i", raw_size)[0])
        if data is None:
            return None
        req_id, pkt_type = struct.unpack("<ii", data[:8])
        return req_id, pkt_type, data[8:-2] if len(data) > 10 else b""

    try:
        with socket.create_connection((host, int(port)), timeout=timeout) as sock:
            sock.sendall(make_packet(random.randint(1, 2**31 - 1), auth_type, password.encode("utf-8")))
            pkt = read_packet(sock)
            if pkt is None or pkt[0] == -1:
                raise ConnectionError("RCON authentication failed")
            sock.sendall(make_packet(random.randint(1, 2**31 - 1), exec_type, command.encode("utf-8")))
            pkt = read_packet(sock)
            if pkt is None:
                raise ConnectionError("RCON command returned no response")
            return pkt[2].decode("utf-8", errors="replace")
    except OSError as e:
        raise ConnectionError(f"RCON connection failed: {e}") from e


# ─── 프로세스 ─────────────────────────────────────────────────

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
            os.kill(pid, signal.SIGKILL)
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

    number → int/float, boolean → bool. 그 외(text/password/select/미선언)는
    문자열을 유지한다 — 큰 숫자 시드나 숫자 비밀번호가 손상되지 않게 한다.
    """
    text = str(value).strip()
    if field_type == "boolean":
        lowered = text.lower()
        if lowered in ("true", "false"):
            return lowered == "true"
        return text
    if field_type == "number":
        if _INT_RE.match(text):
            return int(text)
        try:
            number = float(text)
        except ValueError:
            return text
        return int(number) if number.is_integer() and "." not in text and "e" not in text.lower() else number
    return text


# ─── CLI 진입점 ───────────────────────────────────────────────

def run_lifecycle(functions):
    """`python lifecycle.py <function>` 진입점.

    stdin의 JSON config를 함수에 넘기고 결과 JSON 한 개를 stdout에 출력한다.
    알 수 없는 함수나 예외도 JSON으로 응답하고 종료 코드 0을 유지한다
    (데몬은 0이 아닌 종료 코드를 HTTP 500으로 바꾼다).
    """
    def emit(result):
        sys.stdout.write(json.dumps(result, ensure_ascii=False) + "\n")
        sys.stdout.flush()

    if len(sys.argv) < 2:
        emit({"success": False, "message": "Usage: lifecycle.py <function>"})
        return

    name = sys.argv[1]
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
    except Exception as e:  # 모듈 함수 오류도 JSON으로 돌려준다
        import traceback
        print(traceback.format_exc(), file=sys.stderr)
        result = {"success": False, "message": f"Error in {name}: {e}"}
    emit(result)
