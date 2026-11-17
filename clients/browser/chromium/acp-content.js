// Between the page's world and the service worker. The page can forge anything it sends here, so the one thing this adds is what
// the page cannot forge from its own world: whether the person has just acted.
(() => {
  const TAG = "canoa-acp";
  const KEEP_ALIVE_MS = 20_000;
  let page;
  let worker;
  let keepAlive;

  function connectWorker() {
    worker = chrome.runtime.connect({ name: TAG });
    worker.onMessage.addListener((message) => page.postMessage(message));
    worker.onDisconnect.addListener(() => {
      clearInterval(keepAlive);
      worker = undefined;
      // A service worker that was stopped has dropped its connections, and the next call starts it again.
      page.postMessage({ closedAll: "failed" });
    });
    // A message resets the service worker's idle timer, which would otherwise end its connections.
    keepAlive = setInterval(() => worker?.postMessage({ op: "ping" }), KEEP_ALIVE_MS);
  }

  function fromPage({ data }) {
    if (data === null || typeof data !== "object") return;
    if (worker === undefined) connectWorker();
    const message = data.op === "request" ? { ...data, activated: navigator.userActivation.isActive } : { ...data, activated: false };
    worker.postMessage(message);
  }

  window.addEventListener("message", (event) => {
    if (event.source !== window || event.data === null || typeof event.data !== "object") return;
    const kind = event.data[TAG];
    if (kind === "page-ready" && page === undefined) {
      window.postMessage({ [TAG]: "content-ready" }, location.origin);
    } else if (kind === "hello" && page === undefined && event.ports[0] !== undefined) {
      page = event.ports[0];
      page.onmessage = fromPage;
    }
  });
  window.addEventListener("pagehide", () => {
    clearInterval(keepAlive);
    worker?.disconnect();
    worker = undefined;
  });
  window.postMessage({ [TAG]: "content-ready" }, location.origin);
})();
