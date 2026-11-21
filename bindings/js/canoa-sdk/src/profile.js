// The `profile_item` kind: one thing a profile may set, and what it does to a session.
//
//   export class Region extends EnvProfileItem {
//     static item = 'region';
//     static title = 'Region';
//     static values = { type: 'object', properties: { name: { type: 'string', title: 'Name' } }, required: ['name'] };
//     static variables = { name: ['REGION'] };
//   }
//
// A profile holds, for each item, the values a person gave the members of the item's schema (`values`). When a session starts the
// host asks each item for its effect: environment variables its programs are given, arguments, values of the session's own settings,
// the network its programs use, and notes for the report. An item changes nothing outside the effect it answers with.
//
// The classes that mirror what Canoa carries: `SettingProfileItem` (one text becomes one of the session's own settings, as the model,
// effort, mode and starting instructions items do), `EnvProfileItem` (values become environment variables, as the identity item does)
// and `NetworkProfileItem` (values become how the session's programs reach the network, as the proxy item does).
import { BadValue, Unsupported } from 'guatiao';
import { Provider } from 'guatiao/provide';

import * as kinds from './kinds.js';
import { dataOf } from './model.js';

/** The ways an effect says the session reaches the network. */
export const NETWORK_MODES = ['system', 'none', 'proxy', 'pac', 'command'];

/**
 * The values a profile holds for the item, as an object keyed by the schema's member names: what `apply` is asked with.
 * @param {unknown} request `{ harness, values }`
 * @returns {Record<string, unknown>}
 */
export function valuesOf(request) {
  const values = request && typeof request === 'object' ? /** @type {{values?: unknown}} */ (request).values : undefined;
  if (!values || typeof values !== 'object' || Array.isArray(values)) throw new BadValue('the values are a JSON object');
  return /** @type {Record<string, unknown>} */ (values);
}

/**
 * The member `name` as text, trimmed; `undefined` when it is absent, not text or empty.
 * @param {Record<string, unknown>} values
 * @param {string} name
 * @returns {string | undefined}
 */
export function textOf(values, name) {
  const value = values[name];
  if (typeof value !== 'string') return undefined;
  const text = value.trim();
  return text === '' ? undefined : text;
}

/**
 * The member `name` as a list of the texts it holds, each as it was written; empty when it is absent.
 * @param {Record<string, unknown>} values
 * @param {string} name
 * @returns {string[]}
 */
export function wordsOf(values, name) {
  const value = values[name];
  return Array.isArray(value) ? value.filter((each) => typeof each === 'string') : [];
}

/**
 * Subclass it with `item` (the id the profile keeps the values under), `title` and, for an item that takes values, `values` (a guatiao
 * schema of an object, as plain data) and `form` (a guatiao-intake form document), and implement `apply`. A member the schema marks
 * `x-sensitive` or `writeOnly` is never shown in clear. `harnesses` names the standard registry's ids the item applies to; none
 * means every harness.
 */
export class ProfileItem extends Provider {
  static kinds = [kinds.ProfileItem];

  /** @type {string} the id the profile keeps its values under: 1 to 32 lower-case letters, digits and `_` */ static item;
  /** @type {string} */ static title;
  /** @type {string | undefined} a line saying what it sets */ static help;
  /** @type {string[]} */ static harnesses = [];
  /** @type {object | undefined} a guatiao schema of an object: what the item takes */ static values;
  /** @type {object | undefined} a guatiao-intake form document: how `values` is shown */ static form;

  /** The description, from the static fields. */
  info() {
    const c = /** @type {typeof ProfileItem} */ (this.constructor);
    return { id: c.item, title: c.title, help: c.help, harnesses: c.harnesses, schema: c.values, form: c.form };
  }

  describe() {
    return dataOf(this.info());
  }

  /**
   * What the values do to a session: `{ env, args, settings, network, notes }`, each optional (see `effect`).
   * @param {{harness: string, values: Record<string, unknown>}} _request
   * @returns {object}
   */
  apply(_request) {
    throw new Unsupported('apply is not implemented');
  }
}

/**
 * An effect from its parts, with the empty ones left out.
 * @param {{env?: {name: string, value: string}[], args?: string[], settings?: {key: string, value: string}[], network?: object, notes?: string[]}} parts
 * @returns {object}
 */
export function effect({ env = [], args = [], settings = [], network, notes = [] }) {
  return /** @type {object} */ (dataOf({ env, args, settings, network, notes }));
}

/**
 * One text becomes one of the session's own settings (`model`, `effort`, `mode`, `context`, or one of `access.sandbox`,
 * `access.network`, `access.approvals`): the member `member` (default: the setting's name) of the values, when it is set.
 */
export class SettingProfileItem extends ProfileItem {
  /** @type {string} the setting's key */ static setting;
  /** @type {string | undefined} the member of the values holding it; the setting's key when absent */ static member;
  /** @type {string} what the report says of it */ static note = 'sets a value the session starts with';

  /** @param {{harness: string, values: string}} request */
  apply(request) {
    const c = /** @type {typeof SettingProfileItem} */ (this.constructor);
    const value = valuesOf(request)[c.member ?? c.setting];
    if (typeof value !== 'string') return effect({});
    return effect({ settings: [{ key: c.setting, value }], notes: [c.note] });
  }
}

/**
 * Values become environment variables. `variables` says which: each member of the values that holds text sets the variables it
 * lists (`{ name: ['GIT_AUTHOR_NAME', 'GIT_COMMITTER_NAME'] }`), and `lines`, when set, names a list member of `NAME=value` lines
 * that set whatever they name.
 */
export class EnvProfileItem extends ProfileItem {
  /** @type {Record<string, string[]>} member of the values -> the variables it sets, in order */ static variables = {};
  /** @type {string | undefined} a list member whose entries are `NAME=value` */ static lines;
  /** @type {string} what a line is called in the refusal of one that is not `NAME=value` */ static lineLabel = 'variable';
  /** @type {string} what the report says when any variable is set */ static note = 'gives its programs environment variables';

  /** @param {{harness: string, values: string}} request */
  apply(request) {
    const c = /** @type {typeof EnvProfileItem} */ (this.constructor);
    const values = valuesOf(request);
    /** @type {{name: string, value: string}[]} */
    const env = [];
    for (const [member, names] of Object.entries(c.variables)) {
      const text = textOf(values, member);
      if (text === undefined) continue;
      for (const name of names) env.push({ name, value: text });
    }
    if (c.lines !== undefined) {
      for (const line of wordsOf(values, c.lines)) {
        const at = line.indexOf('=');
        if (at < 0) throw new BadValue(`each ${c.lineLabel} is written NAME=value`);
        env.push({ name: line.slice(0, at).trim(), value: line.slice(at + 1) });
      }
    }
    return effect({ env, notes: env.length > 0 ? [c.note] : [] });
  }
}

/**
 * The entries of the list `name`, each an object with a `pattern` and an optional `identity`.
 * @param {Record<string, unknown>} values
 * @param {string} name
 */
function certificateHosts(values, name) {
  const list = values[name];
  if (list === undefined || list === null) return [];
  const wrong = () => new BadValue(`each entry of ${name} has a pattern and a certificate`);
  if (!Array.isArray(list)) throw wrong();
  return list.map((entry) => {
    if (!entry || typeof entry !== 'object' || Array.isArray(entry)) throw wrong();
    if (typeof entry.pattern !== 'string') throw wrong();
    if (entry.identity !== undefined && entry.identity !== null && typeof entry.identity !== 'string') throw wrong();
    return { pattern: entry.pattern, identity: entry.identity ?? 'system' };
  });
}

/** @type {Record<string, string>} */
const NETWORK_NOTES = {
  none: 'gives its programs no proxy',
  proxy: 'gives its programs the proxy it names and uses it for the host\'s own requests for the session',
  pac: 'gives its programs a proxy of the host\'s that chooses by the PAC script',
  command: 'runs the program it names and gives it to its programs as their proxy',
  system: 'gives its programs a proxy of the host\'s that follows the system',
};

/**
 * Values become how the session's programs reach the network. The members are the proxy item's: `mode` (one of `NETWORK_MODES`,
 * `system` when absent), `url`, `command`, `bypass`, `user`, `password_variable` and `client_certificate_hosts` (entries
 * `{ pattern, identity }`). The proxy itself is the host's: this only says which network the values ask for, which the host checks
 * and runs.
 */
export class NetworkProfileItem extends ProfileItem {
  /** @param {{harness: string, values: string}} request */
  apply(request) {
    const values = valuesOf(request);
    const mode = textOf(values, 'mode') ?? 'system';
    if (!NETWORK_MODES.includes(mode)) {
      throw new BadValue(`the network mode ${JSON.stringify(mode)} is none of ${JSON.stringify(NETWORK_MODES)}`);
    }
    const network = /** @type {{mode: string, url?: string, command: string[], bypass?: string[], intercept: {pattern: string}[]}} */ (dataOf({
      mode,
      url: textOf(values, 'url'),
      command: wordsOf(values, 'command'),
      bypass: wordsOf(values, 'bypass'),
      user: textOf(values, 'user'),
      password_variable: textOf(values, 'password_variable'),
      intercept: certificateHosts(values, 'client_certificate_hosts'),
    }));
    if ((mode === 'proxy' || mode === 'pac') && !network.url) throw new BadValue(`the network mode ${mode} needs an address`);
    if (mode === 'command' && !network.command[0]) throw new BadValue('the network mode command needs a program');
    const notes = [NETWORK_NOTES[mode]];
    const ended = network.intercept.map((/** @type {{pattern: string}} */ entry) => entry.pattern.trim()).join(', ');
    if (ended !== '') notes.push(`ends TLS for ${ended} to present your client certificate to them`);
    return effect({ network, notes });
  }
}
