// Accounts in the built app, against the host library it carries and stand-ins for Claude Code's command line: an API-key account
// is added through the sealed form, a signed-in (scripted login) account through its page and pasted code, each is offered by an
// entry, and a turn runs on each. A canary key is planted and every file the test leaves is scanned for its encodings.
//
//   flutter test integration_test/accounts_test.dart -d windows   (or -d linux)
//
// Needs the stand-in (`cargo build -p canoa-host --examples`, `harness_standin`), found as
// test/fixtures/examples.dart says. CANOA_HOST has to be unset, as it is for a person.
import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:canoa/accounts/account_models.dart';
import 'package:canoa/accounts/accounts_controller.dart';
import 'package:canoa/accounts/accounts_host.dart';
import 'package:canoa/accounts/accounts_section.dart';
import 'package:canoa/acp/client.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/desktop/host_control.dart';
import 'package:canoa/desktop/host_supervisor.dart';
import 'package:canoa/desktop/host_tool_io.dart';
import 'package:canoa_native/canoa_native.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';

import '../test/fixtures/examples.dart';

const _canary = 'sk-ant-CANARY-0d7e53b1a9c4';

String _toml(String text) => jsonEncode(text);

/// The encodings a credential might be written in, the plain one included.
List<String> _encodings(String secret) {
  final bytes = utf8.encode(secret);
  return [
    secret,
    base64.encode(bytes),
    base64Url.encode(bytes),
    base64.encode(bytes).replaceAll('=', ''),
    [for (final byte in bytes) byte.toRadixString(16).padLeft(2, '0')].join(),
    Uri.encodeComponent(secret),
    jsonEncode(secret).replaceAll('"', ''),
  ];
}

/// Every file under [dir] whose bytes (read as text) hold one of [needles], by path.
List<String> _leaks(Directory dir, List<String> needles) => [
  for (final file in dir.listSync(recursive: true, followLinks: false).whereType<File>())
    if (_holds(file, needles)) file.path,
];

bool _holds(File file, List<String> needles) {
  final List<int> bytes;
  try {
    bytes = file.readAsBytesSync();
  } on FileSystemException {
    // A login the host placed for a session is taken away when the session ends, which can be between the listing and this read,
    // and the host's own lock file is locked while it runs: neither can be read by anyone looking for a key.
    return false;
  }
  final text = latin1.decode(bytes);
  return needles.any(text.contains);
}

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('an API-key account and a scripted-login account are added and each runs a turn, the key sealed all the way', (tester) async {
    expect(Platform.environment['CANOA_HOST'], isNull, reason: 'the route under test takes no program');
    final standin = exampleBinary('harness_standin');
    if (standin == null) fail(exampleMissing('harness_standin')!);

    final dir = await Directory.systemTemp.createTemp('canoa-accounts');
    final claude = File('${dir.path}${Platform.pathSeparator}claude${Platform.isWindows ? '.exe' : ''}')..writeAsBytesSync(File(standin).readAsBytesSync());
    final cfg = File('${dir.path}${Platform.pathSeparator}canoa.cfg');
    await cfg.writeAsString('''
[host]
account_store = "file"

[host.agents.claude-accounts]
harness = "claude"
executable = ${_toml(claude.path)}
peers = []
''');
    final tool = LibraryHostTool(CanoaNative.open(), home: '${dir.path}${Platform.pathSeparator}home');
    final host = HostSupervisor(tool);
    addTearDown(() async {
      await host.stop();
      try {
        await dir.delete(recursive: true);
      } on FileSystemException {
        // Windows may hold a file a moment longer.
      }
    });

    Future<void> until(bool Function() done, String what) async {
      final deadline = DateTime.now().add(const Duration(seconds: 45));
      while (!done()) {
        if (DateTime.now().isAfter(deadline)) {
          final shown = [for (final text in find.byType(Text).evaluate()) (text.widget as Text).data].whereType<String>().take(40).toList();
          fail('timed out waiting for $what; the screen says $shown; the host said: ${host.log}');
        }
        await tester.pump(const Duration(milliseconds: 100));
      }
    }

    await host.apply(configPath: cfg.path);
    await until(() => host.phase == HostPhase.running, 'the host to run');

    // What the app writes after an account is made: the entry offers it, in the person's own file, and the host serves it.
    Future<void> change(CanoaConfig Function(CanoaConfig) edit) async {
      final changed = edit(CanoaConfig.parse(await cfg.readAsString(), name: cfg.path));
      await cfg.writeAsString(changed.encode());
      await host.apply(configPath: cfg.path);
    }

    final controller = AccountsController(
      host: ControlAccountsHost(HostControl(host)),
      config: () => CanoaConfig.parse(cfg.readAsStringSync(), name: cfg.path),
      change: change,
    );
    // A list asked while the host restarts fails, and the app asks again when its host serves anew; this controller stands alone, so
    // it is asked again here once the host serves and its last answer was a failure.
    int offers(String entry) {
      if (controller.problem != null && !controller.loading && host.phase == HostPhase.running && host.served.isNotEmpty) {
        unawaited(controller.refresh());
      }
      return controller.offeredBy(entry).length;
    }

    await tester.pumpWidget(MaterialApp(home: Scaffold(body: SingleChildScrollView(child: AccountsSection(controller: controller)))));
    await tester.binding.setSurfaceSize(const Size(1000, 1600));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    await until(() => !controller.loading, 'the host to list accounts');

    Future<void> add(String name, String method) async {
      await tester.tap(find.byKey(const Key('add-account-claude_accounts')));
      await tester.pumpAndSettle();
      await tester.enterText(find.byKey(const Key('add-account-name')), name);
      await tester.tap(find.byKey(Key('login-method $method')));
      await until(
        () => tester.widget<FilledButton>(find.byKey(const Key('add-account-start'))).onPressed != null,
        'Sign in to be offered for $method',
      );
      await tester.tap(find.byKey(const Key('add-account-start')));
      // A login that takes a plain input first (a subscription's optional email) asks for it in a dialog of its own before it starts.
      await until(
        () => find.byKey(const Key('add-account-start')).evaluate().isEmpty || find.byKey(const Key('program-ask')).evaluate().isNotEmpty,
        'the sign-in of $name to start or ask',
      );
      if (find.byKey(const Key('program-ask')).evaluate().isNotEmpty) {
        await tester.tap(find.byKey(const Key('ask-submit')));
        await tester.pumpAndSettle();
      }
      await until(() => find.byKey(const Key('add-account-start')).evaluate().isEmpty, 'the sign-in of $name to start');
    }

    Future<void> typeSealed(String field, String value) async {
      await until(() => find.byKey(ValueKey('secret-$field')).evaluate().isNotEmpty, 'the sealed form for $field');
      await tester.enterText(find.byKey(ValueKey('secret-$field')), value);
      await tester.pump();
      await tester.tap(find.text('Send sealed'));
      await tester.pump();
    }

    Future<void> done(String name) async {
      await until(() => find.byKey(const Key('sign-in-done')).evaluate().isNotEmpty || find.byKey(const Key('sign-in-failed')).evaluate().isNotEmpty, 'the sign-in of $name to end');
      expect(find.byKey(const Key('sign-in-failed')), findsNothing, reason: 'sign-in of $name');
      await tester.tap(find.byKey(const Key('add-account-done')));
      await tester.pumpAndSettle();
    }

    // An API-key account: the key is typed into the sealed form the host asked for.
    await add('metered', 'api-key');
    await typeSealed('api_key', _canary);
    await done('metered');
    // The entry now offers it, written before the host is restarted to serve that: the next sign-in waits for the host.
    await until(() => offers('claude-accounts') == 1, 'the entry to offer the account');
    await tester.pump(const Duration(seconds: 1));
    await until(() => host.phase == HostPhase.running && host.served.isNotEmpty, 'the host to serve again');

    // A scripted login: the stand-in's `claude auth login` takes a pasted code.
    await add('work', 'subscription');
    await until(() => find.byKey(const Key('sign-in-page')).evaluate().isNotEmpty || find.byType(TextField).evaluate().isNotEmpty, 'the login to ask');
    await typeSealed('code', '123456');
    await done('work');
    await until(() => offers('claude-accounts') == 2, 'the entry to offer both accounts');
    await until(() => host.phase == HostPhase.running && host.served.isNotEmpty, 'the host to serve again');

    await controller.refresh();
    expect(controller.accounts.map((each) => each.name), containsAll(['metered', 'work']));
    expect(controller.accounts.every((each) => each.status == AccountStatus.signedIn), isTrue);
    expect(controller.offeredBy('claude-accounts').length, 2);

    // A turn on each, over the in-process link as the app starts one: the entry the app made reaches no paired key, and the
    // session names the account it runs on in the answer, never a credential.
    await until(() => host.phase == HostPhase.running && host.listening.any((entry) => entry.name == 'claude-accounts'), 'the host to serve the entry');
    Future<String> turn(String account) async {
      final link = host.openLink('claude-accounts');
      // The host names the account in the answer to `initialize`, which the client does not keep: read it as it goes by.
      Object? reported;
      final messages = link.messages.map((message) {
        final value = jsonDecode(message);
        final meta = value is Map && value['result'] is Map ? (value['result'] as Map)['_meta'] : null;
        if (meta is Map && meta.containsKey(accountMeta)) reported = meta[accountMeta];
        return message;
      });
      final client = VerifiedAgent.fromChild(
        label: 'claude-accounts',
        messages: messages,
        send: link.send,
        close: link.close,
        decider: (request, from) => Future<AcpDecision>.value(const AcpAllow()),
        initializeMeta: {accountMeta: account},
      );
      try {
        await client.initialize();
        final opened = await client.openSession(cwd: dir.path);
        expect((reported as Map?)?['name'], account, reason: 'the answer names the account the session runs on');
        final written = StringBuffer();
        final listening = client.updates.listen((update) => written.write(update.text ?? ''));
        await client.prompt(opened.id, 'hello on $account');
        await Future<void>.delayed(const Duration(milliseconds: 200));
        await listening.cancel();
        return '$written ${jsonEncode(opened.raw)}';
      } finally {
        await client.close();
      }
    }

    expect(await turn('metered'), contains('hello on metered'));
    expect(await turn('work'), contains('hello on work'));
    final leaked = _leaks(dir, _encodings(_canary).where((each) => each.length > 8).toList());
    // The sealed store's file holds it only encrypted; canoa.cfg, the ledger and every other file hold no form of it.
    expect(leaked, isEmpty, reason: 'a file holds the planted key: $leaked');
    expect(host.log, isNot(contains(_canary)));
  });
}
