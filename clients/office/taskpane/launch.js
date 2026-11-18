// Starting the Canoa on this computer from the task pane, and the secret a manifest rendered for a shared computer carries. Pure:
// it takes `fetch` and the clock as arguments and imports nothing from Office.js, so it runs under `node --test`.
//
// A page cannot start a program. What it can do is ask the small launcher that holds this address while no host runs
// (`canoa office-launcher`), which answers `/canoa.json` with `{"launcher": "/office/launch"}` and starts the host the page asks it
// to. A host that runs answers `/canoa.json` with its entries.

/** Where the launcher is asked to start a host. The launcher's own index names it; this is the only value taken. */
export const LAUNCH_PATH = '/office/launch';
/** The header the secret travels in, and the address parameter the manifest carries it in. */
export const KEY_HEADER = 'x-canoa-office-key';
export const KEY_PARAMETER = 'office_key';

/**
 * The secret in a pane's address (`?office_key=<64 hex digits>`), or undefined. Only that shape is taken: the page sends it on,
 * and a value of any other shape is not one the host made.
 * @param {string} search  `location.search`
 * @returns {string|undefined}
 */
export function keyOf(search) {
  const key = new URLSearchParams(search).get(KEY_PARAMETER);
  return key !== null && /^[0-9a-fA-F]{64}$/.test(key) ? key.toLowerCase() : undefined;
}

/**
 * The headers of a request to this host's own routes.
 * @param {string|undefined} key
 * @param {Record<string, string>} [more]
 * @returns {Record<string, string>}
 */
export function headersFor(key, more = {}) {
  return key === undefined ? { ...more } : { ...more, [KEY_HEADER]: key };
}

/**
 * What a `/canoa.json` answer says of what answered: `host` (a Canoa lists its entries), `launcher` (nothing runs; this offers to
 * start it) or `unknown` (anything else, or no answer).
 * @param {unknown} index
 * @returns {'host'|'launcher'|'unknown'}
 */
export function indexKind(index) {
  if (index === null || typeof index !== 'object') return 'unknown';
  if (index.launcher === LAUNCH_PATH) return 'launcher';
  return Array.isArray(index.entries) ? 'host' : 'unknown';
}

/**
 * What answers at this address now.
 * @param {typeof fetch} fetchImpl
 * @returns {Promise<'host'|'launcher'|'unknown'>}
 */
export async function whatAnswers(fetchImpl = fetch) {
  try {
    const response = await fetchImpl('/canoa.json', { cache: 'no-store' });
    return response.ok ? indexKind(await response.json()) : 'unknown';
  } catch {
    return 'unknown';
  }
}

/**
 * Asks the launcher to start a host.
 * @param {{key?: string, fetchImpl?: typeof fetch}} options
 * @returns {Promise<{started: boolean, said?: string}>}
 */
export async function startHost({ key, fetchImpl = fetch } = {}) {
  try {
    const response = await fetchImpl(LAUNCH_PATH, { method: 'POST', credentials: 'same-origin', headers: headersFor(key) });
    if (response.ok) return { started: true };
    let said;
    try {
      said = (await response.json()).said;
    } catch {
      said = undefined;
    }
    return { started: false, said: typeof said === 'string' ? said : `the launcher answered ${response.status}` };
  } catch {
    return { started: false, said: 'the launcher did not answer' };
  }
}

/**
 * Waits for a host to answer at this address, looking every `every` milliseconds up to `tries` times.
 * @param {{fetchImpl?: typeof fetch, sleep?: (ms: number) => Promise<void>, tries?: number, every?: number}} options
 * @returns {Promise<boolean>}
 */
export async function waitForHost({ fetchImpl = fetch, sleep = (ms) => new Promise((done) => setTimeout(done, ms)), tries = 60, every = 1000 } = {}) {
  for (let look = 0; look < tries; look += 1) {
    if ((await whatAnswers(fetchImpl)) === 'host') return true;
    await sleep(every);
  }
  return false;
}

/**
 * The whole offer: asks the launcher to start a host and waits for it to answer. The result says which step ended it.
 * @param {{key?: string, fetchImpl?: typeof fetch, sleep?: (ms: number) => Promise<void>, tries?: number, every?: number}} options
 * @returns {Promise<{ok: boolean, said: string}>}
 */
export async function offerStart(options = {}) {
  const asked = await startHost(options);
  if (!asked.started) return { ok: false, said: asked.said ?? 'Canoa was not started' };
  return (await waitForHost(options))
    ? { ok: true, said: 'Canoa is running' }
    : { ok: false, said: 'Canoa was asked to start and has not answered yet' };
}
