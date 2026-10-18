"""시험이 띄운 프로세스 정리.

이름으로 죽이지 않는다(실제 설치본의 데몬·게임 서버가 같이 죽는다). 명령줄에 이번 실행의 고유 루트
경로가 들어간 프로세스만 찾아서 끝낸다. 데몬·러너·게임 서버는 모두 그 루트 아래 경로를 인자로 받는다.
"""

import json
import os
import signal
import subprocess
from pathlib import Path

from .paths import IS_WINDOWS


def processes_mentioning(marker: str) -> list[tuple[int, str]]:
    """명령줄에 marker가 들어간 (pid, 명령줄). 자기 자신은 뺀다."""
    me = os.getpid()
    found = []
    if IS_WINDOWS:
        script = (
            "Get-CimInstance Win32_Process | Where-Object { $_.CommandLine -ne $null } | "
            "Select-Object ProcessId, CommandLine | ConvertTo-Json -Compress"
        )
        out = subprocess.run(["powershell", "-NoProfile", "-Command", script], capture_output=True, text=True, encoding="utf-8", errors="replace").stdout
        rows = json.loads(out or "[]")
        if isinstance(rows, dict):
            rows = [rows]
        needle = marker.lower()
        for row in rows:
            cmd = row.get("CommandLine") or ""
            if needle in cmd.lower() and row["ProcessId"] != me and "Get-CimInstance" not in cmd:
                found.append((int(row["ProcessId"]), cmd))
        return found
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit() or int(entry.name) == me:
            continue
        try:
            cmd = (entry / "cmdline").read_bytes().replace(b"\0", b" ").decode(errors="replace")
        except OSError:
            continue
        if marker in cmd:
            found.append((int(entry.name), cmd))
    return found


def pid_alive(pid: int) -> bool:
    if IS_WINDOWS:
        out = subprocess.run(["tasklist", "/FI", f"PID eq {pid}", "/NH"], capture_output=True, text=True).stdout
        return str(pid) in out
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    try:
        stat = Path(f"/proc/{pid}/stat").read_text()
        return stat.rsplit(")", 1)[1].split()[0] != "Z"
    except OSError:
        return False


def kill_pid(pid: int) -> None:
    if IS_WINDOWS:
        subprocess.run(["taskkill", "/F", "/T", "/PID", str(pid)], capture_output=True)
        return
    try:
        os.kill(pid, signal.SIGKILL)
    except ProcessLookupError:
        pass


def kill_mentioning(marker: str) -> list[tuple[int, str]]:
    """남은 프로세스를 끝내고 무엇을 끝냈는지 돌려준다(누수 보고용)."""
    leftover = processes_mentioning(marker)
    for pid, _ in leftover:
        kill_pid(pid)
    return leftover
