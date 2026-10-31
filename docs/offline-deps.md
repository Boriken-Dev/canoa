## Offline dependencies

The [deps branch](https://github.com/Boriken-Dev/Canoa/tree/deps) follows AI6's pub/ and cdn/ cache layout. Refreshing uses the [GitHub Actions workflow](https://github.com/Boriken-Dev/Canoa/actions/workflows/deps.yml), which fetches resolved Dart hosted packages and versioned, SHA256-verified Unicode fallback fonts. CanvasKit stays local by using --no-web-resources-cdn.

The initial branch is only a scaffold. A cache is ready when manifest.json reports status complete. The Flutter SDK, engine, native toolchains and deployment rewrites for font fallbacks are separate requirements.

To consume after refresh, create a git worktree for deps, point PUB_CACHE at its pub directory and run flutter pub get --offline. Never mirror tokens or private registry credentials.
