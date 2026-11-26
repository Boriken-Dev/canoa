// The `content_source` and `catalog_generator` kinds: a file or a tree obtained by URI, and what turns it into a catalog.
//
//   export class Team extends ContentSource {
//     static source = 'team_files';
//     static title = "A team's files";
//     static schemes = [scheme('team', "The team's files", 'team://name/path')];
//     open(request) {
//       return openedInline(statFile(), readSomehow(request.target.uri));
//     }
//   }
//
//   export class TeamSkills extends CatalogGenerator {
//     static generator = 'team_skills';
//     static title = "A team's skills";
//     static types = ['skills'];
//     generate(request, spawner, host) {
//       try {
//         return host.openText(host.child(request.source, 'skills.json'), 1048576n);
//       } finally {
//         release(spawner);
//         release(host);
//       }
//     }
//   }
//
// A content source serves URI schemes. `describe` lists them (`scheme(name, title, grammar, { lists, resolves })`); the host calls `stat`,
// `list`, `open` and `resolve` for a URI of one of them, with a `target` (`uri`, `person`: whether the person wrote the URI, and
// `timeout_ms`). A question the scheme cannot answer is left unimplemented: the host is told `UNSUPPORTED`. An expected failure of the
// remote place (not found, denied, rate limited) is not an exception: answer it with `refused(kind, why)`. It only reads.
//
// A catalog generator turns what a source holds into a catalog in a Store type's format. `generate` is handed the request (the type, the
// label, the entry's `source`, the form's `fields` as JSON), the host's `spawner` and a `host` to read through (`stat`, `list`,
// `openText`, `openJson`, `openBytes`, `resolve`, `child`), which keeps to the entry's source and the places the generator names in
// `endpoints`. The generator releases both objects, even when it refuses.
import { Unsupported } from 'guatiao';
import { Provider } from 'guatiao/provide';

import * as kinds from './kinds.js';
import { dataOf } from './model.js';
import { release } from './source.js';

/**
 * One URI scheme a content source serves.
 * @param {string} name
 * @param {string} title
 * @param {string} grammar one line, such as `hf://owner/name[/path][?ref=REV]`
 * @param {{ lists?: boolean, resolves?: boolean, ranges?: boolean }} [can]
 */
export function scheme(name, title, grammar, can = {}) {
  return { name, title, grammar, lists: can.lists ?? false, resolves: can.resolves ?? false, ranges: can.ranges ?? false };
}

/**
 * What a source knows of a file without reading it.
 * @param {{ size?: number | bigint, etag?: string, content_type?: string, revision?: string, immutable?: boolean }} [known]
 */
export function statFile(known = {}) {
  return { kind: 'file', digests: [], immutable: false, ...known };
}

/** What a source knows of a folder. */
export function statDir() {
  return { kind: 'dir', digests: [], immutable: false };
}

/** @param {object} stat the answer of `stat` */
export function statFound(stat) {
  return { stat };
}

/**
 * The answer of `list`: entries `{ path, kind }` relative to the listed URI, and the cursor of the next page.
 * @param {{ path: string, kind: 'file' | 'dir' }[]} entries
 * @param {string} [next]
 */
export function listFound(entries, next) {
  return { listing: { entries, ...(next === undefined ? {} : { next }) } };
}

/**
 * The answer of `open`: the file's bytes, inline.
 * @param {{ size?: number | bigint }} stat
 * @param {Uint8Array | string} data
 */
export function openedInline(stat, data) {
  const bytes = typeof data === 'string' ? new TextEncoder().encode(data) : data;
  return { opened: { stat: { ...stat, size: stat.size ?? BigInt(bytes.length) }, delivery: { inline: bytes } } };
}

/**
 * The answer of `resolve`: the same place with its revision fixed.
 * @param {string} uri
 * @param {string} revision
 * @param {boolean} [immutable]
 */
export function resolved(uri, revision, immutable = false) {
  return { resolved: { uri, revision, immutable } };
}

/**
 * An expected failure of the remote place, as the answer of any question: `kind` is `not_found`, `denied`, `rate_limited`, `bad_reply`,
 * `too_large`, `unreachable`, `timeout`, `refused_place` or `changed`. `why` names the scheme and the place and holds no credential.
 * @param {string} kind
 * @param {string} why
 * @param {number} [retryAfterSeconds]
 */
export function refused(kind, why, retryAfterSeconds) {
  return { refusal: { kind, why, ...(retryAfterSeconds === undefined ? {} : { retry_after_seconds: retryAfterSeconds }) } };
}

/**
 * Subclass it with `source` (the id `[host.disabled]` names it by), `title` and `schemes`, and implement what the schemes answer:
 * `stat`, `list`, `open` and `resolve`. A slot left alone answers `UNSUPPORTED`.
 */
export class ContentSource extends Provider {
  static kinds = [kinds.ContentSource];

  /** @type {string} 1 to 32 lower-case letters, digits and `_` */ static source;

  /** The provider's id is the source's, since the host finds a provider by the id it is described under. */
  static get id() {
    return this.source;
  }

  /** @type {string} */ static title;
  /** @type {string | undefined} a line saying what it reads */ static help;
  /** @type {object[]} the schemes it serves, each from `scheme(...)` */ static schemes = [];
  /** @type {string[]} the https places its deliveries may point to */ static endpoints = [];

  /** The description, from the static fields. */
  info() {
    const c = /** @type {typeof ContentSource} */ (this.constructor);
    return { id: c.source, title: c.title, help: c.help, schemes: c.schemes, endpoints: c.endpoints };
  }

  describe() {
    return dataOf(this.info());
  }

  /** @param {unknown} _target */
  stat(_target) {
    throw new Unsupported('stat is not implemented');
  }

  /** @param {unknown} _request */
  list(_request) {
    throw new Unsupported('list is not implemented');
  }

  /** @param {unknown} _request */
  open(_request) {
    throw new Unsupported('open is not implemented');
  }

  /** @param {unknown} _target */
  resolve(_target) {
    throw new Unsupported('resolve is not implemented');
  }
}

/**
 * Subclass it with `generator` (the id canoa.cfg records), `title`, `types` (the Store types it serves, none for every type), `reads`
 * (`'required'`, `'optional'` or `'absent'`: whether it reads the entry's `source`) and, for a generator that asks for something,
 * `values` (a guatiao schema of an object) and `form`, and implement `generate`.
 */
export class CatalogGenerator extends Provider {
  static kinds = [kinds.CatalogGenerator];

  /** @type {string} */ static generator;

  static get id() {
    return this.generator;
  }

  /** @type {string} */ static title;
  /** @type {string | undefined} */ static help;
  /** @type {string[]} the Store types it serves, by id; none serves every type */ static types = [];
  /** @type {'required' | 'optional' | 'absent'} */ static reads = 'required';
  /** @type {string | undefined} what a `source` must look like, in a line */ static grammar;
  /** @type {string[]} the https places it may read besides the entry's source */ static endpoints = [];
  /** @type {string[]} the publisher key fingerprints it verifies with */ static keys = [];
  /** @type {object | undefined} a guatiao schema of an object: what its form asks for */ static values;
  /** @type {object | undefined} a guatiao-intake form document */ static form;

  /** The description, from the static fields. */
  info() {
    const c = /** @type {typeof CatalogGenerator} */ (this.constructor);
    return {
      id: c.generator,
      title: c.title,
      help: c.help,
      types: c.types,
      source: c.reads,
      grammar: c.grammar,
      endpoints: c.endpoints,
      keys: c.keys,
      schema: c.values,
      form: c.form,
    };
  }

  describe() {
    return dataOf(this.info());
  }

  /**
   * The catalog's text.
   * @param {{kind: string, label: string, source?: string, fields: string}} _request
   * @param {{ destroy(): void }} spawner
   * @param {{ destroy(): void }} host
   * @returns {string}
   */
  generate(_request, spawner, host) {
    release(spawner);
    release(host);
    throw new Unsupported('generate is not implemented');
  }
}
