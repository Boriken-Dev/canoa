"""An account maker written in Python: a gateway in front of models, as a library an extension of canoa.cfg offers.

The library is no agent, so the host offers its kind to every program that accepts a standard the kind provides: the kind here
provides the OpenAI API, so Codex lists it, signs an account in through it and runs on the account, with no change to Canoa. An
account is an address and a key the person pastes; nothing is asked of the address.
"""

from __future__ import annotations

from typing import Any, Dict

from coqui_acp_api_kinds import AccountKeeper, AccountProvider, LoginPrompter, ProcessSpawner

from guatiao import Ref, provide
from guatiao.errors import BadValue, Unsupported

__version__ = "1.0.0"
__all__ = ["Accounts"]

KIND = "standin-gateway"
METHOD = "standin-key"
KEY = "api_key"
ADDRESS = "base_url"
STANDARD = "openai-api"


def _credential() -> Dict[str, Any]:
    return {"type": "string", "writeOnly": True, "x-sensitive": True}


def _offer() -> Dict[str, Any]:
    address = {"type": "string", "format": "uri"}
    kind = {
        "id": KIND,
        "title": "Stand-in gateway",
        "description": "An address and a key, for a gateway that speaks the OpenAI API.",
        "settings": {
            "type": "object",
            "properties": {ADDRESS: address, KEY: _credential()},
            "required": [ADDRESS, KEY],
            "additionalProperties": False,
        },
        "endpoints": [{"key": ADDRESS, "purpose": "model_api"}],
        "rotates": False,
        "history_portable": True,
        "state_dir": False,
        "renews": False,
        "notices": [],
        "provides": [STANDARD],
    }
    method = {
        "id": METHOD,
        "title": "An address and a key",
        "description": "",
        "produces": KIND,
        "inputs": {"type": "object", "properties": {ADDRESS: address}, "required": [ADDRESS]},
        "steps": ["ask"],
        "remote": False,
    }
    return {"kinds": [kind], "logins": [method], "usage": [], "local": False}


class Accounts(provide.Provider, AccountProvider, display_name="Stand-in gateway accounts", default=True):
    """Makes the accounts of the stand-in gateway."""

    def describe(self) -> Dict[str, Any]:
        return {"title": "Stand-in gateway", "description": "An account maker an extension offers."}

    def offers(self) -> Dict[str, Any]:
        return _offer()

    def login(self, request: Ref, spawner: ProcessSpawner, prompter: LoginPrompter) -> Dict[str, Any]:
        spawner.destroy()
        try:
            asked = request.to_python()
            if asked.get("method") != METHOD:
                raise BadValue("the stand-in gateway has no such way to sign in")
            address = (asked.get("inputs") or {}).get(ADDRESS)
            if not isinstance(address, str) or not address:
                raise BadValue("the address of the gateway is needed")
            form = {
                "title": "Enter the key",
                "schema": {"type": "object", "properties": {KEY: _credential()}, "required": [KEY]},
            }
            answer = prompter.ask(form)
            answer = answer.to_python() if hasattr(answer, "to_python") else answer
            key = (answer.get("secrets") or {}).get(KEY, "").strip()
            if not key:
                raise BadValue("login refused: nothing was entered")
        finally:
            prompter.destroy()
        return {
            "kind": KIND,
            "settings": {ADDRESS: address.rstrip("/")},
            "credentials": {KEY: key},
            "label": "stand-in",
            "checked": False,
        }

    def renew(self, request: Ref, spawner: ProcessSpawner) -> Dict[str, Any]:
        spawner.destroy()
        raise Unsupported("a key you were given is not renewed here")

    def watch(self, request: Ref, spawner: ProcessSpawner, keeper: AccountKeeper) -> Any:
        spawner.destroy()
        keeper.destroy()
        raise Unsupported("a gateway key has no login to watch")

    def usage(self, query: Ref, spawner: ProcessSpawner, keeper: AccountKeeper) -> Dict[str, Any]:
        spawner.destroy()
        keeper.destroy()
        account = query.to_python()["account"]["id"]
        return {
            "supported": False,
            "unsupported_because": "a stand-in gateway reports no plan limits",
            "sources": [],
            "account": account,
            "revision": 0,
            "observed_at": 0,
            "buckets": [],
        }

    def models(self, query: Ref, spawner: ProcessSpawner, keeper: AccountKeeper) -> Dict[str, Any]:
        spawner.destroy()
        keeper.destroy()
        return {"known": True, "models": [{"id": "standin-model"}]}
