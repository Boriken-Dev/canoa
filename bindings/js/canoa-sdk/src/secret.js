// The `secret_provider` kind: somewhere a secret comes from and, for some, goes to.
//
//   export class Vault extends SecretProvider {
//     static title = 'Vault';
//     static can_list = true;
//     get(request, spawner) {
//       spawner.destroy();
//       return nameOf(request) === 'demo' ? { value: 'hunter2' } : {};
//     }
//   }
//
// A value is text and is never in a description, a name or an error. `get` answers `{}` for a name it holds nothing under, and
// `{ value, expires_in }` (seconds) for one that ends by itself. A provider that cannot be reached or has not been unlocked throws
// `Gone` with a reason that holds no value, and the host asks the person for the secret instead. One that is asked for what it cannot
// do throws `Unsupported`, which is what `put`, `forget` and `names` do until a class says `can_store` or `can_list`.
//
// Every call that may run a program is handed the host's `spawner`; a script has no other way to start one. The provider owns the
// handle and destroys it, even when it refuses (`release`).
//
// The classes that mirror what Canoa carries are `FormSecretProvider` (the person's form, which holds nothing: the host asks in the
// app) and `SystemStoreSecretProvider` (a provider over a store of keys and texts you supply, with the index of names such a store
// needs). The operating system's own store is the host's `system` provider: an entry of `[secrets.providers]` names it with
// `provider = "system"`.
import { BadValue, Internal, Unsupported } from 'guatiao';
import { Provider } from 'guatiao/provide';

import * as kinds from './kinds.js';
import { dataOf } from './model.js';

/** @typedef {import('./model.js').Spawner} Spawner the host's, which the provider destroys */

/**
 * Lets go of the host's spawner a call was handed: every call does, whether it runs a program or refuses.
 * @param {Spawner | undefined} spawner
 */
export function release(spawner) {
  if (spawner && typeof spawner.destroy === 'function') spawner.destroy();
}

/** @param {unknown} name */
function checked(name) {
  if (typeof name !== 'string' || name.length === 0) throw new BadValue("a secret's name is text that is not empty");
  // eslint-disable-next-line no-control-regex
  if (/[\u0000-\u001f\u007f]/.test(name)) throw new BadValue("a secret's name has a control character");
  return name;
}

/**
 * The name a `get` or `forget` request asks for, checked as the kind says a name is: text that is not empty, with no
 * control character. A request that holds anything else is refused, so a field a newer host adds is never dropped silently.
 * @param {unknown} request
 * @returns {string}
 */
export function nameOf(request) {
  const keys = request && typeof request === 'object' ? Object.keys(request) : [];
  if (keys.length !== 1 || keys[0] !== 'name') throw new BadValue('a request names one secret and nothing else');
  return checked(/** @type {{name: unknown}} */ (request).name);
}

/**
 * The name and the value a `put` request carries, checked the same way; the value is text.
 * @param {unknown} request
 * @returns {{name: string, value: string}}
 */
export function keepingOf(request) {
  const keys = request && typeof request === 'object' ? Object.keys(request).sort() : [];
  if (keys.length !== 2 || keys[0] !== 'name' || keys[1] !== 'value') {
    throw new BadValue('a request keeps one value under one name and nothing else');
  }
  const { name, value } = /** @type {{name: unknown, value: unknown}} */ (request);
  if (typeof value !== 'string') throw new BadValue('a secret is text');
  return { name: checked(name), value };
}

/**
 * Subclass it with `title` (and the optional fields of the description) and implement `get`; add `names`, `put` and `forget` for a
 * provider that lists or keeps (and set `can_list` and `can_store`). A provider's settings are its `static config` (an `s` schema from
 * `guatiao/provide`) with a form from `static meta = formMeta(...)`; a setting that is another secret (the key that unlocks a vault)
 * is marked `'x-from-secret': true` in its schema and the host fills it, never the form.
 */
export class SecretProvider extends Provider {
  static kinds = [kinds.SecretProvider];

  /** @type {string} */ static title;
  /** @type {string | undefined} */ static description;
  /** @type {boolean} it keeps what it is given */ static can_store = false;
  /** @type {boolean} it lists its names */ static can_list = false;
  /** @type {boolean} answering needs someone at the screen: a form to fill, a vault to unlock */ static needs_person = false;

  /** The description, from the static fields. */
  info() {
    const c = /** @type {typeof SecretProvider} */ (this.constructor);
    return {
      title: c.title,
      description: c.description,
      can_store: c.can_store,
      can_list: c.can_list,
      needs_person: c.needs_person,
    };
  }

  describe() {
    return dataOf(this.info());
  }

  /**
   * The names it holds and nothing else: `{ names: [...] }`, every one it holds.
   * @param {Spawner} spawner
   * @returns {{names: string[]}}
   */
  names(spawner) {
    release(spawner);
    throw new Unsupported('this provider cannot list what it holds');
  }

  /**
   * What it holds under the request's name: `{}` for nothing, else `{ value, expires_in? }`.
   * @param {unknown} _request `{ name }`
   * @param {Spawner} spawner
   * @returns {{value?: string, expires_in?: number}}
   */
  get(_request, spawner) {
    release(spawner);
    throw new Unsupported('get is not implemented');
  }

  /**
   * Keeps a value, replacing one of that name.
   * @param {unknown} _request `{ name, value }`
   * @param {Spawner} spawner
   */
  put(_request, spawner) {
    release(spawner);
    throw new Unsupported('this provider keeps nothing');
  }

  /**
   * Forgets what it keeps under the name; not having it is no error.
   * @param {unknown} _request `{ name }`
   * @param {Spawner} spawner
   */
  forget(_request, spawner) {
    release(spawner);
    throw new Unsupported('this provider keeps nothing');
  }
}

/**
 * The person's form: it holds nothing, so a secret it is to answer is typed in the app, which the host asks itself. It is what a host
 * with no provider for a name falls back on; subclass it to change the title or the words.
 */
export class FormSecretProvider extends SecretProvider {
  static title = 'Form';
  static description =
    'Asks you in the app, and keeps the value as long as you choose: once, for the session, until Canoa closes or for a time.';
  static needs_person = true;

  /** @param {unknown} request @param {Spawner} spawner */
  get(request, spawner) {
    release(spawner);
    nameOf(request);
    return {};
  }

  /** @param {Spawner} spawner @returns {{names: string[]}} */
  names(spawner) {
    release(spawner);
    throw new Unsupported('a form holds nothing to list');
  }

  /** @param {unknown} _request @param {Spawner} spawner */
  put(_request, spawner) {
    release(spawner);
    throw new Unsupported('a form keeps nothing: the host keeps it for as long as you chose');
  }

  /** @param {unknown} _request @param {Spawner} spawner */
  forget(_request, spawner) {
    release(spawner);
    throw new Unsupported('a form keeps nothing');
  }
}

/**
 * A provider over a store of keys and texts that cannot be listed by what it holds (a credential store): it keeps each secret under
 * `s:` and its name and an index of the names under `i:names`, as Canoa's system provider does, so `names` can answer.
 *
 * Implement the three calls of the store: `read(key)` (the text, or `null` when there is none), `write(key, text)` and `remove(key)`
 * (not having it is no error). A store that cannot be reached throws `Gone` with a reason that holds no value.
 */
export class SystemStoreSecretProvider extends SecretProvider {
  static title = 'Store';
  static can_store = true;
  static can_list = true;
  static INDEX = 'i:names';

  /** @param {string} _key @returns {string | null} */
  read(_key) {
    throw new Unsupported('read is not implemented');
  }

  /** @param {string} _key @param {string} _text */
  write(_key, _text) {
    throw new Unsupported('write is not implemented');
  }

  /** @param {string} _key */
  remove(_key) {
    throw new Unsupported('remove is not implemented');
  }

  /** @returns {string[]} */
  #held() {
    const text = this.read(SystemStoreSecretProvider.INDEX);
    if (text === null || text === undefined) return [];
    let names;
    try {
      names = JSON.parse(text);
    } catch {
      throw new Internal("the store's list of names is damaged");
    }
    if (!Array.isArray(names) || !names.every((each) => typeof each === 'string')) {
      throw new Internal("the store's list of names is damaged");
    }
    return names;
  }

  /** @param {string[]} names */
  #index(names) {
    this.write(SystemStoreSecretProvider.INDEX, JSON.stringify([...new Set(names)].sort()));
  }

  /** @param {Spawner} spawner */
  names(spawner) {
    release(spawner);
    return { names: [...this.#held()].sort() };
  }

  /** @param {unknown} request @param {Spawner} spawner */
  get(request, spawner) {
    release(spawner);
    const text = this.read(`s:${nameOf(request)}`);
    return text === null || text === undefined ? {} : { value: text };
  }

  /** @param {unknown} request @param {Spawner} spawner */
  put(request, spawner) {
    release(spawner);
    const { name, value } = keepingOf(request);
    const names = this.#held();
    // The value goes first: an index that names a secret the store lacks would be worse than the reverse.
    this.write(`s:${name}`, value);
    if (!names.includes(name)) this.#index([...names, name]);
  }

  /** @param {unknown} request @param {Spawner} spawner */
  forget(request, spawner) {
    release(spawner);
    const name = nameOf(request);
    const names = this.#held();
    this.remove(`s:${name}`);
    if (names.includes(name)) this.#index(names.filter((each) => each !== name));
  }
}
