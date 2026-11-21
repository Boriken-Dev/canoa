// Plain-data helpers: what a kind's slot answers is plain data, with an absent field left out.
//
// The types of what the host hands a provider. Guatiao's source for JavaScript (QuickJS) gives an object it owns a `destroy()` and
// answers a map or a byte string as plain data, which the Node binding's declarations do not say, so the parts a provider uses are
// written here.

/** @typedef {{running: boolean, code?: number | null, signal?: number | null}} Exit how a program stands: running, or how it ended */
/**
 * @typedef {object} Process a program a spawner started; the provider destroys it
 * @property {(bytes: Uint8Array) => void} write
 * @property {() => void} closeInput
 * @property {(dst: Uint8Array, timeoutMs: bigint) => number | bigint} read standard output: the count, 0 at the end, negative for nothing yet
 * @property {(dst: Uint8Array, timeoutMs: bigint) => number | bigint} readError standard error, the same
 * @property {(timeoutMs: bigint) => Exit} wait
 * @property {() => void} kill
 * @property {() => void} destroy
 */
/**
 * @typedef {object} Spawner the host's spawner, which the provider destroys
 * @property {(spec: object) => Process} spawn starts a program as the person
 * @property {(path: string, maxBytes: bigint) => Uint8Array<ArrayBuffer>} readFile reads a file anywhere the person's account can
 * @property {() => void} destroy
 */

/**
 * `value` as the plain data a guatiao slot answers: a field that is `undefined` or `null` is left out, lists and objects by their
 * members.
 * @param {unknown} value
 * @returns {unknown}
 */
export function dataOf(value) {
  if (Array.isArray(value)) return value.map(dataOf);
  if (value && typeof value === 'object') {
    /** @type {Record<string, unknown>} */
    const data = {};
    for (const [key, each] of Object.entries(value)) {
      if (each !== undefined && each !== null) data[key] = dataOf(each);
    }
    return data;
  }
  return value;
}
