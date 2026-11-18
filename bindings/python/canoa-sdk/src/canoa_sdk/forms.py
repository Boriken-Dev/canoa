"""A provider's settings as a guatiao schema and a guatiao-intake form, written once on a dataclass.

The schema is what Guatiao reads from the dataclass (`str`, `int`, `bool`, `Literal`, `Optional`, `list`, `Annotated`, ...);
the form says how a person is shown it. There is no field type of Canoa's own: `ui()` places a field in a section and adds the
form's placeholder, unit and condition.

```python
@dataclass
class Settings:
    command: Annotated[list[str], ui(section="run", placeholder="whisper-cli")]
    language: Annotated[str, ui(section="run")] = "en"
    translate: bool = False
    beam: Annotated[int, ui(section="run", unit="beams", visible_when=("translate", True))] = 5

class Whisper(SpeechProvider, form=Settings):
    def __init__(self, settings: Settings) -> None: ...
```
"""

from __future__ import annotations

import dataclasses
import typing
from typing import Any, Dict, Iterable, Optional, Tuple

__all__ = ["FORM_KEY", "Section", "UI", "form_meta", "form_of", "section", "ui"]

#: The key a provider's form is kept under in its `meta`: guatiao-intake's `PROVIDER_FORM_KEY`.
FORM_KEY = "form"


class UI(Dict[str, Any]):
    """What `ui()` returns: the schema keyword that places a field in a section (a `dict`, so Guatiao adds it to the field's
    schema) with the form's own words kept beside it."""

    placeholder: Optional[str] = None
    unit: Optional[str] = None
    visible_when: Optional[Dict[str, Any]] = None


@dataclasses.dataclass(frozen=True)
class Section:
    """A section of the form: its id, what it is called and, optionally, a line under it."""

    id: str
    title: str
    description: Optional[str] = None


def section(id: str, title: str, description: Optional[str] = None) -> Section:
    """A section a form lists; a field names it in `ui(section=...)`."""
    return Section(id, title, description)


def ui(
    *,
    section: Optional[str] = None,
    placeholder: Optional[str] = None,
    unit: Optional[str] = None,
    visible_when: Optional[Tuple[str, Any]] = None,
) -> UI:
    """Metadata for `Annotated[T, ui(...)]`: the section the field sits in, a placeholder, a unit, and a field it is shown
    only while another has a value (`visible_when=("translate", True)`)."""
    keywords = UI({"x-section": section} if section is not None else {})
    keywords.placeholder = placeholder
    keywords.unit = unit
    keywords.visible_when = (
        {"field": visible_when[0], "equals": visible_when[1]} if visible_when is not None else None
    )
    return keywords


def form_of(settings: type, sections: Iterable[Section] = ()) -> Dict[str, Any]:
    """The guatiao-intake form document of the dataclass `settings`: its sections, as given, and what each field's `ui()` says.

    A section a field names that is not listed is listed with its id for a title."""
    if not (dataclasses.is_dataclass(settings) and isinstance(settings, type)):
        raise TypeError("a form is written on a dataclass")
    listed = list(sections)
    fields: Dict[str, Dict[str, Any]] = {}
    for name, hint in typing.get_type_hints(settings, include_extras=True).items():
        for each in getattr(hint, "__metadata__", ()):
            if not isinstance(each, UI):
                continue
            entry = {
                key: value
                for key, value in (
                    ("placeholder", each.placeholder),
                    ("unit", each.unit),
                    ("visibleWhen", each.visible_when),
                )
                if value is not None
            }
            if entry:
                fields[name] = entry
            named = each.get("x-section")
            if named is not None and all(s.id != named for s in listed):
                listed.append(Section(named, named))
    document: Dict[str, Any] = {}
    if listed:
        document["sections"] = [
            {"id": s.id, "title": s.title, **({"description": s.description} if s.description else {})}
            for s in listed
        ]
    if fields:
        document["fields"] = fields
    return document


def form_meta(settings: type) -> Dict[str, Any]:
    """The `meta` entry carrying `settings`' form: what a provider class given `form=settings` publishes.

    The sections are the dataclass's own `__sections__` tuple of `section()`, when it has one."""
    return {FORM_KEY: form_of(settings, getattr(settings, "__sections__", ()))}
