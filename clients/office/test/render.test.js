import assert from 'node:assert/strict';
import test from 'node:test';

import { IDS, checkOrigin, parseArguments, render } from '../render.mjs';

/** Whether `xml`'s tags open and close in order: no parser is needed to catch a template edit that breaks the nesting. */
function balanced(xml) {
  const stack = [];
  for (const match of xml.matchAll(/<(\/?)([A-Za-z][\w:.-]*)[^<>]*?(\/?)>/g)) {
    const [, closing, name, selfClosing] = match;
    if (selfClosing) continue;
    if (closing) {
      if (stack.pop() !== name) return false;
    } else {
      stack.push(name);
    }
  }
  return stack.length === 0;
}

const origin = 'https://canoa.example';

test('the nesting check sees a broken one', () => {
  assert.ok(balanced('<a><b/></a>'));
  assert.ok(!balanced('<a><b></a></b>'));
  assert.ok(!balanced('<a>'));
});

test('the Outlook manifest names the origin in every address and nowhere else', () => {
  const xml = render('outlook', { origin });
  assert.ok(balanced(xml.replace(/<\?xml[^>]*\?>|<!--[\s\S]*?-->/g, '')));
  assert.ok(!xml.includes('{{'), 'every placeholder is filled');
  assert.ok(xml.includes(`<AppDomain>${origin}</AppDomain>`));
  assert.ok(xml.includes(`<SourceLocation DefaultValue="${origin}/office/taskpane.html"/>`));
  const addresses = [...xml.replace(/<!--[\s\S]*?-->/g, '').matchAll(/https?:\/\/[^"<\s]+/g)].map((m) => m[0]).filter((a) => !a.includes('schemas.microsoft.com') && !a.includes('www.w3.org'));
  assert.ok(addresses.length > 0);
  for (const address of addresses) assert.ok(address.startsWith(`${origin}/office/`) || address === origin, address);
  assert.ok(xml.includes('<Permissions>ReadItem</Permissions>'), 'read item, not read-write');
  assert.ok(xml.includes('<SupportsPinning>true</SupportsPinning>'));
  assert.ok(xml.includes(IDS.outlook));
});

test('the documents manifest names the hosts it is asked for, all three by default', () => {
  const all = render('documents', { origin });
  assert.ok(balanced(all.replace(/<\?xml[^>]*\?>|<!--[\s\S]*?-->/g, '')));
  for (const name of ['Document', 'Workbook', 'Presentation']) assert.ok(all.includes(`<Host Name="${name}"/>`), name);
  assert.ok(all.includes('<Permissions>ReadDocument</Permissions>'));
  const one = render('documents', { origin, hosts: ['Workbook', 'Workbook'] });
  assert.equal(one.match(/<Host Name=/g).length, 1);
  assert.throws(() => render('documents', { origin, hosts: ['Mailbox'] }), /unknown host/);
});

test('the ids are fixed per kind, so a second render for another origin is the same add-in', () => {
  assert.notEqual(IDS.outlook, IDS.documents);
  assert.ok(render('outlook', { origin }).includes(IDS.outlook));
  assert.ok(render('outlook', { origin: 'https://other.example:8443' }).includes(IDS.outlook));
  assert.match(IDS.outlook, /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/);
});

test('an origin that is not https, or has more than a host and a port, is refused', () => {
  assert.equal(checkOrigin('https://canoa.example:8443'), 'https://canoa.example:8443');
  assert.equal(checkOrigin('https://canoa.example/'), 'https://canoa.example');
  for (const bad of ['http://canoa.example', 'https://canoa.example/path', 'https://a@canoa.example', 'https://canoa.example?x=1', 'canoa.example', '']) {
    assert.throws(() => checkOrigin(bad), Error, bad);
  }
  assert.throws(() => render('nonsense', { origin }), /unknown manifest/);
});

test('an origin cannot break out of an attribute', () => {
  assert.throws(() => checkOrigin('https://x.example"/><Evil'), Error);
});

test('the command line reads a kind and its options', () => {
  const parsed = parseArguments(['documents', '--origin', origin, '--hosts', 'Document,Workbook', '--out', 'm.xml']);
  assert.deepEqual(parsed, { kind: 'documents', origin, hosts: ['Document', 'Workbook'], out: 'm.xml' });
  assert.throws(() => parseArguments(['outlook', '--origin']), /unexpected argument/);
  assert.throws(() => parseArguments(['outlook', '--nope', 'x']), /unexpected argument/);
});

test('no comment holds two hyphens, which XML forbids there', () => {
  for (const kind of ['outlook', 'documents']) {
    for (const [comment] of render(kind, { origin }).matchAll(/<!--[\s\S]*?-->/g)) {
      assert.ok(!comment.slice(4, -3).includes('--'), `${kind}: ${comment}`);
    }
  }
});
