import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import test from 'node:test';

import {
  APP_PATH,
  CONVERSATION,
  DOCUMENT,
  appAddress,
  documentName,
  hashKey,
  normalizeDocumentUrl,
  relationForDocument,
  relationForMail,
  sameRelation,
} from '../taskpane/relation.js';

test('a mail conversation relates by a hash of its id, not by the id', async () => {
  const id = 'AAQkAGI2THVSAAA=';
  const relation = await relationForMail({ conversationId: id, subject: 'Quarterly numbers' });
  assert.equal(relation.kind, CONVERSATION);
  assert.match(relation.key, /^[0-9a-f]{32}$/);
  assert.ok(!relation.key.includes(id));
  assert.equal(relation.label, 'Quarterly numbers');
});

test('the same conversation gives the same key on every call, and another conversation another', async () => {
  const first = await relationForMail({ conversationId: 'C1', subject: 'a' });
  const again = await relationForMail({ conversationId: 'C1', subject: 'a different subject' });
  const other = await relationForMail({ conversationId: 'C2', subject: 'a' });
  assert.equal(first.key, again.key, 'a subject is a label, not part of the key');
  assert.notEqual(first.key, other.key);
  assert.ok(sameRelation(first, again));
  assert.ok(!sameRelation(first, other));
});

test('a mail item with no conversation id has no relation', async () => {
  for (const conversationId of [null, undefined, '', '   ']) {
    assert.equal(await relationForMail({ conversationId, subject: 'x' }), null, JSON.stringify(conversationId));
  }
  assert.equal(await relationForMail(null), null, 'nothing selected in a pinned pane');
  assert.equal(await relationForMail(undefined), null);
});

test('a reply whose subject was changed has a new conversation id, so a new relation', async () => {
  const before = await relationForMail({ conversationId: 'CONV-1', subject: 'Re: plan' });
  const after = await relationForMail({ conversationId: 'CONV-2', subject: 'Re: a new plan' });
  assert.ok(!sameRelation(before, after));
});

test('a subject is one trimmed line of at most 80 characters, and a blank one is no label', async () => {
  const long = await relationForMail({ conversationId: 'C', subject: `  one\n two\t${'x'.repeat(200)}  ` });
  assert.ok(long.label.length <= 80);
  assert.ok(long.label.startsWith('one two xxx'));
  assert.ok(long.label.endsWith('\u2026'));
  for (const subject of [null, undefined, '', '  ', 7]) {
    const relation = await relationForMail({ conversationId: 'C', subject });
    assert.equal('label' in relation, false, JSON.stringify(subject));
  }
});

test('a document relates by its address, the host application beside it', async () => {
  const url = 'https://contoso.sharepoint.com/sites/team/Shared%20Documents/Plan.docx';
  const word = await relationForDocument({ app: 'word', url });
  assert.equal(word.kind, DOCUMENT);
  assert.match(word.key, /^[0-9a-f]{32}$/);
  assert.equal(word.label, 'Plan.docx');
  const again = await relationForDocument({ app: 'Word', url });
  assert.equal(word.key, again.key, 'the application name is not case-sensitive');
  const sheet = await relationForDocument({ app: 'excel', url });
  assert.notEqual(word.key, sheet.key, 'one address under two applications is two documents');
});

test('a document address with a query and a fragment is told apart by its query, not its fragment', async () => {
  const base = 'https://contoso.sharepoint.com/_layouts/15/Doc.aspx';
  const one = await relationForDocument({ app: 'word', url: `${base}?sourcedoc=%7BAAA%7D&action=edit#page=3` });
  const sameWithOtherFragment = await relationForDocument({ app: 'word', url: `${base}?sourcedoc=%7BAAA%7D&action=edit#page=9` });
  const other = await relationForDocument({ app: 'word', url: `${base}?sourcedoc=%7BBBB%7D&action=edit` });
  assert.ok(sameRelation(one, sameWithOtherFragment));
  assert.ok(!sameRelation(one, other));
  assert.equal(normalizeDocumentUrl('HTTPS://Contoso.SharePoint.com/a/B.docx#x'), 'https://contoso.sharepoint.com/a/B.docx');
});

test('an unsaved document, or one whose address is unavailable, has no relation', async () => {
  for (const url of [null, undefined, '', '  ']) {
    assert.equal(await relationForDocument({ app: 'word', url }), null, JSON.stringify(url));
  }
  assert.equal(await relationForDocument(null), null);
});

test('a local file path is kept as written, and named by its last part', async () => {
  const relation = await relationForDocument({ app: 'excel', url: 'C:\\Users\\someone\\Documents\\Budget 2027.xlsx' });
  assert.equal(relation.label, 'Budget 2027.xlsx');
  assert.ok(!relation.key.includes('someone'));
  assert.equal(documentName('https://host.example/'), undefined, 'an address with no file name');
  assert.equal(documentName('https://host.example/a/%E0%A4%A.docx'), '%E0%A4%A.docx', 'a bad escape is shown as it is');
});

test('a relation and nothing are the same only as nothing', () => {
  assert.ok(sameRelation(null, undefined));
  assert.ok(!sameRelation(null, { kind: CONVERSATION, key: 'k' }));
  assert.ok(!sameRelation({ kind: CONVERSATION, key: 'k' }, { kind: DOCUMENT, key: 'k' }), 'kinds never relate');
});

test('the key is stable, and fixed for a known input', async () => {
  const a = await hashKey('conversation', 'abc');
  const b = await hashKey('conversation', 'abc');
  assert.equal(a, b);
  assert.equal(a, createHash('sha256').update('conversation\nabc').digest('hex').slice(0, 32));
  assert.notEqual(a, await hashKey('document', 'abc'));
});

test('the app address asks for the chat alone and carries the relation as the app reads it', async () => {
  const relation = await relationForMail({ conversationId: 'C1', subject: 'Plan & budget' });
  const address = new URL(appAddress('https://canoa.example', relation));
  assert.equal(address.origin, 'https://canoa.example');
  assert.equal(address.pathname, APP_PATH);
  assert.equal(address.searchParams.get('mode'), 'chat');
  assert.equal(address.searchParams.get('relation'), `conversation:${relation.key}`);
  assert.equal(address.searchParams.get('label'), 'Plan & budget');
  assert.equal(address.searchParams.has('scope'), false);
  const none = new URL(appAddress('https://canoa.example', null));
  assert.equal(none.search, '?mode=chat', 'a new message or an unsaved document is unrelated');
});
