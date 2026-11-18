// The speech kinds: `speech_provider` (audio in, text out) and `speech_output` (text in, audio out). A provider's own settings are its
// `static config` (an `s` schema from `guatiao/provide`), with a form from `static meta = formMeta(...)` (see `forms.js`); a provider that
// starts a program or opens a connection does it through the `spawner` the host hands `open` or `say`, never by itself.
import { Unsupported } from 'guatiao';
import { Provider } from 'guatiao/provide';

import * as kinds from './kinds.js';
import { dataOf } from './model.js';

/** @typedef {import('guatiao/kinds').ObjectHandle<typeof kinds.ProcessSpawner>} Spawner the host's, which the provider destroys */

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

  info() {
    const c = /** @type {typeof SpeechProvider} */ (this.constructor);
    return {
      title: c.title,
      encodings: c.encodings,
      description: c.description,
      partials: c.partials,
      max_seconds: c.max_seconds,
      hints: c.hints,
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
   * @returns {object} the `VoiceList`
   */
  voices(_query, _spawner) {
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
