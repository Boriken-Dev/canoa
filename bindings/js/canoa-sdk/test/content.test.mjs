// Unit tests for ContentSource and CatalogGenerator subclasses.
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { NotFound, Unsupported } from 'guatiao';
import { library } from 'guatiao/provide';

import {
  CatalogGenerator,
  ContentSource,
  listFound,
  openedInline,
  refused,
  resolved,
  scheme,
  statDir,
  statFile,
  statFound,
} from '../src/index.js';

class Files extends ContentSource {
  static source = 'files_of_team';
  static title = 'Team files';
  static schemes = [scheme('team', 'Team files', 'team://name/path', { lists: true })];

  open(request) {
    if (request.target.uri.endsWith('/missing')) return refused('not_found', 'team: no such file');
    return openedInline(statFile(), request.target.uri);
  }

  stat() {
    return statFound(statFile({ size: 3n }));
  }
}

class Listing extends CatalogGenerator {
  static generator = 'team_listing';
  static title = "A team's listing";
  static types = ['skills'];
  static grammar = 'team://name';

  generate(request, spawner, host) {
    try {
      return host.openText(host.child(request.source, 'list.json'), 1024n);
    } finally {
      spawner.destroy();
      host.destroy();
    }
  }
}

function host(files) {
  const held = { destroyed: false };
  return Object.assign(held, {
    child: (base, name) => `${base.replace(/\/$/, '')}/${name}`,
    openText: (uri) => {
      if (!(uri in files)) throw new NotFound('no such document');
      return files[uri];
    },
    destroy() {
      held.destroyed = true;
    },
  });
}

test('a content source is described by its schemes and answers with data or a refusal', () => {
  assert.equal(Files.id, 'files_of_team');
  assert.equal(ContentSource.id, undefined);
  const described = new Files().describe();
  assert.equal(described.id, 'files_of_team');
  assert.deepEqual(described.schemes, [
    { name: 'team', title: 'Team files', grammar: 'team://name/path', lists: true, resolves: false, ranges: false },
  ]);
  const opened = new Files().open({ target: { uri: 'team://core/a' } });
  assert.deepEqual(new TextDecoder().decode(opened.opened.delivery.inline), 'team://core/a');
  assert.equal(opened.opened.stat.size, 13n);
  assert.deepEqual(new Files().open({ target: { uri: 'team://core/missing' } }), {
    refusal: { kind: 'not_found', why: 'team: no such file' },
  });
  assert.equal(new Files().stat().stat.size, 3n);
  assert.throws(() => new Files().list({}), Unsupported);
  assert.throws(() => new Files().resolve({}), Unsupported);
  assert.deepEqual(listFound([{ path: 'a', kind: 'file' }], '2'), { listing: { entries: [{ path: 'a', kind: 'file' }], next: '2' } });
  assert.deepEqual(listFound([]), { listing: { entries: [] } });
  assert.deepEqual(resolved('team://x?ref=1', '1', true), { resolved: { uri: 'team://x?ref=1', revision: '1', immutable: true } });
  assert.equal(statDir().kind, 'dir');
  assert.deepEqual(library('content_test', '1', { providers: [Files, Listing] }).kinds, ['content_source', 'catalog_generator']);
});

test('a catalog generator reads through the host and lets go of what it was handed', () => {
  assert.equal(Listing.id, 'team_listing');
  const reading = host({ 'team://core/list.json': '{"skills": []}' });
  const log = [];
  const spawner = { destroy: () => log.push('spawner') };
  assert.equal(new Listing().generate({ kind: 'skills', label: 'x', source: 'team://core', fields: '{}' }, spawner, reading), '{"skills": []}');
  assert.deepEqual(log, ['spawner']);
  assert.equal(reading.destroyed, true);
  const described = new Listing().describe();
  assert.deepEqual([described.id, described.types, described.source, described.grammar], ['team_listing', ['skills'], 'required', 'team://name']);
  assert.throws(() => new Listing().generate({ source: 'team://other' }, spawner, host({})), NotFound);
});

test('a generator that reads no source says so, and one that is not implemented lets go and says unsupported', () => {
  class Runs extends CatalogGenerator {
    static generator = 'runs';
    static title = 'Runs';
    static reads = 'absent';
  }
  assert.equal(new Runs().describe().source, 'absent');
  const reading = host({});
  assert.throws(() => new Runs().generate({}, { destroy() {} }, reading), Unsupported);
  assert.equal(reading.destroyed, true);
});
