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

    def test_unparseable_tags_are_skipped(self):
        self.assertEqual(self.tags([rel("nightly"), rel("v0.1.5")], "v0.1.6"), ["v0.1.5"])


if __name__ == "__main__":
    unittest.main()
