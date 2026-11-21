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
    program = vendor = aos = model_variable = tool_name = skills_folder = mark = look = None
    standards: Sequence[StandardTitle] = (); names = markers = (); modes: Sequence[ModeTitle] = ()
    def info(self) -> VendorInfo                             # override to compute the description
class StoreExtension:     # canoa_sdk.store: adds types to the Store
    title: str; publisher: str; types: Sequence[ExtensionType] = ()
    def info(self) -> ExtensionInfo
class ProgramStore:       # canoa_sdk.store: knows one program; abstract slots program, latest, plan, apply
class ModelStore:         # canoa_sdk.store: lists, moves and sets up the models of a type's programs; info() (the ModelStoreInfo), abstract slots list, act, keep, service (each takes the host's spawner)
    def info(self) -> dict                                   # the StoreInfo
class SpeechProvider:     # canoa_sdk.speech: audio in, text out; abstract slot open(request, spawner) -> SpeechSession
    title: str; encodings: Sequence[str]; description = max_seconds = None; partials = False; hints = ()
class SpeechOutput:       # canoa_sdk.speech: text in, audio out; abstract slots voices, say; info() -> dict (the OutputInfo)
class AccountSpeech:      # canoa_sdk.speech: the speech-to-text services the accounts of some kinds can call; services: Sequence[AccountSpeechService] (id, title, standard, address, url, credential, summary, location, models: [AccountSpeechModel(id, label)])
class SecretProvider:     # canoa_sdk.secret: somewhere a secret comes from; abstract slot get(request, spawner); names, put, forget refuse UNSUPPORTED
    title: str; description = None; can_store = can_list = needs_person = False
class UsageReader:        # canoa_sdk.usage: how an agent's own quota is read; reader: str, bucket_prefix: str, settle_ms = 10000, retries_ms = (), cache_ms = 15000, private_limit_ms = 40000, private_retries_ms = (); abstract slot open(members) -> UsageReading
class UsageReading:       # canoa_sdk.usage: __init__(members); opened(result); abstract-ish read(root, tree) -> reading([...]) or failed(why, retry); processes(root) -> list[int]
bucket(id, *, label, models, unit, used, limit, remaining, resets_at, status, observed_at, evidence) -> dict; reading(buckets); failed(why, retry=Retry.NEVER); Retry.NEVER|NOT_YET|NO_ANSWER
class FormSecretProvider(SecretProvider)         # the person's form: needs_person, holds nothing
class SystemStoreSecretProvider(SecretProvider)  # a store of keys and texts you supply (abstract read, write, remove), with an index of names
name_of(request) -> str; keeping_of(request) -> (name, value)   # a request's name and value, checked as the kind says
class ProfileItem:        # canoa_sdk.profile: one thing a profile sets; abstract slot apply(request) -> an effect (effect(...))
    item: str; title: str; help = None; harnesses = ()        # item = the id the profile keeps its values under
    values = None         # a dataclass: its schema (Guatiao) and form (canoa_sdk.forms); values_schema / values_form give plain data instead
class SettingProfileItem(ProfileItem)   # one text becomes a session setting: setting, member = None, note
class EnvProfileItem(ProfileItem)       # values become variables: variables = {member: [NAMES]}, lines = None, line_label, note
class NetworkProfileItem(ProfileItem)   # values become a network effect, the proxy item's members (the proxy itself is the host's)
values_of(request); text_of(values, name); words_of(values, name); effect(env=, args=, settings=, network=, notes=); NETWORK_MODES
class CatalogSource:      # canoa_sdk.source: where a Store type's catalog comes from; abstract slot read(request, spawner) -> the catalog's text
    source: str; title: str; help = None; types = ()          # source = the id canoa.cfg records, and the provider's id
    values = None         # a dataclass, as above
class CommandCatalogSource(CatalogSource)   # a program the person named; its output (or `file`) is the catalog
class ReadCatalogSource(CatalogSource)      # a place the type reads itself; read answers a file on this computer
fields_of(request); MAX_BYTES; DEFAULT_SECONDS; TAIL_BYTES
class SettingsSection:    # canoa_sdk.settings: section, title, help, icon, group, under, keywords, stored = "config" | "provider", values; read(), write(values)
class PanelTab:           # canoa_sdk.panel: tab, title, help, icon, shows = "log" | "terminal" | "page", shell, url; tail(request)
class TerminalPanelTab(PanelTab); class PagePanelTab(PanelTab)   # a page at `url`, on this computer's loopback
class LogPanelTab(PanelTab)   # lines() -> the log's lines, oldest first
class AccountProvider:    # canoa_sdk.account: title, description, kinds_offered ([AccountKindInfo]), logins ([LoginMethodInfo]), usage_kinds, local; offer(), login/renew/watch refuse UNSUPPORTED, usage/models answer unsupported/unknown (and release handles)
class AccountWatch        # canoa_sdk.account: stop()
credential_schema(title?, description?); models_listed(models); models_unknown(reason, unknown_kind?); usage_unsupported(reason, account?); renewed(credentials, expires_at?); renew_unchanged(reason); release(*handles)
class ModelGateway:       # canoa_sdk.gateway: kind, vendor, table?; abstract slot open(request, spawner) -> GatewayLease
class GatewayLease        # canoa_sdk.gateway: address() -> str, bearer() -> str, models() -> ModelListing (defaults to unknown)
request_of(request) -> dict
```

A catalog source's `read` is handed the host's `spawner`: `spawner.spawn(spec)` starts a program as the person, `spawner.read_file(path, max)`
reads a file anywhere the person's account can (an absolute path); the source destroys it. A class that names its `source` is the provider
of that id (the host finds a source by the id it is described under); `item`, `section` and `tab` are the ids the profile, Settings and the
panel keep, and the provider's own id stays Guatiao's (`<module>_<class in snake case>`).

Models (`VendorInfo`, `StandardTitle`, `ModeTitle`, `ExtensionInfo`, `ExtensionType`, `SpeechInfo`, `ProviderInfo`, `Held`, `ItemEffect`, `NetworkEffect`, `ReaderInfo`) are dataclasses with `to_data()`: the
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
