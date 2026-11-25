// The `agent_vendor` kind: who an agent is and how Canoa shows it, so the host and the app hold no list of agents.
//
//   export class Quill extends AgentVendor {
//     static agent = 'quill';
//     static title = 'Quill';
//     static vendor = 'quill-labs';
//     static standards = [{ id: 'quill-key', title: 'Quill API key' }];
//   }
import { Provider } from 'guatiao/provide';

import * as kinds from './kinds.js';
import { dataOf } from './model.js';

/**
 * Subclass it and set the static fields below; the provider answers `describe` from them. Override `info()` to compute the
 * description instead. `agent` is the id of the agent's `acp_harness` provider when it has one, else its source's name in lower
 * case. `mark` is SVG text and `look` a JSON document, each at most 65536 characters; the host refuses a description that does not
 * fit and says why in its log.
 */
export class AgentVendor extends Provider {
  static kinds = [kinds.AgentVendor];

  /** @type {string} */ static agent;
  /** @type {string} */ static title;
  /** @type {string | undefined} */ static program;
  /** @type {string | undefined} */ static vendor;
  /** @type {string | undefined} */ static aos;
  /** @type {string | undefined} */ static model_variable;
  /** @type {string | undefined} */ static tool_name;
  /** @type {string | undefined} */ static skills_folder;
  /** @type {string[]} */ static context_files = [];
  /** @type {string | undefined} */ static home_variable;
  /** @type {string | undefined} */ static home_folder;
  /** @type {string[]} */ static home_context_files = [];
  /** @type {{id: string, title: string}[]} */ static standards = [];
  /** @type {string[]} */ static names = [];
  /** @type {string[]} */ static markers = [];
  /** @type {{id: string, title: string}[]} */ static modes = [];
  /** @type {string[]} */ static endpoints = [];
  /** @type {string | undefined} */ static mark;
  /** @type {string | undefined} */ static look;

  /** The description, from the static fields. */
  info() {
    const c = /** @type {typeof AgentVendor} */ (this.constructor);
    return {
      agent: c.agent,
      title: c.title,
      program: c.program,
      vendor: c.vendor,
      aos: c.aos,
      model_variable: c.model_variable,
      tool_name: c.tool_name,
      skills_folder: c.skills_folder,
      context_files: c.context_files,
      home_variable: c.home_variable,
      home_folder: c.home_folder,
      home_context_files: c.home_context_files,
      standards: c.standards,
      names: c.names,
      markers: c.markers,
      modes: c.modes,
      endpoints: c.endpoints,
      mark: c.mark,
      look: c.look,
    };
  }

  describe() {
    return dataOf(this.info());
  }
}
