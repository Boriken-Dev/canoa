"""The `panel_tab` kind: a kind of tab the application's bottom panel can open.

```python
class Builds(LogPanelTab):
    tab = "builds"
    title = "Builds"

    def lines(self):
        return ["build 41 passed", "build 42 failed"]
```

A tab shows a shell on the host (`shows = "terminal"`, optionally the one `shell` names), a log, lines of text the provider keeps which
the application asks for every couple of seconds while the tab is open (`shows = "log"`, answered by `tail`), or a page the extension
serves on this computer's loopback (`shows = "page"`, at `url`: `http` or `https` on `127.0.0.1`, `[::1]` or `localhost`, which the
application opens in the person's browser).

The classes that mirror what Canoa carries are `TerminalPanelTab` (a shell), `LogPanelTab` (a log whose lines the subclass supplies)
and `PagePanelTab` (a page at `url`).
"""

from __future__ import annotations

from typing import Any, ClassVar, Dict, List, Optional

from guatiao import Ref, provide
from guatiao.kinds import ValueAnswer

from . import kinds
from ._model import present
from ._provider import WithForm

__all__ = ["LogPanelTab", "MAX_TAIL_LINES", "PagePanelTab", "PanelTab", "TerminalPanelTab"]

#: The most lines a `tail` answers.
MAX_TAIL_LINES = 5000


class PanelTab(WithForm, provide.Provider, kinds.PanelTab, abstract=True):
    """Subclass it with `tab` (1 to 32 lower-case letters, digits and `_`), `title` and `shows` (`"terminal"`, `"log"` or `"page"`, which also names `url`); a log answers `tail`."""

    tab: ClassVar[str]
    title: ClassVar[str]
    help: ClassVar[Optional[str]] = None
    icon: ClassVar[Optional[str]] = None
    shows: ClassVar[str] = "log"
    shell: ClassVar[Optional[str]] = None
    url: ClassVar[Optional[str]] = None

    def info(self) -> Dict[str, Any]:
        """The description, from the class attributes."""
        return present(
            {
                "id": self.tab,
                "title": self.title,
                "help": self.help,
                "icon": self.icon,
                "shows": self.shows,
                "shell": self.shell,
                "url": self.url,
            }
        )

    def describe(self) -> ValueAnswer:
        return self.info()

    def tail(self, request: Ref) -> str:
        """The latest lines of the log, oldest first, each ended by a newline; empty for a kind that shows no log."""
        return ""


class TerminalPanelTab(PanelTab, abstract=True):
    """A shell on the host: the one `shell` names, or the host's own choice."""

    shows = "terminal"


class PagePanelTab(PanelTab, abstract=True):
    """A page the extension serves on this computer's loopback: set `url` to `http` or `https` on `127.0.0.1`, `[::1]` or `localhost`."""

    shows = "page"


class LogPanelTab(PanelTab, abstract=True):
    """A log: override `lines()` to answer every line the log holds so far, oldest first; `tail` answers the latest of them."""

    shows = "log"

    def lines(self) -> List[str]:
        """Every line the log holds, oldest first, with no line break."""
        return []

    def tail(self, request: Ref) -> str:
        body = request.to_python() if hasattr(request, "to_python") else request
        asked = body.get("lines") if isinstance(body, dict) else 0
        wanted = min(max(int(asked or 0), 0), MAX_TAIL_LINES)
        latest = self.lines()[-wanted:] if wanted else []
        return "".join(f"{line}\n" for line in latest)
