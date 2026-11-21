"""The `account_provider` kind: how a library adds an account kind, its sign-in, renewal, watch, usage and model listing.

```python
class DemoAccounts(AccountProvider):
    title = "Demo accounts"
    kinds_offered = [
        AccountKindInfo(
            id="demo-key",
            title="Demo gateway",
            settings={
                "type": "object",
                "properties": {"base_url": {"type": "string", "format": "uri"}, "api_key": credential_schema()},
                "required": ["base_url", "api_key"],
            },
            endpoints=[EndpointSlot(purpose="model_api", key="base_url")],
            provides=["openai-api"],
        )
    ]
```
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, ClassVar, Dict, List, Optional, Sequence, Union

from guatiao import Ref, provide
from guatiao.errors import Unsupported
from guatiao.kinds import Taken, ValueAnswer

from . import kinds
from ._model import Model, data_of
from ._provider import WithForm

__all__ = [
    "AccountKindInfo",
    "AccountOffer",
    "AccountProvider",
    "AccountProviderInfo",
    "AccountWatch",
    "EndpointSlot",
    "LoginForm",
    "LoginMethodInfo",
    "LoginOutcome",
    "ModelInfo",
    "OpenUrl",
    "credential_schema",
    "models_listed",
    "models_unknown",
    "release",
    "renew_unchanged",
    "renewed",
    "usage_unsupported",
]


def _plain(value: Any) -> Any:
    return value.to_python() if hasattr(value, "to_python") else value


def release(*handles: Any) -> None:
    """Destroys host object handles (`spawner`, `prompter`, `keeper`) a call was handed."""
    for handle in handles:
        if handle is not None and hasattr(handle, "destroy"):
            handle.destroy()


def credential_schema(title: Optional[str] = None, description: Optional[str] = None) -> Dict[str, Any]:
    """JSON Schema of a secret property: `type: string`, `writeOnly`, `x-sensitive`."""
    schema: Dict[str, Any] = {"type": "string", "writeOnly": True, "x-sensitive": True}
    if title is not None:
        schema["title"] = title
    if description is not None:
        schema["description"] = description
    return schema


def models_listed(models: Sequence[Union["ModelInfo", Dict[str, Any]]]) -> Dict[str, Any]:
    """A known model list."""
    return {"known": True, "models": [data_of(model) for model in models]}


def models_unknown(reason: str, *, unknown_kind: Optional[str] = None) -> Dict[str, Any]:
    """A model listing that could not be answered, with the reason (`not_installed` or `failed` optionally)."""
    result: Dict[str, Any] = {"known": False, "unknown_because": reason, "models": []}
    if unknown_kind is not None:
        result["unknown_kind"] = unknown_kind
    return result


def usage_unsupported(reason: str, *, account: Optional[str] = None) -> Dict[str, Any]:
    """An answer for an account kind whose plan usage is not offered."""
    result: Dict[str, Any] = {
        "supported": False,
        "unsupported_because": reason,
        "sources": [],
        "revision": 0,
        "observed_at": 0,
        "buckets": [],
    }
    if account is not None:
        result["account"] = account
    return result


def renewed(credentials: Dict[str, str], *, expires_at: Optional[str] = None) -> Dict[str, Any]:
    """A renewal that produced fresh credentials."""
    result: Dict[str, Any] = {"credentials": dict(credentials)}
    if expires_at is not None:
        result["expires_at"] = expires_at
    return result


def renew_unchanged(reason: str) -> Dict[str, Any]:
    """A renewal that changed nothing, with the reason."""
    return {"not_renewed_because": reason}


@dataclass
class AccountProviderInfo(Model):
    """Who an account provider is; the fields are `coqui_acp_api::AccountProviderInfo`'s."""

    title: str
    description: str = ""


@dataclass
class EndpointSlot(Model):
    """A URL slot an account kind talks to (`purpose`: `model_api`, `auth` or `proxy`)."""

    purpose: str
    key: Optional[str] = None
    default: Optional[str] = None


@dataclass
class AccountKindInfo(Model):
    """One kind of account a provider makes and keeps."""

    id: str
    title: str
    description: str = ""
    settings: Dict[str, Any] = field(default_factory=lambda: {"type": "object"})
    endpoints: List[EndpointSlot] = field(default_factory=list)
    rotates: bool = False
    history_portable: bool = False
    state_dir: bool = False
    renews: bool = False
    notices: List[str] = field(default_factory=list)
    provides: List[str] = field(default_factory=list)


@dataclass
class LoginMethodInfo(Model):
    """One way a person signs in to make an account."""

    id: str
    title: str
    produces: str
    description: str = ""
    inputs: Dict[str, Any] = field(default_factory=lambda: {"type": "object"})
    steps: List[str] = field(default_factory=list)
    remote: bool = False


@dataclass
class AccountOffer(Model):
    """What an account provider offers: its kinds, login methods, usage kinds and whether it has a local account."""

    kinds: List[AccountKindInfo] = field(default_factory=list)
    logins: List[LoginMethodInfo] = field(default_factory=list)
    usage: List[str] = field(default_factory=list)
    local: bool = False


@dataclass
class LoginForm(Model):
    """A sealed form shown to the person through `LoginPrompter.ask`."""

    title: str
    schema: Dict[str, Any] = field(default_factory=lambda: {"type": "object"})


@dataclass
class OpenUrl(Model):
    """A page the person is asked to open through `LoginPrompter.open_url`."""

    url: str
    user_code: Optional[str] = None
    expires_ms: Optional[int] = None


@dataclass
class LoginOutcome(Model):
    """The account made by `AccountProvider.login`."""

    kind: str
    settings: Dict[str, Any] = field(default_factory=dict)
    credentials: Dict[str, str] = field(default_factory=dict)
    label: Optional[str] = None
    expires_at: Optional[str] = None
    checked: bool = False


@dataclass
class ModelInfo(Model):
    """One model an account may use."""

    id: str
    name: Optional[str] = None
    description: Optional[str] = None
    resolved: Optional[str] = None
    version: Optional[str] = None
    efforts: List[str] = field(default_factory=list)
    default_effort: Optional[str] = None
    is_default: bool = False


class AccountWatch(kinds.AccountWatch):
    """A watch of one account's credentials from `AccountProvider.watch`. Subclass and implement `stop()` to clean up."""

    def stop(self) -> None:
        """Ends watching the account."""


class AccountProvider(WithForm, provide.Provider, kinds.AccountProvider, abstract=True):
    """Subclass it with `title`, `kinds_offered`, `logins` (or override `offer()`) and implement `login`, `usage` and `models` as
    the kinds need. Slots handed host objects (`spawner`, `prompter`, `keeper`) own and must destroy them (`release(...)`)."""

    title: ClassVar[str]
    description: ClassVar[str] = ""
    kinds_offered: ClassVar[Sequence[AccountKindInfo]] = ()
    logins: ClassVar[Sequence[LoginMethodInfo]] = ()
    usage_kinds: ClassVar[Sequence[str]] = ()
    local: ClassVar[bool] = False

    def info(self) -> AccountProviderInfo:
        """The provider's description, from the class attributes."""
        return AccountProviderInfo(title=self.title, description=self.description)

    def describe(self) -> ValueAnswer:
        return self.info().to_data()

    def offer(self) -> AccountOffer:
        """What this provider offers, from the class attributes; override to compute dynamically."""
        return AccountOffer(
            kinds=list(self.kinds_offered),
            logins=list(self.logins),
            usage=list(self.usage_kinds),
            local=self.local,
        )

    def offers(self) -> ValueAnswer:
        return self.offer().to_data()

    def login(
        self,
        request: Ref,
        spawner: Taken[kinds.ProcessSpawner],
        prompter: Taken[kinds.LoginPrompter],
    ) -> ValueAnswer:
        """Signs in to make an account; default refuses as unsupported and destroys the host handles."""
        release(spawner, prompter)
        raise Unsupported("this account provider has no login")

    def renew(self, request: Ref, spawner: Taken[kinds.ProcessSpawner]) -> ValueAnswer:
        """Renews an account's credentials; default refuses as unsupported and destroys `spawner`."""
        release(spawner)
        raise Unsupported("this account kind cannot be renewed")

    def watch(
        self,
        request: Ref,
        spawner: Taken[kinds.ProcessSpawner],
        keeper: Taken[kinds.AccountKeeper],
    ) -> kinds.AccountWatch:
        """Starts watching an account; default refuses as unsupported and destroys `spawner` and `keeper`."""
        release(spawner, keeper)
        raise Unsupported("this account kind has no watch")

    def usage(
        self,
        query: Ref,
        spawner: Taken[kinds.ProcessSpawner],
        keeper: Taken[kinds.AccountKeeper],
    ) -> ValueAnswer:
        """Reports plan usage for an account; default destroys handles and answers unsupported with the account id."""
        try:
            body = _plain(query)
            account = (body.get("account") or {}).get("id") if isinstance(body, dict) else None
        finally:
            release(spawner, keeper)
        return usage_unsupported("this account provider reports no plan limits", account=account)

    def models(
        self,
        query: Ref,
        spawner: Taken[kinds.ProcessSpawner],
        keeper: Taken[kinds.AccountKeeper],
    ) -> ValueAnswer:
        """Lists the models an account may use; default destroys handles and answers unknown."""
        release(spawner, keeper)
        return models_unknown("this account provider does not list models")
