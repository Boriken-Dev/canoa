// Office pane tool bridging and frame-sender validation for the browser extension. Pure (no chrome.*), so Node tests run directly against it.
import { createToolProvider } from "./tool-provider.js";

export const OFFICE_APPS = new Set(["outlook", "word", "excel"]);

export function secureOrigin(origin) {
  try {
    const url = new URL(origin);
    return url.protocol === "https:" || (url.protocol === "http:" && ["localhost", "127.0.0.1", "[::1]"].includes(url.hostname));
  } catch {
    return false;
  }
}

/**
 * Verifies a connecting runtime port sender using only browser-reported sender metadata.
 * Own UI pages (`chrome-extension://<id>/ui/...`) stay marked as `"own"`. External pages (top frame
 * or granted iframe, `frameId >= 0`) are accepted when their browser-reported `origin` is secure and
 * matches `sender.url`'s origin.
 */
export function senderEndpoint(sender, { runtimeId, ownOrigin, uiPrefix }) {
  const { id, origin, frameId, tab, url } = sender ?? {};
  if (id !== runtimeId) return null;
  if (origin === ownOrigin && typeof url === "string" && url.startsWith(uiPrefix)) {
    return { kind: "own", origin, tabId: tab?.id };
  }
  if (typeof frameId !== "number" || frameId < 0 || tab?.id === undefined || !secureOrigin(origin)) return null;
  try {
    if (typeof url !== "string" || new URL(url).origin !== origin) return null;
  } catch {
    return null;
  }
  return { kind: "page", origin, tabId: tab.id, frameId };
}

/**
 * Bridges Office pane tool providers (`outlook`, `word`, `excel`) arriving over granted page ports to the
 * local host's `@tools` door, and tells `gateway` which `(origin, tabId)` panes currently hold active tools.
 */
export function createOfficeToolsBridge({ dialLocal, gateway, callTimeoutMs = 190_000 }) {
  /** @type {Map<string, { provider: ReturnType<typeof createToolProvider>, regs: Set<object> }>} */
  const byApp = new Map();
  /** @type {Set<object>} */
  const ports = new Set();
  let nextCall = 1;

  function activeAppsFor(origin, tabId) {
    const apps = new Set();
    for (const entry of ports) {
      if (entry.origin === origin && entry.tabId === tabId) {
        for (const app of entry.apps) apps.add(app);
      }
    }
    return apps;
  }

  function syncTab(origin, tabId) {
    gateway.setPageTools(origin, tabId, activeAppsFor(origin, tabId));
  }

  function pickRegistration(app) {
    const bucket = byApp.get(app);
    if (!bucket || bucket.regs.size === 0) return null;
    // Pick the most recently registered connected pane for this Office app.
    let chosen = null;
    for (const reg of bucket.regs) chosen = reg;
    return chosen;
  }

  function getOrCreateApp(app) {
    let bucket = byApp.get(app);
    if (bucket !== undefined) return bucket;
    const regs = new Set();
    const provider = createToolProvider({
      provider: app,
      dial: dialLocal,
      serve: ({ stream, session, request, still }) => new Promise((resolve) => {
        if (!still()) {
          resolve(null);
          return;
        }
        const reg = pickRegistration(app);
        if (reg === null || reg.closed) {
          resolve(null);
          return;
        }
        if (!reg.streams.has(stream)) {
          reg.streams.add(stream);
          reg.port.postMessage({ toolsEvent: { app, event: { type: "open", stream, session } } });
        }
        const callId = nextCall++;
        const finish = (answer) => {
          if (!reg.pending.delete(callId)) return;
          clearTimeout(timer);
          resolve(answer);
        };
        const timer = setTimeout(() => finish(null), callTimeoutMs);
        reg.pending.set(callId, { stream, resolve: finish });
        try {
          reg.port.postMessage({ toolsCall: callId, app, event: { type: "mcp", stream, message: request } });
        } catch {
          finish(null);
        }
      }),
      forget: (_principal, stream) => {
        for (const reg of regs) {
          for (const [callId, pending] of [...reg.pending.entries()]) {
            if (pending.stream === stream) pending.resolve(null);
          }
          if (reg.streams.delete(stream)) {
            try {
              reg.port.postMessage({ toolsEvent: { app, event: { type: "close", stream } } });
            } catch {
              // Port is already gone.
            }
          }
        }
      },
    });
    bucket = { provider, regs };
    byApp.set(app, bucket);
    return bucket;
  }

  function removeApp(entry, app) {
    if (!entry.apps.delete(app)) return;
    const bucket = byApp.get(app);
    if (bucket !== undefined) {
      bucket.regs.delete(entry);
      for (const [callId, pending] of [...entry.pending.entries()]) {
        pending.resolve(null);
        entry.pending.delete(callId);
      }
      entry.streams.clear();
      if (bucket.regs.size === 0) {
        bucket.provider.stop();
        byApp.delete(app);
      }
    }
    syncTab(entry.origin, entry.tabId);
  }

  async function registerApp(entry, app, messageId) {
    if (typeof app !== "string" || !OFFICE_APPS.has(app)) {
      entry.port.postMessage({ id: messageId, error: { name: "NotAllowedError", message: "this page may not do that" } });
      return;
    }
    const bucket = getOrCreateApp(app);
    entry.apps.add(app);
    bucket.regs.add(entry);
    syncTab(entry.origin, entry.tabId);
    try {
      await bucket.provider.ensure();
      if (entry.closed || !entry.apps.has(app)) return;
      entry.port.postMessage({ id: messageId, result: { ok: true } });
    } catch (error) {
      removeApp(entry, app);
      if (!entry.closed) {
        entry.port.postMessage({
          id: messageId,
          error: { name: "NetworkError", message: String(error?.message ?? "the host is not available") },
        });
      }
    }
  }

  return {
    attach(port, { origin, tabId }) {
      const gatewayListeners = [];
      const disconnectListeners = [];
      const entry = {
        port,
        origin,
        tabId,
        apps: new Set(),
        streams: new Set(),
        pending: new Map(),
        closed: false,
      };
      ports.add(entry);

      const gatewayPort = {
        postMessage: (message) => port.postMessage(message),
        onMessage: { addListener: (fn) => gatewayListeners.push(fn) },
        onDisconnect: { addListener: (fn) => disconnectListeners.push(fn) },
        disconnect: () => port.disconnect?.(),
      };

      port.onMessage.addListener((message) => {
        if (message !== null && typeof message === "object") {
          if (message.toolsReply !== undefined) {
            const pending = entry.pending.get(message.toolsReply);
            if (pending !== undefined) pending.resolve(message.message ?? null);
            return;
          }
          if (message.op === "tools.register" && typeof message.id === "number") {
            void registerApp(entry, message.args?.app, message.id);
            return;
          }
          if (message.op === "tools.unregister" && typeof message.id === "number") {
            removeApp(entry, message.args?.app);
            port.postMessage({ id: message.id, result: { ok: true } });
            return;
          }
        }
        for (const listen of gatewayListeners) listen(message);
      });

      port.onDisconnect.addListener(() => {
        entry.closed = true;
        ports.delete(entry);
        for (const app of [...entry.apps]) removeApp(entry, app);
        for (const listen of disconnectListeners) listen();
      });

      gateway.attach(gatewayPort, { origin, tabId });
    },
  };
}
