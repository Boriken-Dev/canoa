// The tools of the open Word document, through Office.js's Word API. `word` is Office.js's `Word` object, passed in so a test can give
// a stand-in. Reads answer at once; every change is asked in the pane first (`write: true`), by `tools.js`.

const MAX_TEXT = 500_000;
const MAX_COMMENTS = 500;

const LOCATIONS = {
  selection_start: ['selection', 'Start'],
  selection_end: ['selection', 'End'],
  document_start: ['body', 'Start'],
  document_end: ['body', 'End'],
};

const clip = (text) => String(text ?? '').slice(0, MAX_TEXT);

/** @param {typeof Word} word */
export function wordTools(word) {
  return [
    {
      name: 'word_get_text',
      description: 'The text of the open Word document.',
      inputSchema: { type: 'object', properties: {}, additionalProperties: false },
      run: () =>
        word.run(async (context) => {
          const body = context.document.body;
          body.load('text');
          await context.sync();
          return { text: clip(body.text), truncated: String(body.text ?? '').length > MAX_TEXT };
        }),
    },
    {
      name: 'word_get_selection',
      description: 'The text the person has selected in the open Word document (empty when nothing is selected).',
      inputSchema: { type: 'object', properties: {}, additionalProperties: false },
      run: () =>
        word.run(async (context) => {
          const selection = context.document.getSelection();
          selection.load('text');
          await context.sync();
          return { text: clip(selection.text), isEmpty: String(selection.text ?? '') === '' };
        }),
    },
    {
      name: 'word_insert_text',
      description: 'Inserts text into the open Word document at the start or end of the selection or of the document.',
      inputSchema: {
        type: 'object',
        properties: {
          text: { type: 'string' },
          location: { type: 'string', enum: Object.keys(LOCATIONS), description: 'Where to insert (default selection_end)' },
        },
        required: ['text'],
        additionalProperties: false,
      },
      write: true,
      summary: ({ text, location }) => `Insert at ${location ?? 'selection_end'}: ${String(text ?? '').slice(0, 160)}`,
      run: ({ text, location = 'selection_end' }) => {
        if (typeof text !== 'string' || text === '') throw new Error('Give the text to insert.');
        if (!Object.hasOwn(LOCATIONS, location)) throw new Error(`location is one of ${Object.keys(LOCATIONS).join(', ')}`);
        const [target, where] = LOCATIONS[location];
        return word.run(async (context) => {
          const range = target === 'body' ? context.document.body : context.document.getSelection();
          range.insertText(text, where);
          await context.sync();
          return { inserted: text.length };
        });
      },
    },
    {
      name: 'word_replace_selection',
      description: 'Replaces the text the person has selected in the open Word document.',
      inputSchema: {
        type: 'object',
        properties: { text: { type: 'string' } },
        required: ['text'],
        additionalProperties: false,
      },
      write: true,
      summary: ({ text }) => `Replace the selection with: ${String(text ?? '').slice(0, 160)}`,
      run: ({ text }) => {
        if (typeof text !== 'string') throw new Error('Give the replacement text.');
        return word.run(async (context) => {
          const selection = context.document.getSelection();
          selection.load('text');
          await context.sync();
          if (String(selection.text ?? '') === '') throw new Error('Nothing is selected to replace.');
          selection.insertText(text, 'Replace');
          await context.sync();
          return { replaced: true };
        });
      },
    },
    {
      name: 'word_list_comments',
      description: 'The comments of the open Word document: author, date, text and whether each is resolved.',
      inputSchema: { type: 'object', properties: {}, additionalProperties: false },
      run: () =>
        word.run(async (context) => {
          const comments = context.document.body.getComments();
          comments.load('items/id,items/authorName,items/creationDate,items/content,items/resolved');
          await context.sync();
          return {
            comments: comments.items.slice(0, MAX_COMMENTS).map((c) => ({
              id: c.id,
              author: c.authorName,
              date: c.creationDate instanceof Date ? c.creationDate.toISOString() : c.creationDate,
              text: c.content,
              resolved: c.resolved === true,
            })),
            truncated: comments.items.length > MAX_COMMENTS,
          };
        }),
    },
    {
      name: 'word_add_comment',
      description: 'Adds a comment on the text the person has selected in the open Word document.',
      inputSchema: {
        type: 'object',
        properties: { text: { type: 'string' } },
        required: ['text'],
        additionalProperties: false,
      },
      write: true,
      summary: ({ text }) => `Comment on the selection: ${String(text ?? '').slice(0, 160)}`,
      run: ({ text }) => {
        if (typeof text !== 'string' || text === '') throw new Error('Give the comment.');
        return word.run(async (context) => {
          const selection = context.document.getSelection();
          selection.load('text');
          await context.sync();
          if (String(selection.text ?? '') === '') throw new Error('Select the text to comment on first.');
          selection.insertComment(text);
          await context.sync();
          return { commented: true };
        });
      },
    },
  ];
}
