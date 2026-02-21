"""
saba-chan Docker Engine Extension
==================================
Portable Docker Engine management: auto-download static binaries,
manage a local ``dockerd`` daemon, and provide Docker Compose support
-- all without requiring Docker Desktop.

On **Windows**, Docker runs inside **WSL2** (Windows Subsystem for Linux)
because Linux game-server container images need a Linux kernel.

On **Linux**, Docker runs natively via downloaded static binaries.

This extension is called by the Rust daemon when an instance uses
``use_docker: true`` in its configuration.

Usage (standalone):
    python -m extensions.docker_engine            # prints JSON status
    python extensions/docker_engine.py status      # same via plugin proto
    python extensions/docker_engine.py ensure      # download + start daemon
"""

from __future__ import annotations

import json
import os
import platform
import shutil
import signal
import stat
import subprocess
import sys
import tarfile
import time
import urllib.request
from pathlib import Path
from typing import Any, Optional

# ── Force UTF-8 stdout/stderr on Windows ─────────────────────────
if sys.stdout.encoding != "utf-8":
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")  # type: ignore[attr-defined]
if sys.stderr.encoding != "utf-8":
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")  # type: ignore[attr-defined]

# ── Constants ────────────────────────────────────────────────────

_SYSTEM = platform.system()  # "Windows", "Linux", "Darwin"

# Docker Engine static binaries (Linux -- used natively or via WSL2)
_DOCKER_ENGINE_URL = (
    "https://download.docker.com/linux/static/stable/x86_64/docker-27.5.1.tgz"
)
_COMPOSE_URL = (
    "https://github.com/docker/compose/releases/download/"
    "v2.33.1/docker-compose-linux-x86_64"
)

# WSL2 paths (Windows only)
_WSL2_DIR = "/opt/saba-chan/docker"
_WSL2_DATA = "/opt/saba-chan/docker/data"
_WSL2_LOG = "/opt/saba-chan/docker/dockerd.log"
_WSL2_PID = "/opt/saba-chan/docker/dockerd.pid"


# ── Helpers ──────────────────────────────────────────────────────

def _log(msg: str) -> None:
    """Write to stderr so it doesn't pollute stdout JSON."""
    print(f"[docker_engine] {msg}", file=sys.stderr, flush=True)


def _creation_flags() -> int:
    if _SYSTEM == "Windows":
        return 0x08000000  # CREATE_NO_WINDOW
    return 0


def _progress(percent: int, message: str) -> None:
    """Emit a structured progress line on stderr for the Rust daemon to parse."""
    import json as _json
    line = _json.dumps({"percent": percent, "message": message}, ensure_ascii=True)
    print(f"PROGRESS:{line}", file=sys.stderr, flush=True)


def _download(url: str, dest: Path, *, timeout: int = 300, label: str = "") -> None:
    """Download a file from *url* to *dest* with progress reporting."""
    _log(f"Downloading {url}")
    req = urllib.request.Request(url, headers={"User-Agent": "saba-chan/1.0"})
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        total = int(resp.headers.get("Content-Length", 0))
        if total > 0:
            # Chunked download with progress
            chunks: list[bytes] = []
            downloaded = 0
            chunk_size = 256 * 1024  # 256 KB
            last_pct = -1
            while True:
                chunk = resp.read(chunk_size)
                if not chunk:
                    break
                chunks.append(chunk)
                downloaded += len(chunk)
                pct = min(int(downloaded * 100 / total), 100)
                if pct != last_pct:
                    dl_mb = downloaded / 1_048_576
                    total_mb = total / 1_048_576
                    msg = f"{label or dest.name}: {dl_mb:.1f}/{total_mb:.1f} MB" if label else f"{dl_mb:.1f}/{total_mb:.1f} MB"
                    _progress(pct, msg)
                    last_pct = pct
            data = b"".join(chunks)
        else:
            data = resp.read()
    dest.parent.mkdir(parents=True, exist_ok=True)
    dest.write_bytes(data)
    mb = len(data) / 1_048_576
    _log(f"Downloaded {mb:.1f} MB -> {dest.name}")


def _check_log_for_fatal(text: str) -> Optional[str]:
    """Return the first fatal-error line found in *text*, or ``None``."""
    for line in text.splitlines()[-15:]:
        if "failed to start daemon" in line.lower():
            return line.strip()
    return None


def _win_to_wsl_path(win_path: Path) -> str:
    """Convert ``C:\\foo\\bar`` to ``/mnt/c/foo/bar``."""
    s = str(win_path.resolve()).replace("\\", "/")
    if len(s) >= 2 and s[1] == ":":
        return f"/mnt/{s[0].lower()}{s[2:]}"
    return s


# ── WSL2 helpers (Windows only) ──────────────────────────────────

def _wsl_run(
    args: list[str],
    *,
    timeout: int = 30,
    root: bool = False,
) -> subprocess.CompletedProcess:
    """Run *args* inside the default WSL2 distro."""
    cmd: list[str] = ["wsl"]
    if root:
        cmd += ["-u", "root"]
    cmd += ["--"]
    cmd += args
    return subprocess.run(
        cmd, capture_output=True, timeout=timeout,
        creationflags=_creation_flags(),
    )


def _wsl_available() -> bool:
    """Return ``True`` if WSL2 is installed and a default distro responds."""
    if _SYSTEM != "Windows":
        return False
    try:
        r = subprocess.run(
            ["wsl", "echo", "OK"],
            capture_output=True, timeout=15,
            creationflags=_creation_flags(),
        )
        return r.returncode == 0
    except (FileNotFoundError, subprocess.TimeoutExpired):
        return False


def _wsl_docker_installed() -> bool:
    """Return ``True`` if our Docker binaries exist inside WSL2."""
    try:
        r = _wsl_run(["test", "-x", f"{_WSL2_DIR}/dockerd"], root=True, timeout=10)
        return r.returncode == 0
    except Exception:
        return False


def _wsl_daemon_running() -> bool:
    """Return ``True`` if dockerd inside WSL2 is responding."""
    try:
        r = _wsl_run(
            [f"{_WSL2_DIR}/docker", "-H", "unix:///var/run/docker.sock", "info"],
            root=True, timeout=10,
        )
        return r.returncode == 0
    except Exception:
        return False


def _wsl_compose_installed() -> bool:
    try:
        r = _wsl_run(["test", "-x", f"{_WSL2_DIR}/docker-compose"], root=True, timeout=10)
        return r.returncode == 0
    except Exception:
        return False


def _wsl_read_log_tail(lines: int = 20) -> str:
    try:
        r = _wsl_run(["tail", f"-{lines}", _WSL2_LOG], root=True, timeout=5)
        return r.stdout.decode(errors="replace")
    except Exception:
        return ""


# ── Portable directory ───────────────────────────────────────────

def _portable_dir() -> Path:
    """Directory for saba-chan's portable Docker Engine metadata.

    On Windows this only stores local state; actual Docker binaries
    live inside WSL2 at ``/opt/saba-chan/docker/``.
    """
    exe_dir = os.environ.get("SABA_EXE_DIR", "")
    if exe_dir:
        return Path(exe_dir) / "docker"
    script_parent = Path(__file__).resolve().parent.parent
    if (script_parent / "saba-core.exe").exists() or (script_parent / "saba-core").exists():
        return script_parent / "docker"
    if _SYSTEM == "Windows":
        appdata = os.environ.get("APPDATA", "")
        if appdata:
            return Path(appdata) / "saba-chan" / "docker"
    home = os.environ.get("HOME", "")
    if home:
        return Path(home) / ".config" / "saba-chan" / "docker"
    return Path(".") / "docker"


# ── Core Class ───────────────────────────────────────────────────

class DockerEngine:
    """Manages a portable Docker Engine for saba-chan.

    On Windows every ``dockerd`` interaction goes through WSL2.
    On Linux the static binaries run natively.
    """

    def __init__(self, base_dir: Optional[str | Path] = None) -> None:
        self.wsl_mode: bool = (_SYSTEM == "Windows")
        self._dir = Path(base_dir) if base_dir else _portable_dir()
        self._dir.mkdir(parents=True, exist_ok=True)
        self._pid_file = self._dir / "dockerd.pid"

    # ── Paths (native Linux) ─────────────────────────────────

    @property
    def docker_exe(self) -> Path:
        return self._dir / "docker"

    @property
    def dockerd_exe(self) -> Path:
        return self._dir / "dockerd"

    @property
    def compose_exe(self) -> Path:
        return self._dir / "docker-compose"

    @property
    def data_root(self) -> Path:
        return self._dir / "data"

    @property
    def log_file(self) -> Path:
        return self._dir / "dockerd.log"

    # ── Detection ────────────────────────────────────────────

    @property
    def binaries_ready(self) -> bool:
        if self.wsl_mode:
            return _wsl_docker_installed()
        return self.docker_exe.exists() and self.dockerd_exe.exists()

    @property
    def compose_ready(self) -> bool:
        if self.wsl_mode:
            return _wsl_compose_installed()
        return self.compose_exe.exists()

    def _system_docker_running(self) -> bool:
        """Check if *any* (system/Docker Desktop) daemon is accessible."""
        try:
            r = subprocess.run(
                ["docker", "info"],
                capture_output=True, timeout=10,
                creationflags=_creation_flags(),
            )
            return r.returncode == 0
        except Exception:
            return False

    def _daemon_running(self) -> bool:
        """Check if our portable daemon is responding."""
        if self.wsl_mode:
            return _wsl_daemon_running()
        docker = str(self.docker_exe) if self.docker_exe.exists() else "docker"
        try:
            r = subprocess.run(
                [docker, "info"],
                capture_output=True, timeout=15,
                creationflags=_creation_flags(),
            )
            return r.returncode == 0
        except Exception:
            return False

    # ── Prerequisites ────────────────────────────────────────

    def check_prerequisites(self) -> Optional[str]:
        """Return an error string if prerequisites are missing, else ``None``."""
        if not self.wsl_mode:
            return None  # Linux: no special requirements
        if not _wsl_available():
            return (
                "WSL2_REQUIRED: Docker mode requires WSL2.\n"
                "Install it with an admin PowerShell:\n"
                "  wsl --install\n"
                "A reboot will be required after installation."
            )
        # Ensure iptables is available (dockerd needs it for bridge networking)
        self._wsl_ensure_iptables()
        return None

    def _wsl_ensure_iptables(self) -> None:
        """Install iptables inside WSL2 if missing (needed by dockerd)."""
        r = _wsl_run(["which", "iptables"], root=True, timeout=5)
        if r.returncode == 0:
            return  # already installed
        _log("iptables not found in WSL2, installing...")
        r = _wsl_run(
            ["sh", "-c",
             "apt-get update -qq && apt-get install -y -qq iptables 2>&1 | tail -3"],
            root=True, timeout=120,
        )
        if r.returncode != 0:
            _log("WARNING: iptables installation may have failed (non-fatal)")
        else:
            _log("iptables installed in WSL2")

    # ── Download ─────────────────────────────────────────────

    def ensure_available(self, *, timeout: int = 300) -> None:
        if not self.binaries_ready:
            self._download_engine(timeout=timeout)
        if not self.compose_ready:
            self._download_compose(timeout=timeout)
        # saba-docker-io: bidirectional stdin/stdout bridge for containers
        self._deploy_io_bridge()

    # -- IO Bridge --

    def _deploy_io_bridge(self) -> None:
        """Deploy saba-docker-io binary to Docker runtime directory.
        
        On WSL2 mode, copies from the extension build directory to /opt/saba-chan/docker/.
        On native Linux, the binary is found next to this extension file.
        """
        dest_name = "saba-docker-io"
        # Check if already deployed
        if self.wsl_mode:
            r = _wsl_run(["test", "-x", f"{_WSL2_DIR}/{dest_name}"], root=True, timeout=5)
            if r.returncode == 0:
                return  # Already installed
        else:
            native_dest = self._dir / dest_name
            if native_dest.exists():
                return

        # Find the built binary
        ext_dir = Path(__file__).resolve().parent
        # Look for pre-built binary in known locations
        # Go build outputs directly to the project dir (no target/ subdirectory)
        candidates = [
            ext_dir / dest_name / dest_name,       # Go build output (same dir as go.mod)
            ext_dir / "bin" / dest_name,            # manual placement
        ]
        source = None
        for c in candidates:
            if c.exists():
                source = c
                break

        if source is None:
            _log(f"saba-docker-io binary not found (checked {len(candidates)} locations), "
                 "stdin bridging to Docker containers will not be available")
            return

        if self.wsl_mode:
            wsl_tmp = _win_to_wsl_path(source)
            _wsl_run(["cp", wsl_tmp, f"{_WSL2_DIR}/{dest_name}"], root=True)
            _wsl_run(["chmod", "+x", f"{_WSL2_DIR}/{dest_name}"], root=True)
            _log(f"saba-docker-io deployed to WSL2 ({_WSL2_DIR}/{dest_name})")
        else:
            import shutil
            shutil.copy2(source, self._dir / dest_name)
            dest = self._dir / dest_name
            dest.chmod(dest.stat().st_mode | 0o755)
            _log(f"saba-docker-io deployed to {dest}")

    # -- Engine --

    def _download_engine(self, *, timeout: int = 300) -> None:
        if self.wsl_mode:
            return self._wsl_download_engine(timeout=timeout)
        self._native_download_engine(timeout=timeout)

    def _native_download_engine(self, *, timeout: int = 300) -> None:
        self._dir.mkdir(parents=True, exist_ok=True)
        archive = self._dir / "docker.tgz"
        _download(_DOCKER_ENGINE_URL, archive, timeout=timeout, label="Docker Engine")
        _log(f"Extracting to {self._dir}")
        with tarfile.open(archive, "r:gz") as tf:
            for member in tf.getmembers():
                if member.name.startswith("docker/"):
                    member.name = member.name[len("docker/"):]
                    if member.name:
                        tf.extract(member, self._dir)
        for exe in [self.docker_exe, self.dockerd_exe]:
            if exe.exists():
                exe.chmod(exe.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
        archive.unlink(missing_ok=True)
        if not self.dockerd_exe.exists():
            raise FileNotFoundError(f"dockerd not found after extraction in {self._dir}")
        _log(f"Docker Engine ready at {self._dir}")

    def _wsl_download_engine(self, *, timeout: int = 300) -> None:
        _log("Downloading Docker Engine for WSL2...")
        tmp = Path(os.environ.get("TEMP", ".")) / "saba-docker-linux.tgz"
        _download(_DOCKER_ENGINE_URL, tmp, timeout=timeout, label="Docker Engine")
        wsl_tmp = _win_to_wsl_path(tmp)
        _wsl_run(["mkdir", "-p", _WSL2_DIR], root=True)
        _wsl_run(["mkdir", "-p", _WSL2_DATA], root=True)
        _log(f"Extracting into WSL2 at {_WSL2_DIR}...")
        r = _wsl_run(
            ["tar", "xzf", wsl_tmp, "-C", _WSL2_DIR, "--strip-components=1"],
            root=True, timeout=120,
        )
        tmp.unlink(missing_ok=True)
        if r.returncode != 0:
            raise RuntimeError(
                f"Failed to extract Docker in WSL2 (exit {r.returncode}): "
                + r.stderr.decode(errors="replace")[:300]
            )
        if not _wsl_docker_installed():
            raise FileNotFoundError("dockerd not found in WSL2 after extraction")
        _log("Docker Engine binaries installed in WSL2")

    # -- Compose --

    def _download_compose(self, *, timeout: int = 300) -> None:
        if self.wsl_mode:
            return self._wsl_download_compose(timeout=timeout)
        self._native_download_compose(timeout=timeout)

    def _native_download_compose(self, *, timeout: int = 300) -> None:
        _download(_COMPOSE_URL, self.compose_exe, timeout=timeout, label="Docker Compose")
        self.compose_exe.chmod(
            self.compose_exe.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH
        )
        _log(f"Docker Compose ready at {self.compose_exe}")

    def _wsl_download_compose(self, *, timeout: int = 300) -> None:
        _log("Downloading Docker Compose for WSL2...")
        tmp = Path(os.environ.get("TEMP", ".")) / "docker-compose-linux"
        _download(_COMPOSE_URL, tmp, timeout=timeout, label="Docker Compose")
        wsl_tmp = _win_to_wsl_path(tmp)
        wsl_dest = f"{_WSL2_DIR}/docker-compose"
        _wsl_run(["cp", wsl_tmp, wsl_dest], root=True)
        _wsl_run(["chmod", "+x", wsl_dest], root=True)
        # Symlink as CLI plugin so `docker compose` (space) also works
        _wsl_run(["mkdir", "-p", "/root/.docker/cli-plugins"], root=True)
        _wsl_run(["ln", "-sf", wsl_dest, "/root/.docker/cli-plugins/docker-compose"], root=True)
        tmp.unlink(missing_ok=True)
        _log("Docker Compose installed in WSL2")

    # ── Daemon Management ────────────────────────────────────

    def start_daemon(self) -> dict[str, Any]:
        if self.wsl_mode:
            return self._wsl_start_daemon()
        return self._native_start_daemon()

    def _native_start_daemon(self) -> dict[str, Any]:
        if self._daemon_running():
            _log("Docker daemon is already running")
            return {"started": False, "reason": "already_running", "pid": self._read_pid()}
        if not self.dockerd_exe.exists():
            raise FileNotFoundError(f"dockerd not found at {self.dockerd_exe}")
        self.data_root.mkdir(parents=True, exist_ok=True)
        args = [str(self.dockerd_exe), "--data-root", str(self.data_root)]
        _log(f"Spawning: {' '.join(args)}")
        log_fh = open(self.log_file, "a")
        kwargs: dict[str, Any] = {
            "stdout": log_fh, "stderr": log_fh,
            "start_new_session": True,
        }
        proc = subprocess.Popen(args, **kwargs)
        self._pid_file.write_text(str(proc.pid))
        _log(f"dockerd spawned (PID {proc.pid})")
        return {"started": True, "pid": proc.pid}

    def _wsl_start_daemon(self) -> dict[str, Any]:
        if _wsl_daemon_running():
            _log("WSL2 Docker daemon is already running")
            return {"started": False, "reason": "already_running", "pid": self._wsl_read_pid()}
        # Clear old log
        _wsl_run(["sh", "-c", f"> {_WSL2_LOG}"], root=True)

        # Write a launcher script *inside* WSL so it persists across sessions.
        launcher = f"""\
#!/bin/sh
export PATH="{_WSL2_DIR}:$PATH"
# Start containerd if not running
if ! pgrep -x containerd > /dev/null 2>&1; then
    {_WSL2_DIR}/containerd > /dev/null 2>&1 &
    sleep 2
fi
# Start dockerd (foreground -- the WSL session keeps it alive)
exec {_WSL2_DIR}/dockerd \\
  -H unix:///var/run/docker.sock \\
  --data-root {_WSL2_DATA} \\
  --userland-proxy-path {_WSL2_DIR}/docker-proxy \\
  >> {_WSL2_LOG} 2>&1
"""
        launcher_path = f"{_WSL2_DIR}/start-dockerd.sh"
        # Write launcher via Popen stdin pipe (heredoc breaks on Windows)
        write_proc = subprocess.Popen(
            ["wsl", "-u", "root", "--", "sh", "-c",
             f"cat > {launcher_path} && chmod +x {launcher_path}"],
            stdin=subprocess.PIPE,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            creationflags=_creation_flags(),
        )
        _, write_err = write_proc.communicate(input=launcher.encode("utf-8"), timeout=10)
        if write_proc.returncode != 0:
            _log(f"Failed to write launcher script: {write_err.decode(errors='replace')[:200]}")
            return {"started": False, "reason": "launcher_write_failed"}

        # Launch the script via Popen so the WSL session stays alive.
        # The subprocess keeps running after we return; dockerd lives
        # as long as this WSL session lives.
        _log("Starting WSL2 Docker daemon (Popen)...")
        wsl_proc = subprocess.Popen(
            ["wsl", "-u", "root", "--", launcher_path],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            creationflags=_creation_flags(),
        )
        # wsl_proc.pid is the Windows-side PID of the `wsl` wrapper process
        # Save it so we can kill it later to stop the whole tree
        pid = wsl_proc.pid
        _wsl_run(["sh", "-c", f"echo {pid} > {_WSL2_PID}"], root=True, timeout=5)

        # Also store the Windows PID on the host side for stop_daemon
        self._pid_file.parent.mkdir(parents=True, exist_ok=True)
        self._pid_file.write_text(str(pid))

        _log(f"WSL2 dockerd session started (WSL wrapper PID {pid})")
        return {"started": True, "pid": pid}

    def wait_for_ready(self, timeout_secs: int = 120) -> bool:
        if self.wsl_mode:
            return self._wsl_wait_for_ready(timeout_secs)
        return self._native_wait_for_ready(timeout_secs)

    def _native_wait_for_ready(self, timeout_secs: int = 120) -> bool:
        docker = str(self.docker_exe) if self.docker_exe.exists() else "docker"
        _log(f"Waiting up to {timeout_secs}s for Docker daemon...")
        deadline = time.monotonic() + timeout_secs
        attempt = 0
        while time.monotonic() < deadline:
            attempt += 1
            try:
                r = subprocess.run(
                    [docker, "info"], capture_output=True, timeout=10,
                    creationflags=_creation_flags(),
                )
                if r.returncode == 0:
                    _log(f"Docker daemon ready (attempt {attempt})")
                    return True
            except Exception:
                pass
            if attempt % 3 == 0 and self.log_file.exists():
                text = self.log_file.read_text(errors="replace")
                fatal = _check_log_for_fatal(text)
                if fatal:
                    _log(f"dockerd fatal: {fatal}")
                    return False
            remaining = int(deadline - time.monotonic())
            if attempt % 5 == 0:
                _log(f"Still waiting... ({remaining}s remaining)")
            time.sleep(3)
        return False

    def _wsl_wait_for_ready(self, timeout_secs: int = 120) -> bool:
        _log(f"Waiting up to {timeout_secs}s for WSL2 Docker daemon...")
        deadline = time.monotonic() + timeout_secs
        attempt = 0
        while time.monotonic() < deadline:
            attempt += 1
            if _wsl_daemon_running():
                _log(f"WSL2 Docker daemon ready (attempt {attempt})")
                return True
            if attempt % 3 == 0:
                log_tail = _wsl_read_log_tail(10)
                fatal = _check_log_for_fatal(log_tail)
                if fatal:
                    _log(f"dockerd fatal: {fatal}")
                    return False
            remaining = int(deadline - time.monotonic())
            if attempt % 5 == 0:
                _log(f"Still waiting for WSL2 dockerd... ({remaining}s remaining)")
            time.sleep(3)
        return False

    def stop_daemon(self) -> dict[str, Any]:
        if self.wsl_mode:
            return self._wsl_stop_daemon()
        return self._native_stop_daemon()

    def _native_stop_daemon(self) -> dict[str, Any]:
        pid = self._read_pid()
        if pid is None:
            return {"stopped": False, "reason": "no_pid_file"}
        try:
            os.kill(pid, signal.SIGTERM)
            for _ in range(10):
                try:
                    os.kill(pid, 0)
                except OSError:
                    break
                time.sleep(1)
            self._pid_file.unlink(missing_ok=True)
            _log(f"dockerd (PID {pid}) stopped")
            return {"stopped": True, "pid": pid}
        except Exception as e:
            return {"stopped": False, "error": str(e), "pid": pid}

    def _wsl_stop_daemon(self) -> dict[str, Any]:
        # The PID stored in _pid_file is the Windows-side wsl.exe wrapper.
        # Killing it tears down the WSL session and stops dockerd+containerd.
        pid = self._read_pid()  # Windows PID from host-side file
        if pid is None:
            # Fallback: try to kill dockerd inside WSL directly
            _wsl_run(["sh", "-c", "pkill -x dockerd 2>/dev/null; pkill -x containerd 2>/dev/null"],
                      root=True, timeout=10)
            _wsl_run(["rm", "-f", _WSL2_PID], root=True)
            return {"stopped": True, "reason": "killed_via_pkill"}
        try:
            subprocess.run(
                ["taskkill", "/F", "/PID", str(pid)],
                capture_output=True, timeout=10,
                creationflags=_creation_flags(),
            )
            self._pid_file.unlink(missing_ok=True)
            _wsl_run(["rm", "-f", _WSL2_PID], root=True)
            # Also clean up any orphans
            _wsl_run(["sh", "-c", "pkill -x dockerd 2>/dev/null; pkill -x containerd 2>/dev/null"],
                      root=True, timeout=10)
            _log(f"WSL2 Docker session stopped (Win PID {pid})")
            return {"stopped": True, "pid": pid}
        except Exception as e:
            return {"stopped": False, "error": str(e), "pid": pid}

    # ── PID helpers ──────────────────────────────────────────

    def _read_pid(self) -> Optional[int]:
        if self._pid_file.exists():
            try:
                return int(self._pid_file.read_text().strip())
            except ValueError:
                pass
        return None

    def _wsl_read_pid(self) -> Optional[int]:
        try:
            r = _wsl_run(["cat", _WSL2_PID], root=True, timeout=5)
            if r.returncode == 0:
                return int(r.stdout.decode(errors="replace").strip())
        except Exception:
            pass
        return None

    # ── Status ───────────────────────────────────────────────

    def status(self) -> dict[str, Any]:
        if self.wsl_mode:
            return {
                "wsl_mode": True,
                "binaries_ready": self.binaries_ready,
                "compose_ready": self.compose_ready,
                "daemon_running": _wsl_daemon_running(),
                "system_docker": self._system_docker_running(),
                "portable_dir": _WSL2_DIR,
                "docker_exe": f"{_WSL2_DIR}/docker",
                "compose_exe": f"{_WSL2_DIR}/docker-compose",
                "pid": self._wsl_read_pid(),
            }
        docker = str(self.docker_exe) if self.docker_exe.exists() else "docker"
        return {
            "wsl_mode": False,
            "binaries_ready": self.binaries_ready,
            "compose_ready": self.compose_ready,
            "daemon_running": self._daemon_running(),
            "system_docker": self._system_docker_running(),
            "portable_dir": str(self._dir),
            "docker_exe": str(self.docker_exe) if self.docker_exe.exists() else None,
            "compose_exe": str(self.compose_exe) if self.compose_exe.exists() else None,
            "pid": self._read_pid(),
        }

    def info(self) -> dict[str, Any]:
        versions: dict[str, Optional[str]] = {
            "docker_version": None,
            "compose_version": None,
        }
        if self.wsl_mode:
            try:
                r = _wsl_run([f"{_WSL2_DIR}/docker", "--version"], root=True, timeout=10)
                if r.returncode == 0:
                    versions["docker_version"] = r.stdout.decode(errors="replace").strip()
            except Exception:
                pass
            try:
                r = _wsl_run([f"{_WSL2_DIR}/docker-compose", "version"], root=True, timeout=10)
                if r.returncode == 0:
                    versions["compose_version"] = r.stdout.decode(errors="replace").strip()
            except Exception:
                pass
        else:
            docker = str(self.docker_exe) if self.docker_exe.exists() else "docker"
            try:
                r = subprocess.run(
                    [docker, "--version"], capture_output=True, text=True,
                    timeout=10, creationflags=_creation_flags(),
                )
                if r.returncode == 0:
                    versions["docker_version"] = r.stdout.strip()
            except Exception:
                pass
            compose = str(self.compose_exe) if self.compose_exe.exists() else None
            if compose:
                try:
                    r = subprocess.run(
                        [compose, "version"], capture_output=True, text=True,
                        timeout=10, creationflags=_creation_flags(),
                    )
                    if r.returncode == 0:
                        versions["compose_version"] = r.stdout.strip()
                except Exception:
                    pass
        return {**self.status(), **versions}


# ── Plugin Runner Interface ──────────────────────────────────────

def _plugin_ensure(config: dict) -> dict:
    """Download binaries + start daemon + wait for readiness."""
    engine = DockerEngine(config.get("base_dir"))

    # 0. Check if system Docker (Docker Desktop etc.) is already running
    if engine._system_docker_running():
        return {
            "success": True,
            "message": "System Docker is already running",
            "daemon_ready": True,
            "wsl_mode": False,
        }

    # 1. Our portable daemon already running?
    if engine.binaries_ready and engine._daemon_running():
        return {
            "success": True,
            "message": "Portable Docker daemon is running",
            "daemon_ready": True,
            "wsl_mode": engine.wsl_mode,
        }

    # 2. Prerequisites
    prereq_err = engine.check_prerequisites()
    if prereq_err:
        tag = prereq_err.split(":")[0] if ":" in prereq_err[:20] else ""
        display_msg = prereq_err.split(": ", 1)[-1] if tag else prereq_err
        return {
            "success": False,
            "message": display_msg,
            "daemon_ready": False,
            "needs_reboot": "REBOOT" in tag,
            "wsl_mode": engine.wsl_mode,
        }

    # 3. Download if needed
    if not engine.binaries_ready:
        engine.ensure_available(timeout=config.get("timeout", 300))

    # 4. Start daemon
    engine.start_daemon()

    # 5. Wait for readiness
    ready = engine.wait_for_ready(timeout_secs=config.get("wait_timeout", 120))

    # 6. Build result
    if engine.wsl_mode:
        log_tail = _wsl_read_log_tail(20)
    else:
        log_tail = ""
        if engine.log_file.exists():
            try:
                log_tail = engine.log_file.read_text(errors="replace")
            except Exception:
                pass

    fatal = _check_log_for_fatal(log_tail) if not ready else None

    if fatal and not ready:
        message = f"Docker daemon start failed: {fatal}"
    elif ready:
        message = "Docker Engine ready"
    else:
        message = "dockerd started but did not respond in time"

    result: dict[str, Any] = {
        "success": ready,
        "message": message,
        "daemon_ready": ready,
        "wsl_mode": engine.wsl_mode,
        **engine.status(),
    }

    if not ready and log_tail:
        lines = log_tail.splitlines()
        result["dockerd_log_tail"] = "\n".join(lines[-20:])

    return result


def _plugin_start_daemon(config: dict) -> dict:
    engine = DockerEngine(config.get("base_dir"))
    result = engine.start_daemon()
    started_or_running = result.get("started") or result.get("reason") == "already_running"
    if started_or_running:
        ready = engine.wait_for_ready(timeout_secs=config.get("wait_timeout", 60))
        return {"success": ready, "daemon_ready": ready, "wsl_mode": engine.wsl_mode, **result}
    return {"success": False, "daemon_ready": False, "wsl_mode": engine.wsl_mode, **result}


def _plugin_stop_daemon(config: dict) -> dict:
    engine = DockerEngine(config.get("base_dir"))
    result = engine.stop_daemon()
    return {"success": result.get("stopped", False), **result}


def _plugin_status(config: dict) -> dict:
    engine = DockerEngine(config.get("base_dir"))
    return {"success": True, **engine.status()}


def _plugin_info(config: dict) -> dict:
    engine = DockerEngine(config.get("base_dir"))
    return {"success": True, **engine.info()}


FUNCTIONS = {
    "ensure": _plugin_ensure,
    "start_daemon": _plugin_start_daemon,
    "stop_daemon": _plugin_stop_daemon,
    "status": _plugin_status,
    "info": _plugin_info,
}


# ── CLI ──────────────────────────────────────────────────────────

if __name__ == "__main__":
    if len(sys.argv) < 2:
        engine = DockerEngine()
        print(json.dumps(engine.info(), indent=2))
        sys.exit(0)

    function_name = sys.argv[1]
    fn = FUNCTIONS.get(function_name)
    if fn is None:
        result = {"success": False, "message": f"Unknown function: {function_name}"}
    else:
        try:
            input_data = sys.stdin.read()
            # Strip UTF-8 BOM if present (PowerShell may inject one)
            if input_data.startswith("\ufeff"):
                input_data = input_data[1:]
            config_data = json.loads(input_data) if input_data.strip() else {}
            result = fn(config_data)
        except Exception as e:
            result = {"success": False, "message": str(e)}

    json.dump(result, sys.stdout, ensure_ascii=True)
