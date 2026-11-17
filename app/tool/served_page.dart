// What the web app does with the host that served it, as a bare page for a browser test that cannot click through Flutter's canvas:
// the app's own code for folders, uploads, canoa.cfg, the overlay store, the session list and a session of the host's agent, run by
// the real browser's key proof against a real `canoa serve`. Built as a web target beside the app:
//   flutter build web --csp --no-web-resources-cdn -t tool/served_page.dart --output <dir>      (python tool/build_noise.py first)
// and loaded as <dir>/index.html, which runs this file's main.
import 'dart:convert';
import 'dart:js_interop';
import 'dart:js_interop_unsafe';

import 'package:canoa/acp/browser_connect.dart';
import 'package:canoa/acp/client.dart';
import 'package:canoa/acp/connect.dart';
import 'package:canoa/auth/browser_peer.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/profile/report.dart';
import 'package:canoa/served/served_app.dart';
import 'package:canoa/store/store.dart';
import 'package:canoa/store/store_index.dart';

Future<VerifiedAgent> _connect(ServedApp app, String entry, {Map<String, Object?> meta = const {}}) {
  final listed = app.host.entries.firstWhere((each) => each.name == entry);
  return connectBrowserAcp(
    chatEndpoints(webSocket: listed.url),
    pinnedPeerId: app.host.peer,
    method: app.host.method,
    initializeMeta: meta,
  );
}

Future<String> _said(VerifiedAgent client, String session, String text) async {
  final written = StringBuffer();
  final listening = client.updates.listen((update) {
    if (update.kind == 'agent_message_chunk') written.write(update.text ?? '');
  });
  await client.prompt(session, text);
  await Future<void>.delayed(const Duration(milliseconds: 150));
  await listening.cancel();
  return written.toString();
}

/// The page's step 1: what a web page does with its host. [options] holds `entry` (the agent entry), `catalog` (a git location
/// and `@ref#file` of a local catalog) and `overlay` (the catalog's entry to install).
Future<String> _first(String options) async {
  final asked = jsonDecode(options) as Map<String, Object?>;
  final report = <String, Object?>{};
  final app = (await ServedApp.discover(base: Uri.base.replace(path: '/', query: '', fragment: '')))!;
  try {
    await app.probe();
    report['reachable'] = app.reachable;
    report['problem'] = app.problem;
    report['sources'] = [for (final source in app.supervisor.listening) source.name];

    // A folder, chosen the way the dialog does: from the places, down to the project.
    final places = await app.files.listFolders();
    var at = places.entries.first.path;
    for (final name in const ['projects', 'app']) {
      at = (await app.files.listFolders(path: at)).entries.firstWhere((entry) => entry.name == name).path;
    }
    report['folder'] = at;

    final kept = await app.files.upload('notes.txt', utf8.encode('hello from the page'));
    report['upload'] = {'path': kept.path, 'size': kept.size};

    final client = await _connect(app, asked['entry']! as String);
    final opened = await client.openSession(cwd: at);
    report['answer'] = await _said(client, opened.id, 'hello');
    final options0 = (opened.raw['configOptions'] as List<dynamic>).cast<Map<String, dynamic>>();
    for (final id in ['model', 'effort']) {
      final option = options0.firstWhere((option) => option['id'] == id);
      final other = [for (final value in option['options'] as List<dynamic>) (value as Map<String, dynamic>)['value'] as String]
          .firstWhere((value) => value != option['currentValue']);
      final changed = await client.setConfigOption(opened.id, id, other);
      final now = (changed['configOptions'] as List<dynamic>).cast<Map<String, dynamic>>().firstWhere((option) => option['id'] == id);
      report[id] = now['currentValue'] == other ? other : 'unchanged';
    }

    final ledger = SessionLedger(app.ledger);
    await ledger.load();
    ledger.record(
      peerId: app.host.peer,
      sessionId: opened.id,
      entry: asked['entry']! as String,
      place: SessionPlace.host,
      cwd: at,
      title: 'Fix calc',
      endpoint: app.host.entries.firstWhere((each) => each.name == asked['entry']).url,
      method: app.host.method.id,
    );
    await ledger.flush();
    report['session'] = opened.id;
    await client.close();

    // The overlay: previewed from a local catalog, installed by the host, selected in canoa.cfg, and given to the next session.
    report['at'] = 'config';
    var current = (await app.loadConfig())!;
    // The host lists the catalogs canoa.cfg names, so the catalog is written there first.
    current = current.withCatalog('team', asked['catalog']! as String);
    await app.configStore.save(current);
    final listing = StoreListing(await ControlStoreIndex.over(app.control)!.catalogs());
    report['at'] = 'catalog';
    report['catalogs'] = [for (final loaded in listing.catalogs) {'label': loaded.label, 'problem': loaded.problem, 'entries': loaded.catalog?.entries.keys.toList()}];
    final entry = listing.choose('team/${asked['overlay']}').entry;
    report['at'] = 'preview';
    final preview = await previewOverlay(app.overlays, installName(entry), installSource(entry), os: 'linux');
    report['previewAdded'] = preview.update.added;
    report['at'] = 'install';
    current = await installPreview(app.overlays, current, preview, catalog: 'team');
    report['restart'] = await app.configStore.save(current);
    report['digest'] = preview.update.digest;
    report['at'] = 'session with the overlay';
    final withOverlay = await _connect(app, asked['entry']! as String, meta: {overlaysMeta: [asked['overlay']]});
    final second = await withOverlay.openSession(cwd: at);
    final profile = hostProfileOf((second.raw['_meta'] as Map<String, dynamic>?)?[hostProfileMeta], fallback: asked['entry']! as String);
    report['profileItems'] = [for (final item in profile?.report.items ?? const []) item.key];
    await withOverlay.close();
    report['ok'] = true;
  } on Object catch (error, stack) {
    report['error'] = '$error';
    report['stack'] = '$stack'.split('\n').take(6).join('\n');
  } finally {
    await app.close();
  }
  return jsonEncode(report);
}

/// The page's step 2, in a page loaded afterwards: the host's session list, canoa.cfg and overlays as that page reads them, and the
/// session reopened.
Future<String> _second(String options) async {
  final asked = jsonDecode(options) as Map<String, Object?>;
  final report = <String, Object?>{};
  final app = (await ServedApp.discover(base: Uri.base.replace(path: '/', query: '', fragment: '')))!;
  try {
    await app.probe();
    report['reachable'] = app.reachable;
    final ledger = SessionLedger(app.ledger);
    await ledger.load();
    report['entries'] = [for (final entry in ledger.entries) {'id': entry.sessionId, 'title': entry.title, 'cwd': entry.cwd}];
    final kept = ledger.find(app.host.peer, asked['session']! as String);
    report['found'] = kept != null;
    if (kept != null) {
      final client = await _connect(app, kept.entry);
      final reopened = await client.reopenSession(kept.sessionId, cwd: kept.cwd!);
      report['reopened'] = reopened.id;
      await client.close();
    }
    final config = (await app.loadConfig())!;
    report['selected'] = config.overlays.keys.toList();
    await app.refreshOverlays();
    report['installed'] = app.overlays.installed(asked['overlay']! as String)?.digest;
    report['ok'] = true;
  } on Object catch (error, stack) {
    report['error'] = '$error';
    report['stack'] = '$stack'.split('\n').take(6).join('\n');
  } finally {
    await app.close();
  }
  return jsonEncode(report);
}

Future<String> _ownId(String method) => localBrowserPeerId(PeerMethod.parse(method));

void main() {
  globalContext
    ..setProperty('canoaOwnId'.toJS, ((JSString method) => _ownId(method.toDart).then((id) => id.toJS).toJS).toJS)
    ..setProperty('canoaFirst'.toJS, ((JSString options) => _guard(_first(options.toDart)).then((answer) => answer.toJS).toJS).toJS)
    ..setProperty('canoaSecond'.toJS, ((JSString options) => _guard(_second(options.toDart)).then((answer) => answer.toJS).toJS).toJS)
    ..setProperty('canoaReady'.toJS, true.toJS);
}

Future<String> _guard(Future<String> step) => step.catchError((Object error) => jsonEncode({'error': '$error'}));
