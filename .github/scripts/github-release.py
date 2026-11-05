"""릴레이에 등록된 릴리스를 GitHub Release에 올릴 파일로 받는다.

    python .github/scripts/github-release.py --relay <RELAY_URL> --version <ver> \
        --notes release-notes/<ver>.json --out gh-release --notes-out gh-release-notes.md

릴레이의 서명된 manifest가 가리키는 R2 파일을 받아 sha256·크기를 manifest와 대조한다.
그래서 GitHub에 올라가는 파일은 앱이 받는 파일과 바이트까지 같다. R2는 읽기만 한다.
<out>에는 올릴 파일만 둔다: asset들, release-manifest.json(서명된 manifest 원문), SHA256SUMS.txt.
본문은 --notes-out에 쓴다.
"""

import argparse
import hashlib
import json
import sys
import urllib.request
from pathlib import Path


def fail(msg):
    print(f"::error::{msg}", file=sys.stderr)
    raise SystemExit(1)


def fetch_url(url, headers=None):
    # Cloudflare가 urllib 기본 User-Agent를 막는다
    req = urllib.request.Request(url, headers={"User-Agent": "saba-chan-release-workflow", **(headers or {})})
    with urllib.request.urlopen(req, timeout=120) as r:
        return r.read()


def assets(envelope, version):
    """manifest의 모든 플랫폼 구성요소를 asset 이름당 하나로 모은다."""
    m = envelope.get("manifest") or {}
    if m.get("version") != version:
        fail(f"relay manifest is {m.get('version')!r}, expected {version!r}")
    groups = [m.get("components") or {}]
    groups += [(p or {}).get("components") or {} for p in (m.get("platforms") or {}).values()]
    seen = {}
    for comps in groups:
        for c in comps.values():
            prev = seen.get(c["asset"])
            if prev and (prev["sha256"], prev["url"]) != (c["sha256"], c["url"]):
                fail(f"{c['asset']} has two different files in the manifest")
            seen[c["asset"]] = c
    return [seen[k] for k in sorted(seen)]


def download(a, out, fetch=fetch_url):
    name = a["asset"]
    if "/" in name or "\\" in name or name in ("", ".", ".."):
        fail(f"bad asset name {name!r}")
    data = fetch(a["url"])
    if len(data) != a["size_bytes"] or hashlib.sha256(data).hexdigest() != a["sha256"]:
        fail(f"{name} does not match the signed manifest")
    (out / name).write_bytes(data)


def notes(path):
    n = json.loads(Path(path).read_text(encoding="utf-8"))
    parts = [n.get("ko", "").strip(), n.get("en", "").strip()]
    body = "\n\n---\n\n".join(p for p in parts if p)
    return (body + "\n\n---\n\n"
            "앱은 이 파일들을 직접 받지 않고, 서명된 manifest(`release-manifest.json`)로 같은 파일을 받습니다. "
            "Windows 실행 파일은 아직 코드 서명이 없습니다. 받은 파일은 `SHA256SUMS.txt`로 확인할 수 있습니다.\n"
            "The app updates from the signed manifest (`release-manifest.json`), not from this page. "
            "Windows executables are not code-signed yet. Verify downloads with `SHA256SUMS.txt`.\n")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--relay", required=True)
    ap.add_argument("--version", required=True)
    ap.add_argument("--notes", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--notes-out", required=True)
    args = ap.parse_args()
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=False)
    raw = fetch_url(f"{args.relay.rstrip('/')}/v1/updates/manifest/{args.version}", {"X-Saba-Client": "1"})
    envelope = json.loads(raw)
    items = assets(envelope, args.version)
    for a in items:
        download(a, out)
        print(f"  {a['asset']} {a['sha256']}")
    (out / "release-manifest.json").write_bytes(raw)
    (out / "SHA256SUMS.txt").write_text(
        "".join(f"{a['sha256']}  {a['asset']}\n" for a in items), encoding="utf-8")
    Path(args.notes_out).write_text(notes(args.notes), encoding="utf-8")


if __name__ == "__main__":
    main()
