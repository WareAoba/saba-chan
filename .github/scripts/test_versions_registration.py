"""versions.py registration 시험: python3 -m unittest discover -s .github/scripts -p 'test_*.py'"""

import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path

_path = Path(__file__).resolve().parents[2] / "scripts" / "versions.py"
_spec = importlib.util.spec_from_file_location("versions", _path)
versions = importlib.util.module_from_spec(_spec)
sys.modules["versions"] = versions
_spec.loader.exec_module(versions)


class RegistrationNotes(unittest.TestCase):
    def register(self, channel, notes):
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            signed = d / "signed.json"
            signed.write_text(json.dumps({"manifest": {
                "version": "9.9.9", "channel": channel, "platform": "windows-x64", "components": {}}}))
            notes_path = d / "notes.json"
            if notes is not None:
                notes_path.write_text(json.dumps(notes))
            out = d / "out.json"
            args = versions.argparse.Namespace(
                signed=str(signed), rollout=100, critical=False, notes=str(notes_path), out=str(out))
            versions.cmd_registration(args)
            return json.loads(out.read_text())

    def test_stable_release_without_notes_is_refused(self):
        # 노트 없이 등록하면 받은 클라이언트의 패치노트가 빈 채로 남는다
        with self.assertRaises(SystemExit):
            self.register("stable", None)

    def test_stable_release_carries_its_notes(self):
        self.assertEqual(self.register("stable", {"ko": "고침"})["notes"], {"ko": "고침"})

    def test_beta_release_may_go_without_notes(self):
        self.assertIsNone(self.register("beta", None)["notes"])


if __name__ == "__main__":
    unittest.main()
