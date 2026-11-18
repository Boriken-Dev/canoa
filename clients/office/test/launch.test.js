import assert from 'node:assert/strict';
import test from 'node:test';

import { KEY_HEADER, LAUNCH_PATH, headersFor, indexKind, keyOf, offerStart, startHost, waitForHost, whatAnswers } from '../taskpane/launch.js';

const secret = 'ab12'.repeat(16);
const reply = (status, body) => ({ ok: status >= 200 && status < 300, status, json: async () => body });
const hostIndex = { peer: 'sha256:aa', entries: [{ name: 'Claude' }] };
const launcherIndex = { launcher: LAUNCH_PATH };
const quickly = { sleep: async () => {}, every: 0 };

/** A Canoa that is not running, with the launcher at its address: asking it to start makes a host answer after `after` more looks. */
function stoppedHost({ after = 2, refuse } = {}) {
  const state = { started: 0, looks: 0, launches: [], up: false };
  state.fetchImpl = async (url, init = {}) => {
    if (url === '/canoa.json') {
      state.looks += 1;
      if (state.started > 0 && state.looks > state.started + after - 1) state.up = true;
      return reply(200, state.up ? hostIndex : launcherIndex);
    }
    if (url === LAUNCH_PATH && init.method === 'POST') {
      state.launches.push(init.headers ?? {});
      if (refuse) return reply(403, { said: refuse });
      state.started = state.looks + 1;
      return reply(200, { starting: true });
    }
    throw new Error(`not a route: ${url}`);
  };
  return state;
}

test('the secret is read from the pane address only in the shape the host makes', () => {
  assert.equal(keyOf(`?office_key=${secret}`), secret);
  assert.equal(keyOf(`?x=1&office_key=${secret.toUpperCase()}`), secret, 'case is not part of it');
  for (const search of ['', '?office_key=', '?office_key=abc', `?office_key=${secret}0`, `?office_key=${'g'.repeat(64)}`, '?other=1']) {
    assert.equal(keyOf(search), undefined, search);
  }
});

test('a request carries the secret in its header when the pane has one, and no header when it has none', () => {
  assert.deepEqual(headersFor(secret), { [KEY_HEADER]: secret });
  assert.deepEqual(headersFor(undefined), {});
  assert.deepEqual(headersFor(secret, { a: 'b' }), { a: 'b', [KEY_HEADER]: secret });
});

test('what answers at the address is told apart: a host lists entries, the launcher says where to ask', () => {
  assert.equal(indexKind(hostIndex), 'host');
  assert.equal(indexKind(launcherIndex), 'launcher');
  for (const other of [null, undefined, 7, 'x', {}, { launcher: '/elsewhere' }, { entries: 'no' }]) assert.equal(indexKind(other), 'unknown', JSON.stringify(other));
});

test('nothing answering, or something that is not JSON, is unknown and not an error', async () => {
  assert.equal(await whatAnswers(async () => { throw new Error('refused'); }), 'unknown');
  assert.equal(await whatAnswers(async () => reply(404, {})), 'unknown');
  assert.equal(await whatAnswers(async () => ({ ok: true, status: 200, json: async () => { throw new Error('not json'); } })), 'unknown');
});

test('a stopped host is started from the pane\'s offer, and the pane learns when it answers', async () => {
  const host = stoppedHost();
  assert.equal(await whatAnswers(host.fetchImpl), 'launcher');
  const result = await offerStart({ key: secret, fetchImpl: host.fetchImpl, ...quickly });
  assert.deepEqual(result, { ok: true, said: 'Canoa is running' });
  assert.equal(host.launches.length, 1, 'asked once');
  assert.deepEqual(host.launches[0], { [KEY_HEADER]: secret });
  assert.equal(await whatAnswers(host.fetchImpl), 'host');
});

test('with no secret in the address no header is sent, so a host that wants one refuses', async () => {
  const host = stoppedHost({ refuse: 'this add-in\'s secret is needed' });
  const result = await offerStart({ fetchImpl: host.fetchImpl, ...quickly });
  assert.equal(result.ok, false);
  assert.match(result.said, /secret is needed/);
  assert.deepEqual(host.launches, [{}]);
  assert.equal(host.started, 0, 'nothing started');
});

test('a launcher that does not answer, or a host that never comes, ends the offer with what happened', async () => {
  const gone = await startHost({ fetchImpl: async () => { throw new Error('refused'); } });
  assert.deepEqual(gone, { started: false, said: 'the launcher did not answer' });
  const slow = stoppedHost({ after: 1000 });
  const result = await offerStart({ fetchImpl: slow.fetchImpl, tries: 3, ...quickly });
  assert.equal(result.ok, false);
  assert.match(result.said, /has not answered yet/);
  assert.equal(await waitForHost({ fetchImpl: async () => reply(200, hostIndex), ...quickly }), true);
});
