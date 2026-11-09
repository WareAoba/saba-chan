"""versions.py 실시간 업데이트 발행 시험 (부분 릴리즈·조합 관문·베타 먼저·승격):
python3 -m unittest discover -s .github/scripts -p 'test_*.py'"""

import hashlib
import importlib.util
import json
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path

_path = Path(__file__).resolve().parents[2] / "scripts" / "versions.py"
_spec = importlib.util.spec_from_file_location("versions", _path)
versions = importlib.util.module_from_spec(_spec)
sys.modules["versions"] = versions
_spec.loader.exec_module(versions)

from test_versions_manifest import fake_exe  # noqa: E402

RUNNER_RS = """
/// 이 데몬이 붙을 수 있는 러너 형식 범위, 양 끝 포함.
#[allow(dead_code)]
pub const READS_RUNNER_FORMATS: (u32, u32) = (0, 1);
"""


def data(core="0.1.14", cli="0.1.14", gui="0.1.13", locales="0.1.13", release="0.1.14"):
    return {
        "release": {"version": release, "channel": "stable"},
        "components": {
            "saba-core": {"version": core, "asset": "saba-core-{platform}.zip", "exes": ["saba-core.exe"]},
            "cli": {"version": cli, "asset": "saba-chan-cli-{platform}.zip", "exes": ["saba-chan-cli.exe"]},
            "gui": {"version": gui, "asset": "saba-chan-gui-{platform}.zip", "exes": ["saba-chan-gui.exe"]},
            "locales": {"version": locales, "asset": "locales.zip", "install_dir": "locales"},
        },
    }


def entry(asset, version, release="0.1.13", sha="a" * 64, **extra):
    return {"asset": asset, "install_dir": ".", "sha256": sha, "size_bytes": 10,
            "url": f"https://r2.example/releases/{release}/{asset}", "version": version, **extra}


def previous(core="0.1.13", cli="0.1.13", gui="0.1.13", locales="0.1.13", release="0.1.13"):
    def own(p):
        return {
            "saba-core": entry(f"saba-core-{p}.zip", core, release, verified_with_core="0.1.12"),
            "cli": entry(f"saba-chan-cli-{p}.zip", cli, release),
            "gui": entry(f"saba-chan-gui-{p}.zip", gui, release, sha="b" * 64),
            "locales": entry("locales.zip", locales, release, sha="c" * 64),
        }
    return {"version": release, "channel": "stable", "platform": "windows-x64", "components": own("windows-x64"),
            "platforms": {"linux-x64": {"components": own("linux-x64")}}}


class ReleasePlan(unittest.TestCase):
    """H 부분 릴리즈: 지난 릴리즈와 버전이 같은 컴포넌트는 빌드하지 않고 지난 항목을 쓴다"""

    def test_raised_versions_are_built_and_the_rest_reused(self):
        plan = versions.release_plan(data(), previous())
        self.assertEqual(plan["build"], ["cli", "saba-core"])
        self.assertEqual(plan["reuse"], ["gui", "locales"])
        self.assertTrue(plan["binaries"])
        self.assertFalse(plan["locales"])

    def test_only_locales_changed_needs_no_binary_build(self):
        plan = versions.release_plan(data(core="0.1.13", cli="0.1.13", locales="0.1.14"), previous())
        self.assertEqual(plan["build"], ["locales"])
        self.assertFalse(plan["binaries"])
        self.assertTrue(plan["locales"])

    def test_a_component_the_previous_release_lacks_is_built(self):
        prev = previous()
        for listed in [prev["components"], prev["platforms"]["linux-x64"]["components"]]:
            del listed["gui"]
        self.assertIn("gui", versions.release_plan(data(), prev)["build"])

    def test_a_component_lower_than_the_previous_release_is_refused(self):
        with self.assertRaises(SystemExit):
            versions.release_plan(data(gui="0.1.12"), previous())

    def test_a_release_not_above_the_previous_one_is_refused(self):
        with self.assertRaises(SystemExit):
            versions.release_plan(data(release="0.1.13"), previous())

    def test_the_signed_envelope_is_read_like_the_manifest(self):
        plan = versions.release_plan(data(), {"manifest": previous(), "signature": "x", "keyId": "k"})
        self.assertEqual(plan["reuse"], ["gui", "locales"])


class RunnerFormats(unittest.TestCase):
    """manifest saba-core.reads_runner_formats는 데몬 소스의 상수에서 읽는다"""

    def test_the_constant_is_read_from_the_daemon_source(self):
        with tempfile.TemporaryDirectory() as d:
            src = Path(d) / "runner.rs"
            src.write_text(RUNNER_RS.replace("(0, 1)", "(1, 3)"), encoding="utf-8")
            self.assertEqual(versions.runner_formats(src), [1, 3])

    def test_a_missing_constant_stops_the_release(self):
        with tempfile.TemporaryDirectory() as d:
            src = Path(d) / "runner.rs"
            src.write_text("pub const OTHER: u32 = 1;\n", encoding="utf-8")
            with self.assertRaises(SystemExit):
                versions.runner_formats(src)
            with self.assertRaises(SystemExit):
                versions.runner_formats(Path(d) / "missing.rs")


class PartialManifest(unittest.TestCase):
    """버전이 같은 컴포넌트는 지난 manifest 항목(url·sha256·version)을 그대로 싣는다.
    컴포넌트마다 그 조합의 코어 판을 verified_with_core로 싣는다"""

    def build(self, values=None, prev=None, files=("windows-x64", "linux-x64")):
        values = values or data()
        prev = prev or previous()
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            runner = d / "runner.rs"
            runner.write_text(RUNNER_RS, encoding="utf-8")
            for p in files:
                exe = ".exe" if p.startswith("windows") else ""
                for comp, name in (("saba-core", "saba-core"), ("cli", "saba-chan-cli"), ("gui", "saba-chan-gui")):
                    with zipfile.ZipFile(d / f"{values['components'][comp]['asset'].format(platform=p)}", "w") as zf:
                        v = values["components"][comp]["version"]
                        zf.writestr(name + exe, fake_exe(v) if exe else b"\x7fELF")
            with zipfile.ZipFile(d / "locales.zip", "w") as zf:
                zf.writestr("ko/gui.json", "{}")
            (d / "previous.json").write_text(json.dumps({"manifest": prev, "signature": "x", "keyId": "k"}))
            out = d / "out.json"
            args = versions.argparse.Namespace(
                assets=str(d), base_url="https://r2.example", platform="windows-x64", also_platform=["linux-x64"],
                channel=None, out=str(out), modules_repo=None, extensions_repo=None, addons_ref=None,
                previous=str(d / "previous.json"), runner_source=str(runner))
            versions.cmd_manifest(values, args)
            return json.loads(out.read_text())

    def test_unchanged_components_keep_the_previous_entry(self):
        m = self.build()
        for listed, p in ((m["components"], "windows-x64"), (m["platforms"]["linux-x64"]["components"], "linux-x64")):
            gui = listed["gui"]
            self.assertEqual(gui["version"], "0.1.13")
            self.assertEqual(gui["sha256"], "b" * 64)
            self.assertEqual(gui["url"], f"https://r2.example/releases/0.1.13/saba-chan-gui-{p}.zip")
            self.assertEqual(listed["locales"]["url"], "https://r2.example/releases/0.1.13/locales.zip")

    def test_raised_components_point_at_the_new_release(self):
        m = self.build()
        core = m["components"]["saba-core"]
        self.assertEqual(core["version"], "0.1.14")
        self.assertEqual(core["url"], "https://r2.example/releases/0.1.14/saba-core-windows-x64.zip")
        self.assertNotEqual(core["sha256"], "a" * 64)

    def test_every_component_records_the_core_it_was_verified_with(self):
        m = self.build()
        for listed in (m["components"], m["platforms"]["linux-x64"]["components"]):
            self.assertEqual({k: c["verified_with_core"] for k, c in listed.items()},
                             {k: "0.1.14" for k in listed})

    def test_a_new_core_carries_the_runner_formats_it_reads(self):
        m = self.build()
        self.assertEqual(m["components"]["saba-core"]["reads_runner_formats"], [0, 1])
        self.assertEqual(m["platforms"]["linux-x64"]["components"]["saba-core"]["reads_runner_formats"], [0, 1])
        self.assertNotIn("reads_runner_formats", m["components"]["cli"])

    def test_a_reused_core_keeps_what_its_own_release_said(self):
        # 지난 코어(형식 칸이 없던 판)를 그대로 쓰면 소스 상수를 싣지 않는다: 그 코어가 읽는 범위가 아니다
        m = self.build(values=data(core="0.1.13", gui="0.1.14"))
        core = m["components"]["saba-core"]
        self.assertEqual(core["url"], "https://r2.example/releases/0.1.13/saba-core-windows-x64.zip")
        self.assertNotIn("reads_runner_formats", core)
        self.assertEqual(core["verified_with_core"], "0.1.13")
        self.assertEqual(m["components"]["gui"]["verified_with_core"], "0.1.13")

    def test_only_locales_changed_needs_no_binaries(self):
        # 빌드 작업을 건너뛰어 에셋 폴더에 실행 파일 zip이 없어도 지난 항목으로 manifest가 선다
        m = self.build(values=data(core="0.1.13", cli="0.1.13", locales="0.1.14"), files=())
        self.assertEqual(m["components"]["locales"]["url"], "https://r2.example/releases/0.1.14/locales.zip")
        self.assertEqual(m["platforms"]["linux-x64"]["components"]["saba-core"]["version"], "0.1.13")

    def test_a_lower_component_than_the_previous_release_is_refused(self):
        with self.assertRaises(SystemExit):
            self.build(values=data(gui="0.1.12"))


def registration(manifest, channel=None, release="0.1.14"):
    with tempfile.TemporaryDirectory() as d:
        d = Path(d)
        (d / "signed.json").write_text(json.dumps({"manifest": manifest, "signature": "x", "keyId": "k"}))
        (d / "notes.json").write_text(json.dumps({"ko": "노트"}))
        out = d / "out.json"
        args = versions.argparse.Namespace(signed=str(d / "signed.json"), rollout=100, critical=False,
                                           notes=str(d / "notes.json"), out=str(out), channel=channel)
        versions.cmd_registration(args)
        return json.loads(out.read_text())


def partial_manifest():
    return {"version": "0.1.14", "channel": "stable", "platform": "windows-x64", "components": {
        "saba-core": entry("saba-core-windows-x64.zip", "0.1.14", "0.1.14", sha="d" * 64),
        "gui": entry("saba-chan-gui-windows-x64.zip", "0.1.13", "0.1.13", sha="b" * 64),
    }}


class BetaFirstRegistration(unittest.TestCase):
    """등록은 늘 베타 먼저. 서명된 manifest 채널은 승격할 수 있는 끝(versions.toml 채널)이다"""

    def test_registration_channel_overrides_the_signed_channel(self):
        body = registration(partial_manifest(), channel="beta")
        self.assertEqual(body["channel"], "beta")

    def test_a_beta_signed_release_cannot_be_registered_on_stable(self):
        m = partial_manifest()
        m["channel"] = "beta"
        with self.assertRaises(SystemExit):
            registration(m, channel="stable")

    def test_reused_assets_keep_their_previous_r2_key(self):
        body = registration(partial_manifest(), channel="beta")
        keys = {a["component"]: a["r2Key"] for a in body["assets"]}
        self.assertEqual(keys["saba-core"], "releases/0.1.14/saba-core-windows-x64.zip")
        self.assertEqual(keys["gui"], "releases/0.1.13/saba-chan-gui-windows-x64.zip")

    def test_only_new_files_are_uploaded(self):
        body = registration(partial_manifest(), channel="beta")
        self.assertEqual(versions.r2_uploads(body),
                         [("saba-core-windows-x64.zip", "releases/0.1.14/saba-core-windows-x64.zip")])
        # 지난 릴리즈 파일은 올리지 않고 R2에 아직 있는지만 본다
        self.assertEqual(versions.r2_uploads(body, reused=True),
                         [("saba-chan-gui-windows-x64.zip", "releases/0.1.13/saba-chan-gui-windows-x64.zip")])


class Combination(unittest.TestCase):
    """조합 관문: 이번에 빌드한 파일과 지난 릴리즈 파일을 manifest의 sha256으로 맞춰 한 폴더에 모은다"""

    def manifest(self, blobs):
        return {"version": "0.1.14", "platform": "windows-x64", "components": {}, "platforms": {"linux-x64": {"components": {
            key: {"asset": name, "sha256": hashlib.sha256(blob).hexdigest(), "size_bytes": len(blob),
                  "url": f"https://r2.example/releases/0.1.13/{name}", "version": "0.1.13"}
            for key, (name, blob) in blobs.items()}}}}

    def test_built_files_are_used_and_reused_ones_fetched(self):
        m = self.manifest({"saba-core": ("saba-core-linux-x64.zip", b"core"), "cli": ("saba-chan-cli-linux-x64.zip", b"cli")})
        fetched = []
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            (d / "assets").mkdir()
            (d / "assets" / "saba-core-linux-x64.zip").write_bytes(b"core")
            got = versions.assemble_assets(m, "linux-x64", ["saba-core", "cli"], d / "assets", d / "out",
                                           lambda url: fetched.append(url) or b"cli")
            self.assertEqual(sorted(p.name for p in got), ["saba-chan-cli-linux-x64.zip", "saba-core-linux-x64.zip"])
            self.assertEqual((d / "out" / "saba-chan-cli-linux-x64.zip").read_bytes(), b"cli")
        self.assertEqual(fetched, ["https://r2.example/releases/0.1.13/saba-chan-cli-linux-x64.zip"])

    def test_a_file_that_does_not_match_the_manifest_stops_the_gate(self):
        m = self.manifest({"cli": ("saba-chan-cli-linux-x64.zip", b"cli")})
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            with self.assertRaises(SystemExit):
                versions.assemble_assets(m, "linux-x64", ["cli"], d, d / "out", lambda url: b"tampered")

    def test_a_missing_component_stops_the_gate(self):
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaises(SystemExit):
                versions.assemble_assets(self.manifest({}), "linux-x64", ["saba-core"], Path(d), Path(d) / "out", None)


class Promotion(unittest.TestCase):
    """정식 승격은 사용자가 수동으로: 베타에 있는 그 판 하나를 고르고, 서명이 허락하는 채널인지 본다"""

    LISTING = {"releases": [
        {"id": 7, "version": "0.1.14", "channel": "beta", "yanked": False, "rollbacks": 0},
        {"id": 6, "version": "0.1.14-beta.2", "channel": "beta", "yanked": False, "rollbacks": 2},
        {"id": 5, "version": "0.1.13", "channel": "stable", "yanked": False, "rollbacks": 0},
        {"id": 4, "version": "0.1.12", "channel": "beta", "yanked": True, "rollbacks": 0},
    ]}

    def test_the_beta_row_of_that_version_is_picked(self):
        self.assertEqual(versions.promotion_release_id(self.LISTING, "0.1.14", "stable", False), 7)

    def test_a_version_already_on_that_channel_is_refused(self):
        with self.assertRaises(SystemExit):
            versions.promotion_release_id(self.LISTING, "0.1.13", "stable", False)

    def test_unknown_or_yanked_versions_are_refused(self):
        for v in ("0.1.99", "0.1.12"):
            with self.assertRaises(SystemExit):
                versions.promotion_release_id(self.LISTING, v, "stable", False)

    def test_rolled_back_releases_need_an_explicit_yes(self):
        with self.assertRaises(SystemExit):
            versions.promotion_release_id(self.LISTING, "0.1.14-beta.2", "rc", False)
        self.assertEqual(versions.promotion_release_id(self.LISTING, "0.1.14-beta.2", "rc", True), 6)

    def test_the_signed_channel_must_reach_the_target(self):
        signed = {"manifest": {"version": "0.1.14", "channel": "beta"}}
        self.assertIsNotNone(versions.promotion_channel_problem(signed, "stable"))
        signed["manifest"]["channel"] = "stable"
        self.assertIsNone(versions.promotion_channel_problem(signed, "stable"))
        self.assertIsNone(versions.promotion_channel_problem(signed, "rc"))


if __name__ == "__main__":
    unittest.main()
