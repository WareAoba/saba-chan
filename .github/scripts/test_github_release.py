"""github-release.py 시험: python3 -m unittest discover -s .github/scripts -p 'test_*.py'"""

import hashlib
import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path

_path = Path(__file__).resolve().parent / "github-release.py"
_spec = importlib.util.spec_from_file_location("github_release", _path)
github_release = importlib.util.module_from_spec(_spec)
sys.modules["github_release"] = github_release
_spec.loader.exec_module(github_release)

BASE = "https://r2.example/releases/1.0.0"


def comp(asset, data, version="1.0.0"):
    return {"asset": asset, "url": f"{BASE}/{asset}", "version": version,
            "sha256": hashlib.sha256(data).hexdigest(), "size_bytes": len(data)}


FILES = {"saba-core-windows-x64.zip": b"core-win", "saba-core-linux-x64.zip": b"core-linux",
         "locales.zip": b"locales", "module-minecraft.zip": b"mc"}


def envelope(version="1.0.0", channel="beta"):
    shared = {"locales": comp("locales.zip", FILES["locales.zip"]),
              "module-minecraft": comp("module-minecraft.zip", FILES["module-minecraft.zip"], "0.1.9")}
    return {"keyId": "saba-updater-v1", "signature": "sig", "manifest": {
        "version": version, "channel": channel, "platform": "windows-x64",
        "components": {"saba-core": comp("saba-core-windows-x64.zip", FILES["saba-core-windows-x64.zip"]), **shared},
        "platforms": {"linux-x64": {"components": {
            "saba-core": comp("saba-core-linux-x64.zip", FILES["saba-core-linux-x64.zip"]), **shared}}}}}


class Assets(unittest.TestCase):
    def test_every_platform_asset_once(self):
        got = github_release.assets(envelope(), "1.0.0")
        self.assertEqual(sorted(a["asset"] for a in got), sorted(FILES))

    def test_rejects_manifest_of_other_version(self):
        with self.assertRaises(SystemExit):
            github_release.assets(envelope(version="1.0.1"), "1.0.0")

    def test_rejects_same_name_with_different_bytes(self):
        env = envelope()
        env["manifest"]["platforms"]["linux-x64"]["components"]["locales"] = comp("locales.zip", b"other")
        with self.assertRaises(SystemExit):
            github_release.assets(env, "1.0.0")


class Download(unittest.TestCase):
    def test_keeps_file_only_when_hash_and_size_match(self):
        with tempfile.TemporaryDirectory() as d:
            out = Path(d)
            a = comp("locales.zip", b"locales")
            github_release.download(a, out, fetch=lambda url: b"locales")
            self.assertEqual((out / "locales.zip").read_bytes(), b"locales")
            with self.assertRaises(SystemExit):
                github_release.download(comp("x.zip", b"good"), out, fetch=lambda url: b"evil")
            self.assertFalse((out / "x.zip").exists())

    def test_rejects_asset_name_with_path(self):
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaises(SystemExit):
                github_release.download(comp("../x.zip", b"x"), Path(d), fetch=lambda url: b"x")


class Notes(unittest.TestCase):
    def test_korean_then_english_with_checksums_hint(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "n.json"
            p.write_text(json.dumps({"ko": "한국어 노트", "en": "English notes"}), encoding="utf-8")
            body = github_release.notes(p)
            self.assertLess(body.index("한국어 노트"), body.index("English notes"))
            self.assertIn("SHA256SUMS.txt", body)


if __name__ == "__main__":
    unittest.main()
