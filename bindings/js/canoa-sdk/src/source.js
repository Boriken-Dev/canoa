// The `catalog_source` kind: where a Store type's catalog comes from, and the form the person fills in to name one.
//
//   export class Team extends CatalogSource {
//     static source = 'team';
//     static title = 'A team\'s list';
//     static types = ['skills'];
//     read(request, spawner) {
//       release(spawner);
//       return JSON.stringify({ skills: [] });
//     }
//   }
//
// The Store lists, for a type, the sources that serve it; the person picks one and fills in its `values`; canoa.cfg records the
// source's id and those values. When a catalog is refreshed the host calls `read` with the type, the label and the values as JSON
// text, and the source answers with the catalog's text in the form the type reads, or throws with the reason, which the Store shows
// beside the last good catalog it keeps.
//
// A source that runs a program or reads a file does it through the `spawner` `read` is handed: it starts a program (`spawn`) and reads
// a file anywhere the person's account can (`read_file`, by an absolute path). A script has no other way to do either, and the source
// destroys the spawner it was handed, even when it refuses (`release`).
//
// The classes that mirror what Canoa carries are `CommandCatalogSource` (a program the person named, run on each refresh, whose
// output is the catalog) and `ReadCatalogSource` (a place the Store type reads itself: a git source, an address or a file).
import { BadValue, Internal, NotFound, Unsupported } from 'guatiao';
import { Provider } from 'guatiao/provide';

import * as kinds from './kinds.js';
import { dataOf } from './model.js';

/** @typedef {import('guatiao/kinds').ObjectHandle<typeof kinds.ProcessSpawner>} Spawner the host's, which the source destroys */

/** The most bytes of a catalog a source may answer with. */
export const MAX_BYTES = 4 * 1024 * 1024;
/** How long `CommandCatalogSource` lets a program run, unless the form says otherwise, in seconds. */
export const DEFAULT_SECONDS = 60;
/** How much of what a failed program printed on its error stream is kept for the reason. */
export const TAIL_BYTES = 2 * 1024;
/** How long one read of a pipe waits before the clock is looked at again, in milliseconds. */
const POLL = 100n;

/**
 * Lets go of the host's spawner a call was handed: every call does, whether it runs a program or refuses.
 * @param {Spawner | undefined} spawner
 */
export function release(spawner) {
  if (spawner && typeof spawner.destroy === 'function') spawner.destroy();
}

/**
 * What the form's fields hold, as an object keyed by the schema's member names; a field the person left empty is absent.
 * @param {unknown} request `{ kind, label, fields }`
 * @returns {Record<string, unknown>}
 */
export function fieldsOf(request) {
  const text = request && typeof request === 'object' ? /** @type {{fields?: unknown}} */ (request).fields : undefined;
  let fields;
  try {
    fields = typeof text === 'string' ? JSON.parse(text) : undefined;
  } catch {
    fields = undefined;
  }
  if (!fields || typeof fields !== 'object' || Array.isArray(fields)) throw new BadValue('the fields are a JSON object');
  return /** @type {Record<string, unknown>} */ (fields);
}

/**
 * A catalog's bytes as text, or why they are not one.
 * @param {Uint8Array} bytes
 * @returns {string}
 */
export function textOfCatalog(bytes) {
  if (bytes.length > MAX_BYTES) {
    throw new Internal(`the catalog holds ${bytes.length} bytes, more than the ${MAX_BYTES} this host reads`);
  }
  // QuickJS's decoder replaces what is not UTF-8 instead of refusing it, so what it made is written back and compared (it drops a
  // leading byte order mark, which the comparison allows for).
  const text = new TextDecoder().decode(bytes);
  const again = new TextEncoder().encode(text);
  const mark = bytes.length >= 3 && bytes[0] === 0xef && bytes[1] === 0xbb && bytes[2] === 0xbf && again.length + 3 === bytes.length ? 3 : 0;
  if (again.length + mark !== bytes.length || again.some((byte, at) => byte !== bytes[at + mark])) {
    throw new Internal('the catalog is not text');
  }
  return text;
}

/**
 * Subclass it with `source` (the id canoa.cfg records as `provider`), `title`, `types` (the Store types it serves, none for every
 * type) and, for a source that asks for something, `values` (a guatiao schema of an object, as plain data) and `form` (a
 * guatiao-intake form document), and implement `read`.
 */
export class CatalogSource extends Provider {
  static kinds = [kinds.CatalogSource];

  /** @type {string} 1 to 32 lower-case letters, digits and `_` */ static source;

  /** The provider's id is the source's, since the host finds a source by the id it is described under. */
  static get id() {
    return this.source;
  }

  /** @type {string} */ static title;
  /** @type {string | undefined} a line saying what it reads */ static help;
  /** @type {string[]} the Store types it serves, by id; none serves every type */ static types = [];
  /** @type {object | undefined} a guatiao schema of an object: what its form asks for */ static values;
  /** @type {object | undefined} a guatiao-intake form document: how `values` is shown */ static form;

  /** The description, from the static fields. */
  info() {
    const c = /** @type {typeof CatalogSource} */ (this.constructor);
    return { id: c.source, title: c.title, help: c.help, types: c.types, schema: c.values, form: c.form };
  }

  describe() {
    return dataOf(this.info());
  }

  /**
   * The catalog's text.
   * @param {{kind: string, label: string, fields: string}} _request
   * @param {Spawner} spawner
   * @returns {string}
   */
  read(_request, spawner) {
    release(spawner);
    throw new Unsupported('read is not implemented');
  }
}

/**
 * A place the Store type reads itself: a git repository, a web address or a file, in the form the type reads. The host's own reader
 * does the reading of the first two, since it keeps what it fetched by commit and checks it; `read` answers a file on this computer.
 */
export class ReadCatalogSource extends CatalogSource {
  static source = 'read';
  static title = 'Read an address or a file';
  static help = 'A git repository, a web address or a file on this computer, in the form the type reads.';
  static values = {
    type: 'object',
    additionalProperties: false,
    required: ['source'],
    properties: {
      source: {
        type: 'string',
        title: 'Source',
        description:
          'Overlays and extensions: <repository>@<ref>#<file>. Skills: owner/repo, a git URL or a folder. MCP servers: the registry\'s address.',
      },
    },
  };

  /**
   * @param {{kind: string, label: string, fields: string}} request
   * @param {Spawner} spawner
   */
  read(request, spawner) {
    try {
      const source = fieldsOf(request).source;
      if (typeof source !== 'string' || source === '') throw new BadValue('the source is needed');
      let bytes;
      try {
        bytes = spawner.readFile(source, BigInt(MAX_BYTES) + 1n);
      } catch (error) {
        if (error instanceof NotFound) throw new Internal(`${source} cannot be read: it is not there`);
        throw new Internal(`${source} cannot be read: ${error instanceof Error ? error.message : 'refused'}`);
      }
      return textOfCatalog(bytes);
    } finally {
      release(spawner);
    }
  }
}

/**
 * Reads one stream of a program until it ends or the clock passes `deadline` (milliseconds), giving each piece to `take`. `false`
 * when the deadline came first.
 * @param {(buffer: Uint8Array, wait: bigint) => number | bigint} read
 * @param {number} deadline
 * @param {(piece: Uint8Array) => void} take
 */
function drain(read, deadline, take) {
  const buffer = new Uint8Array(16 * 1024);
  for (;;) {
    if (Date.now() > deadline) return false;
    const got = Number(read(buffer, POLL));
    if (got === 0) return true;
    if (got < 0) continue; // nothing arrived yet, or the link is down: look at the clock and ask again
    take(buffer.slice(0, got));
  }
}

/** The first `MAX_BYTES + 1` bytes of a stream, and how many there were: one byte past the most, so a catalog too long is known. */
function head() {
  /** @type {Uint8Array[]} */
  const chunks = [];
  const kept = { size: 0, total: 0 };
  return {
    take: (/** @type {Uint8Array} */ piece) => {
      kept.total += piece.length;
      // Past what is kept the program is still read, so one that prints on never waits for ever on a full pipe.
      if (kept.size <= MAX_BYTES) {
        chunks.push(piece);
        kept.size += piece.length;
      }
    },
    bytes: () => {
      const all = new Uint8Array(kept.size);
      let at = 0;
      for (const chunk of chunks) {
        all.set(chunk, at);
        at += chunk.length;
      }
      return all;
    },
    over: () => kept.total > MAX_BYTES,
  };
}

/** The last `TAIL_BYTES` bytes of a stream. */
function tail() {
  let kept = new Uint8Array(0);
  return {
    take: (/** @type {Uint8Array} */ piece) => {
      const all = new Uint8Array(kept.length + piece.length);
      all.set(kept);
      all.set(piece, kept.length);
      kept = all.length > TAIL_BYTES ? all.slice(all.length - TAIL_BYTES) : all;
    },
    text: () => new TextDecoder().decode(kept).trim(),
  };
}

/** Whether `path` names a place whatever folder the program runs in. */
function absolute(path) {
  return path.startsWith('/') || path.startsWith('\\') || /^[A-Za-z]:[\\/]/.test(path);
}

/** The seconds a program may run, from the form's field: a whole number, as a number or as text. */
function secondsOf(value) {
  const wrong = new Internal('the time it may run is a whole number of seconds');
  if (value === undefined || value === null) return DEFAULT_SECONDS;
  if (typeof value === 'number') {
    if (!Number.isInteger(value) || value < 0) throw wrong;
    return value;
  }
  if (typeof value === 'string' && /^\s*\d+\s*$/.test(value)) return Number(value.trim());
  throw wrong;
}

/**
 * A program the person named, run on each refresh, whose standard output (or a file it writes, `file`, in its folder `cwd`) is the
 * catalog. It runs as the person, with their environment, and serves every type: what it prints is read in the form the type
 * documents (an overlay or extension catalog file, a JSON list of skills, a registry's list of MCP servers).
 */
export class CommandCatalogSource extends CatalogSource {
  static source = 'command';
  static title = 'Run a program';
  static help =
    'A program and its arguments, run each time the catalog is refreshed, as you, with your environment. What it prints is the catalog, in the form the type reads: an overlay or extension catalog file, a JSON list of skills, or a registry\'s list of MCP servers.';
  static values = {
    type: 'object',
    additionalProperties: false,
    required: ['command'],
    properties: {
      command: {
        type: 'array',
        items: { type: 'string' },
        title: 'Program and arguments',
        description: 'The program first, then each argument.',
      },
      cwd: { type: 'string', title: 'Folder', description: "Where it runs; the host's own folder when empty." },
      file: {
        type: 'string',
        title: 'Output file',
        description: 'A file it writes, in its folder, to take instead of what it prints.',
      },
      timeout: {
        type: 'integer',
        minimum: 0,
        default: DEFAULT_SECONDS,
        title: 'Seconds it may run',
        description: 'How long it may run.',
      },
    },
  };
  static form = { fields: { timeout: { unit: 's' } } };

  /**
   * @param {{kind: string, label: string, fields: string}} request
   * @param {Spawner} spawner
   */
  read(request, spawner) {
    try {
      return this.#run(fieldsOf(request), spawner);
    } finally {
      release(spawner);
    }
  }

  /** The program of `fields` run through `spawner`: the catalog it printed, or why it did not produce one. */
  #run(fields, spawner) {
    const words = Array.isArray(fields.command) ? fields.command.filter((word) => typeof word === 'string') : [];
    if (words.length === 0 || words[0] === '') throw new Internal('the command is a program, then its arguments');
    const [program, ...args] = words;
    const seconds = secondsOf(fields.timeout);
    const dir = typeof fields.cwd === 'string' && fields.cwd !== '' ? fields.cwd : undefined;
    const file = typeof fields.file === 'string' && fields.file !== '' ? fields.file : undefined;

    const spec = { program, args, error_output: 'separate' };
    if (dir !== undefined) spec.cwd = dir;
    let process;
    try {
      try {
        process = spawner.spawn(spec);
      } catch (error) {
        throw new Internal(`${program} could not start: ${error instanceof Error ? error.message : 'refused'}`);
      }
      // The program has no input: it reads the end at once.
      try {
        process.closeInput();
      } catch {
        // a program that has already ended
      }
      const deadline = Date.now() + seconds * 1000;
      const printed = head();
      const said = tail();
      const ended =
        drain((buffer, wait) => process.read(buffer, wait), deadline, printed.take) &&
        drain((buffer, wait) => process.readError(buffer, wait), deadline, said.take);
      const exit = ended ? process.wait(BigInt(Math.max(1, deadline - Date.now()))) : null;
      const why = (what) => (said.text() === '' ? what : `${what}:
${said.text()}`);
      if (exit === null || exit.running) {
        process.kill();
        throw new Internal(why(`${program} was stopped after ${seconds} s`));
      }
      if (exit.code !== 0) throw new Internal(why(`${program} ended with exit code ${exit.code}`));
      let bytes = printed.bytes();
      if (file !== undefined) {
        const path = absolute(file) || dir === undefined ? file : `${dir.replace(/[\\/]+$/, '')}/${file}`;
        try {
          bytes = spawner.readFile(path, BigInt(MAX_BYTES) + 1n);
        } catch (error) {
          throw new Internal(`${path} cannot be read: ${error instanceof NotFound ? 'it is not there' : 'refused'}`);
        }
      } else if (printed.over()) {
        throw new Internal(`the catalog holds more than the ${MAX_BYTES} bytes this host reads`);
      }
      if (bytes.length === 0) throw new Internal(why(`${program} printed no catalog`));
      return textOfCatalog(bytes);
    } finally {
      process?.destroy();
    }
  }
}
