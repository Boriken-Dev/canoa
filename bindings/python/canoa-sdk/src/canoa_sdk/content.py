"""The `content_source` and `catalog_generator` kinds: a file or a tree obtained by URI, and what turns it into a catalog.

```python
class Team(ContentSource):
    source = "team_files"
    title = "A team's files"
    schemes = [scheme("team", "The team's files", "team://name/path")]

    def open(self, request):
        uri = fields_of(request)["target"]["uri"]
        return opened_inline(stat_file(), read_somehow(uri))


class TeamSkills(CatalogGenerator):
    generator = "team_skills"
    title = "A team's skills"
    types = ["skills"]

    def generate(self, request, spawner, host):
        try:
            return host.open_text(host.child(request_of(request)["source"], "skills.json"), 1 << 20)
        finally:
            spawner.destroy()
            host.destroy()
```

A **content source** serves URI schemes. `describe` lists them (`scheme(name, title, grammar, lists=False, resolves=False)`); the host calls
`stat`, `list`, `open` and `resolve` for a URI of one of them, with a `target` (`uri`, `person`: whether the person wrote the URI, and
`timeout_ms`). A question the scheme cannot answer is left unimplemented: the host is told `UNSUPPORTED`. An expected failure of the
remote place (not found, denied, rate limited) is not an exception: answer it with `refused(kind, why)`. It only reads.

A **catalog generator** turns what a source holds into a catalog in a Store type's format. `generate` is handed the request (the type,
the label, the entry's `source`, the form's `fields` as JSON), the host's `spawner` (a program it starts, a file it reads by absolute
path) and a `host` to read through (`stat`, `list`, `open_text`, `open_json`, `open_bytes`, `resolve`, `child`), which keeps to the entry's
source and the places the generator names in `endpoints`. The generator destroys both objects, even when it refuses.
"""

from __future__ import annotations

import json
from typing import Any, ClassVar, Dict, Optional, Sequence

from guatiao import Ref, provide
from guatiao._typed_schema import schema_for
from guatiao.errors import BadValue, Unsupported
from guatiao.kinds import Taken, ValueAnswer

from . import kinds
from ._model import present
from ._provider import WithForm, abstract
from .forms import form_of

__all__ = [
    "CatalogGenerator",
    "ContentSource",
    "fields_json",
    "list_found",
    "opened_inline",
    "refused",
    "request_of",
    "resolved",
    "scheme",
    "stat_dir",
    "stat_file",
    "stat_found",
]


def _plain(value: Any) -> Any:
    return value.to_python() if hasattr(value, "to_python") else value


def request_of(request: Ref) -> Dict[str, Any]:
    """A request the host made, as a plain dict (a `GenerateRequest`, a `Target`, an `OpenRequest`, a `ListRequest`)."""
    body = _plain(request)
    if not isinstance(body, dict):
        raise BadValue("the request is a map")
    return body


def scheme(name: str, title: str, grammar: str, *, lists: bool = False, resolves: bool = False, ranges: bool = False) -> Dict[str, Any]:
    """One URI scheme a content source serves."""
    return {"name": name, "title": title, "grammar": grammar, "lists": lists, "resolves": resolves, "ranges": ranges}


def stat_file(*, size: Optional[int] = None, etag: Optional[str] = None, content_type: Optional[str] = None, revision: Optional[str] = None, immutable: bool = False) -> Dict[str, Any]:
    """What a source knows of a file without reading it."""
    return present({"kind": "file", "size": size, "etag": etag, "content_type": content_type, "revision": revision, "immutable": immutable, "digests": []})


def stat_dir() -> Dict[str, Any]:
    """What a source knows of a folder."""
    return {"kind": "dir", "digests": [], "immutable": False}


def stat_found(stat: Dict[str, Any]) -> Dict[str, Any]:
    """The answer of `stat`."""
    return {"stat": stat}


def list_found(entries: Sequence[Dict[str, Any]], next: Optional[str] = None) -> Dict[str, Any]:
    """The answer of `list`: entries `{"path", "kind"}` relative to the listed URI, and the cursor of the next page."""
    return present({"listing": {"entries": list(entries), "next": next}})


def opened_inline(stat: Dict[str, Any], data: bytes) -> Dict[str, Any]:
    """The answer of `open`: the file's bytes, inline."""
    return {"opened": {"stat": {**stat, "size": stat.get("size", len(data))}, "delivery": {"inline": bytes(data)}}}


def resolved(uri: str, revision: str, immutable: bool = False) -> Dict[str, Any]:
    """The answer of `resolve`: the same place with its revision fixed."""
    return {"resolved": {"uri": uri, "revision": revision, "immutable": immutable}}


def refused(kind: str, why: str, retry_after_seconds: Optional[int] = None) -> Dict[str, Any]:
    """An expected failure of the remote place as the answer of any question: `kind` is `not_found`, `denied`, `rate_limited`,
    `bad_reply`, `too_large`, `unreachable`, `timeout`, `refused_place` or `changed`. `why` names the scheme and the place and holds no
    credential."""
    return {"refusal": present({"kind": kind, "why": why, "retry_after_seconds": retry_after_seconds})}


class ContentSource(WithForm, provide.Provider, kinds.ContentSource, abstract=True):
    """Subclass it with `source` (the id `[host.disabled]` names it by), `title` and `schemes`, and implement what the schemes answer:
    `stat`, `list`, `open` and `resolve`. A slot left alone answers `UNSUPPORTED`."""

    source: ClassVar[str]
    title: ClassVar[str]
    help: ClassVar[Optional[str]] = None
    schemes: ClassVar[Sequence[Dict[str, Any]]] = ()
    endpoints: ClassVar[Sequence[str]] = ()

    def __init_subclass__(cls, *, id: Optional[str] = None, abstract: bool = False, **keywords: Any) -> None:
        if id is None and not abstract and isinstance(getattr(cls, "source", None), str):
            id = cls.source
        if id is not None:
            keywords["id"] = id
        super().__init_subclass__(abstract=abstract, **keywords)

    def info(self) -> Dict[str, Any]:
        """The description, from the class attributes."""
        return present(
            {
                "id": self.source,
                "title": self.title,
                "help": self.help,
                "schemes": [dict(each) for each in self.schemes],
                "endpoints": list(self.endpoints),
            }
        )

    def describe(self) -> ValueAnswer:
        return self.info()

    def stat(self, target: Ref) -> ValueAnswer:
        raise Unsupported("stat is not implemented")

    def list(self, request: Ref) -> ValueAnswer:
        raise Unsupported("list is not implemented")

    def open(self, request: Ref) -> ValueAnswer:
        raise Unsupported("open is not implemented")

    def resolve(self, target: Ref) -> ValueAnswer:
        raise Unsupported("resolve is not implemented")


class CatalogGenerator(WithForm, provide.Provider, kinds.CatalogGenerator, abstract=True):
    """Subclass it with `generator` (the id canoa.cfg records), `title`, `types` (the Store types it serves, none for every type),
    `reads` (`"required"`, `"optional"` or `"absent"`: whether it reads the entry's `source`) and, for a generator that asks for
    something, `values` (a dataclass: Guatiao reads its schema and `canoa_sdk.forms` its form), and implement `generate`."""

    generator: ClassVar[str]
    title: ClassVar[str]
    help: ClassVar[Optional[str]] = None
    types: ClassVar[Sequence[str]] = ()
    reads: ClassVar[str] = "required"
    grammar: ClassVar[Optional[str]] = None
    endpoints: ClassVar[Sequence[str]] = ()
    keys: ClassVar[Sequence[str]] = ()
    values: ClassVar[Optional[type]] = None
    values_schema: ClassVar[Optional[Dict[str, Any]]] = None
    values_form: ClassVar[Optional[Dict[str, Any]]] = None

    def __init_subclass__(cls, *, id: Optional[str] = None, abstract: bool = False, **keywords: Any) -> None:
        if id is None and not abstract and isinstance(getattr(cls, "generator", None), str):
            id = cls.generator
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
                "id": self.generator,
                "title": self.title,
                "help": self.help,
                "types": list(self.types),
                "source": self.reads,
                "grammar": self.grammar,
                "endpoints": list(self.endpoints),
                "keys": list(self.keys),
                "schema": schema,
                "form": form or None,
            }
        )

    def describe(self) -> ValueAnswer:
        return self.info()

    @abstract
    def generate(self, request: Ref, spawner: Taken[kinds.ProcessSpawner], host: Taken[kinds.ContentHost]) -> str:
        """The catalog's text."""
        raise Unsupported("generate is not implemented")


def fields_json(request: Ref) -> Dict[str, Any]:
    """What the form's fields hold, as a dict; a field the person left empty is absent."""
    text = request_of(request).get("fields")
    try:
        fields = json.loads(text) if isinstance(text, str) else None
    except ValueError:
        fields = None
    if not isinstance(fields, dict):
        raise BadValue("the fields are a JSON object")
    return fields
