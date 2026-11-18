"""The speech kinds: `speech_provider` (audio in, text out) and `speech_output` (text in, audio out).

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

__all__ = ["SpeechInfo", "SpeechOutput", "SpeechProvider", "SpeechSession", "SpeechUtterance"]

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


class SpeechProvider(WithForm, provide.Provider, kinds.SpeechProvider, abstract=True):
    """Subclass it with `title` and `encodings` (and the optional attributes of `SpeechInfo`) and implement `open`."""

    title: ClassVar[str]
    encodings: ClassVar[Sequence[str]]
    description: ClassVar[Optional[str]] = None
    partials: ClassVar[bool] = False
    max_seconds: ClassVar[Optional[int]] = None
    hints: ClassVar[Sequence[str]] = ()

    def info(self) -> SpeechInfo:
        """The description, from the class attributes."""
        return SpeechInfo(
            self.title,
            list(self.encodings),
            self.description,
            self.partials,
            self.max_seconds,
            list(self.hints),
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
    def voices(self, query: Ref, spawner: Taken[kinds.ProcessSpawner]) -> ValueAnswer:
        raise Unsupported("voices is not implemented")

    @abstract
    def say(self, request: Ref, spawner: Taken[kinds.ProcessSpawner]) -> kinds.SpeechUtterance:
        raise Unsupported("say is not implemented")
