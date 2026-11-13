// Loaded only in packaged extension UI, not in the standalone PWA.
// Only the trusted extension service worker has access to the pending context.
const send = (type, message = {}) => chrome.runtime.sendMessage({ type, ...message }).then((reply) => {
  if (reply?.error) throw new Error(reply.error);
  return reply ?? {};
});

// Who this page is to the worker, which works it out from the address itself: the side panel, or a window a session was popped out to.
const query = new URLSearchParams(globalThis.location?.search ?? "");
const CLIENT = query.get("view") === "window" ? `window:${query.get("popout")}` : "panel";

const web = (tab) => Number.isSafeInteger(tab.id) && /^https?:\/\//.test(tab.url || "");

// Which session is in which tab, and which tabs it shares with its agent (see tab-bindings.js). The panel calls `call` with a
// name and JSON arguments and gets JSON back; `watch` tells it when any of that has changed.
const TABS = {
  async snapshot() {
    const all = (await chrome.tabs.query({})).filter(web);
    // A window of its own is no browser window's front: the tab it works with is the web tab the person used last.
    const front = CLIENT === "panel"
      ? (await chrome.tabs.query({ active: true, lastFocusedWindow: true }))[0]
      : (await chrome.tabs.query({ active: true })).filter(web).sort((a, b) => (b.lastAccessed ?? 0) - (a.lastAccessed ?? 0))[0];
    const { sessions } = await send("canoa.tabs.list");
    return {
      tabs: all.map((tab) => ({ id: tab.id, title: tab.title ?? "", url: tab.url, active: tab.id === front?.id })),
      active: front?.id ?? null,
      sessions
    };
  },
  attach: (sessionId, tabId) => send("canoa.tabs.attach", { sessionId, tabId }),
  // The session stays as it is and this client takes the tab binding, on the tab in front: nothing is resumed or granted.
  async move(sessionId, tabId) {
    return send("canoa.tabs.move", { sessionId, tabId });
  },
  detach: (sessionId) => send("canoa.tabs.detach", { sessionId }),
  // From the person's click: the browser asks for the site here if it has not been given it, and the worker checks it was.
  async share(sessionId, tabId, peerId) {
    const tab = await chrome.tabs.get(tabId);
    if (!web(tab)) throw new Error("That tab cannot be shared");
    if (!(await chrome.permissions.request({ origins: [`${new URL(tab.url).origin}/*`] }))) throw new Error("Browser host access was refused");
    return send("canoa.tabs.share", { sessionId, tabId, peerId });
  },
  unshare: (sessionId, tabId) => send("canoa.tabs.unshare", { sessionId, tabId })
};

// Sessions shown in windows of their own (see popout-windows.js). Only the panel may ask; the worker refuses a window.
const POPOUT = {
  open: (sessionId, link, title, look, density) => send("canoa.popout.open", { sessionId, link, title, look, density }),
  focus: (sessionId) => send("canoa.popout.focus", { sessionId }),
  close: (sessionId) => send("canoa.popout.close", { sessionId })
};

function watch(callback) {
  let timer;
  const soon = () => {
    clearTimeout(timer);
    timer = setTimeout(callback, 100);
  };
  chrome.tabs.onUpdated.addListener((_id, changed) => {
    if (changed.status !== undefined || changed.title !== undefined || changed.url !== undefined) soon();
  });
  chrome.tabs.onRemoved.addListener(soon);
  chrome.tabs.onActivated.addListener(soon);
  chrome.tabs.onCreated.addListener(soon);
  chrome.storage.onChanged.addListener((_changed, area) => {
    if (area === "session") soon();
  });
}

window.canoaBridge = Object.freeze({
  client: CLIENT,
  popout: Object.freeze({
    call: async (name, args) => {
      if (!Object.hasOwn(POPOUT, name)) throw new Error("Unknown call");
      return JSON.stringify((await POPOUT[name](...JSON.parse(args))) ?? {});
    },
    // The window that went, by id, so the panel can tell which of its windows closed.
    watch: (callback) => chrome.windows.onRemoved.addListener(callback)
  }),
  tabs: Object.freeze({
    call: async (name, args) => {
      if (!Object.hasOwn(TABS, name)) throw new Error("Unknown call");
      return JSON.stringify((await TABS[name](...JSON.parse(args))) ?? {});
    },
    watch
  }),
  async claimPendingContext() {
    try {
      const response = await chrome.runtime.sendMessage({
        type: "canoa.context.claim"
      });
      return response?.context ? JSON.stringify(response.context) : null;
    } catch {
      return null;
    }
  }
});
