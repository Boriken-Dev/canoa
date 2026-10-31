# Boriken Dev naming and namespaces

Canoa and associated Coqui libraries are published under **[Boriken Dev](https://boriken.dev)**. Use these conventions for every new package, application, extension, identity, and service.

| Context | Convention | Example |
| --- | --- | --- |
| Publisher / company | Boriken Dev | Boriken Dev |
| Primary public domain | `boriken.dev` | `https://boriken.dev` |
| GitHub organization | `Boriken-Dev` | `Boriken-Dev/Canoa` |
| Java / Kotlin / Android application ID | reversed domain `dev.boriken.*` | `dev.boriken.canoa` |
| Apple bundle ID | reversed domain `dev.boriken.*` | `dev.boriken.canoa` |
| Chrome native messaging host | lowercase dotted identifier | `dev.boriken.canoa` |
| Optional future native helper | child of base identifier | `dev.boriken.canoa.host` |
| Related Java / Kotlin library | `dev.boriken.coqui.*` | `dev.boriken.coqui.net.transport` |
| Rust crates | ecosystem-standard lowercase kebab-case | `canoa`, `coqui-net-transport`, `coqui-peer` |
| Dart / Flutter pub package | lowercase snake_case | `canoa` |
| Python distribution | PyPI-compatible project name | `coqui-net-transport` only if available |
| npm package | verified account/scope, ecosystem naming | `@boriken/canoa-chromium` is currently private; availability must be verified before publication |
| Browser extension display name | product name | **Canoa** |

This is a *namespace convention*, not a claim that every future distribution name, subdomain or account is already registered.

## Public sites and docs

Canoa's confirmed repository documentation currently uses [GitHub Pages](https://boriken-dev.github.io/Canoa/). Do not switch MkDocs `site_url`, change package links, or create deployment instructions for a `*.boriken.dev` hostname until DNS, HTTPS, and publication are configured.

## Transport naming

Use **`coqui-net-transport`** for the shared WebTransport/WebSocket connection, framing and fallback library. This follows the existing Boriken Dev naming family (`coqui-net-acl`, `coqui-peer`). Its API remains application-protocol-neutral; ACP, MCP and Guatiao frame/application semantics do not belong in the transport core.

The Rust crate is currently developed in Canoa's workspace and has not been published independently.

## Browser peer authentication

The reusable browser method is under `coqui-peer/methods/webcrypto`, using the profile `dev.boriken.coqui.peer-webcrypto/v1`. The Canoa-specific browser private-key store is named `dev.boriken.canoa.peer`; another application must use its **own** isolated store name rather than share that identity implicitly.

The method's experimental JavaScript module is bundled locally into Canoa as a Git-blob-pinned vendor artifact to comply with offline/MV3 constraints.
