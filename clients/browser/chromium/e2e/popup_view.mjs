// A second Flutter view drawn into a window.open popup, in a real browser: what the engine's own handling lets a view in another
// browser window do, and which framework-side shim (`app/tool/popup_view/shims.js`, letters a to e) each gap needs.
// The page is `app/tool/popup_view_page.dart`: the app starts in multi-view mode, a click on "open" calls window.open with no await
// and adds a view of the same app to the popup's document. Real input is dispatched into the popup target through the protocol:
//
//  - open and draw, a click (one state shared by both views), typing ASCII and non-ASCII, IME composition, Backspace, arrow keys
//    (the caret), Enter, Tab, a Ctrl shortcut, a paste;
//  - focus and blur when the other window is used, frames with the opener minimized, the pixel ratio of the popup's own screen;
//  - close: the popup closed, removeView, and the opener's pagehide closing every popup.
//
// One browser per shim set; each set is run from a clean profile and prints `ok` or `FAIL` per item. `CANOA_E2E_SHIMS` lists the
// sets, space separated, `-` for none (default: none, each letter alone, then growing sets). `CANOA_E2E_REQUIRE` names a set whose
// run must pass every item (the exit code); without it the exit code only says the measurement ran. `CANOA_E2E_OUT` writes the
// results as JSON.
//
//   cd app && flutter build web --release --no-web-resources-cdn -t tool/popup_view_page.dart -o <dir>
//   CANOA_E2E_BROWSER=path/to/chrome CANOA_E2E_PAGE=<dir> [CANOA_E2E_SHIMS="- a abcde"] node e2e/popup_view.mjs
import { spawn } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import http from "node:http";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { lastCall, Session } from "./cdp.mjs";

const { CANOA_E2E_BROWSER: browser, CANOA_E2E_PAGE: pageDir, CANOA_E2E_SHIMS: sets, CANOA_E2E_REQUIRE: require, CANOA_E2E_OUT: out } = process.env;
if (!browser || !pageDir) throw new Error("set CANOA_E2E_BROWSER and CANOA_E2E_PAGE");
const toolDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../../../app/tool/popup_view");
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const TYPES = { ".html": "text/html", ".js": "text/javascript", ".json": "application/json", ".wasm": "application/wasm", ".otf": "font/otf", ".ttf": "font/ttf", ".png": "image/png", ".css": "text/css" };
const CTRL = 2;
const SHIFT = 8;

/** The build's files, with the probe's page, shims and start in place of the app's own. */
const bootstrap = readFileSync(path.join(pageDir, "flutter_bootstrap.js"), "utf8");
const served = {
  "index.html": readFileSync(path.join(toolDir, "index.html")),
  "shims.js": readFileSync(path.join(toolDir, "shims.js")),
  "flutter_bootstrap.js": bootstrap.slice(0, bootstrap.lastIndexOf("_flutter.loader.load(")) + readFileSync(path.join(toolDir, "boot.js"), "utf8"),
};
const server = http.createServer((request, response) => {
  const name = decodeURIComponent(new URL(request.url, "http://x").pathname).replace(/^\/+/, "") || "index.html";
  const file = path.join(pageDir, name);
  const body = served[name] ?? (file.startsWith(path.resolve(pageDir)) && existsSync(file) ? readFileSync(file) : null);
  if (!body) {
    response.writeHead(404).end();
    return;
  }
  response.writeHead(200, { "content-type": TYPES[path.extname(name)] ?? "application/octet-stream", "cache-control": "no-store" });
  response.end(body);
});
await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
const origin = `http://127.0.0.1:${server.address().port}`;

async function waitFor(what, test, ms = 20_000) {
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

const work = mkdtempSync(path.join(tmpdir(), "canoa-popup-view-"));
let chrome;
let exitCode = 1;
const watchdog = setTimeout(() => {
  console.log(`FAILED: the run took too long, at: ${lastCall()}`);
  chrome?.kill();
  process.exit(1);
}, 1_800_000);

/** One browser, one shim set: every item's outcome, as `{ item, ok, detail }`. */
async function runSet(shims) {
  const results = [];
  const profile = path.join(work, `profile-${shims || "none"}`);
  chrome = spawn(browser, [
    "--headless=new", `--user-data-dir=${profile}`, "--remote-debugging-port=0", "--no-first-run", "--no-default-browser-check",
    "--window-size=1000,720", "about:blank",
  ], { stdio: "ignore" });
  const debug = await waitFor("the browser", () => {
    try {
      return readFileSync(path.join(profile, "DevToolsActivePort"), "utf8").split("\n")[0];
    } catch {
      return null;
    }
  }, 60_000);
  const root = new Session((await (await fetch(`http://127.0.0.1:${debug}/json/version`)).json()).webSocketDebuggerUrl);
  await root.call("Browser.grantPermissions", { origin, permissions: ["clipboardReadWrite", "clipboardSanitizedWrite"] });
  const targets = async () => (await (await fetch(`http://127.0.0.1:${debug}/json/list`)).json()).filter((each) => each.type === "page");
  async function attach(target) {
    const page = new Session(target.webSocketDebuggerUrl);
    page.id = target.id;
    await page.call("Page.enable");
    await page.call("Runtime.enable");
    return page;
  }

  async function check(item, run) {
    let record;
    try {
      const detail = await run();
      record = { item, ok: true, detail: detail ?? "" };
    } catch (error) {
      record = { item, ok: false, detail: String(error.message).slice(0, 600) };
    }
    results.push(record);
    console.log(`[${shims || "-"}] ${record.ok ? "ok  " : "FAIL"} ${item}${record.detail ? `: ${record.detail}` : ""}`);
  }

  const made = await (await fetch(`http://127.0.0.1:${debug}/json/new?about:blank`, { method: "PUT" })).json();
  const opener = await attach(made);
  await opener.call("Page.bringToFront");
  await opener.call("Page.navigate", { url: `${origin}/index.html?shims=${shims}` });
  await waitFor("the page", () => opener.run("typeof window.probeState === 'function' && Boolean(window.flutterApp) && Boolean(document.querySelector('flutter-view'))").catch(() => false), 60_000).catch((error) => {
    throw new Error(`${error.message}; the page's console: ${opener.logs.slice(-8).join(" // ")}`);
  });
  await sleep(1500);

  const state = async () => JSON.parse(await opener.run("window.probeState()"));
  const mouse = (page, type, x, y) => page.call("Input.dispatchMouseEvent", { type, x, y, button: "left", clickCount: 1, buttons: type === "mousePressed" ? 1 : 0 });
  async function click(page, x, y) {
    await mouse(page, "mouseMoved", x, y);
    await mouse(page, "mousePressed", x, y);
    await mouse(page, "mouseReleased", x, y);
    await sleep(500);
  }
  const center = (rect) => [rect.x + rect.w / 2, rect.y + rect.h / 2];
  const front = async (page) => {
    await page.call("Page.bringToFront");
    await sleep(300);
  };
  const ids = async () => {
    const now = await state();
    const popup = now.popupViews.at(-1);
    const own = Object.keys(now.views).find((key) => Number(key) !== popup);
    return { now, popup: popup === undefined ? undefined : String(popup), own };
  };
  async function modifierKey(page, { key: name, code, vk, modifiers = 0, text }) {
    const held = [[CTRL, "Control", "ControlLeft", 17], [SHIFT, "Shift", "ShiftLeft", 16]].filter(([flag]) => modifiers & flag);
    let down = 0;
    for (const [flag, modifierName, modifierCode, modifierVk] of held) {
      down |= flag;
      await page.call("Input.dispatchKeyEvent", { type: "rawKeyDown", key: modifierName, code: modifierCode, windowsVirtualKeyCode: modifierVk, nativeVirtualKeyCode: modifierVk, modifiers: down });
    }
    const base = { key: name, code, windowsVirtualKeyCode: vk, nativeVirtualKeyCode: vk, modifiers };
    await page.call("Input.dispatchKeyEvent", { type: text ? "keyDown" : "rawKeyDown", ...base, ...(text ? { text, unmodifiedText: text } : {}) });
    await page.call("Input.dispatchKeyEvent", { type: "keyUp", ...base });
    for (const [flag, modifierName, modifierCode, modifierVk] of held.reverse()) {
      down &= ~flag;
      await page.call("Input.dispatchKeyEvent", { type: "keyUp", key: modifierName, code: modifierCode, windowsVirtualKeyCode: modifierVk, nativeVirtualKeyCode: modifierVk, modifiers: down });
    }
    await sleep(350);
  }
  const named = {
    Backspace: { key: "Backspace", code: "Backspace", vk: 8 },
    Enter: { key: "Enter", code: "Enter", vk: 13, text: "\r" },
    Tab: { key: "Tab", code: "Tab", vk: 9 },
    Left: { key: "ArrowLeft", code: "ArrowLeft", vk: 37 },
    K: { key: "k", code: "KeyK", vk: 75, modifiers: CTRL },
    V: { key: "v", code: "KeyV", vk: 86, modifiers: CTRL },
  };
  async function typeAscii(page, text) {
    for (const character of text) await modifierKey(page, { key: character, code: `Key${character.toUpperCase()}`, vk: character.toUpperCase().charCodeAt(0), text: character });
  }
  /** What the popup's document says, for the evidence of a failure. */
  const diag = async (page) => {
    const text = await page.run(`(() => { const active = document.activeElement; return JSON.stringify({ active: active?.tagName, value: active?.value, popupHasFocus: document.hasFocus(), visible: document.visibilityState, dpr: devicePixelRatio, views: document.querySelectorAll('flutter-view').length }); })()`).catch((error) => String(error.message));
    const own = await opener.run("JSON.stringify({ openerHasFocus: document.hasFocus(), visible: document.visibilityState })").catch(() => "?");
    const trail = await opener.run(`window.probeTrail(${Number(process.env.CANOA_E2E_TRAIL ?? 14)})`).catch(() => "?");
    return `${text} ${own} events: ${trail}`;
  };
  const withDiag = async (page, run) => {
    try {
      return await run();
    } catch (error) {
      throw new Error(`${error.message} | ${await diag(page)}`);
    }
  };

  let popup;
  let popupId;
  let ownId;
  async function openPopup() {
    const known = (await targets()).map((each) => each.id);
    await front(opener);
    const view = (await state()).views[(await ids()).own];
    await click(opener, ...center(view.rects.open));
    const target = await waitFor("the popup's target", async () => (await targets()).find((each) => !known.includes(each.id)), 15_000);
    const page = await attach(target);
    await waitFor("the popup's view", async () => {
      const now = await state();
      const id = now.popupViews.at(-1);
      return id !== undefined && now.views[id]?.rects.a && now.frames > 0;
    }, 15_000);
    await front(page);
    popup = page;
    const found = await ids();
    popupId = found.popup;
    ownId = found.own;
    return found;
  }

  await check("open: window.open in the click, a view added to the popup's document, drawn", async () => {
    const found = await openPopup();
    const view = found.now.views[found.popup];
    if (!view.width || view.width < 100) throw new Error(`the view has no size: ${JSON.stringify(view.width)}`);
    const counted = await popup.run("document.querySelectorAll('flutter-view').length");
    same(counted, 1, "flutter-view elements in the popup's document");
    const shot = await popup.call("Page.captureScreenshot", { format: "png" });
    const bytes = Buffer.from(shot.data, "base64").length;
    if (bytes < 4000) throw new Error(`the popup's picture is ${bytes} bytes, blank`);
    if (process.env.CANOA_E2E_SHOT) writeFileSync(path.join(process.env.CANOA_E2E_SHOT, `popup-${shims || "none"}.png`), Buffer.from(shot.data, "base64"));
    return `view ${found.popup} ${view.width}px wide, picture ${bytes} bytes`;
  });

  await check("pointer: a click in the popup changes the state the opener's view shows (one isolate)", async () => {
    const view = (await state()).views[popupId];
    await click(popup, ...center(view.rects.inc));
    same((await state()).shared, 1, "shared after a click in the popup");
    await front(opener);
    const own = (await state()).views[ownId];
    await click(opener, ...center(own.rects.inc));
    await front(popup);
    same((await state()).shared, 2, "shared after a click in the opener");
  });

  await check("focus: a click on a field gives it the keyboard (the popup's hidden input is the active element)", async () => {
    const view = (await state()).views[popupId];
    await click(popup, ...center(view.rects.a));
    await withDiag(popup, async () => {
      await waitFor("the field focused", async () => (await state()).views[popupId].aFocus, 3000);
      const tag = await popup.run("document.activeElement?.tagName");
      if (!/^(INPUT|TEXTAREA)$/.test(tag)) throw new Error(`the active element is ${tag}`);
    });
  });

  await check("type ASCII: key events with text reach the field", async () => {
    await typeAscii(popup, "hello");
    await withDiag(popup, async () => same((await state()).views[popupId].a, "hello", "the field"));
  });

  await check("type non-ASCII: inserted text (é, a CJK character, an emoji) reaches the field", async () => {
    await popup.call("Input.insertText", { text: "é你😀" });
    await sleep(400);
    await withDiag(popup, async () => same((await state()).views[popupId].a, "helloé你😀", "the field"));
  });

  await check("arrow keys: Left moves the caret, and the framework's selection follows", async () => {
    await popup.call("Input.dispatchKeyEvent", { type: "rawKeyDown", key: "End", code: "End", windowsVirtualKeyCode: 35, nativeVirtualKeyCode: 35 });
    await popup.call("Input.dispatchKeyEvent", { type: "keyUp", key: "End", code: "End", windowsVirtualKeyCode: 35, nativeVirtualKeyCode: 35 });
    await sleep(300);
    const length = [..."helloé你😀"].join("").length;
    await modifierKey(popup, named.Left);
    await modifierKey(popup, named.Left);
    await withDiag(popup, async () => {
      // An emoji is two UTF-16 units and one Left: the caret is two code units plus one unit left of the end.
      const selection = (await state()).views[popupId].selection;
      same(selection[0], selection[1], "a collapsed caret");
      same(selection[0], length - 3, "the caret after two Left presses over 'é你😀'");
    });
  });

  await check("Backspace: removes the character before the caret", async () => {
    await modifierKey(popup, named.Backspace);
    await withDiag(popup, async () => same((await state()).views[popupId].a, "hello你😀", "the field after Backspace removed the é before the caret"));
  });

  await check("Enter: the field's submit action fires once", async () => {
    await opener.run("window.probeReset()");
    await modifierKey(popup, named.Enter);
    await withDiag(popup, async () => same((await state()).views[popupId].submitted, 1, "submitted"));
  });

  await check("Ctrl shortcut: Ctrl+K reaches Shortcuts/Actions", async () => {
    await modifierKey(popup, named.K);
    await withDiag(popup, async () => {
      const now = await state();
      same(now.views[popupId].pings, 1, `pings (the framework heard ${JSON.stringify(now.keys.slice(-6))})`);
    });
  });

  await check("Tab: moves the focus from the first field to the second", async () => {
    await modifierKey(popup, named.Tab);
    await withDiag(popup, async () => {
      const view = (await state()).views[popupId];
      same([view.aFocus, view.bFocus], [false, true], "focus of the first and second field");
    });
  });

  /** A composition in a field of [view] of [page]: the composing range while it is open, then no range after the commit. */
  async function composeIn(page, id, field) {
    let seen;
    const view = (await state()).views[id];
    await click(page, ...center(view.rects[field]));
    await page.call("Input.imeSetComposition", { text: "ni", selectionStart: 2, selectionEnd: 2 });
    await sleep(500);
    await withDiag(page, async () => {
      const during = (await state()).views[id];
      same(during[field], "ni", "the field during composition");
      seen = during.composing;
    });
    await page.call("Input.imeSetComposition", { text: "に", selectionStart: 1, selectionEnd: 1 });
    await sleep(300);
    await page.call("Input.insertText", { text: "に" });
    await sleep(500);
    await withDiag(page, async () => {
      const after = (await state()).views[id];
      same(after[field], "に", "the field after the commit");
      same(after.composing, [-1, -1], "no composing range after the commit");
    });
    return `composing range seen during the composition: ${JSON.stringify(seen)}`;
  }

  await check("IME: a composition shows in the field, then commits (the composing range is read too: the control says whether Flutter reports it at all)", () => composeIn(popup, popupId, "b"));

  await check("clipboard: a paste key puts the clipboard's text in the field", async () => {
    await popup.run("navigator.clipboard.writeText('pasted')", { gesture: true });
    const before = (await state()).views[popupId].b;
    await modifierKey(popup, named.V);
    await withDiag(popup, async () => same((await state()).views[popupId].b, before + "pasted", "the field"));
  });

  await check("control: the same composition in the opener's own view (a void result means the protocol's IME does not drive this page)", async () => {
    await front(opener);
    await composeIn(opener, ownId, "b");
    await front(popup);
  });

  await check("control: Tab in the opener's own view, after the shims' keys (what a single window does)", async () => {
    await front(opener);
    const own = (await state()).views[ownId];
    await click(opener, ...center(own.rects.a));
    await modifierKey(opener, named.Tab);
    await withDiag(opener, async () => {
      const view = (await state()).views[ownId];
      same([view.aFocus, view.bFocus], [false, true], "focus of the opener's first and second field");
    });
    return `events: ${(await opener.run("window.probeTrail(40)")).split(" ").filter((each) => /Tab|focus/.test(each)).slice(-9).join(" ")}`;
    await front(popup);
  });

  await check("focus and blur: after the other window was used, typing in this one reaches its field", async () => {
    await front(opener);
    const own = (await state()).views[ownId];
    await click(opener, ...center(own.rects.a));
    await typeAscii(opener, "q");
    let now = await state();
    await withDiag(opener, async () => same(now.views[ownId].a, "q", "the opener's field after typing there"));
    await front(popup);
    const view = now.views[popupId];
    const before = view.a;
    await click(popup, ...center(view.rects.a));
    await typeAscii(popup, "z");
    now = await state();
    await withDiag(popup, async () => {
      same(now.views[popupId].a, before + "z", `the popup's field after typing there (framework focus of the popup's and the opener's first field: ${JSON.stringify([now.views[popupId].aFocus, now.views[ownId].aFocus])})`);
      same(now.views[ownId].a, "q", "the opener's field is left alone");
    });
  });

  await check("pixel ratio: the popup's view uses its own window's device pixel ratio", async () => {
    await popup.call("Emulation.setDeviceMetricsOverride", { width: 640, height: 520, deviceScaleFactor: 2, mobile: false });
    await sleep(1200);
    const own = await popup.run("devicePixelRatio");
    const ratio = (await state()).views[popupId].ratio;
    await popup.call("Emulation.clearDeviceMetricsOverride");
    await sleep(500);
    same([own, ratio], [2, 2], "the popup window's ratio, the view's ratio");
  });

  await check("frames: they keep coming with the opener minimized (the popup's own animation frames as the control)", async () => {
    await popup.run("window.__raf = 0; (function loop() { window.__raf++; requestAnimationFrame(loop); })()");
    const { windowId } = await root.call("Browser.getWindowForTarget", { targetId: opener.id });
    await root.call("Browser.setWindowBounds", { windowId, bounds: { windowState: "minimized" } });
    await sleep(1000);
    const hidden = await opener.run("window.realVisibility()");
    try {
      if (hidden !== "hidden") throw new Error(`could not hide the opener: its real visibilityState is ${hidden}`);
      const before = [(await state()).frames, await popup.run("window.__raf"), await popup.run("document.visibilityState")];
      await sleep(1500);
      const after = [(await state()).frames, await popup.run("window.__raf")];
      if (after[1] - before[1] < 20) throw new Error(`the popup itself drew no frames (${before[1]} to ${after[1]}, visible: ${before[2]}): the control is void`);
      if (after[0] - before[0] < 20) throw new Error(`the app's frames went ${before[0]} to ${after[0]} while the popup's own went ${before[1]} to ${after[1]}`);
      return `app frames ${before[0]} to ${after[0]}, popup's own ${before[1]} to ${after[1]}`;
    } finally {
      await root.call("Browser.setWindowBounds", { windowId, bounds: { windowState: "normal" } });
      await sleep(500);
    }
  });

  await check("close: closing the popup window removes its view and leaves the opener's running", async () => {
    const closing = popup;
    await fetch(`http://127.0.0.1:${debug}/json/close/${closing.id}`);
    await waitFor("the view removed", async () => {
      const now = await state();
      return now.popupViews.length === 0 && Object.keys(now.views).length === 1;
    }, 5000);
    const before = (await state()).frames;
    await sleep(500);
    if ((await state()).frames <= before) throw new Error("the opener stopped drawing");
  });

  await check("close: removeView then close (what destroy does) removes the view and the window", async () => {
    await openPopup();
    await front(opener);
    await opener.run("window.probeClose()");
    await waitFor("the view and window gone", async () => {
      const now = await state();
      return now.popupViews.length === 0 && Object.keys(now.views).length === 1 && !(await targets()).some((each) => each.id === popup.id);
    }, 5000);
  });

  await check("close: the opener's pagehide closes every popup", async () => {
    await openPopup();
    const second = popup.id;
    await front(opener);
    await opener.call("Page.navigate", { url: "about:blank" });
    await waitFor("the popup gone", async () => !(await targets()).some((each) => each.id === second), 8000);
  });

  root.close();
  chrome.kill();
  await sleep(800);
  return results;
}

try {
  const all = [];
  for (const shims of (sets ?? "- a b c d e ab abc abcd abcde").split(/\s+/).filter(Boolean).map((each) => (each === "-" ? "" : each))) {
    all.push({ shims, results: await runSet(shims) });
  }
  const items = all[0].results.map((each) => each.item);
  console.log("\nitem x shim set");
  console.log(["item", ...all.map((each) => each.shims || "-")].join(" | "));
  for (const item of items) console.log([item.slice(0, 60), ...all.map((each) => (each.results.find((r) => r.item === item)?.ok ? "ok" : "FAIL"))].join(" | "));
  if (out) writeFileSync(out, JSON.stringify(all, null, 2));
  const required = require === undefined ? null : all.find((each) => each.shims === (require === "-" ? "" : require));
  exitCode = required ? (required.results.every((each) => each.ok) ? 0 : 1) : 0;
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
    // The browser may still hold its profile; the folder is in the temporary directory.
  }
  console.log(exitCode === 0 ? "DONE" : "FAILED");
  process.exit(exitCode);
}
