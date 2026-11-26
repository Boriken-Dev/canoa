# canoa-sdk (JavaScript and TypeScript)

Write a Canoa provider in JavaScript or TypeScript. Each kind Canoa loads has one base class: a new provider subclasses it, sets a few
static fields or overrides a few methods, and exports the class. A [Guatiao](https://github.com/Boriken-Dev/guatiao) source for
JavaScript (QuickJS, built in: no Node needed) or the Node binding loads the module like any other, and the host offers the provider
with no change to Canoa.

```js
import { AgentVendor } from './canoa-sdk/src/vendor.js';

export class Quill extends AgentVendor {
  static agent = 'quill';
  static title = 'Quill';
  static standards = [{ id: 'quill-key', title: 'Quill API key' }];
}
```

The package is plain ES modules with JSDoc and generated `.d.ts` files (`npm run types` writes `types/`), so it works from JavaScript
and TypeScript alike. Its modules import each other and `guatiao`, `guatiao/kinds` and `guatiao/provide` only: QuickJS resolves
nothing else, so a library imports the SDK by a relative path to where it is under the library's `paths`
(`examples/providers/js` does) or bundles it.

| Kind | Base class | Module |
| --- | --- | --- |
| `agent_vendor` | `AgentVendor` | `canoa-sdk/vendor` |
| `store_extension`, `program_store`, `model_store` | `StoreExtension`, `ProgramStore`, `ModelStore` | `canoa-sdk/store` |
| `speech_provider`, `speech_output` | `SpeechProvider`, `SpeechOutput` | `canoa-sdk/speech` |
| `secret_provider` | `SecretProvider`, `FormSecretProvider`, `SystemStoreSecretProvider` | `canoa-sdk/secret` |
| `profile_item` | `ProfileItem`, `SettingProfileItem`, `EnvProfileItem`, `NetworkProfileItem` | `canoa-sdk/profile` |
| `catalog_source` | `CatalogSource`, `CommandCatalogSource`, `ReadCatalogSource` | `canoa-sdk/source` |
| `settings_section` | `SettingsSection` | `canoa-sdk/section` |
| `notifier` | `Notifier`, `delivery`, `noticeOf`, `settingsOf` | `canoa-sdk/notifier` |
| `panel_tab` | `PanelTab`, `TerminalPanelTab`, `LogPanelTab`, `PagePanelTab` | `canoa-sdk/panel` |
| `usage_reader` | `UsageReader`, `UsageReading` | `canoa-sdk/usage` |

The examples in `examples/providers/js` are loaded by `crates/canoa-host/tests/script_providers.rs`. See `AGENTS.md` for the API.
Development: `npm install` (TypeScript is a dev dependency), then `GUATIAO_NODE=<guatiao>/bindings/node/guatiao npm test`, which typechecks the SDK and the
examples in `examples/providers/js`, writes the `.d.ts` files to a folder it removes afterwards and runs the unit tests in `test/` (`npm run typecheck` is the check alone, `npm run unit` the tests alone, `npm run types` writes `types/`);
after a kind's description changes, `GUATIAO_NODE=<guatiao>/bindings/node/guatiao node tools/write_kinds.mjs`.
