// The Store's kinds: `store_extension`, which adds types to the Store, and `program_store`, which knows one program.
//
//   export class Notes extends StoreExtension {
//     static title = 'Notes';
//     static publisher = 'Example';
//     static types = [{ id: 'notes', title: 'Notes', stores: [] }];
//   }
//
// A type's `stores` name `program_store` providers the same library offers; the host lists and installs them as any store.
import { Unsupported } from 'guatiao';
import { Provider } from 'guatiao/provide';

import * as kinds from './kinds.js';
import { dataOf } from './model.js';

/** @typedef {import('guatiao/kinds').ObjectHandle<typeof kinds.ProcessSpawner>} Spawner the host's, which the provider destroys */
/** @typedef {import('guatiao/kinds').ObjectHandle<typeof kinds.InstallTools>} Tools */
/** @typedef {import('guatiao/kinds').ObjectHandle<typeof kinds.InstallPrompter>} Prompter */

/** Subclass it with `title`, `publisher` and `types`, or override `info()`. */
export class StoreExtension extends Provider {
  static kinds = [kinds.StoreExtension];

  /** @type {string} */ static title;
  /** @type {string} */ static publisher;
  /** @type {{id: string, title: string, stores?: string[], icon?: string}[]} */ static types = [];

  info() {
    const c = /** @type {typeof StoreExtension} */ (this.constructor);
    return { title: c.title, publisher: c.publisher, types: c.types.map((type) => ({ stores: [], ...type })) };
  }

  describe() {
    return dataOf(this.info());
  }
}

/**
 * A provider that knows one program: where its copies are, what is newest, how it installs. Override `info()` (the `StoreInfo`:
 * `program`, `endpoints`, `serves`, `summary`, `icon`, `commands`) and the four slots `program`, `latest`, `plan` and `apply`, as
 * `canoa.store.kinds.json` describes them. Every one is required and synchronous; a slot takes the host's objects (`spawner`,
 * `tools`, `prompter`) and the provider owns and destroys them. A slot left in place refuses with `Unsupported`.
 */
export class ProgramStore extends Provider {
  static kinds = [kinds.ProgramStore];

  /** @returns {object} the `StoreInfo` */
  info() {
    throw new Unsupported('a ProgramStore says its info()');
  }

  describe() {
    return dataOf(this.info());
  }

  /**
   * @param {unknown} _query
   * @param {Spawner} _spawner
   * @param {Tools} _tools
   * @returns {object} the `ProgramReport`
   */
  program(_query, _spawner, _tools) {
    throw new Unsupported('program is not implemented');
  }

  /**
   * @param {unknown} _query
   * @param {Tools} _tools
   * @returns {object} the `LatestReport`
   */
  latest(_query, _tools) {
    throw new Unsupported('latest is not implemented');
  }

  /**
   * @param {unknown} _request
   * @param {Tools} _tools
   * @returns {object} the `InstallPlan`
   */
  plan(_request, _tools) {
    throw new Unsupported('plan is not implemented');
  }

  /**
   * @param {unknown} _request
   * @param {Spawner} _spawner
   * @param {Tools} _tools
   * @param {Prompter} _prompter
   * @returns {object} the `ApplyOutcome`
   */
  apply(_request, _spawner, _tools, _prompter) {
    throw new Unsupported('apply is not implemented');
  }
}
