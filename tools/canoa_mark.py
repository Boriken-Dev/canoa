"""Render the brand's mark into every raster icon the repository ships.

The mark is the SVG the brand file (app/brand.json, or the one `--brand` names) points at, drawn in its accent. Needs Pillow:
``python canoa_mark.py <repo> [--brand FILE] [--out DIR] [--sheet out.png] [--check]``. With `--out` the icons are written
under DIR, in the layout of the repository, and nothing in the repository changes.

Each size is drawn from the SVG's paths at a high resolution and box-filtered down, by the same
rules as ``app/lib/theme/canoa_mark.dart``: no paddle below 20 px, where the hull is stroked
``data-small-weight`` units wide instead, and no spiral below 64 px. The
paths are absolute M, L, C and Z only; anything else is refused rather than drawn wrong.
"""
from __future__ import annotations

import argparse
import math
import re
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

from PIL import Image, ImageDraw

BRAND = Path("app/brand.json")
NS = {"s": "http://www.w3.org/2000/svg"}
PADDLE_FROM, SPIRAL_FROM = 20, 64
# The web app's own background (app/web/manifest.json), behind the maskable icons.
WEB_BACKGROUND = (0x10, 0x18, 0x20)
# Each maskable icon keeps the mark inside the safe circle (radius 40%): a square of 0.8 / sqrt(2).
MASKABLE_SCALE = 0.56


class Mark:
    def __init__(self, svg: Path, color: tuple[int, int, int] | None = None, on_dark: tuple[int, int, int] | None = None):
        root = ET.parse(svg).getroot()
        paths = {p.get("id"): p for p in root.iter("{http://www.w3.org/2000/svg}path")}
        self.hull = _flatten(paths["hull"].get("d"))
        self.small_weight = float(paths["hull"].get("data-small-weight"))
        self.cut = _flatten(paths["cut"].get("d"))
        self.paddle = _flatten(paths["paddle"].get("d"))
        self.spiral = _flatten(paths["spiral"].get("d"), closed=False)
        self.spiral_width = float(paths["spiral"].get("stroke-width"))
        fill = root.find("s:g", NS).get("fill").lstrip("#")
        self.color = color or tuple(int(fill[i:i + 2], 16) for i in (0, 2, 4))
        # The maskable icons sit on the web app's dark background, in the brand's accent for it.
        self.maskable_color = on_dark or self.color

    def mask(self, size: int) -> Image.Image:
        """The mark's coverage at size x size pixels, with the size rules applied."""
        ss = max(4, min(16, 4096 // size))
        big = size * ss
        k = big / 64
        m = Image.new("L", (big, big), 0)
        draw = ImageDraw.Draw(m)
        paddle = size >= PADDLE_FROM

        def fill(polys, value):
            for poly in polys:
                draw.polygon([(x * k, y * k) for x, y in poly], fill=value)

        fill(self.hull, 255)
        if not paddle:
            # The hull alone is stroked too, round-joined, so a long low hull keeps its depth at tray sizes.
            for poly in self.hull:
                pts = [(x * k, y * k) for x, y in poly]
                draw.line(pts + pts[:2], fill=255, width=round(self.small_weight * k), joint="curve")
        if paddle:
            fill(self.cut, 0)
        if size >= SPIRAL_FROM:
            # A round-capped, round-joined stroke as discs along the path: Pillow's wide lines serrate on curves.
            r = self.spiral_width * k / 2
            for line in self.spiral:
                pts = [(x * k, y * k) for x, y in line]
                for (x0, y0), (x1, y1) in zip(pts, pts[1:]):
                    n = max(1, int(math.hypot(x1 - x0, y1 - y0) / (r / 8)) + 1)
                    for i in range(n + 1):
                        x, y = x0 + (x1 - x0) * i / n, y0 + (y1 - y0) * i / n
                        draw.ellipse((x - r, y - r, x + r, y + r), fill=0)
        if paddle:
            fill(self.paddle, 255)
        return m.resize((size, size), Image.Resampling.BOX)

    def icon(self, size: int, color=None) -> Image.Image:
        im = Image.new("RGBA", (size, size), (color or self.color) + (0,))
        im.putalpha(self.mask(size))
        return im

    def maskable(self, size: int, color) -> Image.Image:
        inner = round(size * MASKABLE_SCALE)
        im = Image.new("RGBA", (size, size), WEB_BACKGROUND + (255,))
        off = (size - inner) // 2
        im.alpha_composite(self.icon(inner, color), (off, off))
        return im


def _flatten(d: str, closed: bool = True, steps: int = 48):
    tokens = re.findall(r"[A-Za-z]|-?\d+(?:\.\d+)?", d)
    polys, cur, pos, i = [], [], (0.0, 0.0), 0

    def nums(n):
        nonlocal i
        vals = [float(t) for t in tokens[i:i + n]]
        i += n
        return vals

    while i < len(tokens):
        cmd = tokens[i]
        i += 1
        if cmd == "M":
            if cur:
                polys.append(cur)
            pos = tuple(nums(2))
            cur = [pos]
        elif cmd == "L":
            pos = tuple(nums(2))
            cur.append(pos)
        elif cmd == "C":
            v = nums(6)
            p = [pos, (v[0], v[1]), (v[2], v[3]), (v[4], v[5])]
            for s in range(1, steps + 1):
                t = s / steps
                u = 1 - t
                cur.append(tuple(u**3 * p[0][j] + 3 * u * u * t * p[1][j] + 3 * u * t * t * p[2][j] + t**3 * p[3][j]
                                 for j in (0, 1)))
            pos = p[3]
        elif cmd == "Z":
            polys.append(cur)
            cur = []
        else:
            raise ValueError(f"path command {cmd!r} is not drawn by canoa_mark.py or canoa_mark.dart")
    if cur:
        polys.append(cur)
    if closed and any(len(p) < 3 for p in polys):
        raise ValueError("a filled path has fewer than three points")
    return polys


def save_ico(mark: Mark, path: Path, sizes):
    images = [mark.icon(s) for s in sizes]
    # Pillow takes the first image as the base and picks each size from append_images when one matches,
    # so every entry is its own rendering rather than a resampled 256 px image.
    images[-1].save(path, format="ICO", sizes=[(s, s) for s in sizes], append_images=images[:-1])
    with Image.open(path) as check:
        found = sorted(check.info["sizes"])
        if found != sorted((s, s) for s in sizes):
            raise RuntimeError(f"{path} holds {found}, not {sizes}")
        for s in sizes:
            check.size = (s, s)
            frame = check.copy().convert("RGBA")
            want = images[sizes.index(s)]
            if frame.tobytes() != want.tobytes():
                raise RuntimeError(f"{path}'s {s} px entry is not the {s} px rendering")


def write_all(repo: Path, mark: Mark):
    icons = repo / "app/assets/icons"
    web = repo / "app/web"
    ext = repo / "clients/browser/chromium/icons"
    (web / "icons").mkdir(exist_ok=True)
    ext.mkdir(exist_ok=True)
    (repo / "docs/assets").mkdir(exist_ok=True)

    written = []

    def png(im, path):
        im.save(path, optimize=True)
        written.append(path)

    # The tray: Windows loads the .ico (LoadImage with IMAGE_ICON takes no PNG); Linux and macOS take one PNG.
    png(mark.icon(32), icons / "canoa_tray.png")
    save_ico(mark, icons / "canoa.ico", [16, 24, 32, 48, 64, 256])
    written.append(icons / "canoa.ico")

    save_ico(mark, web / "favicon.ico", [16, 32, 48])
    written.append(web / "favicon.ico")
    for s in (192, 512):
        png(mark.icon(s), web / f"icons/Icon-{s}.png")
        png(mark.maskable(s, mark.maskable_color), web / f"icons/Icon-maskable-{s}.png")

    for s in (16, 32, 48, 128):
        png(mark.icon(s), ext / f"canoa-{s}.png")

    save_ico(mark, repo / "docs/assets/favicon.ico", [16, 32, 48])
    written.append(repo / "docs/assets/favicon.ico")
    return written


def sheet(mark: Mark, out: Path):
    """Every size at 1:1 and enlarged with nearest-neighbour, on dark, light and Windows-dark backgrounds."""
    backgrounds = [(0x17, 0x24, 0x2D), (0xF3, 0xF3, 0xF3), (0x20, 0x20, 0x20)]
    sizes = [16, 24, 32, 48, 64, 128, 256]
    rows = []
    for bg in backgrounds:
        tiles = []
        for s in sizes:
            im = Image.new("RGBA", (s, s), bg + (255,))
            im.alpha_composite(mark.icon(s))
            tiles.append(im)
            if s <= 64:
                tiles.append(im.resize((s * 6, s * 6) if s <= 32 else (s * 3, s * 3), Image.Resampling.NEAREST))
        h = max(t.height for t in tiles)
        row = Image.new("RGBA", (sum(t.width + 10 for t in tiles), h), (128, 128, 128, 255))
        x = 0
        for t in tiles:
            row.paste(t, (x, 0))
            x += t.width + 10
        rows.append(row)
    W = max(r.width for r in rows)
    out_im = Image.new("RGBA", (W, sum(r.height + 10 for r in rows)), (90, 90, 90, 255))
    y = 0
    for r in rows:
        out_im.paste(r, (0, y))
        y += r.height + 10
    out_im.save(out)


def brand_mark(brand_file: Path) -> Mark:
    """The mark of the brand `brand_file` describes: its icon, in its accent."""
    import json

    brand = json.loads(brand_file.read_text(encoding="utf-8"))

    def rgb(text: str) -> tuple[int, int, int]:
        return tuple(int(text.lstrip("#")[i:i + 2], 16) for i in (0, 2, 4))

    on_dark = brand.get("accent_on_dark")
    return Mark(brand_file.parent / brand["icon"], rgb(brand["accent"]), rgb(on_dark) if on_dark else None)


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("repo", type=Path, help="a Canoa checkout")
    ap.add_argument("--brand", type=Path, help="the brand file (default: app/brand.json of the checkout)")
    ap.add_argument("--out", type=Path, help="write the icons under this folder, in the repository's layout, not in the checkout")
    ap.add_argument("--sheet", type=Path, help="also write a contact sheet of every size here")
    ap.add_argument("--check", action="store_true", help="write nothing; exit 1 if a committed icon differs")
    args = ap.parse_args()
    mark = brand_mark(args.brand or args.repo / BRAND)
    if args.check:
        import tempfile
        import shutil
        with tempfile.TemporaryDirectory() as tmp:
            t = Path(tmp)
            for rel in ("app/assets/icons", "app/web", "clients/browser/chromium", "docs"):
                (t / rel).mkdir(parents=True, exist_ok=True)
            stale = []
            for p in write_all(t, mark):
                rel = p.relative_to(t)
                committed = args.repo / rel
                if not committed.is_file() or _pixels(committed) != _pixels(p):
                    stale.append(str(rel))
            shutil.rmtree(t, ignore_errors=True)
        if stale:
            print("differ from the brand mark:", *stale, sep="\n  ")
            sys.exit(1)
        print("every icon matches the brand mark")
        return
    target = args.out or args.repo
    if args.out:
        for rel in ("app/assets/icons", "app/web", "clients/browser/chromium", "docs"):
            (target / rel).mkdir(parents=True, exist_ok=True)
    for p in write_all(target, mark):
        print(p.relative_to(target))
    if args.sheet:
        sheet(mark, args.sheet)


def _pixels(path: Path):
    with Image.open(path) as im:
        frames = []
        for s in sorted(im.info.get("sizes", {im.size})):
            if "sizes" in im.info:
                im.size = s
            frames.append(im.copy().convert("RGBA").tobytes())
        return frames


if __name__ == "__main__":
    main()
