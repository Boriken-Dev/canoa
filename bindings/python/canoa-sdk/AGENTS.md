# canoa-sdk — API header

Python base classes for the providers Canoa loads. Python 3.9 or later; depends on Guatiao's Python binding (`guatiao`), which a
guatiao source puts on `sys.path`. Not published. Names are `canoa_sdk.<module>.<name>`, all re-exported from `canoa_sdk` except
`canoa_sdk.acp` and `canoa_sdk.kinds`.

## A provider

A provider is a class in a module that lists it in `__all__`; the module is the library (its `__version__` is the version), and a
`*.guatiao` manifest (`type = "python"`, `module`, `kinds`, `paths`) names both. The provider's id is `<module>_<class in snake case>`;
Guatiao's class keywords (`named`, `id`, `display_name`, `meta`, ...) work as they do on `guatiao.provide.Provider`.

Every base class takes two more keywords: `form=<dataclass>` (the form of the provider's settings, below) and `abstract=True` (a base
for other classes: it may leave `@abstract` slots in place; a class that does not say it leaves one is a `TypeError` naming it).

## Kinds

```python
class AgentVendor:        # canoa_sdk.vendor: who an agent is and how Canoa shows it
    agent: str; title: str                                   # required; agent = its acp_harness id, else its source's name
    program = vendor = aos = model_variable = mark = look = None
    standards: Sequence[StandardTitle] = (); names = markers = (); modes: Sequence[ModeTitle] = ()
    def info(self) -> VendorInfo                             # override to compute the description
class StoreExtension:     # canoa_sdk.store: adds types to the Store
    title: str; publisher: str; types: Sequence[ExtensionType] = ()
    def info(self) -> ExtensionInfo
class ProgramStore:       # canoa_sdk.store: knows one program; abstract slots program, latest, plan, apply
    def info(self) -> dict                                   # the StoreInfo
class SpeechProvider:     # canoa_sdk.speech: audio in, text out; abstract slot open(request, spawner) -> SpeechSession
    title: str; encodings: Sequence[str]; description = max_seconds = None; partials = False; hints = ()
class SpeechOutput:       # canoa_sdk.speech: text in, audio out; abstract slots voices, say; info() -> dict (the OutputInfo)
```

Models (`VendorInfo`, `StandardTitle`, `ModeTitle`, `ExtensionInfo`, `ExtensionType`, `SpeechInfo`) are dataclasses with `to_data()`: the
plain map the host reads, an absent (`None`) field left out. The host checks what a provider answers and says why it refuses it in
its log; the SDK checks only what a missing attribute makes impossible.

`SpeechSession`, `SpeechUtterance` and the kinds' other object classes (`kinds.ProcessSpawner`, `kinds.InstallTools`, ...) are the
protocol classes a slot's objects are, written from `crates/canoa/include/canoa.*.kinds.json` into `canoa_sdk/kinds.py` by
`tools/write_kinds.py` (`--check` says whether it is current). `canoa_sdk.acp` re-exports coqui-acp's `AcpHarness`, `AccountProvider`,
`AccountKeeper`, `AccountWatch`, `AgentProvider`, `AgentSession`, `Channel`, `ChannelSink` and `LoginPrompter`; it needs coqui-acp's `python` folders on `sys.path` and says so when
they are not.

## Forms (`canoa_sdk.forms`)

The settings are a dataclass (what Guatiao reads a schema from). `ui(section=, placeholder=, unit=, visible_when=(field, value))` is
metadata for `Annotated[T, ui(...)]`; `section(id, title, description=None)` names a section (a dataclass may list them in a
`__sections__` tuple); `form_of(dataclass, sections=())` is the guatiao-intake form document, `form_meta(dataclass)` the `meta` entry
(`{"form": ...}`), and a provider class given `form=` carries it. There is no field type of Canoa's own.

## Gotchas

- A Python library shares one interpreter with every other: two libraries needing different versions of a package need separate hosts.
- A provider that starts a program does it through the `spawner` its slot is handed, and destroys the spawner it was handed.
- `Provider` classes must come before kind protocols in the bases; the SDK's already do, so subclass the SDK's.
