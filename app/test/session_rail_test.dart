// The rail: sessions grouped by what they have for the person (Pinned, Working, Sessions, Settled folded, Archived folded), a
// row per state, and a menu with Pin, Rename, Fork, Archive and Close that its keys also reach.
import 'dart:io' show Directory;

import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/rail/session_groups.dart';
import 'package:canoa/rail/session_rail.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/worktree/worktree.dart';
import 'package:canoa/theme/look.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/store_fixtures.dart' show PlatformTolerant;

final _now = DateTime.utc(2026, 11, 12, 12);

/// A session of the store that is open, idle and connected, last active [ago] before [_now].
ChatSession _open(SessionStore store, String title, {String? cwd, Duration ago = const Duration(minutes: 5)}) {
  final session = store.addProviderSession(
    title: title, agent: 'Claude Code', peerId: 'local:claude', remoteSessionId: 'r-$title', cwd: cwd,
  )
    ..status = SessionStatus.connected;
  session.messages.add(ChatMessage(role: MessageRole.agent, text: 'hello', at: _now.subtract(ago)));
  return session;
}

/// The rail in a column [width] wide, with a clock that stands still. Nothing here starts an agent.
Future<List<String>> _show(WidgetTester tester, SessionStore store,
    {double width = 260, double height = 800, List<String>? closed, Brightness brightness = Brightness.dark, bool unfold = false}) async {
  await tester.binding.setSurfaceSize(Size(width, height));
  addTearDown(() => tester.binding.setSurfaceSize(null));
  final asked = closed ?? <String>[];
  await tester.pumpWidget(MaterialApp(
    debugShowCheckedModeBanner: false,
    theme: themeFor(LookId.canoa, brightness),
    home: Scaffold(
      body: SessionRail(
        store: store,
        onConnect: (_) async {},
        onUseBrowserAgents: (_, {required cwd, required title}) async {},
        onNewSession: () => asked.add('new'),
        onSettings: () => asked.add('settings'),
        onHosting: () => asked.add('hosting'),
        onSearch: () => asked.add('search'),
        onClose: (session) async => asked.add('close ${session.title}'),
        now: () => _now,
        touch: false,
      ),
    ),
  ));
  await tester.pump();
  if (unfold) {
    for (final fold in find.textContaining(RegExp(r'^(Settled|Archived) \d+$')).evaluate().toList()) {
      await tester.tap(find.byWidget(fold.widget));
      await tester.pump();
    }
  }
  return asked;
}

Finder _row(String title) => find.ancestor(of: find.text(title), matching: find.byType(ListTile)).first;

/// The text the row shows under its title.
String _under(WidgetTester tester, String title) => find
    .descendant(of: _row(title), matching: find.byType(Text))
    .evaluate()
    .map((element) => (element.widget as Text).data ?? '')
    .where((text) => text != title)
    .join('|');

Future<void> _openMenu(WidgetTester tester, String title) async {
  final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
  await mouse.addPointer(location: Offset.zero);
  await mouse.moveTo(tester.getCenter(_row(title)));
  await tester.pump();
  await tester.tap(find.descendant(of: _row(title), matching: find.byTooltip('Session actions')));
  // A working row's spinner never settles, so this waits long enough for the menu to open.
  await tester.pump();
  await tester.pump(const Duration(milliseconds: 400));
  await mouse.removePointer();
}

double _top(WidgetTester tester, String title) => tester.getTopLeft(_row(title)).dy;

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/session_rail_test.dart')));

  group('a row per state', () {
    // The state, how to put a session in it, and the word it shows.
    final states = <(String, void Function(SessionStore, ChatSession), String)>[
      ('Asks', (store, session) => session.pendingPermissions.add(PermissionQuestion('Edit a.txt', 'edit')), 'Needs you'),
      ('Runs', (store, session) => session.status = SessionStatus.working, 'Working'),
      ('Broke', (store, session) => session.failed = true, 'Failed'),
      ('Ended', (store, session) => session.unread = true, 'Done'),
      ('Quiet', (store, session) {}, 'Idle'),
      ('Away', (store, session) => session.status = SessionStatus.disconnected, 'Offline'),
      ('Booting', (store, session) {
        session
          ..status = SessionStatus.connecting
          ..remoteSessionId = null;
      }, 'Starting'),
    ];

    for (final (title, put, word) in states) {
      testWidgets(word, (tester) async {
        final store = SessionStore(includePreview: false);
        final session = _open(store, title, cwd: '/work/app');
        store.createDraft(title: 'Draft');
        put(store, session);
        // Another session is the one chosen, so looking is not what clears a mark.
        await _show(tester, store, unfold: true);
        expect(_under(tester, title), contains(word));
        expect(find.descendant(of: _row(title), matching: find.text(word)), findsOneWidget);
        store.dispose();
      });
    }

    testWidgets('New for a session not started, Preview for the offline preview', (tester) async {
      final store = SessionStore();
      store.createDraft(title: 'Draft');
      await _show(tester, store);
      expect(find.descendant(of: _row('Draft'), matching: find.text('New')), findsOneWidget);
      expect(find.descendant(of: _row('Getting started'), matching: find.text('Preview')), findsOneWidget);
      store.dispose();
    });

    testWidgets('marks keep their tooltips: Needs you, Working and New since you looked', (tester) async {
      final store = SessionStore(includePreview: false);
      _open(store, 'Asks').pendingPermissions.add(PermissionQuestion('Edit a.txt', 'edit'));
      _open(store, 'Runs').status = SessionStatus.working;
      _open(store, 'Ended').unread = true;
      store.createDraft(title: 'Draft');
      await _show(tester, store);
      expect(find.descendant(of: _row('Asks'), matching: find.byTooltip('Needs you')), findsOneWidget);
      expect(find.descendant(of: _row('Runs'), matching: find.byTooltip('Working')), findsOneWidget);
      expect(find.descendant(of: _row('Ended'), matching: find.byTooltip('New since you looked')), findsOneWidget);
      store.dispose();
    });

    testWidgets('the row reads title, then state, branch or folder, agent and time on one line each', (tester) async {
      final store = SessionStore(includePreview: false);
      _open(store, 'Fix the build', cwd: '/home/ana/devel/canoa', ago: const Duration(minutes: 7));
      _open(store, 'On a branch', cwd: '/home/ana/devel/canoa', ago: const Duration(hours: 3)).worktree =
          const SessionWorktree(path: '/w/canoa-x', branch: 'canoa/x', repo: '/home/ana/devel/canoa');
      _open(store, 'Old one', ago: const Duration(days: 2));
      store.createDraft(title: 'Draft');
      await _show(tester, store, unfold: true);
      expect(_under(tester, 'Fix the build'), 'Idle| · canoa · Claude Code · 7m');
      expect(_under(tester, 'On a branch'), 'Idle| · canoa/x · Claude Code · 3h');
      expect(_under(tester, 'Old one'), 'Idle| · Claude Code · 2d');
      store.dispose();
    });

    testWidgets('a title is one line however long', (tester) async {
      final store = SessionStore(includePreview: false);
      _open(store, 'A title that goes on and on and on and on, well past the width of the rail');
      store.createDraft(title: 'Draft');
      await _show(tester, store, unfold: true);
      final height = tester.getSize(find.ancestor(of: find.textContaining('A title that goes on'), matching: find.byType(ListTile)).first).height;
      expect(height, lessThanOrEqualTo(64), reason: 'two lines of a dense tile');
      store.dispose();
    });
  });

  group('grouping', () {
    testWidgets('Working holds what needs the person, then what runs; Settled is folded with a count; Sessions holds the rest', (tester) async {
      final store = SessionStore(includePreview: false);
      _open(store, 'Quiet one');
      _open(store, 'Quiet two');
      _open(store, 'Runs').status = SessionStatus.working;
      _open(store, 'Asks').pendingPermissions.add(PermissionQuestion('Edit a.txt', 'edit'));
      _open(store, 'Ended').unread = true;
      store.createDraft(title: 'Draft');
      await _show(tester, store);
      expect(find.text('Working'), findsWidgets);
      expect(find.text('Sessions'), findsOneWidget);
      expect(find.text('Settled 2'), findsOneWidget);
      expect(find.text('Pinned'), findsNothing);
      expect(_top(tester, 'Asks'), lessThan(_top(tester, 'Runs')), reason: 'a request before a turn that runs');
      expect(_top(tester, 'Runs'), lessThan(_top(tester, 'Ended')));
      expect(find.text('Quiet one'), findsNothing, reason: 'Settled is folded');
      expect(find.text('Quiet two'), findsNothing);
      await tester.tap(find.text('Settled 2'));
      await tester.pump();
      expect(find.text('Quiet one'), findsOneWidget);
      expect(find.text('Quiet two'), findsOneWidget);
      expect(_top(tester, 'Quiet one'), greaterThan(_top(tester, 'Ended')), reason: 'Settled is at the bottom');
      await tester.tap(find.text('Settled 2'));
      await tester.pump();
      expect(find.text('Quiet one'), findsNothing);
      store.dispose();
    });

    testWidgets('a group with nothing in it is not shown', (tester) async {
      final store = SessionStore(includePreview: false);
      store.createDraft(title: 'Draft');
      await _show(tester, store);
      expect(find.text('Sessions'), findsOneWidget);
      expect(find.text('Working'), findsNothing);
      expect(find.textContaining('Settled'), findsNothing);
      expect(find.textContaining('Archived'), findsNothing);
      store.dispose();
    });

    testWidgets('Settled opens by itself for the session the person is in', (tester) async {
      final store = SessionStore(includePreview: false);
      final quiet = _open(store, 'Quiet one');
      _open(store, 'Quiet two');
      store.createDraft(title: 'Draft');
      await _show(tester, store);
      expect(find.text('Quiet one'), findsNothing);
      store.select(quiet.id);
      await tester.pump();
      expect(find.text('Quiet one'), findsOneWidget);
      store.dispose();
    });

    test('the order Ctrl+1 to Ctrl+9 count in is the list\'s, folded groups included', () {
      final store = SessionStore(includePreview: false);
      final quiet = _open(store, 'Quiet');
      final pinned = _open(store, 'Pinned one')..pinned = true;
      final runs = _open(store, 'Runs')..status = SessionStatus.working;
      final archived = _open(store, 'Away')..archived = true;
      final fresh = store.createDraft(title: 'Draft');
      expect(listOrder(store.sessions).map((s) => s.id), [pinned.id, runs.id, fresh.id, quiet.id, archived.id]);
      store.dispose();
    });
  });

  group('pin and archive', () {
    testWidgets('Pin lifts a session under Pinned and Unpin puts it back', (tester) async {
      final store = SessionStore(includePreview: false);
      final first = _open(store, 'First');
      _open(store, 'Second');
      store.createDraft(title: 'Draft');
      store.select(first.id);
      await _show(tester, store);
      await _openMenu(tester, 'First');
      await tester.tap(find.text('Pin'));
      await tester.pumpAndSettle();
      expect(first.pinned, isTrue);
      expect(find.text('Pinned'), findsOneWidget);
      expect(_top(tester, 'First'), lessThan(_top(tester, 'Draft')));
      expect(find.descendant(of: _row('First'), matching: find.byIcon(Icons.push_pin)), findsOneWidget);
      await _openMenu(tester, 'First');
      await tester.tap(find.text('Unpin'));
      await tester.pumpAndSettle();
      expect(first.pinned, isFalse);
      expect(find.text('Pinned'), findsNothing);
      store.dispose();
    });

    testWidgets('Archive folds a session away under Archived, and Unarchive brings it back', (tester) async {
      final store = SessionStore(includePreview: false);
      final first = _open(store, 'First');
      _open(store, 'Second');
      store.createDraft(title: 'Draft');
      store.select(first.id);
      await _show(tester, store);
      await _openMenu(tester, 'First');
      await tester.tap(find.text('Archive'));
      await tester.pumpAndSettle();
      expect(first.archived, isTrue);
      expect(find.text('Archived 1'), findsOneWidget);
      // It is still the session the person is in, so it stays in sight.
      store.select(store.sessions.first.id);
      await tester.pump();
      expect(find.text('First'), findsNothing);
      await tester.tap(find.text('Archived 1'));
      await tester.pump();
      expect(find.text('First'), findsOneWidget);
      await _openMenu(tester, 'First');
      await tester.tap(find.text('Unarchive'));
      await tester.pumpAndSettle();
      expect(find.textContaining('Archived'), findsNothing);
      store.dispose();
    });

    testWidgets('an archived session that needs the person is not hidden', (tester) async {
      final store = SessionStore(includePreview: false);
      final first = _open(store, 'First')..archived = true;
      store.createDraft(title: 'Draft');
      await _show(tester, store);
      expect(find.text('Archived 1'), findsOneWidget);
      first.pendingPermissions.add(PermissionQuestion('Edit a.txt', 'edit'));
      store.setArchived(first, true);
      await tester.pump();
      expect(find.textContaining('Archived'), findsNothing);
      expect(find.text('First'), findsOneWidget);
      expect(find.descendant(of: _row('First'), matching: find.text('Needs you')), findsOneWidget);
      store.dispose();
    });

    test('pin and archive are kept with the session\'s ledger note and come back with it', () async {
      final ledger = SessionLedger(MemoryLedgerBackend());
      final store = SessionStore(includePreview: false, ledger: ledger);
      ledger.record(peerId: 'local:claude', sessionId: 'r-One', entry: 'Claude', place: SessionPlace.local, title: 'One');
      final session = _open(store, 'One');
      store.setPinned(session, true);
      store.setArchived(session, true);
      final entry = ledger.find('local:claude', 'r-One')!;
      expect((entry.pinned, entry.archived), (true, true));
      final again = LedgerEntry.fromJson(entry.toJson())!;
      expect((again.pinned, again.archived), (true, true));
      store.setPinned(session, false);
      expect(LedgerEntry.fromJson(ledger.find('local:claude', 'r-One')!.toJson())!.pinned, isFalse);
      expect(entry.toJson().containsKey('pinned'), isFalse, reason: 'nothing is written for the usual case');
      store.dispose();
    });
  });

  group('the menu', () {
    testWidgets('it lists Pin, Rename, Fork, Archive and Close, each with its key', (tester) async {
      final store = SessionStore(includePreview: false);
      final first = _open(store, 'First');
      store.select(first.id);
      await _show(tester, store);
      await _openMenu(tester, 'First');
      for (final (label, key) in const [('Pin', 'P'), ('Rename', 'R'), ('Fork', 'F'), ('Archive', 'A'), ('Close', 'C')]) {
        final item = find.ancestor(of: find.text(label), matching: find.byType(MenuItemButton)).first;
        expect(find.descendant(of: item, matching: find.text(key)), findsOneWidget, reason: '$label has the key $key');
      }
      store.dispose();
    });

    testWidgets('the button shows on the row the pointer is over and on the one chosen, not on the rest', (tester) async {
      final store = SessionStore(includePreview: false);
      final first = _open(store, 'First');
      _open(store, 'Second');
      store.select(first.id);
      await _show(tester, store);
      Finder button(String title) => find.descendant(of: _row(title), matching: find.byTooltip('Session actions'));
      expect(button('First'), findsOneWidget);
      expect(button('Second'), findsNothing);
      final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
      await mouse.addPointer(location: Offset.zero);
      addTearDown(mouse.removePointer);
      await mouse.moveTo(tester.getCenter(_row('Second')));
      await tester.pump();
      expect(button('Second'), findsOneWidget);
      store.dispose();
    });

    testWidgets('R renames while the menu is open, and a key does nothing with it closed', (tester) async {
      final store = SessionStore(includePreview: false);
      final first = _open(store, 'First');
      store.select(first.id);
      await _show(tester, store);
      await tester.sendKeyEvent(LogicalKeyboardKey.keyR);
      await tester.pumpAndSettle();
      expect(find.text('Rename session'), findsNothing);
      await _openMenu(tester, 'First');
      await tester.sendKeyEvent(LogicalKeyboardKey.keyR);
      await tester.pumpAndSettle();
      expect(find.text('Rename session'), findsOneWidget);
      await tester.enterText(find.byKey(const Key('rename-session')), 'Better name');
      await tester.tap(find.text('Rename'));
      await tester.pumpAndSettle();
      expect(first.title, 'Better name');
      expect(first.agentNames, isFalse, reason: 'the agent stops naming what the person named');
      expect(find.text('Better name'), findsOneWidget);
      store.dispose();
    });

    testWidgets('F forks: a new session in the same folder and source, with nothing started', (tester) async {
      final store = SessionStore(includePreview: false);
      final first = _open(store, 'First', cwd: '/work/app');
      store.select(first.id);
      await _show(tester, store);
      await _openMenu(tester, 'First');
      await tester.sendKeyEvent(LogicalKeyboardKey.keyF);
      await tester.pumpAndSettle();
      expect(store.sessions, hasLength(2));
      final fork = store.selected!;
      expect(fork.title, 'Fork of First');
      expect(fork.status, SessionStatus.draft);
      expect(fork.messages, isEmpty, reason: 'what was said is the agent\'s, not copied');
      store.dispose();
    });

    test('a fork of a session in a worktree starts a worktree of its own from the repository', () {
      final store = SessionStore(includePreview: false);
      final first = _open(store, 'First', cwd: '/w/canoa-x')
        ..worktree = const SessionWorktree(path: '/w/canoa-x', branch: 'canoa/x', repo: '/home/ana/devel/canoa');
      final fork = store.fork(first);
      expect(fork.cwd, '/home/ana/devel/canoa');
      expect(fork.wantsWorktree, isTrue);
      store.dispose();
    });

    testWidgets('A archives and C closes', (tester) async {
      final store = SessionStore(includePreview: false);
      final first = _open(store, 'First');
      store.select(first.id);
      final asked = await _show(tester, store);
      await _openMenu(tester, 'First');
      await tester.sendKeyEvent(LogicalKeyboardKey.keyA);
      await tester.pumpAndSettle();
      expect(first.archived, isTrue);
      await _openMenu(tester, 'First');
      await tester.sendKeyEvent(LogicalKeyboardKey.keyC);
      await tester.pumpAndSettle();
      expect(asked, ['close First']);
      store.dispose();
    });

    testWidgets('a right click opens the same menu', (tester) async {
      final store = SessionStore(includePreview: false);
      final first = _open(store, 'First');
      store.createDraft(title: 'Draft');
      store.select(first.id);
      await _show(tester, store);
      final click = await tester.createGesture(kind: PointerDeviceKind.mouse, buttons: kSecondaryButton);
      await click.down(tester.getCenter(_row('First')));
      await click.up();
      await tester.pumpAndSettle();
      expect(find.text('Rename'), findsOneWidget);
      store.dispose();
    });
  });

  group('the entries', () {
    testWidgets('New session, Search, Connect, Hosting and Settings are named, with the keys of the first two', (tester) async {
      final store = SessionStore(includePreview: false);
      final asked = await _show(tester, store);
      for (final label in const ['New session', 'Search', 'Connect', 'Hosting', 'Settings']) {
        expect(find.text(label), findsOneWidget);
      }
      expect(find.text('Ctrl+N'), findsOneWidget);
      expect(find.text('Ctrl+K'), findsOneWidget);
      await tester.tap(find.text('New session'));
      await tester.tap(find.text('Search'));
      await tester.tap(find.text('Hosting'));
      await tester.tap(find.text('Settings'));
      expect(asked, ['new', 'search', 'hosting', 'settings']);
      expect(find.text('Agents on other machines need a verified ACP peer.'), findsNothing, reason: 'that note is Connect\'s');
      store.dispose();
    });

    testWidgets('the entries keep their tooltips', (tester) async {
      final store = SessionStore(includePreview: false);
      await _show(tester, store);
      for (final tip in const ['New session', 'Canoa settings', 'Hosting', 'Connect to verified agent']) {
        expect(find.byTooltip(tip), findsOneWidget);
      }
      store.dispose();
    });
  });

  group('goldens', () {
    Future<SessionStore> busy() async {
      final store = SessionStore(includePreview: false);
      final pinned = _open(store, 'Release notes', cwd: '/home/ana/devel/canoa', ago: const Duration(hours: 20))..pinned = true;
      _open(store, 'Quiet one', cwd: '/home/ana/devel/canoa', ago: const Duration(days: 1));
      _open(store, 'Quiet two', cwd: '/home/ana/devel/coqui-acp', ago: const Duration(days: 3));
      _open(store, 'Away for now', cwd: '/home/ana/devel/canoa', ago: const Duration(days: 9)).archived = true;
      _open(store, 'Fix the failing test', cwd: '/home/ana/devel/canoa', ago: const Duration(minutes: 12)).failed = true;
      _open(store, 'Refactor the rail', cwd: '/home/ana/devel/canoa', ago: const Duration(hours: 2)).unread = true;
      _open(store, 'Add worktree support', cwd: '/home/ana/devel/canoa', ago: const Duration(seconds: 20))
        ..worktree = const SessionWorktree(path: '/w/canoa-wt', branch: 'canoa/worktrees', repo: '/home/ana/devel/canoa')
        ..status = SessionStatus.working;
      _open(store, 'Review the diff', cwd: '/home/ana/devel/canoa', ago: const Duration(minutes: 1))
          .pendingPermissions
          .add(PermissionQuestion('Edit a.txt', 'edit'));
      store.createDraft(title: 'Draft', cwd: '/home/ana/devel/canoa');
      store.select(pinned.id);
      return store;
    }

    for (final (label, width, height) in const [('desktop', 260.0, 1000.0), ('phone', 304.0, 1000.0)]) {
      testWidgets('the rail at $label width', (tester) async {
        final store = await busy();
        await _show(tester, store, width: width, height: height);
        await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/rail_$label.png'));
        store.dispose();
      });
    }

    testWidgets('the rail with Settled and Archived open, and a menu', (tester) async {
      final store = await busy();
      await _show(tester, store, width: 260, height: 1000);
      await tester.tap(find.text('Settled 2'));
      await tester.tap(find.text('Archived 1'));
      await tester.pump();
      await _openMenu(tester, 'Refactor the rail');
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/rail_open_menu.png'));
      store.dispose();
    });
  });
}
