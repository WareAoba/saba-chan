"""Tauri GUI를 W3C WebDriver로 조작한다: tauri-driver(리눅스 WebKitWebDriver, Windows msedgedriver)를 띄우고 그 위에 세션을 연다.

화면 조작은 접근성 이름(aria-label·글자)으로 찾는 작은 JS로 한다. 사람이 보는 이름과 같은 것으로 누른다.
"""

from __future__ import annotations

import base64
import shutil
import subprocess
import time
from pathlib import Path
from typing import Any

import requests

from . import procs
from .daemon import E2EError
from .paths import IS_WINDOWS
from .sandbox import free_port

ENTER = ""

# 이름이 같은 보이는 요소 하나 (button·a·[role=button]·label). exact가 아니면 포함
_FIND = """
const [name, exact, kinds] = arguments;
const norm = (s) => (s || '').replace(/\\s+/g, ' ').trim();
const visible = (el) => !!(el.offsetWidth || el.offsetHeight || el.getClientRects().length);
for (const el of document.querySelectorAll(kinds)) {
  if (!visible(el)) continue;
  const names = [el.getAttribute('aria-label'), el.getAttribute('title'), el.innerText].map(norm);
  if (names.some((n) => n && (exact ? n === norm(name) : n.includes(norm(name))))) return el;
}
return null;
"""


def tauri_driver() -> str | None:
    """tauri-driver 실행 파일 (없으면 None)"""
    return shutil.which("tauri-driver")


class Gui:
    def __init__(self, app: Path, env: dict[str, str], log: Path):
        port, native = free_port(), free_port()
        self._log = open(log, "w", encoding="utf-8")
        cmd = [tauri_driver(), "--port", str(port), "--native-port", str(native)]
        self.proc = subprocess.Popen(cmd, env=env, stdout=self._log, stderr=subprocess.STDOUT, start_new_session=not IS_WINDOWS)
        self.base = f"http://127.0.0.1:{port}"
        self.sid: str | None = None
        deadline = time.time() + 30
        while True:
            try:
                if requests.get(f"{self.base}/status", timeout=2).ok:
                    break
            except requests.RequestException:
                pass
            if time.time() > deadline or self.proc.poll() is not None:
                self.close()
                raise E2EError(f"tauri-driver가 뜨지 않음 (로그 {log})")
            time.sleep(0.2)
        caps = {"capabilities": {"alwaysMatch": {"tauri:options": {"application": str(app)}}}}
        r = requests.post(f"{self.base}/session", json=caps, timeout=120)
        if not r.ok:
            self.close()
            raise E2EError(f"GUI 세션을 열지 못함: {r.status_code} {r.text[:500]}")
        self.sid = r.json()["value"]["sessionId"]

    def _call(self, method: str, path: str, body: dict | None = None, timeout: float = 60) -> Any:
        r = requests.request(method, f"{self.base}/session/{self.sid}{path}", json=body, timeout=timeout)
        value = r.json().get("value") if r.content else None
        if not r.ok:
            raise E2EError(f"WebDriver {method} {path}: {r.status_code} {value}")
        return value

    def js(self, script: str, *args: Any) -> Any:
        return self._call("POST", "/execute/sync", {"script": script, "args": list(args)})

    def text(self) -> str:
        return self.js("return document.body ? document.body.innerText : ''") or ""

    def wait(self, cond, timeout: float, what: str):
        deadline = time.time() + timeout
        last: Exception | None = None
        while time.time() < deadline:
            try:
                got = cond()
                if got:
                    return got
            except E2EError as e:
                last = e
            time.sleep(0.25)
        raise E2EError(f"{timeout:.0f}초 안에 화면에 나오지 않음: {what}" + (f" ({last})" if last else ""))

    def wait_text(self, needle: str, timeout: float = 60) -> None:
        self.wait(lambda: needle in self.text(), timeout, needle)

    def wait_gone(self, selector: str, timeout: float = 60) -> None:
        self.wait(lambda: not self.js("return !!document.querySelector(arguments[0])", selector), timeout, f"{selector} 사라짐")

    def find(self, name: str, exact: bool = True, kinds: str = "button, a, [role=button], label") -> dict | None:
        return self.js(_FIND, name, exact, kinds)

    def click(self, name: str, exact: bool = True, timeout: float = 30, kinds: str = "button, a, [role=button], label") -> None:
        el = self.wait(lambda: self.find(name, exact, kinds), timeout, f"[{name}]")
        self.js("arguments[0].click()", el)

    def enabled(self, name: str, exact: bool = True) -> bool:
        el = self.find(name, exact)
        return bool(el) and not self.js("return arguments[0].disabled || arguments[0].getAttribute('aria-disabled') === 'true'", el)

    def element(self, css: str) -> str:
        """CSS로 찾은 요소의 WebDriver id (키 입력용)"""
        found = self._call("POST", "/element", {"using": "css selector", "value": css})
        return next(iter(found.values()))

    def type(self, css: str, text: str) -> None:
        el = self.element(css)
        self._call("POST", f"/element/{el}/value", {"text": text})

    def fill(self, css: str, text: str) -> None:
        """입력칸을 비우고 새로 친다"""
        el = self.element(css)
        self._call("POST", f"/element/{el}/clear", {})
        self._call("POST", f"/element/{el}/value", {"text": text})

    def handles(self) -> list[str]:
        return self._call("GET", "/window/handles")

    def switch(self, handle: str) -> None:
        self._call("POST", "/window", {"handle": handle})

    def screenshot(self, path: Path) -> None:
        try:
            path.write_bytes(base64.b64decode(self._call("GET", "/screenshot")))
        except Exception:  # 스크린샷은 증거일 뿐 실패 판정에 쓰지 않는다
            pass

    def snap(self, art: Path, name: str) -> None:
        art.mkdir(parents=True, exist_ok=True)
        self.screenshot(art / f"{name}.png")
        try:
            (art / f"{name}.txt").write_text(self.text(), encoding="utf-8")
        except E2EError:
            pass

    def close(self) -> None:
        if self.sid:
            try:
                requests.delete(f"{self.base}/session/{self.sid}", timeout=30)
            except requests.RequestException:
                pass
            self.sid = None
        if self.proc.poll() is None:
            procs.kill_group(self.proc.pid)
            try:
                self.proc.wait(30)
            except subprocess.TimeoutExpired:
                pass
        self._log.close()
