"""The `sdk_provider` kind: a vendor's client for an account type, opened over the HTTP the host lends.

```python
class VendorClient(SdkProvider):
    def describe(self): return {"title": "Vendor client"}
    def open(self, request, http): return VendorSdk(http)
```

`describe` answers what the client is and implements, with nothing run and no connection opened. `open` returns a `kinds.Sdk`
over `http` (a `kinds.LentHttp`), which the provider owns from then on and destroys.
"""

from __future__ import annotations

from guatiao import Ref, provide
from guatiao.errors import Unsupported
from guatiao.kinds import Taken, ValueAnswer

from . import kinds
from ._provider import WithForm, abstract

__all__ = ["Sdk", "SdkProvider"]


class SdkProvider(WithForm, provide.Provider, kinds.SdkProvider, abstract=True):
    """Subclass it and implement `describe` and `open`."""

    @abstract
    def describe(self) -> ValueAnswer:
        raise Unsupported("describe is not implemented")

    @abstract
    def open(self, request: Ref, http: Taken[kinds.LentHttp]) -> kinds.Sdk:
        raise Unsupported("open is not implemented")


class Sdk(kinds.Sdk):
    """An opened client, answered by `SdkProvider.open`. Override the operations `describe` declared; the others refuse with
    `Unsupported`, and the host asks only a declared one."""

    def models(self) -> ValueAnswer:
        raise Unsupported("this client lists no models")

    def chat(self, request: Ref) -> ValueAnswer:
        raise Unsupported("this client does not chat")

    def embed(self, request: Ref) -> ValueAnswer:
        raise Unsupported("this client does not embed")

    def rerank(self, request: Ref) -> ValueAnswer:
        raise Unsupported("this client does not rerank")
