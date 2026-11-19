// The Store's kinds: `store_extension`, which adds types to the Store, `program_store`, which knows one program, and `model_store`,
// which lists the models a type's programs run.
//
//   export class Notes extends StoreExtension {
//     static title = 'Notes';
//     static publisher = 'Example';
//     static types = [{ id: 'notes', title: 'Notes', stores: [] }];
//   }
//
// A type's `stores` name `program_store` providers the same library offers; the host lists and installs them as any store. Its `models`
// names the `model_store` provider that lists its models, which the Store shows as a page of their own.
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
  /** @type {{id: string, title: string, stores?: string[], icon?: string, models?: string}[]} */ static types = [];

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

/**
 * The models of a Store type: what the Store lists, moves and sets up for the programs of the type's `program_store`. Override `info()`
 * (the `ModelStoreInfo`: `title`, `catalog`, `publisher`, `missing`, and `settings` and `speech` when the type keeps a setting per
 * model or its service can transcribe) and the four slots `list`, `act`, `keep` and `service`, as `canoa.model_store.kinds.json`
 * describes them. Every one is required and synchronous; a slot takes the host's `spawner`, which the provider owns and destroys, and
 * runs the type's program through it. A slot left in place refuses with `Unsupported`.
 */
export class ModelStore extends Provider {
  static kinds = [kinds.ModelStore];

  /** @returns {object} the `ModelStoreInfo` */
  info() {
    throw new Unsupported('a ModelStore says its info()');
  }

  describe() {
    return dataOf(this.info());
  }

  /**
   * @param {unknown} _query
   * @param {Spawner} _spawner
   * @returns {object} the `ModelList`
   */
  list(_query, _spawner) {
    throw new Unsupported('list is not implemented');
  }

  /**
   * @param {unknown} _request
   * @param {Spawner} _spawner
   * @returns {object} the `ModelActed`
   */
  act(_request, _spawner) {
    throw new Unsupported('act is not implemented');
  }

  /**
   * @param {unknown} _request
   * @param {Spawner} _spawner
   * @returns {object} the `ModelKept`
   */
  keep(_request, _spawner) {
    throw new Unsupported('keep is not implemented');
  }

  /**
   * @param {unknown} _query
   * @param {Spawner} _spawner
   * @returns {object} the `ServiceReport`
   */
  service(_query, _spawner) {
    throw new Unsupported('service is not implemented');
  }
}
