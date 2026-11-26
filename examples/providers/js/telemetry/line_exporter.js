// Somewhere the host's telemetry can go, written in JavaScript: an exporter that appends one line of counts for each batch to a file.
//
// It takes the three groups and keeps nothing of what a batch holds but how many spans, metrics and events it had; a real exporter would
// send the batch on. The host's own `file` exporter writes the whole batch as OTLP's JSON.
import { appendFileSync } from 'node:fs';

import { s } from 'guatiao/provide';

import { TelemetryExporter } from '../../../../bindings/js/canoa-sdk/src/telemetry.js';

/** One line a batch: how many spans, metrics and events it held. */
export class LineExporter extends TelemetryExporter {
  static title = 'A line a batch';
  static help = 'Appends the counts of each batch to a file.';
  static config = s.object({ path: s.string() });

  /** @param {{path: string}} settings */
  constructor(settings) {
    super();
    this.path = settings.path;
  }

  /** @param {{spans?: unknown[], metrics?: unknown[], logs?: unknown[]}} batch */
  export(batch) {
    const counts = { spans: (batch.spans ?? []).length, metrics: (batch.metrics ?? []).length, logs: (batch.logs ?? []).length };
    appendFileSync(this.path, `${JSON.stringify(counts)}\n`);
  }
}
