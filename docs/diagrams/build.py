#!/usr/bin/env python3
"""사용자 문서용 다이어그램(SVG)을 만든다.

    python docs/diagrams/build.py                     # saba-chan README 그림만
    python docs/diagrams/build.py --modules ../saba-chan-modules --website ../server-chan/website

- saba-chan:         docs/images/architecture-{light,dark}.svg (README, GitHub 라이트/다크)
- saba-chan-modules: docs/images/module-call-{light,dark}.svg (모듈 개발 가이드 §2)
- 소개 사이트:        public/assets/docs/<언어>/{architecture,discord-flow,pairing}.svg (다크 사이트라 다크만)

그림 내용은 코드 기준이다. 바뀌면 문구(strings.py)와 배치를 고치고 다시 만든다.
근거: 데몬 API·인증 src/ipc, 릴레이 연결 src/relay_agent.rs, 페어링 src/ipc/handlers/relay.rs +
server-chan relay-server/src/routes/pair.ts·services/pairing.ts, 모듈 호출 src/plugin/mod.rs.
"""

import argparse
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from strings import ARCH, FLOW, PAIR, MODULE_CALL, WEBSITE_LANGS  # noqa: E402
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


def sequence(lang, theme, s, cols, regions, steps, width=980):
    """cols: [(key, role)] 왼→오, regions: [(첫 열, 끝 열, 라벨 키, role)], steps: [(종류, ...)]"""
    g = Svg(width, 100, theme, lang, s["title"], s["desc"])
    t = g.t
    n = len(cols)
    margin = 40
    span = (width - 2 * margin) / n
    xs = [margin + span * (i + 0.5) for i in range(n)]
    idx = {k: i for i, (k, _) in enumerate(cols)}
    colw = span - 30
    head_y, head_h = 62, 66

    # 단계 배치를 먼저 계산해 높이를 정한다
    y = head_y + head_h + 46
    plan = []
    for num, st in enumerate(steps, 1):
        if st[0] == "msg":
            _, a, b, key, dashed = st
            plan.append(("msg", num, a, b, key, dashed, y))
            y += 66
        else:
            _, c, key = st
            plan.append(("note", num, c, key, y))
            y += 64
    bottom = y - 10

    for a, b, key, role in regions:
        x0 = xs[idx[a]] - span / 2 + 8
        x1 = xs[idx[b]] + span / 2 - 8
        g.region(x0, 20, x1 - x0, bottom - 4, s[key], COLORS[role])
    for i, (k, role) in enumerate(cols):
        g.add(
            f'<line x1="{xs[i]:.1f}" y1="{head_y + head_h}" x2="{xs[i]:.1f}" y2="{bottom}" '
            f'stroke="{t["region"]}" stroke-width="1.5" stroke-dasharray="3 5"/>'
        )
        g.box(xs[i] - colw / 2, head_y, colw, head_h, role, s[k], s[k + "Sub"])

    for p in plan:
        if p[0] == "msg":
            _, num, a, b, key, dashed, yy = p
            xa, xb = xs[idx[a]], xs[idx[b]]
            d = 1 if xb > xa else -1
            g.arrow([(xa + 6 * d, yy + 18), (xb - 6 * d, yy + 18)], dashed=dashed)
            g.pill((xa + xb) / 2, yy - 2, s[key], abs(xb - xa) + 10, num=num, color=COLORS[cols[idx[a]][1]])
        else:
            _, num, c, key, yy = p
            g.note(xs[idx[c]], yy + 6, colw, s[key], num, cols[idx[c]][1])

    used = g.footer(30, bottom + 40, s["footer"], width - 60)
    g.h = int(bottom + 40 + used + 12)
    return g.render()


def discord_flow(lang, theme):
    return sequence(
        lang,
        theme,
        FLOW[lang],
        cols=[("user", "discord"), ("bot", "discord"), ("daemon", "daemon"), ("game", "game")],
        regions=[("bot", "bot", "cloud", "discord"), ("daemon", "game", "pc", "daemon")],
        steps=[
            ("msg", "daemon", "bot", "s1", True),
            ("msg", "user", "bot", "s2", False),
            ("msg", "bot", "daemon", "s3", False),
            ("note", "daemon", "s4"),
            ("msg", "daemon", "game", "s5", False),
            ("msg", "daemon", "bot", "s6", False),
            ("msg", "bot", "user", "s7", False),
        ],
    )


def pairing(lang, theme):
    return sequence(
        lang,
        theme,
        PAIR[lang],
        cols=[("gui", "iface"), ("daemon", "daemon"), ("relay", "discord"), ("discord", "discord")],
        regions=[("gui", "daemon", "pc", "daemon"), ("relay", "relay", "cloud", "discord")],
        steps=[
            ("msg", "gui", "daemon", "p1", False),
            ("msg", "daemon", "relay", "p2", False),
            ("msg", "relay", "daemon", "p3", False),
            ("msg", "daemon", "gui", "p4", False),
            ("msg", "discord", "relay", "p5", False),
            ("note", "relay", "p6"),
            ("msg", "gui", "daemon", "p7", True),
            ("msg", "daemon", "relay", "p8", True),
            ("msg", "relay", "daemon", "p9", False),
            ("note", "daemon", "p10"),
            ("msg", "daemon", "gui", "p11", False),
        ],
    )


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
    ap.add_argument("--website", help="server-chan/website 경로")
    args = ap.parse_args()
    root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

    for theme in ("light", "dark"):
        write(os.path.join(root, "docs", "images", f"architecture-{theme}.svg"), architecture("ko", theme))
    if args.modules:
        for theme in ("light", "dark"):
            write(os.path.join(args.modules, "docs", "images", f"module-call-{theme}.svg"), module_call(theme))
    if args.website:
        for lang in WEBSITE_LANGS:
            out = os.path.join(args.website, "public", "assets", "docs", lang)
            write(os.path.join(out, "architecture.svg"), architecture(lang, "dark"))
            write(os.path.join(out, "discord-flow.svg"), discord_flow(lang, "dark"))
            write(os.path.join(out, "pairing.svg"), pairing(lang, "dark"))


if __name__ == "__main__":
    main()
