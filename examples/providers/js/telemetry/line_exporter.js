// Somewhere the host's telemetry can go, written in JavaScript: an exporter that keeps one line of counts for each batch.
//
// A script has no sockets or files, so this one keeps nothing of what a batch holds but how many spans, metrics and events it had,
// the last `keep` lines in memory; a real exporter would send the batch on, through a program the host starts for it or a service a
// library of the host offers. The host's own `file` exporter writes the whole batch as OTLP's JSON.
import { s } from 'guatiao/provide';

import { TelemetryExporter } from '../../../../bindings/js/canoa-sdk/src/telemetry.js';

/** How many lines are kept unless `keep` says another number. */
const KEEP = 100;

/** One line a batch: how many spans, metrics and events it held. */
export class LineExporter extends TelemetryExporter {
  static title = 'A line a batch';
  static help = 'Keeps the counts of each batch.';
  static config = s.object({ keep: s.integer({ default: KEEP }) });

  /** @type {string[]} */
  #lines = [];
  #keep;

  /** @param {{keep?: number}} settings */
  constructor(settings) {
    super();
    this.#keep = Math.max(1, settings.keep ?? KEEP);
  }

  /** The lines kept, the oldest first. */
  get lines() {
    return [...this.#lines];
  }

  /** @param {{spans?: unknown[], metrics?: unknown[], logs?: unknown[]}} batch */
  export(batch) {
    const counts = { spans: (batch.spans ?? []).length, metrics: (batch.metrics ?? []).length, logs: (batch.logs ?? []).length };
    this.#lines.push(JSON.stringify(counts));
    if (this.#lines.length > this.#keep) this.#lines.splice(0, this.#lines.length - this.#keep);
  }
}
