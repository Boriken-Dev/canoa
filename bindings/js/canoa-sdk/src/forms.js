// A provider's settings as a guatiao schema and a guatiao-intake form. The schema is written with `s` from `guatiao/provide`; the form says
// how a person is shown it. There is no field type of Canoa's own: `ui()` places a field in a section, and `formMeta()` carries the
// form's sections, placeholders, units and conditions in the provider's `meta`.
//
//   import { s } from 'guatiao/provide';
//   class Whisper extends SpeechProvider {
//     static config = s.object({
//       command: s.array(s.string(), ui('run')),
//       beam: s.integer({ default: 5, ...ui('run') }),
//     });
//     static meta = formMeta({
//       sections: [section('run', 'Run')],
//       fields: { beam: { unit: 'beams', visibleWhen: when('translate', true) } },
//     });
//   }

/** The key a provider's form is kept under in its `meta`: guatiao-intake's `PROVIDER_FORM_KEY`. */
export const FORM_KEY = 'form';

/**
 * A section of the form.
 * @param {string} id
 * @param {string} title
 * @param {string} [description]
 * @returns {{id: string, title: string, description?: string}}
 */
export function section(id, title, description) {
  return description === undefined ? { id, title } : { id, title, description };
}

/**
 * The schema keyword that places a field in a section: pass it as the last argument of an `s` builder.
 * @param {string} sectionId
 * @returns {{'x-section': string}}
 */
export function ui(sectionId) {
  return { 'x-section': sectionId };
}

/**
 * A condition: the field is shown only while the field `name` holds `equals`.
 * @param {string} name
 * @param {unknown} equals
 * @returns {{field: string, equals: unknown}}
 */
export function when(name, equals) {
  return { field: name, equals };
}

/**
 * The `meta` entry carrying a form, for `static meta` of a provider class.
 * @param {{sections?: ReturnType<typeof section>[], fields?: Record<string, {placeholder?: string, unit?: string, visibleWhen?: ReturnType<typeof when>}>}} form
 * @returns {{form: object}}
 */
export function formMeta({ sections = [], fields = {} }) {
  const document = {};
  if (sections.length > 0) document.sections = sections;
  if (Object.keys(fields).length > 0) document.fields = fields;
  return { [FORM_KEY]: document };
}
