#!/usr/bin/env python3
"""맥 앱 묶음(Saba-chan.app)과 서명 없는 .dmg를 만든다.

    python .github/scripts/package-macos.py --bin target/release --out output --version 0.1.12 [--platform macos-arm64]

구성요소 zip은 리눅스와 같은 package-linux.py로 만든다(`--platform macos-arm64`). 이 스크립트는 그 실행 파일을 한 묶음에 넣는다:
.dmg를 받아 응용 프로그램 폴더에 끌어 넣는 설치용이다. 설치기도 같은 모양(`Contents/MacOS`에 모든 실행 파일)으로 묶음을 만든다.
Apple 서명·공증 없이 임시(ad-hoc) 서명만 한다. 아이콘·서명·.dmg는 맥 도구(sips, iconutil, codesign, hdiutil)가 있어야 만든다.
"""

import argparse
import plistlib
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

APP_NAME = "Saba-chan.app"
# GUI의 Tauri 식별자와 같다 (saba-chan-gui/src-tauri/tauri.conf.json, updater notify_app_id::GUI_APP_ID)
BUNDLE_ID = "com.saba-chan.app"
MAIN_EXE = "saba-chan-gui"
EXES = ["saba-chan-gui", "saba-core", "saba-runner", "saba-chan-cli", "saba-chan-updater"]
ICON_SOURCE = ROOT / "saba-chan-gui/src-tauri/icons/icon.png"
ICON_FILE = "saba-chan.icns"


def info_plist(version: str) -> dict:
    # CFBundleVersion은 숫자와 점만 받는다: 미리보기 꼬리(-beta.1)는 짧은 버전 문자열에만 둔다
    numeric = version.split("-", 1)[0]
    return {
        "CFBundleDevelopmentRegion": "ko",
        "CFBundleDisplayName": "Saba-chan",
        "CFBundleExecutable": MAIN_EXE,
        "CFBundleIconFile": ICON_FILE,
        "CFBundleIdentifier": BUNDLE_ID,
        "CFBundleInfoDictionaryVersion": "6.0",
        "CFBundleName": "Saba-chan",
        "CFBundlePackageType": "APPL",
        "CFBundleShortVersionString": version,
        "CFBundleVersion": numeric,
        "LSApplicationCategoryType": "public.app-category.utilities",
        "LSMinimumSystemVersion": "11.0",
        "NSHighResolutionCapable": True,
    }


def build_app(bin_dir: Path, out_dir: Path, version: str) -> Path:
    """실행 파일을 `Contents/MacOS`에, Info.plist를 `Contents`에 넣는다 (아이콘·서명은 [`finish_app`])"""
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
    with open(app / "Contents" / "Info.plist", "wb") as f:
        plistlib.dump(info_plist(version), f)
    return app


def make_icns(png: Path, dest: Path) -> None:
    with tempfile.TemporaryDirectory() as d:
        iconset = Path(d) / "saba-chan.iconset"
        iconset.mkdir()
        for size in (16, 32, 128, 256, 512):
            for scale in (1, 2):
                name = f"icon_{size}x{size}{'@2x' if scale == 2 else ''}.png"
                px = str(size * scale)
                subprocess.run(["sips", "-z", px, px, str(png), "--out", str(iconset / name)], check=True, capture_output=True)
        subprocess.run(["iconutil", "-c", "icns", str(iconset), "-o", str(dest)], check=True)


def finish_app(app: Path) -> None:
    make_icns(ICON_SOURCE, app / "Contents" / "Resources" / ICON_FILE)
    # M칩 맥은 서명 없는 실행 파일을 돌리지 않는다: 개발자 인증서 없이 묶음 전체에 임시 서명을 한다
    subprocess.run(["codesign", "--force", "--deep", "--sign", "-", str(app)], check=True)
    subprocess.run(["codesign", "--verify", "--deep", "--strict", str(app)], check=True)


def make_dmg(app: Path, dest: Path) -> None:
    """앱 묶음과 응용 프로그램 폴더 바로가기를 담은 압축 .dmg"""
    with tempfile.TemporaryDirectory() as d:
        stage = Path(d) / "Saba-chan"
        stage.mkdir()
        subprocess.run(["cp", "-R", str(app), str(stage)], check=True)
        (stage / "Applications").symlink_to("/Applications")
        if dest.exists():
            dest.unlink()
        subprocess.run(["hdiutil", "create", "-volname", "Saba-chan", "-srcfolder", str(stage),
                        "-ov", "-format", "UDZO", str(dest)], check=True)


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
        app = build_app(Path(args.bin), Path(d), args.version)
        finish_app(app)
        dmg = out / f"Saba-chan-{args.platform}.dmg"
        make_dmg(app, dmg)
    print(f"    → {dmg.name} ({dmg.stat().st_size / 1024 / 1024:.2f} MB)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
