// The `model_gateway` kind: a model that runs on this computer, served to an agent the way a cloud endpoint is.
//
//   export class LocalModels extends ModelGateway {
//     static kind = 'local-models';
//     static vendor = 'local';
//     open(_request, spawner) {
//       release(spawner);
//       return new LocalLease();
//     }
//   }
//
// Note that `canoa-host` currently loads `model_gateway` providers from bundled libraries; extensions carry the contract and
// `account_provider` loads dynamically from `canoa.cfg`.
import { Unsupported } from 'guatiao';
import { Provider } from 'guatiao/provide';

import { modelsUnknown } from './account.js';
import * as kinds from './kinds.js';
import { dataOf } from './model.js';

/** @typedef {import('./model.js').Spawner} Spawner */

/**
 * One session's access to a gateway. Subclass it and implement `address()` (`http://127.0.0.1:PORT`, no path) and `bearer()`.
 * Destroying it ends the session's access.
 */
export class GatewayLease {
  /** @returns {string} */
  address() {
    throw new Unsupported('address is not implemented');
  }

  /** @returns {string} */
  bearer() {
    throw new Unsupported('bearer is not implemented');
  }

  models() {
    return modelsUnknown('this gateway does not list models');
  }
}

/**
 * Subclass it with `kind` (the account kind it serves) and `vendor` (and optional `table` of `canoa.cfg`), and implement
 * `open(request, spawner)` answering a `GatewayLease`.
 */
export class ModelGateway extends Provider {
  static kinds = [kinds.ModelGateway];

  /** @type {string} */ static kind;
  /** @type {string} */ static vendor;
  /** @type {string | undefined} */ static table;

  /** The description, from the static fields. */
  info() {
    const c = /** @type {typeof ModelGateway} */ (this.constructor);
    return { kind: c.kind, vendor: c.vendor, table: c.table };
  }

  describe() {
    return dataOf(this.info());
  }

  /**
   * Opens a session lease on the gateway of `request.account`. `spawner` is the host's and the gateway destroys it.
   * @param {unknown} _request
   * @param {Spawner} _spawner
   * @returns {object} a `GatewayLease`
   */
  open(_request, _spawner) {
    throw new Unsupported('open is not implemented');
  }
}
