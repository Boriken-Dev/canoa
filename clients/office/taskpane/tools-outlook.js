// The tools of the open Outlook item, through Office.js. `outlook_open_item` is the one that names it, by identifiers alone. `office` is Office.js's `Office` object, passed in so a test can give a
// stand-in. Office.js can read the item that is open and the attachments it lists; it cannot search the mailbox, and it exposes
// the conversation as an id only, so there is no tool for other messages of the thread.
//
// A reply is drafted by opening Outlook's own reply form with the text in it: the person reads it and sends it, or does not. No tool
// here sends anything.

const MAX_BODY = 200_000;
/** The longest identifier passed on: an Internet message id is at most 998 characters in a header, and the others are shorter. */
const MAX_ID = 998;
const MAX_ATTACHMENT = 3 * 1024 * 1024;

/** Office.js's callback style as a promise: `call(done)` where `done` receives an AsyncResult. */
function result(office, call) {
  return new Promise((resolve, reject) => {
    call((outcome) => {
      if (outcome.status === office.AsyncResultStatus.Succeeded) resolve(outcome.value);
      else reject(new Error(outcome.error?.message ?? 'Office.js failed'));
    });
  });
}

/** A property that is a value when reading and a getter when composing. */
async function property(office, value) {
  if (value && typeof value.getAsync === 'function') return result(office, (done) => value.getAsync(done));
  return value;
}

/** An identifier as text, or null when Outlook has none (a message being written has no Internet message id, and no item id until saved). */
const identifier = (value) => (typeof value === 'string' && value.trim() !== '' ? value.trim().slice(0, MAX_ID) : null);

const person = (value) => (value ? { name: value.displayName ?? '', address: value.emailAddress ?? '' } : null);
const people = (value) => (Array.isArray(value) ? value.map(person) : []);

/** @param {typeof Office} office */
export function outlookTools(office) {
  const item = () => {
    const open = office.context.mailbox.item;
    if (!open) throw new Error('No message is open in Outlook.');
    return open;
  };
  const composing = (open) => typeof open.subject?.getAsync === 'function';

  return [
    {
      name: 'outlook_open_item',
      description:
        'Which message is open in Outlook now, by identifiers only and never its text: its Internet message id (give it as message_id to mail_related or mail_read of the canoa-mail tools, which then find the same message in the person\'s mail and the mail related to it), its Exchange item id and its conversation id. A message being written has none yet. outlook_get_item reads the message.',
      inputSchema: { type: 'object', properties: {}, additionalProperties: false },
      async run() {
        const open = item();
        return {
          open: true,
          mode: composing(open) ? 'compose' : 'read',
          internetMessageId: identifier(open.internetMessageId),
          itemId: identifier(open.itemId),
          conversationId: identifier(open.conversationId),
        };
      },
    },
    {
      name: 'outlook_get_item',
      description:
        'The message open in Outlook now: subject, sender, recipients, dates, the body as text and the names of its attachments, with its conversation id. Outlook gives a task pane the open item only, not the rest of the mailbox or the other messages of the conversation.',
      inputSchema: { type: 'object', properties: {}, additionalProperties: false },
      async run() {
        const open = item();
        const compose = composing(open);
        const [subject, to, cc, from, bodyText] = await Promise.all([
          property(office, open.subject),
          property(office, open.to),
          property(office, open.cc),
          compose ? Promise.resolve(null) : Promise.resolve(open.from),
          result(office, (done) => open.body.getAsync(office.CoercionType.Text, done)),
        ]);
        let attachments = open.attachments;
        if (compose) attachments = typeof open.getAttachmentsAsync === 'function' ? await result(office, (done) => open.getAttachmentsAsync(done)) : [];
        return {
          mode: compose ? 'compose' : 'read',
          subject,
          from: person(from),
          to: people(to),
          cc: people(cc),
          conversationId: open.conversationId ?? null,
          internetMessageId: open.internetMessageId ?? null,
          received: open.dateTimeCreated ? new Date(open.dateTimeCreated).toISOString() : null,
          body: String(bodyText).slice(0, MAX_BODY),
          bodyTruncated: String(bodyText).length > MAX_BODY,
          attachments: (attachments ?? []).map((a) => ({ id: a.id, name: a.name, contentType: a.contentType, size: a.size, inline: a.isInline === true })),
        };
      },
    },
    {
      name: 'outlook_get_attachment',
      description: 'The content of one attachment of the open message, by the id outlook_get_item gave: text for a text file, else base64.',
      inputSchema: {
        type: 'object',
        properties: { id: { type: 'string', description: 'The attachment id from outlook_get_item' } },
        required: ['id'],
        additionalProperties: false,
      },
      async run({ id }) {
        const open = item();
        if (typeof id !== 'string' || id === '') throw new Error('Name the attachment by its id.');
        const known = (open.attachments ?? []).find((a) => a.id === id);
        if (known && typeof known.size === 'number' && known.size > MAX_ATTACHMENT) throw new Error('That attachment is larger than a tool answer carries.');
        const content = await result(office, (done) => open.getAttachmentContentAsync(id, done));
        const body = String(content.content ?? '');
        if (body.length > MAX_ATTACHMENT * 1.4) throw new Error('That attachment is larger than a tool answer carries.');
        return { name: known?.name ?? null, format: content.format, content: body };
      },
    },
    {
      name: 'outlook_draft_reply',
      description:
        'Opens Outlook\'s reply form for the open message with the text you give in it. The person reads it there and sends it or not: nothing is sent.',
      inputSchema: {
        type: 'object',
        properties: {
          body: { type: 'string', description: 'The reply, in plain text' },
          replyAll: { type: 'boolean', description: 'Reply to everyone on the message (default false)' },
        },
        required: ['body'],
        additionalProperties: false,
      },
      write: true,
      summary: ({ body, replyAll }) => `Open a ${replyAll ? 'reply-all' : 'reply'} form with this text (nothing is sent): ${String(body ?? '').slice(0, 160)}`,
      async run({ body, replyAll }) {
        const open = item();
        if (composing(open)) throw new Error('A message being written has no one to reply to.');
        if (typeof body !== 'string' || body === '') throw new Error('Give the text of the reply.');
        const html = `<div>${body.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/\n/g, '<br>')}</div>`;
        if (replyAll === true) open.displayReplyAllForm({ htmlBody: html });
        else open.displayReplyForm({ htmlBody: html });
        return { opened: true, sent: false };
      },
    },
  ];
}
