"""The `notifier` kind: somewhere a notice about what happened on the host is sent.

```python
class Webhook(Notifier):
    notifier = "webhook"
    title = "Webhook"
    values = WebhookValues       # a dataclass: its schema and form are what the notifier asks for

    def send(self, request: Ref) -> ValueAnswer:
        notice, settings = notice_of(request), settings_of(request)
        post(settings["url"], notice["title"], notice["body"])
        return delivery(notice["id"])
```

The host decides which events happen and which notifiers are told; a notifier delivers one notice a `send`. A notice is built by the
host from fixed words and ids (`id`, `event`, `title`, `body`, `session`, `urgency`, `at`), never from what an agent said. `settings`
is the notifier entry's settings, a sensitive member resolved from the host's secret. A failure is an exception: `Gone` when nothing
can be told now, `BadValue` for settings that do not fit, any other for a refusal; each message is one line with no secret.

`reply` and `withdraw` are for a notifier that takes the person's answer (`replies = True`) and need not be written otherwise.
"""

from __future__ import annotations

import json
from typing import Any, ClassVar, Dict, Optional, Sequence

from guatiao import Ref, provide
from guatiao._typed_schema import schema_for
from guatiao.kinds import ValueAnswer

from . import kinds
from ._model import present
from ._provider import WithForm, abstract
from .forms import form_of

__all__ = ["Notifier", "delivery", "notice_of", "settings_of"]


def _plain(request: Ref) -> Dict[str, Any]:
    body = request.to_python() if hasattr(request, "to_python") else request
    return body if isinstance(body, dict) else {}


def notice_of(request: Ref) -> Dict[str, Any]:
    """The notice of a `send` request, as a map."""
    notice = _plain(request).get("notice")
    return notice if isinstance(notice, dict) else {}


def settings_of(request: Ref) -> Dict[str, Any]:
    """The notifier entry's settings of a request, as a map (the request carries them as JSON text)."""
    text = _plain(request).get("settings")
    parsed = json.loads(text) if isinstance(text, str) and text.strip() else {}
    return parsed if isinstance(parsed, dict) else {}


def delivery(identifier: str, replies: bool = False) -> Dict[str, Any]:
    """What `send` answers: the notifier's own `identifier` for the delivered notice."""
    return {"id": identifier, "replies": replies}


class Notifier(WithForm, provide.Provider, kinds.Notifier, abstract=True):
    """Subclass it with `notifier` (1 to 32 lower-case letters, digits and `_`), `title`, optionally `values` (a dataclass: Guatiao
    reads its schema and `canoa_sdk.forms` its form) and implement `send`. `events` limits the events it takes (none takes any);
    `replies = True` says `reply` can answer."""

    notifier: ClassVar[str]
    title: ClassVar[str]
    help: ClassVar[Optional[str]] = None
    events: ClassVar[Optional[Sequence[str]]] = None
    replies: ClassVar[bool] = False
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
                "id": self.notifier,
                "title": self.title,
                "help": self.help,
                "events": list(self.events) if self.events is not None else None,
                "replies": self.replies,
                "schema": schema,
                "form": form or None,
            }
        )

    def describe(self) -> ValueAnswer:
        return self.info()

    @abstract
    def send(self, request: Ref) -> ValueAnswer:
        """Delivers `notice_of(request)` with `settings_of(request)`: answer `delivery(id)`."""
        raise NotImplementedError("send is not implemented")

    def reply(self, request: Ref) -> ValueAnswer:
        """The person's answer to a delivered notice: `{"action": ..., "nonce": ...}`, or `None` for no answer within `wait_ms`."""
        return None

    def withdraw(self, request: Ref) -> None:
        """Takes a delivered notice away, when it was answered elsewhere."""
