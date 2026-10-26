"""versions.py signing 시험: python3 -m unittest discover -s .github/scripts -p 'test_*.py'"""

import importlib.util
import struct
import sys
import tempfile
import unittest
from pathlib import Path

_path = Path(__file__).resolve().parents[2] / "scripts" / "versions.py"
_spec = importlib.util.spec_from_file_location("versions", _path)
versions = importlib.util.module_from_spec(_spec)
sys.modules["versions"] = versions
_spec.loader.exec_module(versions)


def version_string(key, value, terminate=True):
    """VS_VERSIONINFO의 String 구조체 하나 (rc.exe는 값 끝 NUL을 넣지 않을 수 있다)"""
    k = key.encode("utf-16-le") + b"\0\0"
    v = value.encode("utf-16-le") + (b"\0\0" if terminate else b"")
    head = 6 + len(k)
    pad = b"\0" * ((4 - head % 4) % 4)
    body = k + pad + v
    return struct.pack("<HHH", 6 + len(body), len(v) // 2, 1) + body


def fake_exe(version, product="Saba-chan", product_version=None, terminate=True, before=b""):
    """파일 버전(VS_FIXEDFILEINFO)과 ProductName·ProductVersion 문자열만 흉내 낸 exe 바이트.
    before는 버전 리소스 앞(코드·데이터 영역)에 둘 바이트"""
    major, minor, patch = (int(p) for p in version.split("-")[0].split("."))
    body = b"MZ" + b"\0" * 62 + before
    body += struct.pack("<IIIIII", 0xFEEF04BD, 0x00010000, major << 16 | minor, patch << 16, major << 16 | minor, patch << 16)
    body += version_string("ProductName", product, terminate)
    body += version_string("ProductVersion", product_version or version, terminate)
    return body + b"\0" * 32


DATA = {
    "release": {"version": "0.1.11", "channel": "stable"},
    "components": {
        "saba-core": {"version": "0.1.11", "asset": "saba-core-{platform}.zip", "exes": ["saba-core.exe", "saba-runner.exe"]},
        "cli": {"version": "0.1.10", "asset": "saba-chan-cli-{platform}.zip", "exes": ["saba-chan-cli.exe"]},
        "locales": {"version": "0.1.11", "asset": "locales.zip"},
    },
}


class SigningCheck(unittest.TestCase):
    """서명 요청 전에 빌드된 exe가 SignPath 제약(제품 이름, 제품 버전)과 versions.toml에 맞는지 본다"""

    def run_signing(self, files):
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            for name, blob in files.items():
                (d / name).write_bytes(blob)
            return versions.signing_parameters(DATA, d)

    def good(self):
        return {
            "saba-core.exe": fake_exe("0.1.11"),
            "saba-runner.exe": fake_exe("0.1.11", terminate=False),
            "saba-chan-cli.exe": fake_exe("0.1.10"),
        }

    def test_matching_build_gives_one_version_parameter_per_component(self):
        self.assertEqual(self.run_signing(self.good()), {"saba_core_version": "0.1.11", "cli_version": "0.1.10"})

    def test_product_name_other_than_the_project_name_is_refused(self):
        files = self.good()
        files["saba-chan-cli.exe"] = fake_exe("0.1.10", product="Saba-chan CLI")
        with self.assertRaises(SystemExit):
            self.run_signing(files)

    def test_exe_built_from_another_version_is_refused(self):
        files = self.good()
        files["saba-runner.exe"] = fake_exe("0.1.10")
        with self.assertRaises(SystemExit):
            self.run_signing(files)

    def test_missing_exe_is_refused(self):
        files = self.good()
        del files["saba-chan-cli.exe"]
        with self.assertRaises(SystemExit):
            self.run_signing(files)

    def test_exe_no_component_declares_is_refused(self):
        # 선언하지 않은 실행 파일이 서명 묶음에 섞여 들어가지 않게 한다
        files = self.good()
        files["other.exe"] = fake_exe("0.1.11")
        with self.assertRaises(SystemExit):
            self.run_signing(files)

    def test_prerelease_product_version_string_is_compared_whole(self):
        data = {"release": DATA["release"], "components": {"cli": {"version": "0.1.11-rc.1", "exes": ["saba-chan-cli.exe"]}}}
        with tempfile.TemporaryDirectory() as d:
            (Path(d) / "saba-chan-cli.exe").write_bytes(fake_exe("0.1.11", product_version="0.1.11"))
            with self.assertRaises(SystemExit):
                versions.signing_parameters(data, Path(d))
            (Path(d) / "saba-chan-cli.exe").write_bytes(fake_exe("0.1.11-rc.1"))
            self.assertEqual(versions.signing_parameters(data, Path(d)), {"cli_version": "0.1.11-rc.1"})

    def test_strings_outside_the_version_resource_are_ignored(self):
        # 데이터 영역에 같은 UTF-16 문자열이 있어도 버전 리소스의 값을 읽는다
        files = self.good()
        files["saba-chan-cli.exe"] = fake_exe("0.1.10", before=version_string("ProductName", "Other"))
        self.assertEqual(self.run_signing(files)["cli_version"], "0.1.10")

    def test_collect_copies_only_declared_exes(self):
        with tempfile.TemporaryDirectory() as src, tempfile.TemporaryDirectory() as dest:
            for name in ("saba-core.exe", "saba-runner.exe", "saba-chan-cli.exe", "build-script-build.exe"):
                (Path(src) / name).write_bytes(b"MZ")
            versions.collect_exes(DATA, Path(src), Path(dest))
            self.assertEqual(sorted(p.name for p in Path(dest).iterdir()),
                             ["saba-chan-cli.exe", "saba-core.exe", "saba-runner.exe"])


class SignPathConfiguration(unittest.TestCase):
    """SignPath에 넣는 산출물 설정이 versions.toml의 exes·매개변수와 같아야 빌드 뒤에 거절되지 않는다"""

    def test_artifact_configuration_matches_versions_toml(self):
        import xml.etree.ElementTree as ET
        ns = {"s": "http://signpath.io/artifact-configuration/v1"}
        root = ET.parse(Path(__file__).resolve().parents[1] / "signpath" / "artifact-configuration.xml").getroot()
        data = versions.load()
        want = {}
        for key, comp in data["components"].items():
            for name in comp.get("exes", []):
                want[name] = "${" + key.replace("-", "_") + "_version}"
        files = {f.get("path"): f for f in root.iterfind("s:zip-file/s:pe-file", ns)}
        self.assertEqual({p: f.get("product-version") for p, f in files.items()}, want)
        self.assertEqual({f.get("product-name") for f in files.values()}, {versions.PRODUCT_NAME})
        self.assertTrue(all(f.find("s:authenticode-sign", ns) is not None for f in files.values()))
        params = {p.get("name") for p in root.iterfind("s:parameters/s:parameter", ns)}
        self.assertEqual(params, {v[2:-1] for v in want.values()})


if __name__ == "__main__":
    unittest.main()
