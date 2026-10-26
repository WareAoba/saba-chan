#!/usr/bin/env python3
"""versions.toml 하나로 모든 버전을 관리한다.

  sync      versions.toml의 버전을 각 컴포넌트의 files와 Cargo.lock에 써 넣는다
  check     어긋난 파일이 있으면 목록을 출력하고 1로 끝난다 (CI)
  manifest  빌드된 에셋 폴더로 릴레이용 릴리즈 manifest(서명 전 JSON)를 만든다.
            서명은 `cargo run -p saba-chan-updater-lib --example release_signing -- sign`.
  registration  서명된 봉투로 릴레이 `POST /admin/releases` 본문을 만든다.
            `release-notes/<version>.json`({언어: 본문})을 notes로 같이 싣는다(stable은 필수).

manifest에 --modules-repo / --extensions-repo를 주면 그 저장소의 모듈(module.toml이 있는
폴더)과 익스텐션(manifest.json이 있는 폴더)을 zip으로 묶어 같은 manifest에 넣는다 (U9e).
키는 module-<이름>, ext-<id>, 버전은 각자의 module.toml / manifest.json, 목록 화면용
정보는 meta에 든다.

manifest 형식은 운영 중인 릴레이(0.1.1)와 같다:
  {channel, platform, version, components: {<키>: {asset, install_dir, sha256, size_bytes, url, version}}}
"""

from __future__ import annotations

import argparse
import hashlib
import json
import io
import re
import struct
import sys
import tomllib
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / "versions.toml"
SEMVER = re.compile(r"^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$")
# 발행 채널: 정식·미리보기(rc)·베타. 위 채널은 아래 채널 릴리즈도 받는다 (updater relay.rs `CHANNELS`)
CHANNELS = ("stable", "rc", "beta")


def load() -> dict:
    data = tomllib.loads(SOURCE.read_text(encoding="utf-8"))
    errors = []
    release = data.get("release", {}).get("version", "")
    if not SEMVER.match(release):
        errors.append(f"[release] version {release!r} is not semver")
    channel = data.get("release", {}).get("channel", "stable")
    if channel not in CHANNELS:
        errors.append(f"[release] channel {channel!r} is not one of {', '.join(CHANNELS)}")
    elif channel == "stable" and "-" in release:
        errors.append(f"[release] stable channel cannot carry a prerelease version ({release})")
    for key, comp in data.get("components", {}).items():
        if not SEMVER.match(comp.get("version", "")):
            errors.append(f"[components.{key}] version {comp.get('version')!r} is not semver")
        for f in comp.get("files", []):
            if not (ROOT / f).is_file():
                errors.append(f"[components.{key}] file {f} does not exist")
    if errors:
        sys.exit("versions.toml:\n  " + "\n  ".join(errors))
    return data


# ── 파일 형식별 읽기·쓰기 ─────────────────────────────────────

CARGO_PACKAGE_VERSION = re.compile(r'(?ms)^(\[package\][^\[]*?^version\s*=\s*")([^"]*)(")')


def cargo_get(text: str) -> str | None:
    m = CARGO_PACKAGE_VERSION.search(text)
    return m.group(2) if m else None


def cargo_set(text: str, version: str) -> str:
    return CARGO_PACKAGE_VERSION.sub(lambda m: m.group(1) + version + m.group(3), text, count=1)


def cargo_name(text: str) -> str | None:
    m = re.search(r'(?ms)^\[package\][^\[]*?^name\s*=\s*"([^"]*)"', text)
    return m.group(1) if m else None


def json_get(text: str) -> str | None:
    return json.loads(text).get("version")


def json_set(text: str, version: str) -> str:
    data = json.loads(text)
    if json.dumps(data, indent=2, ensure_ascii=False) + "\n" == text:
        # 형식을 그대로 재현할 수 있으면 구조적으로 고친다 (package-lock의 packages."" 포함)
        data["version"] = version
        if "" in data.get("packages", {}):
            data["packages"][""]["version"] = version
        return json.dumps(data, indent=2, ensure_ascii=False) + "\n"
    # 아니면 첫 "version"(최상위)만 바꿔 원래 형식을 지킨다
    return re.sub(r'("version"\s*:\s*")[^"]*(")', lambda m: m.group(1) + version + m.group(2), text, count=1)


def json_lock_extra(text: str) -> str | None:
    """package-lock의 packages."" 버전 (최상위와 같아야 한다)"""
    return json.loads(text).get("packages", {}).get("", {}).get("version")


def file_versions(path: Path) -> list[str | None]:
    text = path.read_text(encoding="utf-8")
    if path.name == "Cargo.toml":
        return [cargo_get(text)]
    if path.suffix == ".json":
        found = [json_get(text)]
        if path.name == "package-lock.json":
            found.append(json_lock_extra(text))
        return found
    raise SystemExit(f"unsupported version file: {path}")


def set_file_version(path: Path, version: str) -> bool:
    text = path.read_text(encoding="utf-8")
    new = cargo_set(text, version) if path.name == "Cargo.toml" else json_set(text, version)
    if new != text:
        path.write_bytes(new.encode("utf-8"))
        return True
    return False


# ── Cargo.lock: 워크스페이스 크레이트 항목 ─────────────────────

def cargo_targets(data: dict) -> dict[str, str]:
    """크레이트 이름 → 기대 버전"""
    out = {}
    for comp in data["components"].values():
        for f in comp.get("files", []):
            if f.endswith("Cargo.toml"):
                name = cargo_name((ROOT / f).read_text(encoding="utf-8"))
                if name:
                    out[name] = comp["version"]
    return out


LOCK_ENTRY = re.compile(r'(\[\[package\]\]\nname = "([^"]+)"\nversion = ")([^"]*)(")')


def lock_mismatches(data: dict) -> list[str]:
    lock = ROOT / "Cargo.lock"
    if not lock.is_file():
        return []
    targets = cargo_targets(data)
    return [
        f"Cargo.lock {m.group(2)} = {m.group(3)} (expected {targets[m.group(2)]})"
        for m in LOCK_ENTRY.finditer(lock.read_text(encoding="utf-8"))
        if m.group(2) in targets and m.group(3) != targets[m.group(2)]
    ]


def lock_sync(data: dict) -> bool:
    lock = ROOT / "Cargo.lock"
    if not lock.is_file():
        return False
    targets = cargo_targets(data)
    text = lock.read_text(encoding="utf-8")
    new = LOCK_ENTRY.sub(
        lambda m: m.group(1) + targets.get(m.group(2), m.group(3)) + m.group(4), text
    )
    if new != text:
        lock.write_bytes(new.encode("utf-8"))
        return True
    return False


# ── 명령 ───────────────────────────────────────────────────

def cmd_check(data: dict) -> int:
    problems = []
    for key, comp in data["components"].items():
        for f in comp.get("files", []):
            for found in file_versions(ROOT / f):
                if found != comp["version"]:
                    problems.append(f"{f} = {found} (versions.toml {key} = {comp['version']})")
    problems += lock_mismatches(data)
    if problems:
        print("버전이 versions.toml과 다릅니다. `python scripts/versions.py sync`를 실행하세요:")
        for p in problems:
            print("  " + p)
        return 1
    print(f"OK: release {data['release']['version']}, {len(data['components'])} components")
    return 0


def cmd_sync(data: dict) -> int:
    changed = [
        f
        for comp in data["components"].values()
        for f in comp.get("files", [])
        if set_file_version(ROOT / f, comp["version"])
    ]
    if lock_sync(data):
        changed.append("Cargo.lock")
    for f in changed:
        print("updated " + f)
    if not changed:
        print("already in sync")
    return 0


def r2_key(release: str, asset: str) -> str:
    return f"releases/{release}/{asset}"


SKIP_PARTS = {"__pycache__", ".git", ".pytest_cache", "tests"}


def zip_folder(src: Path, dest: Path) -> None:
    """폴더 안 파일을 zip 루트에 넣는다 (캐시·테스트 제외, 순서 고정)."""
    import zipfile

    with zipfile.ZipFile(dest, "w", zipfile.ZIP_DEFLATED) as zf:
        for path in sorted(src.rglob("*")):
            rel = path.relative_to(src)
            if path.is_dir() or SKIP_PARTS & set(rel.parts) or path.suffix == ".pyc":
                continue
            zf.write(path, rel.as_posix())


def raw_url(repo: str, ref: str | None, rel: str) -> str | None:
    if not ref:
        return None
    return f"https://raw.githubusercontent.com/{repo}/{ref}/{rel}"


def extension_core_requirement(ext: dict) -> str | None:
    """익스텐션이 요구하는 saba-core 버전 (`dependencies."saba-core"`, 없으면 `min_app_version`). 업데이터가 `requires`로 검사한다"""
    deps = ext.get("dependencies")
    core = deps.get("saba-core") if isinstance(deps, dict) else None
    if core and core != "*":
        return core
    return f">={ext['min_app_version']}" if ext.get("min_app_version") else None


def addon_components(args: argparse.Namespace, assets: Path) -> dict:
    """모듈·익스텐션 저장소를 zip으로 묶고 `{키: {version, asset, install_dir, meta, requires?}}`를 돌려준다."""
    found = {}
    if args.modules_repo:
        root = Path(args.modules_repo)
        for toml_path in sorted(root.glob("*/module.toml")):
            folder = toml_path.parent
            if folder.name.startswith(("_", ".")):
                continue
            mod = tomllib.loads(toml_path.read_text(encoding="utf-8"))
            info = mod.get("module", {})
            name = info.get("name", folder.name)
            asset = f"module-{name}.zip"
            zip_folder(folder, assets / asset)
            meta = {
                "display_name": info.get("display_name") or info.get("game_name") or name,
                "game_name": info.get("game_name"),
                "description": info.get("description", ""),
                "requires_extensions": mod.get("install", {}).get("requires_extensions", []),
            }
            icon = raw_url("WareAoba/saba-chan-modules", args.addons_ref, f"{folder.name}/{info.get('icon', 'icon.png')}")
            if icon and (folder / info.get("icon", "icon.png")).is_file():
                meta["icon"] = icon
            comp = {"version": info["version"], "asset": asset, "install_dir": f"modules/{name}",
                    "meta": {k: v for k, v in meta.items() if v is not None}}
            core = info.get("dependencies", {}).get("saba-core")
            if core:
                comp["requires"] = {"saba-core": core}
            found[f"module-{name}"] = comp
    if args.extensions_repo:
        root = Path(args.extensions_repo)
        for man_path in sorted(root.glob("*/manifest.json")):
            folder = man_path.parent
            ext = json.loads(man_path.read_text(encoding="utf-8"))
            ext_id = ext["id"]
            asset = f"ext-{ext_id}.zip"
            zip_folder(folder, assets / asset)
            deps = ext.get("dependencies", [])
            meta = {
                "name": ext.get("name", ext_id),
                "description": ext.get("description", ""),
                "author": ext.get("author", ""),
                "min_app_version": ext.get("min_app_version"),
                "dependencies": deps,
                "has_gui": "gui" in ext,
                "has_i18n": (folder / "i18n").is_dir(),
            }
            comp = {"version": ext["version"], "asset": asset, "install_dir": f"extensions/{folder.name}",
                    "meta": {k: v for k, v in meta.items() if v is not None}}
            core = extension_core_requirement(ext)
            if core:
                comp["requires"] = {"saba-core": core}
            found[f"ext-{ext_id}"] = comp
    return found


def zip_contents(blob: bytes) -> dict[str, str]:
    """zip 안 파일마다 sha256 (zip 시각·압축 방식은 보지 않는다)"""
    with zipfile.ZipFile(io.BytesIO(blob)) as zf:
        return {i.filename: hashlib.sha256(zf.read(i)).hexdigest() for i in zf.infolist() if not i.is_dir()}


def addon_version_problems(components: dict, assets: Path, previous: dict, fetch) -> list[str]:
    """지난 릴리즈(`previous` manifest)보다 버전이 낮거나, 같은 버전인데 파일이 바뀐 모듈·익스텐션.
    업데이터·데몬은 버전이 오른 것만 받으므로, 바뀐 것을 같은 버전으로 실으면 이미 깐 사용자는 받지 못한다.
    같은 버전이면 지난 zip을 `fetch(url)`로 받아 파일 내용을 비교한다"""
    problems = []
    for key, comp in components.items():
        old = previous.get("components", {}).get(key)
        if not old or not key.startswith(("module-", "ext-")):
            continue
        new_v, old_v = (versions_semver(comp["version"]), versions_semver(old["version"]))
        if new_v < old_v:
            problems.append(f"{key} {comp['version']} is lower than {old['version']} in {previous.get('version')}")
        elif new_v == old_v and zip_contents((assets / comp["asset"]).read_bytes()) != zip_contents(fetch(old["url"])):
            problems.append(f"{key} changed since {previous.get('version')} but keeps version {comp['version']}; raise it")
    return problems


def versions_semver(version: str) -> tuple:
    """비교용 (major, minor, patch, 정식 여부, 시험판 식별자). 시험판은 정식보다 낮다"""
    core, _, pre = version.partition("-")
    nums = tuple(int(p) for p in core.split("."))
    ids = tuple((0, int(x), "") if x.isdigit() else (1, 0, x) for x in pre.split(".")) if pre else ()
    return (*nums, not pre, ids)


def fetch_url(url: str) -> bytes:
    import urllib.request

    with urllib.request.urlopen(url, timeout=60) as resp:
        return resp.read()


def exe_file_version(blob: bytes) -> str | None:
    """exe의 파일 버전 리소스(VS_FIXEDFILEINFO)의 FILEVERSION을 `major.minor.patch`로. 리소스가 없으면 None"""
    start = 0
    while (i := blob.find(b"\xbd\x04\xef\xfe", start)) >= 0:
        start = i + 4
        if len(blob) < i + 16:
            break
        struc, ms, ls = struct.unpack_from("<III", blob, i + 4)
        if struc == 0x00010000:
            return f"{ms >> 16}.{ms & 0xFFFF}.{ls >> 16}"
    return None


def asset_exe_versions(name: str, blob: bytes) -> dict[str, str]:
    """에셋(exe 또는 zip 루트의 exe)마다 박힌 파일 버전. 버전 리소스가 없는 exe(saba-runner 등)는 빠진다"""
    exes = {}
    if name.lower().endswith(".exe"):
        exes[name] = blob
    elif name.lower().endswith(".zip"):
        with zipfile.ZipFile(io.BytesIO(blob)) as zf:
            for info in zf.infolist():
                inner = info.filename.replace("\\", "/")
                if inner.lower().endswith(".exe") and "/" not in inner:
                    exes[inner] = zf.read(info)
    return {n: v for n, b in exes.items() if (v := exe_file_version(b))}


def check_exe_versions(key: str, name: str, blob: bytes, version: str) -> None:
    """서명할 버전과 올린 실행 파일의 버전이 다르면 멈춘다 (다른 판으로 빌드한 exe를 새 버전으로 서명하지 않게)"""
    has_exe = name.lower().endswith(".exe") or (
        name.lower().endswith(".zip") and any(n.lower().endswith(".exe") for n in zipfile.ZipFile(io.BytesIO(blob)).namelist()))
    if not has_exe:
        return
    found = asset_exe_versions(name, blob)
    if not found:
        sys.exit(f"{key}: no executable in {name} carries a file version to compare with {version}")
    expected = ".".join(str(int(p)) for p in re.findall(r"\d+", version.split("-")[0])[:3])
    wrong = {n: v for n, v in found.items() if v != expected}
    if wrong:
        sys.exit(f"{key}: {name} was built as {wrong}, not {version}")


def platform_components(data: dict, args: argparse.Namespace, assets: Path, platform: str, packaged: dict) -> dict:
    """이 플랫폼의 앱 컴포넌트. `{platform}`이 든 asset은 그 플랫폼 파일이 있을 때만, 없는 asset은
    `platforms`(없으면 모든 플랫폼)에 들 때만 싣는다"""
    base = args.base_url.rstrip("/")
    release = data["release"]["version"]
    components = {}
    for key, comp in data["components"].items():
        if "asset" not in comp:
            continue
        if "{platform}" not in comp["asset"] and platform not in comp.get("platforms", [platform]):
            continue
        name = comp["asset"].format(platform=platform)
        path = assets / name
        if not path.is_file():
            print(f"skip {key} ({platform}): {path} not found", file=sys.stderr)
            continue
        blob = path.read_bytes()
        sha = hashlib.sha256(blob).hexdigest()
        version = comp["version"]
        pkg = packaged.get(key)
        if pkg:
            if pkg.get("sha256") and pkg["sha256"] != sha:
                sys.exit(f"{key}: {name} differs from the packaged one (sha256 mismatch)")
            if pkg.get("version") and pkg["version"] != version:
                print(f"{key}: reused asset is {pkg['version']}, not {version}; signing it as {pkg['version']}", file=sys.stderr)
                version = pkg["version"]
        check_exe_versions(key, name, blob, version)
        components[key] = {
            "asset": name,
            "install_dir": comp.get("install_dir", "."),
            "sha256": sha,
            "size_bytes": len(blob),
            "url": f"{base}/{r2_key(release, name)}",
            "version": version,
        }
    return components


def cmd_manifest(data: dict, args: argparse.Namespace) -> int:
    """에셋 폴더에서 각 컴포넌트의 `asset` 파일을 찾아 manifest를 만든다. 없는 컴포넌트는 빠진다.
    `--also-platform`의 플랫폼은 `platforms.<플랫폼>.components`에 그 플랫폼의 목록 전체를 싣는다 (D3c)."""
    assets = Path(args.assets)
    base = args.base_url.rstrip("/")
    release = data["release"]["version"]
    # 패키저(package-release.py)가 남긴 manifest.json: 이전 릴리즈에서 가져온 에셋은 실제 버전이 적혀 있다.
    # 패키저는 맨 위 플랫폼(windows-x64)만 다룬다
    packaged_path = assets / "manifest.json"
    packaged = json.loads(packaged_path.read_text(encoding="utf-8")).get("components", {}) if packaged_path.is_file() else {}
    components = platform_components(data, args, assets, args.platform, packaged)
    addons = addon_components(args, assets)
    if getattr(args, "previous", None):
        previous = json.loads(Path(args.previous).read_text(encoding="utf-8"))
        problems = addon_version_problems(addons, assets, previous.get("manifest", previous), fetch_url)
        if problems:
            sys.exit("modules/extensions:\n  " + "\n  ".join(problems))
    addon_entries = {}
    for key, comp in addons.items():
        blob = (assets / comp["asset"]).read_bytes()
        addon_entries[key] = {
            **comp,
            "sha256": hashlib.sha256(blob).hexdigest(),
            "size_bytes": len(blob),
            "url": f"{base}/{r2_key(release, comp['asset'])}",
        }
    components.update(addon_entries)
    if not components:
        sys.exit(f"no assets found in {assets}")
    manifest = {
        "channel": args.channel or data["release"].get("channel", "stable"),
        "components": components,
        "platform": args.platform,
        "version": release,
    }
    platforms = {}
    for other in getattr(args, "also_platform", None) or []:
        own = platform_components(data, args, assets, other, {})
        if "saba-core" not in own:
            sys.exit(f"{other}: no saba-core asset in {assets}")
        platforms[other] = {"components": {**own, **addon_entries}}
    if platforms:
        manifest["platforms"] = platforms
    write_json(manifest, args.out)
    return 0


def cmd_registration(args: argparse.Namespace) -> int:
    """`POST /admin/releases` 본문. signedManifest는 서명 봉투 전체를 JSON 문자열로 넣는다."""
    signed = json.loads(Path(args.signed).read_text(encoding="utf-8"))
    manifest = signed["manifest"]
    release = manifest["version"]
    notes_path = Path(args.notes) if args.notes else ROOT / "release-notes" / f"{release}.json"
    notes = load_notes(notes_path)
    # 클라이언트는 받을 때의 노트를 보관해 적용 뒤 패치노트로 보여 준다. stable은 노트 없이 내지 않는다
    if notes is None and manifest["channel"] == "stable":
        raise SystemExit(f"{notes_path}: stable release needs release notes")
    body = {
        "version": release,
        "channel": manifest["channel"],
        "rolloutPercent": args.rollout,
        "isCritical": args.critical,
        "signedManifest": json.dumps(signed, ensure_ascii=False, separators=(",", ":"), sort_keys=True),
        "notes": notes,
        "assets": [
            {
                "component": key,
                "platform": platform,
                "filename": comp["asset"],
                "sizeBytes": comp["size_bytes"],
                "sha256": comp["sha256"],
                "r2Key": r2_key(release, comp["asset"]),
            }
            # 릴레이는 에셋의 플랫폼으로 그 플랫폼의 릴리즈를 고른다. 같은 파일(모듈·locales)은 플랫폼마다 한 줄씩
            for platform, listed in [(manifest["platform"], manifest["components"])]
            + [(p, v["components"]) for p, v in sorted(manifest.get("platforms", {}).items())]
            for key, comp in sorted(listed.items())
        ],
    }
    write_json(body, args.out)
    return 0


NOTES_LANG = re.compile(r"^[a-z]{2,3}(-[A-Za-z0-9]{2,8})*$")


def load_notes(path: Path) -> dict | None:
    """릴리즈 노트 {언어: 본문}. 서명 밖의 표시용이라 파일이 없으면 None(노트 없음).
    릴레이가 거부할 모양(`routes/updates.ts` parseNotes)이면 등록 전에 여기서 멈춘다."""
    if not path.is_file():
        return None
    notes = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(notes, dict) or not notes or len(notes) > 20:
        raise SystemExit(f"{path}: notes must be a non-empty object of at most 20 languages")
    for lang, text in notes.items():
        if not NOTES_LANG.match(lang) or not isinstance(text, str) or not text.strip() or len(text) > 8000:
            raise SystemExit(f"{path}: invalid note for {lang!r}")
    return notes


def write_json(value: dict, out: str | None) -> None:
    # 한글은 \uXXXX로 내보낸다. 본문이 ASCII뿐이라 PowerShell 5.1 Invoke-RestMethod처럼
    # UTF-8로 보내지 않는 클라이언트를 거쳐도 릴레이에 ???로 깨져 들어가지 않는다
    text = json.dumps(value, indent=2, ensure_ascii=True) + "\n"
    if out:
        Path(out).write_text(text, encoding="utf-8")
    else:
        sys.stdout.write(text)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="cmd", required=True)
    sub.add_parser("check")
    sub.add_parser("sync")
    m = sub.add_parser("manifest")
    m.add_argument("--assets", required=True, help="빌드된 zip 폴더")
    m.add_argument("--base-url", required=True, help="R2 공개 URL 접두사 (<base>/<version>/<file>)")
    m.add_argument("--platform", required=True, help="windows-x64, linux-x64 ...")
    m.add_argument("--also-platform", action="append", default=[], help="platforms.<플랫폼>에 함께 실을 플랫폼 (여러 번)")
    m.add_argument("--channel")
    m.add_argument("--out")
    m.add_argument("--modules-repo", help="saba-chan-modules 체크아웃 (모듈을 같이 싣는다)")
    m.add_argument("--extensions-repo", help="saba-chan-extensions 체크아웃")
    m.add_argument("--addons-ref", help="아이콘 URL에 쓸 모듈 저장소 커밋")
    m.add_argument("--previous", help="지난 릴리즈의 (서명된) manifest. 바뀐 모듈·익스텐션의 버전이 오르지 않았으면 멈춘다")
    r = sub.add_parser("registration")
    r.add_argument("--signed", required=True, help="release_signing sign 출력")
    r.add_argument("--rollout", type=int, default=100)
    r.add_argument("--critical", action="store_true")
    r.add_argument("--notes", help="릴리즈 노트 JSON (기본: release-notes/<version>.json)")
    r.add_argument("--out")
    args = parser.parse_args()

    if args.cmd == "registration":
        return cmd_registration(args)

    data = load()
    if args.cmd == "check":
        return cmd_check(data)
    if args.cmd == "sync":
        return cmd_sync(data)
    return cmd_manifest(data, args)


if __name__ == "__main__":
    sys.exit(main())
