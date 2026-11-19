// What a panel tab describes about itself and what its log answers, from the SDK's classes alone: no host and no native library.
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { LogPanelTab, PagePanelTab, PanelTab, TerminalPanelTab } from '../src/panel.js';

test('a log answers the latest lines it was asked for', () => {
  class Builds extends LogPanelTab {
    static tab = 'builds';
    static title = 'Builds';
    lines() {
      return ['one', 'two', 'three'];
    }
  }
  assert.deepEqual(new Builds().describe(), { id: 'builds', title: 'Builds', shows: 'log' });
  assert.equal(new Builds().tail({ lines: 2 }), 'two\nthree\n');
  assert.equal(new Builds().tail({ lines: 0 }), '');
  assert.equal(new Builds().tail({ lines: 10 }), 'one\ntwo\nthree\n');
});

test('a terminal names its shell and shows no page', () => {
  class Shell extends TerminalPanelTab {
    static tab = 'dev';
    static title = 'Dev shell';
    static shell = 'git-bash';
  }
  assert.deepEqual(new Shell().describe(), { id: 'dev', title: 'Dev shell', shows: 'terminal', shell: 'git-bash' });
  assert.equal(new Shell().tail({ lines: 5 }), '');
});

test('a tab can show a page at an address', () => {
  class Docs extends PagePanelTab {
    static tab = 'docs';
    static title = 'Docs';
    static icon = 'book';
    static url = 'http://127.0.0.1:4100/docs';
  }
  assert.deepEqual(new Docs().describe(), {
    id: 'docs',
    title: 'Docs',
    icon: 'book',
    shows: 'page',
    url: 'http://127.0.0.1:4100/docs',
  });
  assert.equal(new Docs().tail({ lines: 5 }), '');

  class Plain extends PanelTab {
    static tab = 'plain';
    static title = 'Plain';
    static shows = 'page';
    static url = 'https://localhost/';
  }
  assert.equal(new Plain().describe().shows, 'page');
  assert.equal(new Plain().describe().url, 'https://localhost/');
});
