// Copy and paste in the terminal of the web app, in a real browser: the Flutter terminal panel of `app/tool/terminal_page.dart`
// (a shell the test prints to and reads the input of), with real key events: each modifier goes down before the key, as on a keyboard.
//
//  - a paste key reaches the shell through the browser's `paste` event with no clipboard-read permission granted;
//  - the control, a page that reads the clipboard on the key instead, is refused and the shell hears nothing;
//  - Ctrl+Shift+C and Ctrl+C (with a selection, on Windows) put the selection on the clipboard, and the second Ctrl+C is the
//    interrupt.
//
//   cd app && dart pub get && flutter build web --release --no-web-resources-cdn -t tool/terminal_page.dart -o <dir>
//   CANOA_E2E_BROWSER=path/to/chrome CANOA_E2E_PAGE=<dir> node e2e/terminal_clipboard.mjs
import { spawn } from "node:child_process";
import { createReadStream, existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import http from "node:http";
import { tmpdir } from "node:os";
import path from "node:path";

import { lastCall, Session } from "./cdp.mjs";

const { CANOA_E2E_BROWSER: browser, CANOA_E2E_PAGE: pageDir } = process.env;
if (!browser || !pageDir) throw new Error("set CANOA_E2E_BROWSER and CANOA_E2E_PAGE");
const CTRL = 2;
const SHIFT = 8;
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const TYPES = { ".html": "text/html", ".js": "text/javascript", ".json": "application/json", ".wasm": "application/wasm", ".otf": "font/otf", ".ttf": "font/ttf", ".png": "image/png", ".css": "text/css" };

const server = http.createServer((request, response) => {
  const name = decodeURIComponent(new URL(request.url, "http://x").pathname).replace(/^\/+/, "") || "index.html";
  const file = path.join(pageDir, name);
  if (!file.startsWith(path.resolve(pageDir)) || !existsSync(file)) {
    response.writeHead(404).end();
    return;
  }
  response.writeHead(200, { "content-type": TYPES[path.extname(file)] ?? "application/octet-stream" });
  createReadStream(file).pipe(response);
});
await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
const origin = `http://127.0.0.1:${server.address().port}`;

const work = mkdtempSync(path.join(tmpdir(), "canoa-terminal-clipboard-"));
const profile = path.join(work, "profile");
const results = [];
async function check(name, run) {
  try {
    await run();
    results.push(`ok   ${name}`);
  } catch (error) {
    results.push(`FAIL ${name}: ${String(error.message).slice(0, 800)}`);
  }
  console.log(results.at(-1));
}
async function waitFor(what, test, ms = 60_000) {
  const until = Date.now() + ms;
  for (;;) {
    const value = await test();
    if (value) return value;
    if (Date.now() > until) throw new Error(`timed out waiting for ${what}`);
    await sleep(100);
  }
}
const same = (actual, expected, what) => {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) throw new Error(`${what}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
};

let chrome;
let exitCode = 1;
const watchdog = setTimeout(() => {
  console.log(`FAILED: the run took too long, at: ${lastCall()}`);
  chrome?.kill();
  process.exit(1);
}, 240_000);
try {
  chrome = spawn(browser, [
    "--headless=new", `--user-data-dir=${profile}`, "--remote-debugging-port=0", "--no-first-run", "--no-default-browser-check",
    "--window-size=1000,700", "about:blank",
  ], { stdio: "ignore" });
  const debug = await waitFor("the browser", () => {
    try {
      return readFileSync(path.join(profile, "DevToolsActivePort"), "utf8").split("\n")[0];
    } catch {
      return null;
    }
  });
  const root = new Session((await (await fetch(`http://127.0.0.1:${debug}/json/version`)).json()).webSocketDebuggerUrl);

  /** A page on the test site, with the terminal focused. */
  async function open(query) {
    const made = await (await fetch(`http://127.0.0.1:${debug}/json/new?about:blank`, { method: "PUT" })).json();
    const page = new Session(made.webSocketDebuggerUrl);
    await page.call("Page.enable");
    await page.call("Runtime.enable");
    await page.call("Emulation.setFocusEmulationEnabled", { enabled: true });
    // A page that is not in front gets no key events from the protocol.
    await page.call("Page.bringToFront");
    await page.call("Page.navigate", { url: `${origin}/index.html${query}` });
    await waitFor("the terminal", () => page.run("typeof window.terminalWrites === 'function' && Boolean(document.querySelector('flutter-view, flt-glass-pane'))").catch(() => false));
    await sleep(1500);
    await click(page, 500, 300);
    await waitFor("the keyboard in the terminal", () => page.run("/INPUT|TEXTAREA/.test(document.activeElement?.tagName ?? '')").catch(() => false));
    return page;
  }
  const mouse = (page, type, x, y, extra = {}) => page.call("Input.dispatchMouseEvent", { type, x, y, button: "left", clickCount: 1, ...extra });
  async function click(page, x, y) {
    await mouse(page, "mousePressed", x, y, { buttons: 1 });
    await mouse(page, "mouseReleased", x, y, { buttons: 0 });
    await sleep(1200);
  }
  async function drag(page, from, to) {
    await mouse(page, "mousePressed", from[0], from[1], { buttons: 1 });
    for (let step = 1; step <= 8; step++) await mouse(page, "mouseMoved", from[0] + ((to[0] - from[0]) * step) / 8, from[1] + ((to[1] - from[1]) * step) / 8, { buttons: 1 });
    await mouse(page, "mouseReleased", to[0], to[1], { buttons: 0 });
    await sleep(300);
  }
  /** A key with modifiers (2 Ctrl, 8 Shift), pressed as a keyboard does: each modifier goes down first, so the page knows it is held. */
  async function key(page, { key: name, code, vk, modifiers = 0 }) {
    const held = [[CTRL, "Control", "ControlLeft", 17], [SHIFT, "Shift", "ShiftLeft", 16]].filter(([flag]) => modifiers & flag);
    let down = 0;
    for (const [flag, modifierKey, modifierCode, modifierVk] of held) {
      down |= flag;
      await page.call("Input.dispatchKeyEvent", { type: "rawKeyDown", key: modifierKey, code: modifierCode, windowsVirtualKeyCode: modifierVk, nativeVirtualKeyCode: modifierVk, modifiers: down });
    }
    const base = { key: name, code, windowsVirtualKeyCode: vk, nativeVirtualKeyCode: vk, modifiers };
    await page.call("Input.dispatchKeyEvent", { type: "rawKeyDown", ...base });
    await page.call("Input.dispatchKeyEvent", { type: "keyUp", ...base });
    for (const [flag, modifierKey, modifierCode, modifierVk] of held.reverse()) {
      down &= ~flag;
      await page.call("Input.dispatchKeyEvent", { type: "keyUp", key: modifierKey, code: modifierCode, windowsVirtualKeyCode: modifierVk, nativeVirtualKeyCode: modifierVk, modifiers: down });
    }
    await sleep(500);
  }
  const writes = async (page) => JSON.parse(await page.run("window.terminalWrites()"));
  const setClipboard = (page, text) => page.run(`navigator.clipboard.writeText(${JSON.stringify(text)})`, { gesture: true });
  const readClipboard = (page) => page.run("navigator.clipboard.readText()", { gesture: true });

  await check("a paste key reaches the shell through the browser's paste event, with no permission to read the clipboard", async () => {
    const page = await open("");
    await root.call("Browser.grantPermissions", { origin, permissions: ["clipboardSanitizedWrite"] });
    await setClipboard(page, "echo one\r\necho two\necho three");
    await key(page, { key: "v", code: "KeyV", vk: 86, modifiers: CTRL });
    same(await writes(page), ["echo one\recho two\recho three"], "Ctrl+V");
    await key(page, { key: "V", code: "KeyV", vk: 86, modifiers: CTRL | SHIFT });
    same((await writes(page)).length, 2, "Ctrl+Shift+V adds one more write");
    same((await writes(page)).at(-1), "echo one\recho two\recho three", "Ctrl+Shift+V");
    const left = (await writes(page)).join("");
    if (left.includes("\x16")) throw new Error("the shell was sent ^V");
    await page.run("window.terminalPrint('\\u001b[?2004h')");
    await sleep(300);
    await key(page, { key: "v", code: "KeyV", vk: 86, modifiers: CTRL });
    same((await writes(page)).at(-1), "\x1b[200~echo one\recho two\recho three\x1b[201~", "bracketed");
    page.close();
  });

  await check("control: a page that reads the clipboard on the key is refused without permission, and the shell hears nothing", async () => {
    const page = await open("?read=1");
    await setClipboard(page, "must not arrive");
    await key(page, { key: "v", code: "KeyV", vk: 86, modifiers: CTRL });
    await sleep(1500);
    same(await writes(page), [], "reading needs the permission this run never grants");
    page.close();
  });

  await check("copy: Ctrl+Shift+C and, with a selection, Ctrl+C put it on the clipboard; the next Ctrl+C is the interrupt", async () => {
    const page = await open("");
    await root.call("Browser.grantPermissions", { origin, permissions: ["clipboardSanitizedWrite", "clipboardReadWrite"] });
    await page.run("window.terminalPrint('build ok\\r\\n')");
    await sleep(500);
    await setClipboard(page, "stale");
    await drag(page, [10, 54], [400, 54]);
    const selected = await page.run("window.terminalSelection()");
    if (!selected.startsWith("build ok")) throw new Error(`nothing selected: ${JSON.stringify(selected)}`);
    await key(page, { key: "C", code: "KeyC", vk: 67, modifiers: CTRL | SHIFT });
    same(await readClipboard(page), selected, "Ctrl+Shift+C");
    same(await writes(page), [], "a copy sends nothing to the shell");

    await setClipboard(page, "stale again");
    await key(page, { key: "c", code: "KeyC", vk: 67, modifiers: CTRL });
    same(await readClipboard(page), selected, "Ctrl+C with a selection");
    same(await writes(page), [], "a copy is not an interrupt");
    same(await page.run("window.terminalSelection()"), "", "the selection is dropped");
    await key(page, { key: "c", code: "KeyC", vk: 67, modifiers: CTRL });
    same(await writes(page), ["\x03"], "the next Ctrl+C");
    page.close();
  });
  await check("copy where the clipboard API is missing (a page that is not a secure context): the copy command still puts it there", async () => {
    const page = await open("");
    await root.call("Browser.grantPermissions", { origin, permissions: ["clipboardSanitizedWrite", "clipboardReadWrite"] });
    await page.run("window.terminalPrint('build ok\\r\\n')");
    await sleep(500);
    await page.run("window.__clipboard = navigator.clipboard; Object.defineProperty(navigator, 'clipboard', { value: undefined, configurable: true })");
    await drag(page, [10, 54], [400, 54]);
    const selected = await page.run("window.terminalSelection()");
    if (!selected.startsWith("build ok")) throw new Error(`nothing selected: ${JSON.stringify(selected)}`);
    await page.run("window.__clipboard.writeText('stale')", { gesture: true });
    await key(page, { key: "C", code: "KeyC", vk: 67, modifiers: CTRL | SHIFT });
    same(await page.run("window.__clipboard.readText()", { gesture: true }), selected, "Ctrl+Shift+C");
    same(await page.run("document.activeElement.tagName"), "INPUT", "the terminal keeps the keyboard");
    page.close();
  });
  exitCode = results.some((line) => line.startsWith("FAIL")) ? 1 : 0;
} catch (error) {
  console.log(`FAILED: ${error.stack ?? error}`);
} finally {
  clearTimeout(watchdog);
  chrome?.kill();
  server.close();
  await sleep(500);
  try {
    rmSync(work, { recursive: true, force: true });
  } catch {
    // The browser may still hold its profile; the temporary folder is removed with the machine's own cleanup.
  }
  console.log(exitCode === 0 ? "PASS" : "FAILED");
  process.exit(exitCode);
}
