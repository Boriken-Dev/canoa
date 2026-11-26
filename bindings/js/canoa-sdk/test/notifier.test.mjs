// What a notifier describes about itself and how its request is read, from the SDK's classes alone: no host and no native library.
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { Notifier, delivery, noticeOf, settingsOf } from '../src/notifier.js';

test('a notifier describes itself from its static fields and answers a delivery', () => {
  class Webhook extends Notifier {
    static notifier = 'webhook';
    static title = 'Webhook';
    static events = ['finished', 'failed'];
    static values = { type: 'object', properties: { url: { type: 'string', title: 'Address' } } };
    send(request) {
      return delivery(noticeOf(request).id);
    }
  }
  assert.deepEqual(new Webhook().describe(), {
    id: 'webhook',
    title: 'Webhook',
    events: ['finished', 'failed'],
    replies: false,
    schema: { type: 'object', properties: { url: { type: 'string', title: 'Address' } } },
  });
  const request = { settings: '{"url":"http://127.0.0.1/"}', notice: { id: 'n1', event: 'finished', title: 'T', body: 'B' } };
  assert.deepEqual(settingsOf(request), { url: 'http://127.0.0.1/' });
  assert.deepEqual(new Webhook().send(request), { id: 'n1', replies: false });
  assert.equal(new Webhook().reply(request), null);
  assert.equal(new Webhook().withdraw(request), undefined);
});

test('a notifier that does not implement send refuses', () => {
  class Silent extends Notifier {
    static notifier = 'silent';
    static title = 'Silent';
  }
  assert.throws(() => new Silent().send({}), /send is not implemented/);
});
