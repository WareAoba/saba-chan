"""package-linux.py 시험: python3 -m unittest discover -s .github/scripts -p 'test_*.py'"""

import importlib.util
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path

_path = Path(__file__).resolve().parent / "package-linux.py"
_spec = importlib.util.spec_from_file_location("package_linux", _path)
package_linux = importlib.util.module_from_spec(_spec)
sys.modules["package_linux"] = package_linux
_spec.loader.exec_module(package_linux)


class LinuxZips(unittest.TestCase):
    def test_binaries_are_executable_and_core_carries_the_runner(self):
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            for exe in ("saba-core", "saba-runner", "saba-chan-cli", "saba-chan-updater"):
                (d / exe).write_bytes(b"\x7fELF")
            made = package_linux.package(d, d / "out", "linux-x64")
            self.assertEqual(sorted(p.name for p in made), [
                "saba-chan-cli-linux-x64.zip", "saba-chan-updater-linux-x64.zip", "saba-core-linux-x64.zip"])
            with zipfile.ZipFile(d / "out" / "saba-core-linux-x64.zip") as zf:
                modes = {i.filename: (i.external_attr >> 16) & 0o777 for i in zf.infolist()}
        self.assertEqual(modes, {"saba-core": 0o755, "saba-runner": 0o755, "i18n.py": 0o644, "daemon_rcon.py": 0o644, "common_locales.json": 0o644})

    def test_a_missing_binary_stops_packaging(self):
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaises(SystemExit):
                package_linux.package(Path(d), Path(d) / "out", "linux-x64")


if __name__ == "__main__":
    unittest.main()
