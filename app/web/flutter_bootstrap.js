{{flutter_js}}
{{flutter_build_config}}

// No service worker: the host revalidates every file by its ETag, and a worker would only be one more thing to wait
// for. Glyphs the bundled Roboto lacks are looked for beside the page, never at Google's font host: the host's
// Content-Security-Policy allows the site's own origin alone, and the build puts the fonts there.
const config = { fontFallbackBaseUrl: "assets/fonts/fallback/" };

// Whether this page starts the engine in multi-view mode, so that a browser window the app opens is another view of the one
// running app (lib/popout/platform_web.dart). Only when `?windows=views` asks for it, and not where the page is a window of its
// own (`popout`), the chat alone in a frame or the extension's panel (`mode=chat`, the extension's bridge, a framing page), nor in
// a browser whose navigator has no `userAgentData`: the shims that view needs lean on the engine's use of the opener's window and
// document, which was measured in Chromium only. It is not the default because xterm2 5.4.0 opens its text input on the engine's
// implicit view, which a multi-view engine has none of, so the Terminal section would take no input in such a page.
// `?windows=copy` is the default's name: a second copy of the app in the window.
function startsViews() {
  const query = new URLSearchParams(location.search);
  const asked = query.get("windows");
  if (query.has("popout") || query.get("mode") === "chat" || window.canoaBridge || window.top !== window) return false;
  if (typeof window.open !== "function") return false;
  return asked === "views" && navigator.userAgentData !== undefined;
}

_flutter.loader.load({
  config,
  onEntrypointLoaded: async (initializer) => {
    const views = startsViews();
    const engine = await initializer.initializeEngine({ ...config, multiViewEnabled: views });
    // The app's main() runs inside runApp() and reads this; the engine's handle exists only once it returns.
    window.canoaMultiView = views;
    const app = await engine.runApp();
    if (!views) return;
    // The page's own view fills the window; the app's windows are views added to the documents of the browser windows it opens.
    const host = document.createElement("div");
    host.style.cssText = "position:fixed;inset:0";
    document.body.append(host);
    window.canoaMainView = app.addView({ hostElement: host });
    window.canoaFlutterApp = app;
  },
});
