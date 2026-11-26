// The `sdk_provider` kind: a vendor's client for an account type, opened over the HTTP the host lends. `describe` answers what the
// client is and implements, with nothing run and no connection opened. `open` returns an `Sdk` over `http`, which the provider owns
// from then on and destroys.
import { Unsupported } from 'guatiao';
import { Provider } from 'guatiao/provide';

import * as kinds from './kinds.js';

/** An opened client, answered by `SdkProvider.open`. Override the operations `describe` declared; the others refuse with `Unsupported`. */
export class Sdk {
  models() {
    throw new Unsupported('this client lists no models');
  }

  /** @param {unknown} _request */
  chat(_request) {
    throw new Unsupported('this client does not chat');
  }

  /** @param {unknown} _request */
  embed(_request) {
    throw new Unsupported('this client does not embed');
  }

  /** @param {unknown} _request */
  rerank(_request) {
    throw new Unsupported('this client does not rerank');
  }
}

/** Subclass it and implement `describe` and `open`. */
export class SdkProvider extends Provider {
  static kinds = [kinds.SdkProvider];

  describe() {
    throw new Unsupported('describe is not implemented');
  }

  /**
   * @param {unknown} _request
   * @param {unknown} _http the host's `lent_http`, destroyed by the provider
   * @returns {object} an `Sdk`
   */
  open(_request, _http) {
    throw new Unsupported('open is not implemented');
  }
}
