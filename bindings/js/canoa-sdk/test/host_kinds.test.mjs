// Unit tests for ConfigStore, Environment and SdkProvider: each is a provider of its kind, and a slot left in place refuses.
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { Unsupported } from 'guatiao';
import { library } from 'guatiao/provide';

import { ConfigStore, Environment, Sdk, SdkProvider } from '../src/index.js';

test('ConfigStore, Environment and SdkProvider are libraries of their kinds and refuse a slot left in place', () => {
  class Memory extends ConfigStore {
    describe() {
      return { title: 'Memory', layers: [] };
    }
  }
  class Sandbox extends Environment {
    describe() {
      return { title: 'Sandbox' };
    }
  }
  class Client extends SdkProvider {
    describe() {
      return { title: 'Client' };
    }
  }
  assert.deepEqual(library('host_kinds', '1', { providers: [Memory] }).kinds, ['config_store']);
  assert.deepEqual(library('host_kinds', '1', { providers: [Sandbox] }).kinds, ['environment']);
  assert.deepEqual(library('host_kinds', '1', { providers: [Client] }).kinds, ['sdk_provider']);

  const store = new Memory();
  assert.equal(store.describe().title, 'Memory');
  for (const call of [() => store.read(), () => store.get({}), () => store.change({}), () => store.revision()]) {
    assert.throws(call, Unsupported);
  }

  const place = new Sandbox();
  for (const call of [
    () => place.spawner(),
    () => place.readFile({}),
    () => place.writeFile({}),
    () => place.listDir({}),
    () => place.call({}),
  ]) {
    assert.throws(call, Unsupported);
  }

  assert.throws(() => new Client().open({}, {}), Unsupported);
  const sdk = new Sdk();
  for (const call of [() => sdk.models(), () => sdk.chat({}), () => sdk.embed({}), () => sdk.rerank({})]) {
    assert.throws(call, Unsupported);
  }
});
