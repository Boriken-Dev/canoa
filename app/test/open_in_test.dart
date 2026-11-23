// Open the workspace in VS Code or the file manager from the app that runs the host: which sessions are on this computer, and what
// the header and the palette offer by what the host says it can start.
import 'dart:io' show Directory;

import 'package:canoa/desktop/control_client.dart' show ControlFailed;
import 'package:canoa/session_store.dart';
import 'package:canoa/workspace/open_in.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter/services.dart' show LogicalKeyboardKey;
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/open_host.dart';
import 'fixtures/store_fixtures.dart' show PlatformTolerant;


WorkspaceAction _action(WorkspaceOpener opener, String id) => opener.actions.firstWhere((action) => action.id == id);

void main() {
  group('where a workspace is', () {
    final pipe = Uri.parse('pipe:canoa');

    test('on this computer: a local session, or one reached through this app\'s own host by its pipe', () async {
      final opener = await OpenHost().opener();
      expect(opener.workspaceOf(place: SessionPlace.local, cwd: '/work').target?.path, '/work');
      expect(opener.workspaceOf(place: SessionPlace.host, cwd: '/work', endpoint: pipe).target?.path, '/work');
    });

    test('on another host: no target, and the reason', () async {
      final opener = await OpenHost().opener();
      final remote = opener.workspaceOf(place: SessionPlace.peer, cwd: '/work', endpoint: Uri.parse('wss://box.example/acp'));
      expect(remote.target, isNull);
      expect(remote.reason, contains('another host'));
      expect(opener.workspaceOf(place: SessionPlace.host, cwd: '/work').target, isNull, reason: 'a host with no known address is not trusted to be local');
      expect(opener.workspaceOf(place: SessionPlace.browser, cwd: '/work').reason, contains('browser'));
    });

    test('no folder, or an app with no host: nothing is offered', () async {
      expect((await OpenHost().opener()).workspaceOf(place: SessionPlace.local, cwd: null), (target: null, reason: null));
      const none = WorkspaceOpener.none();
      expect(none.canOpen, isFalse);
      expect(none.workspaceOf(place: SessionPlace.local, cwd: '/work'), (target: null, reason: null));
      expect(none.actionsFor(WorkspaceContext.header, const WorkspaceTarget('/work')), isEmpty);
    });
  });

  group('Open in browser', () {
    test('an https address is handed to the launcher as given', () async {
      final launched = <Uri>[];
      final opener = WorkspaceOpener.none(launch: (uri) async { launched.add(uri); return true; });
      await _action(opener, 'browser').run(const WorkspaceTarget('https://example.com/a%20b?q=%C3%B1'));
      expect(launched.single.scheme, 'https');
      expect(launched.single.host, 'example.com');
    });

    test('a bare host is taken as https, and an http address stays http', () async {
      final launched = <Uri>[];
      final opener = WorkspaceOpener.none(launch: (uri) async { launched.add(uri); return true; });
      await opener.openUrl('localhost:3000/app');
      await opener.openUrl('http://127.0.0.1:8080');
      expect(launched.map((uri) => uri.toString()), ['https://localhost:3000/app', 'http://127.0.0.1:8080']);
    });

    test('file, javascript, data and credentialed addresses are refused and never launched', () async {
      final launched = <Uri>[];
      final opener = WorkspaceOpener.none(launch: (uri) async { launched.add(uri); return true; });
      for (final text in ['file:///etc/passwd', 'javascript:alert(1)', 'data:text/html,hi', 'https://user:pw@example.com/', 'ftp://example.com/', '', 'https://']) {
        await expectLater(opener.openUrl(text), throwsA(isA<OpenInFailure>()), reason: text);
        expect(_action(opener, 'browser').available(WorkspaceTarget(text)), isFalse, reason: text);
      }
      expect(launched, isEmpty);
    });

    test('a launcher that nothing answers is a failure the person reads', () async {
      final opener = WorkspaceOpener.none(launch: (uri) async => false);
      await expectLater(opener.openUrl('https://example.com'), throwsA(isA<OpenInFailure>()));
    });

    test('it is offered in its own context only, and not beside the folder actions', () async {
      final opener = await OpenHost().opener();
      expect(opener.actionsFor(WorkspaceContext.header, const WorkspaceTarget('/work')).map((action) => action.id), isNot(contains('browser')));
      expect(opener.actionsFor(WorkspaceContext.browser, const WorkspaceTarget('https://example.com')).map((action) => action.id), ['browser']);
    });

    test('an app that reaches no host offers it too: the browser opens the address itself', () {
      const page = WorkspaceOpener.none();
      expect(page.actionsFor(WorkspaceContext.browser, const WorkspaceTarget('https://example.com')), hasLength(1));
    });
  });

  group('the app', () {
    late WorkspaceOpener before;
    late OpenHost host;
    setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/open_in_test.dart')));
    setUp(() async {
      before = workspaceOpener;
      host = OpenHost();
      workspaceOpener = await host.opener();
    });
    tearDown(() => workspaceOpener = before);

    SessionStore store(SessionPlace place, {Uri? endpoint}) {
      final store = SessionStore();
      addTearDown(store.dispose);
      store.addProviderSession(title: 'Fix it', agent: 'Claude Code', peerId: 'local:claude', remoteSessionId: 'r', cwd: '/work')
        ..status = SessionStatus.connected
        ..place = place
        ..endpoint = endpoint;
      return store;
    }

    testWidgets('the header opens the folder in VS Code', (tester) async {
      await pumpApp(tester, store(SessionPlace.local));
      await tester.tap(find.byTooltip('Open the workspace'));
      await frames(tester);
      expect(find.text('/work'), findsOneWidget);
      expect(find.text('Open in VS Code'), findsOneWidget);
      expect(find.text('Open in file manager'), findsOneWidget);
      await tester.tap(find.text('Open in VS Code'));
      await frames(tester);
      expect(host.started.single, ('/work', 'vscode'));
    });

    testWidgets('the dialog, drawn', (tester) async {
      await pumpApp(tester, store(SessionPlace.local));
      await tester.tap(find.byTooltip('Open the workspace'));
      await frames(tester);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/open_workspace_desktop.png'));
    });

    testWidgets('without VS Code on the host its action is not shown', (tester) async {
      workspaceOpener = await OpenHost(ids: const ['file_manager', 'reveal']).opener();
      await pumpApp(tester, store(SessionPlace.local));
      await tester.tap(find.byTooltip('Open the workspace'));
      await frames(tester);
      expect(find.text('Open in VS Code'), findsNothing);
      expect(find.text('Open in file manager'), findsOneWidget);
    });

    testWidgets('on another host the dialog says why and offers nothing to start', (tester) async {
      await pumpApp(tester, store(SessionPlace.peer, endpoint: Uri.parse('wss://box.example/acp')));
      await tester.tap(find.byTooltip('Open the workspace'));
      await frames(tester);
      expect(find.textContaining('another host'), findsOneWidget);
      expect(find.text('Open in VS Code'), findsNothing);
      expect(find.text('Open in file manager'), findsNothing);
    });

    testWidgets('a program the host will not start is a snackbar with its words', (tester) async {
      host.failWith = const ControlFailed(-32002, 'Could not start /usr/bin/code: denied');
      await pumpApp(tester, store(SessionPlace.local));
      await tester.tap(find.byTooltip('Open the workspace'));
      await frames(tester);
      await tester.tap(find.text('Open in VS Code'));
      await frames(tester);
      expect(find.textContaining('denied'), findsOneWidget);
    });

    testWidgets('the palette offers them too', (tester) async {
      await pumpApp(tester, store(SessionPlace.local));
      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.keyK);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      await frames(tester);
      expect(find.widgetWithText(ListTile, 'Open in VS Code'), findsOneWidget);
      await tester.tap(find.widgetWithText(ListTile, 'Open in VS Code'));
      await frames(tester);
      expect(host.started.single, ('/work', 'vscode'));
    });

    testWidgets('an app with no host shows no button', (tester) async {
      workspaceOpener = const WorkspaceOpener.none();
      await pumpApp(tester, store(SessionPlace.local), openIn: false);
      expect(find.byTooltip('Open the workspace'), findsNothing);
      expect(find.byIcon(Icons.folder_open), findsNothing);
    });
  });
}
