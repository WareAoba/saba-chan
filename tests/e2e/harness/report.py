"""pytest junit XML을 사람이 읽는 report.md로 바꾼다. 사용: python -m harness.report <junit.xml> <report.md> [제목]"""

import sys
import xml.etree.ElementTree as ET
from pathlib import Path


def main(junit: str, out: str, title: str = "saba-chan E2E 결과") -> int:
    root = ET.parse(junit).getroot()
    cases = root.iter("testcase")
    rows = {"통과": [], "실패": [], "건너뜀": [], "예상된 실패": []}
    for c in cases:
        name = f"{c.get('classname', '').split('.')[-1]}::{c.get('name')}"
        secs = float(c.get("time") or 0)
        failure = c.find("failure") if c.find("failure") is not None else c.find("error")
        skipped = c.find("skipped")
        if failure is not None:
            msg = (failure.get("message") or "").splitlines()[0][:300]
            rows["실패"].append(f"- `{name}` ({secs:.0f}초): {msg}")
        elif skipped is not None:
            kind = "예상된 실패" if (skipped.get("type") or "").endswith("xfail") else "건너뜀"
            rows[kind].append(f"- `{name}`: {(skipped.get('message') or '')[:200]}")
        else:
            rows["통과"].append(f"- `{name}` ({secs:.0f}초)")
    lines = [f"# {title}", "", " · ".join(f"{k} {len(v)}" for k, v in rows.items()), ""]
    for key in ("실패", "예상된 실패", "건너뜀", "통과"):
        if rows[key]:
            lines += [f"## {key}", *rows[key], ""]
    Path(out).write_text("\n".join(lines), encoding="utf-8")
    print("\n".join(lines[:3]))
    return 1 if rows["실패"] else 0


if __name__ == "__main__":
    sys.exit(main(*sys.argv[1:]))
