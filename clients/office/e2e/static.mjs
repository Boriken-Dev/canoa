// The add-in's pages as a static site (`python app/tool/build.py office`), served from nothing but a folder, in a real Chromium
// browser: the task pane loads Office.js from the site and the chat-only app starts, and every request of both goes to the site's
// own origin. The browser resolves no name but localhost, so a request to any other host would fail the run.
//
//   CANOA_E2E_BROWSER=path/to/msedge CANOA_E2E_SITE=target/canoa-office node clients/office/e2e/static.mjs
import { spawn } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { createServer } from 'node:http';
import { tmpdir } from 'node:os';
import path from 'node:path';

import { lastCall, Session } from '../../browser/chromium/e2e/cdp.mjs';

const browser = process.env.CANOA_E2E_BROWSER;
const site = process.env.CANOA_E2E_SITE && path.resolve(process.env.CANOA_E2E_SITE);
if (!browser || !site) throw new Error('set CANOA_E2E_BROWSER and CANOA_E2E_SITE');

const TYPES = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.mjs': 'text/javascript; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.wasm': 'application/wasm',
  '.png': 'image/png',
  '.svg': 'image/svg+xml',
  '.ttf': 'font/ttf',
  '.otf': 'font/otf',
  '.woff2': 'font/woff2',
};
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const work = mkdtempSync(path.join(tmpdir(), 'canoa-office-static-e2e-'));

const server = createServer((request, response) => {
  let pathname = decodeURIComponent(new URL(request.url, 'http://localhost').pathname);
  if (pathname.endsWith('/')) pathname += 'index.html';
  const file = path.join(site, pathname);
  if (pathname.includes('..') || !file.startsWith(site)) {
    response.writeHead(400);
    response.end();
    return;
  }
  try {
    const body = readFileSync(file);
    response.writeHead(200, { 'content-type': TYPES[path.extname(file)] ?? 'application/octet-stream' });
    response.end(body);
  } catch {
    response.writeHead(404);
    response.end();
  }
});
await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
const origin = `http://localhost:${server.address().port}`;

async function waitFor(what, test, ms = 30_000) {
  const until = Date.now() + ms;
  for (;;) {
    const value = await test();
    if (value) return value;
    if (Date.now() > until) throw new Error(`timed out waiting for ${what}`);
    await sleep(250);
  }
}

const results = [];
async function check(name, run) {
  try {
    await run();
    results.push(`ok   ${name}`);
  } catch (error) {
    results.push(`FAIL ${name}: ${String(error.message).slice(0, 600)}`);
  }
  console.log(results.at(-1));
}

let chrome;
let exitCode = 1;
const watchdog = setTimeout(() => {
  console.log(`FAILED: the run took too long, at: ${lastCall()}`);
  chrome?.kill();
  process.exit(1);
}, 240_000);
try {
  const profile = path.join(work, 'profile');
  chrome = spawn(browser, [
    '--headless=new', `--user-data-dir=${profile}`, '--remote-debugging-port=0', '--no-first-run', '--no-default-browser-check',
    '--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE localhost, EXCLUDE 127.0.0.1', '--window-size=1000,800', 'about:blank',
  ], { stdio: 'ignore' });
  const debug = await waitFor('the browser', () => {
    try {
      return readFileSync(path.join(profile, 'DevToolsActivePort'), 'utf8').split('\n')[0];
    } catch {
      return null;
    }
  });
  const open = async (url) => {
    const made = await (await fetch(`http://127.0.0.1:${debug}/json/new?about:blank`, { method: 'PUT' })).json();
    const page = new Session(made.webSocketDebuggerUrl);
    await page.call('Page.enable');
    await page.call('Runtime.enable');
    await page.call('Page.addScriptToEvaluateOnNewDocument', {
      source: 'window.__refused = []; document.addEventListener("securitypolicyviolation", (e) => window.__refused.push(e.violatedDirective + " " + e.blockedURI));',
    });
    await page.call('Page.navigate', { url });
    return page;
  };
  const resources = (page) => page.run('performance.getEntriesByType("resource").map((entry) => entry.name)');

  await check('the task pane loads Office.js from the site and asks for nothing from any other origin', async () => {
    const pane = await open(`${origin}/office/taskpane.html`);
    await waitFor('the pane', () => pane.run('document.readyState === "complete"').catch(() => false));
    await sleep(2000);
    if ((await pane.run('typeof Office')) !== 'object') throw new Error('Office.js is not defined in the pane');
    const loaded = await resources(pane);
    const elsewhere = loaded.filter((name) => !name.startsWith(`${origin}/`));
    if (elsewhere.length) throw new Error(`the pane asked for ${elsewhere.join(', ')}`);
    if (!loaded.includes(`${origin}/office/js/office.js`)) throw new Error(`Office.js was not loaded from the site: ${loaded.join(', ')}`);
    pane.close();
  });

  await check('the chat-only app starts from the site with nothing refused and every request to the site', async () => {
    const app = await open(`${origin}/office/app/?mode=chat`);
    await waitFor("Flutter's view", () => app.run("Boolean(document.querySelector('flutter-view, flt-glass-pane'))").catch(() => false), 60_000);
    await sleep(2000);
    const refused = await app.run('window.__refused');
    if (refused.length) throw new Error(`refused: ${refused.join('; ')}`);
    const elsewhere = (await resources(app)).filter((name) => !name.startsWith(`${origin}/`));
    if (elsewhere.length) throw new Error(`the app asked for ${elsewhere.join(', ')}`);
    app.close();
  });

  exitCode = results.every((line) => line.startsWith('ok')) ? 0 : 1;
} catch (error) {
  console.log(`FAILED: ${error.message}`);
} finally {
  clearTimeout(watchdog);
  chrome?.kill();
  server.close();
  await sleep(1500);
  rmSync(work, { recursive: true, force: true, maxRetries: 10, retryDelay: 500 });
}
console.log(exitCode === 0 ? 'PASSED' : 'FAILED');
process.exit(exitCode);
