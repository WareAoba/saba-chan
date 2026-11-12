"""install.sh가 릴리즈를 읽지 못할 때 말없이 끝나지 않는지, 설치 뒤 무엇을 켜는지 본다 (curl만 가짜로 바꿔 실제 스크립트를 돌린다)."""
import hashlib
import json
import os
import stat
import subprocess
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path

INSTALL_SH = Path(__file__).resolve().parents[2] / "install.sh"

FAKE_CURL = """#!/bin/sh
case "$*" in
    */v1/updates/check*) cat "$FAKE_CHECK" ;;
    */cli.zip*) cp "$FAKE_CLI_ZIP" "$4" ;;
    *) cat "$FAKE_MANIFEST" ;;
esac
"""

# 부른 인자를 적고, `install`이면 데스크톱 구성처럼 설치 폴더에 CLI와 GUI를 놓는 가짜 CLI
FAKE_CLI = """#!/bin/sh
echo "cli $*" >>"$FAKE_LOG"
[ "$1" = install ] || exit 0
mkdir -p "$3"
cp "$0" "$3/saba-chan-cli"
printf '#!/bin/sh\\necho gui >>"$FAKE_LOG"\\n' >"$3/saba-chan-gui"
chmod +x "$3/saba-chan-gui"
"""

FAKE_LDCONFIG = """#!/bin/sh
echo "libwebkit2gtk-4.1.so.0 => /usr/lib/libwebkit2gtk-4.1.so.0"
"""


def put_tool(path: Path, body: str) -> None:
    path.write_text(body)
    path.chmod(path.stat().st_mode | stat.S_IXUSR)


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

    def test_a_desktop_install_without_a_display_starts_the_daemon(self):
        """ssh로 다시 깐 데스크톱 설치: 화면이 없어 GUI가 뜨지 못하니 데몬을 켠다 (GUI만 띄우면 데몬 없이 끝난다)."""
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            (tmp / "bin").mkdir()
            put_tool(tmp / "bin" / "curl", FAKE_CURL)
            put_tool(tmp / "bin" / "ldconfig", FAKE_LDCONFIG)
            put_tool(tmp / "saba-chan-cli", FAKE_CLI)
            with zipfile.ZipFile(tmp / "cli.zip", "w") as z:
                z.write(tmp / "saba-chan-cli", "saba-chan-cli")
            platform = "linux-arm64" if os.uname().machine in ("aarch64", "arm64") else "linux-x64"
            cli = {"url": "https://example.invalid/cli.zip", "sha256": hashlib.sha256((tmp / "cli.zip").read_bytes()).hexdigest()}
            (tmp / "check.json").write_text('{"latest": {"manifestUrl": "https://example.invalid/manifest.json"}}')
            (tmp / "manifest.json").write_text(json.dumps({"manifest": {"platform": platform, "components": {"cli": cli, "gui": {"url": "https://example.invalid/gui.zip"}}}}))
            env = {
                "PATH": f"{tmp / 'bin'}{os.pathsep}{os.environ['PATH']}",
                "HOME": str(tmp),
                "LANG": "C",
                "SSH_CONNECTION": "10.0.0.2 50000 10.0.0.1 22",
                "FAKE_CHECK": str(tmp / "check.json"),
                "FAKE_MANIFEST": str(tmp / "manifest.json"),
                "FAKE_CLI_ZIP": str(tmp / "cli.zip"),
                "FAKE_LOG": str(tmp / "log"),
            }
            done = subprocess.run(["sh", str(INSTALL_SH)], env=env, capture_output=True, text=True, timeout=60, start_new_session=True)
            self.assertEqual(done.returncode, 0, done.stderr)
            self.assertIn("cli daemon start", (tmp / "log").read_text())


if __name__ == "__main__":
    unittest.main()
