# canoa-sdk (JavaScript) — API header

ES-module base classes for the providers Canoa loads, `private` and not published. Node 22 or later, or Guatiao's QuickJS source.
Imports only `guatiao`, `guatiao/kinds`, `guatiao/provide` and its own files by relative path. Every class extends
`guatiao/provide`'s `Provider`, so Guatiao's statics (`named`, `id`, `displayName`, `meta`, `config`, `configSchema`, `default`,
`available()`) work as documented there; a module exports its provider classes and a `*.guatiao` manifest (`type = "js"`, `name`,
`kinds`, `paths`, `main`) names it.

## Kinds

```js
class AgentVendor extends Provider {          // vendor.js
  static agent; static title;                 // required; agent = its acp_harness id, else its source's name
  static program; static vendor; static aos; static model_variable; static tool_name; static skills_folder; static home_variable; static home_folder; static mark; static look;   // optional
  static standards = []; static names = []; static markers = []; static context_files = []; static home_context_files = []; static modes = []; static endpoints = [];   // {id, title} lists; endpoints: the https addresses the program reaches
  info()                                      // override to compute the description
}
class StoreExtension extends Provider {       // store.js
  static title; static publisher; static types = [];   // {id, title, stores?, icon?, models?, catalogs?: {noun, plural, lists, format?, defaults?: [{label, provider, fields?, pin?, snapshot?}]}}
}
class ProgramStore extends Provider {         // store.js: info() (the StoreInfo), program, latest, plan, apply
class ModelStore extends Provider {           // store.js: lists, moves and sets up the models of a type's programs; info() (the ModelStoreInfo), list, act, keep, service (each takes the host's spawner)
}
class SpeechProvider extends Provider {       // speech.js: static title, encodings, description, partials, max_seconds, hints, version, built_for; open(request, spawner)
}
class CommandSpeech extends SpeechProvider    // a program the person names: audio on its input (a WAV stream or the samples), text on its output; id `speech_command`; `finish` and the catalog source's read let the host's other calls through while they wait (`yieldCalls` of wait.js, 0 under Node), so a `cancel` during `finish` ends the recording
wavHeader(format); fill(arg, values); languageCode(tag); MAX_OUTPUT; PCM_S16LE
class SpeechOutput extends Provider {         // speech.js: info() (the OutputInfo), voices, say
class AccountSpeech extends Provider {        // speech.js: static services: [{id, title, standard, address, url, credential, summary?, location?, models?: [{id, label}]}], the speech-to-text services the accounts of some kinds can call
}
class SecretProvider extends Provider {       // secret.js: static title, description, can_store, can_list, needs_person; get(request, spawner); names, put, forget refuse with Unsupported
}
class UsageReader extends Provider {          // usage.js: static reader (the id an entry's `usage` names), bucket_prefix; settle_ms = 10000, retries_ms = [], cache_ms = 15000, private_limit_ms = 40000, private_retries_ms = [], stored = false; open(members) answers a UsageReading
}
class UsageReading {                          // usage.js: not a provider: constructor(members); opened(result); read(root, tree) answers reading([...]) or failed(why, retry); processes(root); destroys members when the host lets it go
}
bucket(id, {label, group, models, model_names, unit, used, limit, remaining, resets_at, status, observed_at, evidence}); reading(buckets); failed(why, retry = Retry.NEVER); Retry.{NEVER, NOT_YET, NO_ANSWER}
class FormSecretProvider extends SecretProvider            // the person's form: needs_person, holds nothing
class SystemStoreSecretProvider extends SecretProvider     // a store of keys and texts you supply (read, write, remove), with an index of names
nameOf(request); keepingOf(request); release(spawner)      // a request's name and value, checked as the kind says; lets the host's spawner go
class ProfileItem extends Provider {          // profile.js: one thing a profile sets; apply(request) -> an effect
  static item; static title; static help; static harnesses = [];   // item = the id the profile keeps its values under
  static values; static form;                 // the values' guatiao schema and intake form, plain data
}
class SettingProfileItem extends ProfileItem  // one text becomes a session setting: static setting, member?, note
class EnvProfileItem extends ProfileItem      // values become variables: static variables = {member: [NAMES]}, lines?, lineLabel, note
class NetworkProfileItem extends ProfileItem  // values become a route effect, the network item's members: `routes` ({route, hosts}), `bypass`, `client_certificate_hosts` (the routes are the host's)
valuesOf(request); textOf(values, name); wordsOf(values, name); effect({env, args, settings, network, notes})
class CatalogSource extends Provider {        // source.js: where a Store type's catalog comes from; read(request, spawner, fetch) -> the catalog's text
  static source; static title; static help; static types = [];     // source = the id canoa.cfg records; its provider id too
  static values; static form;
}
class CommandCatalogSource extends CatalogSource   // a program the person named; its output (or `file`) is the catalog
class ReadCatalogSource extends CatalogSource      // a place the type reads itself; read answers a file on this computer
class ContentSource extends Provider              // content.js: a file or folder by URI; static source (the id), title, help, schemes = [scheme(name, title, grammar, { lists, resolves })], endpoints
                                                  // stat(target), list(request), open(request), resolve(target): each answers an object (statFound, listFound, openedInline, resolved) or refused(kind, why); a slot left alone is UNSUPPORTED
class CatalogGenerator extends Provider           // content.js: static generator (the id canoa.cfg records), title, types, reads = 'required' | 'optional' | 'absent', grammar, endpoints, keys, values, form
                                                  // generate(request, spawner, host) -> the catalog's text; host reads (stat, list, openText, openJson, openBytes, resolve, child) under the entry's source; release both
fieldsOf(request); releaseSpawner(spawner); MAX_BYTES; DEFAULT_SECONDS; TAIL_BYTES
class SettingsSection extends Provider {      // section.js: static section, title, help, icon, group, under, keywords, stored = 'config'|'provider', values, form
  read()  // {} for a config section; write(values)
}
class PanelTab extends Provider {             // panel.js: static tab, title, help, icon, shows = 'log'|'terminal'|'page', shell, url; tail(request)
}
class TerminalPanelTab extends PanelTab; class PagePanelTab extends PanelTab   // a page at `url`, on this computer's loopback
class LogPanelTab extends PanelTab   // lines() -> the log's lines, oldest first
class FileViewer extends Provider    // viewer.js: static viewer, title, help, mimeTypes, extensions, priority, shows = 'surface'|'page', opensSections; render(request)
class SurfaceViewer extends FileViewer   // surface(file: ViewedFile) -> the A2UI messages, an array of objects
class PageViewer extends FileViewer      // page(file: ViewedFile) -> one self-contained HTML document
class ViewedFile                         // name, size, data (Uint8Array the host read), mime, truncated, section; text(); never a path
class TelemetryExporter extends Provider {    // telemetry.js: somewhere the host's traces, metrics and events go; static title, help, signals = ['spans','metrics','logs']; export(batch) (throw Gone to be tried again, BadValue to have the batch dropped); flush() does nothing unless overridden
}
class AccountProvider extends Provider {      // account.js: static title, description, offeredKinds, logins, usageKinds, local; offer(), provides(account) (the `provides` of the kind it names; override when settings pick the standards), login/renew/watch throw Unsupported, usage/models answer unsupported/unknown (and release handles); signedIn(query) says whether the program's own login exists (`loginPresent`, `loginAbsent`, `loginUnknown`): Unsupported without `local`, unknown with it, until overridden
}
class AccountWatch   // account.js: stop()
credentialSchema(title?, description?); accountKind(fields); loginMethod(fields); endpointSlot(fields); loginOutcome(fields); modelsListed(models, signedOut?) (a model: id, name, ..., capabilities, capabilities_from, limits); modelsUnknown(reason, unknownKind?); loginPresent(variable?); loginAbsent(reason); loginUnknown(reason); usageUnsupported(reason, account?); renewed(credentials, expiresAt?); renewUnchanged(reason); release(...handles)
class ConfigStore extends Provider {           // config.js: where the host's configuration lives; describe, read, get, change, revision throw Unsupported until implemented
class Environment extends Provider {          // environment.js: a place an agent works; describe, spawner; readFile, writeFile, listDir, call throw Unsupported until overridden
class SdkProvider extends Provider {          // sdk.js: a vendor's client for an account type; describe, open(request, http) -> Sdk
class Sdk                                     // sdk.js: the opened client; models, chat, embed, rerank throw Unsupported until overridden
class ModelGateway extends Provider {         // gateway.js: static kind, vendor, table?; open(request, spawner) -> GatewayLease
}
class GatewayLease   // gateway.js: address(), bearer(), models() (defaults to modelsUnknown)
```

A catalog source's `read` is handed the host's `spawner`: `spawner.spawn(spec)` starts a program as the person, `spawner.readFile(path, max)`
reads a file anywhere the person's account can (an absolute path). A handle's methods are the slot names in camel case. The provider's id
is its `source` (the host finds a source by the id it is described under); `static item`, `section` and `tab` are the ids the profile,
Settings and the panel keep, and the provider's own id stays Guatiao's (`<library>_<class>`).

`describe()` is answered from the statics (`info()`), an absent field left out. A slot a base leaves in place (`open`, `voices`, `say`,
`program`, ...) refuses with `Unsupported` when called; JavaScript has no class-creation check, so a subclass that forgets one is found
by its first call. `kinds.js` (and `kinds.d.ts`) are the kinds written from `crates/canoa/include/canoa.*.kinds.json` by
`tools/write_kinds.mjs` (`--check` says whether they are current); the host's objects (`kinds.ProcessSpawner`, `kinds.InstallTools`)
are the handles a slot receives.

## Forms (`forms.js`)

`section(id, title, description?)`, `ui(sectionId)` (the `x-section` schema keyword, spread into an `s` builder's keywords),
`when(field, equals)` and `formMeta({ sections, fields })` (the `meta` entry `{ form }`, for `static meta`). The schema is
`static config = s.object({...})` from `guatiao/provide`. There is no field type of Canoa's own.
- `SdkProvider`, `Sdk` and `LentHttp` in `kinds.js` (generated from `canoa.sdk_provider.kinds.json`): a vendor's client for an account type, as in the Python SDK's header. No hand-written base class yet.
