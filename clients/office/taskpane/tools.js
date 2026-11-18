// The pane as a tool provider of the Canoa on this computer: while the pane is open, the sessions of that Canoa (the app's, the page
// it serves, an Office pane's) are offered the open application's tools as one MCP server, `canoa-outlook`, `canoa-word` or
// `canoa-excel`. The pane registers with the host over its own page's requests (`/office/tools/...`), asks the host what its sessions
// send, and answers; the host names the server and carries each session's messages, one stream per session.
//
// This module holds the wire and the rules, with no Office.js and no DOM: `fetch`, the clock, the open item and the question put
// to the person are arguments, so it runs under `node --test`. The tools themselves are in `tools-outlook.js`, `tools-word.js`
// and `tools-excel.js`.
//
// What the person decides stays here. A read is answered; a write is asked in the pane first, once per call, and is refused
// when the person does not answer or the open item has changed meanwhile.
import { headersFor } from './launch.js';

export const TOOLS_PATH = '/office/tools';

const STREAM = /^[0-9a-f]{32}$/;
const MAX_STREAMS = 256;
const MAX_RUNNING = 16;
const MAX_LABEL = 80;
/** How long a write waits for the person before it is refused. */
export const ASK_MS = 120_000;
const RETRY_MIN_MS = 1000;
/** How soon a poll that failed is asked again. */
const POLL_RETRY_MIN_MS = 250;
const RETRY_MAX_MS = 15_000;

const label = (value) => (typeof value === 'string' ? value.replace(/\p{Cf}|\p{Cc}/gu, ' ').trim().slice(0, MAX_LABEL) : '');
const failure = (id, code, message) => ({ jsonrpc: '2.0', id: id ?? null, error: { code, message } });
const reply = (id, result) => ({ jsonrpc: '2.0', id, result });
const text = (value, isError = false) => ({ isError, content: [{ type: 'text', text: typeof value === 'string' ? value : JSON.stringify(value) }] });

/** Where an agent runs, in the host's words, shown to the person as a claim. */
export function whereOf(session) {
  return session?.via === 'paired' ? 'a program paired with Canoa on this computer' : 'the Canoa app on this computer';
}

/** What a tool says to a session that is not allowed to do it. */
const REFUSED = 'The person did not allow this in the Office pane, so nothing was changed.';

/**
 * @typedef {object} Tool
 * @property {string} name
 * @property {string} description
 * @property {object} inputSchema  JSON Schema of the arguments
 * @property {boolean} [write]     asked in the pane before it runs
 * @property {(args: object) => string} [summary]  what the person is shown for a write
 * @property {(args: object) => Promise<unknown>} run  the Office.js work; its result is the tool's answer
 */

/**
 * @param {object} options
 * @param {string} options.app  `outlook`, `word` or `excel`
 * @param {Tool[]} options.tools
 * @param {() => Promise<{kind: string, key: string, label?: string}|null>} options.relation  the open item now
 * @param {(question: {agent: string, where: string, tool: string, summary: string, item: string|undefined}) => Promise<boolean>} options.ask
 * @param {string} [options.key]  the home's secret, when the manifest carries one
 * @param {typeof fetch} [options.fetchImpl]
 * @param {(ms: number) => Promise<void>} [options.sleep]
 * @param {number} [options.askMs]  how long a write waits for the person
 * @param {(status: string) => void} [options.onStatus]  `connected`, `waiting` or `stopped`
 */
export function createPaneTools({
  app,
  tools,
  relation,
  ask,
  key,
  fetchImpl = (...args) => fetch(...args),
  sleep = (ms) => new Promise((done) => setTimeout(done, ms)),
  askMs = ASK_MS,
  onStatus = () => {},
}) {
  const byName = new Map(tools.map((tool) => [tool.name, tool]));
  /** @type {Map<string, {id: string, agent: string, where: string, running: number}>} */
  let streams = new Map();
  let session = null;
  /** The number of the last message of the host's that this pane has handled, which each poll names so the host lets go of those. */
  let seen = 0;
  let stopped = false;
  let running = null;

  const post = (action, query, body) =>
    fetchImpl(`${TOOLS_PATH}/${action}?${new URLSearchParams(query)}`, {
      method: 'POST',
      credentials: 'same-origin',
      headers: headersFor(key, body === undefined ? {} : { 'content-type': 'application/json' }),
      body: body === undefined ? undefined : JSON.stringify(body),
      keepalive: action === 'close',
    });

  async function send(message) {
    if (session === null) return;
    try {
      await post('send', { session }, message);
    } catch {
      // The poll sees the host gone and starts over.
    }
  }

  const listing = () => ({
    tools: tools.map(({ name, description, inputSchema }) => ({ name, description, inputSchema })),
  });

  /** The item the tools act on, as the app keeps it: a key and a label, no address or subject. */
  async function item() {
    try {
      return (await relation()) ?? null;
    } catch {
      return null;
    }
  }

  async function call(stream, params) {
    const tool = byName.get(params?.name);
    if (tool === undefined) return text(`No tool is called ${label(params?.name)}.`, true);
    const args = params.arguments !== null && typeof params.arguments === 'object' ? params.arguments : {};
    const before = await item();
    if (tool.write) {
      // No answer is a refusal: the question is withdrawn from the pane when the time is up.
      const ending = new AbortController();
      let timer;
      const answered = await Promise.race([
        Promise.resolve(
          ask({
            agent: stream.agent,
            where: stream.where,
            tool: tool.name,
            summary: tool.summary ? tool.summary(args) : tool.description,
            item: before?.label,
            signal: ending.signal,
          }),
        ).catch(() => false),
        new Promise((resolve) => {
          timer = setTimeout(() => resolve(false), askMs);
        }),
      ]);
      clearTimeout(timer);
      ending.abort();
      if (answered !== true) return text(REFUSED, true);
      const after = await item();
      if (before?.key !== after?.key || before?.kind !== after?.kind) {
        return text('The open item changed while the person was asked, so nothing was changed. Ask again for the item now open.', true);
      }
    }
    try {
      const result = await tool.run(args);
      return text({ item: before === null ? null : { kind: before.kind, key: before.key, label: before.label }, ...asObject(result) });
    } catch (error) {
      return text(`${tool.name} failed: ${label(String(error?.message ?? error))}`, true);
    }
  }

  function asObject(result) {
    return result !== null && typeof result === 'object' && !Array.isArray(result) ? result : { result };
  }

  /** One MCP message of a session: its answer, or null for a notice. */
  async function serve(stream, message) {
    if (message === null || typeof message !== 'object' || Array.isArray(message)) return null;
    const { id, method, params } = message;
    if (id === undefined || typeof method !== 'string') return null;
    if (method === 'tools/list') return reply(id, listing());
    if (method === 'tools/call') return reply(id, await call(stream, params));
    return failure(id, -32601, 'no such method');
  }

  function open(message) {
    if (!STREAM.test(message.stream ?? '') || streams.has(message.stream) || streams.size >= MAX_STREAMS) return;
    streams.set(message.stream, {
      id: message.stream,
      agent: label(message.session?.agent) || 'An agent',
      where: whereOf(message.session),
      running: 0,
    });
  }

  function heard(message) {
    if (message === null || typeof message !== 'object') return;
    if (message.type === 'open') {
      open(message);
    } else if (message.type === 'close') {
      streams.delete(message.stream);
    } else if (message.type === 'mcp') {
      const stream = streams.get(message.stream);
      if (stream === undefined) return;
      const body = message.message;
      if (stream.running >= MAX_RUNNING) {
        void send({ type: 'mcp', stream: stream.id, message: failure(body?.id, -32000, 'too many calls at once') });
        return;
      }
      stream.running += 1;
      // Not waited for: asking the person takes minutes, and the agent's pings and other calls go on meanwhile.
      void (async () => {
        try {
          const answer = await serve(stream, body);
          if (answer !== null && streams.get(stream.id) === stream) await send({ type: 'mcp', stream: stream.id, message: answer });
        } catch {
          await send({ type: 'mcp', stream: stream.id, message: failure(body?.id, -32603, 'that failed') });
        } finally {
          stream.running -= 1;
        }
      })();
    }
  }

  /** Registers with the host. Resolves the session id, or rejects with the host's reason. */
  async function register() {
    const response = await post('open', { app });
    const body = await response.json().catch(() => ({}));
    if (!response.ok || typeof body.session !== 'string') throw new Error(label(body.why ?? body.said) || `the host answered ${response.status}`);
    return body.session;
  }

  /** Polls until the host says it has forgotten the session. A poll that fails is asked again for the same session. */
  async function poll() {
    let wait = POLL_RETRY_MIN_MS;
    let failing = false;
    while (!stopped) {
      let body;
      try {
        const response = await fetchImpl(`${TOOLS_PATH}/poll?${new URLSearchParams({ session, after: String(seen) })}`, {
          credentials: 'same-origin',
          headers: headersFor(key),
          cache: 'no-store',
        });
        // The host forgot this session (it restarted, or the pane was quiet too long): register again.
        if (response.status === 410 || response.status === 404) return;
        if (!response.ok) throw new Error(`the host answered ${response.status}`);
        body = await response.json();
      } catch {
        // The answer did not arrive, or the host is not there. The host keeps what it sent until a poll names it, so the session
        // is asked again, a little later each time; a host that came back without it says so with a 410.
        failing = true;
        onStatus('waiting');
        if (!stopped) await sleep(wait);
        wait = Math.min(wait * 2, RETRY_MAX_MS);
        continue;
      }
      if (failing) {
        failing = false;
        wait = POLL_RETRY_MIN_MS;
        onStatus('connected');
      }
      // The host numbers what it sends and keeps it until a poll names the number: an answer the browser dropped is sent again by
      // the next poll, and a message this pane already handled is not handled twice.
      const { first, messages } = body ?? {};
      if (!Number.isSafeInteger(first) || !Array.isArray(messages)) continue;
      messages.forEach((message, offset) => {
        const number = first + offset;
        if (number <= seen) return;
        seen = number;
        heard(message);
      });
    }
  }

  async function loop() {
    let wait = RETRY_MIN_MS;
    while (!stopped) {
      try {
        session = await register();
        streams = new Map();
        seen = 0;
        onStatus('connected');
        wait = RETRY_MIN_MS;
        await poll();
      } catch {
        // The host is not there or refused: try again, a little later each time.
      }
      session = null;
      for (const stream of streams.values()) stream.running = 0;
      streams = new Map();
      if (stopped) break;
      onStatus('waiting');
      await sleep(wait);
      wait = Math.min(wait * 2, RETRY_MAX_MS);
    }
    onStatus('stopped');
  }

  return {
    /** Starts offering the tools; keeps trying while the host is not there. */
    start() {
      running ??= loop();
      return running;
    },

    /** Stops offering them and tells the host, which takes the tools away from every session. */
    async stop() {
      stopped = true;
      const ended = session;
      session = null;
      if (ended !== null) {
        try {
          await post('close', { session: ended });
        } catch {
          // The host drops a pane that stops asking.
        }
      }
    },

    /** The sessions the host has open now: what the pane can show. */
    sessions() {
      return [...streams.values()].map(({ agent, where }) => ({ agent, where }));
    },

    /** Answers one stream's message as the pane would, for a test that has no host. */
    serve,
  };
}
