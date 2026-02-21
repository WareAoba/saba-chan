"""
saba-chan SteamCMD Extension
=============================
Portable SteamCMD management: auto-detect, auto-download, install & update
game dedicated servers via Valve's SteamCMD CLI.

This extension is used by game modules whose ``module.toml`` specifies
``method = "steamcmd"`` in the ``[install]`` section.

Usage (from a game module lifecycle.py):
    from extensions.steamcmd import SteamCMD

    steam = SteamCMD()                      # auto-detects or bootstraps
    steam.ensure_available()                 # downloads from Valve CDN if needed
    steam.install(app_id=2394010,            # Palworld dedicated server
                  install_dir="/srv/palworld/server",
                  anonymous=True)

Standalone CLI check:
    python -m extensions.steamcmd            # prints JSON status blob
"""

from __future__ import annotations

import io
import json
import os
import platform
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile
import urllib.request
import zipfile
from pathlib import Path
from typing import Any, Optional

# ── Constants ────────────────────────────────────────────────────

_SYSTEM = platform.system()  # "Windows", "Linux", "Darwin"

_STEAMCMD_URLS: dict[str, str] = {
    "Windows": "https://steamcdn-a.akamaihd.net/client/installer/steamcmd.zip",
    "Linux": "https://steamcdn-a.akamaihd.net/client/installer/steamcmd_linux.tar.gz",
    "Darwin": "https://steamcdn-a.akamaihd.net/client/installer/steamcmd_osx.tar.gz",
}

_EXE_NAME = "steamcmd.exe" if _SYSTEM == "Windows" else "steamcmd.sh"

_PORTABLE_SUBDIR = "steamcmd"


# ── Helpers ──────────────────────────────────────────────────────

def _portable_dir() -> Path:
    """Return the directory used by saba-chan's portable SteamCMD.

    Windows : ``%APPDATA%/saba-chan/steamcmd``
    Linux   : ``~/.config/saba-chan/steamcmd``
    macOS   : ``~/.config/saba-chan/steamcmd``
    """
    if _SYSTEM == "Windows":
        base = os.environ.get("APPDATA", "")
        if not base:
            raise EnvironmentError("APPDATA environment variable not set")
        return Path(base) / "saba-chan" / _PORTABLE_SUBDIR
    else:
        home = os.environ.get("HOME", "")
        if not home:
            raise EnvironmentError("HOME environment variable not set")
        return Path(home) / ".config" / "saba-chan" / _PORTABLE_SUBDIR


def _common_candidates() -> list[Path]:
    """Well-known SteamCMD installation locations per platform."""
    candidates: list[Path] = []
    if _SYSTEM == "Windows":
        candidates.append(Path(r"C:\SteamCMD\steamcmd.exe"))
        candidates.append(Path(r"C:\steamcmd\steamcmd.exe"))
        pf = os.environ.get("ProgramFiles", "")
        if pf:
            candidates.append(Path(pf) / "SteamCMD" / "steamcmd.exe")
        pf86 = os.environ.get("ProgramFiles(x86)", "")
        if pf86:
            candidates.append(Path(pf86) / "SteamCMD" / "steamcmd.exe")
            candidates.append(Path(pf86) / "Steam" / "steamcmd.exe")
    else:
        home = os.environ.get("HOME", "")
        if home:
            candidates.append(Path(home) / "steamcmd" / "steamcmd.sh")
            candidates.append(Path(home) / "Steam" / "steamcmd.sh")
        candidates.append(Path("/usr/games/steamcmd"))
        candidates.append(Path("/usr/local/bin/steamcmd"))
    return candidates


# ── Core Class ───────────────────────────────────────────────────

class SteamCMD:
    """Manages a SteamCMD installation for saba-chan.

    Calling :meth:`ensure_available` guarantees that a working ``steamcmd``
    binary is present — downloading a fresh portable copy from Valve if
    necessary.
    """

    def __init__(self, explicit_path: Optional[str | Path] = None) -> None:
        if explicit_path is not None:
            self._path: Optional[Path] = Path(explicit_path)
        else:
            self._path = self._detect()

    # ── Properties ───────────────────────────────────────────

    @property
    def available(self) -> bool:
        return self._path is not None and self._path.exists()

    @property
    def path(self) -> Optional[Path]:
        return self._path

    # ── Detection ────────────────────────────────────────────

    @staticmethod
    def _detect() -> Optional[Path]:
        """Try to find an existing SteamCMD binary on this system."""
        # 1) Our own portable copy
        try:
            portable_exe = _portable_dir() / _EXE_NAME
            if portable_exe.exists():
                return portable_exe
        except EnvironmentError:
            pass

        # 2) Common install locations
        for candidate in _common_candidates():
            if candidate.exists():
                return candidate

        # 3) System PATH
        found = shutil.which(_EXE_NAME.replace(".exe", "") if _SYSTEM != "Windows" else _EXE_NAME)
        if found:
            return Path(found)

        return None

    # ── Auto-bootstrap ───────────────────────────────────────

    def ensure_available(self, *, timeout: int = 120) -> None:
        """Download and extract a portable SteamCMD if not already present.

        After this call :attr:`available` is guaranteed to be ``True`` (or
        an exception was raised).
        """
        if self.available:
            return

        portable = _portable_dir()
        exe_path = portable / _EXE_NAME

        # Perhaps downloaded in a previous session
        if exe_path.exists():
            self._path = exe_path
            return

        url = _STEAMCMD_URLS.get(_SYSTEM)
        if url is None:
            raise RuntimeError(f"No SteamCMD download URL for platform: {_SYSTEM}")

        _log(f"SteamCMD not found — downloading from Valve CDN ({url}) …")

        portable.mkdir(parents=True, exist_ok=True)

        # Download
        req = urllib.request.Request(url, headers={"User-Agent": "saba-chan/1.0"})
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            total = int(resp.headers.get("Content-Length", 0))
            if total > 0:
                chunks: list[bytes] = []
                downloaded = 0
                chunk_size = 64 * 1024
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
                        _progress(pct, f"SteamCMD: {dl_mb:.1f}/{total_mb:.1f} MB")
                        last_pct = pct
                data = b"".join(chunks)
            else:
                data = resp.read()

        size_mb = len(data) / 1_048_576
        _log(f"Downloaded {size_mb:.1f} MB, extracting …")

        # Extract
        if url.endswith(".zip"):
            with zipfile.ZipFile(io.BytesIO(data)) as zf:
                zf.extractall(portable)
        else:
            with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as tf:
                tf.extractall(portable)

        if not exe_path.exists():
            raise FileNotFoundError(
                f"Archive extracted but {_EXE_NAME} not found in {portable}"
            )

        # Make executable on Unix
        if _SYSTEM != "Windows":
            exe_path.chmod(exe_path.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)

        _log(f"Portable SteamCMD ready at {exe_path}")
        self._path = exe_path

    # ── Install / Update ─────────────────────────────────────

    def install(
        self,
        app_id: int,
        install_dir: str | Path,
        *,
        anonymous: bool = True,
        beta: Optional[str] = None,
        beta_password: Optional[str] = None,
        platform_override: Optional[str] = None,
        validate: bool = True,
    ) -> dict[str, Any]:
        """Run ``steamcmd +app_update`` to install or update a game server.

        Returns a dict with ``success``, ``message``, ``install_dir``,
        ``app_id`` keys.
        """
        if not self.available:
            raise RuntimeError(
                "SteamCMD is not available. Call ensure_available() first."
            )

        install_dir = Path(install_dir)
        install_dir.mkdir(parents=True, exist_ok=True)

        args: list[str] = [str(self._path)]

        # ── Login 전에 설정해야 하는 옵션들 ──

        # Platform override (반드시 login 전)
        if platform_override:
            args += ["+@sSteamCmdForcePlatformType", platform_override]

        # Install directory (반드시 login 전)
        args += ["+force_install_dir", str(install_dir)]

        # Login
        if anonymous:
            args += ["+login", "anonymous"]
        else:
            raise NotImplementedError(
                "Non-anonymous SteamCMD login is not yet supported."
            )

        # App update
        args += ["+app_update", str(app_id)]
        if beta:
            args += [f"-beta {beta}"]
            if beta_password:
                args += [f"-betapassword {beta_password}"]
        if validate:
            args.append("validate")

        args.append("+quit")

        # SteamCMD commonly exits with code 7 on first run (self-update).
        # Retry up to 3 times to handle this.
        max_attempts = 3
        for attempt in range(1, max_attempts + 1):
            _log(f"Running (attempt {attempt}/{max_attempts}): {' '.join(args)}")

            try:
                proc = subprocess.Popen(
                    args,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.STDOUT,
                    text=True,
                    creationflags=_creation_flags(),
                )
                stdout_lines: list[str] = []
                import re as _re
                pct_pattern = _re.compile(r"progress:\s*([\d.]+)\s*\((\d+)\s*/\s*(\d+)\)")
                update_pattern = _re.compile(r"Update state.*?downloading", _re.IGNORECASE)
                last_pct = -1

                for line in iter(proc.stdout.readline, ""):
                    stdout_lines.append(line)
                    stripped = line.strip()
                    if not stripped:
                        continue
                    # Parse SteamCMD progress: "Update state (0x61) downloading, progress: 45.23 (123456789 / 274000000)"
                    m = pct_pattern.search(stripped)
                    if m:
                        pct = min(int(float(m.group(1))), 100)
                        downloaded_bytes = int(m.group(2))
                        total_bytes = int(m.group(3))
                        dl_mb = downloaded_bytes / 1_048_576
                        total_mb = total_bytes / 1_048_576
                        if pct != last_pct:
                            _progress(pct, f"Downloading: {dl_mb:.0f}/{total_mb:.0f} MB ({pct}%)")
                            last_pct = pct
                    elif update_pattern.search(stripped):
                        # SteamCMD state change without explicit percentage
                        _progress(0, "Downloading server files...")

                proc.wait(timeout=600)
                stdout = "".join(stdout_lines)
                returncode = proc.returncode

            except subprocess.TimeoutExpired:
                proc.kill()
                return {
                    "success": False,
                    "message": "SteamCMD timed out after 600 s",
                    "install_dir": str(install_dir),
                    "app_id": app_id,
                }

            if returncode == 0 or "Success! App" in stdout:
                _progress(100, "Server files installed successfully")
                _log(f"SteamCMD install completed for app {app_id}")
                return {
                    "success": True,
                    "message": f"Successfully installed app {app_id} to {install_dir}",
                    "install_dir": str(install_dir),
                    "app_id": app_id,
                }

            # Exit code 7: SteamCMD self-updated or transient failure -- retry
            if returncode == 7 and attempt < max_attempts:
                _log(f"SteamCMD exited with code 7 (attempt {attempt}) -- retrying...")
                import time as _time
                _time.sleep(3)
                continue

            detail = stdout[:2000]
            _log(f"SteamCMD failed for app {app_id}: {detail}")
            return {
                "success": False,
                "message": f"SteamCMD failed (exit code {returncode}): {detail}",
                "install_dir": None,
                "app_id": app_id,
            }

        # Should not reach here, but just in case
        return {
            "success": False,
            "message": "SteamCMD failed after all attempts",
            "install_dir": None,
            "app_id": app_id,
        }

    def update(
        self,
        app_id: int,
        install_dir: str | Path,
        **kwargs: Any,
    ) -> dict[str, Any]:
        """Alias for :meth:`install` — SteamCMD handles incremental updates."""
        return self.install(app_id, install_dir, **kwargs)

    # ── Status ───────────────────────────────────────────────

    def status(self) -> dict[str, Any]:
        """JSON-serialisable status blob for API responses."""
        portable = None
        try:
            portable = str(_portable_dir())
        except EnvironmentError:
            pass

        return {
            "available": self.available,
            "path": str(self._path) if self._path else None,
            "portable_dir": portable,
            "auto_bootstrap": True,
        }


# ── Utilities ────────────────────────────────────────────────────

def _log(msg: str) -> None:
    """Write to stderr so it does not pollute stdout JSON."""
    print(f"[steamcmd] {msg}", file=sys.stderr, flush=True)


def _progress(percent: int, message: str) -> None:
    """Emit a structured progress line on stderr for the Rust daemon to parse."""
    line = json.dumps({"percent": percent, "message": message}, ensure_ascii=True)
    print(f"PROGRESS:{line}", file=sys.stderr, flush=True)


def _creation_flags() -> int:
    """Return subprocess creation flags (Windows: hide console window)."""
    if _SYSTEM == "Windows":
        return 0x08000000  # CREATE_NO_WINDOW
    return 0


# ── CLI entry-point ──────────────────────────────────────────────

def _cli() -> None:
    """``python -m extensions.steamcmd`` — print status JSON."""
    steam = SteamCMD()
    print(json.dumps(steam.status(), indent=2))


# ── Plugin runner interface ──────────────────────────────────────
# Called when the Rust daemon executes:
#   python extensions/steamcmd.py <function> < config.json

def _plugin_ensure(config: dict) -> dict:
    steam = SteamCMD(config.get("explicit_path"))
    steam.ensure_available(timeout=config.get("timeout", 120))
    return steam.status()


def _plugin_install(config: dict) -> dict:
    steam = SteamCMD(config.get("explicit_path"))
    steam.ensure_available()
    return steam.install(
        app_id=config["app_id"],
        install_dir=config["install_dir"],
        anonymous=config.get("anonymous", True),
        beta=config.get("beta"),
        beta_password=config.get("beta_password"),
        platform_override=config.get("platform"),
        validate=config.get("validate", True),
    )


def _plugin_status(config: dict) -> dict:
    steam = SteamCMD(config.get("explicit_path"))
    return steam.status()


FUNCTIONS = {
    "ensure": _plugin_ensure,
    "install": _plugin_install,
    "update": _plugin_install,    # SteamCMD update == install
    "status": _plugin_status,
}


if __name__ == "__main__":
    # Plugin runner protocol: extensions/steamcmd.py <function>
    # Config JSON on stdin.
    if len(sys.argv) < 2:
        _cli()
        sys.exit(0)

    function_name = sys.argv[1]
    fn = FUNCTIONS.get(function_name)
    if fn is None:
        result = {"success": False, "message": f"Unknown function: {function_name}"}
    else:
        try:
            input_data = sys.stdin.read()
            config_data = json.loads(input_data) if input_data.strip() else {}
            result = fn(config_data)
        except Exception as e:
            result = {"success": False, "message": str(e)}

    json.dump(result, sys.stdout)
