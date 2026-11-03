// The extension in a real browser pairing by PIN with a Noise bridge and running a turn over it, then running a turn with a
// WebCrypto bridge, each a real coqui-acp-bridge, and behind real nginx ending TLS when one is given. The person's window is
// the extension's own page, filled and clicked by script; the turn runs the extension's own dialing code in that page.
//
//   CANOA_E2E_BROWSER=path/to/chrome CANOA_E2E_BRIDGE=path/to/coqui-acp-bridge CANOA_E2E_AGENT=path/to/echo_agent \
//   [CANOA_E2E_NGINX=path/to/nginx] [CANOA_E2E_WSL=distribution] node e2e/noise.mjs
//
// With CANOA_E2E_WSL the bridge, the agent and nginx are Linux paths run in that WSL distribution, and the browser reaches
// them through WSL's forwarding of loopback ports.
import { spawn } from "node:child_process";
import { cpSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import readline from "node:readline";
import { fileURLToPath } from "node:url";

import { lastCall, Session } from "./cdp.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const extension = path.resolve(here, "..");
const { CANOA_E2E_BROWSER: browser, CANOA_E2E_BRIDGE: bridge, CANOA_E2E_AGENT: agent, CANOA_E2E_NGINX: nginx, CANOA_E2E_WSL: wsl } = process.env;
if (!browser || !bridge || !agent) throw new Error("set CANOA_E2E_BROWSER, CANOA_E2E_BRIDGE and CANOA_E2E_AGENT");
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

const work = mkdtempSync(path.join(tmpdir(), "canoa-noise-e2e-"));
const profile = path.join(work, "profile");
const unpacked = path.join(work, "extension");
cpSync(extension, unpacked, { recursive: true, filter: (from) => !/(node_modules|[/\\]e2e|[/\\]ui|\.test\.mjs)/.test(path.relative(extension, from) || "x") });

// The agent side, which says what it serves one JSON line at a time.
const script = path.join(here, "noise-side.sh");
const toWsl = (file) => file.replace(/^([A-Za-z]):[\\/]/, (_, drive) => `/mnt/${drive.toLowerCase()}/`).replaceAll("\\", "/");
const side = wsl
  ? spawn("wsl.exe", ["-d", wsl, "--", "bash", toWsl(script), bridge, agent, ...(nginx ? [nginx] : [])], { stdio: ["pipe", "pipe", "inherit"] })
  : spawn("bash", [script, bridge, agent, ...(nginx ? [nginx] : [])], { stdio: ["pipe", "pipe", "inherit"] });
const said = readline.createInterface({ input: side.stdout })[Symbol.asyncIterator]();
const hear = async () => JSON.parse((await said.next()).value);

const results = [];
async function check(name, run) {
  try {
    await run();
    results.push(`ok   ${name}`);
  } catch (error) {
    results.push(`FAIL ${name}: ${String(error.message).slice(0, 300)}`);
  }
  console.log(results.at(-1));
}

let chrome;
let exitCode = 1;
const watchdog = setTimeout(() => {
  console.log(`FAILED: the run took too long, at: ${lastCall()}`);
  chrome?.kill();
  side.kill();
  process.exit(1);
}, 240_000);
try {
  const noiseSide = await hear();
  const flags = noiseSide.spki ? [`--ignore-certificate-errors-spki-list=${noiseSide.spki}`] : [];
  chrome = spawn(browser, [
    "--headless=new", `--user-data-dir=${profile}`, "--remote-debugging-port=0", "--no-first-run", "--no-default-browser-check",
    `--load-extension=${unpacked}`, `--disable-extensions-except=${unpacked}`,
    "--disable-features=DisableLoadExtensionCommandLineSwitch", ...flags, "about:blank",
  ], { stdio: "ignore" });
  const debug = await waitFor("the browser", () => {
    try {
      return readFileSync(path.join(profile, "DevToolsActivePort"), "utf8").split("\n")[0];
    } catch {
      return null;
    }
  }, 30_000);
  const targets = async () => (await fetch(`http://127.0.0.1:${debug}/json/list`)).json();
  const worker = await waitFor("the extension's service worker", async () =>
    (await targets()).find((target) => target.type === "service_worker" && target.url.startsWith("chrome-extension://") && target.url.endsWith("/background.js")), 20_000);
  const id = new URL(worker.url).hostname;
  const opened = await (await fetch(`http://127.0.0.1:${debug}/json/new?${encodeURIComponent(`chrome-extension://${id}/acp-request.html?manage=1`)}`, { method: "PUT" })).json();
  const page = new Session(opened.webSocketDebuggerUrl);
  await waitFor("the agents window", async () => (await page.run("document.querySelectorAll('fieldset').length").catch(() => 0)) >= 2);
  console.log(`browser ${await page.run("navigator.userAgent")}`);

  // A turn on an agent the extension holds, by the extension's own dialing code, answering what the agent echoed.
  const turn = (label) => page.run(`(async () => {
    const { dialAgent } = await import("./acp-dial.js");
    const { agents } = await chrome.runtime.sendMessage({ acp: "manage" });
    const agent = agents.find((agent) => agent.label === ${JSON.stringify(label)});
    const link = await dialAgent(agent);
    const replies = [];
    let waiting;
    link.onMessage((text) => { replies.push(JSON.parse(text)); waiting?.(); });
    const answer = async (id) => {
      for (;;) {
        const found = replies.find((message) => message.id === id);
        if (found) return found;
        await new Promise((resolve) => { waiting = resolve; setTimeout(resolve, 15000); });
      }
    };
    await link.send(JSON.stringify({ jsonrpc: "2.0", id: 1, method: "initialize", params: { protocolVersion: 1, clientCapabilities: {} } }));
    await answer(1);
    await link.send(JSON.stringify({ jsonrpc: "2.0", id: 2, method: "session/new", params: { cwd: "/", mcpServers: [] } }));
    const { result } = await answer(2);
    await link.send(JSON.stringify({ jsonrpc: "2.0", id: 3, method: "session/prompt", params: { sessionId: result.sessionId, prompt: [{ type: "text", text: "hello from ${label}" }] } }));
    const done = await answer(3);
    link.close();
    const echoed = replies.filter((message) => message.method === "session/update").map((message) => message.params.update?.content?.text).join("");
    return { method: agent.method, stop: done.result?.stopReason, echoed };
  })()`);

  await check("the browser pairs by the PIN a Noise bridge shows, through the agents window", async () => {
    const fill = (index, value) => `(() => { const inputs = document.querySelectorAll("fieldset")[1].querySelectorAll("input"); inputs[${index}].value = ${JSON.stringify(value)}; })()`;
    await page.run(fill(0, "noise agent"));
    await page.run(fill(1, noiseSide.noiseUrl));
    await page.run(fill(2, noiseSide.pairUrl));
    await page.run(fill(3, noiseSide.pin));
    await page.run(`[...document.querySelectorAll("fieldset")[1].querySelectorAll("button")].find((b) => b.textContent === "Pair").click()`, { gesture: true });
    const stored = await waitFor("the paired agent", async () => {
      const { agents } = await page.run(`chrome.runtime.sendMessage({ acp: "manage" })`);
      return agents.find((agent) => agent.label === "noise agent");
    }, 60_000);
    if (stored.peer !== noiseSide.noisePeer) throw new Error(`stored ${stored.peer}, the bridge is ${noiseSide.noisePeer}`);
    if (stored.method !== "noise") throw new Error(`stored with method ${stored.method}`);
  });

  await check("a turn runs with the Noise bridge, proved by the paired key", async () => {
    const ran = await turn("noise agent");
    if (ran.method !== "noise" || ran.stop !== "end_turn" || !ran.echoed.includes("hello from noise agent")) throw new Error(JSON.stringify(ran));
  });

  await check("a turn runs with a WebCrypto bridge allowing this browser's WebCrypto key", async () => {
    const own = await page.run(`import("./acp-dial.js").then((dial) => dial.ownPeerId("webcrypto"))`);
    side.stdin.write(`allow ${own}\n`);
    const webcryptoSide = await hear();
    await page.run(`chrome.runtime.sendMessage({ acp: "add", agent: { label: "webcrypto agent", url: ${JSON.stringify(webcryptoSide.webcryptoUrl)}, peer: ${JSON.stringify(webcryptoSide.webcryptoPeer)}, method: "webcrypto" } })`);
    const ran = await turn("webcrypto agent");
    if (ran.method !== "webcrypto" || ran.stop !== "end_turn" || !ran.echoed.includes("hello from webcrypto agent")) throw new Error(JSON.stringify(ran));
  });

  await check("a Noise agent stored as WebCrypto is refused, and nothing tries the other method", async () => {
    await page.run(`chrome.runtime.sendMessage({ acp: "add", agent: { label: "mislabelled", url: ${JSON.stringify(noiseSide.noiseUrl)}, peer: ${JSON.stringify(noiseSide.noisePeer)}, method: "webcrypto" } })`);
    const refused = await turn("mislabelled").then(() => "served", (error) => error.message);
    if (refused === "served") throw new Error("a WebCrypto proof reached a Noise agent");
  });

  side.stdin.end();
  if (nginx) {
    await check("every connection went through nginx and was upgraded", async () => {
      const { access } = await hear();
      const lines = access.trim().split("\n");
      for (const location of ["/pair", "/noise", "/webcrypto"]) {
        if (!lines.some((line) => line.includes(`GET ${location}`) && line.includes('" 101 '))) throw new Error(`no upgrade of ${location}: ${access}`);
      }
      console.log(`nginx upgraded ${lines.filter((line) => line.includes('" 101 ')).length} connections`);
    });
  }
  exitCode = results.every((line) => line.startsWith("ok")) ? 0 : 1;
  page.close();
} catch (error) {
  console.log(`FAILED: ${error.message}`);
} finally {
  clearTimeout(watchdog);
  side.stdin.end();
  chrome?.kill();
  await sleep(1000);
  rmSync(work, { recursive: true, force: true, maxRetries: 10, retryDelay: 500 });
}
console.log(exitCode === 0 ? "PASSED" : "FAILED");
process.exit(exitCode);

async function waitFor(what, test, ms = 15_000) {
  const until = Date.now() + ms;
  for (;;) {
    const value = await test();
    if (value) return value;
    if (Date.now() > until) throw new Error(`timed out waiting for ${what}`);
    await sleep(200);
  }
}
