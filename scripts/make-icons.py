"""exe에 넣는 Windows 아이콘(.ico)을 원본 PNG에서 다시 만든다.

    python scripts/make-icons.py

Windows는 DPI·보기 방식마다 16~256 사이 여러 크기를 고르므로 그 크기를 모두 넣는다.
빠진 크기는 가까운 크기를 늘리거나 줄여 그려서 흐려진다(20·24·40px 등).
256px만 PNG로, 나머지는 32비트 BMP로 넣는다. 필요: Pillow
"""

import io
import struct
from pathlib import Path

from PIL import Image, ImageChops, ImageFilter

ROOT = Path(__file__).resolve().parent.parent
SIZES = (256, 128, 96, 64, 48, 40, 32, 24, 20, 16)

APP_ICON = ROOT / "saba-chan-gui/src-tauri/icons/icon.png"
CORE_ICON = ROOT / "icons/saba-core.png"
# 사바 러너는 GUI 트레이와 같은 생선 파비콘을 쓴다
FISH_ICON = ROOT / "saba-chan-gui/public/favicon-light.png"

TARGETS = {
    "saba-chan-gui/src-tauri/icons/icon.ico": APP_ICON,
    "installer/gui/src-tauri/icons/icon.ico": APP_ICON,
    "saba-chan-cli/icon.ico": APP_ICON,
    "icons/saba-core.ico": CORE_ICON,
    "runner/icons/saba-runner.ico": FISH_ICON,
}


def square(img: Image.Image) -> Image.Image:
    """투명 여백을 걷어 내고 가운데 정사각형에 둔다."""
    img = img.convert("RGBA")
    box = img.getbbox()
    if box:
        img = img.crop(box)
    side = max(img.size)
    out = Image.new("RGBA", (side, side), (0, 0, 0, 0))
    out.paste(img, ((side - img.width) // 2, (side - img.height) // 2))
    return out


def outlined(img: Image.Image, size: int) -> Image.Image:
    """어두운 생선이 어두운 배경(작업 표시줄·탐색기 다크 모드)에서도 보이게 밝은 테두리를 두른다."""
    pad = max(1, round(size / 24))
    inner = size - 2 * pad
    shape = img.resize((inner, inner), Image.LANCZOS)
    canvas = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    canvas.paste(shape, (pad, pad))
    alpha = canvas.getchannel("A")
    ring = alpha.filter(ImageFilter.MaxFilter(2 * pad + 1))
    border = Image.new("RGBA", (size, size), (245, 245, 245, 255))
    border.putalpha(ring)
    return Image.alpha_composite(border, canvas)


def render(src: Path, size: int) -> Image.Image:
    img = square(Image.open(src))
    if src == FISH_ICON:
        return outlined(img, size)
    out = img.resize((size, size), Image.LANCZOS)
    if size <= 48:
        # 작은 크기는 줄이면서 뭉개진 윤곽을 조금 살린다
        out = out.filter(ImageFilter.UnsharpMask(radius=0.6, percent=60, threshold=1))
    return out


def bmp_entry(img: Image.Image) -> bytes:
    w, h = img.size
    header = struct.pack("<IiiHHIIiiII", 40, w, h * 2, 1, 32, 0, 0, 0, 0, 0, 0)
    px = img.tobytes("raw", "BGRA", 0, -1)  # 아래 줄부터
    row = ((w + 31) // 32) * 4
    alpha = img.getchannel("A")
    mask = bytearray()
    for y in range(h - 1, -1, -1):
        bits = bytearray(row)
        for x in range(w):
            if alpha.getpixel((x, y)) == 0:
                bits[x // 8] |= 0x80 >> (x % 8)
        mask += bits
    return header + px + bytes(mask)


def png_entry(img: Image.Image) -> bytes:
    buf = io.BytesIO()
    img.save(buf, "PNG", optimize=True)
    return buf.getvalue()


def build_ico(src: Path) -> bytes:
    # 첫 항목이 가장 큰 크기다: Tauri는 .ico의 첫 항목을 기본 창 아이콘으로 쓴다
    entries = []
    for size in SIZES:
        img = render(src, size)
        data = png_entry(img) if size >= 256 else bmp_entry(img)
        entries.append((size, data))
    head = struct.pack("<HHH", 0, 1, len(entries))
    offset = 6 + 16 * len(entries)
    table = b""
    body = b""
    for size, data in entries:
        dim = 0 if size >= 256 else size
        table += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(data), offset + len(body))
        body += data
    return head + table + body


def main() -> None:
    cache = {}
    for rel, src in TARGETS.items():
        data = cache.setdefault(src, build_ico(src))
        path = ROOT / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        print(f"{rel} ({len(data) // 1024} KB)")


if __name__ == "__main__":
    main()
