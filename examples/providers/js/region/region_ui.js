// What a team might add to Canoa's screens, written in JavaScript: a profile item that tells a session which region it runs in, a
// catalog source that lists a team's skills, a page of Settings and a tab of the bottom panel. One library serves four kinds; each
// class is a provider the host offers as it offers any extension's.
import { LogPanelTab } from '../../../../bindings/js/canoa-sdk/src/panel.js';
import { EnvProfileItem } from '../../../../bindings/js/canoa-sdk/src/profile.js';
import { SettingsSection } from '../../../../bindings/js/canoa-sdk/src/section.js';
import {
  CatalogSource,
  CommandCatalogSource,
  ReadCatalogSource,
  fieldsOf,
  release,
} from '../../../../bindings/js/canoa-sdk/src/source.js';

const REGION_VALUES = {
  type: 'object',
  additionalProperties: false,
  required: ['name'],
  properties: {
    name: { type: 'string', title: 'Name', 'x-section': 'where' },
    hosts: { type: 'array', items: { type: 'string' }, default: [], title: 'Other variables' },
  },
};

/** The region the session runs in, as the variable `REGION`; each entry of `hosts` is a `NAME=value` line. */
export class RegionItem extends EnvProfileItem {
  static item = 'region';
  static title = 'Region';
  static help = 'Where the session runs.';
  static harnesses = ['claude-code'];
  static values = REGION_VALUES;
  static form = { sections: [{ id: 'where', title: 'Where' }], fields: { name: { placeholder: 'eu-west' } } };
  static variables = { name: ['REGION'] };
  static lines = 'hosts';
  static note = 'puts the session in a region';
}

/** A team's skills: no program or file is read, so the spawner the host hands over is let go. */
export class TeamSource extends CatalogSource {
  static source = 'team';
  static title = "A team's list";
  static types = ['skills'];
  static values = {
    type: 'object',
    additionalProperties: false,
    required: ['team'],
    properties: { team: { type: 'string', title: 'Team name' } },
  };

  /**
   * @param {{kind: string, label: string, fields: string}} request
   * @param {import('../../../../bindings/js/canoa-sdk/src/model.js').Spawner} spawner
   */
  read(request, spawner) {
    release(spawner);
    const { team } = fieldsOf(request);
    return JSON.stringify({ skills: [{ name: `from-${team}`, description: 'Listed by a script.', source: 'owner/repo' }] });
  }
}

/** The SDK's command source under an id of its own: the program a catalog names runs through the host's spawner. */
export class EchoCommand extends CommandCatalogSource {
  static source = 'echo_command';
  static title = 'Echo a program';
}

/** The SDK's read source under an id of its own: the file a catalog names is read through the host's spawner. */
export class EchoRead extends ReadCatalogSource {
  static source = 'echo_read';
  static title = 'Echo a file';
}

/** A page of Settings whose values the host keeps in canoa.cfg. */
export class RegionSection extends SettingsSection {
  static section = 'region';
  static title = 'Region';
  static group = 'Team';
  static values = REGION_VALUES;
}

/** A tab of the bottom panel showing the last builds. */
export class BuildsTab extends LogPanelTab {
  static tab = 'builds';
  static title = 'Builds';

  lines() {
    return ['build 41 passed', 'build 42 failed'];
  }
}
