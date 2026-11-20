// Mail in the app: Settings › Mail lists the host's mailboxes with the state of each password, adds one (the password goes to the host
// once and is shown nowhere after), tests a sign-in and removes a mailbox; the chat shows the person what a change to messages
// would do, with the messages listed, before the host applies it.
import 'dart:async';
import 'dart:convert';

import 'package:canoa/acp/client.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/main.dart';
import 'package:canoa/mail/mail_accounts.dart';
import 'package:canoa/session_store.dart';
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' show ChatDensity;
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/fake_mail.dart';
import 'fixtures/settings_fixture.dart';

MailAccountsController _controller(FakeMailHost host) => MailAccountsController(host: host);

Future<MailAccountsController> _show(WidgetTester tester, FakeMailHost host) async {
  final controller = _controller(host);
  await openSettings(tester, section: 'mail', mail: controller);
  await tester.pumpAndSettle();
  return controller;
}

Future<void> _fill(WidgetTester tester, String key, String text) async {
  await tester.enterText(find.byKey(Key(key)), text);
  await tester.pump();
}

void main() {
  testWidgets('Mail is a section only where the host lets this key set mail up', (tester) async {
    await openSettings(tester);
    expect(find.byKey(const Key('settings-nav-mail')), findsNothing);
    await openSettings(tester, mail: _controller(FakeMailHost()));
    expect(find.byKey(const Key('settings-nav-mail')), findsOneWidget);
  });

  testWidgets('the section lists each mailbox with where it is and whether its password is there', (tester) async {
    final host = FakeMailHost(accounts: [
      mailAccount('work', user: 'EXAMPLE\\ana'),
      mailAccount('late', state: 'needs a password'),
      mailAccount('desktop', backend: 'outlook', server: null, state: 'signed in by Outlook'),
    ]);
    await _show(tester, host);
    expect(find.text('Exchange · https://mail.example.test/EWS/Exchange.asmx · EXAMPLE\\ana · password stored'), findsOneWidget);
    expect(find.text('Exchange · https://mail.example.test/EWS/Exchange.asmx · needs a password'), findsOneWidget);
    expect(find.text('Outlook · signed in by Outlook'), findsOneWidget);
    for (final name in ['work', 'late', 'desktop']) {
      expect(find.byKey(Key('mail-test $name')), findsOneWidget);
    }
  });

  testWidgets('with no mailbox the section says so and offers to add one', (tester) async {
    await _show(tester, FakeMailHost());
    expect(find.text('No mailbox'), findsOneWidget);
    expect(find.byKey(const Key('add-mail-account')), findsOneWidget);
  });

  testWidgets('adding a mailbox sends the password to the host once and shows it nowhere', (tester) async {
    final host = FakeMailHost();
    await _show(tester, host);
    await tester.tap(find.byKey(const Key('add-mail-account')));
    await tester.pumpAndSettle();
    await _fill(tester, 'mail-name', 'work');
    await _fill(tester, 'mail-server', 'https://mail.example.test/EWS/Exchange.asmx');
    await _fill(tester, 'mail-user', 'EXAMPLE\\ana');
    await _fill(tester, 'mail-password', canary);
    // The field hides what is typed.
    expect(tester.widget<TextField>(find.byKey(const Key('mail-password'))).obscureText, isTrue);
    await tester.tap(find.byKey(const Key('add-mail-submit')));
    await tester.pumpAndSettle();

    expect(host.added, hasLength(1));
    final sent = host.added.single;
    expect((sent.name, sent.backend, sent.user, sent.password), ('work', 'ews', 'EXAMPLE\\ana', canary));
    expect(find.byKey(const Key('add-mail-dialog')), findsNothing, reason: 'the dialog closed');
    expect(find.byKey(const Key('mail-account work')), findsOneWidget);
    expect(find.textContaining(canary), findsNothing, reason: 'nothing on the page shows the password');
    expect(find.text('Exchange · https://mail.example.test/EWS/Exchange.asmx · EXAMPLE\\ana · password stored'), findsOneWidget);
  });

  testWidgets('a mailbox with no user name signs in with the Windows logon and sends no password', (tester) async {
    final host = FakeMailHost();
    await _show(tester, host);
    await tester.tap(find.byKey(const Key('add-mail-account')));
    await tester.pumpAndSettle();
    await _fill(tester, 'mail-name', 'logon');
    await _fill(tester, 'mail-email', 'ana@example.test');
    await tester.tap(find.byKey(const Key('add-mail-submit')));
    await tester.pumpAndSettle();
    expect(host.added.single.password, isNull);
    expect(host.added.single.user, isNull);
  });

  testWidgets('what is missing is said before the host is asked, and the host\'s own refusal is shown as it words it', (tester) async {
    final host = FakeMailHost(accounts: [mailAccount('work')]);
    await _show(tester, host);
    await tester.tap(find.byKey(const Key('add-mail-account')));
    await tester.pumpAndSettle();
    await _fill(tester, 'mail-name', '');
    await tester.tap(find.byKey(const Key('add-mail-submit')));
    await tester.pump();
    expect(find.text('Name the mailbox'), findsOneWidget);
    // What a name may hold is the host's to say, below: this one goes on to the missing server.
    await _fill(tester, 'mail-name', 'Bad Name');
    await tester.tap(find.byKey(const Key('add-mail-submit')));
    await tester.pump();
    expect(find.text('Give the server address or the mailbox\'s email address'), findsOneWidget);
    await _fill(tester, 'mail-name', 'work');
    await tester.tap(find.byKey(const Key('add-mail-submit')));
    await tester.pump();
    expect(find.text('A mailbox is called that already'), findsOneWidget);
    await _fill(tester, 'mail-name', 'other');
    await tester.tap(find.byKey(const Key('add-mail-submit')));
    await tester.pump();
    expect(find.text('Give the server address or the mailbox\'s email address'), findsOneWidget);
    await _fill(tester, 'mail-server', 'http://mail.example.com/EWS/Exchange.asmx');
    await _fill(tester, 'mail-user', 'u');
    await tester.tap(find.byKey(const Key('add-mail-submit')));
    await tester.pump();
    expect(find.text('Type the password'), findsOneWidget);
    expect(host.added, isEmpty);

    host.refuseAdd = 'the edited canoa.cfg would not be read: server is an https address';
    await _fill(tester, 'mail-password', 'x');
    await tester.tap(find.byKey(const Key('add-mail-submit')));
    await tester.pumpAndSettle();
    expect(find.textContaining('server is an https address'), findsOneWidget);
    expect(find.byKey(const Key('add-mail-dialog')), findsOneWidget, reason: 'the dialog stays so the address can be fixed');
  });

  testWidgets('Test sign-in asks the host and says how it went', (tester) async {
    final host = FakeMailHost(
      accounts: [mailAccount('work', user: 'u'), mailAccount('bad', user: 'u')],
      tests: {'bad': const MailTest(ok: false, problem: 'the server refused the user name or password')},
    );
    await _show(tester, host);
    await tester.tap(find.byKey(const Key('mail-test work')));
    await tester.pumpAndSettle();
    expect(find.text('Signed in. 7 folders.'), findsOneWidget);
    await tester.tap(find.byKey(const Key('mail-test bad')));
    await tester.pumpAndSettle();
    expect(find.text('the server refused the user name or password'), findsOneWidget);
    expect(host.tested, ['work', 'bad']);
  });

  testWidgets('removing a mailbox asks first, then the host forgets it', (tester) async {
    final host = FakeMailHost(accounts: [mailAccount('work', user: 'u'), mailAccount('home')]);
    await _show(tester, host);
    await tester.tap(find.byKey(const Key('mail-remove work')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('remove-mail-account')), findsOneWidget);
    await tester.tap(find.text('Keep it'));
    await tester.pumpAndSettle();
    expect(host.removed, isEmpty);

    await tester.tap(find.byKey(const Key('mail-remove work')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('remove-mail-account-confirm')));
    await tester.pumpAndSettle();
    expect(host.removed, ['work']);
    expect(find.byKey(const Key('mail-account work')), findsNothing);
    expect(find.byKey(const Key('mail-account home')), findsOneWidget);
  });

  testWidgets('a host that does not answer is said in the section', (tester) async {
    final controller = MailAccountsController(host: _Unreachable());
    await openSettings(tester, section: 'mail', mail: controller);
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('mail-problem')), findsOneWidget);
    expect(find.textContaining('did not list the mail accounts'), findsOneWidget);
  });

  group('in the chat', () {
    testWidgets('a change to messages shows the messages it reaches, and Reject reaches the host', (tester) async {
      await tester.binding.setSurfaceSize(const Size(1100, 900));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      final store = SessionStore();
      final agent = _Agent();
      await tester.runAsync(() => agent.start(store));
      await tester.pumpWidget(CanoaApp(store: store));
      await tester.pump();
      await tester.enterText(find.byType(TextField).last, 'Archive the budget mail');
      await tester.tap(find.byTooltip('Send'));
      await _settle(tester);
      // What the host puts to the person for mail_move: the change and the messages, bound to a digest.
      const text = 'Move to Archive\n- Budget 2027 kickoff / Ana Ruiz / 2026-11-11\n- Lunch on Thursday? / Ana Ruiz / 2026-11-12\nBound to: 0123456789abcdef';
      agent.ask(7, toolCall: {
        'title': 'Move to Archive: 2 messages in work',
        'kind': 'other',
        'status': 'pending',
        'content': [
          {'type': 'content', 'content': {'type': 'text', 'text': text}},
        ],
        // The card's own detail is the call's input: the host puts the change and the messages there as well.
        'rawInput': {'change': text},
      });
      await _settle(tester);
      final detail = find.byKey(const Key('permission-detail'));
      expect(detail, findsOneWidget);
      expect(find.descendant(of: detail, matching: find.textContaining('Budget 2027 kickoff')), findsOneWidget);
      expect(find.descendant(of: detail, matching: find.textContaining('Lunch on Thursday?')), findsOneWidget);
      expect(find.descendant(of: detail, matching: find.textContaining('Bound to: 0123456789abcdef')), findsOneWidget);
      await tester.tap(find.byKey(const Key('permission-deny')));
      await _settle(tester);
      expect(agent.answerTo(7)?['result'], {'outcome': {'outcome': 'selected', 'optionId': 'reject'}});
    });

    testWidgets('the mail tools show in the conversation as the tools they are', (tester) async {
      await tester.binding.setSurfaceSize(const Size(1100, 900));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      final store = SessionStore();
      final agent = _Agent();
      await tester.runAsync(() => agent.start(store));
      await tester.pumpWidget(CanoaApp(store: store, config: const CanoaConfig(density: ChatDensity.verbose)));
      await tester.pump();
      await tester.enterText(find.byType(TextField).last, 'What did Ana write?');
      await tester.tap(find.byTooltip('Send'));
      await _settle(tester);
      agent.update({
        'sessionUpdate': 'tool_call',
        'toolCallId': 'm1',
        'title': 'canoa-mail: mail_search',
        'kind': 'search',
        'status': 'completed',
        'rawInput': {'from': 'ana', 'unread': true},
      });
      await _settle(tester);
      expect(find.textContaining('mail_search'), findsWidgets);
    });
  });
}

final class _Unreachable implements MailHost {
  @override
  Future<List<MailAccountInfo>> list() async => throw StateError('no link');

  @override
  Future<MailAccountInfo> add(NewMailAccount account) async => throw StateError('no link');

  @override
  Future<MailTest> test(String name) async => throw StateError('no link');

  @override
  Future<void> remove(String name) async => throw StateError('no link');
}

/// An agent in memory, as the permission card's tests have one: it answers `initialize` and `session/new` and keeps what it was sent.
final class _Agent {
  final incoming = StreamController<String>();
  final sent = <Map<String, dynamic>>[];
  late VerifiedAgent client;

  Future<ChatSession> start(SessionStore store) async {
    client = VerifiedAgent.fromChild(
      label: 'Claude',
      messages: incoming.stream,
      decider: store.askPermission,
      close: () async {},
      send: (line) async {
        final message = jsonDecode(line) as Map<String, dynamic>;
        sent.add(message);
        if (message['method'] == 'initialize') {
          reply(message['id'], {'protocolVersion': 1, 'agentCapabilities': <String, Object?>{}});
        } else if (message['method'] == 'session/new') {
          reply(message['id'], {'sessionId': 's1'});
        } else if (message['id'] != null && message['method'] != null && message['method'] != 'session/prompt') {
          reply(message['id'], <String, Object?>{});
        }
      },
    );
    await client.initialize();
    final opened = await client.openSession(cwd: '/work');
    final session = store.addProviderSession(title: 'Claude', agent: 'Claude', peerId: client.peerId, remoteSessionId: opened.id);
    store.attachVerifiedClient(session.id, client);
    store.select(session.id);
    return session;
  }

  void reply(Object? id, Object? result) => incoming.add(jsonEncode({'jsonrpc': '2.0', 'id': id, 'result': result}));

  void update(Map<String, Object?> update) => incoming.add(jsonEncode({
    'jsonrpc': '2.0',
    'method': 'session/update',
    'params': {'sessionId': 's1', 'update': update},
  }));

  void ask(int id, {required Map<String, Object?> toolCall}) => incoming.add(jsonEncode({
    'jsonrpc': '2.0', 'id': id, 'method': 'session/request_permission',
    'params': {
      'sessionId': 's1',
      'toolCall': {'toolCallId': 'store-change-$id', ...toolCall},
      'options': [
        {'optionId': 'allow-once', 'name': 'Allow', 'kind': 'allow_once'},
        {'optionId': 'reject', 'name': 'Reject', 'kind': 'reject_once'},
      ],
    },
  }));

  Map<String, dynamic>? answerTo(int id) => sent.where((message) => message['id'] == id && message['method'] == null).firstOrNull;
}

Future<void> _settle(WidgetTester tester) async {
  await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 40)));
  await tester.pump();
  await tester.pump(Duration.zero);
}
