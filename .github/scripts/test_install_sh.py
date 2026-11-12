"""install.sh가 릴리즈를 읽지 못할 때 말없이 끝나지 않는지 본다 (curl만 가짜로 바꿔 실제 스크립트를 돌린다)."""
import os
import stat
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

INSTALL_SH = Path(__file__).resolve().parents[2] / "install.sh"

FAKE_CURL = """#!/bin/sh
case "$*" in
    */v1/updates/check*) cat "$FAKE_CHECK" ;;
    *) cat "$FAKE_MANIFEST" ;;
esac
"""


@unittest.skipUnless(sys.platform.startswith("linux"), "리눅스 분기(python3로 JSON 읽기)만 본다")
class InstallShTest(unittest.TestCase):
    def run_install(self, check: str, manifest: str) -> subprocess.CompletedProcess:
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            (tmp / "bin").mkdir()
            curl = tmp / "bin" / "curl"
            curl.write_text(FAKE_CURL)
            curl.chmod(curl.stat().st_mode | stat.S_IXUSR)
            (tmp / "check.json").write_text(check)
            (tmp / "manifest.json").write_text(manifest)
            env = {
                "PATH": f"{tmp / 'bin'}{os.pathsep}{os.environ['PATH']}",
                "HOME": str(tmp),
                "LANG": "C",
                "FAKE_CHECK": str(tmp / "check.json"),
                "FAKE_MANIFEST": str(tmp / "manifest.json"),
            }
            return subprocess.run(["sh", str(INSTALL_SH)], env=env, capture_output=True, text=True, timeout=60)

    def test_a_release_without_this_platforms_cli_says_so(self):
        done = self.run_install(
            '{"latest": {"manifestUrl": "https://example.invalid/manifest.json"}}',
            '{"manifest": {"platform": "windows-x64", "components": {}}}',
        )
        self.assertEqual(done.returncode, 1)
        self.assertIn("The release has no CLI for this platform.", done.stderr)

    def test_an_unreadable_check_answer_says_so(self):
        done = self.run_install("<html>502</html>", "{}")
        self.assertEqual(done.returncode, 1)
        self.assertIn("No release for this platform yet.", done.stderr)


if __name__ == "__main__":
    unittest.main()
