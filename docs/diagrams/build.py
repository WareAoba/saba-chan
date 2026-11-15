#!/usr/bin/env python3
"""사용자 문서용 다이어그램(SVG)을 만든다.

    python docs/diagrams/build.py                     # saba-chan README 그림만
    python docs/diagrams/build.py --modules ../saba-chan-modules

- saba-chan:         docs/images/architecture[-en|-ja]-{light,dark}.svg (README 한/영/일, GitHub 라이트/다크)
- saba-chan-modules: docs/images/module-call-{light,dark}.svg (모듈 개발 가이드 §2)

그림 내용은 코드 기준이다. 바뀌면 문구(strings.py)와 배치를 고치고 다시 만든다.
근거: 데몬 API·인증 src/ipc, 릴레이 연결 src/relay_agent/, 모듈 호출 src/plugin/mod.rs.
"""

import argparse
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from strings import ARCH, MODULE_CALL  # noqa: E402
from svgkit import COLORS, Svg  # noqa: E402


def architecture(lang, theme):
    s = ARCH[lang]
    g = Svg(980, 600, theme, lang, s["title"], s["desc"])
    t = g.t
    # 영역
    g.region(20, 20, 560, 480, s["pc"], COLORS["daemon"])
    g.region(735, 20, 225, 480, s["cloud"], COLORS["discord"])

    # 화살표를 먼저 그리고 이름표를 위에 얹는다
    g.arrow([(165, 158), (165, 222)])
    g.arrow([(435, 158), (435, 222)])
    for x in (123, 300, 477):
        g.arrow([(x, 312), (x, 392)])
    g.arrow([(555, 267), (757, 267)])
    g.arrow([(847, 222), (847, 158)], both=True)

    g.box(45, 70, 240, 88, "iface", s["gui"], s["guiSub"])
    g.box(315, 70, 240, 88, "iface", s["cli"], s["cliSub"])
    g.pill(300, 190, s["api"], 250)
    g.box(45, 222, 510, 90, "daemon", s["daemon"], s["daemonSub"], title_size=16)
    for x, key in ((45, "game"), (222, "module"), (399, "ext")):
        g.box(x, 392, 156, 88, "game" if key == "game" else "addon", s[key], s[key + "Sub"])
    for x, key in ((123, "run"), (300, "call"), (477, "hook")):
        g.pill(x, 352, s[key], 160)
    g.box(757, 70, 180, 88, "discord", s["discord"], s["discordSub"])
    g.box(757, 222, 180, 90, "discord", s["relay"], s["relaySub"])
    g.pill(656, 267, s["out"], 170)

    used = g.footer(30, 530, s["footer"], 920)
    g.h = int(530 + used + 12)
    return g.render()


def module_call(theme):
    s = MODULE_CALL
    g = Svg(960, 400, theme, "ko", s["title"], s["desc"])
    ys = (112, 170, 228, 286)
    # 오른쪽으로(stdin), 왼쪽으로(stdout, stderr, 역호출)
    g.arrow([(272, ys[0]), (620, ys[0])])
    g.arrow([(620, ys[1]), (272, ys[1])])
    g.arrow([(620, ys[2]), (272, ys[2])])
    g.arrow([(620, ys[3]), (272, ys[3])], dashed=True)
    g.box(40, 80, 232, 238, "daemon", s["daemon"], s["daemonSub"], title_size=16)
    g.box(620, 80, 300, 238, "addon", s["proc"], s["procSub"], title_mono=True, title_size=13)
    for y, key in zip(ys, ("a1", "a2", "a3", "a4")):
        g.pill(446, y, s[key], 330)
    g.pill(480, 352, s["env"], 880, size=11.5, mono=True)
    used = g.footer(30, 398, s["footer"], 900)
    g.h = int(398 + used + 12)
    return g.render()


def write(path, svg):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(svg)
    print("wrote", path)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--modules", help="saba-chan-modules 저장소 경로")
    args = ap.parse_args()
    root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

    for theme in ("light", "dark"):
        write(os.path.join(root, "docs", "images", f"architecture-{theme}.svg"), architecture("ko", theme))
        for lang in ("en", "ja"):
            write(os.path.join(root, "docs", "images", f"architecture-{lang}-{theme}.svg"), architecture(lang, theme))
    if args.modules:
        for theme in ("light", "dark"):
            write(os.path.join(args.modules, "docs", "images", f"module-call-{theme}.svg"), module_call(theme))

if __name__ == "__main__":
    main()
