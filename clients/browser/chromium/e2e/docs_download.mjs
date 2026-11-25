// The documentation's Download button in a real browser: which file the main button offers for each platform the browser says it is,
// and what a site built with no release says.
//
//   CANOA_E2E_BROWSER=path/to/edge CANOA_E2E_DOCS_RELEASE=<site built from a release> CANOA_E2E_DOCS_NONE=<site built with none> node e2e/docs_download.mjs
//
// A site is `mkdocs build` output; the first is built with CANOA_DOCS_RELEASE naming a release JSON that holds, for each of
// Windows x64 and arm64 and Linux x64 and arm64, a file named after it (`tools/test_docs_download.py` lists the names).
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { readFileSync, mkdtempSync, rmSync, existsSync } from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import path from "node:path";

import { Session } from "./cdp.mjs";

const browser = process.env.CANOA_E2E_BROWSER;
const withRelease = process.env.CANOA_E2E_DOCS_RELEASE;
const none = process.env.CANOA_E2E_DOCS_NONE;
if (!browser || !withRelease || !none) throw new Error("set CANOA_E2E_BROWSER, CANOA_E2E_DOCS_RELEASE and CANOA_E2E_DOCS_NONE");

const types = { ".html": "text/html", ".css": "text/css", ".js": "text/javascript", ".png": "image/png", ".svg": "image/svg+xml", ".json": "application/json" };
const serve = (root) => new Promise((resolve) => {
  const server = createServer((request, response) => {
    let file = path.join(root, decodeURIComponent(new URL(request.url, "http://x").pathname));
    if (!file.startsWith(root)) return response.writeHead(403).end();
    if (existsSync(file) && !path.extname(file)) file = path.join(file, "index.html");
    if (!existsSync(file)) return response.writeHead(404).end();
    response.writeHead(200, { "content-type": types[path.extname(file)] ?? "application/octet-stream" });
    response.end(readFileSync(file));
  });
  server.listen(0, "127.0.0.1", () => resolve([server, `http://127.0.0.1:${server.address().port}/`]));
});
const [siteA, urlA] = await serve(path.resolve(withRelease));
const [siteB, urlB] = await serve(path.resolve(none));

const work = mkdtempSync(path.join(tmpdir(), "canoa-e2e-docs-"));
const chrome = spawn(browser, ["--headless=new", `--user-data-dir=${path.join(work, "profile")}`, "--remote-debugging-port=0", "--no-first-run", "about:blank"], { stdio: "ignore" });
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
let debug;
for (let i = 0; i < 300 && !debug; i++) {
  try {
    debug = readFileSync(path.join(work, "profile", "DevToolsActivePort"), "utf8").split("\n")[0];
  } catch {
    await sleep(100);
  }
}
assert.ok(debug, "the browser did not start");

const chromium = (platform, architecture, bitness, mobile = false) => ({
  userAgent: `Mozilla/5.0 (${platform === "Windows" ? "Windows NT 10.0; Win64; x64" : platform === "Linux" ? "X11; Linux x86_64" : platform === "macOS" ? "Macintosh; Intel Mac OS X 10_15_7" : "Linux; Android 14; Pixel"}) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 ${mobile ? "Mobile " : ""}Safari/537.36`,
  metadata: { platform, platformVersion: "10", architecture, bitness, model: "", mobile, brands: [{ brand: "Chromium", version: "130" }], fullVersionList: [{ brand: "Chromium", version: "130.0.0.0" }] },
});

async function read(url, { userAgent, metadata }) {
  const made = await (await fetch(`http://127.0.0.1:${debug}/json/new?about:blank`, { method: "PUT" })).json();
  const session = new Session(made.webSocketDebuggerUrl);
  await session.call("Page.enable");
  await session.call("Emulation.setUserAgentOverride", { userAgent, userAgentMetadata: metadata });
  await session.call("Page.navigate", { url });
  for (let i = 0; i < 100; i++) {
    if (await session.run("document.readyState === 'complete' && !!document.querySelector('.canoa-download__main')")) break;
    await sleep(100);
  }
  await sleep(400); // the high-entropy hints answer after the first paint
  const result = await session.run(`(() => {
    const main = document.querySelector(".canoa-download__main");
    const current = document.querySelector("a[aria-current='true']");
    return { text: main.textContent.trim(), href: main.getAttribute("href"), current: current && current.getAttribute("data-platform"),
             options: [...document.querySelectorAll("a[data-platform]")].map((a) => a.getAttribute("data-platform")) };
  })()`);
  session.close();
  await fetch(`http://127.0.0.1:${debug}/json/close/${made.id}`);
  return result;
}

const results = [];
async function check(name, run) {
  try {
    await run();
    results.push(`ok   ${name}`);
  } catch (error) {
    results.push(`FAIL ${name}: ${String(error.message).slice(0, 400)}`);
  }
  console.log(results.at(-1));
}

try {
  const file = (href) => decodeURIComponent(href.split("/").pop());
  await check("Windows x64 gets the Windows x64 file", async () => {
    const r = await read(urlA, chromium("Windows", "x86", "64"));
    assert.equal(r.current, "windows-x64");
    assert.equal(r.text, "Download for Windows, x64");
    assert.match(file(r.href), /windows-x64/);
  });
  await check("Windows on Arm gets the Arm64 file though the user agent says x64", async () => {
    const r = await read(urlA, chromium("Windows", "arm", "64"));
    assert.equal(r.current, "windows-arm64");
    assert.match(file(r.href), /windows-arm64/);
  });
  await check("Linux x64 gets the AppImage for x64", async () => {
    const r = await read(urlA, chromium("Linux", "x86", "64"));
    assert.equal(r.current, "linux-x64");
    assert.match(file(r.href), /AppImage/i);
  });
  await check("Linux on Arm gets the AppImage for Arm64", async () => {
    const r = await read(urlA, chromium("Linux", "arm", "64"));
    assert.equal(r.current, "linux-arm64");
  });
  await check("macOS has no file in this release: the button offers the choice", async () => {
    const r = await read(urlA, chromium("macOS", "arm", "64"));
    assert.equal(r.current, null);
    assert.equal(r.text, "Choose a download");
  });
  await check("a phone has no file: the button offers the choice", async () => {
    const r = await read(urlA, chromium("Android", "", "", true));
    assert.equal(r.current, null);
    assert.equal(r.text, "Choose a download");
  });
  await check("a browser with no client hints falls back to its user agent string", async () => {
    const r = await read(urlA, { userAgent: "Mozilla/5.0 (X11; Linux x86_64; rv:130.0) Gecko/20100101 Firefox/130.0", metadata: undefined });
    assert.equal(r.current, "linux-x64");
  });
  await check("every file of the release is in the arrow's list", async () => {
    const r = await read(urlA, chromium("Windows", "x86", "64"));
    assert.deepEqual(r.options, ["windows-x64", "windows-arm64", "linux-x64", "linux-arm64", "extension"]);
  });
  await check("a site built with no release says so, links no file and leaves the button alone", async () => {
    const r = await read(urlB, chromium("Windows", "x86", "64"));
    assert.equal(r.text, "Build from source");
    assert.equal(r.href, "getting-started/");
    assert.deepEqual(r.options, []);
  });
} finally {
  chrome.kill();
  siteA.close();
  siteB.close();
  await sleep(1000);
  try {
    rmSync(work, { recursive: true, force: true });
  } catch {}
}
process.exit(results.some((line) => line.startsWith("FAIL")) ? 1 : 0);
