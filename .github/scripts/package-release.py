"""
릴리즈 에셋 패키징 & manifest.json 생성 스크립트

GitHub Action에서 실행되어:
1. staging/ 에 다운로드된 raw 에셋(exe, zip 등)을 표준 이름으로 리패키징
2. 이번 릴리즈에 없는 컴포넌트는 이전 릴리즈에서 자동으로 가져옴
3. 소스 locales/ → locales.zip 생성
4. 각 컴포넌트의 Cargo.toml / package.json에서 버전 읽기
5. SHA256 해시가 포함된 manifest.json 생성
6. output/ 디렉터리에 모든 결과물 출력

부분 릴리즈 지원:
  모든 컴포넌트를 매번 빌드·업로드할 필요 없음.
  변경된 컴포넌트만 올리면, 나머지는 이전 릴리즈에서 자동 수집.
  이를 통해 인스톨러/업데이터 모두 매 릴리즈에서 완전한 에셋을 제공받음.

환경변수:
  GITHUB_TOKEN   — API 인증 (gh CLI가 사용)
  RELEASE_TAG    — 릴리즈 태그 (예: v0.5.0)
  RELEASE_ID     — 릴리즈 ID
  REPO_FULL_NAME — owner/repo

디렉터리 구조:
  staging/       — gh release download 결과 (raw 파일들)
  output/        — 최종 업로드 대상 파일들
"""

import hashlib
import json
import os
import re
import shutil
import subprocess
import zipfile
from pathlib import Path

# ─── 컴포넌트 정의 ──────────────────────────────────────
# 각 컴포넌트의 정보
#   key           — manifest.json의 키
#   version_file  — 버전을 읽을 파일 (현재 체크아웃 기준)
#   version_type  — "cargo" 또는 "npm"
#   raw_patterns  — staging/에서 찾을 원본 파일 패턴 (exe, zip 등)
#   output_name   — 최종 zip 파일 이름 (이전 릴리즈 탐색 기준)
#   install_dir   — 인스톨러가 설치할 디렉터리
#   exe_name      — exe를 zip으로 감쌀 때 내부 파일명

COMPONENTS = [
    {
        "key": "saba-core",
        "version_file": "Cargo.toml",
        "version_type": "cargo",
        "raw_patterns": [r"saba-core\.exe$", r"saba-core.*\.zip$"],
        "output_name": "saba-core-windows-x64.zip",
        "install_dir": ".",
        "exe_name": "saba-core.exe",
    },
    {
        "key": "cli",
        "version_file": "saba-chan-cli/Cargo.toml",
        "version_type": "cargo",
        "raw_patterns": [r"saba-chan-cli\.exe$", r"saba-chan-cli.*\.zip$"],
        "output_name": "saba-chan-cli-windows-x64.zip",
        "install_dir": ".",
        "exe_name": "saba-chan-cli.exe",
    },
    {
        "key": "gui",
        "version_file": "saba-chan-gui/package.json",
        "version_type": "npm",
        "raw_patterns": [r"saba-chan-gui\.exe$", r"[Ss]aba.*[Cc]han\.exe$", r"saba-chan-gui.*\.zip$"],
        "output_name": "saba-chan-gui-windows-x64.zip",
        "install_dir": ".",
        "exe_name": "saba-chan-gui.exe",
    },
    {
        "key": "updater",
        "version_file": "updater/Cargo.toml",
        "version_type": "cargo",
        "raw_patterns": [r"saba-chan-updater\.exe$", r"saba-chan-updater.*\.zip$"],
        "output_name": "saba-chan-updater-windows-x64.zip",
        "install_dir": ".",
        "exe_name": "saba-chan-updater.exe",
    },
    {
        "key": "discord_bot",
        "version_file": "discord_bot/package.json",
        "version_type": "npm",
        "raw_patterns": [r"discord.?bot.*\.zip$"],
        "output_name": "discord-bot.zip",
        "install_dir": "discord_bot",
        "exe_name": None,  # zip 그대로 사용
    },
    {
        "key": "locales",
        "version_file": "Cargo.toml",  # 루트 버전 사용
        "version_type": "cargo",
        "raw_patterns": [],  # 소스에서 직접 생성
        "output_name": "locales.zip",
        "install_dir": "locales",
        "exe_name": None,
    },
]

STAGING_DIR = Path("staging")
OUTPUT_DIR = Path("output")


# ─── 유틸리티 ────────────────────────────────────────────

def sha256_file(path: Path) -> str:
    """파일의 SHA256 해시 계산"""
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(8192), b""):
            h.update(chunk)
    return h.hexdigest()


def parse_semver(version_str: str) -> tuple[int, int, int]:
    """시맨틱 버전 → (major, minor, patch) 튜플"""
    v = version_str.lstrip("v").split("-")[0]
    parts = v.split(".")
    return (
        int(parts[0]) if len(parts) > 0 else 0,
        int(parts[1]) if len(parts) > 1 else 0,
        int(parts[2]) if len(parts) > 2 else 0,
    )


def read_cargo_version(file_path: str) -> str | None:
    """Cargo.toml에서 [package] version 추출"""
    try:
        content = Path(file_path).read_text(encoding="utf-8")
        in_package = False
        for line in content.splitlines():
            stripped = line.strip()
            if stripped == "[package]":
                in_package = True
                continue
            if stripped.startswith("[") and stripped != "[package]":
                in_package = False
                continue
            if in_package and stripped.startswith("version"):
                return stripped.split("=", 1)[1].strip().strip('"').strip("'")
    except FileNotFoundError:
        print(f"  ⚠ {file_path} not found")
    return None


def read_npm_version(file_path: str) -> str | None:
    """package.json에서 version 필드 추출"""
    try:
        data = json.loads(Path(file_path).read_text(encoding="utf-8"))
        return data.get("version")
    except (FileNotFoundError, json.JSONDecodeError):
        print(f"  ⚠ {file_path} not found or invalid")
    return None


def find_raw_file(patterns: list[str]) -> Path | None:
    """staging/ 에서 패턴에 매치되는 파일 찾기"""
    if not STAGING_DIR.exists():
        return None
    for f in sorted(STAGING_DIR.iterdir()):
        for pat in patterns:
            if re.search(pat, f.name, re.IGNORECASE):
                return f
    return None


def create_exe_zip(exe_path: Path, output_zip: Path, inner_name: str):
    """단일 exe를 zip으로 패키징"""
    with zipfile.ZipFile(output_zip, "w", zipfile.ZIP_DEFLATED) as zf:
        zf.write(exe_path, inner_name)
    print(f"    → {output_zip.name} ({output_zip.stat().st_size / 1024 / 1024:.2f} MB)")


def copy_or_repackage(raw_file: Path, output_zip: Path, exe_name: str | None):
    """raw 파일이 exe면 zip으로 감싸고, 이미 zip이면 표준 이름으로 복사"""
    if raw_file.suffix.lower() == ".zip":
        shutil.copy2(raw_file, output_zip)
        print(f"    → {output_zip.name} (copied from {raw_file.name})")
    elif raw_file.suffix.lower() == ".exe" and exe_name:
        create_exe_zip(raw_file, output_zip, exe_name)
    else:
        shutil.copy2(raw_file, output_zip)
        print(f"    → {output_zip.name} (copied)")


def create_locales_zip(output_zip: Path):
    """소스의 locales/ 디렉터리를 zip으로 패키징

    zip 내부 구조: en/bot.json, ko/cli.json, ... (locales/ 접두사 없음)
    → install_root/locales/ 에 압축 해제하면 install_root/locales/en/bot.json 이 됨
    """
    locales_dir = Path("locales")
    if not locales_dir.exists():
        print("  ⚠ locales/ directory not found in repo root")
        return False

    with zipfile.ZipFile(output_zip, "w", zipfile.ZIP_DEFLATED) as zf:
        for root, dirs, files in os.walk(locales_dir):
            for f in sorted(files):
                file_path = Path(root) / f
                # locales/en/bot.json → en/bot.json (locales/ 접두사 제거)
                arcname = file_path.relative_to(locales_dir)
                zf.write(file_path, arcname)

    size_kb = output_zip.stat().st_size / 1024
    print(f"    → {output_zip.name} ({size_kb:.1f} KB)")
    return True


# ─── 이전 릴리즈 에셋 가져오기 ──────────────────────────

def fetch_previous_releases() -> list[dict] | None:
    """GitHub API로 이전 릴리즈 목록 가져오기 (gh CLI 사용)"""
    repo = os.environ.get("REPO_FULL_NAME", "")
    if not repo:
        print("  ⚠ REPO_FULL_NAME not set, cannot fetch previous releases")
        return None

    try:
        result = subprocess.run(
            ["gh", "api", f"repos/{repo}/releases",
             "--paginate", "--jq",
             '[.[] | select(.draft == false) '
             '| {tag: .tag_name, prerelease: .prerelease, '
             'assets: [.assets[].name]}]'],
            capture_output=True, text=True, timeout=30,
        )
        if result.returncode != 0:
            print(f"  ⚠ gh api failed: {result.stderr.strip()}")
            return None
        return json.loads(result.stdout)
    except (subprocess.TimeoutExpired, json.JSONDecodeError, FileNotFoundError) as e:
        print(f"  ⚠ Failed to fetch releases: {e}")
        return None


def download_from_previous_release(
    asset_name: str,
    current_tag: str,
    releases: list[dict],
) -> Path | None:
    """이전 릴리즈에서 에셋을 찾아 output/ 에 직접 다운로드

    최신 릴리즈부터 순회하며 해당 에셋이 포함된 첫 번째 릴리즈에서 다운로드.
    """
    repo = os.environ.get("REPO_FULL_NAME", "")
    if not repo:
        return None

    for release in releases:
        tag = release["tag"]
        if tag == current_tag:
            continue

        if asset_name not in release.get("assets", []):
            continue

        print(f"    ← {tag} 에서 발견, 다운로드 중...")
        try:
            result = subprocess.run(
                ["gh", "release", "download", tag,
                 "--pattern", asset_name,
                 "--dir", str(OUTPUT_DIR),
                 "--repo", repo,
                 "--clobber"],
                capture_output=True, text=True, timeout=120,
            )
            if result.returncode == 0:
                dest = OUTPUT_DIR / asset_name
                if dest.exists():
                    size = dest.stat().st_size
                    if size > 1024 * 1024:
                        print(f"    → {asset_name} ({size / 1024 / 1024:.2f} MB, from {tag})")
                    else:
                        print(f"    → {asset_name} ({size / 1024:.1f} KB, from {tag})")
                    return dest
            else:
                print(f"    ⚠ download failed: {result.stderr.strip()}")
        except subprocess.TimeoutExpired:
            print(f"    ⚠ download timed out for {asset_name} from {tag}")

    return None


def read_version_from_previous_manifest(
    asset_name: str,
    comp_key: str,
    current_tag: str,
    releases: list[dict],
) -> str | None:
    """이전 릴리즈의 manifest.json에서 해당 컴포넌트의 버전을 읽어옴.

    이전 릴리즈의 바이너리를 가져올 때, 현재 소스의 버전이 아니라
    해당 빌드 시점의 manifest에 기록된 버전을 사용해야 정확하다.
    """
    repo = os.environ.get("REPO_FULL_NAME", "")
    if not repo:
        return None

    for release in releases:
        tag = release["tag"]
        if tag == current_tag:
            continue
        if asset_name not in release.get("assets", []):
            continue
        if "manifest.json" not in release.get("assets", []):
            continue

        # 이전 릴리즈의 manifest.json 다운로드
        try:
            result = subprocess.run(
                ["gh", "api",
                 f"repos/{repo}/releases/tags/{tag}",
                 "--jq",
                 '.assets[] | select(.name == "manifest.json") | .url'],
                capture_output=True, text=True, timeout=15,
            )
            if result.returncode != 0:
                continue

            asset_url = result.stdout.strip()
            if not asset_url:
                continue

            manifest_result = subprocess.run(
                ["gh", "api", asset_url,
                 "-H", "Accept: application/octet-stream"],
                capture_output=True, text=True, timeout=15,
            )
            if manifest_result.returncode != 0:
                continue

            manifest = json.loads(manifest_result.stdout)
            comp_info = manifest.get("components", {}).get(comp_key, {})
            version = comp_info.get("version")
            if version:
                return version
        except (subprocess.TimeoutExpired, json.JSONDecodeError):
            continue

    return None


# ─── 메인 ────────────────────────────────────────────────

def main():
    release_tag = os.environ.get("RELEASE_TAG", "v0.0.0")

    print(f"{'=' * 60}")
    print(f"  Release Packaging: {release_tag}")
    print(f"{'=' * 60}")
    print()

    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)

    # 이전 릴리즈 목록 미리 가져오기 (부분 릴리즈 대비)
    print("[pre-flight] Fetching previous releases...")
    previous_releases = fetch_previous_releases() or []
    if previous_releases:
        print(f"  {len(previous_releases)} release(s) found")
    else:
        print("  (none found — all components must be in staging)")
    print()

    # 각 컴포넌트 처리
    manifest_components = {}
    all_versions = []
    from_current = []   # 이번 릴리즈에서 패키징된 컴포넌트
    from_previous = []  # 이전 릴리즈에서 가져온 컴포넌트
    skipped = []        # 에셋을 찾지 못한 컴포넌트

    for comp in COMPONENTS:
        key = comp["key"]
        print(f"[{key}]")

        # 1. 버전 읽기 (현재 소스 기준 — 이전 릴리즈에서 가져올 경우 덮어씌움)
        if comp["version_type"] == "cargo":
            version = read_cargo_version(comp["version_file"])
        else:
            version = read_npm_version(comp["version_file"])

        if not version:
            print(f"  version: (not found, skipping)")
            skipped.append(key)
            continue

        print(f"  version: {version} (source)")

        # 2. 에셋 패키징
        output_path = OUTPUT_DIR / comp["output_name"]
        asset_source = "current"  # "current" | "previous" | "generated"

        if key == "locales":
            # locales는 항상 소스에서 직접 생성 (매 릴리즈마다 최신)
            if not create_locales_zip(output_path):
                skipped.append(key)
                continue
            asset_source = "generated"
        else:
            # staging/에서 현재 릴리즈의 raw 에셋 찾기
            raw_file = find_raw_file(comp["raw_patterns"])
            if raw_file:
                print(f"  raw: {raw_file.name}")
                copy_or_repackage(raw_file, output_path, comp.get("exe_name"))
                asset_source = "current"
            else:
                # 이번 릴리즈에 없음 → 이전 릴리즈에서 가져오기
                print(f"  raw: (not in staging, searching previous releases...)")
                downloaded = download_from_previous_release(
                    comp["output_name"], release_tag, previous_releases,
                )
                if downloaded:
                    asset_source = "previous"
                    # 이전 릴리즈의 manifest에서 정확한 버전 가져오기
                    # (현재 소스의 버전과 실제 바이너리 버전이 다를 수 있음)
                    prev_version = read_version_from_previous_manifest(
                        comp["output_name"], key, release_tag, previous_releases,
                    )
                    if prev_version and prev_version != version:
                        print(f"  version: {prev_version} (from previous manifest, overrides source {version})")
                        version = prev_version
                else:
                    print(f"  ⚠ NOT FOUND in any release — skipping")
                    skipped.append(key)
                    continue

        all_versions.append(version)

        # 3. SHA256 계산
        sha = sha256_file(output_path)
        print(f"  sha256: {sha[:16]}...")

        manifest_components[key] = {
            "version": version,
            "asset": comp["output_name"],
            "sha256": sha,
            "install_dir": comp["install_dir"],
        }

        if asset_source == "current":
            from_current.append(key)
        elif asset_source == "previous":
            from_previous.append(key)
        else:
            from_current.append(key)  # generated = current

        print()

    # release_version = 컴포넌트 중 가장 높은 버전
    if all_versions:
        release_version = max(all_versions, key=parse_semver)
    else:
        release_version = release_tag.lstrip("v") if release_tag else "0.0.0"

    # manifest.json 생성
    manifest = {
        "release_version": release_version,
        "tag": release_tag,
        "components": manifest_components,
    }

    manifest_path = OUTPUT_DIR / "manifest.json"
    manifest_path.write_text(
        json.dumps(manifest, indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )

    # ── 결과 요약 ──
    print()
    print(f"{'=' * 60}")
    print(f"  Release: {release_tag} (version {release_version})")
    print(f"{'=' * 60}")
    print()

    if from_current:
        print(f"  New in this release:     {', '.join(from_current)}")
    if from_previous:
        print(f"  From previous release:   {', '.join(from_previous)}")
    if skipped:
        print(f"  ⚠ Skipped (not found):  {', '.join(skipped)}")
    print()

    print("Output files:")
    for f in sorted(OUTPUT_DIR.iterdir()):
        size = f.stat().st_size
        if size > 1024 * 1024:
            print(f"  {f.name:<45} {size / 1024 / 1024:.2f} MB")
        else:
            print(f"  {f.name:<45} {size / 1024:.1f} KB")

    print()
    print("manifest.json:")
    print(json.dumps(manifest, indent=2, ensure_ascii=False))
    print()
    print("✓ Packaging complete")


if __name__ == "__main__":
    main()
