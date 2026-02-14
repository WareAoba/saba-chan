"""
manifest.json 자동 생성 스크립트

릴리즈 발행 시 GitHub Action에서 실행되어:
1. 각 컴포넌트의 소스코드에서 버전을 읽음 (Cargo.toml / package.json)
2. 릴리즈에 업로드된 에셋 목록을 확인
3. 컴포넌트→에셋 매핑이 담긴 manifest.json 생성
4. release_version = 컴포넌트 중 가장 높은 버전

환경변수:
  GITHUB_TOKEN   — API 인증
  RELEASE_TAG    — 릴리즈 태그 (예: v0.5.0)
  RELEASE_ID     — 릴리즈 ID
  REPO_FULL_NAME — owner/repo
"""

import json
import os
import re
import sys
from pathlib import Path
from urllib.request import Request, urlopen
from urllib.error import HTTPError

# ─── 컴포넌트 정의 ──────────────────────────────────────
# (key, 버전 소스 경로, 에셋 파일명 패턴, install_dir)
COMPONENTS = [
    {
        "key": "core_daemon",
        "version_file": "Cargo.toml",           # 워크스페이스 루트
        "version_type": "cargo",
        "asset_pattern": r"core_daemon.*\.zip$",
        "install_dir": ".",
    },
    {
        "key": "cli",
        "version_file": "saba-chan-cli/Cargo.toml",
        "version_type": "cargo",
        "asset_pattern": r"saba-chan-cli.*\.zip$",
        "install_dir": ".",
    },
    {
        "key": "gui",
        "version_file": "saba-chan-gui/package.json",
        "version_type": "npm",
        "asset_pattern": r"saba-chan-gui.*\.zip$",
        "install_dir": "saba-chan-gui",
    },
    {
        "key": "updater",
        "version_file": "updater/Cargo.toml",
        "version_type": "cargo",
        "asset_pattern": r"saba-chan-updater.*\.zip$",
        "install_dir": ".",
    },
    {
        "key": "discord_bot",
        "version_file": "discord_bot/package.json",
        "version_type": "npm",
        "asset_pattern": r"discord-bot.*\.zip$",
        "install_dir": "discord_bot",
    },
]


def parse_semver(version_str: str) -> tuple[int, int, int]:
    """시맨틱 버전 문자열 → (major, minor, patch) 튜플"""
    v = version_str.lstrip("v")
    # 프리릴리스 제거
    v = v.split("-")[0]
    parts = v.split(".")
    major = int(parts[0]) if len(parts) > 0 else 0
    minor = int(parts[1]) if len(parts) > 1 else 0
    patch = int(parts[2]) if len(parts) > 2 else 0
    return (major, minor, patch)


def read_cargo_version(file_path: str) -> str | None:
    """Cargo.toml에서 version = "x.y.z" 추출"""
    try:
        content = Path(file_path).read_text(encoding="utf-8")
        for line in content.splitlines():
            stripped = line.strip()
            if stripped.startswith("version") and "=" in stripped:
                # [package] 섹션의 version만 — [dependencies] 내 version은 무시
                value = stripped.split("=", 1)[1].strip().strip('"').strip("'")
                return value
    except FileNotFoundError:
        print(f"  ⚠ {file_path} not found, skipping")
    return None


def read_npm_version(file_path: str) -> str | None:
    """package.json에서 version 필드 추출"""
    try:
        content = Path(file_path).read_text(encoding="utf-8")
        data = json.loads(content)
        return data.get("version")
    except (FileNotFoundError, json.JSONDecodeError):
        print(f"  ⚠ {file_path} not found or invalid, skipping")
    return None


def fetch_release_assets(repo: str, release_id: str, token: str) -> list[dict]:
    """GitHub API로 릴리즈 에셋 목록 조회"""
    url = f"https://api.github.com/repos/{repo}/releases/{release_id}/assets"
    req = Request(url, headers={
        "Authorization": f"Bearer {token}",
        "Accept": "application/vnd.github+json",
        "User-Agent": "saba-chan-manifest-generator",
    })
    try:
        with urlopen(req) as resp:
            return json.loads(resp.read().decode())
    except HTTPError as e:
        print(f"  ⚠ Failed to fetch release assets: {e}")
        return []


def match_asset(asset_pattern: str, assets: list[dict]) -> dict | None:
    """에셋 목록에서 패턴에 매치되는 첫 번째 에셋 반환"""
    pattern = re.compile(asset_pattern, re.IGNORECASE)
    for asset in assets:
        if pattern.search(asset["name"]):
            return asset
    return None


def main():
    token = os.environ.get("GITHUB_TOKEN", "")
    release_tag = os.environ.get("RELEASE_TAG", "")
    release_id = os.environ.get("RELEASE_ID", "")
    repo = os.environ.get("REPO_FULL_NAME", "")

    print(f"=== Generating manifest.json for {release_tag} ===")
    print(f"Repository: {repo}")
    print(f"Release ID: {release_id}")

    # 릴리즈 에셋 목록 가져오기
    assets = []
    if token and release_id and repo:
        assets = fetch_release_assets(repo, release_id, token)
        print(f"Found {len(assets)} assets in release")
        for a in assets:
            print(f"  - {a['name']} ({a['size']} bytes)")
    else:
        print("⚠ Missing environment variables; running in local/test mode")

    # 각 컴포넌트 버전 수집 & 에셋 매핑
    manifest_components = {}
    all_versions = []

    for comp in COMPONENTS:
        key = comp["key"]
        print(f"\n[{key}]")

        # 버전 읽기
        if comp["version_type"] == "cargo":
            version = read_cargo_version(comp["version_file"])
        else:
            version = read_npm_version(comp["version_file"])

        if not version:
            print(f"  → version not found, skipping")
            continue

        print(f"  version: {version}")
        all_versions.append(version)

        # 에셋 매핑
        matched = match_asset(comp["asset_pattern"], assets)
        asset_name = matched["name"] if matched else None
        if asset_name:
            print(f"  asset: {asset_name} ✓")
        else:
            print(f"  asset: (not in this release)")

        manifest_components[key] = {
            "version": version,
            "asset": asset_name,
            "sha256": None,  # CI에서 빌드 시 해시를 넣을 수 있도록 예약
            "install_dir": comp["install_dir"],
        }

    # release_version = 컴포넌트 중 가장 높은 버전
    if all_versions:
        release_version = max(all_versions, key=parse_semver)
    else:
        # 폴백: 태그에서 추출
        release_version = release_tag.lstrip("v") if release_tag else "0.0.0"

    print(f"\n=== Release version: {release_version} ===")

    manifest = {
        "release_version": release_version,
        "components": manifest_components,
    }

    # manifest.json 출력
    output_path = Path("manifest.json")
    output_path.write_text(
        json.dumps(manifest, indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    print(f"\n✓ Written to {output_path}")
    print(json.dumps(manifest, indent=2, ensure_ascii=False))


if __name__ == "__main__":
    main()
