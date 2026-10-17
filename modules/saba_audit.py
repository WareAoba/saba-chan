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

# 테스트 코드는 실행 코드가 import할 때만 센다 (테스트는 subprocess 등을 마음껏 쓴다).
# 공용 도우미(daemon_rcon.py, i18n.py)는 모듈 폴더 밖에 있으므로 폴더 안의 같은 이름 파일은 모듈 코드다.
TEST_DIRS = ("conformance", "tests")


def is_test_file(rel):
    name = rel.rsplit("/", 1)[-1]
    return name.startswith("test_") or name == "conftest.py" or rel.split("/", 1)[0] in TEST_DIRS


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
            # from os import system → os.system()와 같다
            for a in node.names:
                full = f"{node.module}.{a.name}"
                for perm, calls in CALLS.items():
                    if full in calls:
                        add(perm, node, f"from {node.module} import {a.name}")
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


def imported_names(path):
    """파일이 import하는 이름들 (`import a.b` → a, `from .c import d` → c, d, `__import__("e")` → e)"""
    try:
        tree = ast.parse(path.read_text(encoding="utf-8-sig"))
    except (SyntaxError, UnicodeDecodeError, ValueError):
        return set()
    names = set()
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            names.update(a.name.split(".")[0] for a in node.names)
        elif isinstance(node, ast.ImportFrom):
            if node.module:
                names.add(node.module.split(".")[0])
            names.update(a.name for a in node.names)
        elif (isinstance(node, ast.Call) and dotted(node.func) == "__import__" and node.args
              and isinstance(node.args[0], ast.Constant) and isinstance(node.args[0].value, str)):
            names.add(node.args[0].value.split(".")[0])
    return names


# 소스 없이 Python이 불러오는 파일. 읽을 수 없으니 그대로 보고한다 (__pycache__ 밖에 있으면)
NATIVE_SUFFIXES = (".so", ".pyd", ".dll", ".dylib")
BYTECODE_SUFFIXES = (".pyc", ".pyo")


def unchecked_bytecode(path):
    """소스를 보지 않고 쓰이는 __pycache__ 바이트코드 (PEP 552 unchecked-hash: flags 비트0=1, 비트1=0)"""
    try:
        with open(path, "rb") as f:
            head = f.read(8)
    except OSError:
        return False
    if len(head) < 8:
        return False
    flags = int.from_bytes(head[4:8], "little")
    return flags & 0b11 == 0b01


def opaque_findings(root):
    out = []
    for path in sorted(root.rglob("*")):
        if not path.is_file():
            continue
        rel = path.relative_to(root).as_posix()
        suffix = path.suffix.lower()
        cached = "__pycache__" in path.parts
        if suffix in NATIVE_SUFFIXES or ".so." in path.name.lower():
            out.append({"permission": "native_code", "file": rel, "line": 0, "detail": "native library"})
        elif suffix in BYTECODE_SUFFIXES and (not cached or unchecked_bytecode(path)):
            out.append({"permission": "opaque_code", "file": rel, "line": 0,
                        "detail": "compiled Python without readable source"})
    return out


def scan(folder):
    root = Path(folder)
    files = {}
    for path in sorted(root.rglob("*.py")):
        if "__pycache__" not in path.parts:
            files[path.relative_to(root).as_posix()] = path

    # 실행 코드에서 import로 닿는 테스트 파일까지 검사 대상에 넣는다 (닿는 것이 없을 때까지)
    targets = {rel for rel in files if not is_test_file(rel)}
    pending = set(targets)
    while pending:
        names = set()
        for rel in pending:
            names |= imported_names(files[rel])
        pending = {rel for rel in files if rel not in targets
                   and (Path(rel).stem in names or rel.split("/", 1)[0] in names)}
        targets |= pending

    findings = opaque_findings(root)
    for rel in sorted(targets):
        findings.extend(scan_file(files[rel], rel))
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
