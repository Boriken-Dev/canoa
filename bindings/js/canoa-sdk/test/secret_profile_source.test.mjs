// Unit tests for SecretProvider subclasses, ProfileItem subclasses, CatalogSource subclasses, and SettingsSection.
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { BadValue, Unsupported } from 'guatiao';
import { library } from 'guatiao/provide';

import {
  CatalogSource,
  CommandCatalogSource,
  EnvProfileItem,
  FormSecretProvider,
  NETWORK_MODES,
  NetworkProfileItem,
  ProfileItem,
  ReadCatalogSource,
  SecretProvider,
  SettingProfileItem,
  SettingsSection,
  SystemStoreSecretProvider,
  effect,
  fieldsOf,
  keepingOf,
  nameOf,
  release,
  textOf,
  valuesOf,
  wordsOf,
} from '../src/index.js';
import { textOfCatalog } from '../src/source.js';

function spawner(log) {
  return {
    destroy() {
      log.push('destroyed');
    },
  };
}

test('SecretProvider, FormSecretProvider, and SystemStoreSecretProvider validate requests and manage index', () => {
  assert.equal(nameOf({ name: 'token' }), 'token');
  assert.throws(() => nameOf({}), BadValue);
  assert.throws(() => nameOf({ name: 'a\nb' }), BadValue);
  assert.throws(() => nameOf({ name: 'a', extra: 1 }), BadValue);
  assert.deepEqual(keepingOf({ name: 'token', value: 'hunter2' }), { name: 'token', value: 'hunter2' });
  assert.throws(() => keepingOf({ name: 'token', value: 42 }), BadValue);

  const gone = [];
  const form = new FormSecretProvider();
  assert.deepEqual(form.describe(), {
    title: 'Form',
    description: FormSecretProvider.description,
    can_store: false,
    can_list: false,
    needs_person: true,
  });
  assert.deepEqual(form.get({ name: 'demo' }, spawner(gone)), {});
  assert.throws(() => form.names(spawner(gone)), Unsupported);
  assert.throws(() => form.put({ name: 'demo', value: 'v' }, spawner(gone)), Unsupported);
  assert.throws(() => form.forget({ name: 'demo' }, spawner(gone)), Unsupported);

  const mem = new Map();
  class MemStore extends SystemStoreSecretProvider {
    read(key) {
      return mem.get(key) ?? null;
    }
    write(key, text) {
      mem.set(key, text);
    }
    remove(key) {
      mem.delete(key);
    }
  }
  const store = new MemStore();
  assert.deepEqual(store.names(spawner(gone)), { names: [] });
  store.put({ name: 'beta', value: '2' }, spawner(gone));
  store.put({ name: 'alpha', value: '1' }, spawner(gone));
  assert.deepEqual(store.names(spawner(gone)), { names: ['alpha', 'beta'] });
  assert.deepEqual(store.get({ name: 'alpha' }, spawner(gone)), { value: '1' });
  store.forget({ name: 'alpha' }, spawner(gone));
  assert.deepEqual(store.names(spawner(gone)), { names: ['beta'] });
  assert.deepEqual(store.get({ name: 'alpha' }, spawner(gone)), {});
  release(spawner(gone));
  assert.equal(gone.length, 13);

  class MinimalSecret extends SecretProvider {
    static title = 'Minimal';
    get(_request, spawnerArg) {
      release(spawnerArg);
      return {};
    }
  }
  assert.deepEqual(library('secret_test', '1', { providers: [MinimalSecret] }).kinds, ['secret_provider']);
});

test('ProfileItem subclasses build settings, environment variables, and network effects', () => {
  assert.deepEqual(NETWORK_MODES, ['system', 'none', 'proxy', 'pac', 'command']);
  assert.throws(() => valuesOf({ values: [] }), BadValue);
  assert.equal(textOf({ a: '  hi  ', b: '   ' }, 'a'), 'hi');
  assert.equal(textOf({ b: '   ' }, 'b'), undefined);
  assert.deepEqual(wordsOf({ list: ['a', 1, 'b'] }, 'list'), ['a', 'b']);

  class ModelItem extends SettingProfileItem {
    static item = 'model';
    static title = 'Model';
    static setting = 'model';
    static note = 'sets model';
  }
  assert.deepEqual(new ModelItem().apply({ harness: 'codex', values: { model: 'gpt-5' } }), {
    env: [],
    args: [],
    settings: [{ key: 'model', value: 'gpt-5' }],
    notes: ['sets model'],
  });
  assert.deepEqual(new ModelItem().apply({ harness: 'codex', values: {} }), {
    env: [],
    args: [],
    settings: [],
    notes: [],
  });

  class IdentityItem extends EnvProfileItem {
    static item = 'identity';
    static title = 'Identity';
    static variables = { name: ['GIT_AUTHOR_NAME', 'GIT_COMMITTER_NAME'] };
    static lines = 'extra';
  }
  assert.deepEqual(
    new IdentityItem().apply({ harness: 'claude', values: { name: 'Ada', extra: ['FOO=bar=baz'] } }),
    {
      env: [
        { name: 'GIT_AUTHOR_NAME', value: 'Ada' },
        { name: 'GIT_COMMITTER_NAME', value: 'Ada' },
        { name: 'FOO', value: 'bar=baz' },
      ],
      args: [],
      settings: [],
      notes: ['gives its programs environment variables'],
    },
  );
  assert.throws(
    () => new IdentityItem().apply({ harness: 'claude', values: { extra: ['MALFORMED'] } }),
    BadValue,
  );

  class ProxyItem extends NetworkProfileItem {
    static item = 'proxy';
    static title = 'Proxy';
  }
  assert.deepEqual(
    new ProxyItem().apply({
      harness: 'claude',
      values: { mode: 'proxy', url: 'http://127.0.0.1:8888', client_certificate_hosts: [{ pattern: '*.corp' }] },
    }),
    {
      env: [],
      args: [],
      settings: [],
      network: {
        mode: 'proxy',
        url: 'http://127.0.0.1:8888',
        command: [],
        bypass: [],
        intercept: [{ pattern: '*.corp', identity: 'system' }],
      },
      notes: [
        "gives its programs the proxy it names and uses it for the host's own requests for the session",
        'ends TLS for *.corp to present your client certificate to them',
      ],
    },
  );
  assert.throws(() => new ProxyItem().apply({ harness: 'claude', values: { mode: 'proxy' } }), BadValue);
  assert.deepEqual(effect({ args: ['--fast'] }), { env: [], args: ['--fast'], settings: [], notes: [] });
  assert.deepEqual(library('profile_test', '1', { providers: [ProfileItem, ModelItem] }).kinds, ['profile_item']);
});

test('CatalogSource subclasses and SettingsSection describe and handle requests', () => {
  assert.deepEqual(fieldsOf({ fields: '{"command":"cat"}' }), { command: 'cat' });
  assert.throws(() => fieldsOf({ fields: '[]' }), BadValue);
  assert.equal(textOfCatalog(new Uint8Array([0xef, 0xbb, 0xbf, 0x61, 0x62])), 'ab');
  assert.equal(CatalogSource.id, undefined);
  assert.equal(ReadCatalogSource.id, 'read');
  assert.equal(CommandCatalogSource.id, 'command');
  assert.equal(new ReadCatalogSource().describe().id, 'read');
  assert.deepEqual(library('source_test', '1', { providers: [ReadCatalogSource, CommandCatalogSource] }).kinds, [
    'catalog_source',
  ]);

  class Region extends SettingsSection {
    static section = 'region';
    static title = 'Region';
    static stored = 'provider';
    static values = { type: 'object', properties: { name: { type: 'string' } } };
  }
  const region = new Region();
  assert.deepEqual(region.describe(), {
    id: 'region',
    title: 'Region',
    stored: 'provider',
    schema: { type: 'object', properties: { name: { type: 'string' } } },
  });
  assert.deepEqual(region.read(), {});
});
