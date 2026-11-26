"""The brand a build carries: what the app is called, its identifiers, its environment-variable prefix and its mark.

One file, `app/brand.json` unless a build names another (`--brand FILE`, or `BRAND_FILE`), holds it; every build step reads it
here. Paths in it are relative to the file. Run directly it prints the Flutter `--dart-define` arguments, the runner's
settings or every name as `KEY=value` lines:

    python brand.py [--brand FILE] defines|runner|vars
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
# The names on disk and on the command line: a field the file leaves out takes the default the `Brand` fields describe.
NAMES = ("program", "config_file", "folder", "home_dir")
WINDOWS_DEVICES = {"con", "prn", "aux", "nul", *(f"com{n}" for n in range(1, 10)), *(f"lpt{n}" for n in range(1, 10))}
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
    # The command line's name, and the names derived from it on Linux: the brand's name in lower case, letters and digits, when the
    # file gives none.
    program: str = ""
    # The configuration file's name in every layer: `<program>.cfg` when absent.
    config_file: str = ""
    # The one folder under each operating system's configuration, system and data roots: `name` when absent.
    folder: str = ""
    # The home under the person's own folder, and the per-project folder: `.<program>` when absent.
    home_dir: str = ""

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
            f"--dart-define=BRAND_PROGRAM={self.program}",
            f"--dart-define=BRAND_CONFIG_FILE={self.config_file}",
            f"--dart-define=BRAND_FOLDER={self.folder}",
            f"--dart-define=BRAND_HOME_DIR={self.home_dir}",
            f"--dart-define=BRAND_ACCENT={self.accent}",
            f"--dart-define=BRAND_MARK={json.dumps(self.mark(), separators=(',', ':'))}",
        ]

    def environment(self) -> dict[str, str]:
        """What the Rust build reads: the brand file, from which the host takes its names."""
        return {"BRAND_FILE": str(self.source)}

    def variables(self) -> dict[str, str]:
        """Every name of the brand as the shell scripts read it: upper-case key to value."""
        return {
            "NAME": self.name, "SHORT_NAME": self.short_name, "EXECUTABLE": self.executable, "ID": self.id,
            "ENV_PREFIX": self.env_prefix, "PROGRAM": self.program, "CONFIG_FILE": self.config_file,
            "FOLDER": self.folder, "HOME_DIR": self.home_dir,
        }


def _not_a_component(value: object) -> str | None:
    """Why `value` cannot be one file or folder name on every operating system, or None when it can."""
    if not isinstance(value, str) or not value:
        return "is missing or not text"
    if re.search(r'[/\\:*?"<>|\x00-\x1f]', value) or ".." in value:
        return "is one file or folder name: no separators, colons, wildcards, quotes or control characters"
    if value != value.strip() or value.endswith("."):
        return "cannot start or end with a space, or end with a dot"
    if value.split(".")[0].lower() in WINDOWS_DEVICES:
        return "is a device name on Windows"
    return None


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
        if key not in FIELDS + OPTIONAL + NAMES:
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
    program = data.get("program", re.sub(r"[^a-z0-9]", "", str(data["name"]).lower()))
    if not isinstance(program, str) or not re.fullmatch(r"[a-z][a-z0-9-]{0,30}", program):
        raise BrandError(f"{path}: program is lower-case letters, digits and hyphens, starting with a letter (a unit and a user name)")
    names = {
        "program": program,
        "config_file": data.get("config_file", f"{program}.cfg"),
        "folder": data.get("folder", data["name"]),
        "home_dir": data.get("home_dir", f".{program}"),
    }
    for key in ("config_file", "folder", "home_dir"):
        reason = _not_a_component(names[key])
        if reason:
            raise BrandError(f"{path}: {key} {reason}")
    for key in ("accent",) + tuple(k for k in OPTIONAL if k in data):
        if not isinstance(data[key], str) or not re.fullmatch(r"#[0-9A-Fa-f]{6}", data[key]):
            raise BrandError(f"{path}: {key} is #RRGGBB")
    if re.search(r'["\\\'<>&|^%]', data["name"] + data["short_name"]):
        raise BrandError(f"{path}: name and short_name cannot hold quotes, backslashes or shell and resource markers")
    icon = (path.parent / data["icon"]).resolve()
    if not icon.is_file():
        raise BrandError(f"{path}: icon {data['icon']} is not a file")
    return Brand(**{key: data[key] for key in FIELDS if key != "icon"}, icon=icon, accent_on_dark=data.get("accent_on_dark"), source=path, **names)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--brand", type=Path, help="the brand file (default: app/brand.json)")
    parser.add_argument("what", choices=("defines", "runner", "vars"))
    opts = parser.parse_args()
    try:
        brand = load(opts.brand)
    except BrandError as error:
        sys.exit(f"brand.py: {error}")
    if opts.what == "defines":
        print("\n".join(brand.defines()))
    elif opts.what == "vars":
        print("\n".join(f"{key}={value}" for key, value in brand.variables().items()))
    else:
        print(f"{brand.name}\n{brand.executable}")


if __name__ == "__main__":
    main()
