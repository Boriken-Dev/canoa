// A conversation popped out into a second window of the same engine, on a real desktop. The app runs with the platform's own windows
// (flutter_test's test binding replaces them with stand-ins that have no native window, so this is an app and a driver, as Flutter's
// own windowing tests are), and the driver asks it to act and to say what it sees.
//
//   flutter drive --driver=test_driver/popout_test.dart --target=integration_test/popout_test.dart -d windows   (or -d linux)
//
// The window draws the conversation the main window holds, so a request answered in one window is answered in both, a message
// written in either is in the one queue, and a reply arrives in both at once. Closing the pop-out closes only it; closing the main
// window puts it away in the tray and leaves the pop-out where it is. A button in the pop-out is pressed by calling what it calls:
// a driver cannot reach a second window.
import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/desktop/desktop_shell_io.dart';
import 'package:canoa/main.dart';
import 'package:canoa/popout/platform.dart' as platform;
import 'package:canoa/popout/popout_view.dart';
import 'package:canoa/popout/popout_window.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/workspace/open_in.dart' show workspaceOpener;
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' show ChatDensity;
import 'package:material_ui/material_ui.dart';
// ignore: depend_on_referenced_packages
import 'package:flutter_driver/driver_extension.dart';
import 'package:window_manager/window_manager.dart';

import '../test/fixtures/app_fixture.dart' show CountedWorktrees, FakeClipboard, scripted;
import '../test/fixtures/fake_host.dart' show defaultLocalSources, linkedHost;
import '../test/fixtures/open_host.dart';
import '../test/fixtures/scratch_host.dart';
import '../test/fixtures/scripted_agent.dart';
import '../test/fixtures/source_choices.dart' show FakeHost;
import 'windows_close.dart';

/// Every element of the whole tree, which spans the windows of this engine, that [test] accepts.
List<Element> _elements(bool Function(Element element) test) {
  final found = <Element>[];
  void visit(Element element) {
    if (test(element)) found.add(element);
    element.visitChildren(visit);
  }

  WidgetsBinding.instance.rootElement!.visitChildren(visit);
  return found;
}

bool _inWindow(Element element) {
  var inside = false;
  element.visitAncestorElements((ancestor) {
    if (ancestor.widget is PopoutChat) inside = true;
    return !inside;
  });
  return inside;
}

PopoutChat? get _chat => _elements((e) => e.widget is PopoutChat).map((e) => e.widget as PopoutChat).firstOrNull;

Future<void> main() async {
  final scene = Completer<_Scene>();
  enableFlutterDriverExtension(
    handler: (message) async {
      final ready = await scene.future;
      try {
        return jsonEncode(await ready.handle(message ?? 'state'));
      } on Object catch (error, stack) {
        return jsonEncode({'error': '$error', 'stack': '$stack'});
      }
    },
  );
  scene.complete(await _Scene.start());
}

final class _Scene {
  _Scene(this.store, this.agent, this.session, this.windows, this.shell);

  final SessionStore store;
  final ScriptedAgent agent;
  final ChatSession session;
  final PopoutWindows windows;
  final TrayShell shell;

  static Future<_Scene> start() async {
    final store = SessionStore();
    final windows = platform.createPopoutWindows();
    final shell = TrayShell();
    const config = CanoaConfig(agents: [scripted], density: ChatDensity.verbose);
    await shell.initialize(config, onSettings: () {}, onHosting: () {});
    // Nothing of the person's: a folder of its own for the config and for sessions, and a host that starts nothing.
    final dir = Directory.systemTemp.createTempSync('canoa-popout');
    workspaceOpener = await OpenHost().opener();
    runApp(
      windows.host(
        CanoaApp(
          hostFolders: ScratchHost(dir.path, template: config.sessions.workspace),
          store: store,
          config: config,
          configPath: '${dir.path}/canoa.cfg',
          writeConfig: (_) async {},
          isGitRepository: (_) => false,
          folderExists: (_) => true,
          pickFolder: () async => '/work/repo',
          pickFiles: () async => const [],
          statFile: (_) async => null,
          clipboard: FakeClipboard(),
          worktrees: () => CountedWorktrees(),
          shell: shell,
          host: linkedHost((name) => throw StateError('nothing starts in this test: $name'), sources: () => [...defaultLocalSources, ('Scripted', null)]),
          askChoices: FakeHost().call,
          popoutWindows: windows,
        ),
      ),
    );
    await Future<void>.delayed(const Duration(seconds: 1));
    final agent = ScriptedAgent();
    final session = await agent.start(store);
    // Under v1 a prompt answers when its turn ends, which this one never does.
    unawaited(store.sendText('Fix addNumbers'));
    agent.update({'sessionUpdate': 'agent_message_chunk', 'content': {'type': 'text', 'text': 'Reading calc.js.'}});
    agent.ask(5, 'Edit a.txt');
    agent.ask(6, 'Edit b.txt');
    final scene = _Scene(store, agent, session, windows, shell);
    await scene.until(() => session.pendingPermissions.length == 2, 'the agent\'s requests to be waiting');
    return scene;
  }

  Future<void> until(FutureOr<bool> Function() done, String what, {Duration within = const Duration(seconds: 30)}) async {
    final end = DateTime.now().add(within);
    while (!await done()) {
      if (DateTime.now().isAfter(end)) throw StateError('timed out waiting for $what');
      await Future<void>.delayed(const Duration(milliseconds: 100));
    }
  }

  /// How many drawn texts say [needle], in the pop-out window and elsewhere.
  Map<String, int> _said(String needle) {
    final texts = _elements((e) => e.widget is RichText && (e.widget as RichText).text.toPlainText().contains(needle));
    return {'window': texts.where(_inWindow).length, 'main': texts.where((e) => !_inWindow(e)).length};
  }

  Future<Map<String, Object?>> state() async => {
    'windows': [
      for (final window in windows.windows) {'id': window.id, 'title': window.title, 'link': window.link != null, 'width': window.size.width, 'height': window.size.height},
    ],
    'chats': _elements((e) => e.widget is PopoutChat).length,
    'pending': [for (final q in session.pendingPermissions) q.title],
    'settled': [for (final q in session.chat.settledPermissions) '${q.outcome?.by}'],
    'queue': [for (final m in session.queue) m.text],
    'clients': [for (final c in session.chat.clients) c.id],
    'status': session.status.name,
    'answers': agent.answers().map((m) => m['id']).toList(),
    'prompts': agent.sent.where((m) => m['method'] == 'session/prompt').length,
    'visible': await windowManager.isVisible(),
    'tray': shell.hasTray,
    'said': {
      'Reading calc.js.': _said('Reading calc.js.'),
      'And it is fixed.': _said('And it is fixed.'),
      'Edit b.txt': _said('Edit b.txt'),
      'Edit c.txt': _said('Edit c.txt'),
      'Answered in Canoa.': _said('Answered in Canoa.'),
      '2 messages wait for this turn to end.': _said('2 messages wait for this turn to end.'),
    },
  };

  Future<Map<String, Object?>> handle(String message) async {
    switch (message) {
      case 'state':
        break;
      case 'open':
        final button = _elements((e) => e.widget is IconButton && (e.widget as IconButton).tooltip == 'Open in its own window').single.widget as IconButton;
        button.onPressed!();
        await until(() => windows.windows.length == 1 && _chat != null, 'the window to draw the conversation');
      case 'answer_in_window':
        final question = session.pendingPermissions.first;
        _chat!.conversation.choosePermission(question, question.options.first);
        await until(() => agent.answerTo(5) != null, 'the window\'s answer to reach the agent');
      case 'answer_in_main':
        store.choosePermission(session, session.pendingPermissions.first.options.first);
        await until(() => agent.answerTo(6) != null, 'the main window answer to reach the agent');
        await until(() => _said('Answered in Canoa.')['window']! > 0, 'the window to say where it was answered');
      case 'both_send':
        await Future.wait([store.sendText('from the main window'), _chat!.conversation.send('from the window')]);
        await until(() => session.queue.length == 2, 'both messages to be queued');
        await until(() => _said('2 messages wait for this turn to end.')['window']! > 0, 'the window to show the queue');
      case 'reply':
        agent.update({'sessionUpdate': 'agent_message_chunk', 'content': {'type': 'text', 'text': ' And it is fixed.'}});
        agent.ask(7, 'Edit c.txt');
        await until(() => _said('And it is fixed.').values.every((n) => n > 0) && _said('Edit c.txt').values.every((n) => n > 0), 'the reply and the new request in both windows');
      case 'close_popout':
        if (Platform.isWindows) {
          if (!closeWindowTitled('Fix calc - Canoa')) throw StateError('no window to close among ${windowTitles()}');
        } else {
          await windows.windows.single.close();
        }
        await until(() => windows.windows.isEmpty && _chat == null, 'the window to close');
      case 'close_main':
        await windowManager.close();
        if (shell.hasTray) await until(() async => !await windowManager.isVisible(), 'the main window to go to the tray');
        await Future<void>.delayed(const Duration(seconds: 1));
      case 'restore':
        await shell.open();
        await until(() async => await windowManager.isVisible() && !await windowManager.isMinimized(), 'the main window');
      case 'dock':
        final dock = _elements((e) => e.widget is IconButton && _inWindow(e) && (e.widget as IconButton).tooltip == 'Bring back to the main window').single.widget as IconButton;
        dock.onPressed!();
        await until(() => windows.windows.isEmpty, 'the window to dock back');
      default:
        throw ArgumentError('unknown message $message');
    }
    return state();
  }
}
