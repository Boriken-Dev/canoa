"""Stands in for Bitwarden's `bw` command line, for the tests of canoa_bitwarden.

It speaks the parts of `bw` the provider uses, as `bw` does: `get password|username|notes|totp|uri NAME` prints the part, `list items`
prints a JSON array of items, an item that is not there is `Not found.` on standard error with status 1, and a vault whose session
key is not the one it was started to accept is `Vault is locked.`. The items are made up. The file named by `BW_STANDIN_LOG` gets one
line per call, with the words of the call and whether the session key was in the environment (never its value), so a test can see
what was run and that nothing was put on the command line.
"""

import json
import os
import sys

ITEMS = {
    "github": {"password": "gh-pw-zq9-made-up\n", "username": "ana", "notes": "a note", "totp": "123456", "uri": "https://github.com"},
    "empty": {"password": ""},
    "two lines": {"password": "line one\nline two"},
}
GOOD_KEY = "good-session-key-made-up"


def log(words, key):
    path = os.environ.get("BW_STANDIN_LOG")
    if path:
        with open(path, "a", encoding="utf-8") as out:
            out.write(json.dumps({"words": words, "session_in_env": key is not None}) + "\n")


def main(words):
    key = os.environ.get("BW_SESSION")
    log(words, key)
    if "--nointeraction" not in words or os.environ.get("BW_NOINTERACTION") != "true":
        sys.stderr.write("would have asked a question\n")
        return 2
    words = [word for word in words if word != "--nointeraction"]
    if any(GOOD_KEY in word for word in words):
        sys.stderr.write("the key was on the command line\n")
        return 3
    if key != GOOD_KEY:
        sys.stderr.write("Vault is locked.\n")
        return 1
    if words[:2] == ["list", "items"]:
        sys.stdout.write(json.dumps([{"name": name, "login": {"password": "x"}} for name in ITEMS] + [{"name": "github"}]))
        return 0
    if len(words) == 3 and words[0] == "get":
        item = ITEMS.get(words[2])
        if item is None or words[1] not in item:
            sys.stderr.write("Not found.\n")
            return 1
        # As bytes, so no platform turns a line break into another.
        sys.stdout.buffer.write(item[words[1]].encode("utf-8"))
        if not item[words[1]].endswith("\n"):
            sys.stdout.buffer.write(b"\n")
        return 0
    sys.stderr.write("unknown command\n")
    return 4


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
