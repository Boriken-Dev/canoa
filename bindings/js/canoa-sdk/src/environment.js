// The `environment` kind: a place an agent works, with its own spawner and, when it declares them, files and capabilities. One
// provider instance is one configuration of one place. `describe` and `spawner` are required; `readFile`, `writeFile`, `listDir`
// and `call` are asked only of an environment that declares the capability in `describe`, and refuse with `Unsupported` until
// overridden. Every slot may be called from several threads at once.
import { Unsupported } from 'guatiao';
import { Provider } from 'guatiao/provide';

import * as kinds from './kinds.js';

/** Subclass it, answer `describe` and hand out a `spawner`; override the file and `call` slots for what `describe` declares. */
export class Environment extends Provider {
  static kinds = [kinds.Environment];

  describe() {
    throw new Unsupported('describe is not implemented');
  }

  /** The spawner programs are started through in this environment; the host destroys it when it has no more to start. */
  spawner() {
    throw new Unsupported('spawner is not implemented');
  }

  /** @param {unknown} _request */
  readFile(_request) {
    throw new Unsupported('this environment does not read files');
  }

  /** @param {unknown} _request */
  writeFile(_request) {
    throw new Unsupported('this environment does not write files');
  }

  /** @param {unknown} _request */
  listDir(_request) {
    throw new Unsupported('this environment does not list folders');
  }

  /** @param {unknown} _request */
  call(_request) {
    throw new Unsupported('this environment declares no capability to call');
  }
}
