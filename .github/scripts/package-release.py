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
import sys
import subprocess
import zipfile
from pathlib import Path

# ─── 컴포넌트 정의 ──────────────────────────────────────
# 각 컴포넌트의 정보
#   key           — manifest.json의 키
#   버전은 저장소 루트 versions.toml의 [components.<key>]에서 읽는다
#   raw_patterns  — staging/에서 찾을 원본 파일 패턴 (exe, zip 등)
#   output_name   — 최종 zip 파일 이름 (이전 릴리즈 탐색 기준)
#   install_dir   — 인스톨러가 설치할 디렉터리
#   exe_name      — exe를 zip으로 감쌀 때 내부 파일명

COMPONENTS = [
    {
        "key": "saba-core",
        "raw_patterns": [r"saba-core\.exe$", r"saba-core.*\.zip$"],
        "output_name": "saba-core-windows-x64.zip",
        "install_dir": ".",
        "exe_name": "saba-core.exe",
        # 공유 모듈 유틸리티 — %APPDATA%/saba-chan/modules/ 에 설치됨
        "extra_files": ["modules/i18n.py", "modules/daemon_rcon.py"],
        # 데몬 옆에 함께 설치되는 실행 파일 (staging/에서 찾는다). saba-runner: 서버 콘솔 감시 (C8b)
        "extra_staged": [r"saba-runner\.exe$"],
        # 이 컴포넌트 zip에 함께 든 다른 exe: 재포장할 때 빼지 않는다 (staging에 따로 없을 때 대비)
        "bundles": ["saba-runner.exe"],
    },
    {
        "key": "cli",
        "raw_patterns": [r"saba-chan-cli\.exe$", r"saba-chan-cli.*\.zip$"],
        "output_name": "saba-chan-cli-windows-x64.zip",
        "install_dir": ".",
        "exe_name": "saba-chan-cli.exe",
    },
    {
        "key": "gui",
        "raw_patterns": [r"saba-chan-gui\.exe$", r"[Ss]aba.*[Cc]han\.exe$", r"saba-chan-gui.*\.zip$"],
        "output_name": "saba-chan-gui-windows-x64.zip",
        "install_dir": ".",
        "exe_name": "saba-chan-gui.exe",
    },
    {
        "key": "updater",
        "raw_patterns": [r"saba-chan-updater\.exe$", r"saba-chan-updater.*\.zip$"],
        "output_name": "saba-chan-updater-windows-x64.zip",
        "install_dir": ".",
        "exe_name": "saba-chan-updater.exe",
    },
    {
        "key": "locales",
        "raw_patterns": [],  # 소스에서 직접 생성
        "output_name": "locales.zip",
        "install_dir": "locales",
        "exe_name": None,
    },
    {
        "key": "installer",
        "raw_patterns": [r"saba-chan-installer\.exe$", r"saba-chan-installer.*\.zip$"],
        # zip이 아닌 exe 그대로 싣는다: 소개 페이지 다운로드가 이 에셋을 가리킨다. 업데이터는 받지 않는다
        "output_name": "saba-chan-installer.exe",
        "install_dir": ".",
        "exe_name": "saba-chan-installer.exe",
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


def read_bundled_version(key: str) -> str | None:
    """versions.toml의 [components.<key>] version"""
    import tomllib
    data = tomllib.loads(Path("versions.toml").read_text(encoding="utf-8"))
    return data.get("components", {}).get(key, {}).get("version")


def find_raw_file(patterns: list[str]) -> Path | None:
    """staging/ 에서 패턴에 매치되는 파일 찾기"""
    if not STAGING_DIR.exists():
        return None
    for f in sorted(STAGING_DIR.iterdir()):
        for pat in patterns:
            if re.search(pat, f.name, re.IGNORECASE):
                return f
    return None


def create_exe_zip(exe_path: Path, output_zip: Path, inner_name: str,
                   extra_files: list[str] | None = None):
    """단일 exe를 zip으로 패키징 (+ optional extra files)"""
    with zipfile.ZipFile(output_zip, "w", zipfile.ZIP_DEFLATED) as zf:
        zf.write(exe_path, inner_name)
        for rel in (extra_files or []):
            src = Path(rel)
            if src.exists():
                zf.write(src, src.name)  # 루트에 flat하게 포함
                print(f"    + {src.name} (shared module)")
            else:
                print(f"    ⚠ {rel} not found, skipping")
    print(f"    → {output_zip.name} ({output_zip.stat().st_size / 1024 / 1024:.2f} MB)")


# 다른 컴포넌트가 따로 배포하는 실행 파일: GUI zip 등에 섞여 있으면 뺀다 (GUI만 업데이트해도 데몬이 옛 판으로 덮이지 않게)
OTHER_COMPONENT_EXES = {"saba-core.exe", "saba-runner.exe", "saba-chan-updater.exe", "saba-chan-cli.exe"}


def repack_zip(raw_file: Path, output_zip: Path, exe_name: str | None,
               extra_files: list[str] | None = None, bundles: list[str] | None = None):
    """zip을 표준 형태로 다시 담는다: 경로 구분자 `/`, 루트의 앱 exe(예: Electron의 Saba-chan.exe)는
    `exe_name`으로, 다른 컴포넌트의 exe는 뺀다 (`bundles`에 적은 것은 남긴다).
    `extra_files`와 이름이 같은 항목은 extra 쪽(이번 빌드)으로 바꾼다."""
    keep = {b.lower() for b in (bundles or [])}
    replaced = {Path(rel).name.lower() for rel in (extra_files or []) if Path(rel).exists()}
    with zipfile.ZipFile(raw_file) as src, zipfile.ZipFile(output_zip, "w", zipfile.ZIP_DEFLATED) as dst:
        root_exes = [i.filename.replace("\\", "/") for i in src.infolist()
                     if i.filename.lower().endswith(".exe") and "/" not in i.filename.replace("\\", "/")]
        app_exes = [n for n in root_exes if n.lower() not in OTHER_COMPONENT_EXES]
        has_exe = exe_name and exe_name.lower() in {n.lower() for n in root_exes}
        rename = app_exes[0] if exe_name and not has_exe and len(app_exes) == 1 else None
        for info in src.infolist():
            name = info.filename.replace("\\", "/")
            if name.lower() in replaced:
                continue
            if (name.lower() in OTHER_COMPONENT_EXES and exe_name and name.lower() != exe_name.lower()
                    and name.lower() not in keep):
                print(f"    - {name} (shipped by its own component)")
                continue
            if name == rename:
                print(f"    ~ {name} -> {exe_name}")
                name = exe_name
            entry = zipfile.ZipInfo(name, info.date_time)
            entry.external_attr = info.external_attr
            entry.compress_type = zipfile.ZIP_DEFLATED
            dst.writestr(entry, src.read(info))
        for rel in (extra_files or []):
            extra = Path(rel)
            if extra.exists():
                dst.write(extra, extra.name)
                print(f"    + {extra.name} (shared module)")
    print(f"    → {output_zip.name} (repacked from {raw_file.name})")


def copy_or_repackage(raw_file: Path, output_zip: Path, exe_name: str | None,
                      extra_files: list[str] | None = None, bundles: list[str] | None = None):
    """raw 파일이 exe면 zip으로 감싸고, 이미 zip이면 표준 형태로 다시 담는다.
    출력이 exe면 감싸지 않는다 (zip으로 받았으면 그 안의 `exe_name`을 꺼낸다)"""
    if output_zip.suffix.lower() == ".exe":
        if raw_file.suffix.lower() == ".zip":
            with zipfile.ZipFile(raw_file) as src:
                member = next((i for i in src.infolist()
                               if i.filename.replace("\\", "/").lower() == (exe_name or "").lower()), None)
                if member is None:
                    sys.exit(f"  ✗ {raw_file.name} has no {exe_name} at its root")
                output_zip.write_bytes(src.read(member))
        else:
            shutil.copy2(raw_file, output_zip)
        print(f"    → {output_zip.name} (raw exe)")
    elif raw_file.suffix.lower() == ".zip":
        repack_zip(raw_file, output_zip, exe_name, extra_files, bundles)
    elif raw_file.suffix.lower() == ".exe" and exe_name:
        create_exe_zip(raw_file, output_zip, exe_name, extra_files)
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


def parse_tag_version(tag: str) -> tuple | None:
    """`v0.2.0-beta.10` → 비교용 키. beta 번호는 숫자로 비교하고, 정식판이 같은 번호의 prerelease보다 높다."""
    m = re.fullmatch(r"v?(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?", tag)
    if not m:
        return None
    core = tuple(int(x) for x in m.group(1, 2, 3))
    if not m.group(4):
        return core + ((1,),)
    pre = tuple((0, int(p), "") if p.isdigit() else (1, 0, p) for p in m.group(4).split("."))
    return core + ((0,) + pre,)


def previous_release_candidates(releases: list[dict], current_tag: str) -> list[dict]:
    """부분 릴리즈에서 에셋을 가져올 수 있는 이전 릴리즈: 현재보다 낮은 버전만, 높은 것부터.
    정식판은 prerelease에서 가져오지 않는다 (API 순서는 만든 순서라 버전 순서가 아니다)."""
    current = parse_tag_version(current_tag)
    if current is None:
        return []
    current_is_pre = current[3] != (1,)
    picked = []
    for release in releases:
        key = parse_tag_version(release.get("tag", ""))
        if key is None or key >= current:
            continue
        # GitHub 표시가 빠졌어도 태그가 prerelease면 정식판에 쓰지 않는다
        if (release.get("prerelease") or key[3] != (1,)) and not current_is_pre:
            continue
        picked.append((key, release))
    return [r for _, r in sorted(picked, key=lambda kr: kr[0], reverse=True)]


def zip_has_root_file(path: Path, name: str) -> bool:
    """zip 루트에 `name`이 있는지 (대소문자 무시, `\\` 구분자도 허용)"""
    with zipfile.ZipFile(path) as zf:
        return any(i.filename.replace("\\", "/").lower() == name.lower() for i in zf.infolist())


def download_from_previous_release(
    asset_name: str,
    current_tag: str,
    releases: list[dict],
) -> tuple[Path, str] | None:
    """이전 릴리즈에서 에셋을 찾아 output/ 에 직접 다운로드

    최신 릴리즈부터 순회하며 manifest.json도 함께 실린 첫 번째 릴리즈에서 받는다.
    (파일, 그 릴리즈 태그)를 돌려준다: 버전은 반드시 같은 태그의 manifest에서 읽는다.
    """
    repo = os.environ.get("REPO_FULL_NAME", "")
    if not repo:
        return None

    for release in previous_release_candidates(releases, current_tag):
        tag = release["tag"]

        if asset_name not in release.get("assets", []) or "manifest.json" not in release.get("assets", []):
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
                    return dest, tag
            else:
                print(f"    ⚠ download failed: {result.stderr.strip()}")
        except subprocess.TimeoutExpired:
            print(f"    ⚠ download timed out for {asset_name} from {tag}")

    return None


def read_version_from_previous_manifest(
    comp_key: str,
    tag: str,
) -> tuple[str, str | None] | None:
    """에셋을 받아 온 바로 그 릴리즈(tag)의 manifest.json에서 (버전, sha256)을 읽는다.

    이전 릴리즈의 바이너리를 가져올 때, 현재 소스의 버전이 아니라
    해당 빌드 시점의 manifest에 기록된 버전을 사용해야 정확하다.
    """
    repo = os.environ.get("REPO_FULL_NAME", "")
    if not repo:
        return None

    try:
        result = subprocess.run(
            ["gh", "api", f"repos/{repo}/releases/tags/{tag}",
             "--jq", '.assets[] | select(.name == "manifest.json") | .url'],
            capture_output=True, text=True, timeout=15,
        )
        asset_url = result.stdout.strip() if result.returncode == 0 else ""
        if not asset_url:
            return None
        manifest_result = subprocess.run(
            ["gh", "api", asset_url, "-H", "Accept: application/octet-stream"],
            capture_output=True, text=True, timeout=15,
        )
        if manifest_result.returncode != 0:
            return None
        comp_info = json.loads(manifest_result.stdout).get("components", {}).get(comp_key, {})
    except (subprocess.TimeoutExpired, json.JSONDecodeError):
        return None
    version = comp_info.get("version")
    return (version, comp_info.get("sha256")) if version else None


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
    from_current = []   # 이번 릴리즈에서 패키징된 컴포넌트
    from_previous = []  # 이전 릴리즈에서 가져온 컴포넌트
    skipped = []        # 에셋을 찾지 못한 컴포넌트

    for comp in COMPONENTS:
        key = comp["key"]
        print(f"[{key}]")

        # 1. 버전 읽기 (현재 소스 기준 — 이전 릴리즈에서 가져올 경우 덮어씌움)
        version = read_bundled_version(key)

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
                extra = list(comp.get("extra_files", []))
                for pattern in comp.get("extra_staged", []):
                    staged = find_raw_file([pattern])
                    if staged:
                        extra.append(str(staged))
                    elif raw_file.suffix.lower() == ".zip" and all(
                            zip_has_root_file(raw_file, b) for b in comp.get("bundles", [])):
                        print(f"    {pattern} not in staging — using the copy bundled in {raw_file.name}")
                    else:
                        # exe만 올리면 데몬 옆에 있어야 할 파일이 빠진 채 서명된다
                        sys.exit(f"  ✗ {key}: {pattern} not in staging; upload it with {raw_file.name}")
                copy_or_repackage(raw_file, output_path, comp.get("exe_name"), extra, comp.get("bundles"))
                asset_source = "current"
            else:
                # 이번 릴리즈에 없음 → 이전 릴리즈에서 가져오기
                print(f"  raw: (not in staging, searching previous releases...)")
                downloaded = download_from_previous_release(
                    comp["output_name"], release_tag, previous_releases,
                )
                if downloaded:
                    downloaded, prev_tag = downloaded
                    asset_source = "previous"
                    # 버전과 sha는 이 파일을 받아 온 바로 그 릴리즈의 manifest에서만 읽는다
                    prev = read_version_from_previous_manifest(key, prev_tag)
                    if not prev:
                        # 실제 버전을 모르는 바이너리를 소스 버전으로 서명하면 안 된다
                        sys.exit(f"  ✗ {key}: reused {comp['output_name']} from {prev_tag} but its version is unknown "
                                 f"(manifest unreadable); build it in this release instead")
                    prev_version, prev_sha = prev
                    if prev_sha and sha256_file(downloaded) != prev_sha:
                        sys.exit(f"  ✗ {key}: {comp['output_name']} from {prev_tag} does not match that release's manifest sha256")

                    # 이전 릴리즈 zip에도 공유 모듈 주입
                    extra = comp.get("extra_files")
                    if extra and downloaded.suffix.lower() == ".zip":
                        with zipfile.ZipFile(downloaded, "a", zipfile.ZIP_DEFLATED) as zf:
                            existing = set(zf.namelist())
                            for rel in extra:
                                src = Path(rel)
                                if src.exists() and src.name not in existing:
                                    zf.write(src, src.name)
                                    print(f"    + {src.name} (shared module injected)")

                    if prev_version != version:
                        print(f"  version: {prev_version} (from previous manifest, overrides source {version})")
                        version = prev_version
                else:
                    print(f"  ⚠ NOT FOUND in any release — skipping")
                    skipped.append(key)
                    continue


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

    # release_version = versions.toml의 [release] version
    import tomllib
    release_version = tomllib.loads(Path("versions.toml").read_text(encoding="utf-8"))["release"]["version"]

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
