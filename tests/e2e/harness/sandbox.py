"""시험 하나가 쓰는 격리 폴더와 환경 변수.

데몬·CLI·GUI·모듈 Python·SteamCMD가 읽는 경로를 전부 이 폴더 안으로 돌린다.
steamcmd 익스텐션은 HOME 아래(~/.config/saba-chan/steamcmd)를 쓰고, Python 런타임은 exe 옆을 쓸 수 있으며,
Electron은 APPDATA를 본다. 그래서 SABA_* 외에 HOME·APPDATA·LOCALAPPDATA·TEMP·XDG_*도 바꾼다.
"""

import os
import shutil
import socket
from pathlib import Path

from .paths import EXTENSIONS_REPO, IS_WINDOWS, MODULES_REPO

# 시험 환경에 새면 결과가 흔들리는 변수 (예: 클라우드 프록시의 JAVA_TOOL_OPTIONS는 콘솔 첫 줄을 더럽힌다)
_DROP_ENV = ("JAVA_TOOL_OPTIONS", "SABA_IPC_URL", "SABA_PYTHON", "IPC_BASE", "RUST_LOG")


def free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def free_ports(n: int, step: int = 1) -> list[int]:
    """연속하지 않아도 되는 빈 포트 n개. 게임 포트는 +1(쿼리) 등을 쓰므로 step만큼 비었는지도 본다."""
    ports: list[int] = []
    while len(ports) < n:
        p = free_port()
        if all(abs(p - q) > step for q in ports) and all(_port_free(p + k) for k in range(step + 1)):
            ports.append(p)
    return ports


def _port_free(port: int) -> bool:
    with socket.socket() as s:
        try:
            s.bind(("0.0.0.0", port))
            return True
        except OSError:
            return False


def _copy_repo(src: Path, dest: Path) -> None:
    if not src.is_dir():
        raise RuntimeError(f"저장소 체크아웃이 없습니다: {src} (SABA_E2E_*_REPO로 지정)")
    shutil.copytree(src, dest, ignore=shutil.ignore_patterns(".git", "__pycache__", ".pytest_cache", "node_modules"))


class Sandbox:
    def __init__(self, root: Path, *, modules: bool = True, extensions: bool = True):
        self.root = root
        self.home = root / "home"
        self.data = root / "data"
        self.tmp = root / "tmp"
        self.modules = root / "modules"
        # 익스텐션 폴더의 부모에는 데몬이 격리 실행(Windows)에 목록 권한을 준다. 시험 폴더 루트는 HOME의 위라
        # 데몬이 주지 않으므로 한 단계 아래에 둔다
        self.extensions = root / "plugins" / "extensions"
        self.servers = root / "servers"  # 가져오기용 기존 서버 폴더
        self.artifacts = root / "artifacts"
        for d in (self.home, self.data, self.tmp, self.servers, self.artifacts):
            d.mkdir(parents=True, exist_ok=True)
        if modules:
            _copy_repo(MODULES_REPO, self.modules)
        else:
            self.modules.mkdir(exist_ok=True)
        if extensions:
            _copy_repo(EXTENSIONS_REPO, self.extensions)
        else:
            self.extensions.mkdir(parents=True, exist_ok=True)
        self.ipc_port = free_port()

    @property
    def token_path(self) -> Path:
        return self.data / ".ipc_token"

    def token(self) -> str:
        return self.token_path.read_text().strip()

    def env(self, **extra: str) -> dict[str, str]:
        env = {k: v for k, v in os.environ.items() if k not in _DROP_ENV}
        env.update(
            SABA_DATA_DIR=str(self.data),
            SABA_MODULES_PATH=str(self.modules),
            SABA_EXTENSIONS_DIR=str(self.extensions),
            SABA_TOKEN_PATH=str(self.token_path),
            SABA_IPC_PORT=str(self.ipc_port),
            HOME=str(self.home),
            USERPROFILE=str(self.home) if IS_WINDOWS else env.get("USERPROFILE", ""),
            TMPDIR=str(self.tmp),
            TEMP=str(self.tmp),
            TMP=str(self.tmp),
            XDG_CONFIG_HOME=str(self.home / ".config"),
            XDG_DATA_HOME=str(self.home / ".local" / "share"),
            XDG_CACHE_HOME=str(self.home / ".cache"),
            RUST_LOG="info",
            NO_COLOR="1",
        )
        if IS_WINDOWS:
            env["APPDATA"] = str(self.home / "AppData" / "Roaming")
            env["LOCALAPPDATA"] = str(self.home / "AppData" / "Local")
            Path(env["APPDATA"]).mkdir(parents=True, exist_ok=True)
            Path(env["LOCALAPPDATA"]).mkdir(parents=True, exist_ok=True)
        env.update(extra)
        return env

    def write_config(self, patch: dict) -> None:
        """데몬이 뜨기 전에 config.json에 섹션을 합친다 (예: relay URL, 업데이트 relay_url)."""
        import json

        path = self.data / "config.json"
        current = json.loads(path.read_text()) if path.exists() else {}
        for key, value in patch.items():
            if isinstance(value, dict) and isinstance(current.get(key), dict):
                current[key].update(value)
            else:
                current[key] = value
        path.write_text(json.dumps(current, ensure_ascii=False, indent=2))
