"""
모듈·익스텐션 Python 코드 정적 검사 (M12)
==========================================

`python saba_audit.py <폴더>` → stdout에 JSON 하나:
    {"success": true, "findings": [{"permission", "file", "line", "detail"}, ...]}

폴더 안의 .py 파일을 AST로 훑어 위험한 능력을 쓰는 곳을 찾는다. 데몬은 이것을 module.toml
`[permissions]` 선언과 대조한다 (`saba-core module audit`, `GET /api/module/:name/audit`).

정적 검사는 악성 코드를 다 막지 못한다 (동적 import, 난독화). 선언과 다른 곳을 사용자에게
보여 주는 한 겹일 뿐이다. 새 권한은 아래 표에 한 줄씩 더한다.
"""

import ast
import json
import sys
from pathlib import Path

# 권한 → 그 권한을 쓴다고 보는 import 모듈 (앞부분 일치: "urllib"이면 urllib.request도)
IMPORTS = {
    "network": ("socket", "ssl", "urllib", "http", "requests", "httpx", "aiohttp", "ftplib",
                "smtplib", "telnetlib", "websocket", "websockets", "paramiko"),
    "spawn_process": ("subprocess", "multiprocessing", "pty", "pexpect"),
    "native_code": ("ctypes", "cffi", "_ctypes", "winreg"),
    "dynamic_code": ("marshal", "pickle", "shelve", "importlib", "runpy", "code", "codeop"),
}

# 권한 → 그 권한을 쓴다고 보는 호출 (`os.system`처럼 점으로 이은 이름, 또는 내장 함수 이름)
CALLS = {
    "spawn_process": ("os.system", "os.popen", "os.startfile", "os.spawnl", "os.spawnle", "os.spawnlp",
                      "os.spawnv", "os.spawnve", "os.spawnvp", "os.execl", "os.execle", "os.execlp",
                      "os.execv", "os.execve", "os.execvp", "os.fork", "os.kill", "os.killpg"),
    "dynamic_code": ("eval", "exec", "compile", "__import__"),
    "env_secrets": ("os.environ.items", "os.environ.copy", "os.environ.values"),
}

# 이 이름의 공용 도우미는 데몬이 함께 배포하는 것이라 모듈 코드로 세지 않는다
SHARED_HELPERS = {"daemon_rcon.py", "i18n.py", "saba_audit.py"}


def dotted(node):
    """`os.path.join` 같은 이름. 이름이 아니면 None"""
    parts = []
    while isinstance(node, ast.Attribute):
        parts.append(node.attr)
        node = node.value
    if isinstance(node, ast.Name):
        parts.append(node.id)
        return ".".join(reversed(parts))
    return None


def permission_of_import(name):
    for perm, prefixes in IMPORTS.items():
        if any(name == p or name.startswith(p + ".") for p in prefixes):
            return perm
    return None


def scan_file(path, rel):
    try:
        tree = ast.parse(path.read_text(encoding="utf-8-sig"), filename=str(path))
    except (SyntaxError, UnicodeDecodeError, ValueError) as e:
        return [{"permission": "unparsable", "file": rel, "line": getattr(e, "lineno", 0) or 0,
                 "detail": f"cannot parse: {e}"}]
    out = []

    def add(perm, node, detail):
        out.append({"permission": perm, "file": rel, "line": getattr(node, "lineno", 0), "detail": detail})

    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            for a in node.names:
                perm = permission_of_import(a.name)
                if perm:
                    add(perm, node, f"import {a.name}")
        elif isinstance(node, ast.ImportFrom) and node.module and node.level == 0:
            perm = permission_of_import(node.module)
            if perm:
                add(perm, node, f"from {node.module} import ...")
        elif isinstance(node, ast.Call):
            name = dotted(node.func)
            if not name:
                continue
            if name == "dict":
                # dict(os.environ): 환경 변수 전체를 꺼낸다
                if node.args and dotted(node.args[0]) == "os.environ":
                    add("env_secrets", node, "dict(os.environ)")
                continue
            for perm, calls in CALLS.items():
                if name in calls:
                    # __import__("json")처럼 고정 문자열이면 동적 코드가 아니다
                    if name == "__import__" and node.args and isinstance(node.args[0], ast.Constant):
                        continue
                    add(perm, node, f"{name}()")
    return out


def scan(folder):
    root = Path(folder)
    findings = []
    for path in sorted(root.rglob("*.py")):
        if path.name in SHARED_HELPERS or path.name.startswith("test_") or "__pycache__" in path.parts:
            continue
        rel = path.relative_to(root).as_posix()
        if rel.startswith(("conformance/", "tests/")):
            continue
        findings.extend(scan_file(path, rel))
    return findings


def main():
    if len(sys.argv) != 2:
        print(json.dumps({"success": False, "message": "usage: saba_audit.py <folder>"}))
        return
    if not Path(sys.argv[1]).is_dir():
        print(json.dumps({"success": False, "message": f"not a folder: {sys.argv[1]}"}))
        return
    print(json.dumps({"success": True, "findings": scan(sys.argv[1])}, ensure_ascii=False))


if __name__ == "__main__":
    main()
