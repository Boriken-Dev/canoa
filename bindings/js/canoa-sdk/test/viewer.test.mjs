// What a file viewer describes about itself and what it answers for a file, from the SDK's classes alone: no host and no native library.
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { FileViewer, PageViewer, SurfaceViewer, ViewedFile } from '../src/viewer.js';

const request = (text) => ({ name: 'a.stamp', size: BigInt(text.length), data: Buffer.from(text).toString('base64'), truncated: false });

test('a viewer describes itself from its static fields and leaves out what it does not set', () => {
  class Stamp extends SurfaceViewer {
    static viewer = 'stamp';
    static title = 'Stamp';
    static extensions = ['stamp'];
  }
  assert.deepEqual(new Stamp().describe(), {
    id: 'stamp',
    title: 'Stamp',
    mime_types: [],
    extensions: ['stamp'],
    priority: 0,
    shows: 'surface',
    opens_sections: false,
  });

  class Plain extends FileViewer {
    static viewer = 'plain';
    static title = 'Plain';
    static mimeTypes = ['text/*'];
    static priority = 5;
    static shows = 'page';
    static opensSections = true;
    static help = 'Shows text';
  }
  const described = new Plain().describe();
  assert.equal(described.shows, 'page');
  assert.equal(described.priority, 5);
  assert.equal(described.help, 'Shows text');
  assert.equal(described.opens_sections, true);
  assert.equal(new Plain().render({}), '');
});

test('a surface viewer answers its messages as JSON, built from the file the host read and no path', () => {
  class Stamp extends SurfaceViewer {
    static viewer = 'stamp';
    static title = 'Stamp';
    static extensions = ['stamp'];
    surface(file) {
      return [{ version: '1.0', createSurface: { surfaceId: 's', components: [{ id: 'root', component: 'Text', text: `${file.name}: ${file.text()}` }] } }];
    }
  }
  const answer = JSON.parse(new Stamp().render(request('héllo')));
  assert.equal(answer[0].createSurface.components[0].text, 'a.stamp: héllo');
});

test('a page viewer answers its document', () => {
  class Page extends PageViewer {
    static viewer = 'page';
    static title = 'Page';
    static extensions = ['pg'];
    page(file) {
      return `<p>${file.size} bytes</p>`;
    }
  }
  assert.equal(new Page().render(request('abc')), '<p>3 bytes</p>');
  assert.equal(new Page().describe().shows, 'page');
});

test('a file carries its bytes, which standard base64 decodes whatever its padding', () => {
  for (const text of ['', 'a', 'ab', 'abc', 'abcd', '\u00e9\u4e2d\u{1f600}']) {
    const file = new ViewedFile({ name: 'x', size: 0, data: Buffer.from(text).toString('base64') });
    assert.equal(file.text(), text);
  }
  assert.throws(() => new ViewedFile({ data: '!!!' }), /base64/);
});
