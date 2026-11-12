// Loaded only in packaged extension UI, not in the standalone PWA.
// Only the trusted extension service worker has access to the pending context.
const tabs = (type, message = {}) => chrome.runtime.sendMessage({ type, ...message }).then((reply) => {
  if (reply?.error) throw new Error(reply.error);
  return reply ?? {};
});

window.canoaBridge = Object.freeze({
  // Which session is in which tab, and which tabs it shares with its agent (see tab-bindings.js).
  tabs: Object.freeze({
    attach: (sessionId, tabId) => tabs("canoa.tabs.attach", { sessionId, tabId }),
    detach: (sessionId) => tabs("canoa.tabs.detach", { sessionId }),
    list: () => tabs("canoa.tabs.list"),
    share: (sessionId, tabId, peerId) => tabs("canoa.tabs.share", { sessionId, tabId, peerId }),
    unshare: (sessionId, tabId) => tabs("canoa.tabs.unshare", { sessionId, tabId }),
    sessionsFor: (origin) => tabs("canoa.tabs.sessionsFor", { origin })
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
