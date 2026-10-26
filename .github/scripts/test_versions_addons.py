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


if __name__ == "__main__":
    unittest.main()
