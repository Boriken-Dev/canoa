// Write a Canoa provider in JavaScript or TypeScript: subclass a kind's base class, set a few static fields or override a few methods,
// and export the class; a guatiao source loads the module like any other, and the host offers the provider.
//
// The modules import each other by relative path only, since the source that runs them (QuickJS) resolves nothing else beside
// `guatiao`, `guatiao/kinds` and `guatiao/provide`: a library imports this file or one of its siblings by a relative path to where
// the SDK is under the library's `paths`.
export { FORM_KEY, formMeta, section, ui, when } from './forms.js';
export * as kinds from './kinds.js';
export {
  FormSecretProvider,
  MAX_NAME,
  MAX_NAMES,
  MAX_VALUE,
  SecretProvider,
  SystemStoreSecretProvider,
  keepingOf,
  nameOf,
  release,
} from './secret.js';
export { SpeechOutput, SpeechProvider } from './speech.js';
export { ProgramStore, StoreExtension } from './store.js';
export { AgentVendor } from './vendor.js';
