/**
 * Elevated Chromium CDP adapter. Only loaded in a deliberately debugger-
 * enabled manifest. An authenticated MCP peer must ALSO hold a tab/origin-
 * scoped browser.debug grant checked by browser-tools before reaching here.
 *
 * Never accept commands directly from website messages or page scripts.
 */
const commands = new Set([
  'Runtime.enable', 'Runtime.evaluate', 'Runtime.getProperties',
  'DOM.getDocument', 'DOM.querySelector', 'DOM.describeNode',
  'CSS.getComputedStyleForNode',
  'Log.enable', 'Network.enable', 'Network.getResponseBody',
  'Performance.enable', 'Performance.getMetrics',
  'Page.captureScreenshot',
]);
const attached = new Set();
const eventLog = new Map();
const EVENT_LIMIT = 120;
let listening = false;

function api() {
  if (!globalThis.chrome?.debugger) {
    throw Error('CDP requires the dedicated debugger-permission build');
  }
  return chrome.debugger;
}
function checkTab(tabId) {
  if (!Number.isSafeInteger(tabId) || tabId < 0) {
    throw TypeError('Invalid debugging tab');
  }
}
function checkCommand(method, params) {
  if (typeof method !== 'string' || !commands.has(method)) {
    throw Error('CDP method unavailable in this Canoa build');
  }
  if (!params || typeof params !== 'object' || Array.isArray(params)) {
    throw TypeError('Invalid DevTools parameters');
  }
  if (JSON.stringify(params).length > 32768) {
    throw Error('CDP parameters too large');
  }
  if (method === 'Runtime.evaluate' &&
      (typeof params.expression !== 'string' || params.expression.length > 8192)) {
    throw Error('Invalid evaluation expression');
  }
}
function watchDetach(debuggerApi) {
  if (listening) return;
  debuggerApi.onDetach.addListener((target) => {
    if (Number.isSafeInteger(target.tabId)) {
      attached.delete(target.tabId);
      eventLog.delete(target.tabId);
    }
  });
  debuggerApi.onEvent?.addListener((target, method, params) => {
    if (!Number.isSafeInteger(target.tabId) || !attached.has(target.tabId)) return;
    if (!['Runtime.consoleAPICalled','Log.entryAdded','Network.loadingFailed',
      'Network.responseReceived','Runtime.exceptionThrown'].includes(method)) return;
    const log = eventLog.get(target.tabId) ?? [];
    // Keep diagnostic metadata bounded. CDP event payloads can contain secrets.
    const json = JSON.stringify(params ?? {});
    log.push({method, value:json.slice(0,8192), capturedAt:Date.now()});
    if (log.length > EVENT_LIMIT) log.splice(0,log.length-EVENT_LIMIT);
    eventLog.set(target.tabId,log);
  });
  listening = true;
}
async function ensureAttached(tabId, debuggerApi) {
  if (attached.has(tabId)) return;
  watchDetach(debuggerApi);
  await debuggerApi.attach({tabId}, '1.3');
  attached.add(tabId);
}
export async function debugCommand({tabId, method, params = {}}, debuggerApi = api()) {
  checkTab(tabId);
  checkCommand(method, params);
  await ensureAttached(tabId, debuggerApi);
  // Return data is bounded before crossing a remote MCP transport.
  const result = await debuggerApi.sendCommand({tabId}, method, params);
  if (JSON.stringify(result).length > 1048576) {
    throw Error('CDP output exceeds Canoa response limit');
  }
  return result;
}
export async function debugDetach({tabId}, debuggerApi = api()) {
  checkTab(tabId);
  if (attached.has(tabId)) {
    await debuggerApi.detach({tabId});
    attached.delete(tabId);
  }
  eventLog.delete(tabId);
  return {detached: true};
}
export async function detachOnTabClosed(tabId) {
  if (!attached.has(tabId)) return;
  try { await debugDetach({tabId}); } catch { attached.delete(tabId); }
}
export function enabledCommands() { return [...commands]; }

/** Read bounded console, JavaScript exception and network failure activity.
 * First enable the Log/Runtime/Network CDP domains via browser_debug_command.
 */
export function debugEvents({tabId}) {
  checkTab(tabId);
  return {events:[...(eventLog.get(tabId) ?? [])]};
}
