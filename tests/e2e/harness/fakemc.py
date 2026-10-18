"""가짜 마인크래프트 서버 jar(fixtures/fake-minecraft)를 빌드하고 서버 폴더를 만든다."""

import hashlib
import shutil
import subprocess
import tempfile
from pathlib import Path

from .paths import FIXTURES, ROOT

SOURCE = FIXTURES / "fake-minecraft" / "FakeMinecraft.java"


def build_jar() -> Path:
    """소스 해시로 캐시한다. javac·jar(JDK 17 이상)가 PATH에 있어야 한다."""
    digest = hashlib.sha256(SOURCE.read_bytes()).hexdigest()[:12]
    out = ROOT / "cache" / f"fake-minecraft-{digest}.jar"
    if out.exists():
        return out
    out.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        subprocess.run(["javac", "--release", "17", "-d", str(tmp), str(SOURCE)], check=True, capture_output=True)
        (tmp / "manifest.txt").write_text("Main-Class: FakeMinecraft\n")
        subprocess.run(["jar", "cfm", str(tmp / "server.jar"), str(tmp / "manifest.txt"), "-C", str(tmp), "FakeMinecraft.class"], check=True, capture_output=True)
        shutil.copy(tmp / "server.jar", out)
    return out


def make_server(folder: Path, port: int, *, eula: bool = True, version: str = "1.21.1", max_players: int = 7) -> Path:
    folder.mkdir(parents=True, exist_ok=True)
    shutil.copy(build_jar(), folder / "server.jar")
    (folder / "server.properties").write_text(f"server-port={port}\nmax-players={max_players}\nmotd=e2e\nenable-rcon=false\n")
    (folder / "fake-version.txt").write_text(version)
    if eula:
        (folder / "eula.txt").write_text("eula=true\n")
    return folder
