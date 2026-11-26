"""The speech kinds: `speech_provider` (audio in, text out), `speech_output` (text in, audio out) and `account_speech` (the
speech-to-text services the accounts of some kinds can call).

A provider's own settings are its `__init__` dataclass, with a form from `form=` (see `canoa_sdk.forms`); a provider that starts a
program or opens a connection does it through the `spawner` the host hands `open` or `say`, never by itself.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, ClassVar, Dict, List, Optional, Sequence

from guatiao import Ref, provide
from guatiao.errors import Unsupported
from guatiao.kinds import Taken, ValueAnswer

from . import kinds
from ._model import Model
from ._provider import WithForm, abstract

__all__ = [
    "AccountSpeech",
    "AccountSpeechModel",
    "AccountSpeechService",
    "SpeechInfo",
    "SpeechOutput",
    "SpeechProvider",
    "SpeechSession",
    "SpeechUtterance",
]

#: The object a recording runs on: `push(audio)`, `finish()` and `cancel()`; subclass it and answer from `SpeechProvider.open`.
SpeechSession = kinds.SpeechSession
#: The object a spoken text plays on: `format()`, `read(dst, timeout_ms)` and `stop()`; answered from `SpeechOutput.say`.
SpeechUtterance = kinds.SpeechUtterance


@dataclass
class SpeechInfo(Model):
    """What a speech-to-text provider can do: its title, the audio encodings it takes (`pcm_s16le`, `opus`), whether it answers
    partial text, the longest recording it takes and which hints (`language`, `vocabulary`, `profile`) it uses."""

    title: str
    encodings: List[str]
    description: Optional[str] = None
    partials: bool = False
    max_seconds: Optional[int] = None
    hints: List[str] = field(default_factory=list)
    version: Optional[str] = None
    built_for: Optional[str] = None


@dataclass
class AccountSpeechModel(Model):
    """One model a service transcribes with: the name a recording asks for, and what the offer calls it."""

    id: str
    label: str


@dataclass
class AccountSpeechService(Model):
    """A speech-to-text service the accounts of some kind can call: its `id` (lower-case letters and digits; an offer made of an
    account is `ID:ACCOUNT`), its `title`, the credential `standard` an account's kind provides for it, the `address` that kind's
    model API starts from, the `url` a recording is posted to, the `credential` that holds the key, what it is (`summary`), where the
    audio goes (`location`) and its `models`, the recommended one first."""

    id: str
    title: str
    standard: str
    address: str
    url: str
    credential: str
    summary: str = ""
    location: str = ""
    models: List[AccountSpeechModel] = field(default_factory=list)


class AccountSpeech(WithForm, provide.Provider, kinds.AccountSpeech):
    """Subclass it with `services`, or override `info()`: the host offers each of the person's accounts whose kind a service names and
    lends its key for a recording, so the host itself names no service."""

    services: ClassVar[Sequence[AccountSpeechService]] = ()

    def info(self) -> Dict[str, Any]:
        """The `AccountSpeechInfo` as a map, from the class attribute."""
        return {"services": [service.to_data() for service in self.services]}

    def describe(self) -> ValueAnswer:
        return self.info()


class SpeechProvider(WithForm, provide.Provider, kinds.SpeechProvider, abstract=True):
    """Subclass it with `title` and `encodings` (and the optional attributes of `SpeechInfo`) and implement `open`."""

    title: ClassVar[str]
    encodings: ClassVar[Sequence[str]]
    description: ClassVar[Optional[str]] = None
    partials: ClassVar[bool] = False
    max_seconds: ClassVar[Optional[int]] = None
    hints: ClassVar[Sequence[str]] = ()
    version: ClassVar[Optional[str]] = None
    built_for: ClassVar[Optional[str]] = None

    def info(self) -> SpeechInfo:
        """The description, from the class attributes."""
        return SpeechInfo(
            self.title,
            list(self.encodings),
            self.description,
            self.partials,
            self.max_seconds,
            list(self.hints),
            self.version,
            self.built_for,
        )

    def describe(self) -> ValueAnswer:
        return self.info().to_data()

    @abstract
    def open(self, request: Ref, spawner: Taken[kinds.ProcessSpawner]) -> kinds.SpeechSession:
        """Starts a recording and answers its `SpeechSession`. `spawner` is the host's, destroyed by the provider; a provider
        that starts no program just drops it."""
        raise Unsupported("open is not implemented")


class SpeechOutput(WithForm, provide.Provider, kinds.SpeechOutput, abstract=True):
    """A provider of text to speech. Override `info()` (the `OutputInfo` as a map), `voices` and `say`, as
    `canoa.speech_output.kinds.json` describes them."""

    def info(self) -> Dict[str, Any]:
        """The `OutputInfo` this provider answers `describe` with."""
        raise NotImplementedError("a SpeechOutput says its info()")

    def describe(self) -> ValueAnswer:
        return self.info()

    @abstract
    def voices(self, query: Ref, spawner: Taken[kinds.ProcessSpawner], fetch: Taken[kinds.SourceFetch]) -> ValueAnswer:
        raise Unsupported("voices is not implemented")

    @abstract
    def say(self, request: Ref, spawner: Taken[kinds.ProcessSpawner]) -> kinds.SpeechUtterance:
        raise Unsupported("say is not implemented")
