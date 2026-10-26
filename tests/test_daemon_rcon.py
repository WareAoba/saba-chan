import ctypes  # os.name을 "nt"로 바꾼 동안에는 처음 import하지 못한다
import io
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "modules"))

import daemon_rcon


def run_lifecycle(source):
    script = (
        f"import sys; sys.path.insert(0, {str(Path(daemon_rcon.__file__).parent)!r})\n"
        "import daemon_rcon\n"
        "sys.argv = ['lifecycle.py', 'target']\n"
        + source
    )
    return subprocess.run([sys.executable, "-c", script], input="{}", capture_output=True, text=True)


class SdkPathTests(unittest.TestCase):
    def test_safe_join_keeps_names_inside_root(self):
        self.assertEqual(daemon_rcon.SINCE["safe_join"], "0.1.10")
        self.assertEqual(daemon_rcon.SINCE["PathOutside"], "0.1.10")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.assertEqual(
                daemon_rcon.safe_join(root, "worlds", "서울"),
                root.resolve() / "worlds" / "서울",
            )
            for part in ("..", "../world", "world/name", "world\\name", "/world", "C:\\world", "C:world"):
                with self.subTest(part=part), self.assertRaises(daemon_rcon.PathOutside):
                    daemon_rcon.safe_join(root, part)

    def test_safe_join_rejects_link_to_outside_root(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "root"
            outside = Path(directory) / "outside"
            root.mkdir()
            outside.mkdir()
            link = root / "link"
            if os.name == "nt":
                subprocess.run(["cmd", "/c", "mklink", "/J", str(link), str(outside)], check=True, capture_output=True)
            else:
                link.symlink_to(outside, target_is_directory=True)
            with self.assertRaises(daemon_rcon.PathOutside):
                daemon_rcon.safe_join(root, "link", "world")


class SdkLifecycleTests(unittest.TestCase):
    def test_path_outside_keeps_path_and_error_code(self):
        output = run_lifecycle(
            "daemon_rcon.run_lifecycle({'target': lambda config: daemon_rcon.safe_join('.', '..')})"
        )
        self.assertEqual(output.returncode, 0, output.stderr)
        result = json.loads(output.stdout)
        self.assertFalse(result["success"])
        self.assertEqual(result["error_code"], "module_path_outside")
        self.assertTrue(result["message"])
        self.assertTrue(result["path"])

    def test_exception_keeps_traceback_and_error_code(self):
        output = run_lifecycle(
            "def target(config):\n"
            "    raise RuntimeError('lifecycle failed')\n"
            "daemon_rcon.run_lifecycle({'target': target})"
        )
        self.assertEqual(output.returncode, 0, output.stderr)
        result = json.loads(output.stdout)
        self.assertFalse(result["success"])
        self.assertEqual(result["error_code"], "module_exception")
        self.assertIn("lifecycle failed", result["message"])
        self.assertIn("Traceback", output.stderr)

    def test_serialization_failure_still_returns_json_and_exit_zero(self):
        for value in ("{1, 2}", "float('nan')", "cyclic"):
            with self.subTest(value=value):
                output = run_lifecycle(
                    "cyclic = {}; cyclic['self'] = cyclic\n"
                    f"daemon_rcon.run_lifecycle({{'target': lambda config: {{'result': {value}}}}})"
                )
                self.assertEqual(output.returncode, 0, output.stderr)
                result = json.loads(output.stdout)
                self.assertFalse(result["success"])
                self.assertEqual(result["error_code"], "module_bad_output")
                self.assertTrue(result["message"])


class HostDaemon(io.StringIO):
    def __init__(self, directory, reply, prefix="HOST:"):
        super().__init__()
        self.directory = Path(directory)
        self.reply = reply
        self.prefix = prefix
        self.requests = []

    def write(self, text):
        result = super().write(text)
        if text.endswith("\n"):
            request = json.loads(self.getvalue().splitlines()[-1].removeprefix(self.prefix))
            self.requests.append(request)
            if self.reply is not None:
                body = self.reply if isinstance(self.reply, str) else json.dumps(self.reply)
                (self.directory / f"{request['id']}.json").write_text(body, encoding="utf-8")
        return result


class SdkHostTests(unittest.TestCase):
    def setUp(self):
        http = patch.object(daemon_rcon, "daemon_post", side_effect=AssertionError("unexpected HTTP fallback"))
        http.start()
        self.addCleanup(http.stop)

    def call_with_host(self, call, reply):
        with tempfile.TemporaryDirectory() as directory:
            host = HostDaemon(directory, reply)
            with patch.dict(os.environ, {"SABA_HOST_DIR": directory}), patch.object(sys, "stderr", host):
                result = call()
            self.assertEqual(list(Path(directory).iterdir()), [])
            return result, host.requests

    def test_stdin_uses_host_without_a_request_selected_instance(self):
        data = {"success": True, "message": "sent"}
        result, requests = self.call_with_host(
            lambda: daemon_rcon.daemon_stdin_command("ignored-instance", "save 서울"),
            {"ok": True, "data": data},
        )
        self.assertEqual(result, data)
        self.assertEqual(len(requests), 1)
        self.assertRegex(requests[0].pop("id"), r"^[a-f0-9]{32}$")
        self.assertEqual(requests[0], {"op": "instance.stdin", "args": {"command": "save 서울"}})

    def test_rest_keeps_method_path_and_body(self):
        result, requests = self.call_with_host(
            lambda: daemon_rcon.daemon_rest("ignored-instance", "DELETE", "/v1/api/world", body={"name": "서울"}),
            {"ok": True, "data": {"success": True}},
        )
        self.assertEqual(result, {"success": True})
        self.assertEqual(requests[0]["op"], "instance.rest")
        self.assertEqual(requests[0]["args"], {
            "method": "DELETE", "path": "/v1/api/world", "body": {"name": "서울"},
        })

    def test_host_failure_preserves_error_code_and_consumes_reply(self):
        with tempfile.TemporaryDirectory() as directory:
            host = HostDaemon(directory, {"ok": False, "error_code": "no_instance", "error": "instance context is required"})
            with patch.dict(os.environ, {"SABA_HOST_DIR": directory}), patch.object(sys, "stderr", host):
                with self.assertRaises(ConnectionError) as raised:
                    daemon_rcon.daemon_stdin_command("ignored-instance", "stop")
            self.assertEqual(raised.exception.error_code, "no_instance")
            self.assertIn("instance context is required", str(raised.exception))
            self.assertEqual(list(Path(directory).iterdir()), [])

    def test_host_rejects_oversized_request_before_emitting_it(self):
        with tempfile.TemporaryDirectory() as directory:
            host = HostDaemon(directory, None)
            with patch.dict(os.environ, {"SABA_HOST_DIR": directory}), patch.object(sys, "stderr", host):
                with self.assertRaises(ConnectionError) as raised:
                    daemon_rcon.daemon_stdin_command("ignored-instance", "가" * 22000)
            self.assertEqual(raised.exception.error_code, "request_too_large")
            self.assertEqual(host.requests, [])

    def test_host_rejects_oversized_or_invalid_reply_and_removes_it(self):
        for body, code in (("x" * (1024 * 1024 + 1), "response_too_large"), ("not json", "invalid_response")):
            with self.subTest(code=code), tempfile.TemporaryDirectory() as directory:
                host = HostDaemon(directory, body)
                with patch.dict(os.environ, {"SABA_HOST_DIR": directory}), patch.object(sys, "stderr", host):
                    with self.assertRaises(ConnectionError) as raised:
                        daemon_rcon.daemon_stdin_command("ignored-instance", "stop")
                self.assertEqual(raised.exception.error_code, code)
                self.assertEqual(list(Path(directory).iterdir()), [])

    def test_host_wait_times_out(self):
        with tempfile.TemporaryDirectory() as directory:
            host = HostDaemon(directory, None)
            with patch.dict(os.environ, {"SABA_HOST_DIR": directory}), patch.object(sys, "stderr", host):
                with self.assertRaises(TimeoutError):
                    daemon_rcon.daemon_stdin_command("ignored-instance", "stop", timeout=0.01)

    def test_host_wait_never_exceeds_thirty_seconds(self):
        with tempfile.TemporaryDirectory() as directory:
            host = HostDaemon(directory, None)
            with patch.dict(os.environ, {"SABA_HOST_DIR": directory}), patch.object(sys, "stderr", host):
                with patch.object(daemon_rcon.time, "monotonic", side_effect=(0, 31)):
                    with patch.object(daemon_rcon.time, "sleep", side_effect=AssertionError("wait exceeded thirty seconds")):
                        with self.assertRaises(TimeoutError):
                            daemon_rcon.daemon_stdin_command("ignored-instance", "stop", timeout=1000)

    def test_stdin_and_rest_keep_the_http_fallback(self):
        with patch.dict(os.environ), patch.object(daemon_rcon, "daemon_post", return_value={"success": True}) as post:
            os.environ.pop("SABA_HOST_DIR", None)
            self.assertEqual(daemon_rcon.daemon_stdin_command("server-1", "stop", timeout=4), {"success": True})
            post.assert_called_with("/api/instance/server-1/stdin", {"command": "stop"}, timeout=4)
            self.assertEqual(daemon_rcon.daemon_rest("server-1", "PUT", "/world", body={"x": 1}, timeout=6), {"success": True})
            post.assert_called_with("/api/instance/server-1/rest", {"method": "PUT", "endpoint": "/world", "body": {"x": 1}}, timeout=6)


class SdkReplyTests(unittest.TestCase):
    def test_host_and_loopback_retry_busy_and_partial_replies(self):
        for relay in ("HOST", "LOOPBACK"):
            with self.subTest(relay=relay), tempfile.TemporaryDirectory() as directory:
                data = {"success": True} if relay == "HOST" else "YW5zd2Vy"
                daemon = HostDaemon(directory, {"ok": True, "data": data}, prefix=f"{relay}:")
                original_open = Path.open
                reads = 0

                def open_reply(path, mode="r", *args, **kwargs):
                    nonlocal reads
                    if "r" in mode and path.suffix == ".json":
                        reads += 1
                        if reads == 1:
                            raise PermissionError("reply is still open by the writer")
                        if reads == 2:
                            return io.BytesIO(b'{"ok":') if "b" in mode else io.StringIO('{"ok":')
                    return original_open(path, mode, *args, **kwargs)

                with patch.dict(os.environ, {f"SABA_{relay}_DIR": directory}), patch.object(sys, "stderr", daemon):
                    with patch.object(Path, "open", open_reply):
                        if relay == "HOST":
                            result = daemon_rcon.daemon_stdin_command("ignored-instance", "save", timeout=0.2)
                            self.assertEqual(result, {"success": True})
                        else:
                            result = daemon_rcon.loopback_exchange(12345, b"request", timeout=0.2)
                            self.assertEqual(result, b"answer")
                self.assertEqual(list(Path(directory).iterdir()), [])


class SdkVersionTests(unittest.TestCase):
    def test_since_covers_public_sdk_names_and_i18n(self):
        import inspect
        public = {
            name for name, value in vars(daemon_rcon).items()
            if not name.startswith("_") and getattr(value, "__module__", None) == "daemon_rcon"
            and (inspect.isfunction(value) or inspect.isclass(value))
        }
        self.assertEqual(set(daemon_rcon.SINCE), public | {"i18n.I18n"})
        self.assertEqual(daemon_rcon.SINCE["i18n.I18n"], "0.1.1")
        self.assertEqual(daemon_rcon.SINCE["daemon_rest"], "0.1.10")


class SdkAtomicWriteTests(unittest.TestCase):
    """Windows 격리 모듈은 기존 파일의 DACL을 옮길 권한(WRITE_DAC)이 없어 ReplaceFileW가 거부된다"""

    def replace_denied(self, same_access):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "serverconfig.txt"
            path.write_bytes(b"old")
            with patch.object(os, "name", "nt"), \
                    patch.object(daemon_rcon, "_replace_file", return_value=5), \
                    patch.object(daemon_rcon, "_access_of", side_effect=lambda p: "D:AI(A;ID;FA;;;SY)" if same_access or p.endswith("serverconfig.txt") else "D:AI(A;ID;FA;;;WD)"), \
                    patch("ctypes.WinError", lambda code: PermissionError(13, f"WinError {code}"), create=True):
                try:
                    daemon_rcon.atomic_write(path, b"new")
                    error = None
                except OSError as e:
                    error = e
            return path.read_bytes(), error, sorted(p.name for p in Path(directory).iterdir())

    def test_denied_replace_falls_back_when_the_temp_file_has_the_same_access(self):
        data, error, names = self.replace_denied(same_access=True)
        self.assertIsNone(error)
        self.assertEqual(data, b"new")
        self.assertEqual(names, ["serverconfig.txt"])

    def test_denied_replace_writes_a_file_with_its_own_access_in_place(self):
        """PC 실측: 격리된 팰월드 설정 저장이 매번 실패했다. 제자리에 쓰면 원래 파일의 권한·무결성 표시가 그대로다"""
        data, error, names = self.replace_denied(same_access=False)
        self.assertIsNone(error)
        self.assertEqual(data, b"new")
        self.assertEqual(names, ["serverconfig.txt"])

    def test_a_folder_without_create_rights_is_written_in_place(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "server.properties"
            path.write_bytes(b"old")
            inode = path.stat().st_ino
            with patch.object(daemon_rcon.tempfile, "mkstemp", side_effect=PermissionError(13, "denied")):
                daemon_rcon.atomic_write(path, b"new")
            self.assertEqual(path.read_bytes(), b"new")
            self.assertEqual(path.stat().st_ino, inode)

    def test_a_label_that_differs_only_in_the_auto_inherited_flag_is_the_same_access(self):
        """데몬이 기존 파일에 맞춘 상속 표시(S:(ML;ID;..))와 OS가 새 파일에 붙인 것(S:AI(ML;ID;..))은 같은 권한이다"""
        old = "O:BAD:AI(A;ID;FA;;;SY)S:(ML;ID;NW;;;LW)"
        new = "O:BAD:AI(A;ID;FA;;;SY)S:AI(ML;ID;NW;;;LW)"
        self.assertTrue(daemon_rcon._same_access(old, new))
        self.assertFalse(daemon_rcon._same_access("O:BAD:AI(A;ID;FA;;;SY)", new))
        self.assertFalse(daemon_rcon._same_access("O:BAD:AI(A;ID;FA;;;SY)S:(ML;;NW;;;LW)", new))
        self.assertFalse(daemon_rcon._same_access(None, None))

    @unittest.skipUnless(os.name == "nt", "Windows DACL")
    def test_access_of_tells_an_explicit_grant_apart(self):
        with tempfile.TemporaryDirectory() as directory:
            a, b = Path(directory) / "a.txt", Path(directory) / "b.txt"
            a.write_bytes(b"a")
            b.write_bytes(b"b")
            self.assertIsNotNone(daemon_rcon._access_of(str(a)))
            self.assertEqual(daemon_rcon._access_of(str(a)), daemon_rcon._access_of(str(b)))
            subprocess.run(["icacls", str(a), "/grant", "*S-1-1-0:(R)"], check=True, capture_output=True)
            self.assertNotEqual(daemon_rcon._access_of(str(a)), daemon_rcon._access_of(str(b)))


if __name__ == "__main__":
    unittest.main()
