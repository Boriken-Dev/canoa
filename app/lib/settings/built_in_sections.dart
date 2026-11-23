// What the host lists of the application's own Settings pages: the list Settings shows until the host has answered, and when it
// cannot or will not. Written by `CANOA_WRITE_SECTIONS=1 cargo test -p canoa-host --lib settings_sections`, which also checks it.
library;

/// The answer of `settings/sections` for the pages the application draws itself, as JSON.
const builtInSectionsListing = r'''
{
  "sections": [
    {
      "id": "general",
      "title": "General",
      "help": null,
      "icon": "tune",
      "group": "App",
      "under": null,
      "keywords": "tray window startup network webtransport timeout sessions remembered folder workspace keep history",
      "origin": "built-in",
      "stored": "config",
      "schema": null,
      "form": null
    },
    {
      "id": "appearance",
      "title": "Appearance",
      "help": null,
      "icon": "palette",
      "group": "App",
      "under": null,
      "keywords": "look theme brightness dark light detail verbose density preview",
      "origin": "built-in",
      "stored": "config",
      "schema": null,
      "form": null
    },
    {
      "id": "notifications",
      "title": "Notifications",
      "help": null,
      "icon": "notifications",
      "group": "App",
      "under": null,
      "keywords": "alerts permission question finished browser toast",
      "origin": "built-in",
      "stored": "config",
      "schema": null,
      "form": null
    },
    {
      "id": "voice",
      "title": "Voice",
      "help": null,
      "icon": "mic",
      "group": "App",
      "under": null,
      "keywords": "dictation speech microphone input device language push to talk transcribe audio read aloud voice speak speaker playback tts text to speech",
      "origin": "built-in",
      "stored": "config",
      "schema": null,
      "form": null
    },
    {
      "id": "shortcuts",
      "title": "Shortcuts",
      "help": null,
      "icon": "keyboard",
      "group": "App",
      "under": null,
      "keywords": "keyboard keys chords hotkeys",
      "origin": "built-in",
      "stored": "config",
      "schema": null,
      "form": null
    },
    {
      "id": "sources",
      "title": "Sources",
      "help": null,
      "icon": "hub",
      "group": "Agents",
      "under": null,
      "keywords": "agents providers programs command install update version ready",
      "origin": "built-in",
      "stored": "config",
      "schema": null,
      "form": null
    },
    {
      "id": "profiles",
      "title": "Profiles",
      "help": null,
      "icon": "layers",
      "group": "Agents",
      "under": null,
      "keywords": "overlays layers model effort mode skills trusted directories project",
      "origin": "built-in",
      "stored": "config",
      "schema": null,
      "form": null
    },
    {
      "id": "mcp",
      "title": "MCP servers",
      "help": null,
      "icon": "extension",
      "group": "Agents",
      "under": null,
      "keywords": "tools servers mcp allowed per agent",
      "origin": "built-in",
      "stored": "config",
      "schema": null,
      "form": null
    },
    {
      "id": "permissions",
      "title": "Permissions",
      "help": null,
      "icon": "security",
      "group": "Agents",
      "under": null,
      "keywords": "bypass mode allow once remembered rules protected paths log revoke full access",
      "origin": "built-in",
      "stored": "config",
      "schema": null,
      "form": null
    },
    {
      "id": "accounts",
      "title": "Accounts",
      "help": null,
      "icon": "accounts",
      "group": "Agents",
      "under": null,
      "keywords": "login sign in api key subscription credentials add remove several",
      "origin": "built-in",
      "stored": "config",
      "schema": null,
      "form": null
    },
    {
      "id": "shells",
      "title": "Shells",
      "help": null,
      "icon": "terminal",
      "group": "Agents",
      "under": null,
      "keywords": "bash git bash powershell pwsh wsl msys2 terminal shell command windows",
      "origin": "built-in",
      "stored": "config",
      "schema": null,
      "form": null
    },
    {
      "id": "usage",
      "title": "Usage",
      "help": null,
      "icon": "speed",
      "group": "Agents",
      "under": null,
      "keywords": "limit warn percent quota",
      "origin": "built-in",
      "stored": "config",
      "schema": null,
      "form": null
    },
    {
      "id": "store",
      "title": "Store",
      "help": null,
      "icon": "storefront",
      "group": "Store",
      "under": null,
      "keywords": "catalog catalogs sources registries overlays skills mcp servers types limits icons size",
      "origin": "built-in",
      "stored": "config",
      "schema": null,
      "form": null
    },
    {
      "id": "extensions",
      "title": "Extensions",
      "help": null,
      "icon": "extension",
      "group": "Store",
      "under": null,
      "keywords": "store types runtimes add library provider host restart",
      "origin": "built-in",
      "stored": "config",
      "schema": null,
      "form": null
    },
    {
      "id": "about",
      "title": "About",
      "help": null,
      "icon": "info",
      "group": "About",
      "under": null,
      "keywords": "version canoa boriken licence",
      "origin": "built-in",
      "stored": "config",
      "schema": null,
      "form": null
    }
  ]
}
''';
