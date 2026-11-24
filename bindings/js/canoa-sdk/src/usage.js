// The `usage_reader` kind: how an agent's own quota is read when the agent does not report it over ACP.
//
//   export class QuillUsage extends UsageReader {
//     static reader = 'quill';
//     static bucket_prefix = 'quill:';
//     open(members) { return new QuillReading(members); }
//   }
//
//   class QuillReading extends UsageReading {
//     opened(result) { /* keep what the agent said when a session opened */ }
//     read(root, tree) {
//       const processes = this.processes(root);   // the agent's process and the ones below it
//       return reading([bucket('quill:five_hour', { remaining: 750, resets_at: 1893456000 })]);
//     }
//   }
//
// The host decides when to read; the reader says what a reading is. A reading is `{ buckets, failure? }`: every bucket the agent's
// account has (each id starting with `bucket_prefix`), or a `failure` that says why there is none and whether to ask again. A
// script has no sockets or files of its own: what it needs of the machine it asks for through the `members` object `open` is handed
// (the processes that belong to the agent), and what the agent said arrives in `opened`.
import { Unsupported } from 'guatiao';
import { Provider } from 'guatiao/provide';

import * as kinds from './kinds.js';
import { dataOf } from './model.js';

/** @typedef {import('guatiao/kinds').ObjectHandle<typeof kinds.ProcessMembers>} Members the host's, which the reading destroys */

/** Whether a reading that gave nothing is worth asking for again. */
export const Retry = Object.freeze({
  /** The reason will not change by waiting. */
  NEVER: 'never',
  /** The agent shows nothing to read yet: no session has started what serves its figures. */
  NOT_YET: 'not_yet',
  /** It showed something and it did not answer in time or could not be reached. */
  NO_ANSWER: 'no_answer',
});

/**
 * One bucket of the account's quota, as the host reads it: unknown coverage, refill and status, in permille and reported, unless a
 * field says otherwise. `remaining` and `limit` are counts of `unit` (`permille`, `tokens`, `requests`, `micro_usd`); `resets_at` is
 * Unix seconds and makes the refill a fixed window. `group` names the group of allowances the bucket belongs to, `model_names` the
 * models an app shows for it, and `status` may be `disabled` for a window that does not apply now.
 * @param {string} id starts with the reader's `bucket_prefix`
 * @param {{label?: string, group?: string, models?: string[], model_names?: string[], unit?: string, used?: number, limit?: number, remaining?: number, resets_at?: number, status?: string, observed_at?: number, evidence?: string}} [fields]
 * @returns {unknown}
 */
export function bucket(id, fields = {}) {
  const { models, resets_at: resetsAt, ...rest } = fields;
  return dataOf({
    id,
    models: models ? { kind: 'only', models } : { kind: 'unknown' },
    unit: 'permille',
    refill: resetsAt === undefined ? { kind: 'unknown' } : { kind: 'fixed_window', resets_at: resetsAt },
    status: 'unknown',
    observed_at: Math.floor(Date.now() / 1000),
    source: 'reported',
    ...rest,
  });
}

/**
 * A complete reading of `buckets`.
 * @param {unknown[]} buckets
 */
export function reading(buckets) {
  return { buckets };
}

/**
 * No reading, for `why` (one line for a log, never what the agent said), to be asked again as `retry` says.
 * @param {string} why
 * @param {string} [retry] one of `Retry`
 */
export function failed(why, retry = Retry.NEVER) {
  return { buckets: [], failure: { why, retry } };
}

/**
 * Subclass it with `reader` (the id an entry's `usage` names) and `bucket_prefix`, and implement `open(members)`, which answers the
 * reading of one agent: an object that is a `UsageReading`. The timings are in milliseconds; each has the default the host's own
 * readers start from.
 */
export class UsageReader extends Provider {
  static kinds = [kinds.UsageReader];

  /** @type {string} the id an entry's `usage` names */ static reader;
  /** @type {string} what every bucket id of the reader starts with */ static bucket_prefix;
  /** @type {number} how long a session is open with no turn running before its figures are first read */ static settle_ms = 10_000;
  /** @type {number[]} how long each further try waits while the agent shows nothing to read yet */ static retries_ms = [];
  /** @type {number} how long a reading counts as current */ static cache_ms = 15_000;
  /** @type {number} how long an agent the host starts only to read may take */ static private_limit_ms = 40_000;
  /** @type {number[]} how long each wait between tries is for such an agent */ static private_retries_ms = [];
  /** @type {boolean} whether it reads the account from what the agent stores, with no process of it running */ static stored = false;

  /** The description, from the static fields. */
  info() {
    const c = /** @type {typeof UsageReader} */ (this.constructor);
    return {
      id: c.reader,
      bucket_prefix: c.bucket_prefix,
      settle_ms: c.settle_ms,
      retries_ms: c.retries_ms,
      cache_ms: c.cache_ms,
      private_limit_ms: c.private_limit_ms,
      private_retries_ms: c.private_retries_ms,
      stored: c.stored,
    };
  }

  describe() {
    return dataOf(this.info());
  }

  /**
   * A reading of one agent's figures, for the connection or entry the host opens it for. `members` is the host's, and the reading
   * destroys it (a `UsageReading` does, when the host lets it go).
   * @param {Members} _members
   * @returns {object} a `UsageReading`
   */
  open(_members) {
    throw new Unsupported('open is not implemented');
  }
}

/**
 * One agent's quota, read as often as the host asks, one call at a time. Subclass it and implement `read`; keep what `opened` is
 * told if the figures need it. The host destroys a reading when it is done with it, which lets go of `members`.
 */
export class UsageReading {
  /** @param {Members} members the host's process table for the agent */
  constructor(members) {
    this.members = members;
  }

  /**
   * The agent answered a `session/new` or `session/load` request: `result` is its `result` as JSON text.
   * @param {string} _result
   */
  opened(_result) {}

  /**
   * The processes of the agent whose process is `root`: `root` and every process below it, plus on Unix the ones in its process
   * group. An agent may sit below a launcher, so what it owns is not always a child of the process the host started.
   * @param {number} root
   * @returns {number[]}
   */
  processes(root) {
    const answer = /** @type {any} */ (this.members.members(root));
    // Under Node an answer is a Value to read and free; under Guatiao's QuickJS source it is plain data.
    if (answer && typeof answer.toJS === 'function') {
      try {
        return answer.toJS();
      } finally {
        answer[Symbol.dispose]?.();
      }
    }
    return answer;
  }

  /**
   * Reads the figures of the agent whose process is `root`. `tree` says the agent may sit below a launcher, so the whole tree is
   * looked through from the first try; otherwise look at the process itself first and widen when nothing is there.
   * @param {number} _root
   * @param {boolean} _tree
   * @returns {{buckets: unknown[], failure?: {why: string, retry: string}}} `reading(...)` or `failed(...)`
   */
  read(_root, _tree) {
    throw new Unsupported('read is not implemented');
  }

  [Symbol.dispose]() {
    /** @type {any} */ (this.members)?.destroy?.();
  }
}
