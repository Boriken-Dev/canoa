// Unit tests for UsageReader/UsageReading, AccountProvider/AccountWatch, ModelGateway/GatewayLease, and write_kinds --check.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import path from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

import { Unsupported } from 'guatiao';
import { library } from 'guatiao/provide';

import {
  AccountProvider,
  AccountWatch,
  GatewayLease,
  ModelGateway,
  Retry,
  UsageReader,
  UsageReading,
  accountKind,
  bucket,
  credentialSchema,
  endpointSlot,
  failed,
  loginMethod,
  loginOutcome,
  modelsListed,
  modelsUnknown,
  reading,
  renewUnchanged,
  renewed,
  usageUnsupported,
} from '../src/index.js';

const here = path.dirname(fileURLToPath(import.meta.url));
const pkg = path.resolve(here, '..');

test('UsageReader and UsageReading describe, read process members, and build buckets/failures', () => {
  class Members {
    members(root) {
      return [root, root + 1];
    }
    destroy() {
      this.destroyed = true;
    }
  }
  class Quota extends UsageReading {
    read(root, _tree) {
      if (!this.processes(root).includes(root + 1)) return failed('missing', Retry.NEVER);
      return reading([bucket('q:five_hour', { remaining: 750, resets_at: 1893456000, observed_at: 1 })]);
    }
  }
  class QuillUsage extends UsageReader {
    static reader = 'quill';
    static bucket_prefix = 'q:';
    static settle_ms = 100;
    static retries_ms = [2000, 4000];
    open(members) {
      return new Quota(members);
    }
  }
  assert.deepEqual(new QuillUsage().describe(), {
    id: 'quill',
    bucket_prefix: 'q:',
    settle_ms: 100,
    retries_ms: [2000, 4000],
    cache_ms: 15000,
    private_limit_ms: 40000,
    private_retries_ms: [],
  });
  const members = new Members();
  const q = new QuillUsage().open(members);
  assert.deepEqual(q.read(10, false), {
    buckets: [
      {
        id: 'q:five_hour',
        models: { kind: 'unknown' },
        unit: 'permille',
        remaining: 750,
        refill: { kind: 'fixed_window', resets_at: 1893456000 },
        status: 'unknown',
        observed_at: 1,
        source: 'reported',
      },
    ],
  });
  q[Symbol.dispose]();
  assert.equal(members.destroyed, true);
  assert.deepEqual(failed('no port', Retry.NOT_YET), { buckets: [], failure: { why: 'no port', retry: 'not_yet' } });
  assert.deepEqual(library('usage_test', '1', { providers: [QuillUsage] }).kinds, ['usage_reader']);
});

test('AccountProvider, ModelGateway, and helpers build valid answers and destroy handles', () => {
  const destroyed = [];
  const handle = (tag) => ({
    destroy() {
      destroyed.push(tag);
    },
  });

  class DemoAccounts extends AccountProvider {
    static title = 'Demo accounts';
    static description = 'An account kind.';
    static offeredKinds = [
      accountKind({
        id: 'demo-key',
        title: 'Demo key',
        settings: {
          type: 'object',
          properties: { api_key: credentialSchema('Key') },
          required: ['api_key'],
        },
        endpoints: [endpointSlot({ purpose: 'model_api', key: 'base_url' })],
        provides: ['openai-api'],
      }),
    ];
    static logins = [loginMethod({ id: 'demo-login', title: 'Key', produces: 'demo-key', steps: ['ask'] })];
    provides() {
      return ['openai-api'];
    }
  }

  const accounts = new DemoAccounts();
  assert.deepEqual(accounts.describe(), { title: 'Demo accounts', description: 'An account kind.' });
  const offered = accounts.offers();
  assert.equal(offered.kinds[0].id, 'demo-key');
  assert.equal(offered.logins[0].produces, 'demo-key');
  assert.deepEqual(accounts.usage({ account: { id: 'acc-1' } }, handle('s1'), handle('k1')), usageUnsupported('this account provider reports no plan limits', 'acc-1'));
  assert.deepEqual(accounts.models({}, handle('s2'), handle('k2')), modelsUnknown('this account provider does not list models'));
  assert.throws(() => accounts.login({}, handle('s3'), handle('p3')), Unsupported);
  assert.throws(() => accounts.renew({}, handle('s4')), Unsupported);
  assert.throws(() => accounts.watch({}, handle('s5'), handle('k5')), Unsupported);
  assert.deepEqual(destroyed, ['s1', 'k1', 's2', 'k2', 's3', 'p3', 's4', 's5', 'k5']);
  assert.deepEqual(loginOutcome({ kind: 'demo-key', credentials: { api_key: 'k' } }), {
    kind: 'demo-key',
    settings: {},
    credentials: { api_key: 'k' },
    checked: false,
  });
  assert.deepEqual(modelsListed([{ id: 'm1' }]), { known: true, models: [{ id: 'm1' }] });
  assert.deepEqual(renewed({ api_key: 'k2' }), { credentials: { api_key: 'k2' } });
  assert.deepEqual(renewUnchanged('fresh'), { not_renewed_because: 'fresh' });
  new AccountWatch().stop();

  class LocalLease extends GatewayLease {
    address() {
      return 'http://127.0.0.1:4100';
    }
    bearer() {
      return 'session-bearer';
    }
  }
  class LocalGateway extends ModelGateway {
    static kind = 'demo-key';
    static vendor = 'demo';
    static table = 'demo';
    open(request, spawner) {
      spawner.destroy();
      assert.equal(request.account, 'acc-1');
      return new LocalLease();
    }
  }
  const gw = new LocalGateway();
  assert.deepEqual(gw.describe(), { kind: 'demo-key', vendor: 'demo', table: 'demo' });
  const lease = gw.open({ account: 'acc-1' }, handle('gw-s'));
  assert.equal(lease.address(), 'http://127.0.0.1:4100');
  assert.equal(lease.bearer(), 'session-bearer');
  assert.equal(lease.models().known, false);
  assert.equal(destroyed.at(-1), 'gw-s');
  assert.deepEqual(library('account_gw_test', '1', { providers: [DemoAccounts, LocalGateway] }).kinds, [
    'account_provider',
    'model_gateway',
  ]);
});

test('generated kinds match crates/canoa/include', { skip: !process.env.GUATIAO_NODE }, () => {
  const result = spawnSync(process.execPath, [path.join(pkg, 'tools/write_kinds.mjs'), '--check'], {
    cwd: pkg,
    encoding: 'utf8',
  });
  assert.equal(result.status, 0, result.stderr);
});
