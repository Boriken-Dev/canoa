// The `file_viewer` kind: a way to show a file of some type in the application's Files surface.
//
//   export class Stamp extends SurfaceViewer {
//     static viewer = 'stamp';
//     static title = 'Stamp';
//     static extensions = ['stamp'];
//     surface(file) {
//       return [{ version: '1.0', createSurface: { surfaceId: 's', components: [{ id: 'root', component: 'Text', text: file.text() }] } }];
//     }
//   }
//
// A viewer names the types it shows (`mimeTypes`, exact or by family like `image/*`, and `extensions` without the dot) and answers a
// file with either an A2UI surface (`shows = 'surface'`: the application draws it in its own widgets and the person's theme) or a
// self-contained page (`shows = 'page'`: one HTML document with its styles, scripts and pictures inline, which the host seals so it
// can make no request and the application opens in the person's browser). It is never handed a path: the host reads the file and
// gives the viewer its name, type, size and bytes (`ViewedFile`).
//
// The classes that mirror what Canoa carries are `SurfaceViewer` (answers `surface(file)`, a list of A2UI messages) and `PageViewer`
// (answers `page(file)`, an HTML document).
import { Provider } from 'guatiao/provide';

import * as kinds from './kinds.js';
import { dataOf } from './model.js';

/** The longest answer to `render` the host takes, in bytes. */
export const MAX_RENDER_BYTES = 4 * 1024 * 1024;

const ALPHABET = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';

/**
 * The bytes of standard base64 text, with no `Buffer` or `atob` (a script source has neither).
 * @param {string} text
 * @returns {Uint8Array}
 */
export function decodeBase64(text) {
  const clean = text.replace(/[=\s]/g, '');
  const out = new Uint8Array(Math.floor((clean.length * 3) / 4));
  let bits = 0;
  let held = 0;
  let at = 0;
  for (const letter of clean) {
    const value = ALPHABET.indexOf(letter);
    if (value < 0) throw new Error('the file is not base64');
    held = (held << 6) | value;
    bits += 6;
    if (bits >= 8) {
      bits -= 8;
      out[at++] = (held >> bits) & 0xff;
    }
  }
  return out.subarray(0, at);
}

/** The file a viewer is asked to show: what the host read of it, and no path. */
export class ViewedFile {
  /**
   * @param {{name?: string, size?: number | bigint, data?: string, mime?: string | null, truncated?: boolean, section?: string | null}} request
   */
  constructor(request) {
    /** @type {string} */ this.name = request?.name ?? '';
    /** @type {number} */ this.size = Number(request?.size ?? 0);
    /** @type {Uint8Array} what the host read of the file */ this.data = decodeBase64(request?.data ?? '');
    /** @type {string | undefined} */ this.mime = request?.mime ?? undefined;
    /** @type {boolean} */ this.truncated = Boolean(request?.truncated);
    /** @type {string | undefined} */ this.section = request?.section ?? undefined;
  }

  /** The bytes read as UTF-8, anything that is not replaced. */
  text() {
    return new TextDecoder('utf-8').decode(this.data);
  }
}

/**
 * Subclass it with `viewer` (1 to 32 lower-case letters, digits and `_`), `title`, `mimeTypes` and/or `extensions`, and `shows`
 * (`'surface'` or `'page'`), and answer `render`.
 */
export class FileViewer extends Provider {
  static kinds = [kinds.FileViewer];

  /** @type {string} */ static viewer;
  /** @type {string} */ static title;
  /** @type {string | undefined} a line saying what it shows */ static help;
  /** @type {string[]} exact (`application/json`) or by family (`image/*`) */ static mimeTypes = [];
  /** @type {string[]} without the dot */ static extensions = [];
  /** @type {number} which of two viewers that can show a file is tried first: the higher */ static priority = 0;
  /** @type {'surface' | 'page'} */ static shows = 'surface';
  /** @type {boolean} whether it opens a document at a section */ static opensSections = false;

  /** The description, from the static fields. */
  info() {
    const c = /** @type {typeof FileViewer} */ (this.constructor);
    return {
      id: c.viewer,
      title: c.title,
      help: c.help,
      mime_types: c.mimeTypes,
      extensions: c.extensions,
      priority: c.priority,
      shows: c.shows,
      opens_sections: c.opensSections,
    };
  }

  describe() {
    return dataOf(this.info());
  }

  /**
   * The file drawn: a JSON array of A2UI messages for `'surface'`, an HTML document for `'page'`.
   * @param {object} _request
   * @returns {string}
   */
  render(_request) {
    return '';
  }
}

/** A viewer that answers an A2UI surface: override `surface(file)` to answer the list of messages that creates it. */
export class SurfaceViewer extends FileViewer {
  /** @type {'surface' | 'page'} */ static shows = 'surface';

  /**
   * The A2UI messages that draw `file`: a `createSurface` whose components include one with the id `root`.
   * @param {ViewedFile} _file
   * @returns {object[]}
   */
  surface(_file) {
    return [];
  }

  /** @param {object} request */
  render(request) {
    return JSON.stringify(this.surface(new ViewedFile(request)));
  }
}

/** A viewer that answers a page: override `page(file)` to answer one self-contained HTML document. */
export class PageViewer extends FileViewer {
  /** @type {'surface' | 'page'} */ static shows = 'page';

  /**
   * The HTML that shows `file`, with its styles, scripts and pictures inline.
   * @param {ViewedFile} _file
   * @returns {string}
   */
  page(_file) {
    return '';
  }

  /** @param {object} request */
  render(request) {
    return this.page(new ViewedFile(request));
  }
}
