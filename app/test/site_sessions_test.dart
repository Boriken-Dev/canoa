// The panel for the page in front: a session started from a webpage works in a scratch folder of that site, the panel lists the
// site's earlier sessions to resume or start another beside, and nothing of another site reaches it.
import 'dart:async';
import 'dart:convert';

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/extension/tab_bridge.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/main.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/workspace/site_meta.dart';
import 'package:coqui_acp/page.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart' show settle, until;
import 'fixtures/fake_tab_bridge.dart';
import 'fixtures/sample_render.dart' show samplesDir;

final _peer = 'sha256:${'ab' * 32}';
// What the agent's host puts the folders under; the panel does not know it.
const _documents = '/home/ana/Documents';
final _id = RegExp(r'[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}');

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
        result = {'protocolVersion': 1, 'agentCapabilities': {'loadSession': true}, 'agentInfo': {'name': 'site-agent', 'version': '1.0.0'}};
      case 'session/new':
        final site = ((params['_meta'] as Map<String, dynamic>?)?[siteMeta] as Map<String, dynamic>?)?['site'] as String?;
        owner.asked.add((cwd: params['cwd'] as String, site: site));
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
  required FakeTabBridge bridge,
  required _Provider provider,
  String? documents,
  String? workspace,
}) async {
  final store = SessionStore(ledger: ledger);
  addTearDown(store.dispose);
  await tester.binding.setSurfaceSize(const Size(380, 800));
  addTearDown(() => tester.binding.setSurfaceSize(null));
  await tester.pumpWidget(CanoaApp(
    store: store,
    config: workspace == null ? const CanoaConfig() : CanoaConfig(sessions: LedgerSettings(workspace: workspace)),
    tabBridge: bridge,
    browserProvider: Future.value(provider),
    documents: documents,
  ));
  await settle(tester);
  return store;
}

Future<void> _tap(WidgetTester tester, Key key) async {
  await tester.tap(find.byKey(key));
  await settle(tester);
}

Finder _tiles() => find.byWidgetPredicate((widget) => widget.key is ValueKey<String> && (widget.key! as ValueKey<String>).value.startsWith('site-session '));

void main() {
  testWidgets('a session from https://example.com/a works in scratch/example.com/<id>, which the ledger notes with its site', (tester) async {
    final ledger = SessionLedger(MemoryLedgerBackend());
    final provider = _Provider();
    final store = await _panel(tester, ledger: ledger, bridge: _bridge('https://example.com/a'), provider: provider);
    expect(find.byKey(const Key('site-name')), findsOneWidget);
    expect(tester.widget<Text>(find.byKey(const Key('site-name'))).data, 'example.com');

    await _tap(tester, const Key('site-new'));
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
  });

  testWidgets('reopening the panel on example.com lists the session and resumes it in its own folder', (tester) async {
    final ledger = SessionLedger(MemoryLedgerBackend());
    final first = _Provider();
    final opened = await _panel(tester, ledger: ledger, bridge: _bridge('https://example.com/a'), provider: first);
    await _tap(tester, const Key('site-new'));
    await until(tester, () => opened.sessions.isNotEmpty && opened.sessions.first.remoteSessionId != null, 'the session to open');
    final folder = first.newFolders.single;
    final key = ledger.entries.single.key;
    await tester.pumpWidget(const SizedBox());

    // The panel is closed and opened again on the same site: a new store over the same list.
    final provider = _Provider();
    final store = await _panel(tester, ledger: ledger, bridge: _bridge('https://example.com:8443/other?q=1'), provider: provider);
    expect(_tiles(), findsOneWidget);
    expect(find.text('Earlier on example.com'), findsOneWidget);
    expect(find.descendant(of: _tiles(), matching: find.textContaining(folder)), findsOneWidget);

    await _tap(tester, ValueKey('site-resume $key'));
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
    await _tap(tester, const Key('site-new'));
    await until(tester, () => opened.sessions.isNotEmpty && opened.sessions.first.remoteSessionId != null, 'the session to open');
    expect(ledger.entries.single.site, 'example.com');
    await tester.pumpWidget(const SizedBox());

    for (final url in ['https://example.org/', 'https://www.example.com/', 'https://com/', 'http://example.com.evil.test/']) {
      await _panel(tester, ledger: ledger, bridge: _bridge(url), provider: _Provider());
      expect(find.byKey(const Key('site-name')), findsOneWidget, reason: url);
      expect(_tiles(), findsNothing, reason: url);
      expect(find.textContaining('Earlier on'), findsNothing, reason: url);
      expect(find.textContaining('example.com/Canoa'), findsNothing, reason: url);
      expect(find.textContaining('/example.com/'), findsNothing, reason: 'nothing of example.com in $url\'s panel');
      await tester.pumpWidget(const SizedBox());
    }
    expect(ledger.forSite('example.org'), isEmpty);
  });

  testWidgets('a new session from the same site gets a folder of its own, and the site lists both newest first', (tester) async {
    final ledger = SessionLedger(MemoryLedgerBackend(), clock: _Clock().now);
    final provider = _Provider();
    final store = await _panel(tester, ledger: ledger, bridge: _bridge('https://example.com/a'), provider: provider);
    await _tap(tester, const Key('site-new'));
    await until(tester, () => store.sessions.length == 1 && store.sessions.first.remoteSessionId != null, 'the first session');
    await tester.runAsync(() => store.close(store.sessions.single));
    await settle(tester);
    await _tap(tester, const Key('site-new'));
    await until(tester, () => store.sessions.length == 1 && store.sessions.first.remoteSessionId != null, 'the second session');
    expect(provider.newFolders, hasLength(2));
    expect(provider.newFolders.first, isNot(provider.newFolders.last));
    for (final folder in provider.newFolders) {
      expect(folder, matches(RegExp('^$_documents/Canoa/scratch/example\\.com/${_id.pattern}\$')));
    }
    expect({for (final folder in provider.newFolders) folder.substring(0, folder.lastIndexOf('/'))}, hasLength(1), reason: 'one folder for the site');
    final listed = ledger.forSite('example.com');
    expect(listed.map((entry) => entry.cwd), [provider.newFolders.last, provider.newFolders.first], reason: 'newest first');
  });

  testWidgets('a page that is not a site with a safe name shows no list', (tester) async {
    for (final url in [
      'chrome://extensions/', 'file:///C:/Work/a.html', 'about:blank', 'https://[::1]/', 'https://con/', 'https://a%2f..%2fb/',
      'javascript:alert(1)', 'https://c:80:90/',
    ]) {
      await _panel(tester, ledger: SessionLedger(MemoryLedgerBackend()), bridge: _bridge(url), provider: _Provider());
      expect(find.byKey(const Key('site-name')), findsNothing, reason: url);
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
    expect(tester.widget<FilledButton>(find.byKey(const Key('site-new'))).onPressed, isNotNull);
    await _tap(tester, const Key('site-new'));
    await until(tester, () => store.sessions.isNotEmpty && store.sessions.first.remoteSessionId != null, 'the session to open');
    expect(provider.asked.single, (cwd: siteStandInFolder, site: 'example.com'));
    expect(provider.newFolders.single, startsWith('$_documents/Canoa/scratch/example.com/'), reason: 'the host places it');
  });

  testWidgets('a host that does not say which folder it made fails the session, and nothing is kept', (tester) async {
    final ledger = SessionLedger(MemoryLedgerBackend());
    final provider = _Provider()..forgets = true;
    final store = await _panel(tester, ledger: ledger, bridge: _bridge('https://example.com/'), provider: provider);
    await _tap(tester, const Key('site-new'));
    await until(tester, () => find.byKey(const Key('site-failed')).evaluate().isNotEmpty, 'the failure');
    expect(find.textContaining('did not say which folder'), findsOneWidget);
    expect(store.sessions, isEmpty);
    expect(ledger.entries, isEmpty);
  });

  testWidgets('sample: the side panel for a site lists its earlier sessions', (tester) async {
    if (samplesDir == null) return;
    final ledger = SessionLedger(MemoryLedgerBackend(), clock: _Clock().now);
    final provider = _Provider();
    final opened = await _panel(tester, ledger: ledger, bridge: _bridge('https://example.com/docs'), provider: provider);
    for (var count = 1; count <= 2; count++) {
      await _tap(tester, const Key('site-new'));
      await until(tester, () => opened.sessions.length == 1 && opened.sessions.first.remoteSessionId != null, 'session $count');
      ledger.touch(_peer, 'p$count', title: count == 1 ? 'Summarise the changelog' : 'Fix the broken link');
      await tester.runAsync(() => opened.close(opened.sessions.single));
      await settle(tester);
    }
    await tester.pumpWidget(const SizedBox());
    await _panel(tester, ledger: ledger, bridge: _bridge('https://example.com/docs'), provider: _Provider());
    await expectLater(find.byType(MaterialApp), matchesGoldenFile('ext_panel_site_sessions.png'));
  });
}

/// A clock that moves a minute at each reading, so the ledger orders what it notes.
final class _Clock {
  var _at = DateTime.utc(2026, 11, 12, 9);
  DateTime now() => _at = _at.add(const Duration(minutes: 1));
}
