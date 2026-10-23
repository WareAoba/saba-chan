import contextlib
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "modules"))

import saba_audit


class SdkAuditTests(unittest.TestCase):
    def scan(self, source, minimum=None):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "lifecycle.py").write_text(source, encoding="utf-8")
            if minimum is not None:
                (root / "module.toml").write_text(
                    f'[module.dependencies]\nsaba-core = "{minimum}"\n', encoding="utf-8",
                )
            return saba_audit.scan(root)

    def test_import_alias_reports_function_declared_and_required_version(self):
        findings = self.scan("from daemon_rcon import safe_join as join, daemon_post\n", ">=0.1.8")
        self.assertEqual(len(findings), 1, findings)
        self.assertEqual(findings[0]["permission"], "sdk_version")
        self.assertEqual(findings[0]["line"], 1)
        for text in ("daemon_rcon.safe_join", ">=0.1.8", "0.1.10"):
            self.assertIn(text, findings[0]["detail"])

    def test_module_alias_and_i18n_class_check_the_same_table(self):
        findings = self.scan("import daemon_rcon as sdk\nimport i18n as text\nsdk.loopback_http(1, 'GET', '/')\ntext.I18n('.')\n")
        self.assertEqual(len(findings), 2, findings)
        details = "\n".join(item["detail"] for item in findings)
        for text in ("daemon_rcon.loopback_http", "0.1.8", "i18n.I18n", "0.1.1", "missing"):
            self.assertIn(text, details)

    def test_matching_minimum_version_passes(self):
        self.assertEqual(self.scan("from daemon_rcon import safe_join, daemon_rest\nfrom i18n import I18n\n", ">=0.1.10"), [])

    def test_star_import_checks_names_that_are_used(self):
        findings = self.scan("from daemon_rcon import *\nsafe_join('.', 'world')\n", ">=0.1.8")
        self.assertEqual(len(findings), 1, findings)
        self.assertIn("daemon_rcon.safe_join", findings[0]["detail"])

    def test_importing_only_a_module_does_not_require_unused_helpers(self):
        self.assertEqual(self.scan("import daemon_rcon\nimport i18n\n"), [])

    def test_extension_dependency_is_not_reported_as_a_missing_module_manifest(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "hook.py").write_text("from daemon_rcon import daemon_post\n", encoding="utf-8")
            (root / "manifest.json").write_text(json.dumps({"dependencies": {"saba-core": ">=0.1.8"}}), encoding="utf-8")
            self.assertEqual(saba_audit.scan(root), [])

    def test_extension_legacy_minimum_also_satisfies_sdk_requirement(self):
        for dependencies in (["parent"], {"saba-core": ">=0.1.8"}):
            with self.subTest(dependencies=dependencies), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                (root / "hook.py").write_text("from daemon_rcon import safe_join\n", encoding="utf-8")
                manifest = {"min_app_version": "0.1.10", "dependencies": dependencies}
                (root / "manifest.json").write_text(json.dumps(manifest), encoding="utf-8")
                self.assertEqual(saba_audit.scan(root), [])

    def test_invalid_dependency_container_is_reported_without_crashing(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "lifecycle.py").write_text("from daemon_rcon import safe_join\n", encoding="utf-8")
            (root / "module.toml").write_text('[module]\ndependencies = "invalid"\n', encoding="utf-8")
            findings = saba_audit.scan(root)
            self.assertEqual(len(findings), 1, findings)
            self.assertEqual(findings[0]["permission"], "sdk_version")

    def test_embedded_sdk_table_is_read_without_executing_sdk_code(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "lifecycle.py").write_text("from daemon_rcon import safe_join\n", encoding="utf-8")
            source = "raise RuntimeError('SDK must not run')\nSINCE = {'safe_join': '0.1.10'}\n"
            findings = saba_audit.scan(root, sdk_source=source)
            self.assertEqual(len(findings), 1, findings)
            self.assertIn("0.1.10", findings[0]["detail"])

    def test_standalone_without_a_toml_parser_reports_failure_instead_of_sdk_findings(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "lifecycle.py").write_text("from daemon_rcon import safe_join\n", encoding="utf-8")
            (root / "module.toml").write_text('[module.dependencies]\nsaba-core = ">=0.1.10"\n', encoding="utf-8")
            output = io.StringIO()
            with patch.dict(sys.modules, {"tomllib": None}), patch.object(sys, "argv", ["saba_audit.py", directory]):
                with contextlib.redirect_stdout(output):
                    saba_audit.main()
            result = json.loads(output.getvalue())
            self.assertFalse(result["success"], result)
            self.assertNotIn("findings", result)
            self.assertIn("saba-core module audit", result["message"])


if __name__ == "__main__":
    unittest.main()
