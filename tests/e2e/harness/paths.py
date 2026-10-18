"""저장소·바이너리·시험 루트 경로. 환경 변수로 바꿀 수 있다.

- SABA_E2E_ROOT: 시험 루트 (기본: Windows는 C:\\SabaE2E, 그 밖은 시스템 임시 폴더/saba-e2e. PC 시험은 이 폴더 밖에 쓰지 않는다)
- SABA_E2E_BIN_DIR: saba-core·saba-runner·saba-chan-cli가 있는 폴더 (기본: target/debug)
- SABA_E2E_MODULES_REPO / SABA_E2E_EXTENSIONS_REPO / SABA_E2E_SERVER_CHAN_REPO: 형제 저장소 체크아웃
"""

import os
import sys
import tempfile
from pathlib import Path

IS_WINDOWS = sys.platform == "win32"
EXE = ".exe" if IS_WINDOWS else ""

E2E_DIR = Path(__file__).resolve().parent.parent
REPO = E2E_DIR.parent.parent
FIXTURES = E2E_DIR / "fixtures"


def _env_path(name: str, default: Path) -> Path:
    value = os.environ.get(name)
    return Path(value).resolve() if value else default


ROOT = _env_path("SABA_E2E_ROOT", Path("C:/SabaE2E") if IS_WINDOWS else Path(tempfile.gettempdir()) / "saba-e2e")
BIN_DIR = _env_path("SABA_E2E_BIN_DIR", REPO / "target" / "debug")
MODULES_REPO = _env_path("SABA_E2E_MODULES_REPO", REPO.parent / "saba-chan-modules")
EXTENSIONS_REPO = _env_path("SABA_E2E_EXTENSIONS_REPO", REPO.parent / "saba-chan-extensions")
SERVER_CHAN_REPO = _env_path("SABA_E2E_SERVER_CHAN_REPO", REPO.parent / "server-chan")


def binary(name: str) -> Path:
    return BIN_DIR / f"{name}{EXE}"


def real_data_dirs() -> list[Path]:
    """사용자의 실제 데이터 폴더. 시험이 이곳을 바꾸면 안 된다."""
    home = Path.home()
    dirs = [home / ".config" / "saba-chan", home / ".local" / "share" / "saba-chan", home / ".cache" / "saba-chan"]
    for var in ("APPDATA", "LOCALAPPDATA"):
        if os.environ.get(var):
            dirs.append(Path(os.environ[var]) / "saba-chan")
    return dirs
