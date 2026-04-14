#!/usr/bin/env python3
"""
E2E 테스트용 mock 모듈 lifecycle.
데몬이 각 함수를 호출하면 올바른 JSON을 반환합니다.
실제 서버를 실행하지 않고, 프로토콜 계약만 검증합니다.
"""

import sys
import json
import os
import time

MODULE_DIR = os.path.dirname(os.path.abspath(__file__))


def validate(config):
    """서버 시작 전 검증 — 항상 성공"""
    issues = []
    executable = config.get("server_executable", "")
    if not executable:
        issues.append({
            "code": "EXECUTABLE_NOT_FOUND",
            "severity": "warning",
            "message": "No executable specified (mock module)",
            "solution": "This is expected in E2E tests."
        })
    return {"success": True, "issues": issues}


def get_launch_command(config):
    """실행 명령어 반환 — mock: 간단한 echo 프로세스"""
    working_dir = config.get("working_dir", MODULE_DIR)
    port = config.get("port", 27015)

    if sys.platform == "win32":
        return {
            "command": "cmd.exe",
            "args": ["/c", f"echo E2E_MOCK_SERVER_STARTED port={port} && timeout /t 86400 /nobreak > nul"],
            "cwd": working_dir,
            "env": {"E2E_TEST": "1"}
        }
    else:
        return {
            "command": "/bin/sh",
            "args": ["-c", f"echo 'E2E_MOCK_SERVER_STARTED port={port}' && sleep 86400"],
            "cwd": working_dir,
            "env": {"E2E_TEST": "1"}
        }


def stop(config):
    """서버 종료 — mock: 항상 성공"""
    pid = config.get("pid")
    force = config.get("force", False)
    return {"success": True, "message": f"Mock server stopped (pid={pid}, force={force})"}


def status(config):
    """상태 조회 — mock: 항상 running"""
    return {"status": "running", "message": "Mock server is running"}


def configure(config):
    """설정 적용 — mock: 항상 성공"""
    settings = config.get("settings", {})
    return {"success": True, "message": f"Applied {len(settings)} settings"}


def import_settings(config):
    """기존 설정 가져오기 — mock: 기본 설정 반환"""
    return {
        "settings": {
            "server_name": "E2E Test Server",
            "max_players": 10,
            "port": config.get("port", 27015)
        }
    }


def command(config):
    """커스텀 명령어 실행 — mock: echo back"""
    cmd_name = config.get("command", "unknown")
    args = config.get("args", {})
    return {"success": True, "result": f"Executed '{cmd_name}' with args: {json.dumps(args)}"}


def get_installed_version(config):
    """설치된 버전 감지 — mock: install_dir 내 version.txt 읽기"""
    install_dir = config.get("install_dir", "")
    if not install_dir:
        return {"success": False, "message": "No install_dir specified"}
    version_file = os.path.join(install_dir, "version.txt")
    if not os.path.isfile(version_file):
        return {"success": False, "message": "version.txt not found"}
    with open(version_file, "r", encoding="utf-8") as f:
        version = f.read().strip()
    return {"success": True, "version": version}


def install_server(config):
    """서버 설치 — mock: version.txt 작성 + server.jar 생성"""
    version = config.get("version", "")
    install_dir = config.get("install_dir", "")
    jar_name = config.get("jar_name", "server.jar")
    if not version or not install_dir:
        return {"success": False, "message": "Missing version or install_dir"}
    os.makedirs(install_dir, exist_ok=True)
    # Write version marker
    with open(os.path.join(install_dir, "version.txt"), "w", encoding="utf-8") as f:
        f.write(version)
    # Write mock server binary
    jar_path = os.path.join(install_dir, jar_name)
    with open(jar_path, "w", encoding="utf-8") as f:
        f.write(f"MOCK_SERVER_JAR {version}")
    return {
        "success": True,
        "jar_path": jar_path,
        "install_path": install_dir,
        "version": version,
    }


def list_versions(config):
    """사용 가능한 버전 목록 — mock"""
    return {
        "success": True,
        "versions": [
            {"id": "2.0.0", "type": "release"},
            {"id": "1.0.0", "type": "release"},
        ],
        "latest": {"release": "2.0.0"},
        "total": 2,
        "page": 1,
        "per_page": 25,
        "total_pages": 1,
    }


# ─── Entry point ─────────────────────────────────────────
if __name__ == "__main__":
    if len(sys.argv) < 2:
        print(json.dumps({"error": "No function specified"}))
        sys.exit(1)

    func_name = sys.argv[1]
    func_map = {
        "validate": validate,
        "get_launch_command": get_launch_command,
        "stop": stop,
        "status": status,
        "configure": configure,
        "import_settings": import_settings,
        "command": command,
        "get_installed_version": get_installed_version,
        "install_server": install_server,
        "list_versions": list_versions,
    }

    if func_name not in func_map:
        print(json.dumps({"error": f"Unknown function: {func_name}"}))
        sys.exit(1)

    try:
        raw = sys.stdin.read()
        config = json.loads(raw) if raw.strip() else {}
    except json.JSONDecodeError as e:
        print(json.dumps({"error": f"Invalid JSON input: {e}"}))
        sys.exit(1)

    result = func_map[func_name](config)
    print(json.dumps(result))
