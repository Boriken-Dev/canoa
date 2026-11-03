// A target of a browser over its DevTools protocol, for the real-browser runs.
let last = "";

/** What a run was doing when it stopped, for a watchdog's report. */
export const lastCall = () => last;

export class Session {
  constructor(url) {
    this.socket = new WebSocket(url);
    this.pending = new Map();
    this.contexts = [];
    this.next = 1;
    this.opened = new Promise((resolve, reject) => {
      this.socket.onopen = resolve;
      this.socket.onerror = reject;
    });
    // A click can close the window it was in, and then no answer comes.
    this.socket.onclose = () => {
      for (const waiting of this.pending.values()) waiting.reject(new Error("the target closed"));
      this.pending.clear();
    };
    this.socket.onmessage = (event) => {
      const message = JSON.parse(event.data);
      if (message.method === "Runtime.executionContextCreated") this.contexts.push(message.params.context);
      const waiting = this.pending.get(message.id);
      if (!waiting) return;
      this.pending.delete(message.id);
      if (message.error) waiting.reject(new Error(message.error.message));
      else waiting.resolve(message.result);
    };
  }
  async call(method, params = {}) {
    last = `${method} ${String(params.expression ?? "").slice(0, 120)}`;
    await this.opened;
    const id = this.next++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.socket.send(JSON.stringify({ id, method, params }));
    });
  }
  async run(expression, { gesture = false, context } = {}) {
    const reply = await this.call("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true, userGesture: gesture, contextId: context });
    if (reply.exceptionDetails) throw new Error(reply.exceptionDetails.exception?.description ?? reply.exceptionDetails.text);
    return reply.result.value;
  }
  close() {
    this.socket.close();
  }
}
