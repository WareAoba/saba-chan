"""package-release.py 단위 시험: python3 -m unittest discover -s .github/scripts -p 'test_*.py'"""

import importlib.util
import unittest
from pathlib import Path

_spec = importlib.util.spec_from_file_location(
    "package_release", Path(__file__).with_name("package-release.py"))
pr = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(pr)


def rel(tag, prerelease=False):
    return {"tag": tag, "prerelease": prerelease, "assets": ["manifest.json", "saba-core-windows-x64.zip"]}


class PreviousReleaseOrder(unittest.TestCase):
    def tags(self, releases, current):
        return [r["tag"] for r in pr.previous_release_candidates(releases, current)]

    def test_stable_release_never_reuses_a_prerelease(self):
        releases = [rel("v0.2.0-beta.1", prerelease=True), rel("v0.1.5"), rel("v0.1.4")]
        self.assertEqual(self.tags(releases, "v0.1.6"), ["v0.1.5", "v0.1.4"])

    def test_only_older_versions_newest_first_whatever_the_api_order(self):
        releases = [rel("v0.1.4"), rel("v0.1.7"), rel("v0.1.6"), rel("v0.1.5"), rel("v0.1.10")]
        self.assertEqual(self.tags(releases, "v0.1.6"), ["v0.1.5", "v0.1.4"])

    def test_prerelease_may_reuse_older_prereleases_and_stables(self):
        releases = [rel("v0.2.0-beta.2", True), rel("v0.2.0-beta.1", True), rel("v0.1.5")]
        self.assertEqual(self.tags(releases, "v0.2.0-beta.2"), ["v0.2.0-beta.1", "v0.1.5"])

    def test_beta_numbers_compare_as_numbers(self):
        releases = [rel("v0.2.0-beta.9", True), rel("v0.2.0-beta.10", True)]
        self.assertEqual(self.tags(releases, "v0.2.0-beta.11"), ["v0.2.0-beta.10", "v0.2.0-beta.9"])

    def test_stable_release_skips_prerelease_tags_even_if_not_flagged(self):
        releases = [rel("v0.1.6-beta.1", prerelease=False), rel("v0.1.5")]
        self.assertEqual(self.tags(releases, "v0.1.6"), ["v0.1.5"])

    def test_unparseable_tags_are_skipped(self):
        self.assertEqual(self.tags([rel("nightly"), rel("v0.1.5")], "v0.1.6"), ["v0.1.5"])


class CoreZipRunner(unittest.TestCase):
    def zip_with(self, names):
        import tempfile, zipfile
        d = Path(tempfile.mkdtemp())
        z = d / "saba-core.zip"
        with zipfile.ZipFile(z, "w") as zf:
            for n in names:
                zf.writestr(n, b"x")
        return z

    def test_zip_must_carry_the_runner_at_its_root(self):
        self.assertTrue(pr.zip_has_root_file(self.zip_with(["saba-core.exe", "saba-runner.exe"]), "saba-runner.exe"))
        self.assertFalse(pr.zip_has_root_file(self.zip_with(["saba-core.exe", "x/saba-runner.exe"]), "saba-runner.exe"))


class RequiredComponents(unittest.TestCase):
    def test_a_release_without_gui_or_installer_is_refused(self):
        self.assertEqual(pr.missing_required({"saba-core": {}, "cli": {}, "locales": {}}), ["gui", "installer"])

    def test_a_complete_release_passes(self):
        self.assertEqual(pr.missing_required({"saba-core": {}, "gui": {}, "installer": {}}), [])

    def test_versions_file_is_found_from_any_working_folder(self):
        import os
        import tempfile
        here = os.getcwd()
        with tempfile.TemporaryDirectory() as tmp:
            os.chdir(tmp)
            try:
                self.assertEqual(pr.missing_required({"saba-core": {}}), ["gui", "installer"])
                self.assertIsNotNone(pr.read_bundled_version("saba-core"))
            finally:
                os.chdir(here)


if __name__ == "__main__":
    unittest.main()


class OtherPlatformAssets(unittest.TestCase):
    """D3c: 같은 릴리즈에 붙은 리눅스 zip을 Windows 원본으로 집지 않는다 (edited 재실행)"""

    def test_linux_zips_are_not_taken_as_windows_raw_assets(self):
        import tempfile
        with tempfile.TemporaryDirectory() as d:
            staging = Path(d)
            # 같은 Windows라도 다른 CPU(arm64) 몫은 이 패키저의 원본이 아니다
            for name in ("saba-core-linux-x64.zip", "saba-core-macos-arm64.zip", "saba-core-windows-arm64.zip", "saba-core-windows-x64.zip"):
                (staging / name).write_bytes(b"PK")
            old, pr.STAGING_DIR = pr.STAGING_DIR, staging
            try:
                found = pr.find_raw_file(pr.COMPONENTS[0]["raw_patterns"])
            finally:
                pr.STAGING_DIR = old
        self.assertEqual(found.name, "saba-core-windows-x64.zip")
