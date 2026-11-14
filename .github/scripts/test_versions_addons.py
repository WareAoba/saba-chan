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

    def test_the_same_files_zip_to_the_same_bytes(self):
        """다시 실행한 checkout은 파일 시각이 다르다: R2에 이미 올린 zip과 해시가 같아야 등록을 다시 할 수 있다"""
        import os
        with tempfile.TemporaryDirectory() as d:
            src = Path(d) / "game"
            src.mkdir()
            (src / "module.toml").write_text("x")
            (src / "lifecycle.py").write_text("y")
            first, second = Path(d) / "a.zip", Path(d) / "b.zip"
            versions.zip_folder(src, first)
            for f in src.iterdir():
                os.utime(f, (1_900_000_000, 1_900_000_000))
            versions.zip_folder(src, second)
            self.assertEqual(first.read_bytes(), second.read_bytes())

    def test_top_level_dot_entries_are_not_packaged(self):
        """교체는 깐 폴더의 맨 위 점 항목(.git·.env)을 새 폴더로 옮겨 준다: 패키지에 같은 이름이 있으면 부딪힌다"""
        import zipfile
        with tempfile.TemporaryDirectory() as d:
            src = Path(d) / "game"
            (src / ".github").mkdir(parents=True)
            (src / ".github" / "x.yml").write_text("x")
            (src / ".gitignore").write_text("x")
            (src / "sub").mkdir()
            (src / "sub" / ".keep").write_text("x")
            (src / "module.toml").write_text("x")
            out = Path(d) / "a.zip"
            versions.zip_folder(src, out)
            self.assertEqual(sorted(zipfile.ZipFile(out).namelist()), ["module.toml", "sub/.keep"])


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


class ExtensionCatalogMeta(unittest.TestCase):
    """받기 전 익스텐션 목록도 앱 언어의 설명과 아이콘을 보이도록 manifest의 `descriptions`와 아이콘 주소를 meta에 싣는다"""

    def meta(self, manifest, icon=True, ref="abc123"):
        with tempfile.TemporaryDirectory() as d:
            repo = Path(d) / "extensions"
            ext = repo / "pet"
            ext.mkdir(parents=True)
            (ext / "manifest.json").write_text(manifest, encoding="utf-8")
            if icon:
                (ext / "icon.png").write_bytes(b"png")
            args = type("Args", (), {"modules_repo": None, "extensions_repo": str(repo), "addons_ref": None,
                                     "extensions_ref": ref})()
            assets = Path(d) / "assets"
            assets.mkdir()
            return versions.addon_components(args, assets)["ext-pet"]["meta"]

    def test_translated_descriptions_and_icon_go_into_meta(self):
        meta = self.meta('{"id": "pet", "name": "Pet", "version": "0.1.0", "description": "A pet",'
                         ' "descriptions": {"ko": "펫", "ja": "ペット"}}')
        self.assertEqual(meta["descriptions"], {"ko": "펫", "ja": "ペット"})
        self.assertEqual(meta["icon"], "https://raw.githubusercontent.com/WareAoba/saba-chan-extensions/abc123/pet/icon.png")

    def test_no_icon_file_or_ref_leaves_icon_out(self):
        body = '{"id": "pet", "name": "Pet", "version": "0.1.0"}'
        self.assertNotIn("icon", self.meta(body, icon=False))
        self.assertNotIn("icon", self.meta(body, ref=None))
        self.assertNotIn("descriptions", self.meta(body))


if __name__ == "__main__":
    unittest.main()


class AddonPublishEntries(unittest.TestCase):
    """publish-addons.yml: 릴레이에 아직 없는 판만 내고, 버전을 올리지 않은 변경·내린 판 번호 재사용은 막는다"""

    def run_with(self, local_version, published, files=None):
        prev_zip = zip_bytes({"module.toml": b"v1"})
        with tempfile.TemporaryDirectory() as d:
            (Path(d) / "module-game.zip").write_bytes(zip_bytes(files or {"module.toml": b"v1"}, (2026, 11, 9, 0, 0, 0)))
            addons = {"module-game": {"version": local_version, "asset": "module-game.zip",
                                      "install_dir": "modules/game", "requires": {"saba-core": ">=0.1.13"}}}
            return versions.addon_publish_entries(addons, Path(d), published, "https://r2.example/", lambda url: prev_zip)

    def published(self, version="0.1.5", yanked=None):
        return {"addons": [{"manifest": {"key": "module-game", "version": version,
                                         "url": "https://r2.example/addons/module-game/0.1.5/module-game.zip"}}],
                "yanked": [{"manifest": {"key": k, "version": v, "yanked": True}, "signature": "s", "keyId": "k"}
                           for k, vs in (yanked or {}).items() for v in vs]}

    def test_a_new_version_becomes_an_entry_with_its_own_r2_path(self):
        entries, problems = self.run_with("0.1.6", self.published(), {"module.toml": b"v2"})
        self.assertEqual(problems, [])
        self.assertEqual(len(entries), 1)
        e = entries[0]
        self.assertEqual((e["key"], e["version"]), ("module-game", "0.1.6"))
        self.assertEqual(e["url"], "https://r2.example/addons/module-game/0.1.6/module-game.zip")
        self.assertEqual(e["requires"], {"saba-core": ">=0.1.13"})
        self.assertEqual(len(e["sha256"]), 64)

    def test_the_published_version_unchanged_is_skipped(self):
        entries, problems = self.run_with("0.1.5", self.published())
        self.assertEqual((entries, problems), ([], []))

    def test_changed_files_without_a_version_bump_are_refused(self):
        _, problems = self.run_with("0.1.5", self.published(), {"module.toml": b"v2"})
        self.assertEqual(len(problems), 1)

    def test_a_yanked_version_number_is_not_reused(self):
        _, problems = self.run_with("0.1.6", self.published(yanked={"module-game": ["0.1.6"]}), {"module.toml": b"v2"})
        self.assertIn("yanked", problems[0])

    def test_a_module_the_relay_has_never_seen_is_published(self):
        entries, problems = self.run_with("0.1.0", {"addons": [], "yanked": []})
        self.assertEqual(problems, [])
        self.assertEqual([e["version"] for e in entries], ["0.1.0"])


class AddonSeed(unittest.TestCase):
    """따로 내기 전에 앱 릴리즈로 나간 판을 판 목록에 옮겨 담는다: 파일·sha256·주소는 그 릴리즈 그대로"""

    def test_only_modules_and_extensions_become_entries(self):
        release = {"manifest": {"version": "0.1.13", "components": {
            "saba-core": {"version": "0.1.13", "asset": "saba-core.zip", "sha256": "c" * 64, "url": "https://r2/x"},
            "module-game": {"version": "0.1.5", "asset": "module-game.zip", "sha256": "a" * 64,
                            "url": "https://r2/releases/0.1.13/module-game.zip", "requires": {"saba-core": ">=0.1.13-0"}},
            "ext-tool": {"version": "0.1.4", "asset": "ext-tool.zip", "sha256": "b" * 64, "url": "https://r2/releases/0.1.13/ext-tool.zip"},
        }}}
        entries = versions.addon_seed_entries(release)
        self.assertEqual([e["key"] for e in entries], ["ext-tool", "module-game"])
        self.assertEqual(entries[1]["url"], "https://r2/releases/0.1.13/module-game.zip")
        self.assertEqual(entries[1]["requires"], {"saba-core": ">=0.1.13-0"})
