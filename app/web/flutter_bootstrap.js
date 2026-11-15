{{flutter_js}}
{{flutter_build_config}}

// No service worker: the host revalidates every file by its ETag, and a worker would only be one more thing to wait
// for. Glyphs the bundled Roboto lacks are looked for beside the page, never at Google's font host: the host's
// Content-Security-Policy allows the site's own origin alone, and the build puts the fonts there.
_flutter.loader.load({
  config: { fontFallbackBaseUrl: "assets/fonts/fallback/" },
});
