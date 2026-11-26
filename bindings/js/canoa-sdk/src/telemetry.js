// The `telemetry_exporter` kind: somewhere the host's traces, metrics and events go, when the person turned that on.
//
//   export class LineExporter extends TelemetryExporter {
//     static title = 'A line a batch';
//     static signals = ['logs'];
//     export(batch) { /* deliver it, or throw Gone to be tried again, BadValue to have the batch dropped */ }
//   }
//
// The host decides what is recorded (a closed list the person reads and accepts) and owns batching, the queue, retries and the stop; an
// exporter only delivers the batches it is handed. A batch is plain data in the shape of OpenTelemetry's: `resource` (attributes),
// `spans`, `metrics` and `logs`, a time in Unix nanoseconds, an attribute holding one of `text`, `int`, `number` and `flag`.
//
// `export` returns when the batch is delivered. Throw `Gone` when the destination cannot be reached or refused the credentials (the host
// keeps the batch and tries again after a growing pause) and `BadValue` when another try will not help (the batch is dropped and
// counted); the message is one line with no secret and never the destination's own words. `flush` is called once when the host stops;
// leave it as it is when nothing is held.
import { Unsupported } from 'guatiao';
import { Provider } from 'guatiao/provide';

import * as kinds from './kinds.js';
import { dataOf } from './model.js';

/**
 * Subclass it with `title`, optionally `help` and `signals` (which of `'spans'`, `'metrics'` and `'logs'` it takes: all three when left
 * out) and implement `export`.
 */
export class TelemetryExporter extends Provider {
  static kinds = [kinds.TelemetryExporter];

  /** @type {string} */ static title;
  /** @type {string | undefined} a line saying where it sends what it takes */ static help;
  /** @type {('spans' | 'metrics' | 'logs')[]} */ static signals = ['spans', 'metrics', 'logs'];

  /** The description, from the static fields. */
  info() {
    const c = /** @type {typeof TelemetryExporter} */ (this.constructor);
    return { title: c.title, help: c.help, signals: [...c.signals] };
  }

  describe() {
    return dataOf(this.info());
  }

  /**
   * Delivers a batch.
   * @param {unknown} _batch
   */
  export(_batch) {
    throw new Unsupported('export is not implemented');
  }

  /** Finishes what the exporter holds, when the host stops. */
  flush() {}
}
