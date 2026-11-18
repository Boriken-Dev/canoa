"""A `secret_provider` for Bitwarden, written in Python: it reads a secret by name through Bitwarden's own `bw` command.

The module is the library: its id is its name, its version `__version__`, its provider the class in `__all__`. It needs no Rust and
no build, so it is shared between hosts as the file it is.

How it talks to the vault. `bw get password NAME` (or `username`, `notes`, `totp`, `uri`, the setting `part` says which) answers the
named item's part, and `bw list items` its names. The vault is opened with a session key the person made with `bw unlock --raw`; the
host gives it as the setting `session`, taken from another secret the person typed, and this module gives it to `bw` in the
environment (`BW_SESSION`), never on a command line, where any program on the machine could read it. Nothing this module prints,
raises or logs holds a value: the failures say what `bw` could not do and never what it printed.

A vault that is locked, signed out or given a wrong key answers `GONE`, so the host asks the person instead. An item that is not
there is no value, not an error. Bitwarden items have no end of their own, so no answer carries one.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
from dataclasses import dataclass, field
from typing import Annotated, Any, Dict, List, Literal, Optional

from canoa_secret_kinds import SecretProvider

from guatiao import Ref, provide
from guatiao.errors import BadValue, Gone, Internal, Unsupported

__version__ = "1.0.0"
__all__ = ["Bitwarden"]

#: The most names one listing answers, as the kind says.
MAX_NAMES = 1000
#: The longest name of a secret and the longest value, as the kind says.
MAX_NAME = 256
MAX_VALUE = 64 * 1024
#: The most of `bw`'s output read: a listing of a large vault with every item's fields is the biggest.
MAX_OUTPUT = 64 * 1024 * 1024

#: What `bw` prints on standard error when the vault cannot be opened, in the words it uses.
LOCKED = ("vault is locked", "not logged in", "invalid master password", "session key is invalid")
MISSING = ("not found",)


@dataclass(frozen=True)
class BitwardenConfig:
    """What an entry of this provider is configured with."""

    #: The program that is Bitwarden's command line.
    command: str = "bw"
    #: What goes before `get` on the command line: for a program that runs `bw` rather than being it.
    arguments: List[str] = field(default_factory=list)
    #: The part of an item that is the secret.
    part: Literal["password", "username", "notes", "totp", "uri"] = "password"
    #: The session key `bw unlock --raw` printed; absent leaves it to `bw`'s own environment.
    session: Annotated[Optional[str], {"writeOnly": True, "x-sensitive": True}] = None
    #: How long one call to `bw` may take, in seconds.
    timeout_seconds: Annotated[int, {"minimum": 1, "maximum": 600}] = 30


def _py(value: Any) -> Any:
    """A value a host object handed over, as plain Python."""
    return value.to_python() if hasattr(value, "to_python") else value


def _name(request: Any) -> str:
    """The name a request asks for, checked as the kind says a name is."""
    body = _py(request)
    if not isinstance(body, dict) or set(body) != {"name"}:
        raise BadValue("a request names one secret and nothing else")
    name = body["name"]
    if not isinstance(name, str) or not name or len(name.encode("utf-8")) > MAX_NAME:
        raise BadValue(f"a secret's name is 1 to {MAX_NAME} bytes of text")
    if any(ord(char) < 32 or ord(char) == 127 for char in name):
        raise BadValue("a secret's name has a control character")
    return name


class Bitwarden(
    provide.Provider,
    SecretProvider,
    id="bitwarden",
    display_name="Bitwarden",
):
    """Reads secrets from a Bitwarden vault through `bw`."""

    def __init__(self, config: BitwardenConfig) -> None:
        self.config = config

    @classmethod
    def available(cls) -> Optional[str]:
        return None

    def _run(self, *words: str) -> str:
        """`bw` with `words`: what it printed, or why it could not."""
        config = self.config
        program = shutil.which(config.command) or config.command
        env = dict(os.environ)
        env["BW_NOINTERACTION"] = "true"
        if config.session:
            env["BW_SESSION"] = config.session
        else:
            env.pop("BW_SESSION", None)
        try:
            done = subprocess.run(  # noqa: S603 - the program is the person's own setting
                [program, *config.arguments, *words, "--nointeraction"],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                env=env,
                timeout=config.timeout_seconds,
                check=False,
            )
        except FileNotFoundError:
            raise Gone(f"{config.command} is not installed: Bitwarden's command line is needed") from None
        except subprocess.TimeoutExpired:
            raise Gone("Bitwarden's command did not answer in time") from None
        except OSError:
            raise Gone("Bitwarden's command could not be started") from None
        if len(done.stdout) > MAX_OUTPUT:
            raise Internal("Bitwarden's command answered more than a vault holds")
        if done.returncode != 0:
            said = done.stderr.decode("utf-8", "replace").lower()
            if any(words_ in said for words_ in MISSING):
                raise _Missing()
            if any(words_ in said for words_ in LOCKED):
                raise Gone("the vault is locked or the session key is not good: unlock it and give the key again")
            raise Gone("Bitwarden's command could not read the vault")
        return done.stdout.decode("utf-8", "replace")

    def describe(self) -> Dict[str, Any]:
        return {
            "title": "Bitwarden",
            "description": "Your Bitwarden vault, read through its `bw` command with a session key you give once.",
            "can_store": False,
            "can_list": True,
            "needs_person": False,
        }

    def names(self) -> Dict[str, Any]:
        try:
            items = json.loads(self._run("list", "items"))
        except _Missing:
            return {"names": []}
        except ValueError:
            raise Internal("Bitwarden's listing is not what `bw list items` prints") from None
        names = sorted(
            {
                item["name"]
                for item in items
                if isinstance(item, dict) and isinstance(item.get("name"), str) and item["name"]
            }
        )
        return {"names": names[:MAX_NAMES]}

    def get(self, request: Ref) -> Dict[str, Any]:
        name = _name(request)
        try:
            text = self._run("get", self.config.part, name)
        except _Missing:
            return {}
        # `bw` may end its answer with one line break of its own; a value's own are inside it.
        value = text[:-2] if text.endswith("\r\n") else text[:-1] if text.endswith("\n") else text
        if len(value.encode("utf-8")) > MAX_VALUE:
            raise BadValue("the value is longer than a secret may be")
        return {"value": value} if value else {}

    def put(self, request: Ref) -> None:
        raise Unsupported("Bitwarden keeps its own items: add this one in Bitwarden")

    def forget(self, request: Ref) -> None:
        raise Unsupported("Bitwarden keeps its own items: remove this one in Bitwarden")


class _Missing(Exception):
    """`bw` said the item is not there: no value, and no failure."""
