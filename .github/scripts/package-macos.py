#!/usr/bin/env python3
"""맥 앱 묶음(Saba-chan.app)과 서명 없는 .dmg를 만든다.

    python .github/scripts/package-macos.py --bin target/release --out output --version 0.1.12 [--platform macos-arm64]

구성요소 zip은 리눅스와 같은 package-linux.py로 만든다(`--platform macos-arm64`). 이 스크립트는 그 실행 파일을 한 묶음에 넣는다:
.dmg를 받아 응용 프로그램 폴더에 끌어 넣는 설치용이다. 설치기도 같은 모양(`Contents/MacOS`에 모든 실행 파일)으로 묶음을 만든다.
Apple 서명·공증 없이 임시(ad-hoc) 서명만 한다. 서명과 .dmg는 맥 도구(codesign, actool, dmgbuild)가 있어야 만든다.
"""

import argparse
import importlib.util
import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

APP_NAME = "Saba-chan.app"
EXES = ["saba-chan-gui", "saba-core", "saba-runner", "saba-chan-cli", "saba-chan-installer"]
# 설치기도 같은 두 파일로 묶음을 만든다 (Info.plist의 @VERSION@·@BUILD@를 바꿔 쓴다)
PLIST_TEMPLATE = ROOT / "saba-chan-gui/src-tauri/macos/Info.plist"
# 아이콘 원본은 Icon Composer 묶음 하나다. actool(Xcode 26 이상)이 macOS 26의 Assets.car(라이트·다크·틴트)와
# 옛 macOS용 icns로 컴파일한다. Info.plist의 CFBundleIconName(AppIcon)·CFBundleIconFile이 두 파일을 가리킨다
ICON_SOURCE = ROOT / "saba-chan-gui/src-tauri/icons/AppIcon.icon"
ICON_FILE = "saba-chan.icns"
ASSETS_FILE = "Assets.car"
# .dmg에 함께 넣는 첫 실행 안내 (서명이 없어 macOS가 처음 한 번 막는다). 문구는 locales/*/common.json `macos_first_open`
FIRST_OPEN_NOTE = "처음 열 때 (First launch).txt"
# .dmg Finder 창 (pt): 배경의 화살표를 따라 왼쪽 앱을 오른쪽 응용 프로그램 폴더로 끌어 넣는다.
# 배경은 660×400 그림과 레티나용 @2x 한 쌍이다
DMG_BACKGROUND = ROOT / "saba-chan-gui/src-tauri/macos/dmg-background.png"
DMG_WINDOW = (660, 400)
DMG_ICON_SIZE = 112
# 안내의 언어 순서: 한국어, 영어, 나머지는 이름순
FIRST_LANGUAGES = ["ko", "en"]


def shared_files() -> dict[Path, str]:
    """데몬 zip(saba-core)에 함께 드는 공유 모듈 파일 → 묶음 안 이름. 목록은 package-linux.py 하나에서 정한다"""
    spec = importlib.util.spec_from_file_location("package_linux", Path(__file__).resolve().parent / "package-linux.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return {ROOT / rel: name for rel, name in module.LINUX_ASSETS["saba-core"][2].items()}


def first_open_note() -> str:
    """모든 언어의 첫 실행 안내를 한 파일로 (한국어 먼저)"""
    root = ROOT / "locales"
    langs = sorted(p.name for p in root.iterdir() if (p / "common.json").is_file())
    langs = [l for l in FIRST_LANGUAGES if l in langs] + [l for l in langs if l not in FIRST_LANGUAGES]
    parts = []
    for lang in langs:
        text = json.loads((root / lang / "common.json").read_text(encoding="utf-8"))["macos_first_open"]
        parts.append(f"{text['title']}\n\n{text['steps']}\n")
    return "\n----------------------------------------\n\n".join(parts)


def app_icons() -> dict[str, bytes]:
    """아이콘 원본을 컴파일한 `Contents/Resources` 파일 (이름 → 내용). 맥에서만 된다"""
    with tempfile.TemporaryDirectory() as d:
        out = Path(d)
        subprocess.run(["xcrun", "actool", str(ICON_SOURCE), "--compile", str(out), "--platform", "macosx",
                        "--minimum-deployment-target", "11.0", "--app-icon", "AppIcon",
                        "--output-partial-info-plist", str(out / "partial.plist")], check=True, stdout=subprocess.DEVNULL)
        return {ICON_FILE: (out / "AppIcon.icns").read_bytes(), ASSETS_FILE: (out / ASSETS_FILE).read_bytes()}


def info_plist(version: str) -> bytes:
    # CFBundleVersion은 숫자와 점만 받는다: 미리보기 꼬리(-beta.1)는 짧은 버전 문자열에만 둔다
    text = PLIST_TEMPLATE.read_text(encoding="utf-8")
    return text.replace("@VERSION@", version).replace("@BUILD@", version.split("-", 1)[0]).encode("utf-8")


def build_app(bin_dir: Path, out_dir: Path, version: str, icons: dict[str, bytes]) -> Path:
    """실행 파일과 공유 모듈 파일을 `Contents/MacOS`에, Info.plist와 아이콘([`app_icons`])을 넣는다 (서명은 [`finish_app`])"""
    missing = [e for e in EXES if not (bin_dir / e).is_file()]
    if missing:
        sys.exit(f"{APP_NAME}: {', '.join(missing)} not built in {bin_dir}")
    app = out_dir / APP_NAME
    if app.exists():
        shutil.rmtree(app)
    macos = app / "Contents" / "MacOS"
    macos.mkdir(parents=True)
    (app / "Contents" / "Resources").mkdir()
    for exe in EXES:
        shutil.copy2(bin_dir / exe, macos / exe)
        (macos / exe).chmod(0o755)
    for src, name in shared_files().items():
        shutil.copy2(src, macos / name)
        (macos / name).chmod(0o644)
    (app / "Contents" / "Info.plist").write_bytes(info_plist(version))
    for name, data in icons.items():
        (app / "Contents" / "Resources" / name).write_bytes(data)
    return app


def finish_app(app: Path) -> None:
    # M칩 맥은 서명 없는 실행 파일을 돌리지 않는다: 개발자 인증서 없이 묶음 전체에 임시 서명을 한다
    subprocess.run(["codesign", "--force", "--deep", "--sign", "-", str(app)], check=True)
    subprocess.run(["codesign", "--verify", "--deep", "--strict", str(app)], check=True)


def dmg_settings(app: Path, work: Path) -> dict:
    """dmgbuild 설정: 앱 묶음, 응용 프로그램 폴더 바로가기, 첫 실행 안내(`work`에 쓴다)와 꾸민 Finder 창"""
    note = work / FIRST_OPEN_NOTE
    note.write_text(first_open_note(), encoding="utf-8")
    width, height = DMG_WINDOW
    return {
        "format": "UDZO",
        "files": [str(app), str(note)],
        "symlinks": {"Applications": "/Applications"},
        # 마운트한 디스크도 앱 아이콘으로 보인다
        "icon": str(app / "Contents" / "Resources" / ICON_FILE),
        "background": str(DMG_BACKGROUND),
        "window_rect": ((200, 160), (width, height)),
        "default_view": "icon-view",
        "show_toolbar": False, "show_sidebar": False, "show_status_bar": False, "show_tab_view": False,
        "show_pathbar": False, "show_icon_preview": False,
        "icon_size": DMG_ICON_SIZE,
        "text_size": 12,
        "icon_locations": {app.name: (150, 185), "Applications": (510, 185), FIRST_OPEN_NOTE: (330, 330)},
    }


def make_dmg(app: Path, dest: Path) -> None:
    """[`dmg_settings`]로 압축 .dmg를 만든다 (dmgbuild: Finder를 띄우지 않고 창 배치를 기록한다)"""
    import dmgbuild

    with tempfile.TemporaryDirectory() as d:
        if dest.exists():
            dest.unlink()
        dmgbuild.build_dmg(str(dest), "Saba-chan", settings=dmg_settings(app, Path(d)))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--bin", required=True, help="cargo build --release 출력 폴더")
    parser.add_argument("--out", required=True)
    parser.add_argument("--version", required=True, help="versions.toml [release] version")
    parser.add_argument("--platform", default="macos-arm64")
    args = parser.parse_args()
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as d:
        app = build_app(Path(args.bin), Path(d), args.version, app_icons())
        finish_app(app)
        dmg = out / f"Saba-chan-{args.platform}.dmg"
        make_dmg(app, dmg)
    print(f"    → {dmg.name} ({dmg.stat().st_size / 1024 / 1024:.2f} MB)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
