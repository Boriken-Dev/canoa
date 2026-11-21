// The `settings_section` kind: a page, or a group on a page, of the application's Settings.
//
//   export class Region extends SettingsSection {
//     static section = 'region';
//     static title = 'Region';
//     static values = { type: 'object', properties: { name: { type: 'string', title: 'Name' } } };
//   }
//
// With no `under` the section is a page of its own in Settings' list under the heading `group` ('Extensions' when none); with `under`
// naming one of the application's sections (or another provider's) it is a titled group inside that page. Its values are kept in
// `[settings.ID]` of canoa.cfg, written by the host after it checked them against `values` (`stored = 'config'`, the default), or
// by the provider (`stored = 'provider'`): the host then calls `read` to show them and `write` with the checked values to keep them.
import { Provider } from 'guatiao/provide';

import * as kinds from './kinds.js';
import { dataOf } from './model.js';

/** The most bytes of values a section keeps. */
export const MAX_VALUES = 64 * 1024;

/**
 * Subclass it with `section` (1 to 32 lower-case letters, digits and `_`), `title` and, for a section that asks for something, `values`
 * (a guatiao schema of an object, as plain data) and `form` (a guatiao-intake form document). A member the schema marks `x-sensitive`
 * or `writeOnly` is never answered in clear: `read` leaves it out and the host names it as set.
 */
export class SettingsSection extends Provider {
  static kinds = [kinds.SettingsSection];

  /** @type {string} */ static section;
  /** @type {string} */ static title;
  /** @type {string | undefined} a line shown above its form */ static help;
  /** @type {string | undefined} the name of one of the application's icons */ static icon;
  /** @type {string | undefined} the heading it sits under in the list, for a section of its own */ static group;
  /** @type {string | undefined} the id of the section this is a group in */ static under;
  /** @type {string | undefined} words the search over Settings finds it by */ static keywords;
  /** @type {'config' | 'provider'} where its values are kept */ static stored = 'config';
  /** @type {object | undefined} a guatiao schema of an object */ static values;
  /** @type {object | undefined} a guatiao-intake form document */ static form;

  /** The description, from the static fields. */
  info() {
    const c = /** @type {typeof SettingsSection} */ (this.constructor);
    return {
      id: c.section,
      title: c.title,
      help: c.help,
      icon: c.icon,
      group: c.group,
      under: c.under,
      keywords: c.keywords,
      stored: c.stored,
      schema: c.values,
      form: c.form,
    };
  }

  describe() {
    return dataOf(this.info());
  }

  /**
   * What the section holds now, as an object keyed by the schema's member names, a sensitive member left out. Asked only of a
   * `stored = 'provider'` section.
   * @returns {Record<string, unknown>}
   */
  read() {
    return {};
  }

  /**
   * Keeps `values`, an object the host has already checked against the schema; a sensitive member the person left empty is absent,
   * so what was kept stays. Asked only of a `stored = 'provider'` section.
   * @param {Record<string, unknown>} _values
   */
  write(_values) {}
}
