// Loaded only in packaged extension UI, not in the standalone PWA.
// Only the trusted extension service worker has access to the pending context.
const send = (type, message = {}) => chrome.runtime.sendMessage({ type, ...message }).then((reply) => {
  if (reply?.error) throw new Error(reply.error);
  return reply ?? {};
});

const web = (tab) => Number.isSafeInteger(tab.id) && /^https?:\/\//.test(tab.url || "");

// Which session is in which tab, and which tabs it shares with its agent (see tab-bindings.js). The panel calls `call` with a
// name and JSON arguments and gets JSON back; `watch` tells it when any of that has changed.
const TABS = {
  async snapshot() {
    const all = (await chrome.tabs.query({})).filter(web);
    const [front] = await chrome.tabs.query({ active: true, lastFocusedWindow: true });
    const { sessions } = await send("canoa.tabs.list");
    return {
      tabs: all.map((tab) => ({ id: tab.id, title: tab.title ?? "", url: tab.url, active: tab.id === front?.id })),
      active: front?.id ?? null,
      sessions
    };
  },
  attach: (sessionId, tabId) => send("canoa.tabs.attach", { sessionId, tabId }),
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
