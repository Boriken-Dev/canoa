// The panel for the page in front: a session started from a webpage works in a scratch folder of that site, the panel lists the
// site's earlier sessions to resume or start another beside, and nothing of another site reaches it.
import 'dart:async';
import 'dart:convert';

import 'package:canoa/client/client_mode.dart';
import 'package:canoa/client/pane_link.dart';
import 'package:canoa/client/relation.dart';
import 'package:canoa/client/relation_source.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/extension/tab_bridge.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/main.dart';
import 'package:canoa/profile/report.dart' show clientMeta, loadoutMeta, presetMeta;
import 'package:canoa/session_store.dart';
import 'package:canoa/workspace/site_meta.dart';
import 'package:coqui_acp/page.dart';
import 'package:canoa/client/related_rail.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart' show LogicalKeyboardKey;
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart' show settle, until;
import 'fixtures/fake_tab_bridge.dart';
import 'fixtures/sample_render.dart' show samplesSkip;

final _peer = 'sha256:${'ab' * 32}';
// What the agent's host puts the folders under; the panel does not know it.
const _documents = '/home/ana/Documents';
final _id = RegExp(r'[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}');

/// The page that frames the app, as a task pane's: it says what is in front when it learns it, and notes that the app said it listens.
final class _Pane implements PaneLink {
  final _said = StreamController<Relation?>.broadcast();
  var readied = 0;
  var signedOut = 0;
  var disposed = false;

  void say(Relation? relation) => _said.add(relation);

  @override
  Stream<Relation?> get relations => _said.stream;
  @override
  void ready() => readied++;
  @override
  void signIn() => signedOut++;
  @override
  void dispose() {
    disposed = true;
    unawaited(_said.close());
  }
}

final class _Agent implements AcpPageAgent {
  @override
  String get id => 'agent-1';
  @override
  String get label => 'Desk';
  @override
  String? get peer => _peer;
  @override
  bool get verified => true;
}

/// A browser provider with one granted agent, which opens a session of its own for each `session/new` and notes what it was
/// asked. It stands for the host: a request that names a site gets a folder under its Documents, named in the answer, as the
/// real host does.
final class _Provider implements AcpPage {
  final agent = _Agent();

  /// The `cwd` and the site of each `session/new`, as the panel sent them.
  final asked = <({String cwd, String? site})>[];

  /// The folders the host made.
  final newFolders = <String>[];
  final loads = <({String id, String cwd, String? site})>[];

  /// Everything each `session/new` carried, as the agent's host read it.
  final sent = <String>[];

  /// The `_meta` each `initialize` carried, where the host reads the loadout and profile a session starts with.
  final initialized = <Map<String, Object?>>[];

  /// Whether the host leaves its folder out of its answer, as one that does not know the extension would.
  var forgets = false;
  var requests = 0;
  var _next = 0;

  @override
  Future<List<AcpPageAgent>> agents() async => [agent];

  @override
  Future<List<AcpPageAgent>> request({String? purpose, List<String> peers = const [], List<String> mcpOrigins = const [], bool permissions = false}) async {
    requests++;
    return [agent];
  }

  @override
  Future<AcpPageConnection> connect(AcpPageAgent agent) async => _Connection(this);

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

final class _Connection implements AcpPageConnection {
  _Connection(this.owner);
  final _Provider owner;
  final _incoming = StreamController<String>();

  @override
  AcpPageAgent get agent => owner.agent;
  @override
  Stream<String> get messages => _incoming.stream;
  @override
  Future<void> close() async {
    if (!_closed.isCompleted) _closed.complete('closed');
    await _incoming.close();
  }
  @override
  Future<String> get closed => _closed.future;
  final _closed = Completer<String>();

  @override
  Future<void> send(String message) async {
    final request = jsonDecode(message) as Map<String, dynamic>;
    final id = request['id'];
    if (id == null || request['method'] == null) return;
    final params = request['params'] as Map<String, dynamic>? ?? const {};
    Object? result = <String, Object?>{};
    switch (request['method']) {
      case 'initialize':
        owner.initialized.add((params['_meta'] as Map?)?.cast<String, Object?>() ?? const {});
        result = {'protocolVersion': 1, 'agentCapabilities': {'loadSession': true}, 'agentInfo': {'name': 'site-agent', 'version': '1.0.0'}};
      case 'session/new':
        final site = ((params['_meta'] as Map<String, dynamic>?)?[siteMeta] as Map<String, dynamic>?)?['site'] as String?;
        owner.asked.add((cwd: params['cwd'] as String, site: site));
        owner.sent.add(jsonEncode(params));
        final number = ++owner._next;
        final session = 'p$number';
        result = {'sessionId': session};
        if (site != null && !owner.forgets) {
          final made = '00000000-0000-4000-8000-${number.toString().padLeft(12, '0')}';
          final folder = '$_documents/Canoa/scratch/$site/$made';
          owner.newFolders.add(folder);
          result = {'sessionId': session, '_meta': {siteMeta: {'site': site, 'id': made, 'folder': folder}}};
        }
      case 'session/load':
        final site = ((params['_meta'] as Map<String, dynamic>?)?[siteMeta] as Map<String, dynamic>?)?['site'] as String?;
        owner.loads.add((id: params['sessionId'] as String, cwd: params['cwd'] as String, site: site));
    }
    _incoming.add(jsonEncode({'jsonrpc': '2.0', 'id': id, 'result': result}));
  }
}

FakeTabBridge _bridge(String url, {String title = 'Example'}) => FakeTabBridge(front: 1)..open(OpenTab(id: 1, title: title, url: url));

Future<SessionStore> _panel(
  WidgetTester tester, {
  required SessionLedger ledger,
  FakeTabBridge? bridge,
  required _Provider provider,
  String? workspace,
  CanoaConfig? config,
  ClientMode? mode,
  RelationSource? relations,
  bool Function(Uri)? openPage,
  PaneLink? paneLink,
}) async {
  final store = SessionStore(ledger: ledger);
  addTearDown(store.dispose);
  await tester.binding.setSurfaceSize(const Size(380, 800));
  addTearDown(() => tester.binding.setSurfaceSize(null));
  await tester.pumpWidget(CanoaApp(
    store: store,
    config: config ?? (workspace == null ? const CanoaConfig() : CanoaConfig(sessions: LedgerSettings(workspace: workspace))),
    tabBridge: bridge,
    browserProvider: Future.value(provider),
    mode: mode,
    relations: relations,
    openPage: openPage,
    paneLink: paneLink,
  ));
  await settle(tester);
  return store;
}

Future<void> _tap(WidgetTester tester, Key key) async {
  await tester.tap(find.byKey(key));
  await settle(tester);
}

Finder _tiles() => find.byWidgetPredicate((widget) => widget.key is ValueKey<String> && (widget.key! as ValueKey<String>).value.startsWith('related-session '));

void main() {
  chatOnlyTests();

  test('the app marks its sessions as its own person\'s, not a webpage\'s', () {
    expect(siteSetup('example.com').meta, {siteMeta: {'site': 'example.com', 'own': true}});
  });

  testWidgets('a session from https://example.com/a works in scratch/example.com/<id>, which the ledger notes with its site', (tester) async {
    final ledger = SessionLedger(MemoryLedgerBackend());
    final provider = _Provider();
    final store = await _panel(tester, ledger: ledger, bridge: _bridge('https://example.com/a'), provider: provider);
    expect(find.byKey(const Key('related-name')), findsOneWidget);
    expect(tester.widget<Text>(find.byKey(const Key('related-name'))).data, 'example.com');

    await _tap(tester, const Key('related-new'));
    await until(tester, () => store.sessions.isNotEmpty && store.sessions.first.remoteSessionId != null, 'the session to open');
    final folder = provider.newFolders.single;
    expect(provider.asked.single, (cwd: siteStandInFolder, site: 'example.com'), reason: 'the page names the site, not a path');
    expect(folder, matches(RegExp('^$_documents/Canoa/scratch/example\\.com/${_id.pattern}\$')));
    final session = store.sessions.single;
    expect(session.cwd, folder, reason: 'the folder the host said it made');
    expect(session.site, 'example.com');
    final entry = ledger.entries.single;
    expect(entry.site, 'example.com');
    expect(entry.cwd, folder);
    expect(entry.canoaId, folder.split('/').last, reason: 'the host\'s id for the session, which names its folder');
    expect(LedgerEntry.fromJson(entry.toJson())!.site, 'example.com');
    // What the session relates to is the page's origin, with the page's title; it is the ledger's and the agent was told none of it.
    expect(entry.relation, isNotNull);
    expect(entry.relation!.kind, Relation.originKind);
    expect(entry.relation!.key, 'https://example.com');
    expect(entry.relation!.label, 'Example');
    expect(LedgerEntry.fromJson(entry.toJson())!.relation, entry.relation);
    expect(store.sessions.single.relation, entry.relation);
    expect(provider.sent.single, isNot(contains('https://example.com')), reason: 'the origin stays with the app');
    expect(provider.sent.single, isNot(contains('Example')), reason: 'so does the page title');
  });

  testWidgets('reopening the panel on example.com lists the session and resumes it in its own folder', (tester) async {
    final ledger = SessionLedger(MemoryLedgerBackend());
    final first = _Provider();
    final opened = await _panel(tester, ledger: ledger, bridge: _bridge('https://example.com/a'), provider: first);
    await _tap(tester, const Key('related-new'));
    await until(tester, () => opened.sessions.isNotEmpty && opened.sessions.first.remoteSessionId != null, 'the session to open');
    final folder = first.newFolders.single;
    final key = ledger.entries.single.key;
    await tester.pumpWidget(const SizedBox());

    // The panel is closed and opened again on the same site: a new store over the same list.
    final provider = _Provider();
    final store = await _panel(tester, ledger: ledger, bridge: _bridge('https://example.com/other?q=1#top'), provider: provider);
    expect(_tiles(), findsOneWidget);
    expect(find.text('Earlier on example.com'), findsOneWidget);
    expect(find.descendant(of: _tiles(), matching: find.textContaining(folder)), findsOneWidget);

    await _tap(tester, ValueKey('related-resume $key'));
    await until(tester, () => store.sessions.isNotEmpty && store.sessions.first.status == SessionStatus.connected, 'the session to open again');
    expect(provider.loads, [(id: 'p1', cwd: folder, site: 'example.com')], reason: 'reopened in the folder it was made in, naming the site');
    expect(provider.asked, isEmpty);
    final session = store.sessions.single;
    expect(session.cwd, folder);
    expect(session.site, 'example.com');
    expect(ledger.entries, hasLength(1), reason: 'the same record, not a second one');
  });

  testWidgets('another site lists none of it, and neither does a parent or a child domain', (tester) async {
    final ledger = SessionLedger(MemoryLedgerBackend());
    final provider = _Provider();
    final opened = await _panel(tester, ledger: ledger, bridge: _bridge('https://example.com/a'), provider: provider);
    await _tap(tester, const Key('related-new'));
    await until(tester, () => opened.sessions.isNotEmpty && opened.sessions.first.remoteSessionId != null, 'the session to open');
    expect(ledger.entries.single.site, 'example.com');
    await tester.pumpWidget(const SizedBox());

    for (final url in [
      'https://example.org/', 'https://www.example.com/', 'https://com/', 'http://example.com.evil.test/',
      // Another port or another scheme is another origin.
      'https://example.com:8443/', 'http://example.com/',
    ]) {
      await _panel(tester, ledger: ledger, bridge: _bridge(url), provider: _Provider());
      expect(find.byKey(const Key('related-name')), findsOneWidget, reason: url);
      expect(_tiles(), findsNothing, reason: url);
      expect(find.textContaining('Earlier on'), findsNothing, reason: url);
      expect(find.textContaining('example.com/Canoa'), findsNothing, reason: url);
      expect(find.textContaining('/example.com/'), findsNothing, reason: 'nothing of example.com in $url\'s panel');
      await tester.pumpWidget(const SizedBox());
    }
    expect(ledger.forRelation(originRelation('https://example.org/')!), isEmpty);
  });

  testWidgets('a new session from the same site gets a folder of its own, and the site lists both newest first', (tester) async {
    final ledger = SessionLedger(MemoryLedgerBackend(), clock: _Clock().now);
    final provider = _Provider();
    final store = await _panel(tester, ledger: ledger, bridge: _bridge('https://example.com/a'), provider: provider);
    await _tap(tester, const Key('related-new'));
    await until(tester, () => store.sessions.length == 1 && store.sessions.first.remoteSessionId != null, 'the first session');
    await tester.runAsync(() => store.close(store.sessions.single));
    await settle(tester);
    await _tap(tester, const Key('related-new'));
    await until(tester, () => store.sessions.length == 1 && store.sessions.first.remoteSessionId != null, 'the second session');
    expect(provider.newFolders, hasLength(2));
    expect(provider.newFolders.first, isNot(provider.newFolders.last));
    for (final folder in provider.newFolders) {
      expect(folder, matches(RegExp('^$_documents/Canoa/scratch/example\\.com/${_id.pattern}\$')));
    }
    expect({for (final folder in provider.newFolders) folder.substring(0, folder.lastIndexOf('/'))}, hasLength(1), reason: 'one folder for the site');
    final listed = ledger.forRelation(originRelation('https://example.com/')!);
    expect(listed.map((entry) => entry.cwd), [provider.newFolders.last, provider.newFolders.first], reason: 'newest first');
  });

  testWidgets('a page that is not a site with a safe name shows no list', (tester) async {
    for (final url in [
      'chrome://extensions/', 'file:///C:/Work/a.html', 'about:blank', 'https://[::1]/', 'https://con/', 'https://a%2f..%2fb/',
      'javascript:alert(1)', 'https://c:80:90/',
    ]) {
      await _panel(tester, ledger: SessionLedger(MemoryLedgerBackend()), bridge: _bridge(url), provider: _Provider());
      expect(find.byKey(const Key('related-name')), findsNothing, reason: url);
      await tester.pumpWidget(const SizedBox());
    }
  });

  testWidgets('the panel never asks for the Documents folder, and passes no path whatever canoa.cfg says', (tester) async {
    final provider = _Provider();
    final store = await _panel(
      tester, ledger: SessionLedger(MemoryLedgerBackend()), bridge: _bridge('https://Example.COM/a'), provider: provider,
      workspace: r'${documents}/Work/${fqdn}/chat-${session}',
    );
    expect(find.byKey(const Key('site-documents')), findsNothing);
    expect(find.textContaining('Documents'), findsNothing);
    expect(tester.widget<FilledButton>(find.byKey(const Key('related-new'))).onPressed, isNotNull);
    await _tap(tester, const Key('related-new'));
    await until(tester, () => store.sessions.isNotEmpty && store.sessions.first.remoteSessionId != null, 'the session to open');
    expect(provider.asked.single, (cwd: siteStandInFolder, site: 'example.com'));
    expect(provider.newFolders.single, startsWith('$_documents/Canoa/scratch/example.com/'), reason: 'the host places it');
  });

  testWidgets('a host that does not say which folder it made fails the session, and nothing is kept', (tester) async {
    final ledger = SessionLedger(MemoryLedgerBackend());
    final provider = _Provider()..forgets = true;
    final store = await _panel(tester, ledger: ledger, bridge: _bridge('https://example.com/'), provider: provider);
    await _tap(tester, const Key('related-new'));
    await until(tester, () => find.byKey(const Key('related-failed')).evaluate().isNotEmpty, 'the failure');
    expect(find.textContaining('did not say which workspace'), findsOneWidget);
    expect(store.sessions, isEmpty);
    expect(ledger.entries, isEmpty);
  });

  testWidgets('sample: the side panel for a site lists its earlier sessions (CANOA_SAMPLES_DIR)', (tester) async {
    final ledger = SessionLedger(MemoryLedgerBackend(), clock: _Clock().now);
    final provider = _Provider();
    final opened = await _panel(tester, ledger: ledger, bridge: _bridge('https://example.com/docs'), provider: provider);
    for (var count = 1; count <= 2; count++) {
      await _tap(tester, const Key('related-new'));
      await until(tester, () => opened.sessions.length == 1 && opened.sessions.first.remoteSessionId != null, 'session $count');
      ledger.touch(_peer, 'p$count', title: count == 1 ? 'Summarise the changelog' : 'Fix the broken link');
      await tester.runAsync(() => opened.close(opened.sessions.single));
      await settle(tester);
    }
    await tester.pumpWidget(const SizedBox());
    await _panel(tester, ledger: ledger, bridge: _bridge('https://example.com/docs'), provider: _Provider());
    await expectLater(find.byType(MaterialApp), matchesGoldenFile('ext_panel_site_sessions.png'));
  }, skip: samplesSkip);
}

const _chatOnly = ClientMode(chatOnly: true);

/// A canoa.cfg with loadouts and profiles, a default loadout and a default of its own for each of three kinds of client.
final _withLoadouts = CanoaConfig.parse(
  '[loadouts.writing]\n[loadouts.review]\nskills = ["audit"]\n[loadouts.mail]\n[loadouts.docs]\n'
  '[profiles.deep]\nmodel = "big"\nloadout = "review"\n[profiles.careful]\nmodel = "small"\n'
  '[defaults]\nloadout = "writing"\n[defaults.clients]\noutlook = "mail"\nword = "docs"\n',
);

/// Opens the menu of [pill] and chooses [item], then closes the menu.
Future<void> _pick(WidgetTester tester, Key pill, Key item) async {
  await tester.tap(find.byKey(pill));
  await settle(tester);
  await tester.tap(find.byKey(item));
  await settle(tester);
  await tester.tapAt(const Offset(370, 700));
  await settle(tester);
}

Future<void> _openDrawer(WidgetTester tester) async {
  await tester.tap(find.byTooltip('Sessions'));
  await settle(tester);
}

/// Starts a session on the front page and names it.
Future<ChatSession> _start(WidgetTester tester, SessionStore store, String name) async {
  final before = store.sessions.length;
  // With a session open the welcome page, which lists the page's sessions, is not shown: its list is in the drawer.
  if (find.byKey(const Key('related-new')).evaluate().isEmpty) await _openDrawer(tester);
  await _tap(tester, const Key('related-new'));
  await until(tester, () => store.sessions.length == before + 1 && store.sessions.first.remoteSessionId != null, 'session $name');
  final session = store.sessions.first;
  store.rename(session, name);
  await settle(tester);
  return session;
}

void chatOnlyTests() {
  group('the chat-only app', () {
    testWidgets('shows the conversation and the sessions of the page, with none of the whole app\'s sections', (tester) async {
      final provider = _Provider();
      final store = await _panel(
        tester, ledger: SessionLedger(MemoryLedgerBackend()), bridge: _bridge('https://example.com/a'), provider: provider, mode: _chatOnly,
      );
      expect(find.byKey(const Key('related-name')), findsOneWidget);
      await _openDrawer(tester);
      expect(find.descendant(of: find.byType(Drawer), matching: find.byType(RelatedRail)), findsOneWidget);
      for (final section in ['Conversations', 'Hosts', 'Terminal', 'Loadouts', 'Store', 'Canoa settings']) {
        expect(find.byTooltip(section), findsNothing, reason: 'no $section section');
      }
      await tester.tapAt(const Offset(370, 400));
      await settle(tester);
      await _start(tester, store, 'About the page');
      // The header's actions are under one menu on a narrow client: the whole app is one of them, and the workspace is none.
      await tester.tap(find.byTooltip('More'));
      await settle(tester);
      expect(find.text('Open the full app'), findsOneWidget, reason: 'the header links to the whole app');
      expect(find.text('Session details'), findsNothing);
      expect(find.text('Open the workspace'), findsNothing);
    });

    testWidgets('the whole app is unchanged where nothing asks for the chat alone', (tester) async {
      await _panel(tester, ledger: SessionLedger(MemoryLedgerBackend()), bridge: _bridge('https://example.com/a'), provider: _Provider());
      await _openDrawer(tester);
      expect(find.byTooltip('Canoa settings'), findsOneWidget);
      expect(find.byTooltip('Store'), findsOneWidget);
      expect(find.byType(RelatedRail), findsNothing);
    });

    testWidgets('a session started on a page is listed first on that page\'s origin, then the others', (tester) async {
      final ledger = SessionLedger(MemoryLedgerBackend(), clock: _Clock().now);
      final bridge = _bridge('https://example.com/a', title: 'Docs');
      final store = await _panel(tester, ledger: ledger, bridge: bridge, provider: _Provider(), mode: _chatOnly);
      final onExample = await _start(tester, store, 'About example');
      bridge.open(const OpenTab(id: 2, title: 'Other', url: 'https://other.test/x'));
      bridge.bring(2);
      await settle(tester);
      await _openDrawer(tester);
      expect(tester.widget<Text>(find.byKey(const Key('related-name'))).data, 'other.test');
      final onOther = await _start(tester, store, 'About other');

      await _openDrawer(tester);
      // On other.test: its own session is under the page, the one from example.com is among the others.
      expect(find.byKey(ValueKey('related-open ${onOther.id}')), findsOneWidget);
      expect(find.byKey(ValueKey('related-open ${onExample.id}')), findsNothing);
      expect(find.byKey(ValueKey('rail-open ${onExample.id}')), findsOneWidget);
      expect(find.text('Other conversations'), findsOneWidget);
      final related = tester.getTopLeft(find.byKey(ValueKey('related-open ${onOther.id}'))).dy;
      expect(related, lessThan(tester.getTopLeft(find.byKey(ValueKey('rail-open ${onExample.id}'))).dy), reason: 'related first');
      await tester.tapAt(const Offset(370, 400));
      await settle(tester);

      bridge.bring(1);
      await settle(tester);
      await _openDrawer(tester);
      expect(find.byKey(ValueKey('related-open ${onExample.id}')), findsOneWidget);
      expect(find.byKey(ValueKey('rail-open ${onOther.id}')), findsOneWidget);
    });

    testWidgets('earlier sessions of the origin are listed first and the other earlier ones follow', (tester) async {
      final ledger = SessionLedger(MemoryLedgerBackend(), clock: _Clock().now);
      final bridge = _bridge('https://example.com/a');
      final first = await _panel(tester, ledger: ledger, bridge: bridge, provider: _Provider(), mode: _chatOnly);
      await _start(tester, first, 'Kept on example');
      await tester.runAsync(() => first.close(first.sessions.single));
      bridge.open(const OpenTab(id: 2, title: 'Other', url: 'https://other.test/'));
      bridge.bring(2);
      await settle(tester);
      await _start(tester, first, 'Kept on other');
      await tester.runAsync(() => first.close(first.sessions.single));
      await tester.pumpWidget(const SizedBox());

      final store = await _panel(tester, ledger: ledger, bridge: _bridge('https://example.com/b'), provider: _Provider(), mode: _chatOnly);
      await _openDrawer(tester);
      Finder inDrawer(Finder finder) => find.descendant(of: find.byType(Drawer), matching: finder);
      expect(inDrawer(find.text('Earlier on example.com')), findsOneWidget);
      expect(inDrawer(_tiles()), findsOneWidget, reason: 'only the origin\'s own under the page');
      expect(inDrawer(find.text('Kept on example')), findsOneWidget);
      expect(inDrawer(find.text('Earlier conversations')), findsOneWidget);
      expect(inDrawer(find.text('Kept on other')), findsOneWidget, reason: 'the rest follow, as any earlier conversation');
      expect(
        tester.getTopLeft(inDrawer(find.text('Earlier on example.com'))).dy,
        lessThan(tester.getTopLeft(inDrawer(find.text('Earlier conversations'))).dy),
      );
      expect(store.sessions, isEmpty);
    });

    testWidgets('"This page only" narrows the list to the page, not its whole origin', (tester) async {
      final ledger = SessionLedger(MemoryLedgerBackend(), clock: _Clock().now);
      final bridge = _bridge('https://example.com/a');
      final first = await _panel(tester, ledger: ledger, bridge: bridge, provider: _Provider(), mode: _chatOnly);
      await _start(tester, first, 'On a');
      await tester.runAsync(() => first.close(first.sessions.single));
      await tester.pumpWidget(const SizedBox());

      await _panel(tester, ledger: ledger, bridge: _bridge('https://example.com/b'), provider: _Provider(), mode: _chatOnly);
      expect(_tiles(), findsOneWidget, reason: 'the origin');
      await _tap(tester, const Key('related-narrow'));
      expect(_tiles(), findsNothing, reason: 'another page of it');
      await tester.pumpWidget(const SizedBox());

      await _panel(tester, ledger: ledger, bridge: _bridge('https://example.com/a/'), provider: _Provider(), mode: _chatOnly);
      await _tap(tester, const Key('related-narrow'));
      expect(_tiles(), findsOneWidget, reason: 'the same page, its trailing slash aside');
    });

    testWidgets('a relation that is not a webpage works the same: the client gives it, and a session of it is listed first', (tester) async {
      final ledger = SessionLedger(MemoryLedgerBackend(), clock: _Clock().now);
      final provider = _Provider();
      final relation = ManualRelation(const Relation(kind: Relation.conversationKind, key: 'c-1', label: 'Quarterly numbers'));
      addTearDown(relation.dispose);
      final store = await _panel(tester, ledger: ledger, provider: provider, mode: _chatOnly, relations: relation);
      expect(tester.widget<Text>(find.byKey(const Key('related-name'))).data, 'Quarterly numbers');
      expect(find.byKey(const Key('related-narrow')), findsNothing, reason: 'a conversation has nothing narrower');

      await _start(tester, store, 'About the thread');
      expect(provider.asked.single, (cwd: siteStandInFolder, site: servedSite), reason: 'not a webpage: the app\'s own folder');
      expect(ledger.entries.single.relation!.kind, Relation.conversationKind);
      expect(ledger.entries.single.relation!.key, 'c-1');
      expect(provider.sent.single, isNot(contains('c-1')), reason: 'the agent is told none of it');
      expect(provider.sent.single, isNot(contains('Quarterly')));
      await tester.runAsync(() => store.close(store.sessions.single));
      await settle(tester);
      expect(_tiles(), findsOneWidget);

      // The client moves to another item, and then back.
      relation.current = const Relation(kind: Relation.conversationKind, key: 'c-2', label: 'Lunch');
      await settle(tester);
      expect(tester.widget<Text>(find.byKey(const Key('related-name'))).data, 'Lunch');
      expect(_tiles(), findsNothing);
      relation.current = const Relation(kind: Relation.conversationKind, key: 'c-1', label: 'Quarterly numbers');
      await settle(tester);
      expect(_tiles(), findsOneWidget);
      // A document with the same key is not the conversation.
      relation.current = const Relation(kind: Relation.documentKind, key: 'c-1');
      await settle(tester);
      expect(_tiles(), findsNothing);
    });

    testWidgets('a task pane that learns what is open later switches the related list with no reload', (tester) async {
      final ledger = SessionLedger(MemoryLedgerBackend(), clock: _Clock().now);
      final provider = _Provider();
      final pane = _Pane();
      final mode = ClientMode.fromUri(Uri.parse('https://canoa.example/office/app/?mode=chat&relation=conversation:c-1&label=Quarterly%20numbers'));
      final store = await _panel(tester, ledger: ledger, provider: provider, mode: mode, paneLink: pane);
      expect(pane.readied, 1, reason: 'the app says it listens once, after it can hear');
      expect(tester.widget<Text>(find.byKey(const Key('related-name'))).data, 'Quarterly numbers');

      await _start(tester, store, 'About the thread');
      expect(ledger.entries.single.relation!.key, 'c-1');
      await tester.runAsync(() => store.close(store.sessions.single));
      await settle(tester);
      expect(_tiles(), findsOneWidget);

      // The pinned Outlook pane moves to another message: the list follows, the page and the store are the ones they were.
      pane.say(const Relation(kind: Relation.conversationKind, key: 'c-2', label: 'Lunch'));
      await settle(tester);
      expect(tester.widget<Text>(find.byKey(const Key('related-name'))).data, 'Lunch');
      expect(_tiles(), findsNothing);
      expect(pane.readied, 1, reason: 'no reload: the app did not start over');
      pane.say(const Relation(kind: Relation.conversationKind, key: 'c-1', label: 'Quarterly numbers'));
      await settle(tester);
      expect(_tiles(), findsOneWidget, reason: 'back on the first conversation, its session is listed again');
      // A document with the same key is not the conversation, and nothing open lists none.
      pane.say(const Relation(kind: Relation.documentKind, key: 'c-1'));
      await settle(tester);
      expect(_tiles(), findsNothing);
      pane.say(null);
      await settle(tester);
      expect(_tiles(), findsNothing);
      expect(find.byKey(const Key('related-name')), findsNothing, reason: 'nothing is in front');

      await tester.pumpWidget(const SizedBox());
      expect(pane.disposed, isTrue, reason: 'the link is let go with the app');
    });

    testWidgets('a pane link is not used where the app is given its own relations, and the whole app listens to none', (tester) async {
      final pane = _Pane();
      final own = ManualRelation(const Relation(kind: Relation.conversationKind, key: 'c-1', label: 'Mine'));
      addTearDown(own.dispose);
      await _panel(tester, ledger: SessionLedger(MemoryLedgerBackend()), provider: _Provider(), mode: _chatOnly, relations: own, paneLink: pane);
      expect(pane.readied, 0);
      pane.say(const Relation(kind: Relation.conversationKind, key: 'c-2', label: 'Other'));
      await settle(tester);
      expect(tester.widget<Text>(find.byKey(const Key('related-name'))).data, 'Mine');

      final full = _Pane();
      await tester.pumpWidget(const SizedBox());
      await _panel(tester, ledger: SessionLedger(MemoryLedgerBackend()), provider: _Provider(), paneLink: full);
      expect(full.readied, 0, reason: 'the whole app is no pane');
    });

    testWidgets('the address names the mode and what is in front', (tester) async {
      final ledger = SessionLedger(MemoryLedgerBackend());
      final mode = ClientMode.fromUri(Uri.parse('https://canoa.example/app/?mode=chat&relation=document:abc&label=Plan'));
      await _panel(tester, ledger: ledger, provider: _Provider(), mode: mode);
      expect(tester.widget<Text>(find.byKey(const Key('related-name'))).data, 'Plan');
      await _openDrawer(tester);
      expect(find.byTooltip('Canoa settings'), findsNothing);
    });

    testWidgets('a link opens the whole app in a page of its own, with nothing of the chat-only mode in its address', (tester) async {
      final opened = <Uri>[];
      await _panel(
        tester, ledger: SessionLedger(MemoryLedgerBackend()), bridge: _bridge('https://example.com/a'), provider: _Provider(),
        mode: _chatOnly, openPage: (address) {
          opened.add(address);
          return true;
        },
      );
      await _openDrawer(tester);
      await tester.tap(find.byKey(const Key('open-full-app')));
      await settle(tester);
      expect(opened, hasLength(1));
      expect(opened.single.queryParameters['view'], 'full');
      for (final name in ['mode', 'relation', 'scope', 'label']) {
        expect(opened.single.queryParameters.containsKey(name), isFalse, reason: name);
      }
    });

    testWidgets('what the whole app does with a section opens the whole app instead', (tester) async {
      final opened = <Uri>[];
      await _panel(
        tester, ledger: SessionLedger(MemoryLedgerBackend()), bridge: _bridge('https://example.com/a'), provider: _Provider(),
        mode: _chatOnly, openPage: (address) {
          opened.add(address);
          return true;
        },
      );
      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.comma);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      await settle(tester);
      expect(opened, hasLength(1), reason: 'Ctrl+, is Settings there');
    });

    testWidgets('the loadout and profile are chosen beside New session, and the session starts with them', (tester) async {
      final provider = _Provider();
      final store = await _panel(
        tester, ledger: SessionLedger(MemoryLedgerBackend()), bridge: _bridge('https://example.com/a'), provider: provider,
        mode: _chatOnly, config: _withLoadouts,
      );
      expect(find.descendant(of: find.byKey(const Key('start-loadout')), matching: find.text('writing')), findsOneWidget,
          reason: 'the default until one is chosen');
      await _pick(tester, const Key('start-loadout'), const Key('loadout-choice review'));
      await _pick(tester, const Key('start-profile'), const Key('profile-choice deep'));
      expect(find.descendant(of: find.byKey(const Key('start-loadout')), matching: find.text('review')), findsOneWidget);
      expect(find.descendant(of: find.byKey(const Key('start-profile')), matching: find.text('deep')), findsOneWidget);

      await _tap(tester, const Key('related-new'));
      await until(tester, () => store.sessions.isNotEmpty && store.sessions.first.remoteSessionId != null, 'the session to open');
      expect(provider.initialized.single[loadoutMeta], 'review', reason: 'the host starts it with this loadout\'s skills');
      expect(provider.initialized.single[presetMeta], 'deep');
      expect(provider.initialized.single[clientMeta], 'browser');
      expect(store.sessions.single.loadout, 'review');
      expect(find.textContaining('Loadout review'), findsOneWidget, reason: 'the conversation says which loadout it started with');

      // The next session starts from the defaults again.
      await _openDrawer(tester);
      expect(find.descendant(of: find.byKey(const Key('start-loadout')), matching: find.text('writing')), findsOneWidget);
      expect(find.descendant(of: find.byKey(const Key('start-profile')), matching: find.text('Profile')), findsOneWidget);
    });

    testWidgets('a kind of client\'s default loadout applies when none is chosen, and the host is told the kind', (tester) async {
      for (final (client, relation, expected) in [
        ('browser', null, 'writing'),
        ('outlook', const Relation(kind: Relation.conversationKind, key: 'c1', label: 'Plan'), 'mail'),
        ('word', const Relation(kind: Relation.documentKind, key: 'd1', label: 'Budget.docx'), 'docs'),
        ('excel', const Relation(kind: Relation.documentKind, key: 'd2', label: 'Sheet.xlsx'), 'writing'),
      ]) {
        final provider = _Provider();
        final store = await _panel(
          tester, ledger: SessionLedger(MemoryLedgerBackend()), bridge: client == 'browser' ? _bridge('https://example.com/a') : null,
          provider: provider, config: _withLoadouts,
          mode: ClientMode(chatOnly: true, client: client == 'browser' ? null : client),
          relations: relation == null ? null : ManualRelation(relation),
        );
        expect(find.descendant(of: find.byKey(const Key('start-loadout')), matching: find.text(expected)), findsOneWidget, reason: client);
        await _tap(tester, const Key('related-new'));
        await until(tester, () => store.sessions.isNotEmpty && store.sessions.first.remoteSessionId != null, '$client\'s session to open');
        expect(provider.initialized.single[clientMeta], client);
        expect(provider.initialized.single.containsKey(loadoutMeta), isFalse, reason: 'none chosen: the host applies the default for the kind');
        expect(store.sessions.single.loadout, expected, reason: client);
        await tester.pumpWidget(const SizedBox());
      }
    });

    testWidgets('a kind\'s default naming a loadout canoa.cfg lost starts with the default and says so', (tester) async {
      final provider = _Provider();
      final store = await _panel(
        tester, ledger: SessionLedger(MemoryLedgerBackend()), provider: provider,
        config: CanoaConfig.parse('[loadouts.writing]\n[defaults]\nloadout = "writing"\n[defaults.clients]\noutlook = "gone"\n'),
        mode: const ClientMode(chatOnly: true, client: 'outlook'),
        relations: ManualRelation(const Relation(kind: Relation.conversationKind, key: 'c1', label: 'Plan')),
      );
      await _tap(tester, const Key('related-new'));
      await until(tester, () => store.sessions.isNotEmpty && store.sessions.first.remoteSessionId != null, 'the session to open');
      expect(store.sessions.single.loadout, 'writing');
      expect(find.textContaining('the loadout gone, which it no longer holds'), findsOneWidget);
    });

    testWidgets('a profile that names a loadout of its own wins over the kind\'s default', (tester) async {
      final provider = _Provider();
      final store = await _panel(
        tester, ledger: SessionLedger(MemoryLedgerBackend()), provider: provider, config: _withLoadouts,
        mode: const ClientMode(chatOnly: true, client: 'outlook'),
        relations: ManualRelation(const Relation(kind: Relation.conversationKind, key: 'c1', label: 'Plan')),
      );
      expect(find.descendant(of: find.byKey(const Key('start-loadout')), matching: find.text('mail')), findsOneWidget);
      await _pick(tester, const Key('start-profile'), const Key('profile-choice careful'));
      expect(find.descendant(of: find.byKey(const Key('start-loadout')), matching: find.text('mail')), findsOneWidget,
          reason: 'a profile with no loadout leaves the kind\'s');
      await _pick(tester, const Key('start-profile'), const Key('profile-choice deep'));
      expect(find.descendant(of: find.byKey(const Key('start-loadout')), matching: find.text('review')), findsOneWidget,
          reason: 'the profile\'s own loadout, not Outlook\'s default');
      await _tap(tester, const Key('related-new'));
      await until(tester, () => store.sessions.isNotEmpty && store.sessions.first.remoteSessionId != null, 'the session to open');
      expect(provider.initialized.single.containsKey(loadoutMeta), isFalse);
      expect(provider.initialized.single[presetMeta], 'deep');
      expect(store.sessions.single.loadout, 'review');
    });

    testWidgets('canoa.cfg with no loadout and no profile offers nothing to choose, and the session starts as before', (tester) async {
      final provider = _Provider();
      final store = await _panel(
        tester, ledger: SessionLedger(MemoryLedgerBackend()), bridge: _bridge('https://example.com/a'), provider: provider, mode: _chatOnly,
      );
      expect(find.byKey(const Key('start-loadout')), findsNothing);
      expect(find.byKey(const Key('start-profile')), findsNothing);
      await _tap(tester, const Key('related-new'));
      await until(tester, () => store.sessions.isNotEmpty && store.sessions.first.remoteSessionId != null, 'the session to open');
      expect(provider.initialized.single, {clientMeta: 'browser'}, reason: 'only its kind');
      expect(store.sessions.single.loadout, isNull);
    });

    testWidgets('sample: the side panel chooses the loadout and profile beside New session (CANOA_SAMPLES_DIR)', (tester) async {
      await _panel(
        tester, ledger: SessionLedger(MemoryLedgerBackend()), bridge: _bridge('https://example.com/docs', title: 'Docs'), provider: _Provider(),
        mode: _chatOnly, config: _withLoadouts,
      );
      await tester.tap(find.byKey(const Key('start-loadout')));
      await settle(tester);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('ext_panel_start_loadout.png'));
    }, skip: samplesSkip);

    testWidgets('sample: the chat-only drawer lists the page\'s sessions first (CANOA_SAMPLES_DIR)', (tester) async {
      final ledger = SessionLedger(MemoryLedgerBackend(), clock: _Clock().now);
      final bridge = _bridge('https://example.com/docs', title: 'Docs');
      final store = await _panel(tester, ledger: ledger, bridge: bridge, provider: _Provider(), mode: _chatOnly);
      final onExample = await _start(tester, store, 'Summarise the changelog');
      await tester.runAsync(() => store.close(onExample));
      await settle(tester);
      await _start(tester, store, 'Fix the broken link');
      bridge.open(const OpenTab(id: 2, title: 'Other', url: 'https://other.test/x'));
      bridge.bring(2);
      await settle(tester);
      await _start(tester, store, 'Compare the two quotes');
      bridge.bring(1);
      await settle(tester);
      await _openDrawer(tester);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('ext_chat_only_drawer.png'));
      await tester.tapAt(const Offset(370, 400));
      await settle(tester);
      store.select(store.sessions.firstWhere((session) => session.title == 'Fix the broken link').id);
      await settle(tester);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('ext_chat_only_conversation.png'));
    }, skip: samplesSkip);
  });
}

/// A clock that moves a minute at each reading, so the ledger orders what it notes.
final class _Clock {
  var _at = DateTime.utc(2026, 11, 12, 9);
  DateTime now() => _at = _at.add(const Duration(minutes: 1));
}
