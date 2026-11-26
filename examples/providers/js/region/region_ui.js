// What a team might add to Canoa's screens, written in JavaScript: a profile item that tells a session which region it runs in, a
// catalog source that lists a team's skills, a URI scheme of the team's own and a generator that reads it, a page of Settings, a tab
// of the bottom panel, a place notices are sent to and a viewer of the Files surface. One library serves eight kinds; each class is a
// provider the host offers as it offers any extension's.
import { Notifier, delivery, noticeOf } from '../../../../bindings/js/canoa-sdk/src/notifier.js';
import {
  CatalogGenerator,
  ContentSource,
  openedInline,
  scheme,
  statFile,
} from '../../../../bindings/js/canoa-sdk/src/content.js';
import { LogPanelTab, PagePanelTab } from '../../../../bindings/js/canoa-sdk/src/panel.js';
import { EnvProfileItem } from '../../../../bindings/js/canoa-sdk/src/profile.js';
import { SettingsSection } from '../../../../bindings/js/canoa-sdk/src/section.js';
import { SurfaceViewer } from '../../../../bindings/js/canoa-sdk/src/viewer.js';
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

/** A team's skills: no program or file is read, so the spawner and the fetch the host hands over are let go. */
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
   * @param {{destroy(): void}} fetch
   */
  read(request, spawner, fetch) {
    release(spawner);
    release(fetch);
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

/** A URI scheme of a team's own: `team://NAME/skills.json` is a list of that team's skills, made up here instead of fetched. */
export class TeamFiles extends ContentSource {
  static source = 'team_files';
  static title = 'Team files';
  static schemes = [scheme('team', "A team's files", 'team://name/path')];

  /** @param {{target: {uri: string}}} request */
  open(request) {
    const team = request.target.uri.slice('team://'.length).split('/')[0];
    const listing = { skills: [{ name: `from-${team}`, description: 'Read through a scheme a script added.', source: 'owner/repo' }] };
    return openedInline(statFile(), JSON.stringify(listing));
  }
}

/** A generator that makes a skills catalog from what its source holds: it reads `<source>/skills.json` through the host. */
export class TeamListing extends CatalogGenerator {
  static generator = 'team_listing';
  static title = "A team's listing";
  static types = ['skills'];
  static grammar = 'team://name';

  /**
   * @param {{kind: string, label: string, source?: string, fields: string}} request
   * @param {{destroy(): void}} spawner
   * @param {import('../../../../bindings/js/canoa-sdk/src/content.js').ContentHostHandle} host
   * @returns {string}
   */
  generate(request, spawner, host) {
    try {
      return host.openText(host.child(request.source ?? '', 'skills.json'), 1048576n);
    } finally {
      release(spawner);
      release(host);
    }
  }
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

/** A tab of the bottom panel that names a page this computer serves and opens it in the browser. */
export class DocsTab extends PagePanelTab {
  static tab = 'docs';
  static title = 'Team docs';
  static url = 'http://127.0.0.1:4100/docs';
}

/** Somewhere a notice could be sent: it keeps the notice's id and says it was delivered. */
export class TeamChat extends Notifier {
  static notifier = 'teamchat';
  static title = 'Team chat';
  static help = "Sends each notice to the team's chat.";
  static events = ['finished', 'failed'];

  /** @param {unknown} request */
  send(request) {
    return delivery(noticeOf(request).id);
  }
}

/**
 * Shows a `.stamp` file as one line of text in the application's own widgets: the host reads the file and hands over its name and bytes,
 * so the viewer never sees where the file is.
 */
export class StampViewer extends SurfaceViewer {
  static viewer = 'stamp';
  static title = 'Stamp';
  static extensions = ['stamp'];
  static priority = 20;

  surface(file) {
    const text = `${file.name}: ${file.text().trim()}`;
    return [{ version: '1.0', createSurface: { surfaceId: 'stamp', components: [{ id: 'root', component: 'Text', text }] } }];
  }
}
