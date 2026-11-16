// A web page using the agents of the Canoa on this computer through document.acp, in a real browser: the page finds the provider,
// asks, the person's window (clicked by script) grants the host's scripted entry, and the page opens a session that names an https
// MCP server with a bearer and runs a tool and a prompt on it through the agent.
//
//   CANOA_E2E_BROWSER=path/to/msedge.exe CANOA_E2E_HOST=path/to/canoa CANOA_E2E_AGENT=path/to/stdio_agent \
//     [CANOA_E2E_MCP=path/to/mcp_http] [CANOA_E2E_NATIVE_BROWSER=edge|chrome|chromium] node e2e/page_local.mjs
//
// The browser gets a throwaway profile and `COQUI_AGENTIC_PEER_HOME` names a home of the run's own. Registering uses the user's
// registry keys (Windows) for `dev.boriken.canoa`: a run refuses to start when that is already registered. The stand-in MCP
// server (`mcp_http`) makes its certificate for the run and writes it into the run's folder; nothing is committed. Needs no
// `npm install`.
import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import { randomBytes } from "node:crypto";
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { findCanoaWorker, lastCall, Session } from "./cdp.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const extension = path.resolve(here, "..");
const browser = process.env.CANOA_E2E_BROWSER;
// Absolute: what Start runs is recorded, and a program the browser's host starts from its own folder must still be found.
const absolute = (file) => (file ? path.resolve(file) : file);
const canoa = absolute(process.env.CANOA_E2E_HOST);
const agent = absolute(process.env.CANOA_E2E_AGENT);
if (!browser || !canoa || !agent) throw new Error("set CANOA_E2E_BROWSER, CANOA_E2E_HOST and CANOA_E2E_AGENT");
const mcp = absolute(process.env.CANOA_E2E_MCP) ?? path.join(path.dirname(agent), `mcp_http${path.extname(agent)}`);
const NATIVE_BROWSER = process.env.CANOA_E2E_NATIVE_BROWSER ?? "edge";
const HOST_NAME = "dev.boriken.canoa";
const KEYS = { edge: "Microsoft\\Edge", chrome: "Google\\Chrome", chromium: "Chromium" };

if (process.platform === "win32") {
  for (const key of Object.values(KEYS)) {
    let present = true;
    try {
      execFileSync("reg", ["query", `HKCU\\Software\\${key}\\NativeMessagingHosts\\${HOST_NAME}`], { stdio: "ignore" });
    } catch {
      present = false;
    }
    if (present) throw new Error(`${HOST_NAME} is already registered under ${key}: remove it first (canoa native uninstall)`);
  }
}

const work = mkdtempSync(path.join(tmpdir(), "canoa-e2e-page-"));
const home = path.join(work, "home");
const profile = path.join(work, "profile");
const unpacked = path.join(work, "extension");
const config = path.join(work, "canoa.cfg");
const documents = path.join(work, "documents");
const ca = path.join(work, "mcp-ca.pem");
const mcpLog = path.join(work, "mcp.log");
mkdirSync(home);
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const quoted = (text) => JSON.stringify(text);
// What the page gives the agent's MCP server as its bearer: it must reach that server and nothing else.
const CANARY = `canary-${randomBytes(12).toString("hex")}`;

// Two sites on this machine: a page is told apart by its origin, port included.
const servePage = async () => {
  const server = createServer((request, response) => {
    response.writeHead(200, { "content-type": "text/html" });
    response.end(`<!doctype html><title>a page</title><p>page</p><script>
      window.found = null;
      window.addEventListener("acp:ready", (event) => { window.found = event.detail.acp; });
      window.dispatchEvent(new Event("acp:request"));
      window.ready = true;
    </script>`);
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  return { server, url: `http://localhost:${server.address().port}/`, origin: `http://localhost:${server.address().port}` };
};
const siteA = await servePage();
const siteB = await servePage();

let output = "";
const keep = (chunk) => {
  output += chunk;
};

let mcpProcess;
let mcpOrigin;
async function startMcp() {
  mcpProcess = spawn(mcp, [`--log=${mcpLog}`, `--ca=${ca}`], { stdio: ["ignore", "pipe", "pipe"] });
  mcpProcess.stderr.on("data", keep);
  let said = "";
  mcpProcess.stdout.on("data", (chunk) => {
    said += chunk;
  });
  await waitFor("the stand-in MCP server", () => said.includes("\n"), 20_000);
  mcpOrigin = `https://127.0.0.1:${JSON.parse(said.split("\n")[0]).port}`;
}
const mcpSeen = () => (existsSync(mcpLog) ? readFileSync(mcpLog, "utf8").split("\n").filter(Boolean).map((line) => JSON.parse(line)) : []);

writeFileSync(config, [
  "[host.agents.scripted]",
  `program = ${quoted(agent)}`,
  `args = ["--scripted", "--mcp-http", "--modes=default,bypassPermissions", ${quoted(`--mcp-ca=${ca}`)}]`,
  "title = \"Scripted\"",
  "",
].join("\n"));

cpSync(extension, unpacked, { recursive: true, filter: (from) => !/(node_modules|[/\\]e2e|[/\\]ui|\.test\.mjs)/.test(path.relative(extension, from) || "x") });
const manifest = JSON.parse(readFileSync(path.join(unpacked, "manifest.json"), "utf8"));
delete manifest.side_panel;
// What the person's Chrome prompt grants when they let a site ask: the scripts may then run in the pages this run serves.
manifest.host_permissions = ["http://localhost/*"];
writeFileSync(path.join(unpacked, "manifest.json"), JSON.stringify(manifest));

const env = { ...process.env, COQUI_AGENTIC_PEER_HOME: home, XDG_DOCUMENTS_DIR: documents };
const run = (args, options = {}) => execFileSync(canoa, args, { encoding: "utf8", env, ...options });
const serveArgs = ["serve", "--config", config, "--method", "noise"];

let hostProcess;
async function startHost() {
  const child = spawn(canoa, serveArgs, { env, stdio: ["pipe", "pipe", "pipe"] });
  let said = "";
  child.stdout.on("data", (chunk) => {
    said += chunk;
    keep(chunk);
  });
  child.stderr.on("data", keep);
  await waitFor("the host to be ready", () => said.includes('"ready"'), 30_000);
  hostProcess = child;
}

let debug;
const targets = async () => (await fetch(`http://127.0.0.1:${debug}/json/list`)).json();
async function waitFor(what, test, ms = 15_000) {
  const until = Date.now() + ms;
  for (;;) {
    const value = await test();
    if (value) return value;
    if (Date.now() > until) throw new Error(`timed out waiting for ${what}`);
    await sleep(100);
  }
}
const results = [];
async function check(name, body) {
  try {
    await body();
    results.push(`ok   ${name}`);
  } catch (error) {
    results.push(`FAIL ${name}: ${String(error.message).slice(0, 600)}`);
  }
  console.log(results.at(-1));
}

/** Every file under `dir` that holds `needle`, as UTF-8 or as UTF-16. */
function filesHolding(dir, needles) {
  const found = [];
  const walk = (folder) => {
    let names = [];
    try {
      names = readdirSync(folder);
    } catch {
      return;
    }
    for (const name of names) {
      const file = path.join(folder, name);
      let stat;
      try {
        stat = statSync(file);
      } catch {
        continue;
      }
      if (stat.isDirectory()) {
        walk(file);
      } else if (stat.size < 256 * 1024 * 1024) {
        try {
          const data = readFileSync(file);
          if (needles.some((needle) => data.includes(needle))) found.push(file);
        } catch {
          // A file the browser still holds open.
        }
      }
    }
  };
  walk(dir);
  return found;
}

let exitCode = 1;
let registered = false;
let chrome;
const watchdog = setTimeout(() => {
  console.log(`FAILED: the run took too long, at: ${lastCall()}`);
  chrome?.kill();
  hostProcess?.kill();
  mcpProcess?.kill();
  process.exit(1);
}, 420_000);
try {
  await startMcp();
  await startHost();
  run(["native", "install", "--browser", NATIVE_BROWSER, "--launch", canoa, "--", ...serveArgs]);
  registered = true;

  chrome = spawn(browser, [
    "--headless=new", `--user-data-dir=${profile}`, "--remote-debugging-port=0", "--no-first-run", "--no-default-browser-check",
    `--load-extension=${unpacked}`, `--disable-extensions-except=${unpacked}`,
    "--disable-features=DisableLoadExtensionCommandLineSwitch", "about:blank",
  ], { stdio: "ignore", env });
  debug = await waitFor("the browser", () => {
    try {
      return readFileSync(path.join(profile, "DevToolsActivePort"), "utf8").split("\n")[0];
    } catch {
      return null;
    }
  }, 30_000);
  const worker = await waitFor("the extension's service worker", async () => findCanoaWorker(await targets()), 20_000);
  const sw = new Session(worker.webSocketDebuggerUrl);
  await waitFor("the extension's APIs", async () => (await sw.run("typeof chrome")) === "object");
  const id = new URL(worker.url).hostname;
  const openTab = async (url) => {
    const made = await (await fetch(`http://127.0.0.1:${debug}/json/new?${encodeURIComponent(url)}`, { method: "PUT" })).json();
    const session = new Session(made.webSocketDebuggerUrl);
    await session.call("Runtime.enable");
    return session;
  };
  const options = await openTab(`chrome-extension://${id}/permissions.html`);
  const ask = (type, extra = {}) => options.run(`chrome.runtime.sendMessage(${JSON.stringify({ type, ...extra })})`);
  const hostPeer = run(["id", "--method", "noise"]).trim();

  // The extension is paired with the host first, by the PIN the host shows, as any use of the host starts.
  const pairing = spawn(canoa, ["pair", "--native", "--method", "noise"], { env, stdio: ["pipe", "pipe", "ignore"] });
  let pinLine = "";
  pairing.stdout.on("data", (chunk) => {
    pinLine += chunk;
  });
  const shown = JSON.parse(await waitFor("the PIN", () => (pinLine.includes("\n") ? pinLine.split("\n")[0] : null), 15_000));
  const paired = await ask("canoa.host.pair", { pin: shown.pin });
  assert.equal(paired.paired, true, JSON.stringify(paired));

  // What the person's "let this site ask for my agents" does: the two scripts are registered for the site.
  for (const site of [siteA, siteB]) {
    await sw.run(`chrome.scripting.registerContentScripts([
      { id: "m-${new URL(site.url).port}", matches: ["${site.origin}/*"], js: ["acp-page.js"], runAt: "document_start", world: "MAIN" },
      { id: "i-${new URL(site.url).port}", matches: ["${site.origin}/*"], js: ["acp-content.js"], runAt: "document_start", world: "ISOLATED" },
    ]).then(() => true)`);
  }
  const page = await openTab(siteA.url);
  const other = await openTab(siteB.url);

  const answered = new Set();
  const windowOfPerson = async () => {
    const target = await waitFor("the person's window", async () => (await targets()).find((item) => item.url.includes("acp-request.html?ask=") && !answered.has(item.id)));
    answered.add(target.id);
    const view = new Session(target.webSocketDebuggerUrl);
    await waitFor("its contents", () => view.run("(document.querySelector('#main')?.childElementCount ?? 0) > 0"));
    // It has asked the host what it is doing, and its buttons answer only after a moment.
    await waitFor("what it says of Canoa", () => view.run("![...document.querySelectorAll('p')].some((p) => p.textContent.startsWith('Looking for Canoa'))"));
    await waitFor("its buttons", () => view.run("![...document.querySelectorAll('button, input[type=checkbox]')].some((control) => control.disabled)"));
    return view;
  };
  const click = (view, startsWith) => view.run(`[...document.querySelectorAll('button')].find((b) => b.textContent.startsWith(${quoted(startsWith)})).click()`, { gesture: true }).catch(() => {});
  const requested = (site, args) => site.run(`window.asked = document.acp.request(${JSON.stringify(args)}).then(
      (agents) => { window.agent = agents[0]; return agents.length; }, (e) => e.name); true`, { gesture: true });
  const scriptedRow = (view) => view.run(`(() => {
    const row = [...document.querySelectorAll('label.pick')].find((label) => label.textContent.startsWith('Scripted'));
    return row ? { local: row.querySelector('input').dataset.local === 'yes', ticked: row.querySelector('input').checked, text: row.textContent } : null;
  })()`);

  await check("detect: the page hears acp:ready, finds Canoa among the providers, and has no agents", async () => {
    await waitFor("the provider", () => page.run("window.found === document.acp && typeof document.acp === 'object'"));
    assert.deepEqual(await page.run("JSON.stringify(document.acp.providers.map((p) => [p.uuid, p.name, p.rdns]))"), JSON.stringify([["dev.boriken.canoa", "Canoa", "dev.boriken.canoa"]]));
    assert.equal(await page.run("document.acp.agents().then((a) => a.length)"), 0);
    assert.equal(await page.run("document.acp.connect({ id: 'x', provider: 'dev.boriken.canoa' }).then(() => 'connected', (e) => e.name)"), "NotAllowedError");
  });

  await check("window: the person's window lists the host's entry as Canoa on this computer, never ticked, with the server it may name", async () => {
    await requested(page, { purpose: "testing", acpVersions: [1], mcpOrigins: [mcpOrigin], permissions: false, peers: [hostPeer] });
    const view = await windowOfPerson();
    const row = await waitFor("the host's entry", () => scriptedRow(view));
    assert.equal(row.local, true, JSON.stringify(row));
    assert.equal(row.ticked, false, "a page naming the host's key does not tick the host's entries");
    assert.match(row.text, /Canoa on this computer/);
    const text = await view.run("document.querySelector('#main').textContent");
    assert.ok(text.includes(mcpOrigin), "the person is told which server the page may name");
    assert.match(text, /makes its own/, "no directory is asked for the host's entry");
    assert.ok(text.includes(`${mcpOrigin} (this computer or a private network)`), "an origin on this computer is marked as one");
    assert.match(text, /tell these agents what to do/);
    assert.match(text, /can read your files/);
    assert.match(text, /does not apply to agents on this computer/);
    await click(view, "Refuse");
    assert.equal(await page.run("window.asked"), "NotAllowedError");
    assert.equal(await page.run("document.acp.agents().then((a) => a.length)"), 0);
  });

  await check("grant: the person ticks the host's entry and allows it, and the page is told only that one", async () => {
    await requested(page, { purpose: "testing", acpVersions: [1], mcpOrigins: [mcpOrigin], permissions: false });
    const view = await windowOfPerson();
    await waitFor("the host's entry", () => scriptedRow(view));
    await waitFor("it to settle", () => view.run("![...document.querySelectorAll('button, input[type=checkbox]')].some((control) => control.disabled)"));
    // The person also ticks "this site can approve", which cannot reach the host's agents.
    await view.run(`(() => {
      [...document.querySelectorAll('label.pick')].find((label) => label.textContent.startsWith('Scripted')).querySelector('input').click();
      [...document.querySelectorAll('label.pick')].find((label) => label.textContent.includes('approve what the agent does')).querySelector('input').click();
    })()`);
    await click(view, "Allow");
    assert.equal(await page.run("window.asked"), 1);
    const said = JSON.parse(await page.run("JSON.stringify({ label: window.agent.label, source: window.agent.source, identity: window.agent.identity, scope: window.agent.scope, keys: Object.keys(window.agent).sort() })"));
    assert.equal(said.label, "Scripted");
    assert.equal(said.source, "local");
    assert.match(said.identity.peer, /^sha256:[0-9a-f]{64}$/);
    assert.notEqual(said.identity.peer, hostPeer, "the page is told a name of its own for the host's key");
    assert.equal(said.identity.verified, true);
    assert.deepEqual(said.scope.mcpOrigins, [mcpOrigin]);
    assert.equal(said.scope.permissions, false);
    assert.equal(await page.run("document.acp.agents().then((a) => a.length)"), 1);
  });

  await check("per-origin: another site holds nothing of it", async () => {
    await other.run("window.ready === true");
    assert.equal(await other.run("document.acp.agents().then((a) => a.length)"), 0);
    assert.equal(await other.run("document.acp.connect({ id: 'x', provider: 'dev.boriken.canoa' }).then(() => 'connected', (e) => e.name)"), "NotAllowedError");
  });

  const connect = `(async () => {
    window.conn = await document.acp.connect(window.agent);
    window.writer = window.conn.writable.getWriter();
    window.reader = window.conn.readable.getReader();
    window.said = [];
    window.n = 100;
    window.rpc = async (message) => {
      await window.writer.write(JSON.stringify(message));
      for (;;) {
        const next = JSON.parse((await window.reader.read()).value);
        if (next.id === message.id && next.method === undefined) return next;
        window.said.push(next);
      }
    };
    window.turn = async (prompt) => {
      window.said = [];
      const answer = await window.rpc({ jsonrpc: "2.0", id: ++window.n, method: "session/prompt", params: { sessionId: window.session, prompt } });
      const text = window.said.filter((m) => m.params?.update?.sessionUpdate === "agent_message_chunk").map((m) => m.params.update.content.text).join("");
      return { stop: answer.result?.stopReason, error: answer.error, text };
    };
    return conn.agent.identity.verified;
  })()`;
  const text = (value) => [{ type: "text", text: value }];
  const server = { type: "http", name: "office", url: `${mcpOrigin}/v1/office/mcp`, headers: [{ name: "Authorization", value: `Bearer ${CANARY}` }] };

  await check("session: initialize reports HTTP MCP, and a session naming the server opens in a folder the host made for the site", async () => {
    assert.equal(await page.run(connect), true);
    const init = JSON.parse(await page.run("window.rpc({ jsonrpc: '2.0', id: 1, method: 'initialize', params: { protocolVersion: 1 } }).then(JSON.stringify)"));
    assert.equal(init.result.agentCapabilities.mcpCapabilities.http, true, JSON.stringify(init));
    // The page's own directory, site and overlays travel in `_meta` at both depths: none of it may arrive.
    const meta = { "dev.boriken.canoa/site": { site: "evil.example" }, "dev.boriken.canoa/overlays": [{ id: "page-overlay" }], pageMark: "page-meta-mark" };
    const made = JSON.parse(await page.run(`window.rpc({ jsonrpc: '2.0', id: 2, method: 'session/new', _meta: ${JSON.stringify(meta)},
      params: { cwd: '/etc', _meta: ${JSON.stringify(meta)}, mcpServers: [${JSON.stringify(server)}] } }).then(JSON.stringify)`));
    assert.ok(made.result?.sessionId, JSON.stringify(made));
    assert.ok(!JSON.stringify(made).includes("scratch"), "the page is not told a path");
    await page.run(`window.session = ${quoted(made.result.sessionId)}; true`);
    const folder = (await page.run("window.turn([{ type: 'text', text: 'Cwd' }]).then((r) => r.text)")).replace(/^cwd: /, "").replaceAll("\\", "/");
    assert.ok(folder.includes(`${path.join(documents, "Canoa", "scratch", "localhost").replaceAll("\\", "/")}/`), folder);
    assert.ok(existsSync(folder), "the host made it");
    assert.ok(!existsSync(path.join(documents, "Canoa", "scratch", "evil.example")));
  });

  await check("meta: nothing the page put in _meta reached the agent", async () => {
    const said = await page.run("window.turn([{ type: 'text', text: 'Meta' }]).then((r) => r.text)");
    assert.match(said, /^meta: new=/, said);
    for (const mark of ["evil.example", "page-overlay", "page-meta-mark", "overlays"]) assert.ok(!said.includes(mark), `${mark} reached the agent: ${said}`);
  });

  await check("context and tools: a text block leads the first prompt, and the agent calls the page's server's tool and prompt with the bearer", async () => {
    const context = await page.run(`window.turn(${JSON.stringify([{ type: "text", text: "Context from the page: the person is looking at a mission." }])}).then((r) => r.stop)`);
    assert.equal(context, "end_turn");
    const tool = await page.run("window.turn([{ type: 'text', text: 'CallOn office marker {}' }]).then((r) => r.text)");
    assert.equal(tool, "the marker");
    const skill = await page.run("window.turn([{ type: 'text', text: 'PromptOn office skill' }]).then((r) => r.text)");
    assert.equal(skill, "the skill");
    const seen = mcpSeen();
    assert.deepEqual(seen.map((entry) => entry.method), ["tools/call", "prompts/get"], JSON.stringify(seen));
    assert.ok(seen.every((entry) => entry.authorization === `Bearer ${CANARY}`), "the bearer reached the server the page named");
  });

  await check("refused: a stdio entry, an http entry and a server of another origin are -32602, and the agent hears of none", async () => {
    const refuse = async (entry) => JSON.parse(await page.run(`window.rpc({ jsonrpc: '2.0', id: ++window.n, method: 'session/new', params: { cwd: '/', mcpServers: [${JSON.stringify(entry)}] } }).then(JSON.stringify)`));
    for (const entry of [
      { name: "x", command: "node", args: ["-e", "1"] },
      { type: "http", name: "x", url: "http://127.0.0.1:1/mcp", headers: [] },
      { type: "http", name: "x", url: "https://other.example/mcp", headers: [] },
    ]) {
      const answer = await refuse(entry);
      assert.equal(answer.error?.code, -32602, JSON.stringify(answer));
    }
    assert.deepEqual(mcpSeen().length, 2);
  });

  await check("scope: an extension method is not found", async () => {
    const refused = async (method, params) => JSON.parse(await page.run(`window.rpc({ jsonrpc: '2.0', id: ++window.n, method: ${quoted(method)}, params: ${JSON.stringify(params)} }).then(JSON.stringify)`));
    assert.equal((await refused("_coqui_acp/usage", {})).error?.code, -32601);
  });

  // The page asks for a mode that does not ask: the person is asked in the extension's window, never the page, and decides for this
  // session alone.
  const changeMode = async (answer) => {
    const session = await page.run("window.session");
    await page.run(`window.changed = window.rpc({ jsonrpc: '2.0', id: ++window.n, method: 'session/set_config_option', params: { sessionId: ${quoted(session)}, configId: 'mode', value: 'bypassPermissions' } }).then(JSON.stringify); true`);
    const view = await windowOfPerson();
    const shown = await view.run("document.querySelector('#main').textContent");
    await click(view, answer);
    return { shown, answered: JSON.parse(await page.run("window.changed")) };
  };
  // A Deny ends a link's asking, so each check starts on a connection of its own that reopens the same session.
  const freshLink = async () => {
    const session = await page.run("window.session");
    await page.run("window.conn.close(); true");
    await page.run(connect);
    await page.run("window.rpc({ jsonrpc: '2.0', id: 1, method: 'initialize', params: { protocolVersion: 1 } }).then(() => true)");
    await page.run(`window.rpc({ jsonrpc: '2.0', id: 2, method: 'session/resume', params: { sessionId: ${quoted(session)}, mcpServers: [] } }).then(() => true)`);
    await page.run(`window.session = ${quoted(session)}; true`);
  };
  await check("mode: a page's request for bypass names the option and the value and warns, and Deny refuses it", async () => {
    await freshLink();
    const { shown, answered: reply } = await changeMode("Deny");
    assert.match(shown, /localhost wants to change Scripted's mode to bypassPermissions/);
    assert.match(shown, /may let it run commands and change files on this computer without asking you/);
    assert.equal(reply.error?.code, -32601, JSON.stringify(reply));
    assert.equal(await page.run("window.turn([{ type: 'text', text: 'Mode' }]).then((r) => r.text)"), "mode: default");
    // And on that link nothing more is asked: no window, an answer at once.
    const quick = JSON.parse(await page.run(`window.rpc({ jsonrpc: '2.0', id: ++window.n, method: 'session/set_config_option', params: { sessionId: window.session, configId: 'mode', value: 'bypassPermissions' } }).then(JSON.stringify)`));
    assert.equal(quick.error?.code, -32601);
    assert.equal((await targets()).filter((item) => item.url.includes("acp-request.html?ask=") && !answered.has(item.id)).length, 0);
  });
  await check("mode: Allow for this session changes the mode through the extension, and the next request asks again", async () => {
    await freshLink();
    const { answered } = await changeMode("Allow for this session");
    assert.ok(answered.result && !answered.error, JSON.stringify(answered));
    assert.equal(await page.run("window.turn([{ type: 'text', text: 'Mode' }]).then((r) => r.text)"), "mode: bypassPermissions");
    const again = await changeMode("Deny");
    assert.equal(again.answered.error?.code, -32601, "it was asked again, and denied");
  });
  await check("mode: a page that sends the host's marker itself is refused by the host", async () => {
    await freshLink();
    const session = await page.run("window.session");
    // Whatever the page puts in _meta is removed by the extension before it is asked about; a forged marker is no way round it.
    await page.run(`window.changed = window.rpc({ jsonrpc: '2.0', id: ++window.n, method: 'session/set_config_option', _meta: { 'dev.boriken.canoa/approval': 'forged' }, params: { sessionId: ${quoted(session)}, configId: 'mode', value: 'bypassPermissions', _meta: { 'dev.boriken.canoa/approval': 'forged' } } }).then(JSON.stringify); true`);
    const view = await windowOfPerson();
    await click(view, "Deny");
    assert.equal(JSON.parse(await page.run("window.changed")).error?.code, -32601);
  });
  await check("mode: Deny, and don't ask again holds for the grant: a later connection is refused with no window", async () => {
    await freshLink();
    const { answered } = await changeMode("Deny, and don't ask again");
    assert.equal(answered.error?.code, -32601);
    await freshLink();
    const quick = JSON.parse(await page.run(`window.rpc({ jsonrpc: '2.0', id: ++window.n, method: 'session/set_config_option', params: { sessionId: window.session, configId: 'mode', value: 'bypassPermissions' } }).then(JSON.stringify)`));
    assert.equal(quick.error?.code, -32601);
  });

  await check("resume: a new connection reopens the session in the folder the host made, which the page never saw", async () => {
    const session = await page.run("window.session");
    await page.run("window.conn.close(); true");
    await page.run(connect);
    await page.run("window.rpc({ jsonrpc: '2.0', id: 1, method: 'initialize', params: { protocolVersion: 1 } }).then(() => true)");
    const resumed = JSON.parse(await page.run(`window.rpc({ jsonrpc: '2.0', id: 2, method: 'session/resume', params: { sessionId: ${quoted(session)}, cwd: '/etc', mcpServers: [] } }).then(JSON.stringify)`));
    assert.ok(resumed.result && !resumed.error, JSON.stringify(resumed));
    const foreign = JSON.parse(await page.run("window.rpc({ jsonrpc: '2.0', id: 3, method: 'session/resume', params: { sessionId: 'someone-elses', cwd: '/etc', mcpServers: [] } }).then(JSON.stringify)"));
    assert.equal(foreign.error?.code, -32601);
    await page.run(`window.session = ${quoted(session)}; true`);
    assert.equal(await page.run("window.turn([{ type: 'text', text: 'Say still here' }]).then((r) => r.text)"), "still here");
  });

  await check("stopped: with the host stopped the grant stays, connecting is a network error, and the window offers Start", async () => {
    await page.run("window.conn.close(); true");
    hostProcess.kill();
    await waitFor("the host to end", () => hostProcess.exitCode !== null || hostProcess.signalCode !== null, 15_000);
    assert.equal(await page.run("document.acp.agents().then((a) => a.length)"), 1);
    assert.equal(await page.run("document.acp.connect(window.agent).then(() => 'connected', (e) => e.name)"), "NetworkError");
    await requested(page, { purpose: "testing again" });
    const view = await windowOfPerson();
    await waitFor("it to say so", () => view.run("document.querySelector('#main').textContent.includes('is not running')"));
    assert.equal(await scriptedRow(view), null, "no entry of a stopped host can be ticked");
    assert.equal(JSON.parse(run(["native", "status"])).running, false, "the host is stopped");
    await waitFor("its buttons", () => view.run("![...document.querySelectorAll('button')].some((button) => button.disabled)"));
    await click(view, "Start Canoa");
    await waitFor("Canoa to come up", () => JSON.parse(run(["native", "status"])).running, 45_000).catch(async (error) => {
      throw new Error(`${error.message}; the window says: ${await view.run("document.querySelector('#main').textContent.slice(0, 600)")}; the host printed: ${output.slice(-400)}`);
    });
    await waitFor("its entry to show", () => scriptedRow(view), 45_000);
    assert.equal(JSON.parse(run(["native", "status"])).running, true, "Start brought the host up");
    const after = JSON.parse(run(["native", "status"])).holder;
    hostProcess = { exitCode: null, kill: () => {
      try { process.kill(after.pid); } catch { /* gone */ }
    } };
    await waitFor("its buttons", () => view.run("![...document.querySelectorAll('button')].some((button) => button.disabled)"));
    await click(view, "Refuse");
    assert.equal(await page.run("window.asked"), "NotAllowedError");
  });

  await check("again: with the host back, the same grant connects and the session resumes", async () => {
    assert.equal(await page.run(connect), true);
    await page.run("window.rpc({ jsonrpc: '2.0', id: 1, method: 'initialize', params: { protocolVersion: 1 } }).then(() => true)");
    const opened = JSON.parse(await page.run(`window.rpc({ jsonrpc: '2.0', id: 2, method: 'session/new', params: { cwd: '/', mcpServers: [] } }).then(JSON.stringify)`));
    assert.ok(opened.result?.sessionId, JSON.stringify(opened));
    await page.run("window.conn.close(); true");
  });

  chrome.kill();
  hostProcess.kill();
  mcpProcess.kill();
  await sleep(1500);

  await check("canary: the page's bearer is in no file of the host's home, its scratch folders or the browser's profile, nor in anything the host printed", async () => {
    const needles = [Buffer.from(CANARY, "utf8"), Buffer.from(CANARY, "utf16le")];
    for (const folder of [home, documents, profile]) assert.deepEqual(filesHolding(folder, needles), [], folder);
    assert.ok(!output.includes(CANARY), "the host printed the bearer");
    assert.ok(mcpSeen().length >= 2, "the server did see it, which is what the bearer is for");
  });

  exitCode = results.some((line) => line.startsWith("FAIL")) ? 1 : 0;
} catch (error) {
  console.log(`FAILED: ${error.stack ?? error.message}`);
} finally {
  clearTimeout(watchdog);
  siteA.server.close();
  siteB.server.close();
  chrome?.kill();
  hostProcess?.kill();
  mcpProcess?.kill();
  if (registered) {
    try {
      run(["native", "uninstall"]);
    } catch {
      // Nothing left to remove.
    }
  }
  await sleep(500);
  try {
    rmSync(work, { recursive: true, force: true, maxRetries: 15, retryDelay: 400 });
  } catch (error) {
    console.log(`left ${work}: ${error.code}`);
  }
}
process.exit(exitCode);
