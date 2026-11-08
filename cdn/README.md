# Runtime CDN assets

Only immutable dependencies actually needed by the pinned Flutter renderer or Canoa's web build belong here. Each file must have a content hash in `manifest.json`. The workflow builds with `--no-web-resources-cdn` to bundle CanvasKit locally. Flutter's Unicode fallback fonts may still require a separate version-pinned mirror. Not yet populated.
