// document.acp in the page's world, under the contract in coqui-acp's docs/browser.md. A classic script, since a page's CSP can
// block anything else. Everything here is the page's to tamper with: the decisions are in the service worker, which trusts only
// what the browser reports about the sender.
(() => {
  const REGISTRY = Symbol.for("coqui-acp.registry");
  const TAG = "canoa-acp";

  function route(providers, agent) {
    const found = providers.find((provider) => provider.info.uuid === agent?.provider);
    if (found === undefined) throw new DOMException("no such agent", "NotFoundError");
    return found;
  }

  // The same registry as bindings/js/src/registry.mjs in coqui-acp, which another provider may have defined first.
  function makeRegistry() {
    const providers = [];
    const events = new EventTarget();
    const announce = () => window.dispatchEvent(new CustomEvent("acp:ready", { detail: { acp: registry } }));
    const registry = Object.freeze({
      version: 1,
      get providers() {
        return Object.freeze(providers.map((provider) => provider.info));
      },
      async agents() {
        return (await Promise.all(providers.map((provider) => provider.agents()))).flat();
      },
      async request(options = {}) {
        const chosen = options.provider === undefined
          ? providers[0]
          : providers.find((provider) => provider.info.uuid === options.provider);
        if (chosen === undefined) throw new DOMException("no such provider", "NotFoundError");
        return chosen.request(options);
      },
      async connect(agent, options = {}) {
        return route(providers, agent).connect(agent, options);
      },
      async revoke(agent) {
        return route(providers, agent).revoke(agent);
      },
      addEventListener: (...args) => events.addEventListener(...args),
      removeEventListener: (...args) => events.removeEventListener(...args),
      register(provider) {
        providers.push(provider);
        provider.addEventListener?.("agentschange", () => events.dispatchEvent(new Event("agentschange")));
        announce();
      },
    });
    window.addEventListener("acp:request", announce);
    return registry;
  }

  function registerProvider(provider) {
    let registry = window[REGISTRY];
    if (registry === undefined) {
      registry = makeRegistry();
      Object.defineProperty(window, REGISTRY, { value: registry });
      Object.defineProperty(document, "acp", { value: registry, enumerable: true });
    }
    registry.register(provider);
  }

  // The same everywhere, as the name is: it tells a site only what the extension's presence already does.
  const info = Object.freeze({ uuid: "dev.boriken.canoa", name: "Canoa", rdns: "dev.boriken.canoa" });
  const pending = new Map();
  const connections = new Map();
  const events = new EventTarget();
  let port;
  let next = 1;
  let attach;
  const attached = new Promise((resolve) => {
    attach = resolve;
  });

  function closeAll(reason) {
    for (const finish of [...connections.values()]) finish(reason);
  }

  function received({ data }) {
    if (data.id !== undefined) {
      const waiting = pending.get(data.id);
      pending.delete(data.id);
      if (data.error) waiting?.reject(new DOMException(data.error.message, data.error.name));
      else waiting?.resolve(data.result);
    } else if (data.conn !== undefined && data.closed !== undefined) {
      connections.get(data.conn)?.(data.closed);
    } else if (data.conn !== undefined) {
      connections.get(data.conn)?.message(data.msg);
    } else if (data.closedAll !== undefined) {
      closeAll(data.closedAll);
      for (const waiting of pending.values()) waiting.reject(new DOMException("the provider restarted", "NetworkError"));
      pending.clear();
    } else if (data.event === "agentschange") {
      events.dispatchEvent(new Event("agentschange"));
    }
  }

  async function call(op, args) {
    await attached;
    const id = next++;
    return new Promise((resolve, reject) => {
      pending.set(id, { resolve, reject });
      port.postMessage({ id, op, args });
    });
  }

  const tagged = (agent) => Object.freeze({ ...agent, provider: info.uuid });

  const provider = {
    info,
    addEventListener: (...args) => events.addEventListener(...args),
    agents: async () => (await call("agents")).map(tagged),
    request: async (options) => {
      const { provider: _chosen, ...rest } = options ?? {};
      return (await call("request", rest)).map(tagged);
    },
    revoke: async (agent) => {
      await call("revoke", { id: agent?.id });
    },
    async connect(agent) {
      const opened = await call("connect", { id: agent?.id });
      const conn = opened.conn;
      let output;
      let finished;
      const closed = new Promise((resolve) => {
        finished = resolve;
      });
      const finish = (reason) => {
        if (!connections.delete(conn)) return;
        try {
          output.close();
        } catch {
          // The page cancelled its reader.
        }
        finished(Object.freeze({ reason }));
      };
      finish.message = (text) => output.enqueue(text);
      connections.set(conn, finish);
      const readable = new ReadableStream({
        start(controller) {
          output = controller;
        },
      });
      const writable = new WritableStream({
        write(text) {
          if (typeof text !== "string") throw new TypeError("a message is a string");
          port.postMessage({ conn, msg: text });
        },
      });
      return Object.freeze({
        agent: tagged(opened.agent),
        readable,
        writable,
        closed,
        close() {
          port.postMessage({ conn, op: "close" });
          finish("page");
        },
      });
    },
  };

  // Whichever of the two scripts runs first, the page's sends a port when the content script says it is listening.
  window.addEventListener("message", (event) => {
    if (event.source !== window || event.data?.[TAG] !== "content-ready" || port !== undefined) return;
    const channel = new MessageChannel();
    port = channel.port1;
    port.onmessage = received;
    window.postMessage({ [TAG]: "hello" }, location.origin, [channel.port2]);
    attach();
  });
  window.addEventListener("pagehide", () => closeAll("navigation"));
  registerProvider(provider);
  window.postMessage({ [TAG]: "page-ready" }, location.origin);
})();
