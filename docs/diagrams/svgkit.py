"""문서 다이어그램용 작은 SVG 도구.

글자 폭은 글꼴 없이 어림한다(한글·한자 1em, 라틴 약 0.55em). 상자와 화살표 이름표는
어림한 폭이 넘치면 줄을 바꾸고, 줄 수가 넘치면 글자를 줄인다. 넘친 글자가 없는지는
각 <text>의 data-fit(허용 폭)과 브라우저의 getComputedTextLength()를 비교해 확인한다.
"""

from html import escape

# design-reference.md §3 색 토큰
COLORS = {
    "iface": "#06b6d4",  # 인터페이스 (cyan)
    "daemon": "#7c3aed",  # 데몬 (purple)
    "addon": "#3b82f6",  # 모듈·익스텐션 (blue)
    "game": "#22c55e",  # 게임 서버 (green)
    "discord": "#5865F2",  # 디스코드·릴레이 (R6)
}

THEMES = {
    "dark": {
        "bg": "#0b0f1a",
        "frame": "rgba(255,255,255,0.08)",
        "text": "#f1f5f9",
        "text2": "#94a3b8",
        "muted": "#64748b",
        "line": "#94a3b8",
        "region": "#334155",
        "pill": "#111827",
        "pill_stroke": "rgba(255,255,255,0.10)",
        "fill_op": 0.16,
        "stroke_op": 0.75,
    },
    "light": {
        "bg": "#ffffff",
        "frame": "#e2e8f0",
        "text": "#0f172a",
        "text2": "#475569",
        "muted": "#64748b",
        "line": "#64748b",
        "region": "#cbd5e1",
        "pill": "#f8fafc",
        "pill_stroke": "#e2e8f0",
        "fill_op": 0.09,
        "stroke_op": 0.65,
    },
}

FONTS = {
    "ko": "Pretendard, 'Apple SD Gothic Neo', 'Malgun Gothic', 'Noto Sans KR', sans-serif",
    "ja": "'Hiragino Sans', 'Hiragino Kaku Gothic ProN', 'Yu Gothic UI', Meiryo, 'Noto Sans JP', sans-serif",
    "zh-CN": "'PingFang SC', 'Microsoft YaHei UI', 'Microsoft YaHei', 'Noto Sans SC', sans-serif",
    "zh-TW": "'PingFang TC', 'Microsoft JhengHei UI', 'Microsoft JhengHei', 'Noto Sans TC', sans-serif",
    "latin": "Inter, 'Segoe UI', -apple-system, BlinkMacSystemFont, Roboto, 'Helvetica Neue', Arial, sans-serif",
}
MONO = "'Cascadia Code', Consolas, 'SFMono-Regular', Menlo, monospace"


def char_em(c):
    o = ord(c)
    if o >= 0x2E80:
        return 1.0
    if c == " ":
        return 0.3
    if c in "il.,:;|!'`()[]{}/":
        return 0.32
    if c in "mwMW@":
        return 0.88
    if c.isupper():
        return 0.7
    if c.isdigit():
        return 0.6
    if 0x0400 <= o <= 0x04FF:
        return 0.62
    return 0.56


def text_width(s, size, mono=False):
    if mono:
        return sum((1.0 if ord(c) >= 0x2E80 else 0.6) for c in s) * size
    return sum(char_em(c) for c in s) * size


def _greedy(words, sep, maxw, size, mono):
    out, line = [], ""
    for w in words:
        cand = (line + sep + w) if line else w
        if text_width(cand, size, mono) <= maxw or not line:
            line = cand
        else:
            out.append(line)
            line = w
    if line:
        out.append(line)
    return out


def _words(para):
    """공백으로 나누되 '·', '+' 같은 이음표는 앞 낱말에 붙여 줄 머리에 오지 않게 한다."""
    if " " not in para:
        # 공백 없는 일본어 등: 구두점 뒤(・、）」)와 여는 괄호 앞(（「)에서만 끊는다
        chunks, cur = [], ""
        for ch in para:
            if ch in "（「＋" and cur:
                chunks.append(cur)
                cur = ""
            cur += ch
            if ch in "・、，。）」":
                chunks.append(cur)
                cur = ""
        if cur:
            chunks.append(cur)
        return chunks, ""
    words = []
    for w in para.split(" "):
        if words and w in ("·", "+", "-", ":"):
            words[-1] += " " + w
        else:
            words.append(w)
    return words, " "


def wrap(s, maxw, size, mono=False):
    """명시적 줄바꿈(\\n)을 지키고, 넘치는 줄은 공백에서(공백이 없으면 글자 단위로) 줄 길이가 고르게 나눈다."""
    out = []
    for para in s.split("\n"):
        if text_width(para, size, mono) <= maxw:
            out.append(para)
            continue
        words, sep = _words(para)
        if not sep:  # 끊을 곳이 없는 긴 덩어리는 글자 단위로
            words = [p for w in words for p in ([w] if text_width(w, size, mono) <= maxw else list(w))]
        lines = _greedy(words, sep, maxw, size, mono)
        # 같은 줄 수 안에서 가장 긴 줄이 가장 짧아지는 폭을 찾는다
        lo, hi = text_width(para, size, mono) / len(lines), maxw
        for _ in range(12):
            mid = (lo + hi) / 2
            if len(_greedy(words, sep, mid, size, mono)) <= len(lines):
                hi = mid
            else:
                lo = mid
        out.extend(_greedy(words, sep, hi, size, mono))
    return out


def fit(s, maxw, size, min_size, max_lines, mono=False):
    """먼저 줄을 나눠 보고, max_lines를 넘으면 min_size까지 글자를 줄인다. 실제 글꼴 차이를 보고 폭을 4% 남긴다."""
    maxw *= 0.96
    sz = size
    lines = wrap(s, maxw, sz, mono)
    while (len(lines) > max_lines or max(text_width(l, sz, mono) for l in lines) > maxw) and sz > min_size:
        sz -= 0.5
        lines = wrap(s, maxw, sz, mono)
    return sz, lines


class Svg:
    def __init__(self, w, h, theme, lang, title, desc):
        self.w, self.h = w, h
        self.t = THEMES[theme]
        self.theme = theme
        self.lang = lang
        self.font = FONTS.get(lang, FONTS["latin"])
        self.title, self.desc = title, desc
        self.parts = []

    # ── 기본 요소 ──
    def add(self, s):
        self.parts.append(s)

    def text(self, x, y, s, size=13, weight=400, fill=None, anchor="middle", mono=False, fit_w=None):
        fill = fill or self.t["text"]
        fam = f' font-family="{MONO}"' if mono else ""
        fw = f' data-fit="{fit_w:.1f}"' if fit_w else ""
        self.add(
            f'<text x="{x:.1f}" y="{y:.1f}" font-size="{size}" font-weight="{weight}" fill="{fill}" '
            f'text-anchor="{anchor}"{fam}{fw}>{escape(s)}</text>'
        )

    def lines(self, x, y_center, lines, size, weight=400, fill=None, anchor="middle", mono=False, fit_w=None, gap=1.3):
        lh = size * gap
        y0 = y_center - lh * (len(lines) - 1) / 2 + size * 0.35
        for i, ln in enumerate(lines):
            self.text(x, y0 + i * lh, ln, size, weight, fill, anchor, mono, fit_w)

    def region(self, x, y, w, h, label, color):
        t = self.t
        self.add(
            f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="16" fill="none" stroke="{t["region"]}" '
            f'stroke-width="1.5" stroke-dasharray="6 5"/>'
        )
        sz, ln = fit(label, w - 32, 12.5, 10, 1)
        self.add(f'<circle cx="{x + 18}" cy="{y + 20}" r="4" fill="{color}"/>')
        self.text(x + 30, y + 24.5, ln[0], sz, 700, t["text2"], anchor="start", fit_w=w - 32)

    def box(self, x, y, w, h, role, title, sub=None, title_mono=False, title_size=15):
        t, c = self.t, COLORS[role]
        self.add(
            f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="12" fill="{c}" fill-opacity="{t["fill_op"]}" '
            f'stroke="{c}" stroke-opacity="{t["stroke_op"]}" stroke-width="1.5"/>'
        )
        inner = w - 24
        tsz, tl = fit(title, inner, title_size, 11, 2, mono=title_mono)
        blocks = [(tl, tsz, 700, t["text"], title_mono)]
        if sub:
            ssz, sl = fit(sub, inner, 12, 10, 3)
            blocks.append((sl, ssz, 400, t["text2"], False))
        heights = [len(b[0]) * b[1] * 1.3 for b in blocks]
        total = sum(heights) + (6 if sub else 0)
        cy = y + h / 2 - total / 2
        for (ls, sz, wt, fill, mono), hh in zip(blocks, heights):
            self.lines(x + w / 2, cy + hh / 2, ls, sz, wt, fill, mono=mono, fit_w=inner)
            cy += hh + 6

    def pill(self, cx, cy, label, maxw, num=None, size=12, mono=False, color=None):
        """화살표 이름표. num이 있으면 왼쪽에 번호 동그라미를 붙인다."""
        t = self.t
        badge = 22 if num is not None else 0
        sz, ls = fit(label, maxw - 16 - badge, size, 10, 3, mono)
        tw = max(text_width(l, sz, mono) for l in ls)
        pw = tw + 16 + badge
        ph = len(ls) * sz * 1.3 + 10
        x0 = cx - pw / 2
        self.add(
            f'<rect x="{x0:.1f}" y="{cy - ph / 2:.1f}" width="{pw:.1f}" height="{ph:.1f}" rx="{min(ph / 2, 10):.1f}" '
            f'fill="{t["pill"]}" stroke="{t["pill_stroke"]}"/>'
        )
        if num is not None:
            bc = color or COLORS["daemon"]
            self.add(f'<circle cx="{x0 + 8 + 9:.1f}" cy="{cy:.1f}" r="9" fill="{bc}"/>')
            self.text(x0 + 17, cy + 4, str(num), 11, 700, "#ffffff")
        self.lines(x0 + 8 + badge + tw / 2, cy, ls, sz, 500, t["text"], mono=mono, fit_w=maxw - 16 - badge)
        return ph

    def note(self, cx, cy, w, label, num, role):
        """생명선 위에 얹는 처리 단계(점선 상자 + 번호)."""
        t, c = self.t, COLORS[role]
        sz, ls = fit(label, w - 48, 12, 11, 3)
        h = len(ls) * sz * 1.3 + 18
        x0 = cx - w / 2
        self.add(
            f'<rect x="{x0:.1f}" y="{cy - h / 2:.1f}" width="{w:.1f}" height="{h:.1f}" rx="10" fill="{t["bg"]}"/>'
            f'<rect x="{x0:.1f}" y="{cy - h / 2:.1f}" width="{w:.1f}" height="{h:.1f}" rx="10" fill="{c}" '
            f'fill-opacity="{t["fill_op"]}" stroke="{c}" stroke-width="1.2" stroke-dasharray="4 3"/>'
        )
        self.add(f'<circle cx="{x0 + 20:.1f}" cy="{cy:.1f}" r="9" fill="{c}"/>')
        self.text(x0 + 20, cy + 4, str(num), 11, 700, "#ffffff")
        self.lines(cx + 12, cy, ls, sz, 500, t["text"], fit_w=w - 48)

    def arrow(self, pts, dashed=False, both=False, color=None, width=1.6):
        color = color or self.t["line"]
        d = "M" + " L".join(f"{x:.1f} {y:.1f}" for x, y in pts)
        dash = ' stroke-dasharray="5 4"' if dashed else ""
        mid = self.marker(color)
        start = f' marker-start="url(#{mid})"' if both else ""
        self.add(
            f'<path d="{d}" fill="none" stroke="{color}" stroke-width="{width}"{dash} '
            f'marker-end="url(#{mid})"{start} stroke-linecap="round" stroke-linejoin="round"/>'
        )

    _markers = None

    def marker(self, color):
        if self._markers is None:
            self._markers = {}
        if color not in self._markers:
            self._markers[color] = f"ah{len(self._markers)}"
        return self._markers[color]

    def footer(self, x, y, s, maxw, size=12.5, anchor="start"):
        sz, ls = fit(s, maxw, size, 11, 3)
        for i, ln in enumerate(ls):
            self.text(x, y + i * sz * 1.45, ln, sz, 400, self.t["text2"], anchor=anchor, fit_w=maxw)
        return len(ls) * sz * 1.45

    # ── 출력 ──
    def render(self):
        t = self.t
        defs = "".join(
            f'<marker id="{mid}" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" '
            f'orient="auto-start-reverse"><path d="M0 0 L10 5 L0 10 z" fill="{c}"/></marker>'
            for c, mid in (self._markers or {}).items()
        )
        head = (
            f'<svg xmlns="http://www.w3.org/2000/svg" width="{self.w}" height="{self.h}" '
            f'viewBox="0 0 {self.w} {self.h}" role="img" aria-labelledby="t d" lang="{self.lang}" '
            f'font-family="{escape(self.font, quote=True)}">\n'
            f'<title id="t">{escape(self.title)}</title>\n<desc id="d">{escape(self.desc)}</desc>\n'
            "<!-- 생성기: saba-chan docs/diagrams/build.py. 손으로 고치지 말고 생성기를 고친다. -->\n"
            f"<defs>{defs}</defs>\n"
            f'<rect x="0.5" y="0.5" width="{self.w - 1}" height="{self.h - 1}" rx="18" fill="{t["bg"]}" '
            f'stroke="{t["frame"]}"/>\n'
        )
        return head + "\n".join(self.parts) + "\n</svg>\n"
