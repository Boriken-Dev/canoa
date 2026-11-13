// Canoa's chat reaching the agents `canoa` serves, through the app's own connection code: connectVerifiedAcp, the
// peer proof the web build runs, its sealed channel and VerifiedAgent, against the real host binary. The browser's side
// of the proof is the web_peer example, since the Dart VM has no WebCrypto.
//
// CANOA_TEST_HOST names a built canoa, with its examples (stdio_agent, echo_provider) in examples/ beside it, as
// `cargo build -p canoa-host --bin canoa --examples` leaves them. CANOA_TEST_METHOD (`noise` or `webcrypto`) is the method the host serves by, its default unless set; the browser
// proves by the method the host says. Without CANOA_TEST_HOST the tests are skipped.
@Timeout(Duration(minutes: 2))
library;

import 'dart:convert';
import 'dart:io';

import 'package:canoa/acp/client.dart';
import 'package:canoa/acp/connect.dart';
import 'package:canoa/acp/usage_wire.dart' show UsageNotice;
import 'package:canoa/auth/peer_proof.dart';
import 'package:canoa/chat/agent_events.dart';
import 'package:canoa/desktop/host_supervisor.dart';
import 'package:canoa/desktop/host_tool_io.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/sources/source_icons.dart';
import 'package:canoa/sources/sources.dart';
import 'package:canoa/worktree/host.dart';
import 'package:canoa/worktree/worktree.dart';
import 'package:canoa/workspace/site_meta.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/web_peer_bridge.dart';

// Absolute, since the provider's library is named in a canoa.cfg written to a temporary directory and read from there.
final String? _hostBinary = switch (Platform.environment['CANOA_TEST_HOST']) {
  final String path => File(path).absolute.path,
  null => null,
};
final String? _method = switch (Platform.environment['CANOA_TEST_METHOD']) {
  final String method when method.isNotEmpty => method,
  _ => null,
};

/// An agent entry whose working root is [dir]'s `src`, named through this machine's administrative share
/// (`\\localhost\C$\...`), as a project on a file server is.
String _sharedEntry(Directory dir) {
  final path = File('${dir.path}/src').absolute.path;
  return '''
[host.agents.shared]
program = ${jsonEncode(HostUnderTest._example('stdio_agent', library: false))}
args = ["--scripted"]
working_roots = [${jsonEncode(onShare(path))}]
''';
}

/// [path], a drive path, as the same place named through the administrative share.
String onShare(String path) => '\\\\localhost\\${path[0]}\$${path.substring(2).replaceAll('/', '\\')}';

/// `canoa serve` on a canoa.cfg in a directory of its own.
final class HostUnderTest {
  HostUnderTest._(this.process, this.dir, this.urls, this.peer, this.method);

  final Process process;
  final Directory dir;
  final Map<String, String> urls;
  final String peer;

  /// The method the host said it proves by, which a browser stores with its pin.
  final PeerMethod method;

  static String _example(String name, {required bool library}) {
    final examples = File(_hostBinary!).parent.uri.resolve('examples/').toFilePath();
    final file = library
        ? (Platform.isWindows ? '$name.dll' : Platform.isMacOS ? 'lib$name.dylib' : 'lib$name.so')
        : (Platform.isWindows ? '$name.exe' : name);
    return '$examples$file';
  }

  static Future<HostUnderTest> start() async {
    final dir = await Directory.systemTemp.createTemp('canoa-app-test');
    final config = File('${dir.path}/canoa.cfg');
    await config.writeAsString('''
[appearance]
look = "canoa"

[host.agents.fixture]
program = ${jsonEncode(_example('stdio_agent', library: false))}

[host.providers.echo]
library = ${jsonEncode(_example('echo_provider', library: true))}
provider = "echo_provider"

[host.agents.usage]
program = ${jsonEncode(_example('stdio_agent', library: false))}
args = ["--scripted", "--usage", "--usage-claims=the-agents-own"]

[host.agents.parent]
program = ${jsonEncode(_example('stdio_agent', library: false))}
args = ["--scripted", "--mcp-http", "--modes=default"]

[host.agents.child]
program = ${jsonEncode(_example('stdio_agent', library: false))}
args = ["--scripted", "--mcp-http", "--modes=default"]

[host.agents.site]
program = ${jsonEncode(_example('stdio_agent', library: false))}
args = ["--scripted"]

[host.agents.trees]
program = ${jsonEncode(_example('stdio_agent', library: false))}
working_roots = [${jsonEncode(File('${dir.path}/src').absolute.path)}]
${Platform.isWindows ? _sharedEntry(dir) : ''}
''');
    final process = await Process.start(
      _hostBinary!,
      ['serve', '--config', config.path, if (_method case final method?) ...['--method', method]],
      // The Documents folder a webpage's sessions get their folders in, as this test's own.
      environment: {'COQUI_AGENTIC_PEER_HOME': '${dir.path}/home', 'XDG_DOCUMENTS_DIR': '${dir.path}/documents'},
    );
    process.stderr.transform(utf8.decoder).listen(stderr.write);
    final lines = process.stdout.transform(utf8.decoder).transform(const LineSplitter());
    final urls = <String, String>{};
    String? peer;
    String? method;
    await for (final line in lines.take(Platform.isWindows ? 8 : 7).timeout(const Duration(seconds: 30))) {
      final entry = jsonDecode(line) as Map<String, dynamic>;
      urls[entry['name'] as String] = entry['url'] as String;
      peer = entry['peer'] as String;
      method = entry['method'] as String;
    }
    return HostUnderTest._(process, dir, urls, peer!, PeerMethod.parse(method!));
  }

  Future<ProcessResult> command(List<String> args) => Process.run(
    _hostBinary!,
    args,
    environment: {'COQUI_AGENTIC_PEER_HOME': '${dir.path}/home'},
  );

  Future<void> stop() async {
    process.kill();
    await process.exitCode;
    await dir.delete(recursive: true);
  }
}

/// Whether process [pid] runs, asked without signalling it: on Windows any signal ends a process.
Future<bool> _running(int pid) async {
  if (Platform.isWindows) {
    final listed = await Process.run('tasklist', ['/FI', 'PID eq $pid', '/NH', '/FO', 'CSV']);
    return (listed.stdout as String).contains('"$pid"');
  }
  return (await Process.run('kill', ['-0', '$pid'])).exitCode == 0;
}

void main() {
  final skip = _hostBinary == null ? 'CANOA_TEST_HOST names a built canoa' : null;
  late HostUnderTest host;
  late WebPeerBridge browser;

  setUpAll(() async {
    if (skip != null) return;
    host = await HostUnderTest.start();
    browser = await WebPeerBridge.start();
  });

  tearDownAll(() async {
    if (skip != null) return;
    browser.kill();
    await host.stop();
  });

  Future<VerifiedAgent> connect(String entry, {AgentDecider? decider}) => connectVerifiedAcp(
    chatEndpoints(webSocket: host.urls[entry]!),
    expectedPeerId: host.peer,
    verifyPeer: (channel, expected) => provePeer(browser, channel, expected, method: host.method),
    decider: decider,
  );

  /// What the agent wrote during one turn.
  Future<String> turn(VerifiedAgent client, String session, String text) async {
    final written = StringBuffer();
    final listening = client.updates.listen((update) {
      if (update.kind == 'agent_message_chunk') written.write(update.text ?? '');
    });
    await client.prompt(session, text);
    await Future<void>.delayed(const Duration(milliseconds: 50));
    await listening.cancel();
    return written.toString();
  }

  test('a browser key is turned away until the person accepts it on the host', () async {
    await expectLater(connect('fixture'), throwsA(anything));
    final browserId = await browser.localId(host.method);
    final listed = await host.command(['peers', '--format', 'json']);
    expect(listed.exitCode, 0);
    final waiting = const LineSplitter()
        .convert(listed.stdout as String)
        .map((line) => jsonDecode(line) as Map<String, dynamic>)
        .where((peer) => peer['state'] == 'waiting')
        .map((peer) => peer['peer']);
    expect(waiting, contains(browserId));
    final accepted = await host.command(['accept', browserId]);
    expect(accepted.exitCode, 0, reason: accepted.stderr as String);
  }, skip: skip);

  test('the chat reaches a stdio ACP agent, and a permission it asks reaches the person', () async {
    final asked = <AcpPermissionRequest>[];
    final client = await connect('fixture', decider: (request, _) async {
      asked.add(request);
      return const AcpAllow();
    });
    try {
      final opened = await client.openSession(cwd: host.dir.path);
      expect(await turn(client, opened.id, 'hello'), 'echo: hello');
      expect(await turn(client, opened.id, 'ask to write'), 'the person chose allow');
      expect(asked.single.title, 'Write a file');
    } finally {
      await client.close();
    }
  }, skip: skip);

  test("a scripted agent's quota reaches the app under the host's account, not the agent's", () async {
    final client = await connect('usage');
    try {
      expect(client.reportsUsage, isTrue);
      final opened = await client.openSession(cwd: host.dir.path);
      final notices = <UsageNotice>[];
      final listening = client.usageNotices.listen(notices.add);
      expect(await turn(client, opened.id, 'quota 620'), 'quota 620');
      await listening.cancel();
      final notice = notices.single;
      expect(notice.sessionId, opened.id);
      expect(notice.report.account, 'usage', reason: "the entry's name, whatever the agent named");
      final bucket = notice.report.buckets.single;
      expect((bucket.id, bucket.used, bucket.limit, bucket.unit), ('five_hour', 620, 1000, UsageUnit.permille));
      expect(bucket.fractionPermille, 620);
      final asked = await client.fetchUsage();
      expect(asked?.account, 'usage');
      expect(asked?.buckets.single.used, 620);
    } finally {
      await client.close();
    }
  }, skip: skip);

  /// A parent that hands a task to `child` through the host's delegation tool, as an app's conversation with the host's events and
  /// permission requests going to its store.
  Future<(VerifiedAgent, ChatSession, SessionStore, String)> delegated(String prompt) async {
    final store = SessionStore();
    final client = await connect('parent', decider: store.askPermission);
    final opened = await client.openSession(cwd: host.dir.path);
    final session = store.addProviderSession(
      title: 'Parent',
      agent: 'parent',
      peerId: client.peerId,
      remoteSessionId: opened.id,
      cwd: host.dir.path,
    );
    store.attachVerifiedClient(session.id, client);
    final said = await turn(client, opened.id, prompt);
    final task = (jsonDecode(said) as Map<String, dynamic>)['taskId'] as String;
    return (client, session, store, task);
  }

  Future<void> until(bool Function() done, String what) async {
    final limit = DateTime.now().add(const Duration(seconds: 30));
    while (!done()) {
      if (DateTime.now().isAfter(limit)) fail('timed out waiting for $what');
      await Future<void>.delayed(const Duration(milliseconds: 50));
    }
  }

  test('a delegation reaches the task list, and a child\'s permission is answered from the app', () async {
    final (client, session, store, task) = await delegated('Delegate child Please edit src/report.js');
    try {
      await until(() => session.pendingPermissions.isNotEmpty, "the child's permission request");
      final question = session.pendingPermissions.single;
      expect(question.task?.id, task);
      expect(question.task?.source, 'child');
      expect(question.task?.title, contains('edit src'));
      expect(session.needsYou, isTrue);
      await until(() => session.tasks[task]?.phase == TaskPhase.inputRequired, 'the task to read as waiting');
      store.answerPermission(session, allow: true);
      await until(() => session.tasks[task]?.phase == TaskPhase.completed, 'the task to end');
      expect(session.tasks[task]?.result, 'Renamed everywhere.');
      expect(session.tasks[task]?.updates, isNotEmpty, reason: "the child's own updates reach the list");
    } finally {
      await client.close();
    }
  }, skip: skip);

  test('Stop sends one request to the host, and the task ends canceled', () async {
    final (client, session, store, task) = await delegated('Delegate child Wait');
    try {
      await until(() => session.tasks[task]?.phase == TaskPhase.working, 'the task to work');
      await store.cancelSession(session);
      expect(session.tasks[task]?.phase, TaskPhase.canceled);
      expect(session.tasks.live, isEmpty);
    } finally {
      await client.close();
    }
  }, skip: skip);

  test('a lost connection ends the live tasks, and the host keeps none of them for the next connection', () async {
    final (client, session, store, task) = await delegated('Delegate child Wait');
    await until(() => session.tasks[task]?.phase == TaskPhase.working, 'the task to work');
    final remote = session.remoteSessionId!;
    await client.close();
    await until(() => session.tasks[task]?.phase == TaskPhase.canceled, 'the lost connection to end the task');
    expect(session.tasks.live, isEmpty);

    // Another connection, even one that opens a session of the same id, is told nothing of the old connection's task.
    final next = await connect('parent', decider: store.askPermission);
    try {
      final opened = await next.openSession(cwd: host.dir.path);
      expect(await next.taskHistory(opened.id, task), isNull);
      expect(await next.taskHistory(remote, task), isNull);
      expect(await next.stopTasks(opened.id, taskId: task), isFalse);
    } finally {
      await next.close();
    }
  }, skip: skip);

  test('the chat reaches a Guatiao provider', () async {
    final client = await connect('echo');
    try {
      final opened = await client.openSession(cwd: host.dir.path);
      expect(await turn(client, opened.id, 'hello'), 'echo: hello');
      expect(await turn(client, opened.id, 'again'), 'echo: again');
    } finally {
      await client.close();
    }
  }, skip: skip);

  test('a session starts in a folder on a share under the entry\'s working root, and the agent is told that folder', () async {
    final folder = onShare('${host.dir.path}/src/project');
    await Directory(folder).create(recursive: true);
    final client = await connect('shared');
    try {
      final opened = await client.openSession(cwd: folder);
      expect(await turn(client, opened.id, 'Cwd'), 'cwd: $folder');
      await expectLater(client.openSession(cwd: onShare(host.dir.path)), throwsA(anything),
          reason: 'the host directory is outside the root');
      await expectLater(client.openSession(cwd: '$folder\\..\\..'), throwsA(anything), reason: 'a .. segment is refused');
    } finally {
      await client.close();
    }
  }, skip: skip ?? (Platform.isWindows ? null : 'a share is a Windows thing'));

  test('a session for a site starts in a folder the host made, is reopened there, and a refused name makes nothing', () async {
    final client = await connect('site');
    try {
      await client.initialize();
      final opened = await client.openSession(cwd: siteStandInFolder, setup: siteSetup('example.com'));
      final made = madeFolder(opened.raw);
      expect(made, isNotNull, reason: 'the answer says which folder');
      expect(made!.folder.replaceAll(r'\', '/'), startsWith('${host.dir.path.replaceAll(r'\', '/')}/documents/Canoa/scratch/example.com/'));
      expect(Directory(made.folder).existsSync(), isTrue);
      expect(made.folder.endsWith(made.id), isTrue);
      expect(await turn(client, opened.id, 'Cwd'), 'cwd: ${made.folder}');
      await Directory(made.folder).delete();
      await client.reopenSession(opened.id, cwd: made.folder, setup: siteSetup('example.com'));
      expect(Directory(made.folder).existsSync(), isTrue, reason: 'reopened where it was made');
      await expectLater(client.reopenSession(opened.id, cwd: host.dir.path, setup: siteSetup('example.com')), throwsA(anything),
          reason: 'a folder outside the scratch root');
      await expectLater(client.openSession(cwd: siteStandInFolder, setup: siteSetup('..')), throwsA(anything));
      expect(Directory('${host.dir.path}/documents/Canoa/scratch').listSync().map((each) => each.uri.pathSegments.where((s) => s.isNotEmpty).last),
          ['example.com']);
    } finally {
      await client.close();
    }
  }, skip: skip);

  test('two sessions on one repository each work in a worktree the host makes inside its working root', () async {
    Future<ProcessResult> git(String dir, List<String> args) => Process.run('git',
        ['-C', dir, '-c', 'user.name=Canoa Test', '-c', 'user.email=test@canoa.invalid', '-c', 'commit.gpgsign=false', ...args]);
    final repo = '${host.dir.path}/src/proj';
    await Directory(repo).create(recursive: true);
    await File('$repo/README').writeAsString('hello\n');
    for (final args in [['init', '-q'], ['add', '.'], ['commit', '-q', '-m', 'first']]) {
      expect((await git(repo, args)).exitCode, 0);
    }
    final worktrees = HostWorktrees((method, params) => hostWorktreeRequest(
      chatEndpoints(webSocket: host.urls['trees']!), expectedPeerId: host.peer, method: method, params: params,
      verifyPeer: (channel, expected) => provePeer(browser, channel, expected, method: host.method),
    ));
    final first = await worktrees.create(repo, 'canoa/first-a1b2c3');
    final second = await worktrees.create(repo, 'canoa/second-d4e5f6');
    expect(first.path.replaceAll(r'\', '/'), endsWith('/src/proj.worktrees/first-a1b2c3'));
    expect(second.path.replaceAll(r'\', '/'), endsWith('/src/proj.worktrees/second-d4e5f6'));
    for (final worktree in [first, second]) {
      final client = await connect('trees');
      try {
        await client.openSession(cwd: worktree.path);
      } finally {
        await client.close();
      }
    }
    await expectLater(worktrees.create(host.dir.path, 'canoa/outside'), throwsA(isA<WorktreeFailure>()),
        reason: 'the host directory is outside the root');
    await File('${first.path}/draft').writeAsString('unsaved\n');
    expect((await worktrees.status(first)).changed, 1);
    // The host's own git reads the same folder: the untracked file is one added line, and nothing was written.
    final read = await HostGit(worktrees.call).read(first.path);
    expect(read, isNotNull);
    expect([for (final file in read!.files) (file.path, file.status, file.added)], [('draft', 'A', 1)]);
    expect(read.branch, first.branch);
    expect((await worktrees.status(first)).changed, 1);
    await expectLater(worktrees.remove(first), throwsA(isA<WorktreeFailure>()));
    expect((await worktrees.remove(first, discard: true)).branchKept, isFalse);
    expect((await worktrees.remove(second)).branchKept, isFalse);
    expect((await worktrees.status(first)).exists, isFalse);
  }, skip: skip ?? (Process.runSync('git', ['--version']).exitCode == 0 ? null : 'no git on PATH'));

  group('the app running the host', () {
    late Directory dir;
    late HostSupervisor supervisor;
    late File config;

    setUp(() async {
      dir = await Directory.systemTemp.createTemp('canoa-supervised');
      config = File('${dir.path}/canoa.cfg');
      await config.writeAsString('''
[host.agents.fixture]
program = ${jsonEncode(HostUnderTest._example('stdio_agent', library: false))}
''');
      supervisor = HostSupervisor(
        ProcessHostTool(_hostBinary!, environment: {'COQUI_AGENTIC_PEER_HOME': '${dir.path}/home'}),
        firstPause: const Duration(milliseconds: 200),
        // The host's last words on standard error are awaited for as long as a loaded machine takes to deliver them.
        errorDrain: const Duration(minutes: 2),
      );
    });

    tearDown(() async {
      await supervisor.stop();
      supervisor.dispose();
      await dir.delete(recursive: true);
    });

    Future<void> until(bool Function() done, String what) async {
      final deadline = DateTime.now().add(const Duration(seconds: 20));
      while (!done()) {
        if (DateTime.now().isAfter(deadline)) fail('timed out waiting for $what');
        await Future<void>.delayed(const Duration(milliseconds: 50));
      }
    }

    test('serves canoa.cfg, and a key the person pairs in the app reaches its agent', () async {
      await supervisor.apply(configPath: config.path);
      await until(() => supervisor.phase == HostPhase.running, 'the host to run');
      final url = supervisor.served.single.url;
      final method = PeerMethod.parse(supervisor.served.single.method!);
      // A fresh home's method, which the host chose: the app's own proof runs it, and the turn below shows the two agree.
      expect(await browser.unreachable(method), isNull, reason: 'the app proves by the method a fresh home serves by');
      final client = connectVerifiedAcp(
        chatEndpoints(webSocket: url),
        expectedPeerId: supervisor.peerId!,
        verifyPeer: (channel, expected) => provePeer(browser, channel, expected, method: method),
      );
      await expectLater(client, throwsA(anything), reason: 'an unpaired key is turned away');
      final browserId = await browser.localId(method);
      final waiting = await supervisor.keys();
      expect(waiting.where((key) => !key.accepted).map((key) => key.id), contains(browserId));
      await supervisor.accept(browserId);
      expect((await supervisor.keys()).firstWhere((key) => key.id == browserId).accepted, isTrue);
      final agent = await connectVerifiedAcp(
        chatEndpoints(webSocket: url),
        expectedPeerId: supervisor.peerId!,
        verifyPeer: (channel, expected) => provePeer(browser, channel, expected, method: method),
      );
      try {
        final opened = await agent.openSession(cwd: dir.path);
        expect(await turn(agent, opened.id, 'hello'), 'echo: hello');
      } finally {
        await agent.close();
      }
      await supervisor.revoke(browserId);
      expect((await supervisor.keys()).map((key) => key.id), isNot(contains(browserId)));
    }, skip: skip);

    test('a harness entry added to canoa.cfg is a source the app lists, with nothing in the app naming it', () async {
      await supervisor.apply(configPath: config.path);
      await until(() => supervisor.phase == HostPhase.running, 'the host to run');
      expect(hostedSources(supervisor.served), [const AgentSource.hosted('fixture')]);

      final standin = '${dir.path}/claude${Platform.isWindows ? '.exe' : ''}';
      await File(HostUnderTest._example('harness_standin', library: false)).copy(standin);
      await config.writeAsString('''
[host.agents.fixture]
program = ${jsonEncode(HostUnderTest._example('stdio_agent', library: false))}

[host.agents.reviewer]
harness = "claude"
title = "Claude for review"
executable = ${jsonEncode(standin)}
''');
      expect(await supervisor.check(config.path), isNull);
      await supervisor.apply(configPath: config.path);
      await until(() => supervisor.served.length == 2, 'the host to serve both entries');
      final sources = hostedSources(supervisor.served);
      expect(sources, [const AgentSource.hosted('fixture'), const AgentSource.hosted('reviewer')]);
      final reviewer = sources.last;
      expect((reviewer.harness, reviewer.title, reviewer.displayName), ('claude', 'Claude for review', 'Claude for review'));
      expect(sourceProvider(reviewer), 'claude', reason: 'a harness entry draws its provider mark');
      expect(sourceProvider(sources.first), isNull, reason: 'a program entry has the generic icon');
      expect((sources.first.harness, sources.first.title, sources.first.displayName), (null, null, 'fixture'));
    }, skip: skip);

    test('a host that dies is started again, and stopping ends it for good', () async {
      await supervisor.apply(configPath: config.path);
      await until(() => supervisor.phase == HostPhase.running, 'the host to run');
      final first = supervisor.pid!;
      Process.killPid(first, ProcessSignal.sigkill);
      await until(() => supervisor.phase == HostPhase.waiting, 'the supervisor to see it end');
      await until(() => supervisor.phase == HostPhase.running, 'the host to run again');
      final second = supervisor.pid!;
      expect(second, isNot(first));
      await supervisor.stop();
      expect(supervisor.phase, HostPhase.stopped);
      expect(supervisor.pid, isNull);
      await Future<void>.delayed(const Duration(seconds: 1));
      expect(supervisor.phase, HostPhase.stopped, reason: 'nothing starts it again');
      expect(await _running(second), isFalse, reason: 'the host has ended');
    }, skip: skip);

    test('a host whose app dies without stopping it ends too', () async {
      // CANOA_TEST_DART names the Dart that runs the dying app, `dart` by default.
      final ran = await Process.run(Platform.environment['CANOA_TEST_DART'] ?? 'dart', [
        'run',
        'test/fixtures/dying_parent.dart',
        _hostBinary!,
        config.path,
        '${dir.path}/home',
      ], runInShell: Platform.isWindows);
      expect(ran.exitCode, 0, reason: '${ran.stderr}');
      final pid = int.parse((ran.stdout as String).trim().split('\n').last.trim());
      final deadline = DateTime.now().add(const Duration(seconds: 10));
      while (await _running(pid) && DateTime.now().isBefore(deadline)) {
        await Future<void>.delayed(const Duration(milliseconds: 200));
      }
      final alive = await _running(pid);
      if (alive) Process.killPid(pid, ProcessSignal.sigkill);
      expect(alive, isFalse, reason: 'the host outlived the app that started it');
    }, skip: skip);

    test('a canoa.cfg the host refuses is said in its words and not retried', () async {
      await config.writeAsString('[host.agents.fixture]\nprogram = "x"\nlisten = "not an address"\n');
      expect(await supervisor.check(config.path), isNotNull);
      await supervisor.apply(configPath: config.path);
      await until(() => supervisor.phase == HostPhase.failed, 'the host to refuse it');
      expect(supervisor.problem, contains('listen'));
    }, skip: skip);
  });

  test('a key the host removes loses its link and cannot come back', () async {
    final client = await connect('echo');
    final removed = await host.command(['revoke', await browser.localId(host.method)]);
    expect(removed.exitCode, 0);
    try {
      final deadline = DateTime.now().add(const Duration(seconds: 5));
      while (!client.isClosed && DateTime.now().isBefore(deadline)) {
        await Future<void>.delayed(const Duration(milliseconds: 100));
      }
      expect(client.isClosed, isTrue, reason: 'the host ends the link of a removed key');
      await expectLater(client.openSession(cwd: host.dir.path), throwsA(anything));
    } finally {
      await client.close();
    }
    await expectLater(connect('echo'), throwsA(anything));
  }, skip: skip);
}
