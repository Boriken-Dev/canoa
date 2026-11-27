
// Appended by the browser test to the build's flutter_bootstrap.js in place of the app's own start: multi-view mode, with the
// engine's handle kept for `addView` and `removeView`, and the page's own view added to #main.
window.probeShimsPatch?.(new URLSearchParams(location.search).get('shims') || '');
_flutter.loader.load({
  onEntrypointLoaded: async (initializer) => {
    const runner = await initializer.initializeEngine({ multiViewEnabled: true });
    window.flutterApp = await runner.runApp();
    window.flutterApp.addView({ hostElement: document.getElementById('main') });
  },
});
