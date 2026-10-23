"""
모듈·익스텐션 Python 코드 정적 검사 (M12)
==========================================

`python saba_audit.py <폴더>` → stdout에 JSON 하나:
    {"success": true, "findings": [{"permission", "file", "line", "detail"}, ...]}

폴더 안의 .py 파일을 AST로 훑어 위험한 능력을 쓰는 곳을 찾는다. 데몬은 이것을 module.toml
`[permissions]` 선언과 대조한다 (`saba-core module audit`, `GET /api/module/:name/audit`).

정적 검사는 악성 코드를 다 막지 못한다 (동적 import, 난독화). 선언과 다른 곳을 사용자에게
보여 주는 한 겹일 뿐이다. 새 권한은 아래 표에 한 줄씩 더한다.
`opaque_code`(읽을 소스 없는 바이트코드)와 `stdlib_shadow`(표준 모듈을 가리는 파일)는 선언할 수 없어 늘 보고된다.
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


def sdk_versions(source=None):
    if source is None:
        source = Path(__file__).with_name("daemon_rcon.py").read_text(encoding="utf-8-sig")
    for node in ast.parse(source).body:
        if isinstance(node, ast.Assign) and any(isinstance(target, ast.Name) and target.id == "SINCE" for target in node.targets):
            versions = ast.literal_eval(node.value)
            if isinstance(versions, dict) and all(isinstance(k, str) and isinstance(v, str) for k, v in versions.items()):
                return versions
    raise ValueError("the shared SDK has no SINCE table")


def declared_sdk_version(root):
    path = root / "module.toml"
    try:
        if path.is_file():
            import tomllib
            manifest = tomllib.loads(path.read_text(encoding="utf-8-sig")).get("module", {})
        else:
            path = root / "manifest.json"
            if not path.is_file():
                return "missing"
            manifest = json.loads(path.read_text(encoding="utf-8-sig"))
    except ImportError as error:
        raise RuntimeError("TOML parser unavailable; use saba-core module audit for this module") from error
    except (OSError, UnicodeError, ValueError):
        return f"invalid {path.name}"
    dependencies = manifest.get("dependencies", {}) if isinstance(manifest, dict) else None
    if path.name == "manifest.json" and isinstance(dependencies, list):
        dependencies = {}
    if not isinstance(dependencies, dict):
        return f"invalid {path.name} dependencies"
    declared = str(dependencies.get("saba-core", "missing"))
    legacy = manifest.get("min_app_version") if path.name == "manifest.json" else None
    if isinstance(legacy, str) and sdk_version(legacy) is not None:
        if sdk_version(declared) is None or sdk_version(legacy) > sdk_version(declared):
            return legacy
    return declared


def sdk_version(value):
    version = value.removeprefix(">=").strip().split("+", 1)[0]
    release, separator, prerelease = version.partition("-")
    parts = release.split(".")
    if len(parts) != 3 or not all(part.isdigit() for part in parts) or (separator and not prerelease):
        return None
    return tuple(int(part) for part in parts) + (not separator,)


def sdk_findings(tree, rel, versions, declared):
    minimum = sdk_version(declared)
    out, seen, aliases, stars = [], set(), {}, set()

    def add(module, name, node):
        key = f"i18n.{name}" if module == "i18n" else name
        required = versions.get(key)
        if required is None or (minimum is not None and minimum >= sdk_version(required)):
            return
        public = f"{module}.{name}"
        location = (node.lineno, public)
        if location not in seen:
            seen.add(location)
            out.append({"permission": "sdk_version", "file": rel, "line": node.lineno,
                        "detail": f"{public} requires saba-core >={required}; declared: {declared}"})

    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            for name in node.names:
                if name.name in ("daemon_rcon", "i18n"):
                    aliases[name.asname or name.name] = name.name
        elif isinstance(node, ast.ImportFrom) and node.level == 0 and node.module in ("daemon_rcon", "i18n"):
            for name in node.names:
                if name.name == "*":
                    stars.add(node.module)
                else:
                    add(node.module, name.name, node)
    for node in ast.walk(tree):
        if isinstance(node, ast.Attribute) and isinstance(node.value, ast.Name):
            module = aliases.get(node.value.id)
            if module:
                add(module, node.attr, node)
        elif isinstance(node, ast.Name) and isinstance(node.ctx, ast.Load):
            for module in stars:
                add(module, node.id, node)
    return out


def scan_file(path, rel, versions=None, declared="missing"):
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
    if versions is not None:
        out.extend(sdk_findings(tree, rel, versions, declared))
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


def shadow_findings(root):
    """폴더 맨 위의 표준 모듈 이름 파일·패키지. 실행 시 sys.path[0]이 이 폴더라 표준 모듈 대신 불린다"""
    stdlib = getattr(sys, "stdlib_module_names", frozenset())
    out = []
    for path in sorted(root.iterdir()):
        name = path.stem if path.is_file() and path.suffix.lower() in (".py", ".pyc", ".pyd", ".so") else path.name
        if path.is_dir() and not (path / "__init__.py").is_file():
            continue
        if name in stdlib:
            out.append({"permission": "stdlib_shadow", "file": path.relative_to(root).as_posix(), "line": 0,
                        "detail": f"replaces the standard module '{name}'"})
    return out


def scan(folder, *, sdk_source=None, sdk_minimum=None):
    root = Path(folder)
    versions = sdk_versions(sdk_source)
    declared = declared_sdk_version(root) if sdk_minimum is None else sdk_minimum
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

    findings = opaque_findings(root) + shadow_findings(root)
    for rel in sorted(targets):
        findings.extend(scan_file(files[rel], rel, versions, declared))
    return findings


def main():
    if len(sys.argv) != 2 and not (len(sys.argv) == 3 and sys.argv[2] == "--sdk-stdin"):
        print(json.dumps({"success": False, "message": "usage: saba_audit.py <folder>"}))
        return
    if not Path(sys.argv[1]).is_dir():
        print(json.dumps({"success": False, "message": f"not a folder: {sys.argv[1]}"}))
        return
    try:
        metadata = json.load(sys.stdin) if len(sys.argv) == 3 else {}
        if not isinstance(metadata, dict):
            raise ValueError("SDK audit metadata must be an object")
        source, minimum = metadata.get("sdk_source"), metadata.get("sdk_minimum")
        if (source is not None and not isinstance(source, str)) or (minimum is not None and not isinstance(minimum, str)):
            raise ValueError("SDK audit metadata fields must be strings")
        result = {"success": True, "findings": scan(sys.argv[1], sdk_source=source, sdk_minimum=minimum)}
    except (OSError, UnicodeError, ValueError, RuntimeError) as error:
        result = {"success": False, "message": str(error)}
    print(json.dumps(result, ensure_ascii=False))


if __name__ == "__main__":
    main()
