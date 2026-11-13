"""versions.py 채널 검사 시험: python3 -m unittest discover -s .github/scripts -p 'test_*.py'"""

import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path

_path = Path(__file__).resolve().parents[2] / "scripts" / "versions.py"
_spec = importlib.util.spec_from_file_location("versions", _path)
versions = importlib.util.module_from_spec(_spec)
sys.modules["versions"] = versions
_spec.loader.exec_module(versions)


class ReleaseChannel(unittest.TestCase):
    """정식(stable)·미리보기(rc)·베타(beta)만 발행하고, 정식에는 프리릴리스 번호를 쓰지 않는다"""

    def load(self, version, channel):
        with tempfile.TemporaryDirectory() as d:
            src = Path(d) / "versions.toml"
            src.write_text(f'[release]\nversion = "{version}"\nchannel = "{channel}"\n', encoding="utf-8")
            old, versions.SOURCE = versions.SOURCE, src
            try:
                return versions.load()
            finally:
                versions.SOURCE = old

    def test_known_channels_load(self):
        self.load("0.1.10", "stable")
        self.load("0.1.11-rc.1", "rc")
        self.load("0.1.11-beta.2", "beta")

    def test_an_unknown_channel_is_refused(self):
        with self.assertRaises(SystemExit):
            self.load("0.1.11-beta.1", "nightly")

    def test_a_prerelease_number_on_stable_is_refused(self):
        with self.assertRaises(SystemExit):
            self.load("0.1.11-beta.1", "stable")


if __name__ == "__main__":
    unittest.main()


class ChannelNumbering(unittest.TestCase):
    """번호가 채널을 말한다 (사용자 규칙 2026-11-14): 베타는 X.Y.Z-beta.N, RC는 X.Y.Z-rc.N, 정식은 X.Y.Z.
    꼬리표 없는 번호를 베타로 내면 그 번호의 다음 베타가 더 낮은 번호가 되어 업데이트가 끊긴다(0.1.14 사고)"""

    load = ReleaseChannel.load

    def test_a_beta_needs_a_beta_number(self):
        for version in ("0.1.14", "0.1.14-rc.1", "0.1.14-beta", "0.1.14-beta.x"):
            with self.subTest(version=version), self.assertRaises(SystemExit):
                self.load(version, "beta")

    def test_an_rc_needs_an_rc_number(self):
        for version in ("0.1.14", "0.1.14-beta.2"):
            with self.subTest(version=version), self.assertRaises(SystemExit):
                self.load(version, "rc")

    def test_the_registration_channel_follows_the_number(self):
        self.assertEqual(versions.registration_channel("beta"), "beta")
        self.assertEqual(versions.registration_channel("rc"), "rc")
        # 정식은 지금처럼 베타에 먼저 걸고 promote-release.yml로 올린다
        self.assertEqual(versions.registration_channel("stable"), "beta")

