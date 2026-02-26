"""Music extension — dependency checker & installer.

사바쨩 익스텐션 시스템에서 음악 봇에 필요한 외부 의존성을 검증하고,
Node.js npm 패키지가 미설치일 경우 자동 설치를 시도합니다.
daemon.startup 훅으로 실행됩니다.

의존성 구조:
  extensions/music/package.json   — npm 의존성 정의 (@discordjs/voice 등)
  extensions/music/node_modules/  — npm install 결과물
  시스템 PATH                     — ffmpeg, yt-dlp

실제 음악 재생 로직은 discord_bot/extensions/music.js (Node.js) 에 있습니다.
"""

import ctypes
import json
import logging
import os
import shutil
import subprocess
import sys
from pathlib import Path

logger = logging.getLogger("saba.ext.music")

# 이 파일이 위치한 extensions/music/ 디렉토리
_EXT_DIR = Path(__file__).resolve().parent


def check_dependencies(**_kwargs):
    """daemon.startup hook — npm 패키지 + ffmpeg / yt-dlp / opus 의존성 검사.

    npm 의존성이 미설치이면 자동으로 ``npm install`` 을 시도합니다.

    Returns:
        dict: 검사 결과. ``status`` 가 ``"ready"`` 이면 모든 의존성 확인됨.
    """
    # 1. Node.js npm 의존성 확인 & 자동 설치
    npm_result = _ensure_npm_dependencies()

    # 2. 시스템 의존성 확인
    results = {
        "npm_packages": npm_result,
        "ffmpeg": _check_ffmpeg(),
        "yt_dlp": _check_yt_dlp(),
        "opus": _check_opus(),
    }

    all_ok = all(r["available"] for r in results.values())

    if all_ok:
        logger.info("Music dependencies OK: npm packages, ffmpeg, yt-dlp, opus all available")
    else:
        missing = [k for k, v in results.items() if not v["available"]]
        logger.warning("Music dependencies missing: %s", ", ".join(missing))

    return {
        "status": "ready" if all_ok else "degraded",
        "dependencies": results,
    }


# ──────────────────────────────────────────────────────────────
#  npm 의존성 관리
# ──────────────────────────────────────────────────────────────


def _ensure_npm_dependencies():
    """extensions/music/node_modules 확인 & 필요 시 npm install 실행.

    package.json은 이미 extensions/music/ 에 존재한다고 가정합니다.
    """
    node_modules = _EXT_DIR / "node_modules"
    package_json = _EXT_DIR / "package.json"

    if not package_json.exists():
        return {
            "available": False,
            "error": "package.json not found in extensions/music/",
        }

    # 핵심 패키지 존재 여부로 설치 상태 판단
    voice_marker = node_modules / "@discordjs" / "voice" / "package.json"
    if voice_marker.exists():
        return {"available": True, "installed": True}

    # 미설치 → npm install 시도
    logger.info("Music npm dependencies not found, running npm install...")
    npm_cmd = shutil.which("npm")
    if not npm_cmd:
        return {
            "available": False,
            "error": "npm not found in PATH — cannot auto-install music dependencies",
        }

    try:
        result = subprocess.run(
            [npm_cmd, "install", "--omit=dev", "--no-fund", "--no-audit"],
            cwd=str(_EXT_DIR),
            capture_output=True,
            text=True,
            timeout=120,
        )
        if result.returncode == 0:
            logger.info("Music npm dependencies installed successfully")
            return {"available": True, "installed": True, "auto_installed": True}
        else:
            stderr = result.stderr.strip()[:500] if result.stderr else ""
            logger.error("npm install failed (exit %d): %s", result.returncode, stderr)
            return {
                "available": False,
                "error": f"npm install failed (exit {result.returncode})",
                "stderr": stderr,
            }
    except subprocess.TimeoutExpired:
        logger.error("npm install timed out (120s)")
        return {"available": False, "error": "npm install timed out"}
    except Exception as exc:
        logger.error("npm install error: %s", exc)
        return {"available": False, "error": str(exc)}


# ──────────────────────────────────────────────────────────────
#  개별 시스템 의존성 검사
# ──────────────────────────────────────────────────────────────


def _check_ffmpeg():
    """ffmpeg 사용 가능 여부 확인."""
    path = shutil.which("ffmpeg")
    if path:
        try:
            result = subprocess.run(
                [path, "-version"],
                capture_output=True,
                text=True,
                timeout=5,
            )
            version_line = (
                result.stdout.split("\n")[0] if result.stdout else "unknown"
            )
            return {"available": True, "path": path, "version": version_line}
        except Exception as exc:
            return {"available": False, "error": str(exc)}

    return {"available": False, "error": "ffmpeg not found in PATH"}


def _check_yt_dlp():
    """yt-dlp 사용 가능 여부 확인."""
    path = shutil.which("yt-dlp")
    if path:
        try:
            result = subprocess.run(
                [path, "--version"],
                capture_output=True,
                text=True,
                timeout=5,
            )
            version = result.stdout.strip() if result.stdout else "unknown"
            return {"available": True, "path": path, "version": version}
        except Exception as exc:
            return {"available": False, "error": str(exc)}

    return {
        "available": False,
        "error": "yt-dlp not found in PATH. Install: pip install yt-dlp",
    }


def _check_opus():
    """Opus 라이브러리 사용 가능 여부 확인.

    시스템 공유 라이브러리를 탐색합니다.
    찾지 못해도 Node.js 의 opusscript (WASM) 가 대체 가능하므로
    ``available: True`` (fallback) 로 반환합니다.
    """
    if sys.platform == "win32":
        lib_names = ["opus", "libopus-0", "libopus"]
    elif sys.platform == "darwin":
        lib_names = ["libopus.dylib", "libopus.0.dylib"]
    else:
        lib_names = ["libopus.so", "libopus.so.0"]

    for name in lib_names:
        try:
            ctypes.cdll.LoadLibrary(name)
            return {"available": True, "library": name}
        except OSError:
            continue

    # 시스템 opus 가 없어도 Node.js opusscript(WASM) 가 대체 가능
    return {
        "available": True,
        "library": "opusscript (Node.js fallback)",
        "note": "System opus not found — Node.js opusscript/wasm should work",
    }


# ──────────────────────────────────────────────────────────────
#  Plugin runner entry-point
# ──────────────────────────────────────────────────────────────

_FUNCTIONS = {
    "check_dependencies": check_dependencies,
}


def main():
    if len(sys.argv) < 2:
        print(json.dumps({"error": "No function specified"}))
        sys.exit(1)

    func_name = sys.argv[1]
    func = _FUNCTIONS.get(func_name)
    if not func:
        print(json.dumps({"error": f"Unknown function: {func_name}"}))
        sys.exit(1)

    config_str = sys.stdin.read()
    try:
        config = json.loads(config_str) if config_str.strip() else {}
    except json.JSONDecodeError as e:
        print(json.dumps({"error": f"Invalid JSON config: {e}"}))
        sys.exit(1)

    result = func(**config)
    print(json.dumps(result))


if __name__ == "__main__":
    main()
