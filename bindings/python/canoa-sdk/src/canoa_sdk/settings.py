"""The `settings_section` kind: a page, or a group on a page, of the application's Settings.

```python
class Region(SettingsSection):
    section = "region"
    title = "Region"
    values = RegionValues        # a dataclass: its schema and form are what the section asks for
```

With no `under` the section is a page of its own in Settings' list under the heading `group` ("Extensions" when none); with `under`
naming one of the application's sections (or another provider's) it is a titled group inside that page. Its values are kept in
`[settings.ID]` of canoa.cfg, written by the host after it checked them against `values` (`stored = "config"`, the default), or by the
provider (`stored = "provider"`): the host then calls `read` to show them and `write` with the checked values to keep them.
"""

from __future__ import annotations

from typing import Any, ClassVar, Dict, Optional

from guatiao import Ref, provide
from guatiao._typed_schema import schema_for
from guatiao.kinds import ValueAnswer

from . import kinds
from ._model import present
from ._provider import WithForm
from .forms import form_of

__all__ = ["MAX_VALUES", "SettingsSection"]

#: The most bytes of values a section keeps.
MAX_VALUES = 64 * 1024


class SettingsSection(WithForm, provide.Provider, kinds.SettingsSection, abstract=True):
    """Subclass it with `section` (1 to 32 lower-case letters, digits and `_`), `title` and, for a section that asks for something,
    `values` (a dataclass: Guatiao reads its schema and `canoa_sdk.forms` its form). A member the schema marks sensitive is never
    answered in clear: `read` leaves it out and the host names it as set. `values_schema` and `values_form` give the schema and the
    guatiao-intake form as plain data instead."""

    section: ClassVar[str]
    title: ClassVar[str]
    help: ClassVar[Optional[str]] = None
    icon: ClassVar[Optional[str]] = None
    group: ClassVar[Optional[str]] = None
    under: ClassVar[Optional[str]] = None
    keywords: ClassVar[Optional[str]] = None
    stored: ClassVar[str] = "config"
    values: ClassVar[Optional[type]] = None
    values_schema: ClassVar[Optional[Dict[str, Any]]] = None
    values_form: ClassVar[Optional[Dict[str, Any]]] = None

    def info(self) -> Dict[str, Any]:
        """The description, from the class attributes."""
        schema = self.values_schema
        form = self.values_form
        if self.values is not None:
            schema = schema or schema_for(self.values)
            form = form or form_of(self.values, getattr(self.values, "__sections__", ()))
        return present(
            {
                "id": self.section,
                "title": self.title,
                "help": self.help,
                "icon": self.icon,
                "group": self.group,
                "under": self.under,
                "keywords": self.keywords,
                "stored": self.stored,
                "schema": schema,
                "form": form or None,
            }
        )

    def describe(self) -> ValueAnswer:
        return self.info()

    def read(self) -> ValueAnswer:
        """What the section holds now, as an object keyed by the schema's member names, a sensitive member left out. Asked only of a
        `stored = "provider"` section."""
        return {}

    def write(self, values: Ref) -> None:
        """Keeps `values`, an object the host has already checked against the schema; a sensitive member the person left empty is
        absent, so what was kept stays. Asked only of a `stored = "provider"` section."""
