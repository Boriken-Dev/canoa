"""Write a Canoa provider in Python: subclass a kind's base class, set a few attributes or override a few methods, and list the class
in the module's `__all__`; a guatiao source loads the module like any other, and the host offers the provider.

Each kind lives in its own module (`canoa_sdk.vendor`, `.store`, `.speech`, `.secret`, `.profile`, `.source`, `.settings`, `.panel`, `.usage`); `canoa_sdk.acp` re-exports coqui-acp's. A settings form
is written on a dataclass with `canoa_sdk.forms`. `canoa_sdk.kinds` holds the typed `Protocol` classes written from the kinds'
descriptions, which the base classes sit on.
"""

from __future__ import annotations

from .forms import Section, form_meta, form_of, section, ui
from .panel import LogPanelTab, PagePanelTab, PanelTab, TerminalPanelTab
from .profile import (
    EnvProfileItem,
    ItemEffect,
    NetworkEffect,
    NetworkProfileItem,
    ProfileItem,
    SettingProfileItem,
    effect,
    text_of,
    values_of,
    words_of,
)
from .secret import (
    FormSecretProvider,
    Held,
    ProviderInfo,
    SecretProvider,
    SystemStoreSecretProvider,
    keeping_of,
    name_of,
)
from .settings import SettingsSection
from .source import CatalogSource, CommandCatalogSource, ReadCatalogSource, fields_of
from .speech import (
    AccountSpeech,
    AccountSpeechModel,
    AccountSpeechService,
    SpeechInfo,
    SpeechOutput,
    SpeechProvider,
    SpeechSession,
    SpeechUtterance,
)
from .store import ExtensionInfo, ExtensionType, ModelStore, ProgramStore, StoreExtension
from .usage import ReaderInfo, Retry, UsageReader, UsageReading, bucket, failed, reading
from .vendor import AgentVendor, ModeTitle, StandardTitle, VendorInfo

__all__ = [
    "AgentVendor",
    "CatalogSource",
    "CommandCatalogSource",
    "EnvProfileItem",
    "ExtensionInfo",
    "ExtensionType",
    "FormSecretProvider",
    "Held",
    "ItemEffect",
    "LogPanelTab",
    "ModeTitle",
    "NetworkEffect",
    "NetworkProfileItem",
    "PagePanelTab",
    "PanelTab",
    "ProfileItem",
    "ProgramStore",
    "ProviderInfo",
    "ReadCatalogSource",
    "ReaderInfo",
    "Retry",
    "SecretProvider",
    "Section",
    "SettingProfileItem",
    "SettingsSection",
    "SpeechInfo",
    "SpeechOutput",
    "SpeechProvider",
    "SpeechSession",
    "SpeechUtterance",
    "StandardTitle",
    "StoreExtension",
    "SystemStoreSecretProvider",
    "TerminalPanelTab",
    "UsageReader",
    "UsageReading",
    "VendorInfo",
    "bucket",
    "effect",
    "failed",
    "fields_of",
    "form_meta",
    "form_of",
    "keeping_of",
    "name_of",
    "reading",
    "section",
    "text_of",
    "ui",
    "values_of",
    "words_of",
]
