"""versions.py manifest --previous 시험: 바뀐 모듈·익스텐션이 같은 버전으로 다시 나가지 않게 한다"""

import importlib.util
import io
import sys
import unittest
import zipfile
from pathlib import Path
import tempfile

_path = Path(__file__).resolve().parents[2] / "scripts" / "versions.py"
_spec = importlib.util.spec_from_file_location("versions", _path)
versions = importlib.util.module_from_spec(_spec)
sys.modules["versions"] = versions
_spec.loader.exec_module(versions)


def zip_bytes(files, date_time=(2026, 10, 24, 18, 0, 0)):
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w", zipfile.ZIP_DEFLATED) as zf:
        for name, blob in files.items():
            zf.writestr(zipfile.ZipInfo(name, date_time), blob)
    return buf.getvalue()


class AddonVersionGuard(unittest.TestCase):
    """앱 업데이트는 버전이 오른 모듈·익스텐션만 받는다. 바뀐 것을 같은 버전으로 실으면 사용자는 영영 받지 못한다"""

    def problems(self, version, new_files, previous=None):
        prev_zip = zip_bytes({"module.toml": b"v1", "lifecycle.py": b"print(1)"})
        previous = previous or {"version": "0.1.10", "components": {
            "module-game": {"version": "0.1.5", "url": "https://r2.example/releases/0.1.10/module-game.zip"},
        }}
        fetched = []

        def fetch(url):
            fetched.append(url)
            return prev_zip

        with tempfile.TemporaryDirectory() as d:
            (Path(d) / "module-game.zip").write_bytes(zip_bytes(new_files, date_time=(2026, 10, 26, 3, 0, 0)))
            components = {"module-game": {"version": version, "asset": "module-game.zip"},
                          "ext-new": {"version": "0.1.0", "asset": "ext-new.zip"}}
            return versions.addon_version_problems(components, Path(d), previous, fetch), fetched

    def test_unchanged_files_may_keep_their_version(self):
        problems, fetched = self.problems("0.1.5", {"lifecycle.py": b"print(1)", "module.toml": b"v1"})
        self.assertEqual(problems, [])
        self.assertEqual(fetched, ["https://r2.example/releases/0.1.10/module-game.zip"])

    def test_changed_files_with_the_same_version_are_refused(self):
        problems, _ = self.problems("0.1.5", {"module.toml": b"v1", "lifecycle.py": b"print(2)"})
        self.assertEqual(len(problems), 1)
        self.assertIn("module-game", problems[0])
        self.assertIn("0.1.5", problems[0])

    def test_a_lower_version_than_published_is_refused(self):
        problems, fetched = self.problems("0.1.4", {"module.toml": b"v1", "lifecycle.py": b"print(1)"})
        self.assertEqual(len(problems), 1)
        self.assertEqual(fetched, [], "no download needed")

    def test_a_raised_version_and_a_new_addon_pass_without_downloads(self):
        problems, fetched = self.problems("0.1.6", {"module.toml": b"v2"})
        self.assertEqual((problems, fetched), ([], []))

    def test_test_files_in_the_previous_zip_do_not_count_as_a_change(self):
        """테스트 파일을 싣던 지난 zip과 비교해도, 테스트를 뺀 새 zip은 같은 내용이다"""
        previous = {"version": "0.1.10", "components": {
            "module-game": {"version": "0.1.5", "url": "https://r2.example/releases/0.1.10/module-game.zip"},
        }}
        old = zip_bytes({"module.toml": b"v1", "lifecycle.py": b"print(1)", "test_lifecycle.py": b"x", "conftest.py": b"y"})
        with tempfile.TemporaryDirectory() as d:
            (Path(d) / "module-game.zip").write_bytes(zip_bytes({"module.toml": b"v1", "lifecycle.py": b"print(1)"}))
            components = {"module-game": {"version": "0.1.5", "asset": "module-game.zip"}}
            self.assertEqual(versions.addon_version_problems(components, Path(d), previous, lambda url: old), [])


class ZipFolderSkipsDevFiles(unittest.TestCase):
    def test_module_zip_has_no_test_files(self):
        with tempfile.TemporaryDirectory() as d:
            src = Path(d) / "game"
            (src / "locales").mkdir(parents=True)
            (src / "tests").mkdir()
            for name in ["module.toml", "lifecycle.py", "test_lifecycle.py", "test_extra.py", "conftest.py",
                         "locales/test_en.json", "tests/a.py"]:
                (src / name).write_text("x")
            dest = Path(d) / "out.zip"
            versions.zip_folder(src, dest)
            with zipfile.ZipFile(dest) as zf:
                self.assertEqual(sorted(zf.namelist()), ["lifecycle.py", "locales/test_en.json", "module.toml"])


class ModuleCatalogMeta(unittest.TestCase):
    """받기 전 목록에서도 앱 언어의 이름·설명과 돌아가는 OS·CPU를 보이도록 모듈 정보를 meta에 싣는다"""

    def meta(self, toml, locales=None):
        with tempfile.TemporaryDirectory() as d:
            repo = Path(d) / "modules"
            game = repo / "game"
            (game / "locales").mkdir(parents=True)
            (game / "module.toml").write_text(toml, encoding="utf-8")
            for lang, body in (locales or {}).items():
                (game / "locales" / f"{lang}.json").write_text(body, encoding="utf-8")
            args = type("Args", (), {"modules_repo": str(repo), "extensions_repo": None, "addons_ref": None})()
            assets = Path(d) / "assets"
            assets.mkdir()
            return versions.addon_components(args, assets)["module-game"]["meta"]

    def test_declared_platforms_architectures_and_translations_go_into_meta(self):
        meta = self.meta(
            '[module]\nname = "game"\nversion = "1.0.0"\ndescription = "Game server"\ndisplay_name = "Game"\n'
            '[extensions.saba-core]\nplatforms = ["windows", "linux"]\narchitectures = ["x86_64"]\n',
            {"ko": '{"module": {"display_name": "게임", "description": "게임 서버"}}',
             "ja": '{"module": {"description": "ゲームサーバー"}}',
             "broken": "{not json"},
        )
        self.assertEqual(meta["platforms"], ["windows", "linux"])
        self.assertEqual(meta["architectures"], ["x86_64"])
        self.assertEqual(meta["display_names"], {"ko": "게임"})
        self.assertEqual(meta["descriptions"], {"ko": "게임 서버", "ja": "ゲームサーバー"})
        self.assertEqual(meta["description"], "Game server")

    def test_undeclared_values_are_left_out(self):
        meta = self.meta('[module]\nname = "game"\nversion = "1.0.0"\n[extensions.saba-core]\nplatforms = "linux"\n')
        for key in ("platforms", "architectures", "display_names", "descriptions"):
            self.assertNotIn(key, meta)


if __name__ == "__main__":
    unittest.main()
