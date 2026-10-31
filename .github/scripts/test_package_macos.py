"""package-macos.py 시험: python3 -m unittest discover -s .github/scripts -p 'test_*.py'"""

import importlib.util
import plistlib
import sys
import tempfile
import unittest
from pathlib import Path

_path = Path(__file__).resolve().parent / "package-macos.py"
_spec = importlib.util.spec_from_file_location("package_macos", _path)
package_macos = importlib.util.module_from_spec(_spec)
sys.modules["package_macos"] = package_macos
_spec.loader.exec_module(package_macos)


class MacApp(unittest.TestCase):
    def test_bundle_holds_every_binary_and_opens_the_gui(self):
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            for exe in package_macos.EXES:
                (d / exe).write_bytes(b"\xcf\xfa\xed\xfe")
            app = package_macos.build_app(d, d / "out", "0.1.12-beta.1", {"saba-chan.icns": b"icns", "Assets.car": b"car"})
            resources = sorted(p.name for p in (app / "Contents" / "Resources").iterdir())
            macos = sorted(p.name for p in (app / "Contents" / "MacOS").iterdir())
            modes = {p.name: p.stat().st_mode & 0o777 for p in (app / "Contents" / "MacOS").iterdir()}
            with open(app / "Contents" / "Info.plist", "rb") as f:
                plist = plistlib.load(f)
            icon = (app / "Contents" / "Resources" / plist["CFBundleIconFile"]).read_bytes()[:4]
        self.assertEqual(app.name, "Saba-chan.app")
        # 데몬 zip과 같은 공유 모듈 파일도 든다 (.dmg로 넣은 앱도 설치기로 깐 앱과 같은 구성)
        shared = sorted(package_macos.shared_files().values())
        self.assertIn("i18n.py", shared)
        self.assertEqual(macos, sorted(package_macos.EXES + shared))
        self.assertTrue(all(modes[e] == 0o755 for e in package_macos.EXES), modes)
        self.assertTrue(all(modes[f] == 0o644 for f in shared), modes)
        self.assertEqual(plist["CFBundleExecutable"], "saba-chan-gui")
        self.assertEqual(plist["CFBundleIdentifier"], "com.saba-chan.app")
        # 짧은 버전은 그대로, CFBundleVersion은 숫자만
        self.assertEqual(plist["CFBundleShortVersionString"], "0.1.12-beta.1")
        self.assertEqual(plist["CFBundleVersion"], "0.1.12")
        self.assertEqual(icon, b"icns")
        self.assertEqual(resources, ["Assets.car", "saba-chan.icns"])
        self.assertEqual(plist["CFBundleIconName"], "AppIcon")

    def test_bundle_id_matches_the_gui(self):
        import json
        conf = json.loads((package_macos.ROOT / "saba-chan-gui/src-tauri/tauri.conf.json").read_text(encoding="utf-8"))
        plist = plistlib.loads(package_macos.info_plist("0.1.12"))
        self.assertEqual(conf["identifier"], plist["CFBundleIdentifier"])

    def test_first_open_note_has_every_language_korean_first(self):
        import json
        note = package_macos.first_open_note()
        locales = sorted(p.name for p in (package_macos.ROOT / "locales").iterdir() if (p / "common.json").is_file())
        titles = {l: json.loads((package_macos.ROOT / "locales" / l / "common.json").read_text(encoding="utf-8"))["macos_first_open"]["title"] for l in locales}
        self.assertEqual(len(titles), 10)
        self.assertTrue(note.startswith(titles["ko"]), note[:80])
        for title in titles.values():
            self.assertIn(title, note)

    def test_missing_binary_stops(self):
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaises(SystemExit):
                package_macos.build_app(Path(d), Path(d) / "out", "0.1.12", {})


if __name__ == "__main__":
    unittest.main()
