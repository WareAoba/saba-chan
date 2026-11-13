"""모듈 공용 도우미 (daemon_rcon.py 0.1.11, i18n.py 공용 문구) 시험: python -m unittest tests/test_shared_helpers.py"""
import hashlib
import io
import json
import os
import sys
import tarfile
import tempfile
import time
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
stop_command = "quit"

[[settings.fields]]
name = "max_players"
type = "number"
file_key = "max-players"
default = 10.0

[[settings.fields]]
name = "pvp"
type = "boolean"
file_key = "pvp"
default = true

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

[launch]
diagnose_files = ["logs/none.log", "logs/latest.log"]

[remote_access]
forced = { "rest-enabled" = "true" }

[remote_access.rcon]
on = { "enable-rcon" = "true", "broadcast-rcon-to-ops" = "false" }
off = { "enable-rcon" = "false" }
port = "rcon.port"
password = "rcon.password"
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

    def test_defaults_in_file_form_come_from_module_toml(self):
        # 새 서버의 설정 파일 값: 기본값이 있는 file_key 칸만, 파일 표기로
        self.assertEqual(daemon_rcon.setting_defaults_to_file(self.dir), {"max-players": "10", "pvp": "true"})
        self.assertEqual(daemon_rcon.setting_defaults_to_file(self.dir, true="True"), {"max-players": "10", "pvp": "True"})

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

    def test_a_file_no_encoding_can_read_is_not_overwritten(self):
        self.path.write_bytes(b"motd=\x81\n")
        with self.assertRaises(UnicodeDecodeError):
            daemon_rcon.read_key_values(self.path, encodings=("utf-8", "cp1252"))
        with self.assertRaises(UnicodeDecodeError):
            daemon_rcon.update_key_values(self.path, {"a": "1"}, encodings=("utf-8", "cp1252"))
        self.assertEqual(self.path.read_bytes(), b"motd=\x81\n")

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

    def test_the_stop_command_comes_from_module_toml(self):
        # 데몬과 같은 값: [config] stop_command, 없으면 stop
        self.assertEqual(daemon_rcon.stop_command(self.dir), "quit")
        with tempfile.TemporaryDirectory() as empty:
            self.assertEqual(daemon_rcon.stop_command(empty), "stop")

    def test_console_request_needs_an_instance(self):
        self.assertFalse(daemon_rcon.request_console({}, "stop"))
        with patch.object(daemon_rcon, "daemon_stdin_command", side_effect=ConnectionError("409")):
            self.assertFalse(daemon_rcon.request_console({"instance_id": "i"}, "stop"))
        with patch.object(daemon_rcon, "daemon_stdin_command") as send:
            self.assertTrue(daemon_rcon.request_console({"instance_id": "i"}, "stop"))
        send.assert_called_once_with("i", "stop", timeout=10)


class DeleteTests(unittest.TestCase):
    def test_deletes_files_folders_and_links_and_skips_missing(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "world").mkdir()
            (root / "world" / "level.dat").write_text("x")
            (root / "ops.json").write_text("[]")
            (root / "kept").mkdir()
            (root / "kept" / "keep.txt").write_text("k")
            try:
                os.symlink(root / "kept", root / "link", target_is_directory=True)
                link = ["link"]
            except OSError:
                link = []
            deleted, errors = daemon_rcon.delete_paths(["world", "ops.json", "missing"] + link, root=root)
            self.assertEqual((deleted, errors), (["world", "ops.json"] + link, []))
            # 링크는 링크만 지운다
            self.assertTrue((root / "kept" / "keep.txt").is_file())

    def test_an_unreadable_path_is_an_error_not_missing(self):
        with tempfile.TemporaryDirectory() as tmp:
            with patch.object(daemon_rcon.os, "lstat", side_effect=PermissionError("denied")):
                deleted, errors = daemon_rcon.delete_paths([os.path.join(tmp, "Saves")])
            self.assertEqual(deleted, [])
            self.assertEqual(len(errors), 1)
            self.assertIn("denied", errors[0])


class ChatTests(ModuleDirTest):
    def test_chat_reply(self):
        self.assertEqual(daemon_rcon.chat_reply(lambda: {"ok": 1}, self.i18n), {"success": True})
        self.assertEqual(daemon_rcon.chat_reply(lambda: None, self.i18n), {"success": True})
        self.assertEqual(daemon_rcon.chat_reply(lambda: {"success": False, "error": "busy"}, self.i18n),
                         {"success": False, "message": "Game chat send failed: busy"})

        def down():
            raise ConnectionError("no daemon")
        self.assertEqual(daemon_rcon.chat_reply(down, self.i18n), {"success": False, "message": "Game chat send failed: no daemon"})

        def outside():
            raise daemon_rcon.PathOutside("x")
        with self.assertRaises(daemon_rcon.PathOutside):
            daemon_rcon.chat_reply(outside, self.i18n)


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


    def test_diagnose_log_reads_console_lines_or_the_declared_files(self):
        """모듈 `diagnose_log()`의 공용 절차 (0.1.14-beta.2): 데몬이 준 줄이 먼저, 없으면 `[launch] diagnose_files` 중 처음 읽히는 파일의 끝"""
        given = daemon_rcon.diagnose_log({"log_lines": "ok\nFAILED TO BIND"}, self.dir, self.i18n)
        self.assertEqual((given["success"], given["lines_analyzed"], [i["code"] for i in given["issues"]]), (True, 2, ["PORT_IN_USE"]))
        work = self.dir / "server"
        (work / "logs").mkdir(parents=True)
        (work / "logs" / "latest.log").write_text("boot\njava.lang.OutOfMemoryError\n", encoding="utf-8")
        read = daemon_rcon.diagnose_log({"log_lines": [], "working_dir": str(work)}, self.dir, self.i18n)
        self.assertEqual((read["lines_analyzed"], [i["code"] for i in read["issues"]]), (2, ["oom"]))
        # 모듈이 계산한 파일(files)이 선언보다 먼저다. 읽을 줄이 없으면 문제도 없다
        other = self.dir / "other.txt"
        other.write_text("Address already in use\n", encoding="utf-8")
        chosen = daemon_rcon.diagnose_log({"working_dir": str(work)}, self.dir, self.i18n, files=[other])
        self.assertEqual([i["code"] for i in chosen["issues"]], ["PORT_IN_USE"])
        self.assertEqual(daemon_rcon.diagnose_log({}, self.dir, self.i18n), {"success": True, "issues": [], "lines_analyzed": 0})


class RemoteAccessTests(ModuleDirTest):
    """module.toml `[remote_access]`: 사바쨩이 정하는 원격 접속 값 (0.1.14-beta.2)"""
    ON = {"managed": False, "rcon_port": 25580, "rcon_password": "pw"}

    def test_rcon_is_on_only_for_an_unmanaged_server_with_daemon_credentials(self):
        self.assertEqual(daemon_rcon.remote_access_values(self.dir, self.ON),
                         {"rest-enabled": "true", "enable-rcon": "true", "broadcast-rcon-to-ops": "false",
                          "rcon.port": "25580", "rcon.password": "pw"})
        off = {"rest-enabled": "true", "enable-rcon": "false"}
        for config in (None, {}, {"managed": True, "rcon_port": 1, "rcon_password": "pw"}, {"managed": False, "rcon_port": 25580}):
            self.assertEqual(daemon_rcon.remote_access_values(self.dir, config), off, config)

    def test_a_module_without_the_table_forces_nothing(self):
        (self.dir / "module.toml").write_text('[module]\nname = "g"\n', encoding="utf-8")
        self.assertEqual(daemon_rcon.remote_access_values(self.dir, self.ON), {})

    def test_enforce_writes_only_the_values_that_differ(self):
        written = []
        current = {"rest-enabled": "true", "enable-rcon": "true", "motd": "hi"}
        self.assertEqual(daemon_rcon.enforce_remote_access(self.dir, {}, current, written.append), {"enable-rcon": "false"})
        self.assertEqual(written, [{"enable-rcon": "false"}])
        self.assertEqual(daemon_rcon.enforce_remote_access(self.dir, {}, {"rest-enabled": "true", "enable-rcon": "false"}, written.append), {})
        self.assertEqual(len(written), 1)
        # 빈 값으로 맞출 키가 파일에 없으면 이미 빈 것이다: 새로 적지 않는다
        (self.dir / "module.toml").write_text(
            '[module]\nname = "g"\n\n[remote_access.rcon]\nport = "RCONPort"\npassword = "RCONPassword"\noff = { RCONPassword = "" }\n',
            encoding="utf-8")
        self.assertEqual(daemon_rcon.enforce_remote_access(self.dir, {}, {}, written.append), {})
        self.assertEqual(daemon_rcon.enforce_remote_access(self.dir, {}, {"RCONPassword": "old"}, written.append), {"RCONPassword": ""})


class JavaDefaultsTests(unittest.TestCase):
    def test_auto_heap_follows_the_host_memory(self):
        gib = 1024 ** 3
        for host, heap in ((None, 8192), ("junk", 8192), (32 * gib, 8192), (8 * gib, 4096), (4 * gib, 2560), (2 * gib, 2048)):
            env = {} if host is None else {"SABA_HOST_MEMORY": str(host)}
            with patch.dict(os.environ, env, clear=False):
                if host is None:
                    os.environ.pop("SABA_HOST_MEMORY", None)
                self.assertEqual(daemon_rcon.auto_heap_mb(), heap, host)

    def test_java_release_major(self):
        with tempfile.TemporaryDirectory() as tmp:
            self.assertEqual(daemon_rcon.java_release_major(tmp), 0)
            for text, major in (('IMPLEMENTOR="x"\nJAVA_VERSION="25.0.1"\n', 25), ('JAVA_VERSION="1.8.0_462"\n', 8),
                                ('JAVA_VERSION="21"\n', 21), ('JAVA_RUNTIME_VERSION="17.0.2"\n', 0)):
                Path(tmp, "release").write_text(text, encoding="utf-8")
                self.assertEqual(daemon_rcon.java_release_major(tmp), major, text)


class SmallHelperTests(ModuleDirTest):
    def test_extension_required_names_the_missing_extensions(self):
        reply = daemon_rcon.extension_required(self.i18n, "steamcmd", "ue4-ini")
        self.assertEqual(reply, {"success": False, "error_code": "extension_required", "missing_extensions": ["steamcmd", "ue4-ini"],
                                 "message": "steamcmd, ue4-ini extension required. Install and enable it in Settings → Extensions."})

    def test_list_dir(self):
        (self.dir / "b").mkdir()
        self.assertEqual(daemon_rcon.list_dir(self.dir), ["b", "locales", "module.toml"])
        self.assertEqual(daemon_rcon.list_dir(self.dir / "none"), [])
        with self.assertRaises(OSError):
            daemon_rcon.list_dir(self.dir / "module.toml")

    def test_extension_data_dir(self):
        with patch.dict(os.environ, {"SABA_EXTENSION_DATA_ROOT": str(self.dir)}):
            self.assertEqual(daemon_rcon.extension_data_dir("steamcmd"), self.dir / "steamcmd")
            with self.assertRaises(daemon_rcon.PathOutside):
                daemon_rcon.extension_data_dir("../x")
        with patch.dict(os.environ, {"SABA_EXTENSION_DATA_ROOT": ""}), self.assertRaises(EnvironmentError):
            daemon_rcon.extension_data_dir("steamcmd")

    def test_fetch_json(self):
        import http.server
        import threading

        class Handler(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                body = json.dumps({"agent": self.headers.get("User-Agent")}).encode()
                self.send_response(200 if self.path == "/ok" else 404)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, *args):
                pass

        server = http.server.HTTPServer(("127.0.0.1", 0), Handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        try:
            base = f"http://127.0.0.1:{server.server_port}"
            self.assertEqual(daemon_rcon.fetch_json(base + "/ok"), {"agent": "saba-chan"})
            import urllib.error
            with self.assertRaises(urllib.error.HTTPError):
                daemon_rcon.fetch_json(base + "/missing")
        finally:
            server.shutdown()
            server.server_close()


@unittest.skipIf(os.name == "nt", "POSIX process tree")
class ProcessTreeTests(unittest.TestCase):
    def test_descendants_are_found_without_pgrep(self):
        """자식·손자를 /proc(없으면 ps)에서 찾는다: pgrep이 없는 PC에서도 서버가 띄운 프로세스를 함께 끝낸다. 맨 아래부터"""
        import subprocess
        code = ("import subprocess, sys, time; "
                "c = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)']); print(c.pid, flush=True); time.sleep(60)")
        parent = subprocess.Popen([sys.executable, "-c", code], stdout=subprocess.PIPE, text=True)
        try:
            child = int(parent.stdout.readline())
            # /proc이 있으면 바깥 프로그램 없이 찾는다
            with patch.dict(os.environ, {"PATH": ""} if os.path.isdir("/proc") else {}):
                self.assertEqual(daemon_rcon._descendants(os.getpid())[-1], parent.pid)
                self.assertEqual(daemon_rcon._descendants(parent.pid), [child])
                ok, error = daemon_rcon.force_kill(parent.pid)
            self.assertEqual((ok, error), (True, None))
            parent.wait(timeout=10)
            deadline = time.monotonic() + 10
            while daemon_rcon.is_process_alive(child) and time.monotonic() < deadline:
                time.sleep(0.05)
            self.assertFalse(daemon_rcon.is_process_alive(child))
        finally:
            parent.kill()
            parent.stdout.close()


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

    def test_keep_existing_matches_dot_slash_entries(self):
        path = self.root / "a.tar"
        with tarfile.open(path, "w") as tf:
            info = tarfile.TarInfo("./serverconfig.txt")
            info.size = 7
            tf.addfile(info, io.BytesIO(b"default"))
        self.dest.mkdir()
        (self.dest / "serverconfig.txt").write_text("mine")
        daemon_rcon.safe_extract(path, self.dest, keep_existing={"serverconfig.txt"})
        self.assertEqual((self.dest / "serverconfig.txt").read_text(), "mine")

    def test_a_dot_slash_hard_link_still_finds_its_file(self):
        path = self.root / "a.tar"
        with tarfile.open(path, "w") as tf:
            info = tarfile.TarInfo("./lib/real.so")
            info.size = 3
            tf.addfile(info, io.BytesIO(b"lib"))
            hard = tarfile.TarInfo("./lib/copy.so")
            hard.type, hard.linkname = tarfile.LNKTYPE, "./lib/real.so"
            tf.addfile(hard)
        daemon_rcon.safe_extract(path, self.dest)
        self.assertEqual((self.dest / "lib" / "copy.so").read_bytes(), b"lib")

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

    @unittest.skipIf(os.name == "nt", "symlinks need POSIX")
    def test_links_that_stay_inside_are_kept(self):
        # Adoptium JRE tar에는 lib/... -> ../... 같은 상대 링크와 하드 링크가 있다
        path = self.root / "a.tar.gz"
        with tarfile.open(path, "w:gz") as tf:
            info = tarfile.TarInfo("jre/lib/libjsig.so")
            info.size = 3
            tf.addfile(info, io.BytesIO(b"lib"))
            link = tarfile.TarInfo("jre/lib/server/libjsig.so")
            link.type, link.linkname = tarfile.SYMTYPE, "../libjsig.so"
            tf.addfile(link)
            hard = tarfile.TarInfo("jre/lib/copy.so")
            hard.type, hard.linkname = tarfile.LNKTYPE, "jre/lib/libjsig.so"
            tf.addfile(hard)
            out = tarfile.TarInfo("jre/lib/out")
            out.type, out.linkname = tarfile.SYMTYPE, "../../../outside"
            tf.addfile(out)
        daemon_rcon.safe_extract(path, self.dest, strip_top=True)
        self.assertEqual(os.readlink(self.dest / "lib" / "server" / "libjsig.so"), "../libjsig.so")
        self.assertEqual((self.dest / "lib" / "server" / "libjsig.so").read_bytes(), b"lib")
        self.assertEqual((self.dest / "lib" / "copy.so").read_bytes(), b"lib")
        self.assertFalse(os.path.lexists(self.dest / "lib" / "out"))

    @unittest.skipIf(os.name == "nt", "symlinks need POSIX")
    def test_a_zip_symlink_is_not_written_as_a_file(self):
        path = self.root / "a.zip"
        with zipfile.ZipFile(path, "w") as zf:
            zf.writestr("a/real", "x")
            link = zipfile.ZipInfo("a/link")
            link.external_attr = (0o120777 << 16)
            zf.writestr(link, "real")
            out = zipfile.ZipInfo("a/out")
            out.external_attr = (0o120777 << 16)
            zf.writestr(out, "/etc/passwd")
        daemon_rcon.safe_extract(path, self.dest)
        self.assertTrue(os.path.islink(self.dest / "a" / "link"))
        self.assertEqual((self.dest / "a" / "link").read_text(), "x")
        self.assertFalse(os.path.lexists(self.dest / "a" / "out"))

    @unittest.skipIf(os.name == "nt", "symlinks need POSIX")
    def test_chained_links_cannot_reach_outside(self):
        path = self.root / "a.tar"
        with tarfile.open(path, "w") as tf:
            here = tarfile.TarInfo("here")
            here.type, here.linkname = tarfile.SYMTYPE, "."
            tf.addfile(here)
            up = tarfile.TarInfo("sub/up")
            up.type, up.linkname = tarfile.SYMTYPE, "../here/.."
            tf.addfile(up)
        daemon_rcon.safe_extract(path, self.dest)
        self.assertTrue(os.path.islink(self.dest / "here"))
        self.assertFalse(os.path.lexists(self.dest / "sub" / "up"))

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
                daemon_rcon.download(src.as_uri(), Path(d) / "other.bin", checksums={"sha1": "00"})
            self.assertEqual(sorted(p.name for p in Path(d).iterdir()), ["dest.bin", "src.bin"])

    def test_a_failed_download_keeps_the_existing_file(self):
        with tempfile.TemporaryDirectory() as d:
            dest = Path(d) / "server.jar"
            dest.write_bytes(b"working")
            with self.assertRaises(OSError):
                daemon_rcon.download((Path(d) / "missing").as_uri(), dest)
            self.assertEqual(dest.read_bytes(), b"working")

    def test_a_slow_trickle_still_hits_the_overall_deadline(self):
        # 읽기 한 번이 1MiB를 기다리면 전체 한도를 넘겨도 끊지 못한다
        class Trickle:
            headers = {"Content-Length": "100"}
            sent = 0

            def __enter__(self):
                return self

            def __exit__(self, *exc):
                return False

            def read1(self, n):
                self.sent += 1
                return b"x" if self.sent <= 100 else b""

            def read(self, n):
                raise AssertionError("read() waits for the full chunk")

        clock = iter(range(0, 10_000))
        with tempfile.TemporaryDirectory() as d, \
                patch("urllib.request.urlopen", return_value=Trickle()), \
                patch.object(daemon_rcon.time, "monotonic", lambda: next(clock)):
            with self.assertRaises(TimeoutError):
                daemon_rcon.download("https://example.invalid/x", Path(d) / "dest.bin", timeout=1)

    def test_a_connection_that_ends_early_is_a_failure(self):
        class Short(io.BytesIO):
            headers = {"Content-Length": "10"}

            def __enter__(self):
                return self

            def __exit__(self, *exc):
                return False

        with tempfile.TemporaryDirectory() as d, \
                patch("urllib.request.urlopen", return_value=Short(b"abc")):
            dest = Path(d) / "dest.bin"
            with self.assertRaises(IOError):
                daemon_rcon.download("https://example.invalid/x", dest)
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
        for name in ("rcon_access", "remote_access_values", "enforce_remote_access", "auto_heap_mb", "java_release_major",
                     "diagnose_log", "extension_required", "list_dir", "fetch_json", "extension_data_dir"):
            self.assertEqual(daemon_rcon.SINCE[name], "0.1.14-beta.2", name)


class ImportCostTests(unittest.TestCase):
    def test_importing_the_helpers_does_not_load_the_http_client(self):
        """lifecycle 호출마다 새 프로세스라, 받기(download)에만 쓰는 urllib.request를 미리 읽지 않는다"""
        import subprocess
        code = "import sys, daemon_rcon; print('urllib.request' in sys.modules)"
        out = subprocess.run([sys.executable, "-c", code], cwd=os.path.dirname(daemon_rcon.__file__),
                             capture_output=True, text=True, check=True)
        self.assertEqual(out.stdout.strip(), "False")


if __name__ == "__main__":
    unittest.main()
