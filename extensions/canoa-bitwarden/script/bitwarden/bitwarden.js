// A `secret_provider` for Bitwarden, written in JavaScript: it reads a secret by name through Bitwarden's own `bw` command.
//
// The module is the library: its id and version are its package's, its provider the class it exports. It needs no build and is
// shared between hosts as the files it is. QuickJS gives a library no processes, so every program it runs starts through the
// `spawner` each call is handed: the host's, which the provider destroys when the call ends.
//
// How it talks to the vault. `bw get password NAME` (or `username`, `notes`, `totp`, `uri`: the setting `part`) answers the
// named item's part, and `bw list items` its names. The vault is opened with a session key the person made with
// `bw unlock --raw`; the host gives it as the setting `session`, taken from another secret, and this module gives it to `bw` in
// the environment (`BW_SESSION`), never on a command line, where any program on the machine could read it. Nothing this module
// returns, throws or prints holds a value: a failure says what `bw` could not do and never what it printed.
//
// A vault that is locked, signed out or given a wrong key answers `GONE`, so the host asks the person instead. An item that is not
// there is no value, not an error. Bitwarden items have no end of their own, so no answer carries one.

import { Gone, Internal, NotFound, Unsupported } from 'guatiao';

import { SecretProvider, nameOf, release } from './sdk/secret.js';

/** The most of `bw`'s output read: a listing of a large vault with every item's fields is the biggest. */
const MAX_OUTPUT = 64 * 1024 * 1024;
/** What `bw` says on standard error when the vault cannot be opened, in the words it uses. */
const LOCKED = ['vault is locked', 'not logged in', 'invalid master password', 'session key is invalid'];
const MISSING = ['not found'];
const PARTS = ['password', 'username', 'notes', 'totp', 'uri'];
/** How long one read of a pipe waits before the clock is looked at again, in milliseconds. */
const POLL = 100n;

/** `bw` said the item is not there: no value, and no failure. */
class Missing extends Error {}

/** All of what `read` gives, until it ends or `limit` bytes or `deadline` pass. */
function drain(read, limit, deadline) {
  const chunks = [];
  let total = 0;
  const buffer = new Uint8Array(16 * 1024);
  for (;;) {
    if (Date.now() > deadline) return null;
    const got = Number(read(buffer, POLL));
    if (got === 0) break;
    if (got < 0) continue; // nothing arrived yet, or the link is down: look at the clock and ask again
    total += got;
    if (total > limit) throw new Internal("Bitwarden's command answered more than a vault holds");
    chunks.push(buffer.slice(0, got));
  }
  const all = new Uint8Array(total);
  let at = 0;
  for (const chunk of chunks) {
    all.set(chunk, at);
    at += chunk.length;
  }
  return all;
}

export class Bitwarden extends SecretProvider {
  static id = 'bitwarden';
  static displayName = 'Bitwarden';
  static title = 'Bitwarden';
  static description = 'Your Bitwarden vault, read through its `bw` command with a session key you give once.';
  static can_list = true;
  static configSchema = {
    type: 'object',
    additionalProperties: false,
    properties: {
      command: {
        type: 'string',
        default: 'bw',
        title: 'Command',
        description: "Bitwarden's command line: a name found on the path, or the file itself.",
        'x-section': 'vault',
      },
      session: {
        type: 'string',
        writeOnly: true,
        'x-sensitive': true,
        'x-from-secret': true,
        title: 'Session key',
        description: 'What `bw unlock --raw` prints. Give it here once, or take it from another provider, such as the form.',
        'x-section': 'vault',
      },
      part: {
        type: 'string',
        enum: PARTS,
        default: 'password',
        title: 'Part of an item',
        description: 'The part of an item that is the secret.',
        'x-section': 'vault',
      },
      arguments: {
        type: 'array',
        items: { type: 'string' },
        default: [],
        title: 'Arguments',
        description: 'What goes before `get`, for a program that runs `bw` rather than being it.',
        'x-advanced': true,
        'x-section': 'vault',
      },
      timeout_seconds: {
        type: 'integer',
        minimum: 1,
        maximum: 600,
        default: 30,
        title: 'Time to answer',
        description: 'How long one call to `bw` may take, in seconds.',
        'x-advanced': true,
        'x-section': 'vault',
      },
    },
  };
  static meta = {
    form: {
      sections: [{ id: 'vault', title: 'Bitwarden', description: 'Read through `bw`, which has to be signed in (`bw login`).' }],
      fields: {
        command: { placeholder: 'bw' },
        session: { placeholder: 'The session key `bw unlock --raw` printed' },
        timeout_seconds: { unit: 'seconds' },
      },
    },
  };

  static available() {
    return null;
  }

  #command;
  #arguments;
  #part;
  #session;
  #timeout;

  constructor(config) {
    super();
    this.#command = config?.command ?? 'bw';
    this.#arguments = config?.arguments ?? [];
    this.#part = config?.part ?? 'password';
    this.#session = config?.session ?? null;
    this.#timeout = config?.timeout_seconds ?? 30;
  }

  /** `bw` with `words`: what it printed, or why it could not. Every program starts through `spawner`, which is let go after. */
  #run(spawner, words) {
    const environment = { BW_NOINTERACTION: 'true' };
    if (this.#session) environment.BW_SESSION = this.#session;
    const spec = {
      program: this.#command,
      args: [...this.#arguments, ...words, '--nointeraction'],
      environment,
      remove_environment: this.#session ? [] : ['BW_SESSION'],
      error_output: 'separate',
    };
    let process;
    try {
      try {
        process = spawner.spawn(spec);
      } catch (error) {
        if (error instanceof NotFound) throw new Gone(`${this.#command} is not installed: Bitwarden's command line is needed`);
        throw new Gone("Bitwarden's command could not be started");
      }
      const deadline = Date.now() + this.#timeout * 1000;
      const out = drain((buffer, wait) => process.read(buffer, wait), MAX_OUTPUT, deadline);
      const err = out === null ? null : drain((buffer, wait) => process.readError(buffer, wait), MAX_OUTPUT, deadline);
      if (out === null || err === null) {
        process.kill();
        throw new Gone("Bitwarden's command did not answer in time");
      }
      const exit = process.wait(BigInt(Math.max(1, deadline - Date.now())));
      if (exit.running) {
        process.kill();
        throw new Gone("Bitwarden's command did not answer in time");
      }
      if (exit.code !== 0) {
        const said = new TextDecoder().decode(err).toLowerCase();
        if (MISSING.some((words_) => said.includes(words_))) throw new Missing();
        if (LOCKED.some((words_) => said.includes(words_))) {
          throw new Gone('the vault is locked or the session key is not good: unlock it and give the key again');
        }
        throw new Gone("Bitwarden's command could not read the vault");
      }
      return new TextDecoder().decode(out);
    } finally {
      process?.destroy();
      spawner.destroy();
    }
  }

  names(spawner) {
    let items;
    try {
      items = JSON.parse(this.#run(spawner, ['list', 'items']));
    } catch (error) {
      if (error instanceof Missing) return { names: [] };
      if (error instanceof SyntaxError) throw new Internal("Bitwarden's listing is not what `bw list items` prints");
      throw error;
    }
    if (!Array.isArray(items)) throw new Internal("Bitwarden's listing is not what `bw list items` prints");
    const names = [
      ...new Set(
        items.filter((item) => item && typeof item === 'object' && typeof item.name === 'string' && item.name).map((item) => item.name),
      ),
    ].sort();
    return { names };
  }

  get(request, spawner) {
    let name;
    try {
      name = nameOf(request);
    } catch (error) {
      release(spawner);
      throw error;
    }
    let text;
    try {
      text = this.#run(spawner, ['get', this.#part, name]);
    } catch (error) {
      if (error instanceof Missing) return {};
      throw error;
    }
    // `bw` may end its answer with one line break of its own; a value's own are inside it.
    const value = text.endsWith('\r\n') ? text.slice(0, -2) : text.endsWith('\n') ? text.slice(0, -1) : text;
    return value ? { value } : {};
  }

  put(_request, spawner) {
    release(spawner);
    throw new Unsupported('Bitwarden keeps its own items: add this one in Bitwarden');
  }

  forget(_request, spawner) {
    release(spawner);
    throw new Unsupported('Bitwarden keeps its own items: remove this one in Bitwarden');
  }
}
