#!/usr/bin/env python3
"""리눅스·맥 설치용 구성요소 zip을 만든다 (D3c). 맥은 `--platform macos-arm64`, 앱 묶음·.dmg는 package-macos.py.

    python .github/scripts/package-linux.py --bin target/release --headless-bin target/headless/release --out output
        [--platform linux-x64] [--only saba-core,cli,installer.headless]

Windows zip과 같은 이름 규칙(`<컴포넌트>-<플랫폼>.zip`)과 같은 내용(saba-core zip에 saba-runner와 공유 모듈)이다.
실행 파일은 실행 권한(0755)을 zip에 적는다: 설치기가 그 비트를 보고 실행 권한을 준다.
GUI zip은 데스크톱 구성(화면 있는 리눅스)이 받는다.
설치기(설치·제거·업데이트 적용)는 두 번 빌드한 같은 프로그램이다: `--bin`의 창 빌드는 데스크톱 구성(`installer.desktop`),
`--headless-bin`의 창 없는 빌드(`--no-default-features`, GTK·WebKit 없이 돈다)는 헤드리스 구성(`installer.headless`)이 받는다.
"""

import argparse
import importlib.util
import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def _mac():
    """앱 묶음 파일(Info.plist 틀·아이콘)은 package-macos.py 하나에서 정한다"""
    spec = importlib.util.spec_from_file_location("package_macos", Path(__file__).resolve().parent / "package-macos.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module

# 컴포넌트 → (zip 이름 앞부분, 실행 파일, 함께 넣는 저장소 파일 → zip 안 이름)
LINUX_ASSETS = {
    "saba-core": ("saba-core", ["saba-core", "saba-runner"], {
        "modules/i18n.py": "i18n.py", "modules/daemon_rcon.py": "daemon_rcon.py", "modules/common_locales.json": "common_locales.json"}),
    "cli": ("saba-chan-cli", ["saba-chan-cli"], {}),
    "gui": ("saba-chan-gui", ["saba-chan-gui"], {"saba-chan-gui/src-tauri/icons/icon.png": "saba-chan-gui.png"}),
    "installer.desktop": ("saba-chan-installer-desktop", ["saba-chan-installer"], {}),
    "installer.headless": ("saba-chan-installer-headless", ["saba-chan-installer"], {}),
}
# 창 없이 빌드한 실행 파일을 `--headless-bin`에서 찾는 컴포넌트
HEADLESS_BUILDS = {"installer.headless"}


def add(zf: zipfile.ZipFile, src: Path | bytes, name: str, mode: int) -> None:
    info = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
    info.compress_type = zipfile.ZIP_DEFLATED
    info.external_attr = (0o100000 | mode) << 16
    zf.writestr(info, src if isinstance(src, bytes) else src.read_bytes())


def mac_gui_files(version: str) -> dict[str, bytes]:
    """맥 GUI zip에 더하는 앱 묶음 파일: 설치기·업데이터가 `Contents/Info.plist`·`Contents/Resources`에 놓는다"""
    mac = _mac()
    return {"Info.plist": mac.info_plist(version), mac.ICON_FILE: mac.ICON_SOURCE.read_bytes()}


def package(bin_dir: Path, out_dir: Path, platform: str, only: list[str] | None = None, version: str | None = None,
            headless_bin: Path | None = None) -> list[Path]:
    out_dir.mkdir(parents=True, exist_ok=True)
    mac = platform.startswith("macos")
    if mac and (only is None or "gui" in only) and not version:
        sys.exit("macOS GUI zip needs --version (Info.plist)")
    made = []
    for key, (prefix, exes, extras) in LINUX_ASSETS.items():
        if only is not None and key not in only:
            continue
        src = headless_bin if key in HEADLESS_BUILDS else bin_dir
        if src is None:
            sys.exit(f"{prefix}: pass --headless-bin (the --no-default-features build)")
        missing = [e for e in exes if not (src / e).is_file()]
        if missing:
            sys.exit(f"{prefix}: {', '.join(missing)} not built in {src}")
        target = out_dir / f"{prefix}-{platform}.zip"
        with zipfile.ZipFile(target, "w") as zf:
            for exe in exes:
                add(zf, src / exe, exe, 0o755)
            # 리눅스 앱 메뉴 아이콘(png)은 맥에서 쓰지 않는다: 맥은 앱 묶음 파일을 싣는다
            if mac and key == "gui":
                for name, data in mac_gui_files(version).items():
                    add(zf, data, name, 0o644)
            else:
                for rel, name in extras.items():
                    add(zf, ROOT / rel, name, 0o644)
        made.append(target)
        print(f"    → {target.name} ({target.stat().st_size / 1024 / 1024:.2f} MB)")
    return made


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--bin", required=True, help="cargo build --release 출력 폴더")
    parser.add_argument("--out", required=True)
    parser.add_argument("--platform", default="linux-x64")
    parser.add_argument("--headless-bin", help="창 없이 빌드한 설치기 폴더 (cargo build -p saba-chan-installer --no-default-features)")
    parser.add_argument("--only", help="이 컴포넌트만 (쉼표 목록, 예: saba-core,cli,installer.headless). 없으면 모두")
    parser.add_argument("--version", help="versions.toml [release] version (맥 GUI의 Info.plist에 적는다)")
    args = parser.parse_args()
    package(Path(args.bin), Path(args.out), args.platform, args.only.split(",") if args.only else None, args.version,
            Path(args.headless_bin) if args.headless_bin else None)
    return 0


if __name__ == "__main__":
    sys.exit(main())
