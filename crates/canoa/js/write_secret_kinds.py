"""Writes `canoa_secret_kinds.js`, the `secret_provider` kind for a library written in JavaScript, from the kinds' description, or
with `--check` says whether it is current (exit 1 if not). It needs nothing but Python.

A library imports `SecretProvider` from it and names it in `static kinds`; the host gives the source the same description, which is
the one that counts, so a module that describes the kind differently is refused.
"""

import json
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
DOCUMENT = HERE.parent / "include" / "canoa.secret.kinds.json"
MODULE = HERE / "canoa_secret_kinds.js"


def written() -> str:
    document = json.loads(DOCUMENT.read_text(encoding="utf-8"))
    body = json.dumps(document, indent=2, ensure_ascii=False)
    return (
        "// GENERATED from include/canoa.secret.kinds.json. DO NOT EDIT.\n"
        "// Regenerate: python js/write_secret_kinds.py\n"
        "\n"
        "import { kindsOf } from 'guatiao/kinds';\n"
        "\n"
        f"const described = kindsOf({body});\n"
        "\n"
        "/** Somewhere a secret comes from and, for some, goes to. */\n"
        'export const SecretProvider = described["secret_provider"];\n'
    )


def main() -> int:
    text = written()
    if "--check" in sys.argv[1:]:
        if MODULE.read_bytes().decode("utf-8") != text:
            print(f"{MODULE.name} is not what the description writes: regenerate it", file=sys.stderr)
            return 1
        return 0
    MODULE.write_bytes(text.encode("utf-8"))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
