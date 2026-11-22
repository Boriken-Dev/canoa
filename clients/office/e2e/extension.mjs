// A framed Office task pane with the Canoa extension in a real Chromium browser, with zero `[host.web]` loopback server:
//   - Outer Office shell origin (`127.0.0.1`) frames `/office/taskpane.html` on the pane origin (`localhost`), which frames `/office/app/`.
//   - Only the pane origin is granted (`allFrames: true`), so `document.acp` reaches the pane and the app frames while the outer shell has none.
//   - With the extension: the pane detects `document.acp` offering `dev.boriken.canoa`, registers its Word tools over `@tools`,
//     the app starts a session on the local host's scripted agent, runs a Word write tool confirmed in the pane's `#ask` prompt, and shows the result.
//   - Without the extension (or on an ungranted pane origin): the pane falls back cleanly and says so.
//
//   CANOA_E2E_BROWSER=path/to/msedge.exe CANOA_E2E_HOST=path/to/canoa CANOA_E2E_AGENT=path/to/stdio_agent \
//   CANOA_E2E_WEB=app/build/web [CANOA_E2E_NATIVE_BROWSER=edge|chrome|chromium] node clients/office/e2e/extension.mjs
import assert from 'node:assert/strict';
import { execFileSync, spawn } from 'node:child_process';
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { createServer } from 'node:http';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { findCanoaWorker, lastCall, Session } from '../../browser/chromium/e2e/cdp.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const officeDir = path.resolve(here, '..');
const extension = path.resolve(officeDir, '../browser/chromium');
const absolute = (file) => (file ? path.resolve(file) : file);
const browser = process.env.CANOA_E2E_BROWSER;
const canoa = absolute(process.env.CANOA_E2E_HOST);
const agent = absolute(process.env.CANOA_E2E_AGENT);
const web = absolute(process.env.CANOA_E2E_WEB);
if (!browser || !canoa || !agent || !web) {
  throw new Error('set CANOA_E2E_BROWSER, CANOA_E2E_HOST, CANOA_E2E_AGENT and CANOA_E2E_WEB');
}
const NATIVE_BROWSER = process.env.CANOA_E2E_NATIVE_BROWSER ?? 'edge';
const HOST_NAME = 'dev.boriken.canoa';
const KEYS = { edge: 'Microsoft\\Edge', chrome: 'Google\\Chrome', chromium: 'Chromium' };

// Preserve any existing Windows native-messaging entry across the run so developer machines are left unchanged.
let prevRegistry = null;
const regKey = process.platform === 'win32' ? `HKCU\\Software\\${KEYS[NATIVE_BROWSER]}\\NativeMessagingHosts\\${HOST_NAME}` : null;
if (regKey !== null) {
  try {
    const out = execFileSync('reg', ['query', regKey, '/ve'], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] });
    const match = out.match(/REG_SZ\s+(.*)$/m);
    if (match) prevRegistry = match[1].trim();
  } catch {
    prevRegistry = null;
  }
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const work = mkdtempSync(path.join(tmpdir(), 'canoa-office-ext-e2e-'));
const home = path.join(work, 'home');
const documents = path.join(work, 'documents');
const profile = path.join(work, 'profile');
const unpacked = path.join(work, 'extension');
const config = path.join(work, 'canoa.cfg');
mkdirSync(home);
mkdirSync(documents);

const OFFICE_STUB = `
window.__wordCalls = [];
window.Office = {
  AsyncResultStatus: { Succeeded: 'succeeded', Failed: 'failed' },
  HostType: { Word: 'Word', Excel: 'Excel', Outlook: 'Outlook' },
  context: { document: { url: 'https://contoso.example/Shared/Plan.docx' } },
  onReady: (fn) => setTimeout(() => fn({ host: 'Word' }), 0),
};
window.Word = {
  run: async (work) => {
    const context = {
      sync: async () => {},
      document: {
        body: {
          load() {},
          text: 'Document body.',
          insertText(text, where) { window.__wordCalls.push(['body.insertText', text, where]); },
          getComments() { return { load() {}, items: [] }; },
        },
        getSelection() {
          return {
            load() {},
            text: '',
            insertText(text, where) { window.__wordCalls.push(['selection.insertText', text, where]); },
          };
        },
      },
    };
    return work(context);
  },
};
`;

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

function startStaticPaneServer(host) {
  const taskpaneDir = path.join(officeDir, 'taskpane');
  const server = createServer((request, response) => {
    const pathname = new URL(request.url, 'http://localhost').pathname;
    if (pathname === '/office/js/office.js') {
      response.writeHead(200, { 'content-type': 'text/javascript; charset=utf-8' });
      response.end(OFFICE_STUB);
      return;
    }
    if (pathname === '/office/taskpane.html') {
      const html = readFileSync(path.join(taskpaneDir, 'taskpane.html'), 'utf8').replace('@OFFICE_JS@', '/office/js/office.js');
      response.writeHead(200, { 'content-type': 'text/html; charset=utf-8' });
      response.end(html);
      return;
    }
    if (pathname.startsWith('/office/') && !pathname.startsWith('/office/app/')) {
      const rel = pathname.slice('/office/'.length);
      if (rel && !rel.includes('..') && !rel.includes('\\')) {
        try {
          const body = readFileSync(path.join(taskpaneDir, rel));
          response.writeHead(200, { 'content-type': TYPES[path.extname(rel)] ?? 'application/octet-stream' });
          response.end(body);
          return;
        } catch {
          // Fall through to 404.
        }
      }
    }
    if (pathname === '/office/app' || pathname.startsWith('/office/app/')) {
      let rel = pathname === '/office/app' || pathname === '/office/app/' ? 'index.html' : pathname.slice('/office/app/'.length);
      if (!rel.includes('..') && !rel.includes('\\')) {
        try {
          let body = readFileSync(path.join(web, rel));
          if (rel === 'index.html') {
            body = Buffer.from(body.toString('utf8').replace(/<base href="[^"]*">/, '<base href="/office/app/">'));
          }
          response.writeHead(200, { 'content-type': TYPES[path.extname(rel)] ?? 'application/octet-stream' });
          response.end(body);
          return;
        } catch {
          // Fall through to 404.
        }
      }
    }
    response.writeHead(404);
    response.end();
  });
  return new Promise((resolve) => {
    server.listen(0, host, () => resolve({ server, port: server.address().port }));
  });
}

const paneServer = await startStaticPaneServer(undefined);
const paneOrigin = `http://localhost:${paneServer.port}`;
// Ungranted mirror of the exact same static server on 127.0.0.1 (never granted in the extension).
const ungrantedOrigin = `http://127.0.0.1:${paneServer.port}`;

const outerServer = createServer((request, response) => {
  const url = new URL(request.url, 'http://127.0.0.1');
  const paneSrc = url.searchParams.get('pane') ?? `${paneOrigin}/office/taskpane.html`;
  response.writeHead(200, { 'content-type': 'text/html; charset=utf-8' });
  response.end(`<!doctype html><html><body style="margin:0;height:100vh;display:flex">
    <iframe id="pane" src="${paneSrc}" style="border:0;width:440px;height:760px"></iframe>
  </body></html>`);
});
await new Promise((resolve) => outerServer.listen(0, '127.0.0.1', resolve));
const outerOrigin = `http://127.0.0.1:${outerServer.address().port}`;

// Note: zero [host.web] configuration on the host — no loopback HTTP/HTTPS listener for the web!
writeFileSync(config, [
  '[host.agents.scripted]',
  `program = ${JSON.stringify(agent)}`,
  'args = ["--scripted", "--mcp-http"]',
  'title = "Scripted"',
  '',
].join('\n'));

cpSync(extension, unpacked, { recursive: true, filter: (from) => !/(node_modules|[/\\]e2e|[/\\]ui|\.test\.mjs)/.test(path.relative(extension, from) || 'x') });
const manifest = JSON.parse(readFileSync(path.join(unpacked, 'manifest.json'), 'utf8'));
delete manifest.side_panel;
manifest.host_permissions = ['http://localhost/*'];
writeFileSync(path.join(unpacked, 'manifest.json'), JSON.stringify(manifest));

const env = { ...process.env, COQUI_AGENTIC_PEER_HOME: home, XDG_DOCUMENTS_DIR: documents };
const runHost = (args) => execFileSync(canoa, args, { encoding: 'utf8', env });
const serveArgs = ['serve', '--config', config, '--method', 'noise'];

async function waitFor(what, test, ms = 30_000) {
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
    results.push(`FAIL ${name}: ${String(error.message).slice(0, 800)}`);
  }
  console.log(results.at(-1));
}

let hostProcess;
let hostOut = '';
let chrome;
let debug;
let registered = false;
let exitCode = 1;
const watchdog = setTimeout(() => {
  console.log(`FAILED: watchdog timeout at ${lastCall()}`);
  chrome?.kill();
  hostProcess?.kill();
  process.exit(1);
}, 300_000);

try {
  hostProcess = spawn(canoa, serveArgs, { env, stdio: ['pipe', 'pipe', 'pipe'] });
  hostOut = '';
  hostProcess.stdout.on('data', (chunk) => { hostOut += chunk; });
  hostProcess.stderr.on('data', (chunk) => { hostOut += chunk; });
  await waitFor('host ready', () => hostOut.includes('"ready"'), 30_000);

  runHost(['native', 'install', '--browser', NATIVE_BROWSER, '--launch', canoa, '--', ...serveArgs]);
  registered = true;

  chrome = spawn(browser, [
    '--headless=new', `--user-data-dir=${profile}`, '--remote-debugging-port=0', '--no-first-run', '--no-default-browser-check',
    `--load-extension=${unpacked}`, `--disable-extensions-except=${unpacked}`,
    '--disable-features=DisableLoadExtensionCommandLineSwitch', '--window-size=1000,820', 'about:blank',
  ], { stdio: 'ignore', env });
  debug = await waitFor('browser DevTools port', () => {
    try {
      return readFileSync(path.join(profile, 'DevToolsActivePort'), 'utf8').split('\n')[0];
    } catch {
      return null;
    }
  });
  const targets = async () => (await fetch(`http://127.0.0.1:${debug}/json/list`)).json();
  const worker = await waitFor('extension service worker', async () => findCanoaWorker(await targets()), 20_000);
  const sw = new Session(worker.webSocketDebuggerUrl);
  await waitFor('chrome APIs', async () => (await sw.run('typeof chrome')) === 'object');
  const extId = new URL(worker.url).hostname;

  const openTab = async (url) => {
    const made = await (await fetch(`http://127.0.0.1:${debug}/json/new?${encodeURIComponent(url)}`, { method: 'PUT' })).json();
    const s = new Session(made.webSocketDebuggerUrl);
    await s.call('Page.enable');
    await s.call('Runtime.enable');
    await s.call('Page.bringToFront');
    return s;
  };

  // Pair extension with native host by PIN.
  const opts = await openTab(`chrome-extension://${extId}/permissions.html`);
  const pairing = spawn(canoa, ['pair', '--native', '--method', 'noise'], { env, stdio: ['pipe', 'pipe', 'ignore'] });
  let pinLine = '';
  pairing.stdout.on('data', (chunk) => { pinLine += chunk; });
  const shown = JSON.parse(await waitFor('PIN', () => (pinLine.includes('\n') ? pinLine.split('\n')[0] : null), 15_000));
  const paired = await opts.run(`chrome.runtime.sendMessage(${JSON.stringify({ type: 'canoa.host.pair', pin: shown.pin })})`);
  assert.equal(paired.paired, true, JSON.stringify(paired));
  opts.close();

  // Grant only the pane origin (allFrames + matchOriginAsFallback) — outer Office shell origin is untouched.
  await sw.run(`chrome.scripting.registerContentScripts([
    { id: 'office-pane-main', matches: ['${paneOrigin}/*'], js: ['acp-page.js'], runAt: 'document_start', world: 'MAIN', allFrames: true, matchOriginAsFallback: true },
    { id: 'office-pane-iso', matches: ['${paneOrigin}/*'], js: ['acp-content.js'], runAt: 'document_start', world: 'ISOLATED', allFrames: true, matchOriginAsFallback: true },
  ]).then(() => true)`);

  // Case 1: ungranted origin with zero /canoa.json falls back cleanly and says so.
  await check('without extension on ungranted static origin, pane falls back and says so', async () => {
    const tab = await openTab(`${outerOrigin}/?pane=${encodeURIComponent(`${ungrantedOrigin}/office/taskpane.html`)}`);
    const ungrantedPaneCtx = await waitFor('ungranted pane context', async () => {
      for (const ctx of tab.contexts.filter((c) => c.auxData?.isDefault && c.origin === ungrantedOrigin)) {
        const ready = await tab.run('typeof Office === "object" && Boolean(document.getElementById("status"))', { context: ctx.id }).catch(() => false);
        if (ready) return ctx.id;
      }
      return null;
    });
    const statusText = await waitFor('fallback status', async () => {
      const text = await tab.run('document.getElementById("status").textContent', { context: ungrantedPaneCtx }).catch(() => '');
      return text.includes('not enabled') ? text : null;
    });
    assert.match(statusText, /Canoa is not running here, and the Canoa browser extension is not enabled for this site/);
    assert.equal(await tab.run('typeof document.acp', { context: ungrantedPaneCtx }), 'undefined');
    assert.equal(await tab.run('document.getElementById("canoa").hidden', { context: ungrantedPaneCtx }), true);
    tab.close();
  });

  // Case 2: granted pane origin framed inside outer shell chats with local agent and runs a Word tool through the pane.
  const tab = await openTab(`${outerOrigin}/`);
  let paneSession;

  await check('frames: extension injects pane + nested app frames without touching outer Office shell', async () => {
    await waitFor('outer frame ready', () => tab.run('document.readyState === "complete" && Boolean(document.getElementById("pane"))').catch(() => false));
    // `localhost` vs `127.0.0.1` is cross-site, so Edge puts the pane iframe and its same-origin app iframe into an OOPIF target.
    const paneTarget = await waitFor('pane iframe target', async () => (await targets()).find((t) => t.url.startsWith(`${paneOrigin}/office/taskpane.html`)), 30_000);
    paneSession = new Session(paneTarget.webSocketDebuggerUrl);
    await paneSession.call('Runtime.enable');
    await waitFor('pane Office ready', () => paneSession.run('typeof Office === "object" && Boolean(document.getElementById("canoa"))').catch(() => false), 30_000);
    await waitFor('Flutter app in nested iframe', () => paneSession.run('Boolean(document.getElementById("canoa").contentDocument?.querySelector("flutter-view, flt-glass-pane"))').catch(() => false), 60_000);

    assert.equal(await tab.run('typeof document.acp'), 'undefined');
    assert.deepEqual(await paneSession.run('document.acp.providers.map((p) => p.rdns)'), ['dev.boriken.canoa']);
    assert.deepEqual(
      await paneSession.run('document.getElementById("canoa").contentDocument.acp.providers.map((p) => p.rdns)'),
      ['dev.boriken.canoa'],
    );
    const barText = await waitFor('pane status text', async () => {
      const text = await paneSession.run('document.getElementById("status").textContent');
      return text.includes('Plan.docx') ? text : null;
    });
    assert.match(barText, /Sessions of Plan\.docx first/);
  });

  await check('chat & Word write prompt: framed app opens session on local host, calls word_insert_text, confirmed in pane', async () => {
    // Enable Flutter web accessibility tree inside the nested app iframe.
    await waitFor('semantics placeholder', () => paneSession.run('Boolean(document.getElementById("canoa").contentDocument.querySelector("flt-semantics-placeholder"))').catch(() => false), 30_000);
    await paneSession.run('document.getElementById("canoa").contentDocument.querySelector("flt-semantics-placeholder")?.click()', { gesture: true });

    // Locate "New session" inside the nested app iframe and compute its viewport coordinates in the tab.
    const labels = () => paneSession.run(`[...document.getElementById('canoa').contentDocument.querySelectorAll('flt-semantics')].map((n) => n.getAttribute('aria-label') || n.textContent).join(' | ')`).catch(() => '');
    const box = await waitFor('New session semantics node', async () => {
      const inner = await paneSession.run(`(() => {
        const doc = document.getElementById('canoa').contentDocument;
        const area = (n) => { const r = n.getBoundingClientRect(); return r.width * r.height; };
        const node = [...doc.querySelectorAll('flt-semantics')]
          .filter((n) => (n.getAttribute('aria-label') || n.textContent).includes('New session'))
          .sort((a, b) => area(a) - area(b))[0];
        if (!node) return null;
        const r = node.getBoundingClientRect();
        if (r.width === 0 || r.height === 0) return null;
        const appRect = document.getElementById('canoa').getBoundingClientRect();
        return { x: appRect.left + r.left + r.width / 2, y: appRect.top + r.top + r.height / 2 };
      })()`).catch(() => null);
      if (!inner) return null;
      const paneRect = await tab.run(`(() => { const r = document.getElementById('pane').getBoundingClientRect(); return { left: r.left, top: r.top }; })()`);
      return { x: paneRect.left + inner.x, y: paneRect.top + inner.y };
    }, 30_000).catch(async (error) => {
      throw new Error(`${error.message}; the app shows: ${(await labels()).slice(0, 600)}`);
    });

    for (const type of ['mouseMoved', 'mousePressed', 'mouseReleased']) {
      await tab.call('Input.dispatchMouseEvent', { type, x: box.x, y: box.y, button: 'left', buttons: type === 'mousePressed' ? 1 : 0, clickCount: 1 });
    }
    // Grant "Scripted" in the extension's permission window.
    const reqTarget = await waitFor('permission window', async () => (await targets()).find((t) => t.url.includes('acp-request.html?ask=')), 20_000);
    const reqView = new Session(reqTarget.webSocketDebuggerUrl);
    await waitFor('Scripted entry', () => reqView.run("[...document.querySelectorAll('label.pick')].some((l) => l.textContent.startsWith('Scripted'))").catch(() => false), 15_000);
    await waitFor('buttons enabled', () => reqView.run("![...document.querySelectorAll('button, input[type=checkbox]')].some((c) => c.disabled)"), 15_000);
    await reqView.run("[...document.querySelectorAll('label.pick')].find((l) => l.textContent.startsWith('Scripted')).querySelector('input').click()", { gesture: true });
    await reqView.run("[...document.querySelectorAll('button')].find((b) => b.textContent.startsWith('Allow')).click()", { gesture: true }).catch(() => {});

    // Wait for composer text field in app iframe and focus + type CallOn prompt.
    const composer = await waitFor('composer input in app', async () => {
      const inner = await paneSession.run(`(() => {
        const doc = document.getElementById('canoa').contentDocument;
        const el = doc.querySelector('input[type="text"], textarea, [contenteditable="true"], flt-semantics input, flt-semantics textarea, flt-semantics[role="textbox"]');
        if (!el) return null;
        const r = el.getBoundingClientRect();
        if (r.width === 0 || r.height === 0) return null;
        const appRect = document.getElementById('canoa').getBoundingClientRect();
        return { x: appRect.left + r.left + r.width / 2, y: appRect.top + r.top + r.height / 2 };
      })()`).catch(() => null);
      if (!inner) return null;
      const paneRect = await tab.run(`(() => { const r = document.getElementById('pane').getBoundingClientRect(); return { left: r.left, top: r.top }; })()`);
      return { x: paneRect.left + inner.x, y: paneRect.top + inner.y };
    }, 30_000).catch(async (error) => {
      const roles = await paneSession.run(`[...document.getElementById('canoa').contentDocument.querySelectorAll('flt-semantics')].map((n) => (n.getAttribute('role') || '-') + ':' + (n.getAttribute('aria-label') || n.textContent).slice(0, 60)).join(' | ')`).catch(() => '');
      throw new Error(`${error.message}; the app shows: ${roles.slice(0, 900)}`);
    });

    for (const type of ['mouseMoved', 'mousePressed', 'mouseReleased']) {
      await tab.call('Input.dispatchMouseEvent', { type, x: composer.x, y: composer.y, button: 'left', buttons: type === 'mousePressed' ? 1 : 0, clickCount: 1 });
    }
    const promptText = 'CallOn canoa-word word_insert_text {"text":"HelloFromWord","location":"document_end"}';
    // Flutter's text field takes the click's focus a moment later: typing at once can be lost.
    await sleep(700);
    await tab.call('Input.insertText', { text: promptText });
    await sleep(500);
    await tab.call('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Enter', code: 'Enter', windowsVirtualKeyCode: 13 });
    await tab.call('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Enter', code: 'Enter', windowsVirtualKeyCode: 13 });

    // Confirm the write prompt in the Office pane (#ask bar).
    const askText = await waitFor('pane write prompt', async () => {
      const hidden = await paneSession.run('document.getElementById("ask").hidden');
      return hidden ? null : paneSession.run('document.getElementById("ask-text").textContent');
    }, 30_000).catch(async (error) => {
      const shown = await labels();
      throw new Error(`${error.message}; the transcript ends: ${shown.slice(-450).split('\n').join(' ')}`);
    });
    assert.match(askText, /word_insert_text/);
    assert.match(askText, /Plan\.docx/);
    await paneSession.run('document.getElementById("ask-yes").click()', { gesture: true });

    // Verify Word document stub was written and scripted agent completed the turn with the tool answer.
    const wordCalls = await waitFor('Word document write call', async () => {
      const calls = await paneSession.run('window.__wordCalls');
      return calls.length > 0 ? calls : null;
    }, 20_000);
    assert.deepEqual(wordCalls, [['body.insertText', 'HelloFromWord', 'End']]);

    await waitFor('tool output in Flutter transcript', async () => {
      const text = await paneSession.run(`[...document.getElementById("canoa").contentDocument.querySelectorAll('flt-semantics')].map((n) => n.getAttribute('aria-label') || n.textContent).join('\\n')`);
      return text.includes('"inserted":13') ? text : null;
    }, 30_000);
  });

  paneSession?.close();
  tab.close();
  exitCode = results.every((line) => line.startsWith('ok')) ? 0 : 1;
} catch (error) {
  console.log(`FAILED: ${error.message}`);
} finally {
  clearTimeout(watchdog);
  if (regKey !== null) {
    if (prevRegistry !== null) {
      try {
        execFileSync('reg', ['add', regKey, '/ve', '/d', prevRegistry, '/f'], { stdio: 'ignore' });
      } catch {
        // Best-effort restore.
      }
    } else if (registered) {
      try {
        execFileSync(canoa, ['native', 'uninstall', '--browser', NATIVE_BROWSER], { env, stdio: 'ignore' });
      } catch {
        // Best-effort cleanup.
      }
    }
  }
  chrome?.kill();
  hostProcess?.stdin?.end();
  outerServer.close();
  paneServer.server.close();
  await sleep(1000);
  hostProcess?.kill();
  rmSync(work, { recursive: true, force: true, maxRetries: 10, retryDelay: 500 });
}

if (exitCode !== 0 && process.env.CANOA_E2E_HOST_LOG) console.log(hostOut.slice(-4000));
console.log(exitCode === 0 ? 'PASSED' : 'FAILED');
process.exit(exitCode);
