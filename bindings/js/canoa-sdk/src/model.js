// Plain-data helpers: what a kind's slot answers is plain data, with an absent field left out.

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
