"""saba-chan-cli를 실제 바이너리로 실행한다. TTY가 없으므로 확인 질문은 --yes로 넘긴다."""

import json
import subprocess
from dataclasses import dataclass
from typing import Any

from .paths import binary
from .sandbox import Sandbox


@dataclass
class CliResult:
    args: list[str]
    code: int
    stdout: str
    stderr: str

    def json(self) -> Any:
        return json.loads(self.stdout)

    def __str__(self) -> str:
        return f"saba-chan-cli {' '.join(self.args)} → {self.code}\n[stdout]\n{self.stdout[-2000:]}\n[stderr]\n{self.stderr[-2000:]}"


class Cli:
    def __init__(self, sandbox: Sandbox, lang: str = "ko"):
        self.sb = sandbox
        self.lang = lang

    def run(self, *args: str, timeout: float = 180, json_out: bool = True, check: int | None = None) -> CliResult:
        argv = [*(["--json"] if json_out else []), "--lang", self.lang, "--no-color", *args]
        proc = subprocess.run(
            [str(binary("saba-chan-cli")), *argv],
            env=self.sb.env(),
            cwd=self.sb.root,
            stdin=subprocess.DEVNULL,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=timeout,
        )
        result = CliResult(argv, proc.returncode, proc.stdout, proc.stderr)
        log = self.sb.artifacts / "cli.log"
        with open(log, "a", encoding="utf-8") as f:
            f.write(str(result) + "\n\n")
        if check is not None and result.code != check:
            raise AssertionError(f"종료 코드 {check}을 기대함\n{result}")
        return result
