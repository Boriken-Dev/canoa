"""The `environment` kind: a place an agent works, with its own spawner and, when it declares them, files and capabilities.

```python
class Sandbox(Environment):
    def describe(self): return {"title": "Sandbox"}
    def spawner(self): return SandboxSpawner()
```

One provider instance is one configuration of one place. `describe` and `spawner` are required; `read_file`, `write_file`,
`list_dir` and `call` are asked only of an environment that declares the capability in `describe`, and refuse with `Unsupported`
until overridden. Every slot may be called from several threads at once.
"""

from __future__ import annotations

from guatiao import Ref, provide
from guatiao.errors import Unsupported
from guatiao.kinds import ValueAnswer

from . import kinds
from ._provider import WithForm, abstract

__all__ = ["Environment"]


class Environment(WithForm, provide.Provider, kinds.Environment, abstract=True):
    """Subclass it, answer `describe` and hand out a `spawner`; override the file and `call` slots for what `describe` declares."""

    @abstract
    def describe(self) -> ValueAnswer:
        raise Unsupported("describe is not implemented")

    @abstract
    def spawner(self) -> kinds.ProcessSpawner:
        """The spawner programs are started through in this environment; the host destroys it when it has no more to start."""
        raise Unsupported("spawner is not implemented")

    def read_file(self, request: Ref) -> ValueAnswer:
        raise Unsupported("this environment does not read files")

    def write_file(self, request: Ref) -> None:
        raise Unsupported("this environment does not write files")

    def list_dir(self, request: Ref) -> ValueAnswer:
        raise Unsupported("this environment does not list folders")

    def call(self, request: Ref) -> ValueAnswer:
        raise Unsupported("this environment declares no capability to call")
