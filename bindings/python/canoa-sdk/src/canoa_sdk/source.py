"""The `catalog_source` kind: where a Store type's catalog comes from, and the form the person fills in to name one.

```python
class Team(CatalogSource):
    source = "team"
    title = "A team's list"
    types = ["skills"]

    def read(self, request, spawner):
        spawner.destroy()
        return json.dumps({"skills": []})
```

The Store lists, for a type, the sources that serve it; the person picks one and fills in its `values`; canoa.cfg records the source's
id and those values. When a catalog is refreshed the host calls `read` with the type, the label and the values as JSON text, and the
source answers with the catalog's text in the form the type reads, or raises with the reason, which the Store shows beside the last
good catalog it keeps.

A source that runs a program or reads a file does it through the `spawner` `read` is handed: it starts a program (`spawn`) and reads a
file anywhere the person's account can (`read_file`, by an absolute path). A script has no other way to do either, and the source
destroys the spawner it was handed, even when it refuses.

The classes that mirror what Canoa carries are `CommandCatalogSource` (a program the person named, run on each refresh, whose output
is the catalog) and `ReadCatalogSource` (a place the Store type reads itself: a git source, an address or a file).
"""

from __future__ import annotations

import json
import time
from typing import Any, ClassVar, Dict, List, Optional, Sequence

from guatiao import Ref, provide
from guatiao._typed_schema import schema_for
from guatiao.errors import BadValue, Internal, NotFound, Unsupported
from guatiao.kinds import Taken, ValueAnswer

from . import kinds
from ._model import present
from ._provider import WithForm, abstract
from .forms import form_of

__all__ = [
    "CatalogSource",
    "CommandCatalogSource",
    "DEFAULT_SECONDS",
    "MAX_BYTES",
    "ReadCatalogSource",
    "TAIL_BYTES",
    "fields_of",
    "text_of_catalog",
]

#: The most bytes of a catalog a source may answer with.
MAX_BYTES = 4 * 1024 * 1024
#: How long `CommandCatalogSource` lets a program run, unless the form says otherwise, in seconds.
DEFAULT_SECONDS = 60
#: How much of what a failed program printed on its error stream is kept for the reason.
TAIL_BYTES = 2 * 1024
#: How long one read of a pipe waits before the clock is looked at again, in milliseconds.
_POLL = 100


def _plain(value: Any) -> Any:
    return value.to_python() if hasattr(value, "to_python") else value


def fields_of(request: Ref) -> Dict[str, Any]:
    """What the form's fields hold, as a dict keyed by the schema's member names; a field the person left empty is absent."""
    body = _plain(request)
    text = body.get("fields") if isinstance(body, dict) else None
    try:
        fields = json.loads(text) if isinstance(text, str) else None
    except ValueError:
        fields = None
    if not isinstance(fields, dict):
        raise BadValue("the fields are a JSON object")
    return fields


def text_of_catalog(data: bytes) -> str:
    """A catalog's bytes as text, or why they are not one."""
    if len(data) > MAX_BYTES:
        raise Internal(f"the catalog holds {len(data)} bytes, more than the {MAX_BYTES} this host reads")
    try:
        return data.decode("utf-8-sig")
    except UnicodeDecodeError:
        raise Internal("the catalog is not text") from None


def _release(spawner: Any) -> None:
    destroy = getattr(spawner, "destroy", None)
    if callable(destroy):
        destroy()


class CatalogSource(WithForm, provide.Provider, kinds.CatalogSource, abstract=True):
    """Subclass it with `source` (the id canoa.cfg records as `provider`), `title`, `types` (the Store types it serves, none for every
    type) and, for a source that asks for something, `values` (a dataclass: Guatiao reads its schema and `canoa_sdk.forms` its form),
    and implement `read`. `values_schema` and `values_form` give the schema and the guatiao-intake form as plain data instead."""

    source: ClassVar[str]
    title: ClassVar[str]
    help: ClassVar[Optional[str]] = None
    types: ClassVar[Sequence[str]] = ()
    values: ClassVar[Optional[type]] = None
    values_schema: ClassVar[Optional[Dict[str, Any]]] = None
    values_form: ClassVar[Optional[Dict[str, Any]]] = None

    def __init_subclass__(cls, *, id: Optional[str] = None, abstract: bool = False, **keywords: Any) -> None:
        # The host finds a source by the id it is described under, so a class that names its `source` is the provider of that id.
        if id is None and not abstract and isinstance(getattr(cls, "source", None), str):
            id = cls.source
        if id is not None:
            keywords["id"] = id
        super().__init_subclass__(abstract=abstract, **keywords)

    def info(self) -> Dict[str, Any]:
        """The description, from the class attributes."""
        schema = self.values_schema
        form = self.values_form
        if self.values is not None:
            schema = schema or schema_for(self.values)
            form = form or form_of(self.values, getattr(self.values, "__sections__", ()))
        return present(
            {
                "id": self.source,
                "title": self.title,
                "help": self.help,
                "types": list(self.types),
                "schema": schema,
                "form": form or None,
            }
        )

    def describe(self) -> ValueAnswer:
        return self.info()

    @abstract
    def read(self, request: Ref, spawner: Taken[kinds.ProcessSpawner]) -> str:
        """The catalog's text."""
        raise Unsupported("read is not implemented")


class ReadCatalogSource(CatalogSource, abstract=True):
    """A place the Store type reads itself: a git repository, a web address or a file, in the form the type reads. The host's own reader
    does the reading of the first two, since it keeps what it fetched by commit and checks it; `read` answers a file on this computer."""

    source = "read"
    title = "Read an address or a file"
    help = "A git repository, a web address or a file on this computer, in the form the type reads."
    values_schema = {
        "type": "object",
        "additionalProperties": False,
        "required": ["source"],
        "properties": {
            "source": {
                "type": "string",
                "title": "Source",
                "description": "Overlays and extensions: <repository>@<ref>#<file>. Skills: owner/repo, a git URL or a folder. MCP servers: the registry's address.",
            }
        },
    }

    def read(self, request: Ref, spawner: Taken[kinds.ProcessSpawner]) -> str:
        try:
            source = fields_of(request).get("source")
            if not isinstance(source, str) or not source:
                raise BadValue("the source is needed")
            try:
                data = bytes(spawner.read_file(source, MAX_BYTES + 1))
            except NotFound:
                raise Internal(f"{source} cannot be read: it is not there") from None
            except Exception as error:  # a refusal of the host: its words say why
                raise Internal(f"{source} cannot be read: {error}") from None
            return text_of_catalog(data)
        finally:
            _release(spawner)


def _absolute(path: str) -> bool:
    return path.startswith(("/", "\\")) or (len(path) > 2 and path[1] == ":" and path[2] in "/\\")


def _seconds_of(value: Any) -> int:
    wrong = Internal("the time it may run is a whole number of seconds")
    if value is None:
        return DEFAULT_SECONDS
    if isinstance(value, bool):
        raise wrong
    if isinstance(value, int):
        if value < 0:
            raise wrong
        return value
    if isinstance(value, str) and value.strip().isdigit():
        return int(value.strip())
    raise wrong


def _drain(read: Any, deadline: float, take: Any) -> bool:
    """Reads one stream until it ends or the clock passes `deadline`, giving each piece to `take`; False when the deadline came first."""
    buffer = bytearray(16 * 1024)
    while True:
        if time.monotonic() > deadline:
            return False
        got = int(read(buffer, _POLL))
        if got == 0:
            return True
        if got < 0:
            continue  # nothing arrived yet, or the link is down: look at the clock and ask again
        take(bytes(buffer[:got]))


class CommandCatalogSource(CatalogSource, abstract=True):
    """A program the person named, run on each refresh, whose standard output (or a file it writes, `file`, in its folder `cwd`) is the
    catalog. It runs as the person, with their environment, and serves every type: what it prints is read in the form the type
    documents (an overlay or extension catalog file, a JSON list of skills, a registry's list of MCP servers)."""

    source = "command"
    title = "Run a program"
    help = (
        "A program and its arguments, run each time the catalog is refreshed, as you, with your environment. What it prints is the "
        "catalog, in the form the type reads: an overlay or extension catalog file, a JSON list of skills, or a registry's list of MCP "
        "servers."
    )
    values_schema = {
        "type": "object",
        "additionalProperties": False,
        "required": ["command"],
        "properties": {
            "command": {
                "type": "array",
                "items": {"type": "string"},
                "title": "Program and arguments",
                "description": "The program first, then each argument.",
            },
            "cwd": {"type": "string", "title": "Folder", "description": "Where it runs; the host's own folder when empty."},
            "file": {
                "type": "string",
                "title": "Output file",
                "description": "A file it writes, in its folder, to take instead of what it prints.",
            },
            "timeout": {
                "type": "integer",
                "minimum": 0,
                "default": DEFAULT_SECONDS,
                "title": "Seconds it may run",
                "description": "How long it may run.",
            },
        },
    }
    values_form = {"fields": {"timeout": {"unit": "s"}}}

    def read(self, request: Ref, spawner: Taken[kinds.ProcessSpawner]) -> str:
        try:
            return self._run(fields_of(request), spawner)
        finally:
            _release(spawner)

    def _run(self, fields: Dict[str, Any], spawner: Any) -> str:
        command = fields.get("command")
        words = [word for word in command if isinstance(word, str)] if isinstance(command, list) else []
        if not words or not words[0]:
            raise Internal("the command is a program, then its arguments")
        program, args = words[0], words[1:]
        seconds = _seconds_of(fields.get("timeout"))
        cwd = fields.get("cwd")
        folder = cwd if isinstance(cwd, str) and cwd else None
        named = fields.get("file")
        file = named if isinstance(named, str) and named else None

        spec: Dict[str, Any] = {"program": program, "args": args, "error_output": "separate"}
        if folder is not None:
            spec["cwd"] = folder
        try:
            process = spawner.spawn(spec)
        except Exception as error:
            raise Internal(f"{program} could not start: {error}") from None
        try:
            try:
                process.close_input()  # the program has no input: it reads the end at once
            except Exception:
                pass
            deadline = time.monotonic() + seconds
            printed: List[bytes] = []
            kept = [0, 0]  # bytes kept, bytes seen

            def take(piece: bytes) -> None:
                kept[1] += len(piece)
                # Past what is kept the program is still read, so one that prints on never waits for ever on a full pipe.
                if kept[0] <= MAX_BYTES:
                    printed.append(piece)
                    kept[0] += len(piece)

            said = bytearray()

            def take_error(piece: bytes) -> None:
                said.extend(piece)
                del said[:-TAIL_BYTES]

            ended = _drain(process.read, deadline, take) and _drain(process.read_error, deadline, take_error)
            exit_ = process.wait(max(1, int((deadline - time.monotonic()) * 1000))) if ended else None
            tail = said.decode("utf-8", "replace").strip()

            def why(what: str) -> str:
                return f"{what}:\n{tail}" if tail else what

            running = exit_ is None or _plain(exit_).get("running")
            if running:
                process.kill()
                raise Internal(why(f"{program} was stopped after {seconds} s"))
            code = _plain(exit_).get("code")
            if code != 0:
                raise Internal(why(f"{program} ended with exit code {code}"))
            data = b"".join(printed)
            if file is not None:
                path = file if _absolute(file) or folder is None else f"{folder.rstrip('/' + chr(92))}/{file}"
                try:
                    data = bytes(spawner.read_file(path, MAX_BYTES + 1))
                except NotFound:
                    raise Internal(f"{path} cannot be read: it is not there") from None
                except Exception:
                    raise Internal(f"{path} cannot be read: refused") from None
            elif kept[1] > MAX_BYTES:
                raise Internal(f"the catalog holds more than the {MAX_BYTES} bytes this host reads")
            if not data:
                raise Internal(why(f"{program} printed no catalog"))
            return text_of_catalog(data)
        finally:
            process.destroy()
