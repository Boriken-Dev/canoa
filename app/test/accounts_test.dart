// Accounts in the app: Settings › Accounts lists them from the host and adds one through the provider's own sign-in methods, a
// key typed into the sealed form goes to the host and nowhere else, and a sign-in can be given up. The composer's chip lists what an
// entry offers, a held account disabled, and the choice reaches the agent in `initialize`'s `_meta`.
import 'dart:convert';

import 'package:canoa/accounts/account_chooser.dart';
import 'package:canoa/accounts/accounts_controller.dart';
import 'package:canoa/accounts/accounts_host.dart';
import 'package:canoa/accounts/accounts_section.dart';
import 'package:canoa/chat/composer.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/sources/sources.dart';
import 'package:coqui_acp/secret.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/fake_accounts.dart';
import 'fixtures/secret_host.dart' show until;

const _canary = 'sk-ant-CANARY-4f9a2c71e8d0';

AccountsController _controller(FakeAccountsHost host, {CanoaConfig config = const CanoaConfig(), String? Function(String)? holder, List<CanoaConfig>? saved}) {
  var held = config;
  return AccountsController(
    host: host,
    config: () => held,
    change: (edit) async {
      held = edit(held);
      saved?.add(held);
    },
    holder: holder,
  );
}

Future<void> _show(WidgetTester tester, AccountsController controller) async {
  tester.view.physicalSize = const Size(900, 1400);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(MaterialApp(home: Scaffold(body: SingleChildScrollView(child: AccountsSection(controller: controller)))));
  await tester.pump();
  await tester.pump();
}

void main() {
  testWidgets('the section lists the accounts the host reports, with their kind, endpoint and state', (tester) async {
    final host = FakeAccountsHost(accounts: [
      account('work', kind: 'claude-login'),
      account('metered'),
      account('old', kind: 'claude-login', state: 'needs sign-in'),
    ]);
    await _show(tester, _controller(host));
    expect(find.text('Claude'), findsOneWidget);
    expect(find.text('Claude subscription · Signed in'), findsOneWidget);
    expect(find.text('Anthropic API key · https://api.anthropic.com · Signed in'), findsOneWidget);
    expect(find.text('Claude subscription · Needs sign-in'), findsOneWidget);
    expect(find.byKey(const Key('account-sign-in old')), findsOneWidget);
    expect(find.byKey(const Key('account-sign-in work')), findsNothing);
  });

  testWidgets('Add account renders the provider\'s sign-in methods and what their kinds weaken, from the catalog alone', (tester) async {
    final host = FakeAccountsHost();
    await _show(tester, _controller(host));
    await tester.tap(find.byKey(const Key('add-account-claude')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('login-method subscription')), findsOneWidget);
    expect(find.byKey(const Key('login-method api-key')), findsOneWidget);
    expect(find.text('Paste a key from the console'), findsOneWidget);
    expect(find.text("A login on disk the agent's commands can read"), findsWidgets);
    // Nothing is chosen, so nothing starts.
    expect(tester.widget<FilledButton>(find.byKey(const Key('add-account-start'))).onPressed, isNull);
    await tester.enterText(find.byKey(const Key('add-account-name')), 'no spaces');
    await tester.tap(find.byKey(const Key('login-method api-key')));
    await tester.pump();
    expect(find.text('Use 1 to 64 letters, digits, - or _'), findsOneWidget);
    expect(tester.widget<FilledButton>(find.byKey(const Key('add-account-start'))).onPressed, isNull);
    await tester.enterText(find.byKey(const Key('add-account-name')), 'metered');
    await tester.pump();
    expect(tester.widget<FilledButton>(find.byKey(const Key('add-account-start'))).onPressed, isNotNull);
  });

  testWidgets('a method that asks something first shows that form, and what is answered is what the host is given', (tester) async {
    final host = FakeAccountsHost();
    await _show(tester, _controller(host));
    await tester.tap(find.byKey(const Key('add-account-claude')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('login-method sso')));
    await tester.pump();
    await tester.tap(find.byKey(const Key('add-account-start')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('program-ask')), findsOneWidget);
    expect(find.text('Work email'), findsWidgets);
    await tester.enterText(find.byKey(const Key('ask-field-email')), 'ana@example.org');
    await tester.tap(find.byKey(const Key('ask-submit')));
    await tester.pump();
    await tester.pump();
    expect(host.inputsGiven.single, {'email': 'ana@example.org'});
    expect(host.signIns.single.method, 'sso');
  });

  testWidgets('the address, its lines and a failure can be selected, and the address copied', (tester) async {
    String? copied;
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(SystemChannels.platform, (call) async {
      if (call.method == 'Clipboard.setData') copied = (call.arguments as Map)['text'] as String?;
      return null;
    });
    addTearDown(() => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(SystemChannels.platform, null));
    final host = FakeAccountsHost();
    await _show(tester, _controller(host));
    await tester.tap(find.byKey(const Key('add-account-claude')));
    await tester.pumpAndSettle();
    await tester.enterText(find.byKey(const Key('add-account-name')), 'metered');
    await tester.tap(find.byKey(const Key('login-method api-key')));
    await tester.pump();
    await tester.tap(find.byKey(const Key('add-account-start')));
    await tester.pump();
    final run = host.signIns.single;
    run.say('Opening the sign-in');
    run.showPage(const SignInPage(url: 'https://console.example/login?state=abc'));
    await tester.pump();
    expect(find.widgetWithText(SelectableText, 'Opening the sign-in'), findsOneWidget);
    expect(find.byKey(const Key('sign-in-url')), findsOneWidget);
    await tester.tap(find.byKey(const Key('sign-in-copy-url')));
    await tester.pump();
    expect(copied, 'https://console.example/login?state=abc');
    run.fail('The link to the host has ended');
    await tester.pump();
    expect(find.widgetWithText(SelectableText, 'It did not sign in: The link to the host has ended'), findsOneWidget);
  });

  testWidgets('a page and a code are shown to open and confirm, and a key goes to the host sealed, never held here', (tester) async {
    final host = FakeAccountsHost();
    final saved = <CanoaConfig>[];
    final controller = _controller(host, saved: saved);
    await _show(tester, controller);
    await tester.tap(find.byKey(const Key('add-account-claude')));
    await tester.pumpAndSettle();
    await tester.enterText(find.byKey(const Key('add-account-name')), 'metered');
    await tester.tap(find.byKey(const Key('login-method api-key')));
    await tester.pump();
    await tester.tap(find.byKey(const Key('add-account-start')));
    await tester.pump();
    final run = host.signIns.single;
    expect(run.method, 'api-key');
    run.showPage(const SignInPage(url: 'https://console.example/login', userCode: 'ABCD-1234'));
    await tester.pump();
    expect(find.byKey(const Key('sign-in-url')), findsOneWidget);
    expect(find.text('ABCD-1234'), findsOneWidget);
    expect(find.byKey(const Key('sign-in-copy-code')), findsOneWidget);

    final (request, environment, receiver) = (await tester.runAsync(keyRequest))!;
    final answered = run.ask(request, environment);
    await tester.pump();
    await until(tester, find.byKey(const ValueKey('secret-api_key')));
    await tester.enterText(find.byKey(const ValueKey('secret-api_key')), _canary);
    await tester.pump();
    await tester.tap(find.text('Send sealed'));
    final reply = (await tester.runAsync(() => answered.timeout(const Duration(seconds: 5))))!;
    // What leaves the app for the host holds the key only sealed to the host's key.
    expect(jsonEncode(reply.toJson()), isNot(contains(_canary)));
    expect(reply, isA<AcpSecretAccepted>());
    final ends = (await tester.runAsync(linkEnds))!;
    final accepted = reply as AcpSecretAccepted;
    final opened = (await tester.runAsync(() => receiver.openValues(
      ends.hostVerifier,
      ends.app.identity.prover.id,
      accepted.plain,
      accepted.sealed,
      accepted.auth,
      DateTime.now().millisecondsSinceEpoch ~/ 1000,
    )))!;
    expect(opened.secrets['api_key']!.text, _canary);

    run.complete(account('metered'));
    await tester.pump();
    expect(find.byKey(const Key('sign-in-done')), findsOneWidget);
    expect(find.textContaining(_canary), findsNothing);
    // A made account is offered by an entry that runs its provider, which canoa.cfg then names.
    await tester.pump();
    await tester.pump();
    expect(saved.last.hostAccountsOf('claude-accounts'), ['metered']);
    expect(saved.last.hostHarnessOf('claude-accounts'), 'claude');
    // No paired key reaches the entry the app made: only this computer's own application does.
    expect(saved.last.hostEntryIsOpen('claude-accounts'), isFalse);
  });

  testWidgets('Cancel gives up a sign-in: the host is told to stop and the form it asked is cancelled', (tester) async {
    final host = FakeAccountsHost();
    await _show(tester, _controller(host));
    await tester.tap(find.byKey(const Key('add-account-claude')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('login-method api-key')));
    await tester.pump();
    await tester.tap(find.byKey(const Key('add-account-start')));
    await tester.pump();
    final run = host.signIns.single;
    final (request, environment, _) = (await tester.runAsync(keyRequest))!;
    final answered = run.ask(request, environment);
    await tester.pump();
    await tester.tap(find.byKey(const Key('add-account-cancel')));
    await tester.pumpAndSettle();
    expect(host.cancelled, 1);
    expect(run.phase, SignInPhase.cancelled);
    expect(await answered, isA<AcpSecretCancelled>());
    expect(find.byKey(const Key('add-account')), findsNothing);
  });

  testWidgets('a sign-in that fails says why, in the host\'s words', (tester) async {
    final host = FakeAccountsHost();
    await _show(tester, _controller(host));
    await tester.tap(find.byKey(const Key('add-account-claude')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('login-method subscription')));
    await tester.pump();
    await tester.tap(find.byKey(const Key('add-account-start')));
    await tester.pump();
    host.signIns.single.fail('login refused: the code was not accepted');
    await tester.pump();
    expect(find.textContaining('login refused: the code was not accepted'), findsOneWidget);
    expect(find.byKey(const Key('add-account-done')), findsOneWidget);
  });

  testWidgets('removing an account asks first, and is not offered while a session holds it', (tester) async {
    final host = FakeAccountsHost(accounts: [account('work', kind: 'claude-login'), account('metered')]);
    await _show(tester, _controller(host, holder: (name) => name == 'work' ? 'Fix the build' : null));
    expect(find.textContaining('held by "Fix the build"'), findsOneWidget);
    await tester.tap(find.byKey(const Key('account-menu work')));
    await tester.pumpAndSettle();
    expect(tester.widget<PopupMenuItem<String>>(find.byKey(const Key('account-remove work'))).enabled, isFalse);
    await tester.tapAt(const Offset(5, 5));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('account-menu metered')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('account-remove metered')));
    await tester.pumpAndSettle();
    expect(find.textContaining('It does not sign out at Claude'), findsOneWidget);
    await tester.tap(find.byKey(const Key('remove-account-confirm')));
    await tester.pumpAndSettle();
    expect(host.removed, ['metered']);
  });

  group('the composer\'s chip', () {
    const config = CanoaConfig(host: {
      'agents': {
        'Claude-accounts': {'harness': 'claude', 'accounts': ['work', 'metered', 'old']},
      },
    });

    Future<(SessionStore, ChatSession, FakeAccountsHost)> chip(WidgetTester tester, {String? Function(String)? holder}) async {
      final host = FakeAccountsHost(accounts: [
        account('work', kind: 'claude-login'),
        account('metered'),
        account('old', kind: 'claude-login', state: 'needs sign-in'),
      ]);
      final controller = _controller(host, config: config, holder: holder);
      await controller.refresh();
      final store = SessionStore(ledger: SessionLedger(MemoryLedgerBackend()));
      const source = AgentSource.hosted('Claude-accounts', harness: 'claude');
      final session = store.createDraft(source: source, cwd: '/work/repo');
      await tester.pumpWidget(MaterialApp(
        home: Scaffold(body: ListenableBuilder(
          listenable: store,
          builder: (context, _) => AccountChooser(store: store, session: session, kit: ComposerKit(accounts: controller, onAccountSettings: () {})),
        )),
      ));
      return (store, session, host);
    }

    testWidgets('lists what the entry offers, the first the default, and a held or signed-out account disabled', (tester) async {
      final (store, session, _) = await chip(tester, holder: (name) => name == 'metered' ? 'Fix the build' : null);
      expect(find.descendant(of: find.byKey(const Key('option-account')), matching: find.text('work')), findsOneWidget);
      await tester.tap(find.byKey(const Key('option-account')));
      await tester.pumpAndSettle();
      expect(tester.widget<PopupMenuItem<Object>>(find.byKey(const Key('account-choice work'))).enabled, isTrue);
      expect(tester.widget<PopupMenuItem<Object>>(find.byKey(const Key('account-choice metered'))).enabled, isFalse);
      expect(find.text('In use by "Fix the build"'), findsOneWidget);
      expect(tester.widget<PopupMenuItem<Object>>(find.byKey(const Key('account-choice old'))).enabled, isFalse);
      expect(find.text('Needs sign-in'), findsOneWidget);
      expect(find.byKey(const Key('account-add')), findsOneWidget);
      await tester.tap(find.byKey(const Key('account-choice work')));
      await tester.pumpAndSettle();
      expect(session.account, 'work');
      store.dispose();
    });

    testWidgets('a source that offers none shows no chip', (tester) async {
      final host = FakeAccountsHost(accounts: [account('work')]);
      final controller = _controller(host);
      await controller.refresh();
      final store = SessionStore(ledger: SessionLedger(MemoryLedgerBackend()));
      final session = store.createDraft(source: const AgentSource.hosted('Claude-accounts', harness: 'claude'), cwd: '/work/repo');
      await tester.pumpWidget(MaterialApp(home: Scaffold(body: AccountChooser(store: store, session: session, kit: ComposerKit(accounts: controller)))));
      expect(find.byKey(const Key('option-account')), findsNothing);
      store.dispose();
    });
  });

  test('the account is kept with the session in the ledger by its name, and a draft keeps the choice', () async {
    final ledger = SessionLedger(MemoryLedgerBackend());
    ledger.record(peerId: "p", sessionId: "s1", entry: "Claude-accounts", place: SessionPlace.host, account: "work");
    final text = jsonEncode(ledger.entries.single.toJson());
    expect(ledger.entries.single.account, 'work');
    expect(LedgerEntry.fromJson(jsonDecode(text))?.account, 'work');
    final store = SessionStore(ledger: ledger);
    final fork = store.fork(store.createDraft(account: 'metered'));
    expect(fork.account, 'metered');
    store.dispose();
  });

  test('canoa.cfg offers an account by name only, and a removed one goes back to the harness\'s own login', () {
    const base = CanoaConfig();
    final once = base.withAccountOffered(account: 'work', harness: 'claude');
    expect(once.hostAccountsOf('claude-accounts'), ['work']);
    final twice = once.withAccountOffered(account: 'metered', harness: 'claude');
    expect(twice.hostEntriesRunning('claude'), ['claude-accounts']);
    expect(twice.hostAccountsOf('claude-accounts'), ['work', 'metered']);
    expect(twice.encode(), isNot(contains('sk-')));
    final one = twice.withoutAccountOffered('work');
    expect(one.hostAccountsOf('claude-accounts'), ['metered']);
    final none = one.withoutAccountOffered('metered');
    expect(none.hostAccountsOf('claude-accounts'), isEmpty);
    expect(none.encode(), isNot(contains('accounts = ')));
  });

  group('who an account reaches', () {
    const open = CanoaConfig(host: {
      'agents': {
        'Claude': {'harness': 'claude'},
        'Shared': {'harness': 'claude', 'accounts': ['work', 'old']},
      },
    });

    test("a new account never goes to an entry of the person's, and the entry the app makes reaches no paired key", () {
      final changed = open.withAccountOffered(account: 'metered', harness: 'claude');
      expect(changed.hostAccountsOf('Claude'), isEmpty, reason: 'an entry offering none keeps the login it runs on');
      expect(changed.hostAccountsOf('Shared'), ['work', 'old']);
      expect(changed.hostAccountsOf('claude-accounts'), ['metered']);
      expect(changed.hostEntryIsOpen('claude-accounts'), isFalse);
      expect(changed.hostEntryIsOpen('Shared'), isTrue);
      expect(changed.encode(), contains('peers'));
    });

    test('an account offered to a named entry is appended, so the default stays', () {
      final changed = open.withAccountOffered(account: 'metered', harness: 'claude', entry: 'Shared');
      expect(changed.hostAccountsOf('Shared'), ['work', 'old', 'metered']);
    });

    test("an entry's offers can be put back exactly", () {
      final changed = open.withoutAccountOffered('work').withAccountsOf('Shared', ['work', 'old']);
      expect(changed.hostAccountsOf('Shared'), ['work', 'old']);
    });

    testWidgets('a refused removal puts the offers back in their order, the default included', (tester) async {
      final host = FakeAccountsHost(accounts: [account('work', kind: 'claude-login'), account('old', kind: 'claude-login')])
        ..removeRefusal = 'a session holds it';
      final saved = <CanoaConfig>[];
      final controller = _controller(host, config: open, saved: saved);
      await controller.refresh();
      await expectLater(controller.remove(controller.named('work')!), throwsA(anything));
      expect(saved.last.hostAccountsOf('Shared'), ['work', 'old']);
    });

    testWidgets('offering an account to an entry every paired key reaches asks first, and does nothing when declined', (tester) async {
      final host = FakeAccountsHost(accounts: [account('work', kind: 'claude-login'), account('old', kind: 'claude-login'), account('metered')]);
      final saved = <CanoaConfig>[];
      await _show(tester, _controller(host, config: open, saved: saved));
      await tester.tap(find.byKey(const Key('account-menu metered')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('account-offer metered Shared')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('offer-open-entry')), findsOneWidget);
      expect(find.textContaining('every key paired with this host'), findsOneWidget);
      await tester.tap(find.text('Do not offer it'));
      await tester.pumpAndSettle();
      expect(saved, isEmpty);
      await tester.tap(find.byKey(const Key('account-menu metered')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('account-offer metered Shared')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('offer-open-entry-confirm')));
      await tester.pumpAndSettle();
      expect(saved.last.hostAccountsOf('Shared'), ['work', 'old', 'metered']);
    });
  });

  testWidgets('a sign-in that would ask a secret before it starts is refused, not run with the secret dropped', (tester) async {
    final host = FakeAccountsHost();
    await _show(tester, _controller(host));
    await tester.tap(find.byKey(const Key('add-account-claude')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('login-method token-first')));
    await tester.pump();
    await tester.tap(find.byKey(const Key('add-account-start')));
    await tester.pumpAndSettle();
    expect(find.textContaining('asks for a secret before it signs in'), findsOneWidget);
    expect(host.signIns, isEmpty);
    expect(find.byKey(const Key('program-ask')), findsNothing);
  });

  testWidgets('a page that is not a plain web link is shown but never opened', (tester) async {
    final host = FakeAccountsHost();
    await _show(tester, _controller(host));
    await tester.tap(find.byKey(const Key('add-account-claude')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('login-method subscription')));
    await tester.pump();
    await tester.tap(find.byKey(const Key('add-account-start')));
    await tester.pump();
    host.signIns.single.showPage(const SignInPage(url: 'file:///c:/windows/system32/calc.exe'));
    await tester.pump();
    expect(find.byKey(const Key('sign-in-url-refused')), findsOneWidget);
    expect(tester.widget<OutlinedButton>(find.byKey(const Key('sign-in-open'))).onPressed, isNull);
  });
}
