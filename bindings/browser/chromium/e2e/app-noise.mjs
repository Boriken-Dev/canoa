// The web build's peer code in a real browser: Canoa's own pairing and chat connection code, compiled from
// app/tool/peer_page.dart, on the web build's peer bridge (canoa_peer_bridge.mjs and its vendored methods), pairing by PIN with
// a Noise bridge and running a turn over it, then running a turn with a WebCrypto bridge. Each is a real coqui-acp-bridge, and
// behind real nginx ending TLS when one is given. Nothing of the extension runs.
//
//   dart compile js -O1 -o <dir>/page.js tool/peer_page.dart        (in app/)
//   CANOA_E2E_BROWSER=path/to/chrome CANOA_E2E_BRIDGE=path/to/coqui-acp-bridge CANOA_E2E_AGENT=path/to/echo_agent \
//   CANOA_E2E_APP_PAGE=<dir> [CANOA_E2E_WEB=app/build/web] [CANOA_E2E_NGINX=path/to/nginx] [CANOA_E2E_WSL=distribution] \
//   node e2e/app-noise.mjs
//
// CANOA_E2E_WEB is where canoa_peer_bridge.mjs and vendor/ are served from: a `flutter build web` output, or app/web unless
// set. With CANOA_E2E_WSL the bridge, the agent and nginx are Linux paths run in that WSL distribution, as in noise.mjs.
import { spawn } from "node:child_process";
import { createServer } from "node:http";
import { connect } from "node:net";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import readline from "node:readline";
import { fileURLToPath } from "node:url";

import { lastCall, Session } from "./cdp.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const {
  CANOA_E2E_BROWSER: browser, CANOA_E2E_BRIDGE: bridge, CANOA_E2E_AGENT: agent, CANOA_E2E_APP_PAGE: appPage,
  CANOA_E2E_NGINX: nginx, CANOA_E2E_WSL: wsl,
} = process.env;
if (!browser || !bridge || !agent || !appPage) throw new Error("set CANOA_E2E_BROWSER, CANOA_E2E_BRIDGE, CANOA_E2E_AGENT and CANOA_E2E_APP_PAGE");
const web = process.env.CANOA_E2E_WEB ?? path.resolve(here, "../../../../app/web");
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const work = mkdtempSync(path.join(tmpdir(), "canoa-app-noise-e2e-"));
const profile = path.join(work, "profile");

// The page: the web build's bridge as a module, then the compiled Dart, each from where it was built.
const types = { ".mjs": "text/javascript", ".js": "text/javascript", ".html": "text/html" };
const server = createServer((request, response) => {
  const url = new URL(request.url, "http://localhost");
  if (url.pathname === "/") {
    response.writeHead(200, { "content-type": "text/html" });
    response.end('<!doctype html><script type="module" src="/canoa_peer_bridge.mjs"></script><script src="/page.js" defer></script>');
    return;
  }
  const parts = url.pathname.split("/").filter((part) => part !== "" && part !== "..");
  const file = parts.length === 1 && parts[0] === "page.js" ? path.join(appPage, "page.js") : path.join(web, ...parts);
  try {
    const body = readFileSync(file);
    response.writeHead(200, { "content-type": types[path.extname(file)] ?? "application/octet-stream" });
    response.end(body);
  } catch {
    response.writeHead(404);
    response.end();
  }
});
await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
const origin = `http://127.0.0.1:${server.address().port}`;

const script = path.join(here, "noise-side.sh");
const toWsl = (file) => file.replace(/^([A-Za-z]):[\\/]/, (_, drive) => `/mnt/${drive.toLowerCase()}/`).replaceAll("\\", "/");
const side = wsl
  ? spawn("wsl.exe", ["-d", wsl, "--", "bash", toWsl(script), bridge, agent, ...(nginx ? [nginx] : [])], { stdio: ["pipe", "pipe", "inherit"] })
  : spawn("bash", [script, bridge, agent, ...(nginx ? [nginx] : [])], { stdio: ["pipe", "pipe", "inherit"] });
const said = readline.createInterface({ input: side.stdout })[Symbol.asyncIterator]();
const hear = async () => JSON.parse((await said.next()).value);
// WSL forwards a loopback port to Windows only some time after it starts listening, so wait until a connection gets through.
async function reachable(url) {
  const { hostname, port } = new URL(url);
  const until = Date.now() + 20_000;
  while (!(await new Promise((resolve) => {
    const socket = connect(Number(port), hostname, () => {
      socket.destroy();
      resolve(true);
    });
    socket.on("error", () => resolve(false));
    socket.setTimeout(2000, () => resolve(false));
  }))) {
    if (Date.now() > until) throw new Error(`${url} never became reachable`);
    await sleep(250);
  }
}

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
  await reachable(noiseSide.noiseUrl);
  // pair-accept lets in only this page's origin.
  side.stdin.write(`pair ${origin}\n`);
  const { pin } = await hear();
  await reachable(noiseSide.pairUrl);
  const flags = noiseSide.spki ? [`--ignore-certificate-errors-spki-list=${noiseSide.spki}`] : [];
  chrome = spawn(browser, [
    "--headless=new", `--user-data-dir=${profile}`, "--remote-debugging-port=0", "--no-first-run", "--no-default-browser-check",
    ...flags, "about:blank",
  ], { stdio: "ignore" });
  const debug = await waitFor("the browser", () => {
    try {
      return readFileSync(path.join(profile, "DevToolsActivePort"), "utf8").split("\n")[0];
    } catch {
      return null;
    }
  }, 30_000);
  const opened = await (await fetch(`http://127.0.0.1:${debug}/json/new?${encodeURIComponent(`${origin}/`)}`, { method: "PUT" })).json();
  const page = new Session(opened.webSocketDebuggerUrl);
  await waitFor("the page's bridge and code", () => page.run("Boolean(globalThis.canoaPeer && globalThis.canoaTurn)").catch(() => false));
  console.log(`browser ${await page.run("navigator.userAgent")}`);
  const call = (name, ...args) => page.run(`${name}(${args.map((arg) => JSON.stringify(arg)).join(", ")})`);
  const turn = async (url, peer, method, text) => JSON.parse(await call("canoaTurn", url, peer, method, text));

  await check("this browser runs Noise, so nothing selects another method", async () => {
    const why = await page.run("canoaPeer.unreachable('noise')");
    if (why !== null) throw new Error(why);
  });

  await check("the app pairs by the PIN a Noise bridge shows, and learns that bridge's id", async () => {
    const peer = await call("canoaPair", noiseSide.pairUrl, pin);
    if (peer !== noiseSide.noisePeer) throw new Error(`paired with ${peer}, the bridge is ${noiseSide.noisePeer}`);
  });

  await check("a turn runs with the Noise bridge, proved by the paired key", async () => {
    const ran = await turn(noiseSide.noiseUrl, noiseSide.noisePeer, "noise", "hello from noise");
    if (ran.peer !== noiseSide.noisePeer || ran.echoed !== "echo: hello from noise") throw new Error(JSON.stringify(ran));
  });

  await check("a turn runs with a WebCrypto bridge allowing this browser's WebCrypto key, which is not its Noise key", async () => {
    const own = await call("canoaOwnId", "webcrypto");
    if (own === (await call("canoaOwnId", "noise"))) throw new Error("one key for both methods");
    side.stdin.write(`allow ${own}\n`);
    const webcryptoSide = await hear();
    await reachable(webcryptoSide.webcryptoUrl);
    const ran = await turn(webcryptoSide.webcryptoUrl, webcryptoSide.webcryptoPeer, "webcrypto", "hello from webcrypto");
    if (ran.peer !== webcryptoSide.webcryptoPeer || ran.echoed !== "echo: hello from webcrypto") throw new Error(JSON.stringify(ran));
  });

  await check("a Noise agent named as WebCrypto is refused, and nothing tries the other method", async () => {
    const refused = await turn(noiseSide.noiseUrl, noiseSide.noisePeer, "webcrypto", "x").then(() => "served", (error) => error.message);
    if (refused === "served") throw new Error("a WebCrypto proof reached a Noise agent");
  });

  side.stdin.end();
  if (nginx) {
    await check("every connection went through nginx and was upgraded", async () => {
      const { access } = await hear();
      const lines = access.trim().split("\n");
      for (const location of ["/pair", "/noise", "/webcrypto"]) {
        if (!lines.some((line) => line.includes(`GET ${location}`) && line.includes('" 101 '))) throw new Error(`no upgrade of ${location}`);
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
  server.close();
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
