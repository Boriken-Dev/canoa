// The `panel_tab` kind: a kind of tab the application's bottom panel can open.
//
//   export class Builds extends LogPanelTab {
//     static tab = 'builds';
//     static title = 'Builds';
//     lines() { return ['build 41 passed', 'build 42 failed']; }
//   }
//
// A tab shows a shell on the host (`shows = 'terminal'`, optionally the one `shell` names) or a log, lines of text the provider keeps
// which the application asks for every couple of seconds while the tab is open (`shows = 'log'`, answered by `tail`).
//
// The classes that mirror what Canoa carries are `TerminalPanelTab` (a shell) and `LogPanelTab` (a log whose lines the subclass
// supplies).
import { Provider } from 'guatiao/provide';

import * as kinds from './kinds.js';
import { dataOf } from './model.js';

/** The most lines a `tail` answers. */
export const MAX_TAIL_LINES = 5000;

/**
 * Subclass it with `tab` (1 to 32 lower-case letters, digits and `_`), `title` and `shows`; a log answers `tail`.
 */
export class PanelTab extends Provider {
  static kinds = [kinds.PanelTab];

  /** @type {string} */ static tab;
  /** @type {string} */ static title;
  /** @type {string | undefined} a line saying what it is */ static help;
  /** @type {string | undefined} the name of one of the application's icons */ static icon;
  /** @type {'terminal' | 'log'} */ static shows = 'log';
  /** @type {string | undefined} for a terminal: the id of the shell the host lists that a new tab runs */ static shell;

  /** The description, from the static fields. */
  info() {
    const c = /** @type {typeof PanelTab} */ (this.constructor);
    return { id: c.tab, title: c.title, help: c.help, icon: c.icon, shows: c.shows, shell: c.shell };
  }

  describe() {
    return dataOf(this.info());
  }

  /**
   * The latest lines of the log, oldest first, each ended by a newline; empty for a kind that shows no log.
   * @param {{lines: number}} _request at most `MAX_TAIL_LINES`
   * @returns {string}
   */
  tail(_request) {
    return '';
  }
}

/** A shell on the host: the one `shell` names, or the host's own choice. */
export class TerminalPanelTab extends PanelTab {
  static shows = 'terminal';
}

/** A log: override `lines()` to answer every line the log holds so far, oldest first; `tail` answers the latest of them. */
export class LogPanelTab extends PanelTab {
  static shows = 'log';

  /**
   * Every line the log holds, oldest first, with no line break.
   * @returns {string[]}
   */
  lines() {
    return [];
  }

  /** @param {{lines: number}} request */
  tail(request) {
    const wanted = Math.min(Math.max(Math.trunc(Number(request?.lines) || 0), 0), MAX_TAIL_LINES);
    const lines = this.lines();
    const latest = wanted === 0 ? [] : lines.slice(-wanted);
    return latest.map((line) => `${line}\n`).join('');
  }
}
