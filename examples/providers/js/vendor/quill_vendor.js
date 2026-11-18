// An agent vendor written in JavaScript: who the agent is and how Canoa shows it. Listing it in canoa.cfg (an entry's source) is all it
// takes for the host and the app to offer the agent, with no change to Canoa.
import { AgentVendor } from '../../../../bindings/js/canoa-sdk/src/vendor.js';

const MARK =
  "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24'>" +
  "<path d='M3 3h18L3 21h18' fill='none' stroke='#2563EB' stroke-width='2'/></svg>";

/** The vendor of an agent called Quill: the id its harness has, its name, the credential standards that are its own. */
export class Quill extends AgentVendor {
  static agent = 'quill';
  static title = 'Quill';
  static program = 'Quill Agent';
  static vendor = 'quill-labs';
  static aos = 'quill';
  static model_variable = 'QUILL_MODEL';
  static standards = [{ id: 'quill-key', title: 'Quill API key' }];
  static names = ['quill-agent'];
  static modes = [
    { id: 'ask', title: 'Ask' },
    { id: 'code', title: 'Code' },
  ];
  static mark = MARK;
}
