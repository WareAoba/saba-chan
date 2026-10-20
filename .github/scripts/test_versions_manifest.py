"""versions.py manifest 시험: python3 -m unittest discover -s .github/scripts -p 'test_*.py'"""

import importlib.util
import json
import struct
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path

_path = Path(__file__).resolve().parents[2] / "scripts" / "versions.py"
_spec = importlib.util.spec_from_file_location("versions", _path)
versions = importlib.util.module_from_spec(_spec)
sys.modules["versions"] = versions
_spec.loader.exec_module(versions)


def fake_exe(version=None):
    """파일 버전 리소스(VS_FIXEDFILEINFO)만 흉내 낸 exe 바이트. version이 없으면 리소스가 없는 exe"""
    body = b"MZ" + b"\0" * 64
    if version:
        major, minor, patch = (int(p) for p in version.split("."))
        body += "VS_VERSION_INFO".encode("utf-16-le") + b"\0\0"
        body += struct.pack("<IIIIII", 0xFEEF04BD, 0x00010000, major << 16 | minor, patch << 16, major << 16 | minor, patch << 16)
    return body + b"\0" * 32


class ManifestExeVersion(unittest.TestCase):
    """U-2: 서명할 manifest의 버전과 올린 exe에 박힌 파일 버전이 다르면 서명 전에 멈춘다"""

    def manifest(self, files):
        data = {
            "release": {"version": "0.1.9", "channel": "stable"},
            "components": {
                "saba-core": {"version": "0.1.9", "asset": "saba-core-{platform}.zip"},
                "installer": {"version": "0.1.9", "asset": "saba-chan-installer.exe"},
            },
        }
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            with zipfile.ZipFile(d / "saba-core-windows-x64.zip", "w") as zf:
                for name, blob in files.items():
                    zf.writestr(name, blob)
            (d / "saba-chan-installer.exe").write_bytes(fake_exe("0.1.9"))
            out = d / "out.json"
            args = versions.argparse.Namespace(
                assets=str(d), base_url="https://r2.example", platform="windows-x64", channel=None,
                out=str(out), modules_repo=None, extensions_repo=None, addons_ref=None)
            versions.cmd_manifest(data, args)
            return json.loads(out.read_text())

    def test_an_exe_built_from_another_version_is_refused(self):
        with self.assertRaises(SystemExit):
            self.manifest({"saba-core.exe": fake_exe("0.1.8")})

    def test_matching_exes_are_signed(self):
        m = self.manifest({"saba-core.exe": fake_exe("0.1.9"), "saba-runner.exe": fake_exe()})
        self.assertEqual(m["components"]["saba-core"]["version"], "0.1.9")
        self.assertEqual(m["components"]["installer"]["version"], "0.1.9")

    def test_a_zip_whose_exes_carry_no_version_is_refused(self):
        # 버전을 확인할 수 없는 실행 파일만 있으면 대조할 수 없다
        with self.assertRaises(SystemExit):
            self.manifest({"saba-runner.exe": fake_exe()})


if __name__ == "__main__":
    unittest.main()
