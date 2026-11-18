"""Writes `src/canoa_sdk/kinds.py` from every `canoa.*.kinds.json` of `crates/canoa/include`, or with `--check` says whether it
is current (exit 1 if not). A kind two documents both describe (the spawner a speech provider and a store both name) is
written once. Needs Guatiao's Python binding on `PYTHONPATH`.
"""

from __future__ import annotations

import json
import pathlib
import sys

from guatiao.kinds import generate

HERE = pathlib.Path(__file__).resolve().parent
PACKAGE = HERE.parent
INCLUDE = PACKAGE.parents[2] / "crates" / "canoa" / "include"
MODULE = PACKAGE / "src" / "canoa_sdk" / "kinds.py"


def merged() -> str:
    kinds: dict[str, dict] = {}
    for path in sorted(INCLUDE.glob("canoa.*.kinds.json")):
        for kind in json.loads(path.read_text(encoding="utf-8"))["kinds"]:
            if kinds.setdefault(kind["name"], kind) != kind:
                raise SystemExit(f"{path.name}: the kind {kind['name']} differs from another document's")
    return json.dumps({"kinds": list(kinds.values())})


def written() -> str:
    return generate(
        merged(),
        source="crates/canoa/include/canoa.*.kinds.json",
        command="python tools/write_kinds.py",
    )


def main() -> int:
    text = written()
    if "--check" in sys.argv[1:]:
        if not MODULE.is_file() or MODULE.read_bytes().decode("utf-8") != text:
            print(f"{MODULE.name} is not what the descriptions write: regenerate it", file=sys.stderr)
            return 1
        return 0
    MODULE.write_bytes(text.encode("utf-8"))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
