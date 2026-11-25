# /// script
# requires-python = ">=3.10"
# dependencies = ["pillow>=10"]
# ///
"""Draw the noiseman icon into assets/: noiseman.svg (hicolor scalable, the web favicon), noiseman-<size>.png (hicolor
sizes; the small ones are what trays and title bars pick) and noiseman.ico (embedded in noiseman.exe by build.rs).

The icon is the UIs' logo cut down for small sizes: a dark tile in the brown theme's colors (gui/mod.rs theme()) with
the first six bars of its logo. Each PNG is drawn at its own size with the bars snapped to whole pixels, so they stay crisp.

Run: uv run tools/icons.py
"""

import io
import struct
from pathlib import Path

from PIL import Image, ImageDraw

ACCENT = (0xD0, 0x8F, 0x4A)
TRACK = tuple((a * 35 + 0x8A * 65) // 100 for a in ACCENT)  # gui theme(): mix(accent, 0x8a8a8a, 35)
TILE = tuple(a * 22 // 100 for a in ACCENT)  # gui theme(): mix(accent, 0, 22)
BARS = [5, 11, 7, 14, 9, 3]  # the first six of gui LOGO_BARS, in sixteenths of the bar area
PNG_SIZES = [16, 22, 24, 32, 48, 64, 128, 256]
ICO_SIZES = [16, 24, 32, 48, 64, 128, 256]
SS = 4  # supersampling for the tile's rounded corners

ASSETS = Path(__file__).resolve().parent.parent / "assets"


def layout(s: int):
    """Tile box, corner radius, outline width and the bar rectangles at size `s`, in whole pixels."""
    m = round(s / 16) if s >= 32 else 0  # small icons need every pixel; trays pad them anyway
    radius = round(s * 0.19)
    line = max(1, round(s / 64))
    pad = round(s * 0.19)
    unit = (s - 2 * pad) / (len(BARS) + 0.3 * (len(BARS) - 1))
    w, gap = max(1, round(unit)), max(1, round(unit * 0.3))
    total = len(BARS) * w + (len(BARS) - 1) * gap
    while total > s - 2 * (m + line + 1) and w > 1:  # keep a pixel clear of the outline
        w -= 1
        total -= len(BARS)
    x = (s - total) // 2
    bottom, top = s - round(s * 0.21), round(s * 0.2)
    bars = []
    for h in BARS:
        bh = max(1, round((bottom - top) * h / 16))
        bars.append((x, bottom - bh, x + w, bottom))
        x += w + gap
    return (m, m, s - m, s - m), radius, line, bars


def png(s: int) -> Image.Image:
    (x0, y0, x1, y1), radius, line, bars = layout(s)
    big = Image.new("RGBA", (s * SS, s * SS), (0, 0, 0, 0))
    ImageDraw.Draw(big).rounded_rectangle(
        (x0 * SS, y0 * SS, x1 * SS - 1, y1 * SS - 1), radius * SS, fill=TILE, outline=TRACK, width=line * SS
    )
    im = big.resize((s, s), Image.LANCZOS)
    draw = ImageDraw.Draw(im)
    for bx0, by0, bx1, by1 in bars:
        draw.rectangle((bx0, by0, bx1 - 1, by1 - 1), fill=ACCENT)
    return im


def svg() -> str:
    s = 256
    (x0, y0, x1, y1), radius, line, bars = layout(s)
    hexc = lambda c: "#%02x%02x%02x" % c
    half = line / 2
    rects = "\n".join(
        f'  <rect x="{bx0}" y="{by0}" width="{bx1 - bx0}" height="{by1 - by0}" fill="{hexc(ACCENT)}"/>'
        for bx0, by0, bx1, by1 in bars
    )
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{s}" height="{s}" viewBox="0 0 {s} {s}">\n'
        f'  <rect x="{x0 + half}" y="{y0 + half}" width="{x1 - x0 - line}" height="{y1 - y0 - line}" rx="{radius - half}"'
        f' fill="{hexc(TILE)}" stroke="{hexc(TRACK)}" stroke-width="{line}"/>\n{rects}\n</svg>\n'
    )


def ico(images: dict[int, bytes]) -> bytes:
    """An .ico with each size stored as PNG (Windows Vista and later)."""
    head = struct.pack("<HHH", 0, 1, len(images))
    offset = len(head) + 16 * len(images)
    entries, data = b"", b""
    for s, blob in images.items():
        entries += struct.pack("<BBBBHHII", s % 256, s % 256, 0, 0, 1, 32, len(blob), offset + len(data))
        data += blob
    return head + entries + data


def main() -> None:
    encoded = {}
    for s in PNG_SIZES:
        buf = io.BytesIO()
        png(s).save(buf, "PNG", optimize=True)
        encoded[s] = buf.getvalue()
        (ASSETS / f"noiseman-{s}.png").write_bytes(encoded[s])
    (ASSETS / "noiseman.svg").write_text(svg())
    (ASSETS / "noiseman.ico").write_bytes(ico({s: encoded[s] for s in ICO_SIZES}))
    print(f"wrote assets/noiseman.svg, noiseman.ico and {len(PNG_SIZES)} PNGs ({', '.join(map(str, PNG_SIZES))} px)")


if __name__ == "__main__":
    main()
