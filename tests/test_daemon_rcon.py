import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
from urllib.error import HTTPError

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "modules"))
import daemon_rcon


class DaemonRconTests(unittest.TestCase):
    def temporary_directory(self):
        root = Path(__file__).resolve().parents[1] / ".tmp"
        root.mkdir(exist_ok=True)
        return tempfile.TemporaryDirectory(prefix="recovery-core-", dir=root)

    def response(self):
        response = unittest.mock.MagicMock()
        response.__enter__.return_value.read.return_value = json.dumps({"data": "ok"}).encode()
        return response

    def test_request_uses_token_override_and_observes_rotation(self):
        with self.temporary_directory() as directory:
            token_path = Path(directory) / "token"
            with patch.dict(daemon_rcon.os.environ, {"SABA_TOKEN_PATH": str(token_path)}), patch.object(
                daemon_rcon.urllib.request, "urlopen", return_value=self.response()
            ) as request:
                for token in ["fake-first-token", "fake-rotated-token"]:
                    token_path.write_text(f" {token}\n", encoding="utf-8")
                    self.assertEqual(daemon_rcon.daemon_rcon_command("instance", "list"), "ok")
                    self.assertEqual(request.call_args.args[0].get_header("X-saba-token"), token)

    def test_request_uses_data_directory_token(self):
        with self.temporary_directory() as directory:
            (Path(directory) / ".ipc_token").write_text("fake-data-token", encoding="utf-8")
            with patch.dict(daemon_rcon.os.environ, {"SABA_TOKEN_PATH": "", "SABA_DATA_DIR": directory}), patch.object(
                daemon_rcon.urllib.request, "urlopen", return_value=self.response()
            ) as request:
                daemon_rcon.daemon_rcon_command("instance", "list")
                self.assertEqual(request.call_args.args[0].get_header("X-saba-token"), "fake-data-token")

    def test_missing_token_keeps_request_usable_for_auth_disabled_daemon(self):
        with self.temporary_directory() as directory:
            with patch.dict(daemon_rcon.os.environ, {"SABA_TOKEN_PATH": str(Path(directory) / "missing")}), patch.object(
                daemon_rcon.urllib.request, "urlopen", return_value=self.response()
            ) as request:
                self.assertEqual(daemon_rcon.daemon_rcon_command("instance", "list"), "ok")
                self.assertIsNone(request.call_args.args[0].get_header("X-saba-token"))

    def test_http_error_preserves_status_and_response_body(self):
        failure = HTTPError("http://127.0.0.1/test", 401, "Unauthorized", {}, io.BytesIO(b'authentication required'))
        with patch.dict(daemon_rcon.os.environ, {"SABA_TOKEN_PATH": "missing-test-token"}), patch.object(
            daemon_rcon.urllib.request, "urlopen", side_effect=failure
        ):
            with self.assertRaisesRegex(ConnectionError, r"HTTP 401.*authentication required"):
                daemon_rcon.daemon_rcon_command("instance", "list")

    def response_with(self, body):
        response = unittest.mock.MagicMock()
        response.__enter__.return_value.read.return_value = json.dumps(body).encode()
        return response

    def test_rcon_command_returns_daemon_response_text(self):
        # 데몬 /rcon 응답은 data:{response, executed} 형태다.
        body = {"success": True, "data": {"response": "There are 0 players", "executed": "list"}}
        with patch.dict(daemon_rcon.os.environ, {"SABA_TOKEN_PATH": "missing-test-token"}), patch.object(
            daemon_rcon.urllib.request, "urlopen", return_value=self.response_with(body)
        ):
            self.assertEqual(daemon_rcon.daemon_rcon_command("instance", "list"), "There are 0 players")

    def test_daemon_request_sends_token_and_parses_json(self):
        with self.temporary_directory() as directory:
            token_path = Path(directory) / "token"
            token_path.write_text("fake-request-token", encoding="utf-8")
            env = {"SABA_TOKEN_PATH": str(token_path), "DAEMON_API_URL": "http://127.0.0.1:1"}
            with patch.dict(daemon_rcon.os.environ, env), patch.object(
                daemon_rcon.urllib.request, "urlopen", return_value=self.response_with({"success": True})
            ) as request:
                self.assertEqual(daemon_rcon.daemon_request("/api/x", {"a": 1}), {"success": True})
                sent = request.call_args.args[0]
                self.assertEqual(sent.get_header("X-saba-token"), "fake-request-token")
                self.assertEqual(sent.full_url, "http://127.0.0.1:1/api/x")
                self.assertEqual(json.loads(sent.data), {"a": 1})

    def test_stdin_command_uses_command_key(self):
        with patch.dict(daemon_rcon.os.environ, {"SABA_TOKEN_PATH": "missing-test-token"}), patch.object(
            daemon_rcon.urllib.request, "urlopen", return_value=self.response_with({"success": True})
        ) as request:
            daemon_rcon.daemon_stdin_command("abc", "save")
            sent = request.call_args.args[0]
            self.assertTrue(sent.full_url.endswith("/api/instance/abc/stdin"))
            self.assertEqual(json.loads(sent.data), {"command": "save"})

    def test_daemon_error_field_raises(self):
        with patch.dict(daemon_rcon.os.environ, {"SABA_TOKEN_PATH": "missing-test-token"}), patch.object(
            daemon_rcon.urllib.request, "urlopen", return_value=self.response_with({"error": "boom"})
        ):
            with self.assertRaisesRegex(ConnectionError, "boom"):
                daemon_rcon.daemon_rcon_command("instance", "list")

    @unittest.skipUnless(daemon_rcon.os.name == "nt", "Windows 전용 생존 확인")
    def test_process_alive_never_signals_on_windows(self):
        # Windows에서 os.kill(pid, 0)은 프로세스를 종료시킨다.
        with patch.object(daemon_rcon.os, "kill", side_effect=AssertionError("os.kill must not be called")):
            self.assertTrue(daemon_rcon.is_process_alive(daemon_rcon.os.getpid()))
            self.assertFalse(daemon_rcon.is_process_alive(0))

    def test_process_alive_false_for_missing_pid(self):
        self.assertFalse(daemon_rcon.is_process_alive(None))
        self.assertFalse(daemon_rcon.is_process_alive(2 ** 22 + 12345))

    def test_config_value_reads_nested_settings(self):
        config = {"module_settings": {"server_name": "alpha"}, "settings": {"ram": 2}}
        self.assertEqual(daemon_rcon.config_value(config, "server_name", "servertest"), "alpha")
        self.assertEqual(daemon_rcon.config_value(config, "ram", 4), 2)
        self.assertEqual(daemon_rcon.config_value({"server_name": "top", **config}, "server_name"), "top")
        self.assertEqual(daemon_rcon.config_value({"server_name": ""}, "server_name", "servertest"), "servertest")

    def test_coerce_setting_keeps_strings_unless_typed(self):
        self.assertEqual(daemon_rcon.coerce_setting("-4172144997902289642", "text"), "-4172144997902289642")
        self.assertEqual(daemon_rcon.coerce_setting("12345", None), "12345")
        self.assertEqual(daemon_rcon.coerce_setting("007", "password"), "007")
        self.assertEqual(daemon_rcon.coerce_setting("20", "number"), 20)
        self.assertEqual(daemon_rcon.coerce_setting("1.500000", "number"), 1.5)
        self.assertEqual(daemon_rcon.coerce_setting("True", "boolean"), True)
        self.assertEqual(daemon_rcon.coerce_setting("false", "boolean"), False)
        self.assertEqual(daemon_rcon.coerce_setting("abc", "number"), "abc")

    def test_load_setting_types_reads_module_toml(self):
        with self.temporary_directory() as directory:
            (Path(directory) / "module.toml").write_bytes(
                b'\xef\xbb\xbf[module]\nname="x"\n[[settings.fields]]\nname="ram"\ntype="number"\n'
                b'[[settings.fields]]\nname="motd"\n'
            )
            self.assertEqual(daemon_rcon.load_setting_types(directory), {"ram": "number", "motd": "text"})
            self.assertEqual(daemon_rcon.load_setting_types(Path(directory) / "missing"), {})

    def run_cli(self, functions, argv, stdin):
        out = io.StringIO()
        with patch.object(daemon_rcon.sys, "argv", ["lifecycle.py", *argv]), patch.object(
            daemon_rcon.sys, "stdin", io.StringIO(stdin)
        ), patch.object(daemon_rcon.sys, "stdout", out):
            daemon_rcon.run_lifecycle(functions)
        return json.loads(out.getvalue())

    def test_run_lifecycle_dispatches_and_reports_unknown(self):
        echo = {"echo": lambda config: {"success": True, "got": config}}
        self.assertEqual(self.run_cli(echo, ["echo"], '{"a": 1}'), {"success": True, "got": {"a": 1}})

        unknown = self.run_cli(echo, ["nope"], "")
        self.assertFalse(unknown["success"])
        self.assertIn("Unknown function", unknown["message"])
        self.assertEqual(unknown["available"], ["echo"])

        def explode(config):
            raise RuntimeError("kaboom")
        self.assertIn("kaboom", self.run_cli({"x": explode}, ["x"], "{}")["message"])
        self.assertIn("Invalid JSON", self.run_cli(echo, ["echo"], "{bad")["message"])


if __name__ == "__main__":
    unittest.main()
