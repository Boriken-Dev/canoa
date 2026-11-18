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
  static program; static vendor; static aos; static model_variable; static mark; static look;   // optional
  static standards = []; static names = []; static markers = []; static modes = [];             // {id, title} lists
  info()                                      // override to compute the description
}
class StoreExtension extends Provider {       // store.js
  static title; static publisher; static types = [];   // {id, title, stores?, icon?}
}
class ProgramStore extends Provider {         // store.js: info() (the StoreInfo), program, latest, plan, apply
}
class SpeechProvider extends Provider {       // speech.js: static title, encodings, description, partials, max_seconds, hints; open(request, spawner)
}
class SpeechOutput extends Provider {         // speech.js: info() (the OutputInfo), voices, say
}
```

`describe()` is answered from the statics (`info()`), an absent field left out. A slot a base leaves in place (`open`, `voices`, `say`,
`program`, ...) refuses with `Unsupported` when called; JavaScript has no class-creation check, so a subclass that forgets one is found
by its first call. `kinds.js` (and `kinds.d.ts`) are the kinds written from `crates/canoa/include/canoa.*.kinds.json` by
`tools/write_kinds.mjs` (`--check` says whether they are current); the host's objects (`kinds.ProcessSpawner`, `kinds.InstallTools`)
are the handles a slot receives.

## Forms (`forms.js`)

`section(id, title, description?)`, `ui(sectionId)` (the `x-section` schema keyword, spread into an `s` builder's keywords),
`when(field, equals)` and `formMeta({ sections, fields })` (the `meta` entry `{ form }`, for `static meta`). The schema is
`static config = s.object({...})` from `guatiao/provide`. There is no field type of Canoa's own.
