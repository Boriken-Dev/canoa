// Mail in the app: Settings › Mail lists the host's mailboxes with the state of each password, adds one (the password goes to the host
// once and is shown nowhere after), tests a sign-in and removes a mailbox; the chat shows the person what a change to messages
// would do, with the messages listed, before the host applies it.
import 'dart:async';
import 'dart:convert';

import 'package:canoa/acp/client.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/l10n/app_text.dart';
import 'package:canoa/l10n/base_text.dart';
import 'package:canoa/main.dart';
import 'package:canoa/session_store.dart';
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' show ChatDensity;
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/fake_mail.dart';
import 'fixtures/settings_fixture.dart';

Future<void> _show(WidgetTester tester, FakeMailService service) async {
  await openSettings(tester, section: 'mail', hostSections: service.sections());
  await tester.pumpAndSettle();
}

Future<void> _fill(WidgetTester tester, String key, String text) async {
  await tester.enterText(find.byKey(Key(key)), text);
  await tester.pump();
}

void main() {
  spanishWords();
  testWidgets('Mail is a section only where the host lists one: the app has none of its own', (tester) async {
    await openSettings(tester);
    expect(find.byKey(const Key('settings-nav-mail')), findsNothing);
    await openSettings(tester, hostSections: FakeMailService().sections());
    expect(find.byKey(const Key('settings-nav-mail')), findsOneWidget);
  });

  testWidgets('the section lists each mailbox with where it is and whether its password is there', (tester) async {
    final host = FakeMailService(accounts: [
      mailAccount('work', user: 'EXAMPLE\\ana'),
      mailAccount('late', state: 'needs a password'),
      mailAccount('cloud', backend: 'graph', server: null, email: 'ana@example.com', state: 'signed in by Microsoft'),
      mailAccount('desktop', backend: 'outlook', server: null, state: 'signed in by Outlook'),
    ]);
    await _show(tester, host);
    expect(find.text('Exchange Server · https://mail.example.test/EWS/Exchange.asmx · EXAMPLE\\ana · password stored'), findsOneWidget);
    expect(find.text('Exchange Server · https://mail.example.test/EWS/Exchange.asmx · needs a password'), findsOneWidget);
    expect(find.text('Microsoft Graph · ana@example.com · signed in by Microsoft'), findsOneWidget);
    expect(find.text('Outlook · signed in by Outlook'), findsOneWidget);
    for (final name in ['work', 'late', 'cloud', 'desktop']) {
      expect(find.byKey(Key('section-mail-test $name')), findsOneWidget);
    }
  });

  testWidgets('with no mailbox the section says so and offers to add one', (tester) async {
    await _show(tester, FakeMailService());
    expect(find.text('No mailbox'), findsOneWidget);
    expect(find.byKey(const Key('section-mail-add')), findsOneWidget);
  });

  testWidgets('adding a mailbox sends the password to the host once and shows it nowhere', (tester) async {
    const canary = 'Mail-canary-9a7c';
    final host = FakeMailService();
    await _show(tester, host);
    await tester.tap(find.byKey(const Key('section-mail-add')));
    await tester.pumpAndSettle();
    await _fill(tester, 'section-mail-name', 'work');
    await _fill(tester, 'section-mail-field-server', 'https://mail.example.test/EWS/Exchange.asmx');
    await _fill(tester, 'section-mail-field-user', 'EXAMPLE\\ana');
    await _fill(tester, 'section-mail-field-password', canary);
    // The field hides what is typed.
    expect(tester.widget<TextField>(find.byKey(const Key('section-mail-field-password'))).obscureText, isTrue);
    await tester.tap(find.byKey(const Key('section-mail-add-submit')));
    await tester.pumpAndSettle();

    expect(host.added, hasLength(1));
    final sent = host.added.single;
    expect((sent['name'], sent['backend'], sent['user'], sent['password']), ('work', 'ews', 'EXAMPLE\\ana', canary));
    expect(find.byKey(const Key('section-mail-add-dialog')), findsNothing, reason: 'the dialog closed');
    expect(find.byKey(const Key('section-mail-row work')), findsOneWidget);
    expect(find.textContaining(canary), findsNothing, reason: 'nothing on the page shows the password');
    expect(find.text('Exchange Server · https://mail.example.test/EWS/Exchange.asmx · EXAMPLE\\ana · password stored'), findsOneWidget);
  });

  testWidgets('a mailbox with no user name signs in with the Windows logon and sends no password', (tester) async {
    final host = FakeMailService();
    await _show(tester, host);
    await tester.tap(find.byKey(const Key('section-mail-add')));
    await tester.pumpAndSettle();
    await _fill(tester, 'section-mail-name', 'logon');
    await _fill(tester, 'section-mail-field-email', 'ana@example.test');
    await tester.tap(find.byKey(const Key('section-mail-add-submit')));
    await tester.pumpAndSettle();
    expect(host.added.single['password'], isNull);
    expect(host.added.single['user'], isNull);
  });

  testWidgets('what is missing is said before the host is asked, and the host\'s own refusal is shown as it words it', (tester) async {
    final host = FakeMailService(accounts: [mailAccount('work')]);
    await _show(tester, host);
    await tester.tap(find.byKey(const Key('section-mail-add')));
    await tester.pumpAndSettle();
    await _fill(tester, 'section-mail-name', '');
    await tester.tap(find.byKey(const Key('section-mail-add-submit')));
    await tester.pump();
    expect(find.text('Give it a name'), findsOneWidget);
    await _fill(tester, 'section-mail-name', 'work');
    await tester.tap(find.byKey(const Key('section-mail-add-submit')));
    await tester.pump();
    expect(find.text('That name is taken already'), findsOneWidget);

    // Switching to Graph validates its required client_id from the backend schema.
    await _fill(tester, 'section-mail-name', 'cloud');
    await tester.tap(find.byKey(const Key('section-mail-kind-picker')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Microsoft Graph').last);
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('section-mail-add-submit')));
    await tester.pump();
    expect(find.textContaining('Application (client) ID'), findsWidgets);
    expect(host.added, isEmpty);

    host.refuseAdd = 'the edited canoa.cfg would not be read: server is an http or https address';
    await _fill(tester, 'section-mail-field-client_id', 'app-1');
    await tester.tap(find.byKey(const Key('section-mail-add-submit')));
    await tester.pumpAndSettle();
    expect(find.textContaining('server is an http or https address'), findsOneWidget);
    expect(find.byKey(const Key('section-mail-add-dialog')), findsOneWidget, reason: 'the dialog stays so the address can be fixed');
  });

  testWidgets('an extension mail backend renders and submits directly from its schema', (tester) async {
    final host = FakeMailService(backends: [
      {
        'id': 'imap',
        'title': 'IMAP',
        'description': 'Connect to any IMAP server.',
        'schema': {
          'type': 'object',
          'properties': {
            'host': {'type': 'string', 'title': 'IMAP host'},
            'password': {'type': 'string', 'title': 'Password', 'writeOnly': true, 'x-sensitive': true},
          },
          'required': ['host'],
        },
        'form': {
          'fields': {
            'password': {'widget': 'password'},
          },
        },
      },
    ]);
    await _show(tester, host);
    await tester.tap(find.byKey(const Key('section-mail-add')));
    await tester.pumpAndSettle();
    await _fill(tester, 'section-mail-name', 'fastmail');
    await _fill(tester, 'section-mail-field-host', 'imap.fastmail.com');
    await _fill(tester, 'section-mail-field-password', 'app-secret');
    await tester.tap(find.byKey(const Key('section-mail-add-submit')));
    await tester.pumpAndSettle();
    expect(host.added.single, {'name': 'fastmail', 'backend': 'imap', 'host': 'imap.fastmail.com', 'password': 'app-secret'});
    expect(find.text('IMAP · imap.fastmail.com · password stored'), findsOneWidget);
  });

  testWidgets('Test sign-in asks the host and says how it went', (tester) async {
    final host = FakeMailService(
      accounts: [mailAccount('work', user: 'u'), mailAccount('bad', user: 'u')],
      tests: {'bad': {'ok': false, 'problem': 'the server refused the user name or password'}},
    );
    await _show(tester, host);
    await tester.tap(find.byKey(const Key('section-mail-test work')));
    await tester.pumpAndSettle();
    expect(find.text('Signed in. 7 folders.'), findsOneWidget);
    await tester.tap(find.byKey(const Key('section-mail-test bad')));
    await tester.pumpAndSettle();
    expect(find.text('the server refused the user name or password'), findsOneWidget);
    expect(host.tested, ['work', 'bad']);
  });

  testWidgets('removing a mailbox asks first, then the host forgets it', (tester) async {
    final host = FakeMailService(accounts: [mailAccount('work', user: 'u'), mailAccount('home')]);
    await _show(tester, host);
    await tester.tap(find.byKey(const Key('section-mail-remove work')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('section-mail-remove-dialog')), findsOneWidget);
    await tester.tap(find.text('Keep it'));
    await tester.pumpAndSettle();
    expect(host.removed, isEmpty);

    await tester.tap(find.byKey(const Key('section-mail-remove work')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('section-mail-remove-confirm')));
    await tester.pumpAndSettle();
    expect(host.removed, ['work']);
    expect(find.byKey(const Key('section-mail-row work')), findsNothing);
    expect(find.byKey(const Key('section-mail-row home')), findsOneWidget);
  });

  testWidgets('a host that does not answer, or refuses the key, is said in the section', (tester) async {
    await _show(tester, FakeMailService(unreachable: true));
    expect(find.byKey(const Key('section-mail-collection-problem')), findsOneWidget);
    expect(find.textContaining('The host did not list them'), findsOneWidget);
    await _show(tester, FakeMailService(denied: true));
    expect(find.text('This key may not manage this list on this host.'), findsOneWidget);
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
      await tester.pump();
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
      await tester.pump();
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

void spanishWords() {
  test('the list controls of a section read in Spanish and in English', () {
    const es = AppText(language: 'es');
    expect(es('collection.remove'), 'Quitar');
    expect(es('collection.remove_title', {'name': 'work'}), '¿Quitar "work"?');
    expect(const AppText()('collection.remove_title', {'name': 'work'}), 'Remove "work"?');
    for (final key in baseStrings['en']!.keys.where((key) => key.startsWith('collection.'))) {
      expect(baseStrings['es']!.containsKey(key), isTrue, reason: key);
    }
  });
}
