// Open the workspace in VS Code or the file manager: what is detected on each system, the arguments each starts with (the path
// always one argument), which paths are refused, where a workspace is local, and what the header and the palette offer.
import 'dart:io' show Directory;

import 'package:canoa/session_store.dart';
import 'package:canoa/workspace/open_in.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart' show LogicalKeyboardKey;
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/store_fixtures.dart' show PlatformTolerant;

final class _System implements OpenInSystem {
  _System(this.os, {Set<String> files = const {}, Set<String> folders = const {}, this.registry = const {}, this.bundle, this.environment = const {}})
      : files = {...files},
        folders = {...folders};

  @override
  final HostOs? os;
  @override
  final Map<String, String> environment;
  final Set<String> files, folders;
  final Map<String, String> registry;
  final String? bundle;
  final List<LaunchPlan> started = [];
  Object? failWith;

  @override
  bool isDirectory(String path) => folders.contains(path);
  @override
  bool isFile(String path) => files.contains(path);
  @override
  String? registryDefault(String key) => registry[key];
  @override
  String? bundlePath(String id) => id == 'com.microsoft.VSCode' ? bundle : null;
  @override
  Future<void> start(String executable, List<String> arguments) async {
    if (failWith case final failure?) throw failure;
    started.add(LaunchPlan(executable, arguments));
  }
}

WorkspaceAction _action(WorkspaceOpener opener, String id) => opener.actions.firstWhere((action) => action.id == id);

const _winCode = r'D:\Profiles\ana\AppData\Local\Programs\Microsoft VS Code\Code.exe';
const _spaced = r'D:\Profiles\ana\Mis proyectos\canoa ñandú';

void main() {
  group('VS Code is found per system', () {
    test('Windows: App Paths first, then the install folders, then code.cmd beside Code.exe on PATH', () {
      final fromRegistry = WorkspaceOpener(_System(HostOs.windows,
          files: {_winCode}, registry: {r'HKCU\Software\Microsoft\Windows\CurrentVersion\App Paths\Code.exe': _winCode}));
      expect(fromRegistry.vsCodePlan(r'C:\work'), LaunchPlan(_winCode, [r'C:\work']));

      final installed = WorkspaceOpener(_System(HostOs.windows,
          files: {_winCode}, environment: {'LOCALAPPDATA': r'D:\Profiles\ana\AppData\Local'}));
      expect(installed.vsCodePlan(r'C:\work')?.executable, _winCode);

      final onPath = WorkspaceOpener(_System(HostOs.windows,
          files: {r'D:\VSCode\bin\code.cmd', r'D:\VSCode\bin\..\Code.exe'}, environment: {'Path': r'C:\x;D:\VSCode\bin'}));
      expect(onPath.vsCodePlan(r'C:\work')?.executable, r'D:\VSCode\bin\..\Code.exe');
    });

    test('Windows never starts code.cmd, which only a shell can run', () {
      final opener = WorkspaceOpener(_System(HostOs.windows, files: {r'D:\VSCode\bin\code.cmd'}, environment: {'PATH': r'D:\VSCode\bin'}));
      expect(opener.vsCodePlan(r'C:\work'), isNull);
      expect(opener.actionsFor(WorkspaceContext.header, const WorkspaceTarget(r'C:\work')).map((a) => a.id), ['file_manager']);
    });

    test('macOS: the bundle with this id, started through open', () {
      final opener = WorkspaceOpener(_System(HostOs.macos, bundle: '/Applications/Visual Studio Code.app'));
      expect(opener.vsCodePlan('/Users/ana/work'), const LaunchPlan('open', ['-a', '/Applications/Visual Studio Code.app', '/Users/ana/work']));
      expect(WorkspaceOpener(_System(HostOs.macos)).vsCodePlan('/Users/ana/work'), isNull);
    });

    test('Linux: code on PATH', () {
      final opener = WorkspaceOpener(_System(HostOs.linux, files: {'/usr/bin/code'}, environment: {'PATH': '/bin:/usr/bin'}));
      expect(opener.vsCodePlan('/home/ana/work'), const LaunchPlan('/usr/bin/code', ['/home/ana/work']));
      expect(WorkspaceOpener(_System(HostOs.linux, environment: {'PATH': '/bin'})).vsCodePlan('/home/ana/work'), isNull);
    });

    test('the registry query\'s output, with variables filled in', () {
      const output = '\r\nHKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\App Paths\\Code.exe\r\n'
          '    (Default)    REG_EXPAND_SZ    %LOCALAPPDATA%\\Programs\\Microsoft VS Code\\Code.exe\r\n';
      expect(parseRegistryDefault(output, {'LocalAppData': r'D:\Profiles\ana\AppData\Local'}), _winCode);
      expect(parseRegistryDefault('    (Default)    REG_SZ    "C:\\a b\\Code.exe"\r\n', {}), r'C:\a b\Code.exe');
      expect(parseRegistryDefault('ERROR: The system was unable to find the specified registry key', {}), isNull);
    });
  });

  group('the file manager', () {
    test('Windows: Explorer from SystemRoot, a folder opened, an item selected', () {
      final opener = WorkspaceOpener(_System(HostOs.windows, environment: {'SystemRoot': r'C:\Windows'}));
      expect(opener.fileManagerPlan(_spaced), const LaunchPlan(r'C:\Windows\explorer.exe', [_spaced]));
      expect(opener.fileManagerPlan(_spaced, item: true), const LaunchPlan(r'C:\Windows\explorer.exe', ['/select,', _spaced]));
    });

    test('macOS: open, and open -R to reveal', () {
      final opener = WorkspaceOpener(_System(HostOs.macos));
      expect(opener.fileManagerPlan('/Users/ana/Mis proyectos'), const LaunchPlan('open', ['/Users/ana/Mis proyectos']));
      expect(opener.fileManagerPlan('/Users/ana/Mis proyectos', item: true), const LaunchPlan('open', ['-R', '/Users/ana/Mis proyectos']));
    });

    test('Linux: xdg-open, and the file manager\'s own D-Bus call to reveal, else the parent folder', () {
      final plain = WorkspaceOpener(_System(HostOs.linux));
      expect(plain.fileManagerPlan('/home/ana/a b'), const LaunchPlan('xdg-open', ['/home/ana/a b']));
      expect(plain.fileManagerPlan('/home/ana/a b/x.txt', item: true), const LaunchPlan('xdg-open', ['/home/ana/a b']));

      final withBus = WorkspaceOpener(_System(HostOs.linux, files: {'/usr/bin/gdbus'}, environment: {'PATH': '/usr/bin'}));
      final plan = withBus.fileManagerPlan("/home/ana/it's ñ/x.txt", item: true)!;
      expect(plan.executable, '/usr/bin/gdbus');
      expect(plan.arguments, containsAllInOrder(['--method', 'org.freedesktop.FileManager1.ShowItems']));
      expect(plan.arguments[plan.arguments.length - 2], "['file:///home/ana/it%27s%20%C3%B1/x.txt']",
          reason: 'one URI, percent-escaped, with the quote that would end the text escaped as well');
    });

    test('it is labelled with the platform\'s name', () {
      expect(_action(WorkspaceOpener(_System(HostOs.windows)), 'file_manager').label, 'Open in Explorer');
      expect(_action(WorkspaceOpener(_System(HostOs.macos)), 'file_manager').label, 'Open in Finder');
      expect(_action(WorkspaceOpener(_System(HostOs.macos)), 'reveal').label, 'Reveal in Finder');
      expect(_action(WorkspaceOpener(_System(HostOs.linux)), 'file_manager').label, 'Open in file manager');
    });
  });

  group('what is run', () {
    test('a path with spaces and non-ASCII reaches the program as one argument', () async {
      final system = _System(HostOs.windows, files: {_winCode}, folders: {_spaced}, registry: {r'HKCU\Software\Microsoft\Windows\CurrentVersion\App Paths\Code.exe': _winCode});
      final opener = WorkspaceOpener(system);
      await _action(opener, 'vscode').run(const WorkspaceTarget(_spaced));
      await _action(opener, 'file_manager').run(const WorkspaceTarget(_spaced));
      expect(system.started, [
        const LaunchPlan(_winCode, [_spaced]),
        const LaunchPlan('explorer.exe', [_spaced]),
      ]);
    });

    test('a path is a local, absolute, existing folder or it is refused', () async {
      final system = _System(HostOs.windows, folders: {r'C:\work', r'C:\work\a', r'\\filer\team\work', r'\\?\D:\Profiles\ana\work', r'D:\Profiles\ana\work:s'});
      final opener = WorkspaceOpener(system);
      final run = _action(opener, 'file_manager').run;
      for (final bad in [
        r'work', r'..\work', r'C:\work\..\x', r'C:work', r'\\', r'\\\x', 'C:\\work\u0000', 'C:\\work\nx', '', r'C:\missing', r'\\filer\team\missing',
      ]) {
        await expectLater(run(WorkspaceTarget(bad)), throwsA(isA<OpenInFailure>()), reason: bad);
      }
      expect(system.started, isEmpty);
      await run(const WorkspaceTarget('C:/work/a/'));
      expect(system.started.single.arguments, [r'C:\work\a'], reason: 'normalised, one argument');
      // What the system's own file manager opens is opened: a share, a verbatim path, a name with a stream.
      system.started.clear();
      for (final path in [r'\\filer\team\work', r'\\?\D:\Profiles\ana\work', r'D:\Profiles\ana\work:s']) {
        await run(WorkspaceTarget(path));
      }
      expect([for (final plan in system.started) plan.arguments.single], [r'\\filer\team\work', r'\\?\D:\Profiles\ana\work', r'D:\Profiles\ana\work:s']);

      final posix = WorkspaceOpener(_System(HostOs.linux, folders: {'/home/ana'}));
      for (final bad in ['home/ana', '/home/ana/../root', '-n', '/home/ana\n', '/nowhere']) {
        await expectLater(_action(posix, 'file_manager').run(WorkspaceTarget(bad)), throwsA(isA<OpenInFailure>()), reason: bad);
      }
    });

    test('a file is revealed only as an item, and a folder action does not take one', () async {
      final system = _System(HostOs.macos, files: {'/Users/ana/work/a.txt'}, folders: {'/Users/ana/work'});
      final opener = WorkspaceOpener(system);
      const file = WorkspaceTarget('/Users/ana/work/a.txt', item: true);
      expect(opener.actionsFor(WorkspaceContext.files, file).map((a) => a.id), ['reveal']);
      expect(opener.actionsFor(WorkspaceContext.header, file), isEmpty);
      await _action(opener, 'reveal').run(file);
      expect(system.started.single, const LaunchPlan('open', ['-R', '/Users/ana/work/a.txt']));
      await expectLater(_action(opener, 'file_manager').run(const WorkspaceTarget('/Users/ana/work/a.txt')), throwsA(isA<OpenInFailure>()));
    });

    test('a program that will not start is a failure the person reads', () async {
      final system = _System(HostOs.linux, folders: {'/home/ana'})..failWith = OpenInFailure('Could not start xdg-open: not found');
      await expectLater(
        _action(WorkspaceOpener(system), 'file_manager').run(const WorkspaceTarget('/home/ana')),
        throwsA(isA<OpenInFailure>().having((f) => f.message, 'message', contains('xdg-open'))),
      );
    });
  });

  group('where a workspace is', () {
    final opener = WorkspaceOpener(_System(HostOs.linux));
    final pipe = Uri.parse('pipe:canoa');

    test('on this computer: a local session, or one reached through this app\'s own host by its pipe', () {
      expect(opener.workspaceOf(place: SessionPlace.local, cwd: '/work').target?.path, '/work');
      expect(opener.workspaceOf(place: SessionPlace.host, cwd: '/work', endpoint: pipe).target?.path, '/work');
    });

    test('on another host: no target, and the reason', () {
      final remote = opener.workspaceOf(place: SessionPlace.peer, cwd: '/work', endpoint: Uri.parse('wss://box.example/acp'));
      expect(remote.target, isNull);
      expect(remote.reason, contains('another host'));
      expect(opener.workspaceOf(place: SessionPlace.host, cwd: '/work').target, isNull, reason: 'a host with no known address is not trusted to be local');
      expect(opener.workspaceOf(place: SessionPlace.browser, cwd: '/work').reason, contains('browser'));
    });

    test('no folder, or a page that cannot start programs: nothing is offered', () {
      expect(opener.workspaceOf(place: SessionPlace.local, cwd: null), (target: null, reason: null));
      final page = WorkspaceOpener(_System(null));
      expect(page.canOpen, isFalse);
      expect(page.workspaceOf(place: SessionPlace.local, cwd: '/work'), (target: null, reason: null));
      expect(page.actionsFor(WorkspaceContext.header, const WorkspaceTarget('/work')), isEmpty);
    });
  });

  group('the app', () {
    late WorkspaceOpener before;
    late _System system;
    setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/open_in_test.dart')));
    setUp(() {
      before = workspaceOpener;
      system = _System(HostOs.linux, files: {'/usr/bin/code'}, folders: {'/work'}, environment: {'PATH': '/usr/bin'});
      workspaceOpener = WorkspaceOpener(system);
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
      expect(system.started.single, const LaunchPlan('/usr/bin/code', ['/work']));
    });

    testWidgets('the dialog, drawn', (tester) async {
      await pumpApp(tester, store(SessionPlace.local));
      await tester.tap(find.byTooltip('Open the workspace'));
      await frames(tester);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/open_workspace_desktop.png'));
    });

    testWidgets('without VS Code its action is not shown', (tester) async {
      system.files.clear();
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

    testWidgets('a program that will not start is a snackbar', (tester) async {
      system.failWith = OpenInFailure('Could not start /usr/bin/code: denied');
      await pumpApp(tester, store(SessionPlace.local));
      await tester.tap(find.byTooltip('Open the workspace'));
      await frames(tester);
      await tester.tap(find.text('Open in VS Code'));
      await frames(tester);
      expect(find.textContaining('denied'), findsOneWidget);
    });

    testWidgets('the palette offers them too, and nothing on a page', (tester) async {
      await pumpApp(tester, store(SessionPlace.local));
      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.keyK);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      await frames(tester);
      expect(find.widgetWithText(ListTile, 'Open in VS Code'), findsOneWidget);
      await tester.tap(find.widgetWithText(ListTile, 'Open in VS Code'));
      await frames(tester);
      expect(system.started.single, const LaunchPlan('/usr/bin/code', ['/work']));
    });

    testWidgets('a page that cannot start programs shows no button', (tester) async {
      workspaceOpener = WorkspaceOpener(_System(null));
      await pumpApp(tester, store(SessionPlace.local));
      expect(find.byTooltip('Open the workspace'), findsNothing);
      expect(find.byIcon(Icons.folder_open), findsNothing);
    });
  });
}
