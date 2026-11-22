"""An account maker written in Python: a gateway in front of models, as a library an extension of canoa.cfg offers.

The library is no agent, so the host offers its kind to every program that accepts a standard the kind provides: the kind here
provides the OpenAI API, so Codex lists it, signs an account in through it and runs on the account, with no change to Canoa. An
account is an address and a key the person pastes; nothing is asked of the address.
"""

from __future__ import annotations

from typing import Any, Dict, List

from canoa_sdk import (
    AccountKindInfo,
    AccountProvider,
    EndpointSlot,
    LoginForm,
    LoginMethodInfo,
    LoginOutcome,
    ModelInfo,
    credential_schema,
    models_listed,
    release,
)
from guatiao import Ref
from guatiao.errors import BadValue

__version__ = "1.0.0"
__all__ = ["Accounts"]

KIND = "standin-gateway"
METHOD = "standin-key"
KEY = "api_key"
ADDRESS = "base_url"
STANDARD = "openai-api"


class Accounts(AccountProvider, display_name="Stand-in gateway accounts", default=True):
    """Makes the accounts of the stand-in gateway."""

    title = "Stand-in gateway"
    description = "An account maker an extension offers."
    kinds_offered = [
        AccountKindInfo(
            id=KIND,
            title="Stand-in gateway",
            description="An address and a key, for a gateway that speaks the OpenAI API.",
            settings={
                "type": "object",
                "properties": {ADDRESS: {"type": "string", "format": "uri"}, KEY: credential_schema()},
                "required": [ADDRESS, KEY],
                "additionalProperties": False,
            },
            endpoints=[EndpointSlot(purpose="model_api", key=ADDRESS)],
            history_portable=True,
            provides=[STANDARD],
        )
    ]
    logins = [
        LoginMethodInfo(
            id=METHOD,
            title="An address and a key",
            produces=KIND,
            inputs={
                "type": "object",
                "properties": {ADDRESS: {"type": "string", "format": "uri"}},
                "required": [ADDRESS],
            },
            steps=["ask"],
        )
    ]

    def provides(self, account: Ref) -> List[str]:
        asked = account.to_python()
        if asked.get("kind") != KIND:
            raise BadValue("the stand-in gateway keeps no account of that kind")
        return [STANDARD]

    def login(self, request: Ref, spawner: Any, prompter: Any) -> Dict[str, Any]:
        release(spawner)
        try:
            asked = request.to_python()
            if asked.get("method") != METHOD:
                raise BadValue("the stand-in gateway has no such way to sign in")
            address = (asked.get("inputs") or {}).get(ADDRESS)
            if not isinstance(address, str) or not address:
                raise BadValue("the address of the gateway is needed")
            form = LoginForm(
                title="Enter the key",
                schema={"type": "object", "properties": {KEY: credential_schema()}, "required": [KEY]},
            ).to_data()
            answer = prompter.ask(form)
            answer = answer.to_python() if hasattr(answer, "to_python") else answer
            key = (answer.get("secrets") or {}).get(KEY, "").strip()
            if not key:
                raise BadValue("login refused: nothing was entered")
        finally:
            release(prompter)
        return LoginOutcome(
            kind=KIND,
            settings={ADDRESS: address.rstrip("/")},
            credentials={KEY: key},
            label="stand-in",
        ).to_data()

    def models(self, query: Ref, spawner: Any, keeper: Any) -> Dict[str, Any]:
        release(spawner, keeper)
        return models_listed([ModelInfo(id="standin-model")])
