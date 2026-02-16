#!/usr/bin/env python3
"""
saba-chan Module Lifecycle Template

이 파일은 새로운 게임 서버 모듈을 만들 때 참고할 수 있는 템플릿입니다.
필요한 함수만 구현하고 FUNCTIONS 딕셔너리에 등록하면 됩니다.

필수 함수: validate, get_launch_command, status
선택 함수: start, stop, command, configure, read_properties,
          accept_eula, diagnose_log, list_versions, install_server,
          reset_server

모든 함수는 dict를 반환해야 하며, 최소한 {"success": bool, "message": str}를 포함해야 합니다.
stdout으로 JSON을 출력하고, 로그는 stderr로 출력하세요.
"""

import sys
import json
import subprocess
import os
from pathlib import Path


# ─── Init ─────────────────────────────────────────────────────

MODULE_DIR = os.path.dirname(os.path.abspath(__file__))


# ╔═══════════════════════════════════════════════════════════╗
# ║                  Required Functions                       ║
# ╚═══════════════════════════════════════════════════════════╝

def validate(config):
    """
    서버 실행 전 필수 조건을 검증합니다.
    
    Args:
        config: {
            "instance_id": str,
            "working_dir": str,
            "executable_path": str,
            "port": int,
            ... (module_settings에서 추가 필드)
        }
    
    Returns:
        {
            "success": bool,           # 검증 통과 여부
            "issues": [                # 발견된 문제 목록
                {
                    "severity": "error" | "warning",
                    "message": str,
                    "fix_hint": str    # (선택) 해결 방법 힌트
                }
            ]
        }
    """
    issues = []
    
    executable = config.get("executable_path", "")
    if not executable or not os.path.exists(executable):
        issues.append({
            "severity": "error",
            "message": "Server executable not found",
            "fix_hint": "Set the correct path to your server executable"
        })
    
    working_dir = config.get("working_dir", "")
    if not working_dir or not os.path.isdir(working_dir):
        issues.append({
            "severity": "error",
            "message": "Working directory does not exist",
            "fix_hint": "Ensure the working directory path is valid"
        })
    
    return {
        "success": len([i for i in issues if i["severity"] == "error"]) == 0,
        "issues": issues
    }


def get_launch_command(config):
    """
    서버 실행 명령어를 생성합니다.
    데몬이 이 명령어로 managed process를 시작합니다.
    
    Args:
        config: validate()와 동일
    
    Returns:
        {
            "success": bool,
            "program": str,            # 실행 파일 경로
            "args": [str],             # 명령줄 인수
            "working_dir": str,        # 작업 디렉토리
            "env": {str: str}          # (선택) 추가 환경변수
        }
    """
    executable = config.get("executable_path", "")
    working_dir = config.get("working_dir", os.path.dirname(executable))
    
    return {
        "success": True,
        "program": executable,
        "args": [],                    # 게임에 맞는 인수 추가
        "working_dir": working_dir,
    }


def status(config):
    """
    서버의 현재 상태를 확인합니다.
    
    Args:
        config: validate()와 동일 + {"pid": int} (실행 중인 경우)
    
    Returns:
        {
            "success": bool,
            "status": "running" | "stopped" | "starting" | "error",
            "players_online": int,     # (선택)
            "players_max": int,        # (선택)
            "version": str,            # (선택)
            "motd": str,               # (선택)
        }
    """
    pid = config.get("pid")
    
    if pid:
        # PID가 있으면 프로세스 존재 여부 확인
        try:
            if sys.platform == "win32":
                result = subprocess.run(
                    ["tasklist", "/FI", f"PID eq {pid}", "/NH"],
                    capture_output=True, text=True
                )
                is_running = str(pid) in result.stdout
            else:
                os.kill(pid, 0)
                is_running = True
        except (OSError, subprocess.SubprocessError):
            is_running = False
        
        return {
            "success": True,
            "status": "running" if is_running else "stopped"
        }
    
    return {
        "success": True,
        "status": "stopped"
    }


# ╔═══════════════════════════════════════════════════════════╗
# ║                  Optional Functions                       ║
# ╚═══════════════════════════════════════════════════════════╝

def stop(config):
    """
    서버를 정상 종료합니다.
    
    RCON/REST/stdin 등 게임에 맞는 방법으로 종료 명령을 전송합니다.
    데몬이 일정 시간 후에도 종료되지 않으면 force kill을 수행합니다.
    
    Returns:
        {"success": bool, "message": str}
    """
    # 예: RCON으로 종료 명령 전송
    # send_rcon_command(config, "stop")
    return {"success": True, "message": "Stop command sent"}


def configure(config):
    """
    인스턴스 설정이 변경될 때 호출됩니다.
    서버 설정 파일(server.properties 등)에 값을 동기화합니다.
    
    Returns:
        {"success": bool, "updated_keys": [str]}
    """
    return {"success": True, "updated_keys": []}


def read_properties(config):
    """
    서버 설정 파일을 읽어 반환합니다.
    
    Returns:
        {"success": bool, "properties": {key: value, ...}}
    """
    return {"success": True, "properties": {}}


def diagnose_log(config):
    """
    서버 로그를 분석하여 알려진 에러 패턴을 찾아 진단합니다.
    
    Returns:
        {
            "success": bool,
            "issues": [
                {
                    "severity": "error" | "warning" | "info",
                    "message": str,
                    "solution": str
                }
            ]
        }
    """
    return {"success": True, "issues": []}


# ╔═══════════════════════════════════════════════════════════╗
# ║                  Function Registry                        ║
# ╚═══════════════════════════════════════════════════════════╝

# 이 딕셔너리에 등록된 함수만 데몬이 호출합니다.
# 불필요한 함수는 제거하거나 주석 처리하세요.
FUNCTIONS = {
    # 필수
    "validate": validate,
    "get_launch_command": get_launch_command,
    "status": status,
    # 선택 — 필요한 것만 활성화
    "stop": stop,
    "configure": configure,
    "read_properties": read_properties,
    "diagnose_log": diagnose_log,
}


# ╔═══════════════════════════════════════════════════════════╗
# ║                    Entry Point                            ║
# ╚═══════════════════════════════════════════════════════════╝

def main():
    """
    데몬에서 호출될 때의 진입점.
    stdin으로 JSON config를 받고, stdout으로 JSON 결과를 반환합니다.
    """
    if len(sys.argv) < 2:
        print(json.dumps({"success": False, "message": "Usage: lifecycle.py <function_name>"}))
        sys.exit(1)
    
    func_name = sys.argv[1]
    
    if func_name not in FUNCTIONS:
        print(json.dumps({
            "success": False,
            "message": f"Unknown function: {func_name}",
            "available": list(FUNCTIONS.keys())
        }))
        sys.exit(1)
    
    # stdin에서 config 읽기
    try:
        config_str = sys.stdin.read()
        config = json.loads(config_str) if config_str.strip() else {}
    except json.JSONDecodeError:
        config = {}
    
    # 함수 실행
    try:
        result = FUNCTIONS[func_name](config)
        print(json.dumps(result, ensure_ascii=False))
    except Exception as e:
        print(json.dumps({
            "success": False,
            "message": f"Error in {func_name}: {str(e)}"
        }))
        sys.exit(1)


if __name__ == "__main__":
    main()
