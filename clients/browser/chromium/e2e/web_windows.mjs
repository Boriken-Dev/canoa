// The web app's windows in a real browser: the session list, the Files panel and a terminal's shells of the real app
// (`app/tool/windows_page.dart`: two conversations answered by an agent in the page, a folder of two files and a shell that runs what
// it is typed), each opened as a browser window of its own, a child of the page.
//
//  - a session opens in a window that is a child of the page (window.opener), sends from there, and the answer reaches both;
//  - a terminal opens in a window that types into the page's shell, and closing the window leaves the shell where it was;
//  - a file opens in a window that draws the file the page serves it;
//  - the setting: with "this window" a click on a session shows it where it is, with "new windows" the same click opens a window and
//    shows nothing here, and a click on a file does the same;
//  - with `views`, the window's keyboard, against the real composer: ASCII, non-ASCII, a composition, Backspace, the caret, Enter, Tab,
//    a Ctrl shortcut, a paste, focus and blur between two windows, and frames with the page minimized, each read from what the
//    framework holds (`canoaWindowsFocus`), and three controls that take one shim of the page away and expect its check to fail.
//
// `CANOA_E2E_WINDOWS` picks the backend the page is started with: `views` (the default) is the one running app drawing a view into each
// window it opens (the page's `?windows=views`), `copy` a second copy of the app in each window over a link (`?windows=copy`). The
// same actions pass on both. The checks of the window's keyboard, focus and frames are `views` only.
//
//   cd app && flutter build web --release --no-web-resources-cdn -t tool/windows_page.dart -o <dir>
//   CANOA_E2E_BROWSER=path/to/chrome CANOA_E2E_PAGE=<dir> [CANOA_E2E_WINDOWS=views|copy] [CANOA_E2E_SHOT=<dir for pictures>] node e2e/web_windows.mjs
import { spawn } from "node:child_process";
import { createReadStream, existsSync, mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import http from "node:http";
import { tmpdir } from "node:os";
import path from "node:path";

import { lastCall, Session } from "./cdp.mjs";

const { CANOA_E2E_BROWSER: browser, CANOA_E2E_PAGE: pageDir, CANOA_E2E_SHOT: shots, CANOA_E2E_WINDOWS: backend = "views" } = process.env;
if (!browser || !pageDir) throw new Error("set CANOA_E2E_BROWSER and CANOA_E2E_PAGE");
if (!["views", "copy"].includes(backend)) throw new Error("CANOA_E2E_WINDOWS is views or copy");
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

const work = mkdtempSync(path.join(tmpdir(), "canoa-web-windows-"));
const profile = path.join(work, "profile");
if (shots) mkdirSync(shots, { recursive: true });
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
    await sleep(150);
  }
}
const same = (actual, expected, what) => {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) throw new Error(`${what}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
};

let chrome;
let root;
let exitCode = 1;
const watchdog = setTimeout(() => {
  console.log(`FAILED: the run took too long, at: ${lastCall()}`);
  chrome?.kill();
  process.exit(1);
}, 900_000);
try {
  chrome = spawn(browser, [
    "--headless=new", `--user-data-dir=${profile}`, "--remote-debugging-port=0", "--no-first-run", "--no-default-browser-check",
    "--window-size=1100,760", "about:blank",
  ], { stdio: "ignore" });
  const debug = await waitFor("the browser", () => {
    try {
      return readFileSync(path.join(profile, "DevToolsActivePort"), "utf8").split("\n")[0];
    } catch {
      return null;
    }
  });
  const targets = async () => (await (await fetch(`http://127.0.0.1:${debug}/json/list`)).json()).filter((each) => each.type === "page");
  const attach = async (target) => {
    const page = new Session(target.webSocketDebuggerUrl);
    page.id = target.id;
    await page.call("Page.enable");
    await page.call("Runtime.enable");
    await page.call("Emulation.setFocusEmulationEnabled", { enabled: true });
    return page;
  };
  const closeTarget = (id) => fetch(`http://127.0.0.1:${debug}/json/close/${id}`).catch(() => null);
  root = new Session((await (await fetch(`http://127.0.0.1:${debug}/json/version`)).json()).webSocketDebuggerUrl);
  await root.call("Browser.grantPermissions", { origin, permissions: ["clipboardReadWrite", "clipboardSanitizedWrite"] });

  /** The page of the app: the session list, the files and the page's own agent. */
  async function openApp() {
    const made = await (await fetch(`http://127.0.0.1:${debug}/json/new?about:blank`, { method: "PUT" })).json();
    const page = await attach(made);
    await page.call("Page.bringToFront");
    await page.call("Page.navigate", { url: `${origin}/index.html?windows=${backend}` });
    await waitFor("the app", () => page.run("typeof window.canoaWindowsState === 'function' && Boolean(document.querySelector('flutter-view, flt-glass-pane'))").catch(() => false));
    await sleep(1500);
    await page.run("document.querySelector('flt-semantics-placeholder')?.click()");
    await sleep(500);
    return page;
  }
  const state = async (page) => JSON.parse(await page.run("window.canoaWindowsState()"));
  /** A window the page opened: a page of the copy at `?popout=`, or the blank page a view is drawn into. */
  const isWindow = (each) => each.url.includes("popout=") || each.url === "about:blank";
  /** The windows the page opened, that are not in [known]: their targets, as they appear. */
  async function newWindow(known, what) {
    return waitFor(what, async () => (await targets()).find((each) => !known.includes(each.id) && isWindow(each)), 30_000);
  }
  const labels = (page) => page.run(`[...document.querySelectorAll('flt-semantics')].map((node) => node.getAttribute('aria-label') || node.textContent).join('\\n')`);
  /** The middle of the smallest semantics node whose label has [text], or null. */
  const nodeBox = (page, text) => page.run(`(() => {
    const area = (each) => { const rect = each.getBoundingClientRect(); return rect.width * rect.height; };
    const node = [...document.querySelectorAll('flt-semantics')].filter((each) => (each.getAttribute('aria-label') || each.textContent).includes(${JSON.stringify(text)})).sort((a, b) => area(a) - area(b))[0];
    if (!node) return null;
    const rect = node.getBoundingClientRect();
    return { x: rect.x + rect.width / 2, y: rect.y + rect.height / 2 };
  })()`);
  const mouse = (page, type, x, y) => page.call("Input.dispatchMouseEvent", { type, x, y, button: "left", clickCount: 1, buttons: type === "mousePressed" ? 1 : 0 });
  async function click(page, x, y) {
    await mouse(page, "mouseMoved", x, y);
    await mouse(page, "mousePressed", x, y);
    await mouse(page, "mouseReleased", x, y);
    await sleep(900);
  }
  async function clickOn(page, text) {
    const box = await waitFor(`'${text}' on screen`, () => nodeBox(page, text).catch(() => null), 20_000);
    await click(page, box.x, box.y);
  }
  async function enter(page) {
    const base = { key: "Enter", code: "Enter", windowsVirtualKeyCode: 13, nativeVirtualKeyCode: 13 };
    await page.call("Input.dispatchKeyEvent", { type: "rawKeyDown", ...base, text: "\r" });
    await page.call("Input.dispatchKeyEvent", { type: "keyUp", ...base });
    await sleep(600);
  }
  const CTRL = 2;
  /** One key, with the modifiers held around it, as the protocol sends a keyboard's. */
  async function key(page, { key: name, code, vk, modifiers = 0, text }) {
    const held = [[CTRL, "Control", "ControlLeft", 17]].filter(([flag]) => modifiers & flag);
    for (const [, modifierName, modifierCode, modifierVk] of held) {
      await page.call("Input.dispatchKeyEvent", { type: "rawKeyDown", key: modifierName, code: modifierCode, windowsVirtualKeyCode: modifierVk, nativeVirtualKeyCode: modifierVk, modifiers });
    }
    const base = { key: name, code, windowsVirtualKeyCode: vk, nativeVirtualKeyCode: vk, modifiers };
    await page.call("Input.dispatchKeyEvent", { type: text ? "keyDown" : "rawKeyDown", ...base, ...(text ? { text, unmodifiedText: text } : {}) });
    await page.call("Input.dispatchKeyEvent", { type: "keyUp", ...base });
    for (const [, modifierName, modifierCode, modifierVk] of held) {
      await page.call("Input.dispatchKeyEvent", { type: "keyUp", key: modifierName, code: modifierCode, windowsVirtualKeyCode: modifierVk, nativeVirtualKeyCode: modifierVk, modifiers: 0 });
    }
    await sleep(300);
  }
  const keys = {
    Backspace: { key: "Backspace", code: "Backspace", vk: 8 },
    Enter: { key: "Enter", code: "Enter", vk: 13, text: "\r" },
    Tab: { key: "Tab", code: "Tab", vk: 9 },
    End: { key: "End", code: "End", vk: 35 },
    Left: { key: "ArrowLeft", code: "ArrowLeft", vk: 37 },
    K: { key: "k", code: "KeyK", vk: 75, modifiers: CTRL },
    V: { key: "v", code: "KeyV", vk: 86, modifiers: CTRL },
  };
  const typeKeys = async (page, text) => {
    for (const character of text) {
      await key(page, character === " " ? { key: " ", code: "Space", vk: 32, text: " " } : { key: character, code: `Key${character.toUpperCase()}`, vk: character.toUpperCase().charCodeAt(0), text: character });
    }
  };
  /** The framework's focus: the view it is in and, for a text field, its text, selection and composing range. */
  const focusState = async (app) => JSON.parse(await app.run("window.canoaWindowsFocus()"));
  /** The window as a page of its own, in front and with its semantics on. */
  async function intoWindow(target) {
    const page = await attach(target);
    await page.call("Page.bringToFront");
    await waitFor("the window's view", () => page.run("Boolean(document.querySelector('flutter-view, flt-glass-pane'))").catch(() => false));
    await sleep(2000);
    await page.run("document.querySelector('flt-semantics-placeholder')?.click()");
    await sleep(600);
    return page;
  }
  const shot = async (page, name) => {
    if (!shots) return;
    const { data } = await page.call("Page.captureScreenshot", { format: "png" });
    writeFileSync(path.join(shots, name), Buffer.from(data, "base64"));
  };

  const app = await openApp();
  const known = [app.id, ...(await targets()).map((each) => each.id)];
  same((await state(app)).openInNewWindows, false, "the page starts with this window");

  let sessionWindow;
  await check("a session opens in a window of its own that is a child of the page, with the conversation's title", async () => {
    await app.run("window.canoaWindowsOpenSession('Fix calc')", { gesture: true });
    const target = await newWindow(known, "the session's window");
    known.push(target.id);
    sessionWindow = await intoWindow(target);
    same(await sessionWindow.run("window.opener !== null"), true, "the window is opened by the page");
    await waitFor("the title", async () => (await sessionWindow.run("document.title")).startsWith("Fix calc"));
    const held = await state(app);
    same(held.sessions.find((each) => each.title === "Fix calc").poppedOut, true, "the page knows it");
    same(held.windows, ["Fix calc"], "the page lists the window");
    await shot(sessionWindow, "session_window.png");
  });

  await check("a message sent from the window reaches the conversation, and the answer is in the window and in the page", async () => {
    // The composer is the box along the bottom of the window.
    const [width, height] = await sessionWindow.run("[innerWidth, innerHeight]");
    await click(sessionWindow, width / 2, height - 46);
    await waitFor("the keyboard in the composer", () => sessionWindow.run("/INPUT|TEXTAREA/.test(document.activeElement?.tagName ?? '')").catch(() => false), 10_000);
    await sessionWindow.call("Input.insertText", { text: "hello from the window" });
    await sleep(400);
    await enter(sessionWindow);
    const held = await waitFor("the answer", async () => {
      const now = await state(app);
      const texts = now.sessions.find((each) => each.title === "Fix calc").messages;
      return texts.some((text) => text.includes("echo: hello from the window")) ? now : null;
    }, 30_000);
    same(held.sessions.find((each) => each.title === "Fix calc").messages.some((text) => text === "hello from the window"), true, "the question is in the conversation");
    await waitFor("the answer in the window", async () => (await labels(sessionWindow)).includes("echo: hello from the window"), 30_000);
    await shot(sessionWindow, "session_after_send.png");
  });

  await check("a paste key puts the clipboard's text in the composer once, in either backend", async () => {
    const [width, height] = await sessionWindow.run("[innerWidth, innerHeight]");
    await click(sessionWindow, width / 2, height - 46);
    await waitFor("the keyboard in the composer", () => sessionWindow.run("/INPUT|TEXTAREA/.test(document.activeElement?.tagName ?? '')").catch(() => false), 10_000);
    await sessionWindow.run("navigator.clipboard.writeText('PASTE')", { gesture: true });
    await key(sessionWindow, keys.V);
    const held = await sessionWindow.run("document.activeElement?.value ?? ''");
    same(held.split("PASTE").length - 1, 1, `the composer holds ${JSON.stringify(held)}, PASTE`);
    for (let i = 0; i < 5; i++) await key(sessionWindow, keys.Backspace);
  });

  let terminalWindow;
  await check("a terminal opens in a window that types into the page's shell", async () => {
    await app.run("window.canoaWindowsOpenTerminal()", { gesture: true });
    const target = await newWindow(known, "the terminal's window");
    known.push(target.id);
    terminalWindow = await intoWindow(target);
    same(await terminalWindow.run("window.opener !== null"), true, "the window is opened by the page");
    await waitFor("the shell drawn in the window", async () => (await state(app)).shellsOpened === 1, 20_000);
    // The shell takes the keyboard on its own when the window opens; a click makes sure.
    const [width, height] = await terminalWindow.run("[innerWidth, innerHeight]");
    await click(terminalWindow, width / 2, height / 2);
    // The terminal takes its keys from the keyboard events; its text input exists only while the page's accessibility tree is off.
    await typeKeys(terminalWindow, "echo hi");
    await enter(terminalWindow);
    const held = await waitFor("the shell to run the line", async () => {
      const now = await state(app);
      return now.shellScreen.includes("ran: echo hi") ? now : null;
    }, 20_000);
    same(held.shellsOpened, 1, "the window opened no shell of its own");
    same(held.shellWrites[0].includes("echo hi"), true, "the typing reached the page's shell");
    await shot(terminalWindow, "terminal_window.png");
  });

  await check("closing the terminal window leaves the shell running, and it opens again on the same shell", async () => {
    await closeTarget(terminalWindow.id);
    await waitFor("the window gone", async () => (await state(app)).items.length === 0, 20_000);
    const held = await state(app);
    same(held.shellsOpened, 1, "the shell stays the page's");
    same(held.shellScreen.includes("ran: echo hi"), true, "with the screen it had");
    await app.run("window.canoaWindowsOpenTerminal()", { gesture: true });
    const again = await newWindow(known, "the terminal's window again");
    known.push(again.id);
    await waitFor("it is served", async () => (await state(app)).items.length === 1, 20_000);
    same((await state(app)).shellsOpened, 1, "no shell was made for the new window");
    await closeTarget(again.id);
  });


  if (backend === "views") {
    const view = "views";
    let composer;
    let pasted;
    let other;
    const compose = async (page) => {
      const [width, height] = await page.run("[innerWidth, innerHeight]");
      await click(page, width / 2, height - 46);
      await waitFor("the keyboard in the composer", async () => (await focusState(app)).field !== null, 10_000);
    };
    const field = async () => (await focusState(app)).field;
    const text = async () => (await field())?.text;

    await check(`${view}: a click on the composer gives it the keyboard, in the window's own view`, async () => {
      composer = sessionWindow;
      await compose(composer);
      same(await composer.run("/INPUT|TEXTAREA/.test(document.activeElement?.tagName ?? '')"), true, "the window's active element is the hidden input");
      const { view: viewId } = await focusState(app);
      if (!viewId || viewId <= 1) throw new Error(`the focus is in view ${viewId}, not a window's`);
    });

    await check(`${view}: ASCII typed with key events reaches the composer`, async () => {
      await typeKeys(composer, "hello");
      same(await text(), "hello", "the composer");
    });

    await check(`${view}: non-ASCII inserted text (é, a CJK character, an emoji) reaches the composer`, async () => {
      await composer.call("Input.insertText", { text: "é你😀" });
      await sleep(400);
      same(await text(), "helloé你😀", "the composer");
    });

    await check(`${view}: the arrow keys move the framework's caret, not only the browser's`, async () => {
      await key(composer, keys.End);
      await key(composer, keys.Left);
      await key(composer, keys.Left);
      const held = await field();
      same(held.selection[0], held.selection[1], "a collapsed caret");
      same(held.selection[0], 6, "the caret two Left presses from the end of 'helloé你😀'");
    });

    await check(`${view}: Backspace removes the character before the caret`, async () => {
      await key(composer, keys.Backspace);
      same(await text(), "hello你😀", "the composer");
    });

    await check(`${view}: Ctrl+K reaches the framework's keyboard`, async () => {
      await app.run("window.canoaWindowsKeysReset()");
      await key(composer, keys.K);
      const heard = JSON.parse(await app.run("window.canoaWindowsKeys()"));
      if (!heard.includes("down:K+ctrl")) throw new Error(`the framework heard ${JSON.stringify(heard)}`);
    });

    await check(`${view}: Tab moves the framework's focus and the browser's stays in the window`, async () => {
      await key(composer, keys.Tab);
      const held = await focusState(app);
      same(held.field, null, "the composer no longer has the focus");
      if (!held.view) throw new Error("the framework's focus went nowhere");
      const tag = await composer.run("document.activeElement?.tagName");
      if (tag === "BODY") throw new Error("the browser's own Tab took the focus out of the view");
    });

    await check(`${view}: a composition shows in the composer, and the commit leaves no composing range`, async () => {
      await compose(composer);
      await composer.call("Input.imeSetComposition", { text: "ni", selectionStart: 2, selectionEnd: 2 });
      await sleep(500);
      same(await text(), "hello你😀ni", "the composer during the composition");
      await composer.call("Input.imeSetComposition", { text: "に", selectionStart: 1, selectionEnd: 1 });
      await sleep(300);
      await composer.call("Input.insertText", { text: "に" });
      await sleep(500);
      const held = await field();
      same(held.text, "hello你😀に", "the composer after the commit");
      same(held.composing, [-1, -1], "no composing range after the commit");
    });

    await check(`${view}: a paste key puts the clipboard's text in the composer`, async () => {
      await composer.run("navigator.clipboard.writeText('pasted')", { gesture: true });
      await key(composer, keys.V);
      pasted = await text();
      if (!pasted.startsWith("hello你😀に") || !pasted.includes("pasted")) throw new Error(`the composer holds ${JSON.stringify(pasted)}`);
    });

    await check(`${view}: Enter sends what is in the composer, from the window`, async () => {
      await key(composer, keys.Enter);
      await waitFor("the answer", async () => (await state(app)).sessions.find((each) => each.title === "Fix calc").messages.includes(`echo: ${pasted}`), 20_000);
    });

    /** [str] typed into [page], whose composer must then hold what it did plus [str]. */
    const typedInto = async (page, str) => {
      await page.call("Page.bringToFront");
      await compose(page);
      const before = await text();
      await typeKeys(page, str);
      same(await text(), before + str, "the composer typed into");
    };
    const typedIntoOpener = async (str) => {
      await app.call("Page.bringToFront");
      // A text field's semantics node is an input holding its label.
      const box = await waitFor("the page's own field", () => app.run(`(() => {
        const node = [...document.querySelectorAll('input, textarea, flt-semantics')].find((each) => (each.getAttribute('aria-label') || '').includes('opener field'));
        if (!node) return null;
        const rect = node.getBoundingClientRect();
        return { x: rect.x + rect.width / 2, y: rect.y + rect.height / 2 };
      })()`), 10_000);
      await click(app, box.x, box.y);
      await waitFor("the keyboard in the page's own field", async () => (await focusState(app)).view === 1, 10_000);
      const before = (await field()).text;
      await typeKeys(app, str);
      same(await text(), before + str, "the page's own field typed into");
    };
    const betweenWindows = async () => {
      await typedIntoOpener("o");
      await typedInto(other, "b");
      await typedIntoOpener("p");
      await typedInto(composer, "a");
      await typedInto(other, "c");
      await typedIntoOpener("q");
    };
    await check(`${view}: typing goes to the window that was clicked last, between the page and two windows`, async () => {
      const before = (await targets()).map((each) => each.id);
      await app.run("window.canoaWindowsOpenSession('Plan docs')", { gesture: true });
      const target = await newWindow(before, "the second session window");
      known.push(target.id);
      other = await intoWindow(target);
      await betweenWindows();
    });

    await check(`${view}: frames keep coming with the page minimized, drawn from the window`, async () => {
      await composer.call("Page.bringToFront");
      await compose(composer);
      const { windowId } = await root.call("Browser.getWindowForTarget", { targetId: app.id });
      await app.call("Emulation.setFocusEmulationEnabled", { enabled: false });
      await root.call("Browser.setWindowBounds", { windowId, bounds: { windowState: "minimized" } });
      await sleep(1000);
      try {
        same(await app.run("Object.getOwnPropertyDescriptor(Document.prototype, 'visibilityState').get.call(document)"), "hidden", "the page's own visibility");
        const before = await app.run("window.canoaWindowsFrames()");
        // Each character is a frame the framework has to draw; the caret's own blinking adds more.
        for (const character of "frames") {
          await composer.call("Input.insertText", { text: character });
          await sleep(250);
        }
        const after = await app.run("window.canoaWindowsFrames()");
        if (after - before < 6) throw new Error(`the framework drew ${after - before} frames while 6 characters were typed`);
      } finally {
        await root.call("Browser.setWindowBounds", { windowId, bounds: { windowState: "normal" } });
        await app.call("Emulation.setFocusEmulationEnabled", { enabled: true });
        await sleep(500);
      }
    });

    // The controls: one shim of the page taken away, and the check that depends on it must fail, or the check says nothing.
    const minimizedFrames = async (characters) => {
      await composer.call("Page.bringToFront");
      await compose(composer);
      const { windowId } = await root.call("Browser.getWindowForTarget", { targetId: app.id });
      await app.call("Emulation.setFocusEmulationEnabled", { enabled: false });
      await root.call("Browser.setWindowBounds", { windowId, bounds: { windowState: "minimized" } });
      await sleep(1000);
      try {
        same(await app.run("Object.getOwnPropertyDescriptor(Document.prototype, 'visibilityState').get.call(document)"), "hidden", "the page's own visibility");
        const before = await app.run("window.canoaWindowsFrames()");
        for (const character of characters) {
          await composer.call("Input.insertText", { text: character });
          await sleep(250);
        }
        return (await app.run("window.canoaWindowsFrames()")) - before;
      } finally {
        await root.call("Browser.setWindowBounds", { windowId, bounds: { windowState: "normal" } });
        await app.call("Emulation.setFocusEmulationEnabled", { enabled: true });
        await sleep(500);
      }
    };
    await check(`control: without the visibility shim, no frames are drawn with the page minimized`, async () => {
      await app.run("delete document.visibilityState; delete document.hidden");
      const drawn = await minimizedFrames("stop");
      if (drawn >= 4) throw new Error(`${drawn} frames were drawn without the shim: the check proves nothing`);
    });

    await check(`control: without the widened type checks, Enter in a window sends nothing`, async () => {
      await app.run("for (const name of ['KeyboardEvent', 'CompositionEvent', 'InputEvent', 'PointerEvent', 'HTMLInputElement', 'HTMLTextAreaElement']) delete window[name][Symbol.hasInstance]");
      await composer.call("Page.bringToFront");
      await compose(composer);
      await composer.call("Input.insertText", { text: "control" });
      await sleep(300);
      await key(composer, keys.Enter);
      await sleep(1500);
      if ((await state(app)).sessions.find((each) => each.title === "Fix calc").messages.some((each) => each.includes("control"))) {
        throw new Error("Enter sent the message without the shim: the check proves nothing");
      }
    });

    await check(`control: without the focus shim, typing between two windows fails`, async () => {
      await app.run("delete document.activeElement");
      let failed = false;
      try {
        await betweenWindows();
      } catch {
        failed = true;
      }
      if (!failed) throw new Error("typing between two windows worked without the shim: the check proves nothing");
    });

    await closeTarget(other.id);
  }

  await check("a click on a session shows it here with 'this window', and opens a window with 'new windows'", async () => {
    await closeTarget(sessionWindow.id);
    await sleep(1500);
    const before = (await targets()).map((each) => each.id);
    await app.run("window.canoaWindowsOpenIn('this_window')");
    await clickOn(app, "Plan docs");
    const here = await state(app);
    same(here.selected, "Plan docs", "this window: the click selects");
    same((await targets()).map((each) => each.id).sort(), before.sort(), "this window: no window opens");

    await app.run("window.canoaWindowsOpenIn('new_windows')");
    await sleep(500);
    await clickOn(app, "Fix calc");
    const opened = await waitFor("a window for the click", async () => (await targets()).find((each) => !before.includes(each.id) && isWindow(each)), 20_000);
    known.push(opened.id);
    const there = await state(app);
    same(there.selected, "Plan docs", "new windows: the click selects nothing here");
    same(there.openInNewWindows, true, "the setting reached the page's windows");
    same(there.sessions.find((each) => each.title === "Fix calc").poppedOut, true, "and the session is in the window");
    await closeTarget(opened.id);
  });

  await check("a click on a file shows it in the panel with 'this window', and in a window with 'new windows'", async () => {
    await app.run("window.canoaWindowsOpenIn('this_window')");
    await sleep(500);
    const before = (await targets()).map((each) => each.id);
    await clickOn(app, "todo.txt");
    await shot(app, "file_in_panel.png");
    await waitFor("the file read for the panel", async () => (await state(app)).filesRead.includes("/ws/todo.txt"), 15_000);
    same((await state(app)).items, [], "this window: no window for the file");
    same((await targets()).map((each) => each.id).sort(), before.sort(), "this window: no window opens");
    // Back to the listing.
    await app.run("window.canoaWindowsOpenIn('new_windows')");
    await sleep(500);
    await app.run("location.reload()");
    await waitFor("the app again", () => app.run("typeof window.canoaWindowsState === 'function' && Boolean(document.querySelector('flutter-view, flt-glass-pane'))").catch(() => false));
    await sleep(1500);
    await app.run("document.querySelector('flt-semantics-placeholder')?.click()");
    await app.run("window.canoaWindowsOpenIn('new_windows')");
    await sleep(500);
    await clickOn(app, "notes.txt");
    const target = await waitFor("a window for the file", async () => (await targets()).find((each) => !before.includes(each.id) && isWindow(each) && (backend === "views" || each.url.includes("item=file"))), 20_000);
    const window = await intoWindow(target);
    await waitFor("the file read for the window, over the link", async () => (await state(app)).filesRead.includes("/ws/notes.txt"), 30_000);
    await waitFor("the window listed", async () => (await state(app)).items.length === 1, 10_000);
    await shot(window, "file_window.png");
    await closeTarget(target.id);
  });
  exitCode = results.some((line) => line.startsWith("FAIL")) ? 1 : 0;
} catch (error) {
  console.log(`FAILED: ${error.stack ?? error}`);
} finally {
  clearTimeout(watchdog);
  root?.close();
  chrome?.kill();
  server.close();
  await sleep(1500);
  try {
    rmSync(work, { recursive: true, force: true, maxRetries: 10, retryDelay: 500 });
  } catch {
    // The browser may still hold its profile; the folder is in the temporary directory.
  }
}
console.log(exitCode === 0 ? "PASSED" : "FAILED");
process.exit(exitCode);
