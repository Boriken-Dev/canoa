{{flutter_js}}
{{flutter_build_config}}

// Glyphs the bundled Roboto lacks are looked for beside the page, never at Google's font host:
// the host's Content-Security-Policy allows the site's own origin alone.
_flutter.loader.load({
  config: { fontFallbackBaseUrl: "assets/fonts/fallback/" },
  serviceWorkerSettings: { serviceWorkerVersion: {{flutter_service_worker_version}} },
});
