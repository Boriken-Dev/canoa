"""The `config_store` kind: where the host's configuration lives.

```python
class MemoryStore(ConfigStore):
    def describe(self): return {"title": "Memory", "layers": []}
    def read(self): ...
```

One provider instance is one store. Every slot is required: `describe` (what the store is and its layers), `read` (every layer's
tables), `get` (what one path holds, merged), `change` (all-or-none changes in the writable layer) and `revision` (a token that
differs whenever anything in the store did). `canoa.config_store.kinds.json` says what each answers and refuses with.
"""

from __future__ import annotations

from guatiao import Ref, provide
from guatiao.errors import Unsupported
from guatiao.kinds import ValueAnswer

from . import kinds
from ._provider import WithForm, abstract

__all__ = ["ConfigStore"]


class ConfigStore(WithForm, provide.Provider, kinds.ConfigStore, abstract=True):
    """Subclass it and implement the five slots; a slot a store cannot do refuses with `Unsupported`, and one that cannot be reached
    with `Gone`."""

    @abstract
    def describe(self) -> ValueAnswer:
        raise Unsupported("describe is not implemented")

    @abstract
    def read(self) -> ValueAnswer:
        raise Unsupported("read is not implemented")

    @abstract
    def get(self, key: Ref) -> ValueAnswer:
        raise Unsupported("get is not implemented")

    @abstract
    def change(self, changes: Ref) -> ValueAnswer:
        raise Unsupported("change is not implemented")

    @abstract
    def revision(self) -> str:
        raise Unsupported("revision is not implemented")
