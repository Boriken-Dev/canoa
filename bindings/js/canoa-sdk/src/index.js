// Write a Canoa provider in JavaScript or TypeScript: subclass a kind's base class, set a few static fields or override a few methods,
// and export the class; a guatiao source loads the module like any other, and the host offers the provider.
//
// The modules import each other by relative path only, since the source that runs them (QuickJS) resolves nothing else beside
// `guatiao`, `guatiao/kinds` and `guatiao/provide`: a library imports this file or one of its siblings by a relative path to where
// the SDK is under the library's `paths`.
export {
  AccountProvider,
  AccountWatch,
  accountKind,
  credentialSchema,
  endpointSlot,
  loginMethod,
  loginAbsent,
  loginOutcome,
  loginPresent,
  loginUnknown,
  modelsListed,
  modelsUnknown,
  renewUnchanged,
  renewed,
  usageUnsupported,
} from './account.js';
export { FORM_KEY, formMeta, section, ui, when } from './forms.js';
export { GatewayLease, ModelGateway } from './gateway.js';
export * as kinds from './kinds.js';
export {
  FormSecretProvider,
  SecretProvider,
  SystemStoreSecretProvider,
  keepingOf,
  nameOf,
  release,
} from './secret.js';
export { LogPanelTab, MAX_TAIL_LINES, PagePanelTab, PanelTab, TerminalPanelTab } from './panel.js';
export {
  EnvProfileItem,
  NETWORK_MODES,
  NetworkProfileItem,
  ProfileItem,
  SettingProfileItem,
  effect,
  textOf,
  valuesOf,
  wordsOf,
} from './profile.js';
export { MAX_VALUES, SettingsSection } from './section.js';
export { AccountSpeech, SpeechOutput, SpeechProvider } from './speech.js';
export {
  CatalogSource,
  CommandCatalogSource,
  DEFAULT_SECONDS,
  MAX_BYTES,
  ReadCatalogSource,
  TAIL_BYTES,
  fieldsOf,
  release as releaseSpawner,
} from './source.js';
export { ModelStore, ProgramStore, StoreExtension } from './store.js';
export { Retry, UsageReader, UsageReading, bucket, failed, reading } from './usage.js';
export { AgentVendor } from './vendor.js';
