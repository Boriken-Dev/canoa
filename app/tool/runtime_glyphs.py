#!/usr/bin/env python3
"""Draw the glyphs a runtime's store may name as its icon, from the geometry below.

Writes one SVG per glyph to assets/icons/runtimes/ and the same path data to lib/store/runtime_glyphs.dart, which the app draws.
The glyphs are plain geometry on a 24-unit square (circles, bars, polygons), each suggesting what a runtime is for and none a
project's logo. `--check` writes nothing and exits 1 when a file differs. The names are the ones of `STORE_GLYPHS` in
crates/canoa/src/store.rs.
"""
from __future__ import annotations

import argparse
import math
import sys
from pathlib import Path

APP = Path(__file__).resolve().parents[1]
SVG_DIR = APP / "assets" / "icons" / "runtimes"
DART = APP / "lib" / "store" / "runtime_glyphs.dart"
SIDE = 24


def num(value: float) -> str:
    text = f"{value:.2f}".rstrip("0").rstrip(".")
    return "0" if text in ("", "-0") else text


def pts(points: list[tuple[float, float]]) -> str:
    first, *rest = points
    return f"M{num(first[0])} {num(first[1])}" + "".join(f"L{num(x)} {num(y)}" for x, y in rest) + "Z"


def circle(cx: float, cy: float, r: float) -> str:
    return (
        f"M{num(cx - r)} {num(cy)}A{num(r)} {num(r)} 0 1 0 {num(cx + r)} {num(cy)}"
        f"A{num(r)} {num(r)} 0 1 0 {num(cx - r)} {num(cy)}Z"
    )


def rrect(x: float, y: float, w: float, h: float, r: float, hole: bool = False) -> str:
    """A rounded rectangle, clockwise; with [hole] counter-clockwise, which cuts a hole out of a larger one."""
    sweep = 0 if hole else 1
    arc = f"A{num(r)} {num(r)} 0 0 {sweep} "
    if not hole:
        return (
            f"M{num(x + r)} {num(y)}H{num(x + w - r)}{arc}{num(x + w)} {num(y + r)}V{num(y + h - r)}"
            f"{arc}{num(x + w - r)} {num(y + h)}H{num(x + r)}{arc}{num(x)} {num(y + h - r)}"
            f"V{num(y + r)}{arc}{num(x + r)} {num(y)}Z"
        )
    return (
        f"M{num(x + r)} {num(y)}{arc}{num(x)} {num(y + r)}V{num(y + h - r)}{arc}{num(x + r)} {num(y + h)}"
        f"H{num(x + w - r)}{arc}{num(x + w)} {num(y + h - r)}V{num(y + r)}{arc}{num(x + w - r)} {num(y)}Z"
    )


def bar(x1: float, y1: float, x2: float, y2: float, width: float) -> str:
    """A straight bar of [width] between two points, with flat ends."""
    length = math.hypot(x2 - x1, y2 - y1)
    dx, dy = (y2 - y1) / length * width / 2, -(x2 - x1) / length * width / 2
    return pts([(x1 + dx, y1 + dy), (x2 + dx, y2 + dy), (x2 - dx, y2 - dy), (x1 - dx, y1 - dy)])


def glyphs() -> dict[str, list[str]]:
    out: dict[str, list[str]] = {}

    # Node: three nodes joined in a graph.
    a, b, c = (12.0, 4.8), (4.6, 18.4), (19.4, 18.4)
    out["nodes"] = [
        bar(*a, *b, 1.7),
        bar(*a, *c, 1.7),
        bar(*b, *c, 1.7),
        *[circle(*p, 3.1) for p in (a, b, c)],
    ]

    # Python: lines of code, the middle two indented, and the marker that indents them.
    out["indent"] = [
        rrect(3, 3.5, 17, 2.8, 1.4),
        rrect(10, 8.5, 11, 2.8, 1.4),
        rrect(10, 13.5, 8, 2.8, 1.4),
        rrect(3, 18.5, 13, 2.8, 1.4),
        pts([(3, 8.6), (3, 16.2), (7.4, 12.4)]),
    ]

    # Foundry: a crucible over a drop of molten metal.
    out["crucible"] = [
        rrect(2.5, 2.5, 19, 2.6, 1.3),
        pts([(4, 5.1), (20, 5.1), (17.6, 12.6), (6.4, 12.6)]),
        "M12 14.2C14.5 16.9 15.3 18.3 15.3 19.6A3.3 3.3 0 0 1 8.7 19.6C8.7 18.3 9.5 16.9 12 14.2Z",
    ]

    # Any runtime that runs code: a pair of brackets round a dot.
    out["brackets"] = [
        pts([(3, 3), (8.6, 3), (8.6, 5.5), (5.6, 5.5), (5.6, 18.5), (8.6, 18.5), (8.6, 21), (3, 21)]),
        pts([(21, 3), (15.4, 3), (15.4, 5.5), (18.4, 5.5), (18.4, 18.5), (15.4, 18.5), (15.4, 21), (21, 21)]),
        circle(12, 12, 1.7),
    ]

    # A runtime that runs on the machine itself: a chip with its pins.
    pins = []
    for p in (9, 12, 15):
        pins += [bar(p, 2.6, p, 5.5, 1.6), bar(p, 18.5, p, 21.4, 1.6), bar(2.6, p, 5.5, p, 1.6), bar(18.5, p, 21.4, p, 1.6)]
    out["chip"] = [rrect(5.5, 5.5, 13, 13, 2.4), rrect(8, 8, 8, 8, 0.8, hole=True), rrect(10.4, 10.4, 3.2, 3.2, 0.6), *pins]
    return out


def svg(shapes: list[str]) -> str:
    body = "".join(f'  <path d="{d}"/>\n' for d in shapes)
    return f'<?xml version="1.0" encoding="UTF-8"?>\n<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {SIDE} {SIDE}" fill="currentColor">\n{body}</svg>\n'


def dart(all_glyphs: dict[str, list[str]]) -> str:
    lines = [
        "// Written by tool/runtime_glyphs.py from the same geometry as assets/icons/runtimes/*.svg: do not edit by hand.",
        "import '../sources/agent_marks.dart';",
        "",
        "/// The glyphs a runtime's store may name as its icon, each a monochrome mark on a 24-unit square that takes the text colour.",
        "const runtimeGlyphs = <String, AgentMark>{",
    ]
    for name, shapes in all_glyphs.items():
        lines.append(f"  '{name}': AgentMark({SIDE}, [")
        lines += [f"    ('{d}', null)," for d in shapes]
        lines.append("  ]),")
    lines.append("};")
    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="write nothing; exit 1 when a file differs")
    check = parser.parse_args().check
    all_glyphs = glyphs()
    wanted = {SVG_DIR / f"{name}.svg": svg(shapes) for name, shapes in all_glyphs.items()}
    wanted[DART] = dart(all_glyphs)
    stale = []
    for path, text in wanted.items():
        have = path.read_bytes().decode("utf-8") if path.exists() else None
        if have == text:
            continue
        stale.append(path)
        if not check:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(text.encode("utf-8"))
    for path in stale:
        print(("differs: " if check else "wrote: ") + str(path.relative_to(APP)))
    return 1 if check and stale else 0


if __name__ == "__main__":
    sys.exit(main())
