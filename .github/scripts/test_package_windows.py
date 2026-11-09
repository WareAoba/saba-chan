"""package-windows.py 시험: python3 -m unittest discover -s .github/scripts -p 'test_*.py'"""

import importlib.util
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path

_path = Path(__file__).resolve().parent / "package-windows.py"
_spec = importlib.util.spec_from_file_location("package_windows", _path)
package_windows = importlib.util.module_from_spec(_spec)
sys.modules["package_windows"] = package_windows
_spec.loader.exec_module(package_windows)

EXES = ("saba-core.exe", "saba-runner.exe", "saba-chan-cli.exe", "saba-chan.exe", "saba-chan-gui.exe", "saba-chan-installer.exe")


class WindowsAssets(unittest.TestCase):
    def test_assets_match_versions_toml_names_and_core_carries_runner(self):
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            for exe in EXES:
                (d / exe).write_bytes(b"MZ" + exe.encode())
            made = package_windows.package(d, d / "out")
            self.assertEqual(sorted(p.name for p in made), [
                "locales.zip", "saba-chan-cli-windows-x64.zip", "saba-chan-gui-windows-x64.zip",
                "saba-chan-installer.exe", "saba-core-windows-x64.zip"])
            with zipfile.ZipFile(d / "out" / "saba-core-windows-x64.zip") as zf:
                self.assertEqual(sorted(zf.namelist()), [
                    "common_locales.json", "daemon_rcon.py", "i18n.py", "saba-core.exe", "saba-runner.exe"])
            # 짧은 명령 실행기는 bin/ 아래: 설치 폴더 바로 아래의 Saba-chan.exe는 0.1.1 GUI 이름이다
            with zipfile.ZipFile(d / "out" / "saba-chan-cli-windows-x64.zip") as zf:
                self.assertEqual(sorted(zf.namelist()), ["bin/saba-chan.exe", "saba-chan-cli.exe"])
            with zipfile.ZipFile(d / "out" / "saba-chan-gui-windows-x64.zip") as zf:
                self.assertEqual(zf.namelist(), ["saba-chan-gui.exe"])
            # 설치기는 zip이 아닌 exe 그대로: 소개 페이지 다운로드와 업데이트가 이 파일을 쓴다
            self.assertEqual((d / "out" / "saba-chan-installer.exe").read_bytes(), b"MZsaba-chan-installer.exe")
            with zipfile.ZipFile(d / "out" / "locales.zip") as zf:
                names = zf.namelist()
            self.assertTrue(names and all(not n.startswith("locales/") and "\\" not in n for n in names))

    def test_missing_executable_stops(self):
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            for exe in EXES[:-1]:
                (d / exe).write_bytes(b"MZ")
            with self.assertRaises(SystemExit):
                package_windows.package(d, d / "out")

    def test_locales_alone_needs_no_executables(self):
        # 부분 릴리즈: 번역만 바뀌면 빌드 작업 없이 locales.zip만 만든다
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            made = package_windows.package_locales(d / "out")
            self.assertEqual([p.name for p in d.joinpath("out").iterdir()], ["locales.zip"])
            with zipfile.ZipFile(made) as zf:
                self.assertIn("ko/bot.json", zf.namelist())


if __name__ == "__main__":
    unittest.main()
