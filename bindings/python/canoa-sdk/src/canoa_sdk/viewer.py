"""The `file_viewer` kind: a way to show a file of some type in the application's Files surface.

```python
class Stamp(SurfaceViewer):
    viewer = "stamp"
    title = "Stamp"
    extensions = ["stamp"]

    def surface(self, file):
        return [
            {"version": "1.0", "createSurface": {"surfaceId": "s", "components": [
                {"id": "root", "component": "Text", "text": file.text()},
            ]}},
        ]
```

A viewer names the types it shows (`mime_types`, exact or by family like `image/*`, and `extensions` without the dot) and answers a
file with either an A2UI surface (`shows = "surface"`: the application draws it in its own widgets and the person's theme) or a
self-contained page (`shows = "page"`: one HTML document with its styles, scripts and pictures inline, which the host seals so it can
make no request and the application opens in the person's browser). It is never handed a path: the host reads the file and gives the
viewer its name, type, size and bytes (`ViewedFile`).

The classes that mirror what Canoa carries are `SurfaceViewer` (answers `surface(file)`, a list of A2UI messages) and `PageViewer`
(answers `page(file)`, an HTML document).
"""

from __future__ import annotations

import base64
import json
from dataclasses import dataclass
from typing import Any, ClassVar, Dict, List, Optional

from guatiao import Ref, provide
from guatiao.kinds import ValueAnswer

from . import kinds
from ._model import present
from ._provider import WithForm

__all__ = ["MAX_RENDER_BYTES", "FileViewer", "PageViewer", "SurfaceViewer", "ViewedFile", "viewed_file"]

#: The longest answer to `render` the host takes, in bytes.
MAX_RENDER_BYTES = 4 * 1024 * 1024


@dataclass(frozen=True)
class ViewedFile:
    """The file a viewer is asked to show: what the host read of it, and no path."""

    name: str
    size: int
    data: bytes
    mime: Optional[str] = None
    truncated: bool = False
    section: Optional[str] = None

    def text(self) -> str:
        """The bytes read as UTF-8, a character cut off at the end left out and anything that is not UTF-8 replaced."""
        return self.data.decode("utf-8", errors="replace")


def viewed_file(request: Any) -> ViewedFile:
    """The `ViewedFile` a `render` request carries."""
    body = request.to_python() if hasattr(request, "to_python") else request
    return ViewedFile(
        name=body.get("name", ""),
        size=int(body.get("size", 0)),
        data=base64.b64decode(body.get("data", "") or ""),
        mime=body.get("mime"),
        truncated=bool(body.get("truncated", False)),
        section=body.get("section"),
    )


class FileViewer(WithForm, provide.Provider, kinds.FileViewer, abstract=True):
    """Subclass it with `viewer` (1 to 32 lower-case letters, digits and `_`), `title`, `mime_types` and/or `extensions`, and `shows`
    (`"surface"` or `"page"`), and answer `render`."""

    viewer: ClassVar[str]
    title: ClassVar[str]
    help: ClassVar[Optional[str]] = None
    mime_types: ClassVar[List[str]] = []
    extensions: ClassVar[List[str]] = []
    priority: ClassVar[int] = 0
    shows: ClassVar[str] = "surface"
    opens_sections: ClassVar[bool] = False

    def info(self) -> Dict[str, Any]:
        """The description, from the class attributes."""
        return present(
            {
                "id": self.viewer,
                "title": self.title,
                "help": self.help,
                "mime_types": list(self.mime_types),
                "extensions": list(self.extensions),
                "priority": self.priority,
                "shows": self.shows,
                "opens_sections": self.opens_sections,
            }
        )

    def describe(self) -> ValueAnswer:
        return self.info()

    def render(self, request: Ref) -> str:
        """The file drawn: a JSON array of A2UI messages for `"surface"`, an HTML document for `"page"`."""
        return ""


class SurfaceViewer(FileViewer, abstract=True):
    """A viewer that answers an A2UI surface: override `surface(file)` to answer the list of messages that creates it."""

    shows = "surface"

    def surface(self, file: ViewedFile) -> List[Dict[str, Any]]:
        """The A2UI messages that draw `file`: a `createSurface` whose components include one with the id `root`."""
        return []

    def render(self, request: Ref) -> str:
        return json.dumps(self.surface(viewed_file(request)))


class PageViewer(FileViewer, abstract=True):
    """A viewer that answers a page: override `page(file)` to answer one self-contained HTML document."""

    shows = "page"

    def page(self, file: ViewedFile) -> str:
        """The HTML that shows `file`, with its styles, scripts and pictures inline."""
        return ""

    def render(self, request: Ref) -> str:
        return self.page(viewed_file(request))
