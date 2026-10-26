#!/usr/bin/env python3
"""화면 없는 리눅스 설치용 zip을 만든다 (D3c).

    python .github/scripts/package-linux.py --bin target/release --out output [--platform linux-x64]

Windows zip과 같은 이름 규칙(`<컴포넌트>-<플랫폼>.zip`)과 같은 내용(saba-core zip에 saba-runner와 공유 모듈)이다.
실행 파일은 실행 권한(0755)을 zip에 적는다: 업데이터·설치기가 그 비트를 보고 실행 권한을 준다.
GUI와 설치기 zip은 만들지 않는다 (리눅스는 헤드리스 구성과 `saba-chan-cli install`만).
"""

import argparse
import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

# 컴포넌트 → (zip 이름 앞부분, 실행 파일, 함께 넣는 저장소 파일)
LINUX_ASSETS = {
    "saba-core": ("saba-core", ["saba-core", "saba-runner"], ["modules/i18n.py", "modules/daemon_rcon.py"]),
    "cli": ("saba-chan-cli", ["saba-chan-cli"], []),
    "updater": ("saba-chan-updater", ["saba-chan-updater"], []),
}


def add(zf: zipfile.ZipFile, src: Path, name: str, mode: int) -> None:
    info = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
    info.compress_type = zipfile.ZIP_DEFLATED
    info.external_attr = (0o100000 | mode) << 16
    zf.writestr(info, src.read_bytes())


def package(bin_dir: Path, out_dir: Path, platform: str) -> list[Path]:
    out_dir.mkdir(parents=True, exist_ok=True)
    made = []
    for prefix, exes, extras in LINUX_ASSETS.values():
        missing = [e for e in exes if not (bin_dir / e).is_file()]
        if missing:
            sys.exit(f"{prefix}: {', '.join(missing)} not built in {bin_dir}")
        target = out_dir / f"{prefix}-{platform}.zip"
        with zipfile.ZipFile(target, "w") as zf:
            for exe in exes:
                add(zf, bin_dir / exe, exe, 0o755)
            for rel in extras:
                add(zf, ROOT / rel, Path(rel).name, 0o644)
        made.append(target)
        print(f"    → {target.name} ({target.stat().st_size / 1024 / 1024:.2f} MB)")
    return made


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--bin", required=True, help="cargo build --release 출력 폴더")
    parser.add_argument("--out", required=True)
    parser.add_argument("--platform", default="linux-x64")
    args = parser.parse_args()
    package(Path(args.bin), Path(args.out), args.platform)
    return 0


if __name__ == "__main__":
    sys.exit(main())
