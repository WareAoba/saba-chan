#!/usr/bin/env python3
"""versions.toml 하나로 모든 버전을 관리한다.

  sync      versions.toml의 버전을 각 컴포넌트의 files와 Cargo.lock에 써 넣는다
  check     어긋난 파일이 있으면 목록을 출력하고 1로 끝난다 (CI)
  manifest  빌드된 에셋 폴더로 릴레이용 릴리즈 manifest(서명 전 JSON)를 만든다.
            서명은 `cargo run -p saba-chan-updater-lib --example release_signing -- sign`.
  registration  서명된 봉투로 릴레이 `POST /admin/releases` 본문을 만든다.

manifest 형식은 운영 중인 릴레이(0.1.1)와 같다:
  {channel, platform, version, components: {<키>: {asset, install_dir, sha256, size_bytes, url, version}}}
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / "versions.toml"
SEMVER = re.compile(r"^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$")


def load() -> dict:
    data = tomllib.loads(SOURCE.read_text(encoding="utf-8"))
    errors = []
    release = data.get("release", {}).get("version", "")
    if not SEMVER.match(release):
        errors.append(f"[release] version {release!r} is not semver")
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


def cmd_manifest(data: dict, args: argparse.Namespace) -> int:
    """에셋 폴더에서 각 컴포넌트의 `asset` 파일을 찾아 manifest를 만든다. 없는 컴포넌트는 빠진다."""
    assets = Path(args.assets)
    base = args.base_url.rstrip("/")
    release = data["release"]["version"]
    components = {}
    for key, comp in data["components"].items():
        if "asset" not in comp:
            continue
        name = comp["asset"].format(platform=args.platform)
        path = assets / name
        if not path.is_file():
            print(f"skip {key}: {path} not found", file=sys.stderr)
            continue
        blob = path.read_bytes()
        components[key] = {
            "asset": name,
            "install_dir": comp.get("install_dir", "."),
            "sha256": hashlib.sha256(blob).hexdigest(),
            "size_bytes": len(blob),
            "url": f"{base}/{r2_key(release, name)}",
            "version": comp["version"],
        }
    if not components:
        sys.exit(f"no assets found in {assets}")
    manifest = {
        "channel": args.channel or data["release"].get("channel", "stable"),
        "components": components,
        "platform": args.platform,
        "version": release,
    }
    write_json(manifest, args.out)
    return 0


def cmd_registration(args: argparse.Namespace) -> int:
    """`POST /admin/releases` 본문. signedManifest는 서명 봉투 전체를 JSON 문자열로 넣는다."""
    signed = json.loads(Path(args.signed).read_text(encoding="utf-8"))
    manifest = signed["manifest"]
    release = manifest["version"]
    body = {
        "version": release,
        "channel": manifest["channel"],
        "rolloutPercent": args.rollout,
        "isCritical": args.critical,
        "signedManifest": json.dumps(signed, ensure_ascii=False, separators=(",", ":"), sort_keys=True),
        "assets": [
            {
                "component": key,
                "platform": manifest["platform"],
                "filename": comp["asset"],
                "sizeBytes": comp["size_bytes"],
                "sha256": comp["sha256"],
                "r2Key": r2_key(release, comp["asset"]),
            }
            for key, comp in sorted(manifest["components"].items())
        ],
    }
    write_json(body, args.out)
    return 0


def write_json(value: dict, out: str | None) -> None:
    text = json.dumps(value, indent=2, ensure_ascii=False) + "\n"
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
    m.add_argument("--channel")
    m.add_argument("--out")
    r = sub.add_parser("registration")
    r.add_argument("--signed", required=True, help="release_signing sign 출력")
    r.add_argument("--rollout", type=int, default=100)
    r.add_argument("--critical", action="store_true")
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
