// What a telemetry exporter describes about itself and how it is told to refuse, from the SDK's classes alone: no host and no native
// library.
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { TelemetryExporter } from '../src/telemetry.js';

test('an exporter takes all three groups unless it names the ones it takes', () => {
  class Everything extends TelemetryExporter {
    static title = 'Everything';
    export(_batch) {}
  }
  assert.deepEqual(new Everything().describe(), { title: 'Everything', signals: ['spans', 'metrics', 'logs'] });

  class Lines extends TelemetryExporter {
    static title = 'A line a batch';
    static help = 'Writes one line for each batch.';
    static signals = ['logs'];
    export(_batch) {}
  }
  assert.deepEqual(new Lines().describe(), { title: 'A line a batch', help: 'Writes one line for each batch.', signals: ['logs'] });
});

test('export is the slot a subclass must write, and flush may be left alone', () => {
  class Bare extends TelemetryExporter {
    static title = 'Bare';
  }
  assert.throws(() => new Bare().export({}), /export is not implemented/);
  assert.equal(new Bare().flush(), undefined);
});
