"""The brand a build carries: what the app is called, its identifiers, its environment-variable prefix and its mark.

One file, `app/brand.json` unless a build names another (`--brand FILE`, or `BRAND_FILE`), holds it; every build step reads it
here. Paths in it are relative to the file. Run directly it prints the Flutter `--dart-define` arguments or the runner's
settings:

    python brand.py [--brand FILE] defines|runner
"""
from __future__ import annotations

import argparse
import json
import os
import re
import sys
import xml.etree.ElementTree as ET
from dataclasses import dataclass
from pathlib import Path

APP = Path(os.path.abspath(__file__)).parents[1]
DEFAULT_FILE = APP / "brand.json"
FIELDS = ("name", "short_name", "executable", "id", "env_prefix", "icon", "accent")
# Optional: the accent where the mark sits on the web app's dark background (the accent itself when absent).
OPTIONAL = ("accent_on_dark",)
SVG = "{http://www.w3.org/2000/svg}"


class BrandError(ValueError):
    pass


@dataclass(frozen=True)
class Brand:
    name: str
    short_name: str
    executable: str
    id: str
    env_prefix: str
    icon: Path
    accent: str
    accent_on_dark: str | None = None
    source: Path = DEFAULT_FILE

    @property
    def accent_rgb(self) -> tuple[int, int, int]:
        return tuple(int(self.accent[i:i + 2], 16) for i in (1, 3, 5))  # type: ignore[return-value]

    @property
    def org(self) -> str:
        """The reverse-DNS id without its last label: `flutter create --org`, with the project name as the last label."""
        return self.id.rsplit(".", 1)[0]

    def mark(self) -> dict[str, object]:
        """The mark's paths as the Dart painter reads them (`BRAND_MARK`): hull, cut, paddle, spiral and the widths."""
        paths = {p.get("id"): p for p in ET.parse(self.icon).getroot().iter(SVG + "path")}
        for needed in ("hull", "cut", "paddle", "spiral"):
            if needed not in paths:
                raise BrandError(f"{self.icon}: no path with id {needed!r}")
        return {
            "hull": paths["hull"].get("d"),
            "smallWeight": float(paths["hull"].get("data-small-weight")),
            "cut": paths["cut"].get("d"),
            "paddle": paths["paddle"].get("d"),
            "spiral": paths["spiral"].get("d"),
            "spiralWidth": float(paths["spiral"].get("stroke-width")),
        }

    def defines(self) -> list[str]:
        """The `--dart-define` arguments that give the app this brand."""
        return [
            f"--dart-define=BRAND_NAME={self.name}",
            f"--dart-define=BRAND_SHORT_NAME={self.short_name}",
            f"--dart-define=BRAND_ID={self.id}",
            f"--dart-define=BRAND_ENV_PREFIX={self.env_prefix}",
            f"--dart-define=BRAND_ACCENT={self.accent}",
            f"--dart-define=BRAND_MARK={json.dumps(self.mark(), separators=(',', ':'))}",
        ]

    def environment(self) -> dict[str, str]:
        """What the Rust build reads: the prefix of the host's environment variables."""
        return {"BRAND_ENV_PREFIX": self.env_prefix}


def load(file: Path | str | None = None) -> Brand:
    """The brand `file` names, else `BRAND_FILE`, else app/brand.json; BrandError says what is wrong with it."""
    path = Path(file or os.environ.get("BRAND_FILE") or DEFAULT_FILE).resolve()
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        raise BrandError(f"{path}: {error}") from error
    if not isinstance(data, dict):
        raise BrandError(f"{path}: not an object")
    for key in data:
        if key not in FIELDS + OPTIONAL:
            raise BrandError(f"{path}: unknown key {key!r}")
    for key in FIELDS:
        if not isinstance(data.get(key), str) or not data[key].strip():
            raise BrandError(f"{path}: {key} is missing or not text")
    if not re.fullmatch(r"[A-Za-z][A-Za-z0-9]*", data["executable"]):
        raise BrandError(f"{path}: executable is letters and digits, starting with a letter")
    if not re.fullmatch(r"[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)+", data["id"]):
        raise BrandError(f"{path}: id is a reverse-DNS name in lower case, such as com.example.app")
    if not re.fullmatch(r"[A-Z][A-Z0-9]*", data["env_prefix"]):
        raise BrandError(f"{path}: env_prefix is upper-case letters and digits, such as MYAPP")
    for key in ("accent",) + tuple(k for k in OPTIONAL if k in data):
        if not isinstance(data[key], str) or not re.fullmatch(r"#[0-9A-Fa-f]{6}", data[key]):
            raise BrandError(f"{path}: {key} is #RRGGBB")
    if re.search(r'["\\\'<>&|^%]', data["name"] + data["short_name"]):
        raise BrandError(f"{path}: name and short_name cannot hold quotes, backslashes or shell and resource markers")
    icon = (path.parent / data["icon"]).resolve()
    if not icon.is_file():
        raise BrandError(f"{path}: icon {data['icon']} is not a file")
    return Brand(**{key: data[key] for key in FIELDS if key != "icon"}, icon=icon, accent_on_dark=data.get("accent_on_dark"), source=path)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--brand", type=Path, help="the brand file (default: app/brand.json)")
    parser.add_argument("what", choices=("defines", "runner"))
    opts = parser.parse_args()
    try:
        brand = load(opts.brand)
    except BrandError as error:
        sys.exit(f"brand.py: {error}")
    if opts.what == "defines":
        print("\n".join(brand.defines()))
    else:
        print(f"{brand.name}\n{brand.executable}")


if __name__ == "__main__":
    main()
