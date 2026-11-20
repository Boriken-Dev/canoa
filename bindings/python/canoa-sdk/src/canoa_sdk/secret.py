"""The `secret_provider` kind: somewhere a secret comes from and, for some, goes to.

```python
class Vault(SecretProvider):
    title = "Vault"
    can_list = True

    def get(self, request, spawner):
        return Held("hunter2").to_data() if name_of(request) == "demo" else Held().to_data()
```

A value is text and is never in a description, a name or an error. `get` answers `Held()` (an empty map) for a name it holds
nothing under, and `Held(value, expires_in=seconds)` for one that ends by itself. A provider that cannot be reached or has not been
unlocked raises `guatiao.errors.Gone` with a reason that holds no value, and the host asks the person for the secret instead. One that
is asked for what it cannot do raises `Unsupported`, which is what `put`, `forget` and `names` do here until a class says
`can_store` or `can_list`.

Every call that may run a program is handed the host's `spawner`; a script has no other way to start one, and the provider owns
the handle and drops it (or passes it on to what it runs).

The classes that mirror what Canoa carries are `FormSecretProvider` (the person's form, which holds nothing: the host asks in the
app) and `SystemStoreSecretProvider` (a provider over a store of keys and texts you supply, with the index of names such a store
needs). The operating system's own store is the host's `system` provider: an entry of `[secrets.providers]` names it with
`provider = "system"`.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any, ClassVar, Dict, List, Optional, Tuple

from guatiao import Ref, provide
from guatiao.errors import BadValue, Internal, Unsupported
from guatiao.kinds import Taken, ValueAnswer

from . import kinds
from ._model import Model
from ._provider import WithForm, abstract

__all__ = [
    "FormSecretProvider",
    "Held",
    "ProviderInfo",
    "SecretProvider",
    "SystemStoreSecretProvider",
    "keeping_of",
    "name_of",
]


@dataclass
class ProviderInfo(Model):
    """What a secret provider is and can do: its title, whether it keeps what it is given (`can_store`), lists its names
    (`can_list`), and needs someone at the screen to answer (`needs_person`: a form to fill, a vault to unlock). The host asks
    such a provider of no session whose person is not there."""

    title: str
    description: Optional[str] = None
    can_store: bool = False
    can_list: bool = False
    needs_person: bool = False


@dataclass
class Held(Model):
    """What `get` answers: the value, and the seconds it stays good for when it has an end of its own (a token that expires). With
    neither, nothing is held under the name."""

    value: Optional[str] = None
    expires_in: Optional[int] = None


def _plain(value: Any) -> Any:
    """A value a host object handed over, as plain Python."""
    return value.to_python() if hasattr(value, "to_python") else value


def _checked(name: Any) -> str:
    if not isinstance(name, str) or not name:
        raise BadValue("a secret's name is text that is not empty")
    if any(ord(char) < 32 or ord(char) == 127 for char in name):
        raise BadValue("a secret's name has a control character")
    return name


def name_of(request: Ref) -> str:
    """The name a `get` or `forget` request asks for, checked as the kind says a name is: text that is not empty, with no control
    character. A request that holds anything else is refused, so a field a newer host adds is never dropped silently."""
    body = _plain(request)
    if not isinstance(body, dict) or set(body) != {"name"}:
        raise BadValue("a request names one secret and nothing else")
    return _checked(body["name"])


def keeping_of(request: Ref) -> Tuple[str, str]:
    """The name and the value a `put` request carries, checked the same way; the value is text."""
    body = _plain(request)
    if not isinstance(body, dict) or set(body) != {"name", "value"}:
        raise BadValue("a request keeps one value under one name and nothing else")
    value = body["value"]
    if not isinstance(value, str):
        raise BadValue("a secret is text")
    return _checked(body["name"]), value


class SecretProvider(WithForm, provide.Provider, kinds.SecretProvider, abstract=True):
    """Subclass it with `title` (and the optional attributes of `ProviderInfo`) and implement `get`; add `names`, `put` and
    `forget` for a provider that lists or keeps (and set `can_list` and `can_store`). A provider's settings are its `__init__`
    dataclass, with a form from `form=` (see `canoa_sdk.forms`); a setting that is another secret (the key that unlocks a vault)
    is marked `{"x-from-secret": True}` in its annotation and the host fills it, never the form."""

    title: ClassVar[str]
    description: ClassVar[Optional[str]] = None
    can_store: ClassVar[bool] = False
    can_list: ClassVar[bool] = False
    needs_person: ClassVar[bool] = False

    def info(self) -> ProviderInfo:
        """The description, from the class attributes."""
        return ProviderInfo(self.title, self.description, self.can_store, self.can_list, self.needs_person)

    def describe(self) -> ValueAnswer:
        return self.info().to_data()

    def names(self, spawner: Taken[kinds.ProcessSpawner]) -> ValueAnswer:
        """The names it holds and nothing else: `{"names": [...]}`, every one it holds."""
        raise Unsupported("this provider cannot list what it holds")

    @abstract
    def get(self, request: Ref, spawner: Taken[kinds.ProcessSpawner]) -> ValueAnswer:
        """What it holds under the request's name: a `Held` as a map (`Held().to_data()`, an empty map, for nothing)."""
        raise Unsupported("get is not implemented")

    def put(self, request: Ref, spawner: Taken[kinds.ProcessSpawner]) -> None:
        """Keeps a value, replacing one of that name."""
        raise Unsupported("this provider keeps nothing")

    def forget(self, request: Ref, spawner: Taken[kinds.ProcessSpawner]) -> None:
        """Forgets what it keeps under the name; not having it is no error."""
        raise Unsupported("this provider keeps nothing")


class FormSecretProvider(SecretProvider):
    """The person's form: it holds nothing, so a secret it is to answer is typed in the app, which the host asks itself. It is
    what a host with no provider for a name falls back on; subclass it to change the title or the words."""

    title = "Form"
    description = "Asks you in the app, and keeps the value as long as you choose: once, for the session, until Canoa closes or for a time."
    needs_person = True

    def get(self, request: Ref, spawner: Taken[kinds.ProcessSpawner]) -> ValueAnswer:
        name_of(request)
        return Held().to_data()

    def names(self, spawner: Taken[kinds.ProcessSpawner]) -> ValueAnswer:
        raise Unsupported("a form holds nothing to list")

    def put(self, request: Ref, spawner: Taken[kinds.ProcessSpawner]) -> None:
        raise Unsupported("a form keeps nothing: the host keeps it for as long as you chose")

    def forget(self, request: Ref, spawner: Taken[kinds.ProcessSpawner]) -> None:
        raise Unsupported("a form keeps nothing")


class SystemStoreSecretProvider(SecretProvider, abstract=True):
    """A provider over a store of keys and texts that cannot be listed by what it holds (a credential store): it keeps each secret
    under `s:` and its name and an index of the names under `i:names`, as Canoa's system provider does, so `names` can answer.

    Implement the three calls of the store: `read(key)` (the text, or `None` when there is none), `write(key, text)` and
    `remove(key)` (not having it is no error). A store that cannot be reached raises `guatiao.errors.Gone` with a reason that holds
    no value. One call at a time is made on an instance only if the store needs it: this class holds no lock."""

    title = "Store"
    can_store = True
    can_list = True

    INDEX: ClassVar[str] = "i:names"

    @abstract
    def read(self, key: str) -> Optional[str]:
        raise Unsupported("read is not implemented")

    @abstract
    def write(self, key: str, text: str) -> None:
        raise Unsupported("write is not implemented")

    @abstract
    def remove(self, key: str) -> None:
        raise Unsupported("remove is not implemented")

    def _held(self) -> List[str]:
        text = self.read(self.INDEX)
        if text is None:
            return []
        try:
            names = json.loads(text)
        except ValueError:
            raise Internal("the store's list of names is damaged") from None
        if not isinstance(names, list) or not all(isinstance(each, str) for each in names):
            raise Internal("the store's list of names is damaged")
        return names

    def _index(self, names: List[str]) -> None:
        self.write(self.INDEX, json.dumps(sorted(set(names))))

    def names(self, spawner: Taken[kinds.ProcessSpawner]) -> ValueAnswer:
        return {"names": sorted(self._held())}

    def get(self, request: Ref, spawner: Taken[kinds.ProcessSpawner]) -> ValueAnswer:
        text = self.read(f"s:{name_of(request)}")
        return Held().to_data() if text is None else Held(text).to_data()

    def put(self, request: Ref, spawner: Taken[kinds.ProcessSpawner]) -> None:
        name, value = keeping_of(request)
        names = self._held()
        # The value goes first: an index that names a secret the store lacks would be worse than the reverse.
        self.write(f"s:{name}", value)
        if name not in names:
            self._index([*names, name])

    def forget(self, request: Ref, spawner: Taken[kinds.ProcessSpawner]) -> None:
        name = name_of(request)
        names = self._held()
        self.remove(f"s:{name}")
        if name in names:
            self._index([each for each in names if each != name])
