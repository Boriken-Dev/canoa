// The `notifier` kind: somewhere a notice about what happened on the host is sent.
//
//   export class Webhook extends Notifier {
//     static notifier = 'webhook';
//     static title = 'Webhook';
//     static values = { type: 'object', properties: { url: { type: 'string', title: 'Address' } } };
//     send(request) {
//       const notice = noticeOf(request);
//       post(settingsOf(request).url, notice.title, notice.body);
//       return delivery(notice.id);
//     }
//   }
//
// The host decides which events happen and which notifiers are told; a notifier delivers one notice a `send`. A notice is built by the
// host from fixed words and ids (`id`, `event`, `title`, `body`, `session`, `urgency`, `at`), never from what an agent said.
// `settings` is the notifier entry's settings, a sensitive member resolved from the host's secret. A failure is a thrown error: `Gone`
// when nothing can be told now, `BadValue` for settings that do not fit, any other for a refusal; each message is one line with no
// secret.
//
// `reply` and `withdraw` are for a notifier that takes the person's answer (`replies = true`) and need not be written otherwise.
import { Provider } from 'guatiao/provide';

import * as kinds from './kinds.js';
import { dataOf } from './model.js';

/**
 * @param {unknown} request
 * @returns {Record<string, any>}
 */
function plain(request) {
  const held = /** @type {any} */ (request);
  const body = held && typeof held.toJS === 'function' ? held.toJS() : held;
  return body && typeof body === 'object' ? body : {};
}

/**
 * The notice of a `send` request.
 * @param {unknown} request
 * @returns {Record<string, any>}
 */
export function noticeOf(request) {
  const notice = plain(request).notice;
  return notice && typeof notice === 'object' ? notice : {};
}

/**
 * The notifier entry's settings of a request (the request carries them as JSON text).
 * @param {unknown} request
 * @returns {Record<string, any>}
 */
export function settingsOf(request) {
  const text = plain(request).settings;
  const parsed = typeof text === 'string' && text.trim() ? JSON.parse(text) : {};
  return parsed && typeof parsed === 'object' ? parsed : {};
}

/**
 * What `send` answers: the notifier's own `id` for the delivered notice.
 * @param {string} id
 * @param {boolean} [replies]
 */
export function delivery(id, replies = false) {
  return { id, replies };
}

/**
 * Subclass it with `notifier` (1 to 32 lower-case letters, digits and `_`), `title`, optionally `values` (a guatiao schema of an
 * object, as plain data) and `form` (a guatiao-intake form document) and implement `send`. `events` limits the events it takes (none
 * takes any); `replies = true` says `reply` can answer.
 */
export class Notifier extends Provider {
  static kinds = [kinds.Notifier];

  /** @type {string} */ static notifier;
  /** @type {string} */ static title;
  /** @type {string | undefined} a line saying where it sends notices and what that service sees of them */ static help;
  /** @type {string[] | undefined} the event ids it takes; none takes any */ static events;
  /** @type {boolean} */ static replies = false;
  /** @type {object | undefined} a guatiao schema of an object */ static values;
  /** @type {object | undefined} a guatiao-intake form document */ static form;

  /** The description, from the static fields. */
  info() {
    const c = /** @type {typeof Notifier} */ (this.constructor);
    return {
      id: c.notifier,
      title: c.title,
      help: c.help,
      events: c.events,
      replies: c.replies,
      schema: c.values,
      form: c.form,
    };
  }

  describe() {
    return dataOf(this.info());
  }

  /**
   * Delivers `noticeOf(request)` with `settingsOf(request)`: answer `delivery(id)`.
   * @param {unknown} _request
   * @returns {unknown}
   */
  send(_request) {
    throw new Error('send is not implemented');
  }

  /**
   * The person's answer to a delivered notice: `{ action, nonce }`, or `null` for no answer within `wait_ms`.
   * @param {unknown} _request
   * @returns {unknown}
   */
  reply(_request) {
    return null;
  }

  /**
   * Takes a delivered notice away, when it was answered elsewhere.
   * @param {unknown} _request
   */
  withdraw(_request) {}
}
