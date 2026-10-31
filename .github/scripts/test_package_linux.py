"""package-linux.py 시험: python3 -m unittest discover -s .github/scripts -p 'test_*.py'"""

import importlib.util
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path

_path = Path(__file__).resolve().parent / "package-linux.py"
_spec = importlib.util.spec_from_file_location("package_linux", _path)
package_linux = importlib.util.module_from_spec(_spec)
sys.modules["package_linux"] = package_linux
_spec.loader.exec_module(package_linux)


class LinuxZips(unittest.TestCase):
    @staticmethod
    def built(d: Path, exes, magic=b"\x7fELF", headless=True) -> Path:
        """`d`에 창 빌드, `d/headless`에 창 없이 빌드한 설치기를 둔다"""
        for exe in exes:
            (d / exe).write_bytes(magic)
        if headless:
            (d / "headless").mkdir()
            (d / "headless" / "saba-chan-installer").write_bytes(magic + b"-headless")
        return d

    def test_binaries_are_executable_and_core_carries_the_runner(self):
        with tempfile.TemporaryDirectory() as d:
            d = self.built(Path(d), ("saba-core", "saba-runner", "saba-chan-cli", "saba-chan-installer", "saba-chan-gui"))
            made = package_linux.package(d, d / "out", "linux-x64", headless_bin=d / "headless")
            self.assertEqual(sorted(p.name for p in made), [
                "saba-chan-cli-linux-x64.zip", "saba-chan-gui-linux-x64.zip",
                "saba-chan-installer-desktop-linux-x64.zip", "saba-chan-installer-headless-linux-x64.zip",
                "saba-core-linux-x64.zip"])
            with zipfile.ZipFile(d / "out" / "saba-core-linux-x64.zip") as zf:
                modes = {i.filename: (i.external_attr >> 16) & 0o777 for i in zf.infolist()}
            with zipfile.ZipFile(d / "out" / "saba-chan-gui-linux-x64.zip") as zf:
                gui = {i.filename: (i.external_attr >> 16) & 0o777 for i in zf.infolist()}
            # 설치기는 한 프로그램을 두 번 빌드한다: 데스크톱은 창 빌드, 서버는 GTK·WebKit 없이 도는 창 없는 빌드
            with zipfile.ZipFile(d / "out" / "saba-chan-installer-desktop-linux-x64.zip") as zf:
                desktop = {i.filename: zf.read(i) for i in zf.infolist()}
            with zipfile.ZipFile(d / "out" / "saba-chan-installer-headless-linux-x64.zip") as zf:
                headless = {i.filename: ((i.external_attr >> 16) & 0o777, zf.read(i)) for i in zf.infolist()}
        self.assertEqual(modes, {"saba-core": 0o755, "saba-runner": 0o755, "i18n.py": 0o644, "daemon_rcon.py": 0o644, "common_locales.json": 0o644})
        # 데스크톱 리눅스 GUI: 앱 메뉴 항목(설치기)이 GUI 옆 아이콘을 가리킨다
        self.assertEqual(gui, {"saba-chan-gui": 0o755, "saba-chan-gui.png": 0o644})
        self.assertEqual(desktop, {"saba-chan-installer": b"\x7fELF"})
        self.assertEqual(headless, {"saba-chan-installer": (0o755, b"\x7fELF-headless")})

    def test_arm64_packs_only_the_listed_components(self):
        # linux-arm64는 GUI 없이 헤드리스 구성만 싣는다
        with tempfile.TemporaryDirectory() as d:
            d = self.built(Path(d), ("saba-core", "saba-runner", "saba-chan-cli"))
            made = package_linux.package(d, d / "out", "linux-arm64", only=["saba-core", "cli", "installer.headless"],
                                         headless_bin=d / "headless")
            self.assertEqual(sorted(p.name for p in made), [
                "saba-chan-cli-linux-arm64.zip", "saba-chan-installer-headless-linux-arm64.zip", "saba-core-linux-arm64.zip"])

    def test_the_mac_gui_carries_the_app_bundle_files(self):
        # 맥 설치기는 GUI zip의 Info.plist·아이콘을 앱 묶음(Contents) 자리에 놓는다. 업데이트도 같은 파일을 다시 놓는다
        import plistlib
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            d = self.built(d, ("saba-core", "saba-runner", "saba-chan-cli", "saba-chan-installer", "saba-chan-gui"), b"\xcf\xfa\xed\xfe")
            package_linux.package(d, d / "out", "macos-arm64", version="0.1.12-beta.1", headless_bin=d / "headless")
            with zipfile.ZipFile(d / "out" / "saba-chan-gui-macos-arm64.zip") as zf:
                names = sorted(zf.namelist())
                plist = plistlib.loads(zf.read("Info.plist"))
                icon = zf.read("saba-chan.icns")[:4]
        self.assertEqual(names, ["Info.plist", "saba-chan-gui", "saba-chan.icns"])
        self.assertEqual(plist["CFBundleShortVersionString"], "0.1.12-beta.1")
        self.assertEqual(plist["CFBundleIconFile"], "saba-chan.icns")
        self.assertEqual(icon, b"icns")

    def test_the_mac_gui_needs_the_release_version(self):
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            d = self.built(d, ("saba-core", "saba-runner", "saba-chan-cli", "saba-chan-installer", "saba-chan-gui"), b"\xcf\xfa\xed\xfe")
            with self.assertRaises(SystemExit):
                package_linux.package(d, d / "out", "macos-arm64", headless_bin=d / "headless")

    def test_a_missing_binary_stops_packaging(self):
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaises(SystemExit):
                package_linux.package(Path(d), Path(d) / "out", "linux-x64")


if __name__ == "__main__":
    unittest.main()
