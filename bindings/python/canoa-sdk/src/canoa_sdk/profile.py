"""The `profile_item` kind: one thing a profile may set, and what it does to a session.

```python
class Region(EnvProfileItem):
    item = "region"
    title = "Region"
    values = RegionValues            # a dataclass: its schema and form are what the item asks for
    variables = {"name": ["REGION"]}
```

A profile holds, for each item, the values a person gave the members of the item's schema. When a session starts the host asks each
item for its effect: environment variables its programs are given, arguments, values of the session's own settings, the network its
programs use and notes for the report. An item changes nothing outside the effect it answers with.

The classes that mirror what Canoa carries are `SettingProfileItem` (one text becomes one of the session's own settings, as the
model, effort, mode and starting instructions items do), `EnvProfileItem` (values become environment variables, as the identity item
does) and `NetworkProfileItem` (values become how the session's programs reach the network, as the proxy item does).
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Any, ClassVar, Dict, List, Mapping, Optional, Sequence

from guatiao import Ref, provide
from guatiao._typed_schema import schema_for
from guatiao.errors import BadValue, Unsupported
from guatiao.kinds import ValueAnswer

from . import kinds
from ._model import Model, present
from ._provider import WithForm, abstract
from .forms import form_of

__all__ = [
    "EnvProfileItem",
    "ItemEffect",
    "NETWORK_MODES",
    "NetworkEffect",
    "NetworkProfileItem",
    "ProfileItem",
    "SettingProfileItem",
    "effect",
    "text_of",
    "values_of",
    "words_of",
]

#: The ways an effect says the session reaches the network.
NETWORK_MODES = ("system", "none", "proxy", "pac", "command")


@dataclass
class NetworkEffect(Model):
    """How the session's programs reach the network: `mode` is one of `NETWORK_MODES`; `url` is the proxy's address (`proxy`) or the PAC
    script's location (`pac`); `command` the program and its arguments (`command`, with `{listen}` for the address it must listen on);
    `bypass` the hosts that go direct, in the `NO_PROXY` form; `user` and `password_variable` the proxy login (the name of a host
    variable holding the password, never the password); `intercept` the `{pattern, identity}` entries whose destinations get a
    client certificate (`system` and `pac` only)."""

    mode: str
    url: Optional[str] = None
    command: List[str] = field(default_factory=list)
    bypass: List[str] = field(default_factory=list)
    user: Optional[str] = None
    password_variable: Optional[str] = None
    intercept: List[Dict[str, str]] = field(default_factory=list)


@dataclass
class ItemEffect(Model):
    """What an item does to one session: `env` (`{name, value}` variables), `args`, `settings` (`{key, value}`), `network` and
    `notes` (lines for the report, which carry no value). Everything is optional."""

    env: List[Dict[str, str]] = field(default_factory=list)
    args: List[str] = field(default_factory=list)
    settings: List[Dict[str, str]] = field(default_factory=list)
    network: Optional[NetworkEffect] = None
    notes: List[str] = field(default_factory=list)


def effect(
    env: Sequence[Mapping[str, str]] = (),
    args: Sequence[str] = (),
    settings: Sequence[Mapping[str, str]] = (),
    network: Optional[NetworkEffect] = None,
    notes: Sequence[str] = (),
) -> Dict[str, Any]:
    """An effect as the plain map the host reads."""
    return ItemEffect(list(map(dict, env)), list(args), list(map(dict, settings)), network, list(notes)).to_data()


def _plain(value: Any) -> Any:
    return value.to_python() if hasattr(value, "to_python") else value


def values_of(request: Ref) -> Dict[str, Any]:
    """The values a profile holds for the item, as a dict keyed by the schema's member names: what `apply` is asked with, parsed."""
    body = _plain(request)
    text = body.get("values") if isinstance(body, dict) else None
    try:
        values = json.loads(text) if isinstance(text, str) else None
    except ValueError:
        values = None
    if not isinstance(values, dict):
        raise BadValue("the values are a JSON object")
    return values


def text_of(values: Mapping[str, Any], name: str) -> Optional[str]:
    """The member `name` as text, trimmed; `None` when it is absent, not text or empty."""
    value = values.get(name)
    if not isinstance(value, str):
        return None
    return value.strip() or None


def words_of(values: Mapping[str, Any], name: str) -> List[str]:
    """The member `name` as a list of the texts it holds, each as written; empty when it is absent."""
    value = values.get(name)
    return [each for each in value if isinstance(each, str)] if isinstance(value, list) else []


class ProfileItem(WithForm, provide.Provider, kinds.ProfileItem, abstract=True):
    """Subclass it with `item` (the id the profile keeps the values under), `title` and, for an item that takes values, `values` (a
    dataclass: Guatiao reads its schema and `canoa_sdk.forms` its form), and implement `apply`. A member the schema marks sensitive is
    never shown in clear. `harnesses` names the standard registry's ids the item applies to; none means every harness.

    `values_schema` and `values_form` give the schema and the guatiao-intake form as plain data instead, when no dataclass says
    them."""

    item: ClassVar[str]
    title: ClassVar[str]
    help: ClassVar[Optional[str]] = None
    harnesses: ClassVar[Sequence[str]] = ()
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
                "id": self.item,
                "title": self.title,
                "help": self.help,
                "harnesses": list(self.harnesses),
                "schema": schema,
                "form": form or None,
            }
        )

    def describe(self) -> ValueAnswer:
        return self.info()

    @abstract
    def apply(self, request: Ref) -> ValueAnswer:
        """What the values do to a session: an effect as a map (`effect(...)`)."""
        raise Unsupported("apply is not implemented")


class SettingProfileItem(ProfileItem, abstract=True):
    """One text becomes one of the session's own settings (`model`, `effort`, `mode`, `context`, or one of `access.sandbox`,
    `access.network`, `access.approvals`): the member `member` (default: the setting's name) of the values, when it is set."""

    setting: ClassVar[str]
    member: ClassVar[Optional[str]] = None
    note: ClassVar[str] = "sets a value the session starts with"

    def apply(self, request: Ref) -> ValueAnswer:
        value = values_of(request).get(self.member or self.setting)
        if not isinstance(value, str):
            return effect()
        return effect(settings=[{"key": self.setting, "value": value}], notes=[self.note])


class EnvProfileItem(ProfileItem, abstract=True):
    """Values become environment variables. `variables` says which: each member of the values that holds text sets the variables it
    lists (`{"name": ["GIT_AUTHOR_NAME", "GIT_COMMITTER_NAME"]}`), and `lines`, when set, names a list member of `NAME=value` lines
    that set whatever they name."""

    variables: ClassVar[Mapping[str, Sequence[str]]] = {}
    lines: ClassVar[Optional[str]] = None
    line_label: ClassVar[str] = "variable"
    note: ClassVar[str] = "gives its programs environment variables"

    def apply(self, request: Ref) -> ValueAnswer:
        values = values_of(request)
        env: List[Dict[str, str]] = []
        for member, names in self.variables.items():
            text = text_of(values, member)
            if text is not None:
                env.extend({"name": name, "value": text} for name in names)
        if self.lines is not None:
            for line in words_of(values, self.lines):
                name, equals, value = line.partition("=")
                if not equals:
                    raise BadValue(f"each {self.line_label} is written NAME=value")
                env.append({"name": name.strip(), "value": value})
        return effect(env=env, notes=[self.note] if env else [])


_NETWORK_NOTES = {
    "none": "gives its programs no proxy",
    "proxy": "gives its programs the proxy it names and uses it for the host's own requests for the session",
    "pac": "gives its programs a proxy of the host's that chooses by the PAC script",
    "command": "runs the program it names and gives it to its programs as their proxy",
    "system": "gives its programs a proxy of the host's that follows the system",
}


def _certificate_hosts(values: Mapping[str, Any], name: str) -> List[Dict[str, str]]:
    entries = values.get(name)
    if entries is None:
        return []
    wrong = BadValue(f"each entry of {name} has a pattern and a certificate")
    if not isinstance(entries, list):
        raise wrong
    found: List[Dict[str, str]] = []
    for entry in entries:
        if not isinstance(entry, dict) or not isinstance(entry.get("pattern"), str):
            raise wrong
        identity = entry.get("identity")
        if identity is not None and not isinstance(identity, str):
            raise wrong
        found.append({"pattern": entry["pattern"], "identity": identity if identity is not None else "system"})
    return found


class NetworkProfileItem(ProfileItem, abstract=True):
    """Values become how the session's programs reach the network. The members are the proxy item's: `mode` (one of
    `NETWORK_MODES`, `system` when absent), `url`, `command`, `bypass`, `user`, `password_variable` and `client_certificate_hosts`
    (entries `{pattern, identity}`). The proxy itself is the host's: this only says which network the values ask for, which the host
    checks and runs."""

    def apply(self, request: Ref) -> ValueAnswer:
        values = values_of(request)
        mode = text_of(values, "mode") or "system"
        if mode not in NETWORK_MODES:
            raise BadValue(f"the network mode {json.dumps(mode)} is none of {json.dumps(list(NETWORK_MODES))}")
        network = NetworkEffect(
            mode=mode,
            url=text_of(values, "url"),
            command=words_of(values, "command"),
            bypass=words_of(values, "bypass"),
            user=text_of(values, "user"),
            password_variable=text_of(values, "password_variable"),
            intercept=_certificate_hosts(values, "client_certificate_hosts"),
        )
        if mode in ("proxy", "pac") and not network.url:
            raise BadValue(f"the network mode {mode} needs an address")
        if mode == "command" and not (network.command and network.command[0]):
            raise BadValue("the network mode command needs a program")
        notes = [_NETWORK_NOTES[mode]]
        ended = ", ".join(entry["pattern"].strip() for entry in network.intercept)
        if ended:
            notes.append(f"ends TLS for {ended} to present your client certificate to them")
        return effect(network=network, notes=notes)
