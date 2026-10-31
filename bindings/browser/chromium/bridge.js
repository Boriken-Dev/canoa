// Loaded only in packaged extension UI, not in the standalone PWA.
// Only the trusted extension service worker has access to the pending context.
window.canoaBridge = Object.freeze({
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
