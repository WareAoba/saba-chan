"""모듈 공용 도우미 (daemon_rcon.py 0.1.11, i18n.py 공용 문구) 시험: python -m unittest tests/test_shared_helpers.py"""
import hashlib
import io
import json
import os
import sys
import tarfile
import tempfile
import unittest
import zipfile
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "modules"))

import daemon_rcon
from i18n import I18n

MODULE_TOML = """
[module]
name = "g"
version = "0.1.0"

[config]
stop_timeout = 75

[[settings.fields]]
name = "max_players"
type = "number"
file_key = "max-players"

[[settings.fields]]
name = "pvp"
type = "boolean"
file_key = "pvp"

[[settings.fields]]
name = "motd"
type = "text"
file_key = "MOTD"

[[settings.fields]]
name = "ram"
type = "text"
group = "saba-chan"

[[diagnose]]
code = "PORT_IN_USE"
severity = "critical"
patterns = ["Address already in use", "FAILED TO BIND"]

[[diagnose]]
code = "broken"
patterns = ["("]

[[diagnose]]
code = "oom"
severity = "nonsense"
patterns = ["OutOfMemoryError"]
"""


class ModuleDirTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.dir = Path(self._tmp.name)
        (self.dir / "module.toml").write_text(MODULE_TOML, encoding="utf-8")
        (self.dir / "locales").mkdir()
        (self.dir / "locales" / "en.json").write_text(json.dumps({
            "messages": {"graceful_stop": "module wording"},
            "errors": {"detect": {"port_in_use": "port busy"}, "solutions": {"port_in_use": "change port"}},
        }), encoding="utf-8")
        self.i18n = I18n(str(self.dir))

    def tearDown(self):
        self._tmp.cleanup()


class SettingKeyTests(ModuleDirTest):
    def test_file_keys_come_from_module_toml_only(self):
        self.assertEqual(daemon_rcon.setting_file_keys(self.dir), {"max_players": "max-players", "pvp": "pvp", "motd": "MOTD"})

    def test_settings_to_file_formats_values_and_skips_unmapped(self):
        out = daemon_rcon.settings_to_file(self.dir, {"max_players": 20.0, "pvp": True, "ram": "4G", "motd": None})
        self.assertEqual(out, {"max-players": "20", "pvp": "true"})
        self.assertEqual(daemon_rcon.settings_to_file(self.dir, {"pvp": False}, true="1", false="0"), {"pvp": "0"})

    def test_a_line_break_in_a_value_is_refused(self):
        # 줄바꿈이 든 값은 파일에 다른 키를 만든다 (enable-rcon=true 같은)
        with self.assertRaises(ValueError):
            daemon_rcon.settings_to_file(self.dir, {"motd": "hi\nenable-rcon=true"})

    def test_settings_from_file_uses_field_types(self):
        values = {"max-players": "20", "pvp": "TRUE", "MOTD": " 000123 ", "unknown": "x"}
        self.assertEqual(daemon_rcon.settings_from_file(self.dir, values), {"max_players": 20, "pvp": True, "motd": " 000123 "})
        # 모르는 참·거짓 표기는 짐작하지 않는다
        self.assertEqual(daemon_rcon.settings_from_file(self.dir, {"pvp": "yes"}), {})
        self.assertEqual(daemon_rcon.settings_from_file(self.dir, {"pvp": "1"}, true="1", false="0"), {"pvp": True})
        self.assertEqual(daemon_rcon.settings_from_file(self.dir, {"motd": "a"}, case_insensitive=True), {"motd": "a"})


class KeyValueFileTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.path = Path(self._tmp.name) / "server.properties"

    def tearDown(self):
        self._tmp.cleanup()

    def test_update_keeps_comments_order_newlines_and_spacing(self):
        self.path.write_bytes(b"#comment\r\nb=2\r\na= keep \r\nb=3\r\n")
        daemon_rcon.update_key_values(self.path, {"b": "9", "c": "new"})
        self.assertEqual(self.path.read_bytes(), b"#comment\r\nb=9\r\na= keep \r\nc=new\r\n")
        self.assertEqual(daemon_rcon.read_key_values(self.path), {"b": "9", "a": " keep ", "c": "new"})
        self.assertEqual(daemon_rcon.read_key_values(self.path, strip_values=True)["a"], "keep")

    def test_none_deletes_and_empty_deletes_only_when_asked(self):
        self.path.write_text("a=1\nb=2\n", encoding="utf-8")
        daemon_rcon.update_key_values(self.path, {"a": None, "b": ""})
        self.assertEqual(self.path.read_text(encoding="utf-8"), "b=\n")
        daemon_rcon.update_key_values(self.path, {"b": ""}, delete_empty=True)
        self.assertEqual(self.path.read_text(encoding="utf-8"), "")

    def test_case_insensitive_keys_keep_the_file_spelling(self):
        self.path.write_text("WorldName=Old\nworldname=dup\n", encoding="utf-8")
        self.assertEqual(daemon_rcon.read_key_values(self.path, case_insensitive=True, first_wins=True), {"worldname": "Old"})
        daemon_rcon.update_key_values(self.path, {"worldname": "New"}, case_insensitive=True)
        self.assertEqual(self.path.read_text(encoding="utf-8"), "WorldName=New\n")

    def test_the_original_encoding_and_bom_are_kept(self):
        self.path.write_bytes("motd=서울\n".encode("cp949"))
        self.assertEqual(daemon_rcon.read_key_values(self.path), {"motd": "서울"})
        daemon_rcon.update_key_values(self.path, {"port": 1})
        self.assertEqual(self.path.read_bytes(), "motd=서울\nport=1\n".encode("cp949"))
        self.path.write_bytes(b"\xef\xbb\xbfa=1\n")
        daemon_rcon.update_key_values(self.path, {"a": "2"})
        self.assertEqual(self.path.read_bytes(), b"\xef\xbb\xbfa=2\n")

    def test_a_value_the_encoding_cannot_hold_switches_to_utf8(self):
        self.path.write_bytes(b"motd=caf\xe9\n")
        daemon_rcon.update_key_values(self.path, {"motd": "서울"}, encodings=("utf-8", "cp1252"))
        self.assertEqual(self.path.read_text(encoding="utf-8"), "motd=서울\n")

    def test_a_fixed_write_encoding_converts_the_file(self):
        self.path.write_bytes("motd=서울\n".encode("cp949"))
        daemon_rcon.update_key_values(self.path, {"port": 1}, write_encoding="utf-8")
        self.assertEqual(self.path.read_bytes(), "motd=서울\nport=1\n".encode("utf-8"))

    def test_missing_file_reads_empty_and_is_created(self):
        self.assertEqual(daemon_rcon.read_key_values(self.path), {})
        daemon_rcon.update_key_values(self.path, {"a": True})
        self.assertEqual(self.path.read_text(encoding="utf-8"), "a=true\n")

    def test_a_line_break_in_an_update_is_refused(self):
        with self.assertRaises(ValueError):
            daemon_rcon.update_key_values(self.path, {"a": "x\nenable-rcon=true"})
        self.assertFalse(self.path.exists())


class StopTests(ModuleDirTest):
    def test_stop_timeout_order_and_cap(self):
        cases = (({"stop_timeout": "45"}, 45), ({}, 75), ({"stop_timeout": 600}, 90),
                 ({"stop_timeout": 600, "stop_call_limit": 630}, 600), ({"stop_timeout": 600, "stop_call_limit": 100}, 70))
        for config, expected in cases:
            with self.subTest(config=config):
                self.assertEqual(daemon_rcon.stop_timeout(config, self.dir), expected)
        self.assertEqual(daemon_rcon.stop_timeout({}), 30)

    def _stop(self, config, asked=True, exited=True, killed=(True, None)):
        calls = []
        with patch.object(daemon_rcon, "wait_for_exit", side_effect=lambda pid, timeout: calls.append(("wait", timeout)) or exited), \
             patch.object(daemon_rcon, "force_kill", side_effect=lambda pid: calls.append(("kill", pid)) or killed):
            result = daemon_rcon.graceful_stop(config, lambda: calls.append(("ask",)) or asked, self.i18n, module_dir=self.dir)
        return result, calls

    def test_waits_after_asking_and_kills_only_on_timeout(self):
        result, calls = self._stop({"pid": 7, "stop_timeout": 10})
        self.assertEqual(calls, [("ask",), ("wait", 10)])
        self.assertEqual(result, {"success": True, "message": "module wording"})
        result, calls = self._stop({"pid": 7}, exited=False)
        self.assertEqual(calls, [("ask",), ("wait", 75), ("kill", 7)])
        self.assertTrue(result["success"])

    def test_without_a_way_to_ask_it_does_not_kill(self):
        result, calls = self._stop({"pid": 7}, asked=False)
        self.assertEqual(calls, [("ask",)])
        self.assertEqual(result["error_code"], "graceful_stop_unavailable")
        self.assertIn("save", result["message"])  # 공용 문구

    def test_managed_returns_after_asking_and_force_skips_asking(self):
        result, calls = self._stop({"pid": 7, "managed": True})
        self.assertEqual(calls, [("ask",)])
        self.assertTrue(result["success"])
        result, calls = self._stop({"pid": 7, "force": True}, killed=(False, "denied"))
        self.assertEqual(calls, [("kill", 7)])
        self.assertFalse(result["success"])
        self.assertIn("denied", result["message"])

    def test_no_pid_is_already_stopped(self):
        result, calls = self._stop({})
        self.assertEqual(calls, [])
        self.assertTrue(result["success"])

    def test_console_request_needs_an_instance(self):
        self.assertFalse(daemon_rcon.request_console({}, "stop"))
        with patch.object(daemon_rcon, "daemon_stdin_command", side_effect=ConnectionError("409")):
            self.assertFalse(daemon_rcon.request_console({"instance_id": "i"}, "stop"))
        with patch.object(daemon_rcon, "daemon_stdin_command") as send:
            self.assertTrue(daemon_rcon.request_console({"instance_id": "i"}, "stop"))
        send.assert_called_once_with("i", "stop", timeout=10)


class DiagnoseTests(ModuleDirTest):
    def test_rules_from_module_toml(self):
        rules = daemon_rcon.load_diagnose_rules(self.dir)
        self.assertEqual([(r["code"], r["severity"]) for r in rules], [("PORT_IN_USE", "critical"), ("oom", "error")])

    def test_first_matching_line_per_code(self):
        rules = daemon_rcon.load_diagnose_rules(self.dir)
        issues = daemon_rcon.diagnose_lines(["ok", "**** failed to bind to port", "Address already in use"], rules, self.i18n)
        self.assertEqual(issues, [{"code": "PORT_IN_USE", "severity": "critical", "matched_line": "**** failed to bind to port",
                                   "message": "port busy", "solution": "change port"}])

    def test_tail_lines(self):
        log = self.dir / "latest.log"
        log.write_text("".join(f"line {i}\n" for i in range(1000)), encoding="utf-8")
        self.assertEqual(daemon_rcon.tail_lines(log, 3), ["line 997", "line 998", "line 999"])
        self.assertEqual(daemon_rcon.tail_lines(self.dir / "none.log"), [])


class ExtractTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        self.dest = self.root / "out"

    def tearDown(self):
        self._tmp.cleanup()

    def _zip(self, entries):
        path = self.root / "a.zip"
        with zipfile.ZipFile(path, "w") as zf:
            for name, data in entries.items():
                zf.writestr(name, data)
        return path

    def test_an_entry_outside_writes_nothing(self):
        archive = self._zip({"jre/bin/java": "x", "jre/../../evil": "x"})
        with self.assertRaises(daemon_rcon.PathOutside):
            daemon_rcon.safe_extract(archive, self.dest, strip_top=True)
        self.assertFalse((self.root / "evil").exists())
        self.assertFalse((self.dest / "bin" / "java").exists())

    def test_strip_top_prefix_and_keep_existing(self):
        archive = self._zip({"jdk-21/bin/java": "x", "jdk-21/release": "r"})
        daemon_rcon.safe_extract(archive, self.dest, strip_top=True)
        self.assertEqual((self.dest / "release").read_text(), "r")
        archive = self._zip({"1449/Windows/a.exe": "new", "1449/Windows/serverconfig.txt": "new", "1449/Linux/b": "x"})
        (self.dest / "serverconfig.txt").write_text("mine")
        written = daemon_rcon.safe_extract(archive, self.dest, prefix="1449/Windows", keep_existing={"serverconfig.txt"})
        self.assertEqual([Path(p).name for p in written], ["a.exe"])
        self.assertEqual((self.dest / "serverconfig.txt").read_text(), "mine")
        self.assertFalse((self.dest / "b").exists())

    def test_tar_links_are_not_extracted(self):
        path = self.root / "a.tar.gz"
        with tarfile.open(path, "w:gz") as tf:
            info = tarfile.TarInfo("jre/bin/java")
            data = b"#!/bin/sh\n"
            info.size, info.mode = len(data), 0o755
            tf.addfile(info, io.BytesIO(data))
            link = tarfile.TarInfo("jre/link")
            link.type, link.linkname = tarfile.SYMTYPE, "/etc/passwd"
            tf.addfile(link)
        daemon_rcon.safe_extract(path, self.dest, strip_top=True)
        self.assertTrue((self.dest / "bin" / "java").is_file())
        self.assertFalse(os.path.lexists(self.dest / "link"))
        if os.name != "nt":
            self.assertTrue(os.access(self.dest / "bin" / "java", os.X_OK))

    def test_a_bad_crc_is_refused_before_writing(self):
        archive = self._zip({"a/x": "X" * 64})
        data = bytearray(archive.read_bytes())
        data[data.index(b"X" * 64)] = ord("Y")
        archive.write_bytes(bytes(data))
        with self.assertRaises(zipfile.BadZipFile):
            daemon_rcon.safe_extract(archive, self.dest)
        self.assertFalse(self.dest.exists())


class DownloadTests(unittest.TestCase):
    def test_checksums_and_progress(self):
        with tempfile.TemporaryDirectory() as d:
            src, dest = Path(d) / "src.bin", Path(d) / "dest.bin"
            src.write_bytes(b"abc" * 1000)
            seen = []
            sha256 = hashlib.sha256(b"abc" * 1000).hexdigest()
            digests = daemon_rcon.download(src.as_uri(), dest, on_progress=lambda done, total: seen.append((done, total)),
                                           checksums={"sha256": sha256.upper()})
            self.assertEqual(digests["sha256"], sha256)
            self.assertEqual(seen[-1], (3000, 3000))
            with self.assertRaises(daemon_rcon.ChecksumMismatch):
                daemon_rcon.download(src.as_uri(), dest, checksums={"sha1": "00"})
            self.assertFalse(dest.exists())


class ProgressAndLocaleTests(ModuleDirTest):
    def test_progress_line(self):
        with patch("sys.stderr", new_callable=io.StringIO) as err:
            daemon_rcon.progress(5, "받는 중")
        self.assertEqual(err.getvalue(), 'PROGRESS:{"percent": 5, "message": "\\ubc1b\\ub294 \\uc911"}\n')

    def test_common_phrases_fill_keys_the_module_does_not_define(self):
        self.assertEqual(self.i18n.t("messages.graceful_stop"), "module wording")
        self.assertEqual(self.i18n.t("messages.force_killed", pid=7), "Server process 7 was force stopped")
        self.assertEqual(self.i18n.t("errors.nothing_like_this"), "errors.nothing_like_this")
        with patch.dict(os.environ, {"SABA_LANG": "ko"}):
            self.assertEqual(I18n(str(self.dir)).t("messages.force_killed", pid=7), "서버 프로세스 7를 강제로 끝냈습니다")

    def test_every_language_has_the_same_common_keys(self):
        common = json.loads((Path(daemon_rcon.__file__).parent / "common_locales.json").read_text(encoding="utf-8"))
        keys = {lang: sorted(f"{s}.{k}" for s, d in table.items() for k in d) for lang, table in common.items()}
        self.assertEqual(len(keys), 10)
        for lang, names in keys.items():
            self.assertEqual(names, keys["en"], lang)

    def test_new_helpers_are_listed_with_their_version(self):
        for name in ("graceful_stop", "update_key_values", "settings_to_file", "diagnose_lines", "safe_extract", "download", "progress"):
            self.assertEqual(daemon_rcon.SINCE[name], "0.1.11-0", name)


if __name__ == "__main__":
    unittest.main()
