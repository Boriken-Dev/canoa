"""Write a Canoa provider in Python: subclass a kind's base class, set a few attributes or override a few methods, and list the class
in the module's `__all__`; a guatiao source loads the module like any other, and the host offers the provider.

Each kind lives in its own module (`canoa_sdk.vendor`, `.store`, `.speech`); `canoa_sdk.acp` re-exports coqui-acp's. A settings form
is written on a dataclass with `canoa_sdk.forms`. `canoa_sdk.kinds` holds the typed `Protocol` classes written from the kinds'
descriptions, which the base classes sit on.
"""

from __future__ import annotations

from .forms import Section, form_meta, form_of, section, ui
from .speech import SpeechInfo, SpeechOutput, SpeechProvider, SpeechSession, SpeechUtterance
from .store import ExtensionInfo, ExtensionType, ProgramStore, StoreExtension
from .vendor import AgentVendor, ModeTitle, StandardTitle, VendorInfo

__all__ = [
    "AgentVendor",
    "ExtensionInfo",
    "ExtensionType",
    "ModeTitle",
    "ProgramStore",
    "Section",
    "SpeechInfo",
    "SpeechOutput",
    "SpeechProvider",
    "SpeechSession",
    "SpeechUtterance",
    "StandardTitle",
    "StoreExtension",
    "VendorInfo",
    "form_meta",
    "form_of",
    "section",
    "ui",
]
