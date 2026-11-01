"""versions.py 여러 플랫폼 manifest 시험 (D3c): python3 -m unittest discover -s .github/scripts -p 'test_*.py'"""

import importlib.util
import json
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

from test_versions_manifest import fake_exe  # noqa: E402


class PlatformManifest(unittest.TestCase):
    """맨 위는 windows-x64 그대로 두고(0.1.10 업데이터가 읽는다) 다른 플랫폼은 platforms.<플랫폼>에 싣는다"""

    def build(self, d: Path, linux_files=("saba-chan-installer-headless-linux-x64.zip",)):
        data = {
            "release": {"version": "0.1.9", "channel": "stable"},
            "components": {
                "saba-core": {"version": "0.1.9", "asset": "saba-core-{platform}.zip"},
                "gui": {"version": "0.1.9", "asset": "saba-chan-gui-{platform}.zip"},
                "installer": {"version": "0.1.9", "asset": "saba-chan-installer.exe", "platforms": ["windows-x64"],
                              "profile_assets": {"desktop": "saba-chan-installer-desktop-{platform}.zip",
                                                 "headless": "saba-chan-installer-headless-{platform}.zip"}},
                "locales": {"version": "0.1.9", "asset": "locales.zip", "install_dir": "locales"},
            },
        }
        with zipfile.ZipFile(d / "saba-core-windows-x64.zip", "w") as zf:
            zf.writestr("saba-core.exe", fake_exe("0.1.9"))
        with zipfile.ZipFile(d / "saba-chan-gui-windows-x64.zip", "w") as zf:
            zf.writestr("saba-chan-gui.exe", fake_exe("0.1.9"))
        with zipfile.ZipFile(d / "saba-core-linux-x64.zip", "w") as zf:
            zf.writestr("saba-core", b"\x7fELF")
        with zipfile.ZipFile(d / "locales.zip", "w") as zf:
            zf.writestr("ko/gui.json", "{}")
        (d / "saba-chan-installer.exe").write_bytes(fake_exe("0.1.9"))
        for name in linux_files:
            with zipfile.ZipFile(d / name, "w") as zf:
                zf.writestr(name.split("-linux")[0], b"\x7fELF")
        out = d / "out.json"
        args = versions.argparse.Namespace(
            assets=str(d), base_url="https://r2.example", platform="windows-x64", also_platform=["linux-x64"],
            channel=None, out=str(out), modules_repo=None, extensions_repo=None, addons_ref=None)
        versions.cmd_manifest(data, args)
        return json.loads(out.read_text())

    def test_other_platforms_get_their_own_list(self):
        with tempfile.TemporaryDirectory() as d:
            m = self.build(Path(d))
        self.assertEqual(m["platform"], "windows-x64")
        self.assertEqual(set(m["components"]), {"saba-core", "gui", "installer", "locales"})
        linux = m["platforms"]["linux-x64"]["components"]
        # 리눅스 파일이 없는 GUI와 Windows 전용 설치기는 빠지고, 플랫폼과 무관한 locales는 같은 파일이다.
        # 설치기는 구성마다 빌드한 파일이 있는 것만 `installer.<구성>`으로 싣는다
        self.assertEqual(set(linux), {"saba-core", "locales", "installer.headless"})
        self.assertEqual(linux["installer.headless"]["asset"], "saba-chan-installer-headless-linux-x64.zip")
        self.assertEqual(linux["installer.headless"]["version"], "0.1.9")
        self.assertEqual(linux["saba-core"]["asset"], "saba-core-linux-x64.zip")
        self.assertEqual(linux["locales"], m["components"]["locales"])

    def test_other_platforms_must_carry_the_installer_their_clients_read(self):
        # 설치기가 없으면 그 플랫폼 클라이언트가 설치기를 받지 못한다. 창 GUI가 있으면 창 구성 설치기도 있어야 한다
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaises(SystemExit):
                self.build(Path(d), linux_files=())
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaises(SystemExit):
                self.build(Path(d), linux_files=("saba-chan-installer-headless-linux-x64.zip", "saba-chan-gui-linux-x64.zip"))
        with tempfile.TemporaryDirectory() as d:
            m = self.build(Path(d), linux_files=("saba-chan-installer-headless-linux-x64.zip", "saba-chan-gui-linux-x64.zip",
                                                 "saba-chan-installer-desktop-linux-x64.zip"))
        self.assertIn("installer.desktop", m["platforms"]["linux-x64"]["components"])

    def test_registration_lists_each_platforms_assets(self):
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            m = self.build(d)
            signed = d / "signed.json"
            signed.write_text(json.dumps({"manifest": {**m, "channel": "beta"}}))
            out = d / "reg.json"
            versions.cmd_registration(versions.argparse.Namespace(
                signed=str(signed), rollout=100, critical=False, notes=str(d / "none.json"), out=str(out)))
            reg = json.loads(out.read_text())
        pairs = {(a["component"], a["platform"], a["filename"]) for a in reg["assets"]}
        self.assertIn(("saba-core", "linux-x64", "saba-core-linux-x64.zip"), pairs)
        self.assertIn(("locales", "linux-x64", "locales.zip"), pairs)
        self.assertIn(("installer", "windows-x64", "saba-chan-installer.exe"), pairs)
        self.assertNotIn(("installer", "linux-x64", "saba-chan-installer.exe"), pairs)


if __name__ == "__main__":
    unittest.main()
