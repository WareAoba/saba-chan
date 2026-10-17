import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
PWSH = shutil.which("pwsh")
BASH = Path(r"C:\Program Files\Git\bin\bash.exe") if os.name == "nt" else shutil.which("bash")


@unittest.skipUnless(PWSH, "PowerShell is unavailable")
class PowerShellRunnerTests(unittest.TestCase):
    def run_suite(self, suite, failure=0):
        (ROOT / ".tmp").mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=ROOT / ".tmp", prefix="runner-test-") as directory:
            fixture = Path(directory)
            scripts = fixture / "scripts"
            scripts.mkdir()
            for component in ["saba-chan-gui", "saba-chan-mcp"]:
                (fixture / component).mkdir()
            runner = scripts / "run-test.ps1"
            for name in ["run-test.ps1", "run-tests.py"]:
                shutil.copyfile(ROOT / "scripts" / name, scripts / name)
            calls = fixture / "calls.txt"
            binaries = fixture / "bin"
            binaries.mkdir()
            for name, code in [("cargo", 0), ("npm", failure)]:
                executable = binaries / (f"{name}.cmd" if os.name == "nt" else name)
                if os.name == "nt":
                    executable.write_text(f'@echo off\necho {name} %*>> "%RUNNER_TEST_CALLS%"\nexit /b {code}\n')
                else:
                    executable.write_text(f'#!/bin/sh\necho "{name} $*" >> "$RUNNER_TEST_CALLS"\nexit {code}\n')
                    executable.chmod(0o755)
            env = {**os.environ, "RUNNER_TEST_CALLS": str(calls), "SABA_PYTHON": sys.executable,
                   "PATH": str(binaries) + os.pathsep + os.environ["PATH"]}
            result = subprocess.run([PWSH, "-NoProfile", "-NonInteractive", "-File", str(runner),
                                     "-Suite", suite, "-NoInstall"],
                                    env=env, text=True, capture_output=True, timeout=30)
            return result, calls.read_text(encoding="utf-8-sig").splitlines() if calls.exists() else []

    def test_rust_suite_runs_the_whole_workspace_once(self):
        result, calls = self.run_suite("rust")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, ["cargo test --workspace --no-fail-fast"])

    def test_mcp_suite_runs_tests_and_typecheck(self):
        result, calls = self.run_suite("mcp")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, ["npm test", "npm run typecheck"])

    def test_failed_suite_returns_failure(self):
        result, _ = self.run_suite("mcp", failure=7)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("FAIL", result.stdout)


@unittest.skipUnless(BASH and Path(BASH).exists(), "Bash is unavailable")
class BashRunnerTests(unittest.TestCase):
    def test_invalid_or_missing_suite_is_rejected_before_commands_run(self):
        (ROOT / ".tmp").mkdir(exist_ok=True)
        for arguments in [["--suite", "typo"], ["--suite"], ["--unknown"]]:
            with self.subTest(arguments=arguments):
                with tempfile.TemporaryDirectory(dir=ROOT / ".tmp", prefix="runner-bash-") as directory:
                    runner = Path(directory) / "run-test.sh"
                    runner.write_text((ROOT / "scripts" / "run-test.sh").read_text(), newline="\n")
                    shutil.copyfile(ROOT / "scripts" / "run-tests.py", Path(directory) / "run-tests.py")
                    result = subprocess.run([str(BASH), str(runner), *arguments],
                                            env={**os.environ, "SABA_PYTHON": sys.executable},
                                            text=True, capture_output=True, timeout=10)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("usage:", result.stderr)
                self.assertNotIn("passed", result.stdout.lower())


if __name__ == "__main__":
    unittest.main()
