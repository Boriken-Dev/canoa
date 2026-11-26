// The speech kinds: `speech_provider` (audio in, text out), `speech_output` (text in, audio out) and `account_speech` (the speech-to-text
// services the accounts of some kinds can call). A provider's own settings are its
// `static config` (an `s` schema from `guatiao/provide`), with a form from `static meta = formMeta(...)` (see `forms.js`); a provider that
// starts a program or opens a connection does it through the `spawner` the host hands `open` or `say`, never by itself.
//
// `CommandSpeech` is the provider that runs a program the person names: the speech table of canoa.cfg names it as `speech_command`.
import { BadValue, Gone, Internal, Unsupported } from 'guatiao';
import { Provider } from 'guatiao/provide';

import * as kinds from './kinds.js';
import { dataOf } from './model.js';
import { yieldCalls } from './wait.js';

/** @typedef {import('./model.js').Spawner} Spawner the host's, which the provider destroys */

/**
 * Subclass it with `services`, or override `info()`: the host offers each of the person's accounts whose kind a service names and lends
 * its key for a recording, so the host itself names no service. A service is `{id, title, standard, address, url, credential, summary,
 * location, models}`: `id` is lower-case letters and digits (an offer made of an account is `ID:ACCOUNT`), `standard` the credential
 * standard an account's kind provides for it, `address` the address that kind's model API starts from, `url` where a recording is
 * posted, `credential` the credential that holds the key, and `models` are `{id, label}` with the recommended one first.
 */
export class AccountSpeech extends Provider {
  static kinds = [kinds.AccountSpeech];

  /** @type {{id: string, title: string, standard: string, address: string, url: string, credential: string, summary?: string, location?: string, models?: {id: string, label: string}[]}[]} */
  static services = [];

  info() {
    const c = /** @type {typeof AccountSpeech} */ (this.constructor);
    return { services: c.services.map((service) => ({ summary: '', location: '', models: [], ...service })) };
  }

  describe() {
    return dataOf(this.info());
  }
}

/**
 * Subclass it with `title` and `encodings` (and the optional fields of the description) and implement `open(request, spawner)`,
 * which answers the recording's session: an object with `push(audio)`, `finish()` and `cancel()`.
 */
export class SpeechProvider extends Provider {
  static kinds = [kinds.SpeechProvider];

  /** @type {string} */ static title;
  /** @type {string[]} the audio encodings it takes: `pcm_s16le`, `opus` */ static encodings;
  /** @type {string | undefined} */ static description;
  /** @type {boolean} */ static partials = false;
  /** @type {number | undefined} */ static max_seconds;
  /** @type {string[]} the hints it uses: `language`, `vocabulary`, `profile` */ static hints = [];
  /** @type {string | undefined} the engine's release */ static version;
  /** @type {string | undefined} what the engine was built for */ static built_for;

  info() {
    const c = /** @type {typeof SpeechProvider} */ (this.constructor);
    return {
      title: c.title,
      encodings: c.encodings,
      description: c.description,
      partials: c.partials,
      max_seconds: c.max_seconds,
      hints: c.hints,
      version: c.version,
      built_for: c.built_for,
    };
  }

  describe() {
    return dataOf(this.info());
  }

  /**
   * Starts a recording; `spawner` is the host's, and the provider destroys it.
   * @param {unknown} _request the `OpenRequest`
   * @param {Spawner} _spawner
   * @returns {object} the recording's session
   */
  open(_request, _spawner) {
    throw new Unsupported('open is not implemented');
  }
}

/** A provider of text to speech: override `info()` (the `OutputInfo`), `voices` and `say`, as `canoa.speech_output.kinds.json` describes them. */
export class SpeechOutput extends Provider {
  static kinds = [kinds.SpeechOutput];

  info() {
    throw new Unsupported('a SpeechOutput says its info()');
  }

  describe() {
    return dataOf(this.info());
  }

  /**
   * @param {unknown} _query
   * @param {Spawner} _spawner
   * @param {object} _fetch the host's fetch of an `https` document, from the places `OutputInfo.endpoints` names
   * @returns {object} the `VoiceList`
   */
  voices(_query, _spawner, _fetch) {
    throw new Unsupported('voices is not implemented');
  }

  /**
   * @param {unknown} _request the `SayRequest`
   * @param {Spawner} _spawner
   * @returns {object} the utterance
   */
  say(_request, _spawner) {
    throw new Unsupported('say is not implemented');
  }
}

/** The most standard output kept, in bytes; more is dropped and the transcript's front matter says so. */
export const MAX_OUTPUT = 4 << 20;
/** The most standard error kept for a failure's message, in bytes: the last of it. */
const MAX_ERROR = 2048;
/** How long, after the program exits, its output is waited for in milliseconds: something it started may still hold the pipe. */
const OUTPUT_GRACE = 2000;
/** How much audio is written between two looks at what the program has printed, in bytes. */
const SLICE = 16 * 1024;
/** The audio encoding a command reads. */
export const PCM_S16LE = 'pcm_s16le';

/**
 * The header of a WAV stream of 16-bit PCM whose end is not known yet: both sizes say "until the end", which readers that take a
 * pipe accept.
 * @param {{rate?: number, channels?: number}} format
 * @returns {Uint8Array}
 */
export function wavHeader(format) {
  const channels = Math.max(1, Number(format.channels ?? 1));
  const rate = Number(format.rate ?? 16000);
  const block = channels * 2;
  const header = new Uint8Array(44);
  const view = new DataView(header.buffer);
  const text = (/** @type {number} */ at, /** @type {string} */ word) => {
    for (let i = 0; i < word.length; i += 1) header[at + i] = word.charCodeAt(i);
  };
  text(0, 'RIFF');
  view.setUint32(4, 0xffffffff, true);
  text(8, 'WAVEfmt ');
  view.setUint32(16, 16, true);
  view.setUint16(20, 1, true);
  view.setUint16(22, channels, true);
  view.setUint32(24, rate, true);
  view.setUint32(28, rate * block, true);
  view.setUint16(32, block, true);
  view.setUint16(34, 16, true);
  text(36, 'data');
  view.setUint32(40, 0xffffffff, true);
  return header;
}

/**
 * `arg` with each `{name}` or `{name|fallback}` of `values` replaced by its value, or by the fallback (empty without one) when the
 * value is empty. A name `values` does not hold is left as written.
 * @param {string} arg
 * @param {Record<string, string>} values
 * @returns {string}
 */
export function fill(arg, values) {
  let out = '';
  let rest = arg;
  for (;;) {
    const open = rest.indexOf('{');
    if (open < 0) return out + rest;
    out += rest.slice(0, open);
    const tail = rest.slice(open + 1);
    const close = tail.indexOf('}');
    if (close < 0) return out + rest.slice(open);
    const inside = tail.slice(0, close);
    const bar = inside.indexOf('|');
    const name = bar < 0 ? inside : inside.slice(0, bar);
    const fallback = bar < 0 ? '' : inside.slice(bar + 1);
    if (Object.hasOwn(values, name)) out += values[name] === '' ? fallback : values[name];
    else out += rest.slice(open, open + close + 2);
    rest = tail.slice(close + 1);
  }
}

/**
 * A language tag's primary subtag in lower case (`es-PR` is `es`), the form most recognizers take.
 * @param {string} tag
 * @returns {string}
 */
export function languageCode(tag) {
  return tag.split(/[-_]/)[0].toLowerCase();
}

/**
 * The hints of an `OpenRequest` a command is told by name: `language`, `vocabulary` (comma-separated), `accent` and `voice`; an absent
 * hint is left out.
 * @param {any} request
 * @returns {Record<string, string>}
 */
function namedHints(request) {
  /** @type {Record<string, string>} */
  const named = {};
  const hints = request?.hints ?? {};
  if (typeof hints.language === 'string' && hints.language !== '') named.language = hints.language;
  if (Array.isArray(hints.vocabulary) && hints.vocabulary.length > 0) named.vocabulary = hints.vocabulary.join(', ');
  const profile = hints.profile;
  if (profile && typeof profile.accent === 'string' && profile.accent !== '') named.accent = profile.accent;
  if (profile && typeof profile.voice === 'string' && profile.voice !== '') named.voice = profile.voice;
  return named;
}

/** What one program has printed, up to `MAX_OUTPUT` bytes, and whether more was dropped. */
class Capture {
  /** @type {Uint8Array[]} */ chunks = [];
  size = 0;
  dropped = false;

  /** @param {Uint8Array} piece */
  keep(piece) {
    const room = Math.max(0, MAX_OUTPUT - this.size);
    if (piece.length > room) this.dropped = true;
    if (room > 0) {
      this.chunks.push(piece.length > room ? piece.slice(0, room) : piece);
      this.size += Math.min(piece.length, room);
    }
  }

  text() {
    const all = new Uint8Array(this.size);
    let at = 0;
    for (const chunk of this.chunks) {
      all.set(chunk, at);
      at += chunk.length;
    }
    return new TextDecoder().decode(all).trim();
  }
}

/** One program, running for one recording: the session a command provider answers. */
class Recording {
  #process;
  #partials;
  #wait;
  #out = new Capture();
  /** @type {Uint8Array} */ #err = new Uint8Array(0);
  #outEnded = false;
  #errEnded = false;
  #input = true;
  #stopped = false;
  #finishing = false;
  #shown = '';
  #buffer = new Uint8Array(8192);

  /**
   * @param {import('./model.js').Process} process
   * @param {boolean} partials
   * @param {number} seconds how long it may take after the audio ends
   */
  constructor(process, partials, seconds) {
    this.#process = process;
    this.#partials = partials;
    this.#wait = seconds * 1000;
  }

  /** Whether both of the program's output streams have ended. */
  get #ended() {
    return this.#outEnded && this.#errEnded;
  }

  /**
   * Takes what the program has printed, waiting up to `wait` milliseconds for standard output when there is nothing yet: so a program
   * that prints while it reads never fills a pipe this side leaves unread.
   * @param {bigint} wait
   */
  #pump(wait) {
    for (let more = 0; !this.#outEnded && more < 64; more += 1) {
      const got = Number(this.#process.read(this.#buffer, more === 0 ? wait : 0n));
      if (got === 0) this.#outEnded = true;
      else if (got < 0) break; // nothing yet, or the link is down: look again later
      else this.#out.keep(this.#buffer.slice(0, got));
    }
    for (let more = 0; !this.#errEnded && more < 64; more += 1) {
      const got = Number(this.#process.readError(this.#buffer, 0n));
      if (got === 0) this.#errEnded = true;
      else if (got < 0) break;
      else {
        const all = new Uint8Array(this.#err.length + got);
        all.set(this.#err);
        all.set(this.#buffer.subarray(0, got), this.#err.length);
        this.#err = all.length > MAX_ERROR ? all.slice(all.length - MAX_ERROR) : all;
      }
    }
  }

  /**
   * How the program stands, waiting up to `wait` milliseconds for it to end.
   * @param {bigint} wait
   * @returns {import('./model.js').Exit}
   */
  #poll(wait) {
    return this.#process.wait(wait);
  }

  /**
   * Writes to the program's input unless it stopped taking it, which is no error here: how it ended is said at `finish`.
   * @param {Uint8Array} audio
   */
  write(audio) {
    for (let at = 0; this.#input && at < audio.length; at += SLICE) {
      this.#pump(0n);
      try {
        this.#process.write(audio.subarray(at, at + SLICE));
      } catch (error) {
        if (error instanceof Gone) this.#input = false;
        else throw new Internal(`the command's input failed: ${error instanceof Error ? error.message : 'refused'}`);
      }
    }
  }

  /** @param {Uint8Array} audio */
  push(audio) {
    if (this.#stopped) throw new Internal('the recording has ended');
    if (audio.length > 0) this.write(audio);
    this.#pump(0n);
    if (!this.#partials) return {};
    const text = this.#out.text();
    if (text === '' || text === this.#shown) return {};
    this.#shown = text;
    return { partial: text };
  }

  /** A `cancel` that came while `finish` waited ended the program and the recording. */
  #cancelled() {
    if (this.#stopped) throw new Internal('the recording was cancelled');
  }

  finish() {
    if (this.#stopped) throw new Internal('the recording has ended');
    if (this.#finishing) throw new Internal('the recording is already finishing');
    this.#finishing = true;
    try {
      this.#process.closeInput();
    } catch {
      // a program that has already ended
    }
    this.#input = false;
    const started = Date.now();
    /** @type {import('./model.js').Exit} */
    let exit;
    for (;;) {
      if (this.#ended) {
        exit = this.#poll(10n);
      } else {
        this.#pump(10n);
        exit = this.#poll(0n);
      }
      if (!exit.running) break;
      if (Date.now() - started >= this.#wait) {
        this.cancel();
        throw new Internal(`the command did not finish in ${Math.round(this.#wait / 1000)} seconds`);
      }
      // The engine runs one call at a time: let a cancel of this recording, or any other provider's call, through while it waits.
      yieldCalls();
      this.#cancelled();
    }
    const grace = Date.now() + OUTPUT_GRACE;
    while (!this.#ended && Date.now() < grace) {
      this.#pump(50n);
      yieldCalls();
      this.#cancelled();
    }
    this.#stopped = true;
    if (exit.code !== 0) {
      const how = exit.code === null || exit.code === undefined ? 'was ended by a signal' : `exited with ${exit.code}`;
      const said = new TextDecoder().decode(this.#err).trim();
      this.#process.destroy();
      throw new Internal(`the command ${how}: ${said}`);
    }
    const dropped = this.#out.dropped;
    const text = this.#out.text();
    this.#process.destroy();
    return dropped ? { text, front_matter: { output_truncated: true } } : { text };
  }

  cancel() {
    if (this.#stopped) return;
    this.#stopped = true;
    try {
      this.#process.kill();
    } catch {
      // already ended
    }
    this.#process.destroy();
  }

  [Symbol.dispose]() {
    this.cancel();
  }
}

/**
 * A program the person names: audio on its standard input (a WAV stream, or the samples alone), text on its standard output. The
 * program starts when a recording opens and ends when it finishes; its input gets the audio as it arrives, so a program that reads a
 * stream starts work at once, and its output is read as it is written, up to `MAX_OUTPUT` bytes.
 *
 * Each of `args` may name `{language}`, `{language_code}` (its primary subtag in lower case, `es` for `es-PR`), `{vocabulary}`,
 * `{accent}`, `{voice}`, `{rate}` and `{channels}`, which are replaced with what the recording opened with (an absent hint is the
 * empty text, or the word after a bar: `{language_code|auto}` is `auto` when no language was said). The hints that are present are
 * also set as the variables `CANOA_SPEECH_LANGUAGE`, `CANOA_SPEECH_VOCABULARY`, `CANOA_SPEECH_ACCENT` and `CANOA_SPEECH_VOICE`.
 */
export class CommandSpeech extends SpeechProvider {
  static id = 'speech_command';
  static displayName = 'Command';
  static title = 'Command';
  static description = 'Runs a program that reads audio and writes text';
  static encodings = [PCM_S16LE];
  static hints = ['language', 'vocabulary', 'profile'];
  static configSchema = {
    type: 'object',
    additionalProperties: false,
    required: ['program'],
    properties: {
      program: {
        type: 'string',
        title: 'Program',
        description: 'The program to run: a name found on the search path, or a path.',
        'x-section': 'command',
      },
      args: {
        type: 'array',
        items: { type: 'string' },
        default: [],
        title: 'Arguments',
        description: 'Its arguments, one per entry.',
        'x-section': 'command',
      },
      input: {
        type: 'string',
        enum: ['wav', 'pcm'],
        'x-enum-labels': {
          wav: 'A WAV stream: a header whose sizes say "until the end" and then the samples. What `ffmpeg -i -` and most command line recognizers read.',
          pcm: 'The samples alone, 16-bit little-endian, at the rate and channels the recording opened with.',
        },
        default: 'wav',
        title: 'Audio on standard input',
        description: 'How it wants the audio.',
        'x-section': 'command',
      },
      partials: {
        type: 'boolean',
        default: false,
        title: 'Show text while speaking',
        description: 'Whether each change in its output while the person speaks is shown as it appears; off, the text comes when they stop.',
        'x-section': 'command',
      },
      timeout_seconds: {
        type: 'integer',
        minimum: 0,
        maximum: 4294967295,
        default: 120,
        title: 'Time to finish',
        description: 'How long it may take after the audio ends, in seconds, before it is stopped.',
        'x-section': 'command',
        'x-advanced': true,
      },
    },
  };
  static meta = {
    description: 'Runs a program that reads audio and writes text',
    form: {
      sections: [{ id: 'command', title: 'Command' }],
      fields: { program: { placeholder: 'whisper-cli' }, timeout_seconds: { unit: 's' } },
    },
  };

  static available() {
    return null;
  }

  #program;
  #args;
  #input;
  #listening;
  #seconds;

  /** @param {{program?: string, args?: string[], input?: string, partials?: boolean, timeout_seconds?: number}} [config] */
  constructor(config) {
    super();
    this.#program = config?.program ?? '';
    this.#args = config?.args ?? [];
    this.#input = config?.input ?? 'wav';
    this.#listening = config?.partials ?? false;
    this.#seconds = config?.timeout_seconds ?? 120;
  }

  info() {
    return { ...super.info(), partials: this.#listening };
  }

  /**
   * @param {any} request the `OpenRequest`
   * @param {Spawner} spawner
   */
  open(request, spawner) {
    try {
      const format = request?.format ?? {};
      if (format.encoding !== PCM_S16LE) {
        throw new Unsupported(`the encoding ${JSON.stringify(format.encoding)} is not one this provider reads; it reads ${PCM_S16LE}`);
      }
      if (this.#program.trim() === '') throw new BadValue('the command to run is not set');
      const named = namedHints(request);
      const values = {
        language: named.language ?? '',
        language_code: languageCode(named.language ?? ''),
        vocabulary: named.vocabulary ?? '',
        accent: named.accent ?? '',
        voice: named.voice ?? '',
        rate: String(format.rate ?? 16000),
        channels: String(format.channels ?? 1),
      };
      /** @type {Record<string, string>} */
      const environment = {};
      for (const [name, text] of Object.entries(named)) environment[`CANOA_SPEECH_${name.toUpperCase()}`] = text;
      const spec = {
        program: this.#program,
        args: this.#args.map((arg) => fill(arg, values)),
        environment,
        error_output: 'separate',
      };
      let process;
      try {
        process = spawner.spawn(spec);
      } catch (error) {
        throw new Internal(`could not start ${JSON.stringify(this.#program)}: ${error instanceof Error ? error.message : 'refused'}`);
      }
      const recording = new Recording(process, this.#listening, this.#seconds);
      if (this.#input === 'wav') recording.write(wavHeader(format));
      return recording;
    } finally {
      spawner.destroy();
    }
  }
}
