// The `account_provider` kind: how a library adds an account kind, its sign-in, renewal, watch, usage and model listing.
//
//   export class DemoAccounts extends AccountProvider {
//     static title = 'Demo accounts';
//     static offeredKinds = [accountKind({ id: 'demo-key', title: 'Demo gateway', provides: ['openai-api'] })];
//   }
import { Unsupported } from 'guatiao';
import { Provider } from 'guatiao/provide';

import * as kinds from './kinds.js';
import { dataOf } from './model.js';

/** @typedef {import('./model.js').Spawner} Spawner */
/** @typedef {import('guatiao/kinds').ObjectHandle<typeof kinds.LoginPrompter>} Prompter */
/** @typedef {import('guatiao/kinds').ObjectHandle<typeof kinds.AccountKeeper>} Keeper */

/**
 * Lets go of host object handles (`spawner`, `prompter`, `keeper`) a call was handed.
 * @param {...({destroy?: () => void} | undefined)} handles
 */
export function release(...handles) {
  for (const handle of handles) {
    if (handle && typeof handle.destroy === 'function') handle.destroy();
  }
}

/**
 * JSON Schema of a secret property: `type: string`, `writeOnly`, `x-sensitive`.
 * @param {string} [title]
 * @param {string} [description]
 * @returns {{type: 'string', writeOnly: true, 'x-sensitive': true, title?: string, description?: string}}
 */
export function credentialSchema(title, description) {
  return /** @type {{type: 'string', writeOnly: true, 'x-sensitive': true, title?: string, description?: string}} */ (
    dataOf({ type: 'string', writeOnly: true, 'x-sensitive': true, title, description })
  );
}

/**
 * One kind of account a provider makes and keeps.
 * @param {{id: string, title: string, description?: string, settings?: object, endpoints?: {purpose: string, key?: string, default?: string}[], rotates?: boolean, history_portable?: boolean, state_dir?: boolean, renews?: boolean, notices?: string[], provides?: string[]}} fields
 */
export function accountKind(fields) {
  return {
    description: '',
    settings: { type: 'object' },
    endpoints: [],
    rotates: false,
    history_portable: false,
    state_dir: false,
    renews: false,
    notices: [],
    provides: [],
    ...fields,
  };
}

/**
 * One way a person signs in to make an account.
 * @param {{id: string, title: string, produces: string, description?: string, inputs?: object, steps?: string[], remote?: boolean}} fields
 */
export function loginMethod(fields) {
  return {
    description: '',
    inputs: { type: 'object' },
    steps: [],
    remote: false,
    ...fields,
  };
}

/**
 * A URL slot an account kind talks to (`purpose`: `model_api`, `auth` or `proxy`).
 * @param {{purpose: string, key?: string, default?: string}} fields
 */
export function endpointSlot(fields) {
  return dataOf(fields);
}

/**
 * The account made by `AccountProvider.login`.
 * @param {{kind: string, settings?: Record<string, unknown>, credentials?: Record<string, string>, label?: string, expires_at?: string, checked?: boolean}} fields
 */
export function loginOutcome(fields) {
  return dataOf({ settings: {}, credentials: {}, checked: false, ...fields });
}

/**
 * A known model listing.
 * @param {{id: string, name?: string, description?: string, resolved?: string, version?: string, efforts?: string[], default_effort?: string, is_default?: boolean}[]} models
 */
export function modelsListed(models) {
  return { known: true, models: models.map((model) => dataOf(model)) };
}

/**
 * A model listing that could not be answered, with the reason (`unknownKind`: `'not_installed'` or `'failed'`).
 * @param {string} reason
 * @param {'not_installed' | 'failed'} [unknownKind]
 */
export function modelsUnknown(reason, unknownKind) {
  return dataOf({ known: false, unknown_because: reason, unknown_kind: unknownKind, models: [] });
}

/**
 * An answer for an account kind whose plan usage is not offered.
 * @param {string} reason
 * @param {string} [account]
 */
export function usageUnsupported(reason, account) {
  return dataOf({
    supported: false,
    unsupported_because: reason,
    sources: [],
    account,
    revision: 0,
    observed_at: 0,
    buckets: [],
  });
}

/**
 * A renewal that produced fresh credentials.
 * @param {Record<string, string>} credentials
 * @param {string} [expiresAt]
 */
export function renewed(credentials, expiresAt) {
  return dataOf({ credentials, expires_at: expiresAt });
}

/**
 * A renewal that changed nothing, with the reason.
 * @param {string} reason
 */
export function renewUnchanged(reason) {
  return { not_renewed_because: reason };
}

/** A watch of one account's credentials from `AccountProvider.watch`. Subclass and implement `stop()`. */
export class AccountWatch {
  stop() {}
}

/**
 * Subclass it with `title`, `offeredKinds`, `logins` (or override `offer()`) and implement `login`, `usage` and `models` as the
 * kinds need. Slots handed host objects (`spawner`, `prompter`, `keeper`) own and must destroy them (`release(...)`).
 */
export class AccountProvider extends Provider {
  static kinds = [kinds.AccountProvider];

  /** @type {string} */ static title;
  /** @type {string} */ static description = '';
  /** @type {ReturnType<typeof accountKind>[]} */ static offeredKinds = [];
  /** @type {ReturnType<typeof loginMethod>[]} */ static logins = [];
  /** @type {string[]} */ static usageKinds = [];
  /** @type {boolean} */ static local = false;

  /** The provider's description, from the static fields. */
  info() {
    const c = /** @type {typeof AccountProvider} */ (this.constructor);
    return { title: c.title, description: c.description };
  }

  describe() {
    return dataOf(this.info());
  }

  /** What this provider offers, from the static fields; override to compute dynamically. */
  offer() {
    const c = /** @type {typeof AccountProvider} */ (this.constructor);
    return { kinds: c.offeredKinds, logins: c.logins, usage: c.usageKinds, local: c.local };
  }

  offers() {
    return dataOf(this.offer());
  }

  /**
   * Signs in to make an account; default destroys handles and throws `Unsupported`.
   * @param {unknown} _request
   * @param {Spawner} spawner
   * @param {Prompter} prompter
   */
  login(_request, spawner, prompter) {
    release(/** @type {any} */ (spawner), /** @type {any} */ (prompter));
    throw new Unsupported('this account provider has no login');
  }

  /**
   * Renews an account's credentials; default destroys `spawner` and throws `Unsupported`.
   * @param {unknown} _request
   * @param {Spawner} spawner
   */
  renew(_request, spawner) {
    release(spawner);
    throw new Unsupported('this account kind cannot be renewed');
  }

  /**
   * Starts watching an account; default destroys `spawner` and `keeper` and throws `Unsupported`.
   * @param {unknown} _request
   * @param {Spawner} spawner
   * @param {Keeper} keeper
   * @returns {object} an `AccountWatch`
   */
  watch(_request, spawner, keeper) {
    release(/** @type {any} */ (spawner), /** @type {any} */ (keeper));
    throw new Unsupported('this account kind has no watch');
  }

  /**
   * Reports plan usage for an account; default destroys handles and answers `usageUnsupported`.
   * @param {{account?: {id?: string}}} query
   * @param {Spawner} spawner
   * @param {Keeper} keeper
   */
  usage(query, spawner, keeper) {
    release(/** @type {any} */ (spawner), /** @type {any} */ (keeper));
    return usageUnsupported('this account provider reports no plan limits', query?.account?.id);
  }

  /**
   * Lists the models an account may use; default destroys handles and answers `modelsUnknown`.
   * @param {unknown} _query
   * @param {Spawner} spawner
   * @param {Keeper} keeper
   */
  models(_query, spawner, keeper) {
    release(/** @type {any} */ (spawner), /** @type {any} */ (keeper));
    return modelsUnknown('this account provider does not list models');
  }
}
