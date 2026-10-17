#!/usr/bin/env python3
"""커밋 기록으로 릴리즈 노트(마크다운)를 만든다 (U11).

    python scripts/changelog.py                 # 마지막 v* 태그부터 HEAD까지
    python scripts/changelog.py --since v0.1.1  # 지정한 태그부터
    python scripts/changelog.py --until v0.2.0 --since v0.1.1 --version 0.2.0

커밋 제목 규칙 (CONTRIBUTING.md):
    <종류>(<범위>): <요약>        예) feat(A1): 크래시면 서버를 다시 띄운다
    종류: feat fix perf refactor docs test build ci chore
    범위는 선택. 로드맵 ID(A1, C12)나 부품 이름(daemon, gui, cli, updater, module, ext)을 쓴다.
    호환을 깨는 변경은 종류 뒤에 '!'를 붙인다. 예) feat(api)!: ...

규칙을 따르지 않은 예전 커밋은 "기타"로 모은다. 병합 커밋은 뺀다.
"""

import argparse
import re
import subprocess
import sys

SECTIONS = [
    ("breaking", "호환이 깨지는 변경"),
    ("feat", "새 기능"),
    ("fix", "고친 것"),
    ("perf", "성능"),
    ("refactor", "정리"),
    ("docs", "문서"),
    ("other", "기타"),
]
HIDDEN = {"test", "build", "ci", "chore"}
SUBJECT = re.compile(r"^(?P<type>[a-z]+)(?:\((?P<scope>[^)]+)\))?(?P<bang>!)?:\s*(?P<summary>.+)$")


def git(*args: str) -> str:
    return subprocess.run(["git", *args], check=True, capture_output=True, text=True).stdout


def last_tag(until: str) -> str | None:
    try:
        return git("describe", "--tags", "--abbrev=0", "--match", "v*", f"{until}^").strip() or None
    except subprocess.CalledProcessError:
        return None


def classify(subject: str) -> tuple[str, str]:
    m = SUBJECT.match(subject)
    if not m:
        return "other", subject
    kind = m["type"]
    text = f"**{m['scope']}**: {m['summary']}" if m["scope"] else m["summary"]
    if m["bang"]:
        return "breaking", text
    if kind in HIDDEN:
        return "hidden", text
    return (kind if kind in dict(SECTIONS) else "other"), text


def render(subjects: list[str], version: str | None, since: str | None) -> str:
    groups: dict[str, list[str]] = {}
    for s in subjects:
        kind, text = classify(s)
        groups.setdefault(kind, []).append(text)
    title = f"## {version}" if version else "## 변경 사항"
    lines = [title, ""]
    if since:
        lines += [f"{since} 이후", ""]
    for key, label in SECTIONS:
        items = groups.get(key)
        if items:
            lines.append(f"### {label}")
            lines += [f"- {t}" for t in items]
            lines.append("")
    if len(lines) <= 4:
        lines.append("눈에 띄는 변경이 없습니다.")
    return "\n".join(lines).rstrip() + "\n"


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--since", help="시작 태그/커밋 (없으면 마지막 v* 태그)")
    ap.add_argument("--until", default="HEAD", help="끝 태그/커밋 (기본 HEAD)")
    ap.add_argument("--version", help="제목에 쓸 버전")
    ap.add_argument("--output", help="파일로 쓰기 (없으면 표준 출력)")
    args = ap.parse_args()

    since = args.since or last_tag(args.until)
    rev = f"{since}..{args.until}" if since else args.until
    subjects = [s for s in git("log", "--no-merges", "--format=%s", rev).splitlines() if s.strip()]
    text = render(subjects, args.version, since)
    if args.output:
        with open(args.output, "w", encoding="utf-8") as f:
            f.write(text)
    else:
        sys.stdout.write(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
