// The `config_store` kind: where the host's configuration lives. One provider instance is one store. Every slot is required:
// `describe` (what the store is and its layers), `read` (every layer's tables), `get` (what one path holds, merged), `change`
// (all-or-none changes in the writable layer) and `revision` (a token that differs whenever anything in the store did);
// `canoa.config_store.kinds.json` says what each answers and refuses with.
import { Unsupported } from 'guatiao';
import { Provider } from 'guatiao/provide';

import * as kinds from './kinds.js';

/** Subclass it and implement the five slots; a slot left in place refuses with `Unsupported`. */
export class ConfigStore extends Provider {
  static kinds = [kinds.ConfigStore];

  describe() {
    throw new Unsupported('describe is not implemented');
  }

  read() {
    throw new Unsupported('read is not implemented');
  }

  /** @param {unknown} _key */
  get(_key) {
    throw new Unsupported('get is not implemented');
  }

  /** @param {unknown} _changes */
  change(_changes) {
    throw new Unsupported('change is not implemented');
  }

  /** @returns {string} */
  revision() {
    throw new Unsupported('revision is not implemented');
  }
}
