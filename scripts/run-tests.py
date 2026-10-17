#!/usr/bin/env python3
import argparse
import os
from pathlib import Path
import shutil
import site
import subprocess
import sys
import tempfile
import time


ROOT = Path(__file__).resolve().parents[1]
SUITES = ["rust", "relay", "music", "gui", "mcp", "python", "modules", "extensions", "appdata-sync"]


def main():
    parser = argparse.ArgumentParser(description="Saba-chan tests")
    parser.add_argument("--suite", choices=["all", *SUITES], default="all")
    parser.add_argument("--no-install", action="store_true")
    parser.add_argument("--verbose", action="store_true")
    args = parser.parse_args()
    results = []

    def run(name, directory, command, env=None):
        executable = shutil.which(command[0])
        start = time.monotonic()
        print(f"[{name}] {' '.join(command)}", flush=True)
        try:
            if executable is None:
                raise FileNotFoundError(f"Missing command: {command[0]}")
            code = subprocess.run([executable, *command[1:]], cwd=directory, env=env, check=False).returncode
        except OSError as error:
            print(error, file=sys.stderr)
            code = 1
        results.append((name, "PASS" if code == 0 else "FAIL", round(time.monotonic() - start, 2)))
        return code == 0

    def find_directory(*parts):
        for parent in [ROOT.parent, ROOT]:
            candidate = parent.joinpath(*parts)
            if candidate.is_dir():
                return candidate
        return None

    selected = SUITES[:-1] if args.suite == "all" else [args.suite]
    npm = {"gui": ROOT / "saba-chan-gui", "mcp": ROOT / "saba-chan-mcp",
           "relay": find_directory("server-chan", "relay-server"),
           "music": find_directory("server-chan", "music-server")}
    for suite in selected:
        if suite == "rust":
            run(suite, ROOT, ["cargo", "test", "--workspace", "--no-fail-fast"])
        elif suite in npm:
            directory = npm[suite]
            if directory is None or not directory.is_dir():
                results.append((suite, "SKIP", 0))
                continue
            if not args.no_install and not (directory / "node_modules").is_dir():
                if not run(f"{suite}-dependencies", directory, ["npm", "ci"]):
                    continue
            command = ["npm", "test"]
            if suite == "gui":
                command += ["--", "--run"]
            if run(suite, directory, command) and suite == "mcp":
                run("mcp-types", directory, ["npm", "run", "typecheck"])
        elif suite == "appdata-sync":
            run(suite, ROOT, ["pwsh", "-NoProfile", "-File", str(ROOT / "scripts/check-appdata-sync.ps1"), "-Strict"])
        else:
            directory = ROOT if suite == "python" else find_directory(f"saba-chan-{suite}")
            if directory is None:
                results.append((suite, "SKIP", 0))
                continue
            scratch = ROOT / ".tmp"
            scratch.mkdir(exist_ok=True)
            with tempfile.TemporaryDirectory(dir=scratch, prefix=f"test-{suite}-") as temporary:
                env = {**os.environ, "SABA_DATA_DIR": temporary, "APPDATA": temporary,
                       "TEMP": temporary, "TMP": temporary, "PYTHONDONTWRITEBYTECODE": "1"}
                if suite == "python":
                    command = [sys.executable, "-m", "unittest", "discover", "-s", "tests", "-p", "test_*.py"]
                else:
                    # APPDATA 격리로 사라지는 사용자 site-packages(pytest 등)를 유지한다.
                    env["PYTHONPATH"] = os.pathsep.join([str(ROOT / "modules"), site.getusersitepackages()])
                    code = "import i18n, daemon_rcon, pytest, sys; raise SystemExit(pytest.main(sys.argv[1:]))"
                    command = [sys.executable, "-c", code, str(directory), "--import-mode=importlib",
                               "-o", f"cache_dir={temporary}/cache", "-q"]
                run(suite, ROOT, command, env)

    for name, status, duration in results:
        print(f"{name:24} {status:4} {duration}s")
    failed = any(status == "FAIL" for _, status, _ in results)
    skipped = any(status == "SKIP" for _, status, _ in results)
    print("Tests failed." if failed else "Available tests passed; some suites skipped." if skipped else "All selected suites passed.")
    return int(failed)


if __name__ == "__main__":
    raise SystemExit(main())
