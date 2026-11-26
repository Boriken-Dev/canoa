/// The page that edits each table of canoa.cfg the app reads: a section of Settings by its id, or one of [pagesOutsideSettings]. A table
/// the app reads and no page edits has no entry here, and `config_pages_test.dart` fails for it.
const Map<String, String> configPages = {
  'desktop': 'general',
  'network': 'general',
  'appearance': 'appearance',
  'agents': 'sources',
  'host': 'hosting-panel',
  'sessions': 'general',
  'harnesses': 'advanced',
  'overlays': 'store',
  'profile': 'profiles',
  'permissions': 'permissions',
  'notifications': 'notifications',
  'store': 'store',
  'skill_pins': 'store',
  'loadouts': 'loadouts',
  'defaults': 'loadouts',
  'profiles': 'profiles',
  'sources': 'sources',
  'shortcuts': 'shortcuts',
  'attachments': 'general',
  'usage': 'usage',
  'voice': 'voice',
  'speech': 'voice',
  'connections': 'hosts',
};

/// The pages that are not a section of Settings: the Hosting panel of the desktop app, the Hosts list, and the Store, which edits its
/// own tables (overlays, store, skill pins) and the Loadouts.
const Set<String> pagesOutsideSettings = {'hosting-panel', 'hosts', 'loadouts', 'store'};
