"""The `model_gateway` kind: a model that runs on this computer, served to an agent the way a cloud endpoint is.

```python
class LocalModels(ModelGateway):
    kind = "local-models"
    vendor = "local"

    def open(self, request, spawner):
        spawner.destroy()
        return LocalLease()
```

A `ModelGateway` opens a `GatewayLease` for an account of its `kind` (made by an `account_provider` of the same library): the lease
answers the loopback address (`http://127.0.0.1:PORT`, with no path), a per-session bearer and the models the gateway serves.
Destroying a lease ends that session's access. Note that `canoa-host` currently loads `model_gateway` providers from bundled
libraries; extensions carry the contract and `account_provider` loads dynamically from `canoa.cfg`.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, ClassVar, Dict, Optional

from guatiao import Ref, provide
from guatiao.errors import Unsupported
from guatiao.kinds import Taken, ValueAnswer

from . import kinds
from ._model import Model
from ._provider import WithForm, abstract
from .account import models_unknown

__all__ = ["GatewayInfo", "GatewayLease", "ModelGateway", "request_of"]


@dataclass
class GatewayInfo(Model):
    """Who a gateway serves and what it keeps; the fields are `canoa::gateway::GatewayInfo`'s."""

    kind: str
    vendor: str
    table: Optional[str] = None


def request_of(request: Ref) -> Dict[str, Any]:
    """The `GatewayRequest` (`account`, `settings`, `table`, `home`, `helper`) as a plain Python dictionary."""
    data = request.to_python() if hasattr(request, "to_python") else request
    return dict(data)


class GatewayLease(kinds.GatewayLease):
    """One session's access to a gateway. Destroying it ends the session's access, and the last one for an account ends that
    account's gateway."""

    def address(self) -> str:
        """Where the model's API is: `http://127.0.0.1:PORT`, with no path."""
        raise Unsupported("address is not implemented")

    def bearer(self) -> str:
        """The bearer the agent shows, good for this session alone and for a program of this user."""
        raise Unsupported("bearer is not implemented")

    def models(self) -> ValueAnswer:
        """The models this gateway could serve, as an account provider lists them; unknown when it cannot list."""
        return models_unknown("this gateway does not list models")


class ModelGateway(WithForm, provide.Provider, kinds.ModelGateway, abstract=True):
    """Subclass it with `kind` (the account kind it serves) and `vendor` (and optional `table` of `canoa.cfg`), and implement
    `open(request, spawner) -> GatewayLease`.
    """

    kind: ClassVar[str]
    vendor: ClassVar[str]
    table: ClassVar[Optional[str]] = None

    def info(self) -> GatewayInfo:
        """The description, from the class attributes."""
        return GatewayInfo(kind=self.kind, vendor=self.vendor, table=self.table)

    def describe(self) -> ValueAnswer:
        return self.info().to_data()

    @abstract
    def open(self, request: Ref, spawner: Taken[kinds.ProcessSpawner]) -> kinds.GatewayLease:
        """Opens a session lease on the gateway of `request.account`. `spawner` is the host's and the gateway destroys it."""
        raise Unsupported("open is not implemented")
