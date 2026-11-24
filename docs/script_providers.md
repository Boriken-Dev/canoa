# Providers written as scripts

Most of what {{ name }} offers a person is a **provider** of a kind: a secret provider, a profile item, a catalog source, a page of Settings,
a tab of the panel, a Store type, a speech engine, an agent's description. A provider does not have to be Rust. It can be a few lines of
JavaScript or Python that subclass a base class of the {{ name }} SDK, and the same files run on Windows, Linux and macOS with nothing to build.
A script is shared between hosts as the files it is.

| Kind | What it is | JavaScript and Python base classes |
| --- | --- | --- |
| `agent_vendor` | who an agent is and how {{ name }} shows it | `AgentVendor` |
| `usage_reader` | how an agent's quota is read | `UsageReader`, `UsageReading` |
| `store_extension`, `program_store`, `model_store` | a Store type, a program the Store installs, and the models it lists | `StoreExtension`, `ProgramStore`, `ModelStore` |
| `secret_provider` | somewhere a secret comes from | `SecretProvider`, `FormSecretProvider`, `SystemStoreSecretProvider` |
| `profile_item` | one thing a profile may set | `ProfileItem`, `SettingProfileItem`, `EnvProfileItem`, `NetworkProfileItem` |
| `catalog_source` | where a Store type's catalog comes from | `CatalogSource`, `CommandCatalogSource`, `ReadCatalogSource` |
| `settings_section` | a page, or a group on a page, of Settings | `SettingsSection` |
| `panel_tab` | a kind of tab of the bottom panel | `PanelTab`, `TerminalPanelTab`, `LogPanelTab` |
| `speech_provider`, `speech_output` | dictation and a voice | `SpeechProvider`, `SpeechOutput` |
| `account_speech` | the speech-to-text services the accounts of some kinds can call | `AccountSpeech` |

The classes after the first of each row mirror what {{ name }} carries: subclass one to change a few words, or read it as the example.
The SDK is in `bindings/js/canoa-sdk` (plain ES modules with `.d.ts` files, so TypeScript works too) and `bindings/python/canoa-sdk`; each
has an `AGENTS.md` with the whole API, and `examples/providers` has a library of each language that serves four kinds.

## What runs it

**JavaScript is built in.** {{ name }} carries QuickJS, so a script in JavaScript needs no installed runtime, on any platform it runs on.
**Python is optional**: a Guatiao source for Python (`guatiao_source_py`) loads a library written in Python for a person who has Python,
which the Store's Programs list installs. Nothing {{ name }} ships depends on it.

A script has no files, network or programs of its own. It is handed what it may use:

- a **spawner**, for a secret provider, a catalog source, a program store and a speech engine: it starts a program as the person
  (`spawn`) and, for a catalog source, reads a file anywhere the person's account can (`readFile` in JavaScript, `read_file` in Python,
  by an absolute path). Every program a script runs starts through it, and the script destroys it when the call ends.
- **tools**, for a program store: a checked download and a safe unpack.

## A provider in JavaScript

```js
// region_ui.js
import { EnvProfileItem } from './sdk/profile.js';

export class Region extends EnvProfileItem {
  static item = 'region';                       // the id a profile keeps the values under
  static title = 'Region';
  static values = {                             // what the item asks for: a JSON Schema of an object
    type: 'object', required: ['name'],
    properties: { name: { type: 'string', title: 'Name' } },
  };
  static variables = { name: ['REGION'] };      // each text member sets the variables it lists
}
```

```toml
# region_ui.guatiao: what the source reads before it runs anything
type = "js"
name = "region_ui"
version = "1.0.0"
main = "region_ui.js"
kinds = ["profile_item"]
paths = ["."]
```

The module is the library and every class it exports that extends a base class is a provider. A library imports the SDK by a relative
path, since QuickJS resolves nothing else, so a package carries the SDK modules it imports beside it (`sdk/profile.js`, `sdk/kinds.js`,
`sdk/model.js`; the speech and catalog source classes also `sdk/wait.js`, which lets the host's other calls through while a program runs). The same provider in Python:

```python
# region_ui.py
from dataclasses import dataclass
from typing import Annotated

from canoa_sdk import EnvProfileItem, ui

__version__ = "1.0.0"
__all__ = ["Region"]

@dataclass
class RegionValues:
    name: Annotated[str, ui(placeholder="eu-west")]

class Region(EnvProfileItem):
    item = "region"
    title = "Region"
    values = RegionValues                       # its schema and form come from the dataclass
    variables = {"name": ["REGION"]}
```

## Trying one

Name the folder in the host's `{{ config_file }}`. JavaScript uses the engine the host carries:

```toml
[host.extensions.region]
source = "bundled:js"
provider = "region_ui_region"            # the id of a store_extension provider the library offers, or any id when it offers none
[host.extensions.region.source_settings]
dirs = ["C:/canoa/region"]                # the folders that hold a *.guatiao manifest
```

Every kind the library serves is then offered as an extension's would be: the item in Settings, Profiles, the section in Settings, the
tab in the panel's new-tab menu, the source in a Store type's settings. The host reads `{{ config_file }}`'s `[host]` when it starts.

## Publishing one

A package is a folder: the manifest, `package.json`, the modules and the SDK files they import. Put it in a git repository and list it
in an extensions catalog (see [Desktop and configuration](configuration.md#extensions-catalogs)):

```toml
[extensions.region]
source = "./packages/region"                  # a folder of the catalog's repository, or any git repository
pin = "sha256:<the folder's tree digest>"
runtime = "js"                                # a package of scripts: the folder is the library, no `load`, no platform
provider = "region_ui_region"
needs = { kind = "store_extension", version = "1" }
```

The `pin` is the tree digest of the folder, the one an overlay's pin is. A host that is shown a folder whose digest is not the entry's
refuses it and says which digest it found, so the first install of a new package tells the publisher what to write.

Store, Extensions lists the package. Installing it shows a plan (the source, the digest, the provider, where the files go), fetches the
folder, checks it against the `pin` before keeping a byte, keeps it in `<home>/extensions/NAME` and writes
`source = "bundled:js"`, `source_settings`, `pin` and `root` into `[host.extensions.NAME]`. The digest is checked again each time the host
loads it, so a script that changed since is not run. A package written in Python lists `runtime = "python"` and is refused by a host that
has no Python source: such a host names the package's folder in `{{ config_file }}` under the source it has set up.

**Giving the same package to another host.** A catalog entry is pinned content, so every host that reads the catalog installs the very
same bytes, and each checks them itself. The Store of a paired host is a client of that host's control channel: choose the entry there
(`extension/plan`, then `extension/apply`) and that host fetches the package and keeps it, with no copy made through the host you are
sitting at.

## What ships as scripts

{{ name }}'s own Bitwarden secret provider, the identity and proxy profile items (the proxy itself is the host's, the item only says which
network its values ask for) and the command and read catalog sources are packages of JavaScript written with these classes, carried in the
host and offered as an extension's would be. What needs the operating system or the host's own machinery stays Rust: the system
credential store, the proxy and its TLS handling, the agents' adapters and the Store's installers.
