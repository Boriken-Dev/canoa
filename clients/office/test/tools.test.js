import assert from 'node:assert/strict';
import test from 'node:test';

import { KEY_HEADER } from '../taskpane/launch.js';
import { createPaneTools } from '../taskpane/tools.js';
import { excelTools } from '../taskpane/tools-excel.js';
import { outlookTools } from '../taskpane/tools-outlook.js';
import { wordTools } from '../taskpane/tools-word.js';

const secret = 'ab12'.repeat(16);
const STREAM = 'a'.repeat(32);
const OTHER = 'b'.repeat(32);
const item = { kind: 'document', key: '0123456789abcdef0123456789abcdef', label: 'plan.docx' };
const tick = (ms = 5) => new Promise((done) => setTimeout(done, ms));

/** Waits until `found()` is truthy. */
async function until(found, what) {
  for (let look = 0; look < 400; look += 1) {
    const value = found();
    if (value) return value;
    await tick(5);
  }
  assert.fail(`never happened: ${what}`);
}

/**
 * The host as the pane sees it: `/office/tools/open|poll|send|close` over `fetch`. `say` queues what the host has for the pane, as
 * the host does: numbered from 1 and kept until a poll names a number at or past it. `sent` is what the pane answered, `polls` the
 * `after` of each poll, `lose` makes the answers of the next polls fail on the way, after the host has them ready, and `resend`
 * makes the next polls answer everything it ever said, as a host that did not hear the acknowledgement would.
 */
function fakeHost({ refuse } = {}) {
  const host = { queue: [], sent: [], opened: [], closed: [], headers: [], polls: [], session: 0, forget: false, ended: false, last: 0, lose: 0, resend: 0, said: [] };
  host.say = (message) => {
    host.last += 1;
    host.queue.push({ number: host.last, message });
    host.said.push({ number: host.last, message });
  };
  host.fetchImpl = async (url, init = {}) => {
    const [path, query] = url.split('?');
    const params = new URLSearchParams(query);
    host.headers.push(init.headers ?? {});
    const answer = (status, body) => ({ ok: status < 300, status, json: async () => body });
    if (path.endsWith('/open')) {
      if (refuse) return answer(409, { why: refuse });
      host.opened.push(params.get('app'));
      host.session += 1;
      host.forget = false;
      host.queue = [];
      host.said = [];
      host.last = 0;
      return answer(200, { session: `s${host.session}` });
    }
    if (path.endsWith('/poll')) {
      if (host.ended || host.forget || params.get('session') !== `s${host.session}`) return answer(410, { ended: true });
      const after = Number(params.get('after'));
      host.polls.push(after);
      host.queue = host.queue.filter(({ number }) => number > after);
      if (host.queue.length === 0) await tick(10);
      const held = host.resend > 0 ? host.said.slice() : host.queue.slice();
      if (host.resend > 0) host.resend -= 1;
      if (host.lose > 0) {
        host.lose -= 1;
        throw new TypeError('the request was aborted');
      }
      return answer(200, { first: held[0]?.number ?? host.last + 1, messages: held.map(({ message }) => message) });
    }
    if (path.endsWith('/send')) {
      host.sent.push(JSON.parse(init.body));
      return answer(200, { ok: true });
    }
    if (path.endsWith('/close')) {
      host.closed.push(params.get('session'));
      host.ended = true;
      return answer(200, { ok: true });
    }
    throw new Error(`not a route: ${url}`);
  };
  host.open = (stream = STREAM, agent = 'Claude') => host.say({ type: 'open', stream, session: { agent, via: 'app' } });
  host.request = (id, method, params, stream = STREAM) => host.say({ type: 'mcp', stream, message: { jsonrpc: '2.0', id, method, params } });
  host.answerTo = (id) => until(() => host.sent.find((m) => m.message?.id === id), `an answer to ${id}`).then((m) => m.message);
  return host;
}

/** Word as the pane's tools see it. */
function wordStub(document = { text: 'The plan.', selection: 'plan', comments: [] }) {
  const calls = [];
  const word = {
    calls,
    document,
    run: async (work) => {
      const context = {
        sync: async () => {},
        document: {
          body: {
            load() {},
            get text() { return document.text; },
            insertText: (text, where) => calls.push(['body.insertText', text, where]),
            getComments: () => ({ load() {}, items: document.comments }),
          },
          getSelection: () => ({
            load() {},
            get text() { return document.selection; },
            insertText: (text, where) => calls.push(['selection.insertText', text, where]),
            insertComment: (text) => calls.push(['selection.insertComment', text]),
          }),
        },
      };
      return work(context);
    },
  };
  return word;
}

function setup({ app = 'word', tools = wordTools(wordStub()), answer = true, relation = async () => item, askMs = 50, key = secret, host = fakeHost() } = {}) {
  const asked = [];
  const ask = async (question) => {
    asked.push(question);
    return typeof answer === 'function' ? answer(question) : answer;
  };
  const status = [];
  const pane = createPaneTools({ app, tools, relation, ask, key, fetchImpl: host.fetchImpl, sleep: () => tick(5), askMs, onStatus: (s) => status.push(s) });
  return { host, pane, asked, status };
}

test('the pane registers as its application with the secret, and a session lists the tools while the pane is open', async () => {
  const { host, pane, status } = setup();
  void pane.start();
  await until(() => host.opened.length === 1, 'registered');
  assert.deepEqual(host.opened, ['word']);
  assert.equal(host.headers[0][KEY_HEADER], secret, 'the secret goes with every request');
  host.open();
  host.request(1, 'tools/list');
  const listed = await host.answerTo(1);
  assert.deepEqual(
    listed.result.tools.map((t) => t.name),
    ['word_get_text', 'word_get_selection', 'word_insert_text', 'word_replace_selection', 'word_list_comments', 'word_add_comment'],
  );
  for (const tool of listed.result.tools) assert.equal(tool.inputSchema.type, 'object', tool.name);
  assert.deepEqual(pane.sessions(), [{ agent: 'Claude', where: 'the Canoa app on this computer' }]);
  assert.ok(status.includes('connected'));

  // Closing the pane tells the host, which takes the tools away from every session; the pane answers no more.
  await pane.stop();
  assert.deepEqual(host.closed, ['s1']);
  host.open(OTHER);
  host.request(2, 'tools/list', undefined, OTHER);
  await tick(60);
  assert.equal(host.sent.filter((m) => m.message?.id === 2).length, 0);
  await until(() => status.includes('stopped'), 'the loop ended');
});

test('a poll whose answer is lost is followed by one that gets the same messages again, and each is handled once', async () => {
  let runs = 0;
  const count = { name: 'count', description: 'Counts', inputSchema: { type: 'object' }, run: async () => ({ runs: (runs += 1) }) };
  const { host, pane } = setup({ tools: [count] });
  void pane.start();
  await until(() => host.opened.length === 1, 'registered');
  await until(() => host.polls.length > 0, 'polling');
  host.lose = 1;
  host.open();
  host.request(1, 'tools/call', { name: 'count', arguments: {} });
  // The poll that took the open and the call failed on the way; the next one asks from nothing handled and gets both.
  const answered = await host.answerTo(1);
  assert.equal(JSON.parse(answered.result.content[0].text).runs, 1);
  assert.equal(host.lose, 0, 'the answer was lost');
  assert.ok(host.polls.filter((after) => after === 0).length >= 2, `asked from nothing twice: ${host.polls}`);
  assert.deepEqual(pane.sessions(), [{ agent: 'Claude', where: 'the Canoa app on this computer' }]);

  await pane.stop();
});

test('a message the host sends twice with the same number is handled once, and a new registration starts the numbers again', async () => {
  let runs = 0;
  const count = { name: 'count', description: 'Counts', inputSchema: { type: 'object' }, run: async () => ({ runs: (runs += 1) }) };
  const { host, pane } = setup({ tools: [count] });
  void pane.start();
  await until(() => host.opened.length === 1, 'registered');
  host.open();
  host.request(1, 'tools/call', { name: 'count', arguments: {} });
  await host.answerTo(1);
  // The host says everything again, with a new call after it: the first two were handled and are not run again.
  host.request(2, 'tools/call', { name: 'count', arguments: {} });
  host.resend = 1;
  await host.answerTo(2);
  assert.equal(runs, 2, 'the repeated call did not run again');
  assert.equal(host.sent.filter((m) => m.message?.id === 1).length, 1, 'and was not answered twice');

  // The host forgets the session: the pane registers again and its numbers start from 1.
  host.forget = true;
  await until(() => host.opened.length === 2, 'registered again');
  host.open();
  host.request(1, 'tools/call', { name: 'count', arguments: {} });
  await until(() => host.sent.filter((m) => m.message?.id === 1).length === 2, 'the new session is served');
  assert.equal(runs, 3);
  await pane.stop();
});

test('a read returns the open item\'s content, and says which item it was', async () => {
  const { host, pane } = setup();
  void pane.start();
  await until(() => host.opened.length === 1, 'registered');
  host.open();
  host.request(1, 'tools/call', { name: 'word_get_text', arguments: {} });
  const answer = await host.answerTo(1);
  assert.equal(answer.result.isError, false);
  const body = JSON.parse(answer.result.content[0].text);
  assert.equal(body.text, 'The plan.');
  assert.deepEqual(body.item, item, 'the relation of the call to the open item');
  host.request(2, 'tools/call', { name: 'word_get_selection', arguments: {} });
  assert.deepEqual(JSON.parse((await host.answerTo(2)).result.content[0].text).text, 'plan');
  await pane.stop();
});

test('a write is asked in the pane, with who asks and what it would do, and is done after yes only', async () => {
  const word = wordStub();
  const { host, pane, asked } = setup({ tools: wordTools(word), answer: true });
  void pane.start();
  await until(() => host.opened.length === 1, 'registered');
  host.open(STREAM, 'Codex');
  host.request(1, 'tools/call', { name: 'word_replace_selection', arguments: { text: 'schedule' } });
  const answer = await host.answerTo(1);
  assert.equal(asked.length, 1);
  assert.equal(asked[0].agent, 'Codex');
  assert.equal(asked[0].where, 'the Canoa app on this computer');
  assert.equal(asked[0].tool, 'word_replace_selection');
  assert.match(asked[0].summary, /schedule/);
  assert.equal(asked[0].item, 'plan.docx');
  assert.equal(answer.result.isError, false);
  assert.deepEqual(word.calls, [['selection.insertText', 'schedule', 'Replace']]);
  await pane.stop();
});

test('a write the person refuses, or does not answer, changes nothing and says so', async () => {
  const word = wordStub();
  // Refused.
  const no = setup({ tools: wordTools(word), answer: false });
  void no.pane.start();
  await until(() => no.host.opened.length === 1, 'registered');
  no.host.open();
  no.host.request(1, 'tools/call', { name: 'word_insert_text', arguments: { text: 'x' } });
  const refused = await no.host.answerTo(1);
  assert.equal(refused.result.isError, true);
  assert.match(refused.result.content[0].text, /did not allow/);
  await no.pane.stop();

  // Never answered: the question is withdrawn when the time is up.
  let withdrawn = false;
  const silent = setup({
    tools: wordTools(word),
    askMs: 30,
    answer: ({ signal }) => new Promise((resolve) => signal.addEventListener('abort', () => { withdrawn = true; resolve(true); })),
  });
  void silent.pane.start();
  await until(() => silent.host.opened.length === 1, 'registered');
  silent.host.open();
  silent.host.request(1, 'tools/call', { name: 'word_add_comment', arguments: { text: 'look' } });
  const none = await silent.host.answerTo(1);
  assert.equal(none.result.isError, true, 'no answer is a refusal, even for a pane that answers yes once it is withdrawn');
  assert.ok(withdrawn, 'the pane is told to take the question down');
  assert.deepEqual(word.calls, [], 'nothing was written');
  await silent.pane.stop();
});

test('a write is refused when the open item changed while the person was asked', async () => {
  const word = wordStub();
  let current = item;
  const { host, pane } = setup({
    tools: wordTools(word),
    relation: async () => current,
    answer: async () => {
      current = { ...item, key: 'f'.repeat(32), label: 'another.docx' };
      return true;
    },
  });
  void pane.start();
  await until(() => host.opened.length === 1, 'registered');
  host.open();
  host.request(1, 'tools/call', { name: 'word_insert_text', arguments: { text: 'x', location: 'document_end' } });
  const answer = await host.answerTo(1);
  assert.equal(answer.result.isError, true);
  assert.match(answer.result.content[0].text, /item changed/);
  assert.deepEqual(word.calls, []);
  await pane.stop();
});

test('a session that is not open, an unknown tool, a bad argument and an unknown method are answered, never run', async () => {
  const word = wordStub();
  const { host, pane } = setup({ tools: wordTools(word) });
  void pane.start();
  await until(() => host.opened.length === 1, 'registered');
  host.open();
  host.request(1, 'tools/call', { name: 'word_delete_everything', arguments: {} });
  assert.match((await host.answerTo(1)).result.content[0].text, /No tool/);
  host.request(2, 'tools/call', { name: 'word_insert_text', arguments: { text: 'x', location: 'nowhere' } });
  const bad = await host.answerTo(2);
  assert.equal(bad.result.isError, true);
  host.request(3, 'resources/list');
  assert.equal((await host.answerTo(3)).error.code, -32601);
  host.request(4, 'tools/list', undefined, OTHER);
  await tick(60);
  assert.equal(host.sent.filter((m) => m.message?.id === 4).length, 0, 'a stream the host never opened is not answered');
  assert.deepEqual(word.calls, []);
  await pane.stop();
});

test('a host that forgot the session, or was away, is registered with again with no reload', async () => {
  const { host, pane, status } = setup();
  void pane.start();
  await until(() => host.opened.length === 1, 'registered');
  host.forget = true;
  await until(() => host.opened.length === 2, 'registered again');
  host.open();
  host.request(1, 'tools/list');
  assert.ok((await host.answerTo(1)).result.tools.length > 0);
  assert.ok(status.includes('waiting') && status.filter((s) => s === 'connected').length === 2);
  await pane.stop();
});

test('a host that refuses the registration is tried again later and nothing is offered meanwhile', async () => {
  const { host, pane, status } = setup({ host: fakeHost({ refuse: 'this add-in\'s secret is needed' }) });
  void pane.start();
  await until(() => status.includes('waiting'), 'waiting');
  assert.deepEqual(host.opened, []);
  assert.deepEqual(pane.sessions(), []);
  await pane.stop();
});

test('Outlook: the open message is read, its attachment fetched, and a reply is only opened as a form after yes', async () => {
  const forms = [];
  const done = (value) => (callback) => callback({ status: 'succeeded', value });
  const office = {
    AsyncResultStatus: { Succeeded: 'succeeded', Failed: 'failed' },
    CoercionType: { Text: 'text' },
    context: {
      mailbox: {
        item: {
          subject: 'Quarterly numbers',
          from: { displayName: 'Ana', emailAddress: 'ana@example.com' },
          to: [{ displayName: 'Me', emailAddress: 'me@example.com' }],
          cc: [],
          conversationId: 'CONV-1',
          internetMessageId: '<m1@example.com>',
          dateTimeCreated: new Date('2026-11-17T10:00:00Z'),
          attachments: [{ id: 'att1', name: 'q3.xlsx', contentType: 'application/vnd.ms-excel', size: 2048, isInline: false }],
          body: { getAsync: (type, callback) => done('Please see the attached.')(callback) },
          getAttachmentContentAsync: (id, callback) => done({ format: 'base64', content: 'AAEC' })(callback),
          displayReplyForm: (form) => forms.push(['reply', form]),
          displayReplyAllForm: (form) => forms.push(['all', form]),
        },
      },
    },
  };
  const { host, pane, asked } = setup({ app: 'outlook', tools: outlookTools(office), relation: async () => ({ kind: 'conversation', key: 'c'.repeat(32), label: 'Quarterly numbers' }) });
  void pane.start();
  await until(() => host.opened.length === 1, 'registered');
  assert.deepEqual(host.opened, ['outlook']);
  host.open();
  host.request(1, 'tools/call', { name: 'outlook_get_item', arguments: {} });
  const read = JSON.parse((await host.answerTo(1)).result.content[0].text);
  assert.equal(read.subject, 'Quarterly numbers');
  assert.deepEqual(read.from, { name: 'Ana', address: 'ana@example.com' });
  assert.equal(read.body, 'Please see the attached.');
  assert.equal(read.conversationId, 'CONV-1');
  assert.deepEqual(read.attachments, [{ id: 'att1', name: 'q3.xlsx', contentType: 'application/vnd.ms-excel', size: 2048, inline: false }]);
  assert.equal(read.item.kind, 'conversation');
  host.request(2, 'tools/call', { name: 'outlook_get_attachment', arguments: { id: 'att1' } });
  assert.deepEqual(JSON.parse((await host.answerTo(2)).result.content[0].text).content, 'AAEC');
  assert.equal(asked.length, 0, 'reading asks nothing');
  host.request(3, 'tools/call', { name: 'outlook_draft_reply', arguments: { body: 'Thanks <b>Ana</b>\nBest' } });
  const drafted = JSON.parse((await host.answerTo(3)).result.content[0].text);
  assert.equal(drafted.sent, false);
  assert.equal(asked.length, 1);
  assert.equal(forms.length, 1);
  assert.equal(forms[0][1].htmlBody, '<div>Thanks &lt;b&gt;Ana&lt;/b&gt;<br>Best</div>', 'the text is escaped, not run as HTML');
  await pane.stop();
});

test('Outlook: the open message is named by identifiers alone, and its body is never read to do it', async () => {
  const done = (value) => (callback) => callback({ status: 'succeeded', value });
  const office = (item) => ({
    AsyncResultStatus: { Succeeded: 'succeeded', Failed: 'failed' },
    CoercionType: { Text: 'text' },
    context: { mailbox: { item } },
  });
  const reading = {
    subject: 'Quarterly numbers',
    conversationId: 'CONV-1',
    internetMessageId: '<m1@example.com>',
    itemId: 'AAMkAGI2',
    body: { getAsync: () => assert.fail('the body is not read to say which message is open') },
    attachments: [{ id: 'att1', name: 'q3.xlsx' }],
  };
  const { host, pane } = setup({ app: 'outlook', tools: outlookTools(office(reading)), relation: async () => null });
  void pane.start();
  await until(() => host.opened.length === 1, 'registered');
  host.open();
  host.request(1, 'tools/list');
  const listed = (await host.answerTo(1)).result.tools.map((tool) => tool.name);
  assert.equal(listed[0], 'outlook_open_item', 'the tool that names the message is offered first');
  host.request(2, 'tools/call', { name: 'outlook_open_item', arguments: {} });
  const said = JSON.parse((await host.answerTo(2)).result.content[0].text);
  assert.deepEqual(
    { ...said, item: undefined },
    { open: true, mode: 'read', internetMessageId: '<m1@example.com>', itemId: 'AAMkAGI2', conversationId: 'CONV-1', item: undefined },
  );
  assert.deepEqual(Object.keys(said).sort(), ['conversationId', 'internetMessageId', 'item', 'itemId', 'mode', 'open']);
  assert.ok(!JSON.stringify(said).includes('Please'), 'no text of the message');
  await pane.stop();

  // A message being written has no Internet message id and no item id until it is saved.
  const writing = {
    subject: { getAsync: (callback) => done('Re: numbers')(callback) },
    conversationId: 'CONV-2',
    body: { getAsync: () => assert.fail('the body is not read') },
  };
  const second = setup({ app: 'outlook', tools: outlookTools(office(writing)), relation: async () => null });
  void second.pane.start();
  await until(() => second.host.opened.length === 1, 'registered');
  second.host.open();
  second.host.request(1, 'tools/call', { name: 'outlook_open_item', arguments: {} });
  const composing = JSON.parse((await second.host.answerTo(1)).result.content[0].text);
  assert.equal(composing.mode, 'compose');
  assert.equal(composing.internetMessageId, null);
  assert.equal(composing.itemId, null);
  assert.equal(composing.conversationId, 'CONV-2');
  await second.pane.stop();

  // An identifier longer than a header line can hold is cut, so a message cannot make the answer large.
  const long = { ...reading, internetMessageId: `<${'a'.repeat(5000)}@example.com>` };
  const third = setup({ app: 'outlook', tools: outlookTools(office(long)), relation: async () => null });
  void third.pane.start();
  await until(() => third.host.opened.length === 1, 'registered');
  third.host.open();
  third.host.request(1, 'tools/call', { name: 'outlook_open_item', arguments: {} });
  const cut = JSON.parse((await third.host.answerTo(1)).result.content[0].text);
  assert.equal(cut.internetMessageId.length, 998);
  await third.pane.stop();
});

test('Outlook: with nothing open the tools say so', async () => {
  const office = { AsyncResultStatus: { Succeeded: 'succeeded' }, CoercionType: { Text: 'text' }, context: { mailbox: { item: null } } };
  const { host, pane } = setup({ app: 'outlook', tools: outlookTools(office), relation: async () => null });
  void pane.start();
  await until(() => host.opened.length === 1, 'registered');
  host.open();
  host.request(1, 'tools/call', { name: 'outlook_get_item', arguments: {} });
  const answer = await host.answerTo(1);
  assert.equal(answer.result.isError, true);
  assert.match(answer.result.content[0].text, /No message is open/);
  await pane.stop();
});

test('Excel: the selection and a range are read, and a write must fit its range and be allowed', async () => {
  const cells = { 'Sheet1!A1:B2': [[1, 2], [3, 4]] };
  const written = [];
  const rangeFor = (address, values = cells[address]) => ({
    address,
    values,
    formulas: values,
    rowCount: values.length,
    columnCount: values[0].length,
    load() {},
    set values(next) { written.push([address, next]); },
    get values() { return values; },
  });
  const excel = {
    run: async (work) =>
      work({
        sync: async () => {},
        workbook: {
          getSelectedRange: () => rangeFor('Sheet1!A1:B2'),
          worksheets: { getActiveWorksheet: () => ({ getRange: (cellsAddress) => rangeFor(`Sheet1!${cellsAddress}`, cells[`Sheet1!${cellsAddress}`] ?? [[0]]) }), getItem: (name) => ({ getRange: (cellsAddress) => rangeFor(`${name}!${cellsAddress}`, cells[`${name}!${cellsAddress}`] ?? [[0]]) }) },
          tables: { load() {}, items: [{ name: 'Sales', getRange: () => rangeFor('Sheet1!A1:B2') }], getItem: () => ({ getRange: () => rangeFor('Sheet1!A1:B2') }) },
        },
      }),
  };
  const { host, pane, asked } = setup({ app: 'excel', tools: excelTools(excel), answer: true });
  void pane.start();
  await until(() => host.opened.length === 1, 'registered');
  assert.deepEqual(host.opened, ['excel']);
  host.open();
  host.request(1, 'tools/call', { name: 'excel_get_selection', arguments: {} });
  const selection = JSON.parse((await host.answerTo(1)).result.content[0].text);
  assert.deepEqual(selection.values, [[1, 2], [3, 4]]);
  assert.equal(selection.address, 'Sheet1!A1:B2');
  host.request(2, 'tools/call', { name: 'excel_get_table', arguments: { name: 'Sales' } });
  const table = JSON.parse((await host.answerTo(2)).result.content[0].text);
  assert.deepEqual(table.header, [1, 2]);
  assert.deepEqual(table.rows, [[3, 4]]);
  assert.equal(asked.length, 0);
  host.request(3, 'tools/call', { name: 'excel_write_range', arguments: { address: 'Sheet1!A1:B2', values: [[9, 8], [7, 6]] } });
  assert.equal((await host.answerTo(3)).result.isError, false);
  assert.deepEqual(written, [['Sheet1!A1:B2', [[9, 8], [7, 6]]]]);
  assert.equal(asked.length, 1);
  assert.match(asked[0].summary, /2 rows of 2 to Sheet1!A1:B2/);
  host.request(4, 'tools/call', { name: 'excel_write_range', arguments: { address: 'Sheet1!A1:B2', values: [[1]] } });
  const mismatch = await host.answerTo(4);
  assert.equal(mismatch.result.isError, true);
  assert.equal(written.length, 1, 'a write that does not fit is not made');
  await pane.stop();
});
