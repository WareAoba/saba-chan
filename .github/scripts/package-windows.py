#!/usr/bin/env python3
"""Windows 배포 에셋을 만든다 (publish.yml).

    python .github/scripts/package-windows.py --bin dist --out output
    python .github/scripts/package-windows.py --locales-only --out output   # 번역만 바뀐 부분 릴리즈

`--bin`은 `versions.py signing --collect`가 모은 exe 폴더다. 만드는 것:
- saba-core-windows-x64.zip: saba-core.exe, saba-runner.exe, 공유 모듈(목록은 package-linux.py 하나에서 정한다)
- saba-chan-cli-windows-x64.zip: saba-chan-cli.exe와 짧은 명령 실행기 bin/saba-chan.exe
  (실행기를 설치 폴더 바로 아래에 두면 0.1.1 GUI 이름 `Saba-chan.exe`와 겹친다. 설치기가 `bin`을 사용자 PATH에 넣는다)
- saba-chan-gui-windows-x64.zip: 그 exe 하나
- saba-chan-installer.exe: zip이 아닌 exe 그대로 (소개 페이지 다운로드와 업데이트가 이 파일을 쓴다)
- locales.zip: 저장소 locales/ (zip 안에는 locales/ 접두사 없이 en/bot.json 꼴)
"""

import argparse
import importlib.util
import shutil
import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
PLATFORM = "windows-x64"
# zip 안에서 루트가 아닌 곳에 두는 exe (exe 이름 → zip 안 경로)
PLACED = {"saba-chan.exe": "bin/saba-chan.exe"}


def _shared_modules() -> dict[str, str]:
    """데몬 zip에 함께 드는 공유 모듈 (저장소 경로 → zip 안 이름)"""
    spec = importlib.util.spec_from_file_location("package_linux", Path(__file__).resolve().parent / "package-linux.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.LINUX_ASSETS["saba-core"][2]


def package(bin_dir: Path, out_dir: Path) -> list[Path]:
    zips = {
        f"saba-core-{PLATFORM}.zip": (["saba-core.exe", "saba-runner.exe"], _shared_modules()),
        f"saba-chan-cli-{PLATFORM}.zip": (["saba-chan-cli.exe", "saba-chan.exe"], {}),
        f"saba-chan-gui-{PLATFORM}.zip": (["saba-chan-gui.exe"], {}),
    }
    needed = [exe for exes, _ in zips.values() for exe in exes] + ["saba-chan-installer.exe"]
    missing = [e for e in needed if not (bin_dir / e).is_file()]
    if missing:
        sys.exit(f"not built in {bin_dir}: {', '.join(missing)}")
    out_dir.mkdir(parents=True, exist_ok=True)
    made = []
    for name, (exes, extras) in zips.items():
        target = out_dir / name
        with zipfile.ZipFile(target, "w", zipfile.ZIP_DEFLATED) as zf:
            for exe in exes:
                zf.write(bin_dir / exe, PLACED.get(exe, exe))
            for rel, inner in extras.items():
                zf.write(ROOT / rel, inner)
        made.append(target)
    installer = out_dir / "saba-chan-installer.exe"
    shutil.copyfile(bin_dir / "saba-chan-installer.exe", installer)
    made.append(installer)
    made.append(package_locales(out_dir))
    for p in made:
        print(f"    -> {p.name} ({p.stat().st_size / 1024 / 1024:.2f} MB)")
    return made


def package_locales(out_dir: Path) -> Path:
    """locales.zip: 저장소 locales/ (zip 안에는 locales/ 접두사 없이)"""
    out_dir.mkdir(parents=True, exist_ok=True)
    locales = out_dir / "locales.zip"
    with zipfile.ZipFile(locales, "w", zipfile.ZIP_DEFLATED) as zf:
        for f in sorted((ROOT / "locales").rglob("*")):
            if f.is_file():
                zf.write(f, f.relative_to(ROOT / "locales").as_posix())
    return locales


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--bin", help="versions.py signing --collect 가 모은 exe 폴더")
    parser.add_argument("--out", required=True)
    parser.add_argument("--locales-only", action="store_true", help="locales.zip만 만든다 (실행 파일 빌드 없이)")
    args = parser.parse_args()
    if args.locales_only:
        package_locales(Path(args.out))
        return 0
    if not args.bin:
        parser.error("--bin is required")
    package(Path(args.bin), Path(args.out))
    return 0


if __name__ == "__main__":
    sys.exit(main())
