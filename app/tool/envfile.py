"""The repo's .env: where the SDK, the pub cache and the deps checkout are, for the tools a build starts."""
from __future__ import annotations

import os
import re
from pathlib import Path
from typing import Mapping, MutableMapping

# The only keys read from .env and from the environment.
KEYS = ("FLUTTER_ROOT", "FLUTTER", "PUB_CACHE", "CANOA_DEPS", "CARGO_HOME", "RUSTUP_HOME")

_LINE = re.compile(r"^(?:export\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.*)$")
_VAR = re.compile(r"\$\{([A-Za-z_][A-Za-z0-9_]*)\}")


def parse_env(text: str, environ: Mapping[str, str]) -> dict[str, str]:
    """KEY=VALUE lines of a .env file, for the known keys only; `${VAR}` is the process environment's, nothing else."""
    values: dict[str, str] = {}
    for raw in text.splitlines():
        match = _LINE.match(raw.strip())
        if match is None or match.group(1) not in KEYS:
            continue
        key, value = match.group(1), match.group(2).strip()
        if value[:1] in ("'", '"') and value.count(value[0]) >= 2:
            quote = value[0]
            value = value[1:value.index(quote, 1)]
            if quote == "'":
                values[key] = value
                continue
        else:
            value = re.split(r"\s+#", value, maxsplit=1)[0].strip()
        values[key] = _VAR.sub(lambda m: environ.get(m.group(1), ""), value)
    return values


def load_settings(root: Path, environ: Mapping[str, str]) -> dict[str, str]:
    """The repo's .env, with the process environment winning over it."""
    path = root / ".env"
    settings = parse_env(path.read_text(encoding="utf-8"), environ) if path.is_file() else {}
    settings.update({key: environ[key] for key in KEYS if environ.get(key)})
    return settings


def apply_settings(settings: Mapping[str, str], environ: MutableMapping[str, str]) -> None:
    """Make the settings the environment every tool this script starts sees; SDK and cargo bins go first on PATH."""
    for key in ("PUB_CACHE", "CARGO_HOME", "RUSTUP_HOME", "CANOA_DEPS"):
        if key in settings:
            environ[key] = settings[key]
    bins = []
    flutter = settings.get("FLUTTER_ROOT") or settings.get("FLUTTER")
    if flutter:
        bins.append(str(Path(flutter) / "bin"))
    if "CARGO_HOME" in settings:
        bins.append(str(Path(settings["CARGO_HOME"]) / "bin"))
    if bins:
        environ["PATH"] = os.pathsep.join([*bins, environ.get("PATH", "")])
